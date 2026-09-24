//! The ChatGPT subscription backend: keeps the bot's "Sign in with ChatGPT" session alive and
//! walks a channel through signing in when there is none.
//!
//! OpenAI rotates the refresh token on every refresh and treats a replayed one as stolen, revoking
//! the whole sign-in. So every refresh runs under one lock, and nothing else may hold a copy of the
//! token: sign the bot in on its own rather than reusing a Codex CLI login.

mod oauth;

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use eyre::Context as _;
use reqwest::StatusCode;
use rig::{
    client::CompletionClient as _,
    completion::{
        CompletionError, CompletionModel, CompletionRequest, CompletionResponse,
        ProviderCapabilities, Usage,
    },
    providers::chatgpt::{self, ChatGPTAuth},
    streaming::StreamingCompletionResponse,
};
use rig_agent::{ModelHandle, completion::PromptError};
use serenity::all::{
    ChannelId, Colour, CreateActionRow, CreateButton, CreateEmbed, CreateMessage, EditMessage,
    Http, MessageId,
};
use tokio::sync::{Mutex, watch};
use tracing::Instrument as _;

pub type DbPool = diesel_async::pooled_connection::deadpool::Pool<diesel_async::AsyncPgConnection>;

/// The table holds a single sign-in
const ROW_ID: i32 = 1;
/// A run makes many model calls, so it doesn't start on a token about to expire
const MIN_VALIDITY_FOR_RUN: TimeDelta = TimeDelta::minutes(10);
/// While refreshing keeps failing, the old token still serves until this close to expiry
const MIN_VALIDITY_FOR_USE: TimeDelta = TimeDelta::minutes(1);
const REFRESH_RETRY_MIN: Duration = Duration::from_secs(30);
const REFRESH_RETRY_MAX: Duration = Duration::from_secs(30 * 60);
/// Failed polls in a row before a sign-in gives up on the network
const MAX_POLL_NETWORK_FAILURES: u32 = 6;
/// A pending code isn't shown to another channel with less time left than it takes to type it
const MIN_CODE_TIME_LEFT: TimeDelta = TimeDelta::minutes(2);
/// Cap on error details quoted in the channel
const MAX_DETAIL_CHARS: usize = 300;
/// How rig's ChatGPT provider fails a turn that holds no text and no tool call
/// (`completion_response_from_sse_body` in rig-core's Responses streaming module)
const EMPTY_TURN_ERROR: &str = "Response contained no parts";

/// A live sign-in
pub struct Session {
    tokens: oauth::Tokens,
    refreshed_at: DateTime<Utc>,
}

impl Session {
    fn grant(&self) -> Grant {
        Grant {
            access_token: self.tokens.access_token.clone(),
            account_id: self.tokens.account_id.clone(),
        }
    }

    fn valid_for(&self, margin: TimeDelta) -> bool {
        self.tokens.expires_at - Utc::now() > margin
    }

    /// Three quarters into the access token's life, leaving the last quarter for retries
    fn refresh_due_at(&self) -> DateTime<Utc> {
        let lifetime = self.tokens.expires_at - self.refreshed_at;
        if lifetime <= TimeDelta::zero() {
            return self.refreshed_at;
        }
        self.refreshed_at + lifetime * 3 / 4
    }
}

#[derive(Clone)]
pub enum AuthState {
    SignedIn(Arc<Session>),
    /// `reason` says why the previous sign-in ended, if there was one
    SignedOut {
        reason: Option<String>,
    },
}

/// What a run authenticates with
#[derive(Clone)]
pub struct Grant {
    access_token: String,
    account_id: Option<String>,
}

pub enum Unavailable {
    SignedOut {
        reason: Option<String>,
    },
    /// Signed in, but the access token has run out and refreshing it keeps failing
    RefreshFailing(String),
}

enum SignInError {
    Expired,
    Failed(String),
}

struct PendingSignIn {
    user_code: String,
    expires_at: DateTime<Utc>,
    /// Every prompt showing this code, edited in place once the attempt ends
    prompts: Vec<(ChannelId, MessageId)>,
}

pub struct ChatgptAuth {
    db: DbPool,
    http: reqwest::Client,
    state: watch::Sender<AuthState>,
    refresh_lock: Mutex<()>,
    sign_in: Mutex<Option<PendingSignIn>>,
}

impl ChatgptAuth {
    /// Pick up the saved sign-in and keep it fresh in the background
    pub async fn load(db: DbPool) -> Arc<Self> {
        let state = match load_saved(&db).await {
            Ok(Some(session)) => {
                tracing::info!(
                    expires_at = %session.tokens.expires_at,
                    "Loaded the saved ChatGPT sign-in"
                );
                AuthState::SignedIn(Arc::new(session))
            }
            Ok(None) => {
                tracing::info!("No saved ChatGPT sign-in; the bot asks for one when it's needed");
                AuthState::SignedOut { reason: None }
            }
            Err(e) => {
                tracing::error!(?e, "Failed to load the saved ChatGPT sign-in");
                AuthState::SignedOut {
                    reason: Some("the saved sign-in couldn't be loaded from the database".into()),
                }
            }
        };

        let auth = Arc::new(Self {
            db,
            http: reqwest::Client::new(),
            state: watch::Sender::new(state),
            refresh_lock: Mutex::new(()),
            sign_in: Mutex::new(None),
        });
        tokio::spawn(
            auth.clone()
                .keep_fresh()
                .instrument(tracing::info_span!("chatgpt_token_refresher")),
        );
        auth
    }

    /// Changes whenever the bot signs in, refreshes its token, or loses its sign-in
    pub fn subscribe(&self) -> watch::Receiver<AuthState> {
        self.state.subscribe()
    }

    /// A token good for a whole run, refreshed first if it's close to expiring
    pub async fn access(&self) -> Result<Grant, Unavailable> {
        let session = self.session()?;
        if session.valid_for(MIN_VALIDITY_FOR_RUN) {
            return Ok(session.grant());
        }

        match self.refresh(&session).await {
            Ok(fresh) => Ok(fresh.grant()),
            Err(Unavailable::RefreshFailing(e)) if session.valid_for(MIN_VALIDITY_FOR_USE) => {
                tracing::warn!(error = %e, "ChatGPT token refresh failed; using the current token until it expires");
                Ok(session.grant())
            }
            Err(unavailable) => Err(unavailable),
        }
    }

    /// ChatGPT refused `rejected`: refresh unless that already happened, and return the new grant
    pub async fn recover_from_unauthorized(&self, rejected: &Grant) -> Result<Grant, Unavailable> {
        let session = self.session()?;
        if session.tokens.access_token != rejected.access_token {
            return Ok(session.grant());
        }
        self.refresh(&session).await.map(|fresh| fresh.grant())
    }

    /// The ChatGPT model `name`, authenticated as `grant`. Built for every run, so live agent
    /// sessions pick up refreshed tokens.
    pub fn model(&self, grant: &Grant, name: &str) -> eyre::Result<ModelHandle> {
        let client = chatgpt::Client::builder()
            .api_key(ChatGPTAuth::AccessToken {
                access_token: grant.access_token.clone(),
                account_id: grant.account_id.clone(),
            })
            .http_client(self.http.clone())
            // rig prepends a stock "You are ChatGPT" line unless this is empty
            .default_instructions("")
            .allow_device_flow(false)
            .build()
            .context("Failed to build the ChatGPT client")?;
        Ok(ModelHandle::new(ChatgptModel(
            client.completion_model(name),
        )))
    }

    /// Tell the channel why ChatGPT can't be used. Signed out, that is a sign-in prompt: a new
    /// device code, or the one already waiting for someone to enter it.
    pub async fn report_unavailable(
        self: &Arc<Self>,
        http: &Arc<Http>,
        channel_id: ChannelId,
        unavailable: &Unavailable,
    ) {
        match unavailable {
            Unavailable::SignedOut { reason } => {
                self.request_sign_in(http, channel_id, reason.as_deref())
                    .await;
            }
            Unavailable::RefreshFailing(e) => {
                post(http, channel_id, &Notice::refresh_failing(e)).await;
            }
        }
    }

    fn session(&self) -> Result<Arc<Session>, Unavailable> {
        match &*self.state.borrow() {
            AuthState::SignedIn(session) => Ok(session.clone()),
            AuthState::SignedOut { reason } => Err(Unavailable::SignedOut {
                reason: reason.clone(),
            }),
        }
    }

    /// Rotate `stale`'s tokens. Callers holding the same stale session share one refresh, since a
    /// second use of its refresh token would get the sign-in revoked.
    async fn refresh(&self, stale: &Session) -> Result<Arc<Session>, Unavailable> {
        let _guard = self.refresh_lock.lock().await;
        let current = self.session()?;
        if current.tokens.access_token != stale.tokens.access_token {
            return Ok(current);
        }

        match oauth::refresh(&self.http, &current.tokens).await {
            Ok(tokens) => {
                let session = Arc::new(Session {
                    tokens,
                    refreshed_at: Utc::now(),
                });
                if let Err(e) = self.save(&session).await {
                    // The old refresh token is spent, so a restart before the next good save
                    // needs a new sign-in
                    tracing::error!(?e, "Failed to save the refreshed ChatGPT sign-in");
                }
                tracing::info!(
                    expires_at = %session.tokens.expires_at,
                    "Refreshed the ChatGPT access token"
                );
                self.state
                    .send_replace(AuthState::SignedIn(session.clone()));
                Ok(session)
            }
            Err(oauth::RefreshError::Rejected(reason)) => {
                tracing::warn!(%reason, "OpenAI rejected the ChatGPT refresh token");
                if let Err(e) = self.forget().await {
                    tracing::error!(?e, "Failed to delete the dead ChatGPT sign-in");
                }
                self.state.send_replace(AuthState::SignedOut {
                    reason: Some(reason.clone()),
                });
                Err(Unavailable::SignedOut {
                    reason: Some(reason),
                })
            }
            Err(oauth::RefreshError::Transient(e)) => Err(Unavailable::RefreshFailing(e)),
        }
    }

    async fn keep_fresh(self: Arc<Self>) {
        let mut state = self.state.subscribe();
        let mut retry_in = REFRESH_RETRY_MIN;
        loop {
            let session = match &*state.borrow_and_update() {
                AuthState::SignedIn(session) => Some(session.clone()),
                AuthState::SignedOut { .. } => None,
            };
            let Some(session) = session else {
                // Nothing to refresh until someone signs in
                if state.changed().await.is_err() {
                    return;
                }
                continue;
            };

            let wait = (session.refresh_due_at() - Utc::now())
                .to_std()
                .unwrap_or_default();
            tokio::select! {
                // A new sign-in, or a refresh a run did on its own: plan from the new token
                changed = state.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    continue;
                }
                _ = tokio::time::sleep(wait) => {}
            }

            match self.refresh(&session).await {
                Ok(_) => retry_in = REFRESH_RETRY_MIN,
                // Surfaces as a state change on the next pass
                Err(Unavailable::SignedOut { .. }) => {}
                Err(Unavailable::RefreshFailing(e)) => {
                    tracing::warn!(error = %e, ?retry_in, "ChatGPT token refresh failed; retrying");
                    tokio::select! {
                        changed = state.changed() => {
                            if changed.is_err() {
                                return;
                            }
                        }
                        _ = tokio::time::sleep(retry_in) => {}
                    }
                    retry_in = (retry_in * 2).min(REFRESH_RETRY_MAX);
                }
            }
        }
    }

    async fn request_sign_in(
        self: &Arc<Self>,
        http: &Arc<Http>,
        channel_id: ChannelId,
        reason: Option<&str>,
    ) {
        let mut sign_in = self.sign_in.lock().await;
        // A sign-in may have gone through while this channel was deciding to ask for one
        if matches!(*self.state.borrow(), AuthState::SignedIn(_)) {
            return;
        }

        if let Some(pending) = sign_in.as_mut() {
            let prompted_here = pending.prompts.iter().any(|(id, _)| *id == channel_id);
            if !prompted_here && pending.expires_at - Utc::now() > MIN_CODE_TIME_LEFT {
                let notice = Notice::sign_in(reason, &pending.user_code, pending.expires_at);
                if let Some(message_id) = post(http, channel_id, &notice).await {
                    pending.prompts.push((channel_id, message_id));
                }
            }
            return;
        }

        let code = match oauth::request_device_code(&self.http).await {
            Ok(code) => code,
            Err(e) => {
                tracing::error!(error = %e, "Failed to start a ChatGPT device-code sign-in");
                post(http, channel_id, &Notice::sign_in_unavailable(&e)).await;
                return;
            }
        };

        let expires_at = Utc::now()
            + TimeDelta::from_std(oauth::DEVICE_CODE_TTL).unwrap_or(TimeDelta::minutes(15));
        let notice = Notice::sign_in(reason, &code.user_code, expires_at);
        let prompt = post(http, channel_id, &notice).await;
        *sign_in = Some(PendingSignIn {
            user_code: code.user_code.clone(),
            expires_at,
            prompts: prompt.map(|id| (channel_id, id)).into_iter().collect(),
        });
        tracing::info!(%expires_at, "Started a ChatGPT device-code sign-in");

        tokio::spawn(
            self.clone()
                .finish_sign_in(http.clone(), code, expires_at)
                .instrument(tracing::info_span!("chatgpt_sign_in")),
        );
    }

    async fn finish_sign_in(
        self: Arc<Self>,
        http: Arc<Http>,
        code: oauth::DeviceCode,
        expires_at: DateTime<Utc>,
    ) {
        let outcome = match self.wait_for_approval(&code, expires_at).await {
            Ok(tokens) => {
                let session = Arc::new(Session {
                    tokens,
                    refreshed_at: Utc::now(),
                });
                let plan = session
                    .tokens
                    .id_token
                    .as_deref()
                    .and_then(oauth::plan_type);
                let saved = {
                    let _guard = self.refresh_lock.lock().await;
                    let saved = self
                        .save(&session)
                        .await
                        .inspect_err(|e| {
                            tracing::error!(?e, "Failed to save the new ChatGPT sign-in")
                        })
                        .is_ok();
                    self.state.send_replace(AuthState::SignedIn(session));
                    saved
                };
                tracing::info!(?plan, "Signed in to ChatGPT");
                Notice::signed_in(plan.as_deref(), saved)
            }
            Err(SignInError::Expired) => {
                tracing::info!("The ChatGPT sign-in code expired unused");
                Notice::code_expired()
            }
            Err(SignInError::Failed(e)) => {
                tracing::warn!(error = %e, "ChatGPT sign-in failed");
                Notice::sign_in_failed(&e)
            }
        };

        // Only now, after a successful sign-in is in place: a channel finding no pending sign-in
        // while still signed out would start another one
        let prompts = self
            .sign_in
            .lock()
            .await
            .take()
            .map(|pending| pending.prompts)
            .unwrap_or_default();
        for (channel_id, message_id) in prompts {
            replace(&http, channel_id, message_id, &outcome).await;
        }
    }

    async fn wait_for_approval(
        &self,
        code: &oauth::DeviceCode,
        expires_at: DateTime<Utc>,
    ) -> Result<oauth::Tokens, SignInError> {
        let mut network_failures = 0;
        loop {
            tokio::time::sleep(code.poll_interval).await;
            if Utc::now() >= expires_at {
                return Err(SignInError::Expired);
            }

            match oauth::poll_device_code(&self.http, code).await {
                Ok(oauth::PollOutcome::Pending) => network_failures = 0,
                Ok(oauth::PollOutcome::Approved(grant)) => {
                    return oauth::exchange_authorization(&self.http, grant)
                        .await
                        .map_err(SignInError::Failed);
                }
                Err(oauth::PollError::Network(e)) => {
                    network_failures += 1;
                    tracing::warn!(error = %e, network_failures, "Polling the ChatGPT sign-in failed");
                    if network_failures >= MAX_POLL_NETWORK_FAILURES {
                        return Err(SignInError::Failed(format!(
                            "Lost the connection to OpenAI while waiting for the code: {e}"
                        )));
                    }
                }
                Err(e @ oauth::PollError::Rejected(_)) => {
                    return Err(SignInError::Failed(e.to_string()));
                }
            }
        }
    }

    async fn save(&self, session: &Session) -> eyre::Result<()> {
        use crate::schema::chatgpt_auth;

        let row = SavedSignIn {
            id: ROW_ID,
            access_token: session.tokens.access_token.clone(),
            refresh_token: session.tokens.refresh_token.clone(),
            id_token: session.tokens.id_token.clone(),
            account_id: session.tokens.account_id.clone(),
            expires_at: session.tokens.expires_at,
            refreshed_at: session.refreshed_at,
        };
        let mut conn = self.db.get().await.context("No database connection")?;
        diesel::insert_into(chatgpt_auth::table)
            .values(&row)
            .on_conflict(chatgpt_auth::id)
            .do_update()
            .set(&row)
            .execute(&mut conn)
            .await
            .context("Failed to upsert the ChatGPT sign-in")?;
        Ok(())
    }

    async fn forget(&self) -> eyre::Result<()> {
        use crate::schema::chatgpt_auth;

        let mut conn = self.db.get().await.context("No database connection")?;
        diesel::delete(chatgpt_auth::table.find(ROW_ID))
            .execute(&mut conn)
            .await
            .context("Failed to delete the ChatGPT sign-in")?;
        Ok(())
    }
}

/// rig's ChatGPT model, except that a turn with nothing in it ends the run instead of failing it.
///
/// The ChatGPT backend leaves a turn's items out of its final `response.completed` event, so rig
/// rebuilds the turn from the streamed events, and fails it when they hold no text and no tool
/// call. For this bot that is how a run normally ends: the reply already went out through
/// `send_discord_message` and the model has nothing to add. Other providers return an empty turn
/// there, which the agent loop takes as the end of the run and keeps out of history. A failed run
/// instead loses its tool calls from the session, replies included.
#[derive(Clone)]
struct ChatgptModel(chatgpt::ResponsesCompletionModel);

impl CompletionModel for ChatgptModel {
    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        match self.0.completion(request).await {
            Err(CompletionError::ResponseError(message)) if message == EMPTY_TURN_ERROR => {
                tracing::debug!("ChatGPT ended the turn without output");
                Ok(CompletionResponse::new(Vec::new(), Usage::new(), "chatgpt"))
            }
            result => result,
        }
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse, CompletionError> {
        self.0.stream(request).await
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.0.capabilities()
    }
}

#[derive(Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = crate::schema::chatgpt_auth)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(treat_none_as_null = true)]
struct SavedSignIn {
    id: i32,
    access_token: String,
    refresh_token: String,
    id_token: Option<String>,
    account_id: Option<String>,
    expires_at: DateTime<Utc>,
    refreshed_at: DateTime<Utc>,
}

async fn load_saved(db: &DbPool) -> eyre::Result<Option<Session>> {
    use crate::schema::chatgpt_auth;

    let mut conn = db.get().await.context("No database connection")?;
    let row = chatgpt_auth::table
        .find(ROW_ID)
        .select(SavedSignIn::as_select())
        .first(&mut conn)
        .await
        .optional()
        .context("Failed to read the ChatGPT sign-in")?;
    Ok(row.map(|row| Session {
        tokens: oauth::Tokens {
            access_token: row.access_token,
            refresh_token: row.refresh_token,
            id_token: row.id_token,
            account_id: row.account_id,
            expires_at: row.expires_at,
        },
        refreshed_at: row.refreshed_at,
    }))
}

pub fn is_unauthorized(error: &PromptError) -> bool {
    error.provider_response_status() == Some(StatusCode::UNAUTHORIZED)
}

/// Explain a failed run in the channel when ChatGPT is what failed. The agent's own failures,
/// like running out of turns, stay in the logs.
pub async fn report_run_failure(http: &Arc<Http>, channel_id: ChannelId, error: &PromptError) {
    if let Some(notice) = Notice::run_failure(error) {
        post(http, channel_id, &notice).await;
    }
}

/// A status message from the bot about ChatGPT itself. Posted as an embed with no text content,
/// which keeps it out of the conversation the agent sees.
struct Notice {
    title: &'static str,
    body: String,
    colour: Colour,
    /// A code for the user to type, shown on its own
    code: Option<String>,
    /// A button linking to a page
    link: Option<(&'static str, &'static str)>,
}

impl Notice {
    fn sign_in(reason: Option<&str>, user_code: &str, expires_at: DateTime<Utc>) -> Self {
        let lead = match reason {
            Some(reason) => format!("I need a new ChatGPT sign-in: {reason}."),
            None => "I run on a ChatGPT subscription and nobody has signed me in yet.".to_string(),
        };
        Self {
            title: "ChatGPT sign-in needed",
            body: format!(
                "{lead}\n\n\
                 1. Open {url}\n\
                 2. Enter the code below. It expires <t:{expiry}:R>.\n\n\
                 Whoever approves it links their ChatGPT account, and I'll use that account for \
                 everyone here. If the page won't take the code, turn on device code sign-in in \
                 ChatGPT under Settings → Security, then enter it again. I'll get to the waiting \
                 messages as soon as the sign-in goes through.",
                url = oauth::VERIFICATION_URL,
                expiry = expires_at.timestamp(),
            ),
            colour: Colour::GOLD,
            code: Some(user_code.to_string()),
            link: Some(("Open the sign-in page", oauth::VERIFICATION_URL)),
        }
    }

    fn signed_in(plan: Option<&str>, saved: bool) -> Self {
        let plan = plan
            .map(|plan| {
                let mut chars = plan.chars();
                let capitalized: String = chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect())
                    .unwrap_or_default();
                format!(" on the {capitalized} plan")
            })
            .unwrap_or_default();
        let mut body = format!("Signed in{plan}. Getting to the waiting messages now.");
        if !saved {
            body.push_str(
                "\n\nI couldn't save the sign-in to my database, so I'll ask again after my next \
                 restart.",
            );
        }
        Self::plain("Signed in to ChatGPT", body, Colour::DARK_GREEN)
    }

    fn code_expired() -> Self {
        Self::plain(
            "Sign-in code expired",
            "Nobody entered the code in time. Mention me to get a new one.".to_string(),
            Colour::DARK_GREY,
        )
    }

    fn sign_in_failed(error: &str) -> Self {
        Self::plain(
            "ChatGPT sign-in failed",
            format!("{}\n\nMention me to try again.", detail(error)),
            Colour::RED,
        )
    }

    fn sign_in_unavailable(error: &str) -> Self {
        Self::plain(
            "Couldn't start a ChatGPT sign-in",
            format!("{}\n\nMention me to try again.", detail(error)),
            Colour::RED,
        )
    }

    fn refresh_failing(error: &str) -> Self {
        Self::plain(
            "Can't reach ChatGPT right now",
            format!(
                "My sign-in needs a refresh and it keeps failing: {}\n\nI'll keep retrying in \
                 the background and answer as soon as it works.",
                detail(error)
            ),
            Colour::ORANGE,
        )
    }

    fn run_failure(error: &PromptError) -> Option<Self> {
        let again = "Mention me to try again.";
        if let Some(status) = error.provider_response_status() {
            let body = error.provider_response_body().unwrap_or_default();
            if let Some(resets_at) = usage_limit_reset(body) {
                let until = resets_at
                    .map(|at| format!(" until <t:{}:R>", at.timestamp()))
                    .unwrap_or_else(|| " for now".to_string());
                return Some(Self::plain(
                    "ChatGPT usage limit reached",
                    format!(
                        "The ChatGPT subscription I run on is out of usage{until}. Mention me \
                         again after that."
                    ),
                    Colour::ORANGE,
                ));
            }
            let title = if status == StatusCode::UNAUTHORIZED {
                "ChatGPT rejected my sign-in"
            } else {
                "ChatGPT request failed"
            };
            return Some(Self::plain(
                title,
                format!("HTTP {status}: {}\n\n{again}", oauth::error_message(body)),
                Colour::RED,
            ));
        }

        match error {
            PromptError::CompletionError(CompletionError::HttpError(e)) => Some(Self::plain(
                "Couldn't reach ChatGPT",
                format!("{}\n\n{again}", detail(&e.to_string())),
                Colour::RED,
            )),
            PromptError::CompletionError(e) => Some(Self::plain(
                "ChatGPT returned an error",
                format!("{}\n\n{again}", detail(&e.to_string())),
                Colour::RED,
            )),
            _ => None,
        }
    }

    fn plain(title: &'static str, body: String, colour: Colour) -> Self {
        Self {
            title,
            body,
            colour,
            code: None,
            link: None,
        }
    }

    fn embed(&self) -> CreateEmbed {
        let embed = CreateEmbed::new()
            .title(self.title)
            .description(&self.body)
            .colour(self.colour);
        match &self.code {
            Some(code) => embed.field("Code", format!("`{code}`"), false),
            None => embed,
        }
    }

    fn components(&self) -> Vec<CreateActionRow> {
        self.link
            .map(|(label, url)| {
                vec![CreateActionRow::Buttons(vec![
                    CreateButton::new_link(url).label(label),
                ])]
            })
            .unwrap_or_default()
    }

    /// Fallback for channels where the bot may not post embeds
    fn text(&self) -> String {
        let mut text = format!("**{}**\n{}", self.title, self.body);
        if let Some(code) = &self.code {
            text.push_str(&format!("\n\nCode: `{code}`"));
        }
        text
    }
}

/// `Some` when a ChatGPT error body reports the subscription's usage limit, holding the reset time
/// if it gives one
fn usage_limit_reset(body: &str) -> Option<Option<DateTime<Utc>>> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let error = json.get("error")?;
    if error.get("type")?.as_str()? != "usage_limit_reached" {
        return None;
    }
    let resets_at = error
        .get("resets_at")
        .and_then(serde_json::Value::as_i64)
        .and_then(|at| DateTime::from_timestamp(at, 0))
        .or_else(|| {
            error
                .get("resets_in_seconds")
                .and_then(serde_json::Value::as_i64)
                .map(|secs| Utc::now() + TimeDelta::seconds(secs))
        });
    Some(resets_at)
}

fn detail(text: &str) -> String {
    oauth::truncate(text.trim(), MAX_DETAIL_CHARS)
}

async fn post(http: &Arc<Http>, channel_id: ChannelId, notice: &Notice) -> Option<MessageId> {
    let message = CreateMessage::new()
        .embed(notice.embed())
        .components(notice.components());
    match channel_id.send_message(http, message).await {
        Ok(message) => Some(message.id),
        Err(e) => {
            tracing::warn!(
                ?e,
                "Failed to post a ChatGPT notice as an embed; trying plain text"
            );
            channel_id
                .send_message(http, CreateMessage::new().content(notice.text()))
                .await
                .inspect_err(|e| tracing::error!(?e, "Failed to post a ChatGPT notice"))
                .ok()
                .map(|message| message.id)
        }
    }
}

/// Swap a posted notice for `notice`, dropping its buttons
async fn replace(http: &Arc<Http>, channel_id: ChannelId, message_id: MessageId, notice: &Notice) {
    let edit = EditMessage::new()
        .content("")
        .embed(notice.embed())
        .components(vec![]);
    if let Err(e) = channel_id.edit_message(http, message_id, edit).await {
        tracing::warn!(
            ?e,
            "Failed to edit a ChatGPT notice as an embed; trying plain text"
        );
        let edit = EditMessage::new().content(notice.text()).components(vec![]);
        if let Err(e) = channel_id.edit_message(http, message_id, edit).await {
            tracing::error!(?e, "Failed to edit a ChatGPT notice");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::constants::CHATGPT_RESPONDER_MODEL;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    /// A turn as the ChatGPT backend streams it when the model has nothing to add: no output
    /// events, and an empty `output` on the terminal event as always
    const EMPTY_TURN_SSE: &str = r#"data: {"type":"response.completed","response":{"id":"resp_1","object":"response","created_at":1,"status":"completed","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"model":"gpt-6-luna","usage":{"input_tokens":1,"input_tokens_details":{"cached_tokens":0},"output_tokens":0,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":1},"output":[],"tools":[]}}

data: [DONE]

"#;

    /// Answers one HTTP request with `sse` and returns the base URL it listens on
    async fn serve_once(sse: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a local port");
        let address = listener.local_addr().expect("local address");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            // Read the whole request so the client never writes into a closed socket
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.expect("read request");
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let body_len = text[..header_end]
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + body_len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{sse}",
                sse.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });
        format!("http://{address}")
    }

    async fn model_serving(sse: &'static str) -> ChatgptModel {
        let client = chatgpt::Client::builder()
            .api_key(ChatGPTAuth::AccessToken {
                access_token: "token".to_string(),
                account_id: None,
            })
            .base_url(serve_once(sse).await)
            .default_instructions("")
            .allow_device_flow(false)
            .build()
            .expect("client");
        ChatgptModel(client.completion_model(CHATGPT_RESPONDER_MODEL))
    }

    #[tokio::test]
    async fn rig_fails_an_empty_chatgpt_turn_with_the_expected_error() {
        let ChatgptModel(inner) = model_serving(EMPTY_TURN_SSE).await;
        let request = inner.completion_request("hi").build();

        match inner.completion(request).await {
            Err(CompletionError::ResponseError(message)) => assert_eq!(message, EMPTY_TURN_ERROR),
            other => panic!(
                "rig no longer fails an empty turn the way EMPTY_TURN_ERROR expects: {:?}",
                other.map(|response| response.choice)
            ),
        }
    }

    #[tokio::test]
    async fn an_empty_chatgpt_turn_ends_the_run() {
        let model = model_serving(EMPTY_TURN_SSE).await;
        let request = model.completion_request("hi").build();

        let response = model.completion(request).await.expect("an empty turn");
        assert!(response.choice.is_empty());
    }

    fn session(refreshed_at: DateTime<Utc>, expires_at: DateTime<Utc>) -> Session {
        Session {
            tokens: oauth::Tokens {
                access_token: "access".to_string(),
                refresh_token: "refresh".to_string(),
                id_token: None,
                account_id: None,
                expires_at,
            },
            refreshed_at,
        }
    }

    #[test]
    fn refresh_is_due_three_quarters_into_the_token_life() {
        let start = Utc::now();
        let due = session(start, start + TimeDelta::days(8)).refresh_due_at();
        assert_eq!(due, start + TimeDelta::days(6));

        // Already expired when it was saved: refresh straight away
        let due = session(start, start - TimeDelta::hours(1)).refresh_due_at();
        assert_eq!(due, start);
    }

    #[test]
    fn usage_limit_errors_carry_their_reset_time() {
        let reset = usage_limit_reset(
            r#"{"error": {"type": "usage_limit_reached", "resets_at": 2000000000}}"#,
        );
        assert_eq!(
            reset.flatten().map(|at| at.timestamp()),
            Some(2_000_000_000)
        );

        let reset = usage_limit_reset(r#"{"error": {"type": "usage_limit_reached"}}"#);
        assert_eq!(reset, Some(None));

        assert_eq!(
            usage_limit_reset(r#"{"detail": "Unsupported model"}"#),
            None
        );
    }
}
