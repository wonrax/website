//! The subscription backend: keeps the bot signed in to a ChatGPT or GitHub Copilot subscription
//! and walks a channel through signing in when it isn't. Both sign in with a device code and keep
//! a short-lived access token fresh with a long-lived one; [`Provider`] holds what differs.
//!
//! Every refresh runs under one lock: ChatGPT revokes the whole sign-in when a refresh token is
//! used twice.

mod chatgpt;
mod copilot;
mod oauth;

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use eyre::Context as _;
use reqwest::StatusCode;
use rig::{
    error::{ErrorKind, ProviderError},
    http_client::{
        self, DynHttpClient, HeaderMap, HeaderValue,
        middleware::{Exchange, HttpMiddleware},
    },
    providers::openai::{OpenAI, OpenAIConfig},
    wasm_compat::WasmBoxedFuture,
};
use rig_agent::completion::PromptError;
use rig_reqwest::ReqwestClient;
use serenity::all::{
    ChannelId, Colour, CreateActionRow, CreateButton, CreateEmbed, CreateMessage, EditMessage,
    Http, MessageId,
};
use tokio::sync::{Mutex, watch};
use tracing::Instrument as _;

use self::oauth::{DeviceCode, PollError, PollOutcome, RefreshError, Tokens};
use crate::discord::agent::AgentModel;

pub type DbPool = diesel_async::pooled_connection::deadpool::Pool<diesel_async::AsyncPgConnection>;

/// Which subscription the bot signs in to
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Chatgpt,
    Copilot,
}

impl Provider {
    /// The name people know the subscription by
    pub fn title(self) -> &'static str {
        match self {
            Self::Chatgpt => "ChatGPT",
            Self::Copilot => "GitHub Copilot",
        }
    }

    /// Who runs the sign-in
    fn issuer(self) -> &'static str {
        match self {
            Self::Chatgpt => "OpenAI",
            Self::Copilot => "GitHub",
        }
    }

    /// The key its sign-in is saved under
    fn key(self) -> &'static str {
        match self {
            Self::Chatgpt => "chatgpt",
            Self::Copilot => "copilot",
        }
    }

    /// What the sign-in prompt says after the steps
    fn sign_in_help(self) -> &'static str {
        match self {
            Self::Chatgpt => {
                "Whoever approves it links their ChatGPT account, and I'll use that account for \
                 everyone here. If the page won't take the code, turn on device code sign-in in \
                 ChatGPT under Settings → Security, then enter it again."
            }
            Self::Copilot => {
                "Whoever approves it links their GitHub account, which needs a Copilot plan, and \
                 I'll use that account for everyone here."
            }
        }
    }

    async fn request_device_code(self, http: &reqwest::Client) -> Result<DeviceCode, String> {
        match self {
            Self::Chatgpt => chatgpt::request_device_code(http).await,
            Self::Copilot => copilot::request_device_code(http).await,
        }
    }

    async fn poll_device_code(
        self,
        http: &reqwest::Client,
        code: &mut DeviceCode,
    ) -> Result<PollOutcome, PollError> {
        match self {
            Self::Chatgpt => chatgpt::poll_device_code(http, code).await,
            Self::Copilot => copilot::poll_device_code(http, code).await,
        }
    }

    async fn refresh(
        self,
        http: &reqwest::Client,
        previous: &Tokens,
    ) -> Result<Tokens, RefreshError> {
        match self {
            Self::Chatgpt => chatgpt::refresh(http, previous).await,
            Self::Copilot => copilot::refresh(http, previous).await,
        }
    }

    /// The plan behind a sign-in (plus, pro, ...), when the provider says
    fn plan(self, tokens: &Tokens) -> Option<String> {
        match self {
            Self::Chatgpt => tokens.id_token.as_deref().and_then(chatgpt::plan_type),
            Self::Copilot => None,
        }
    }

    /// rig's configuration for the provider's API, signed in as `grant`
    fn config(self, grant: &Grant) -> OpenAIConfig {
        match self {
            Self::Chatgpt => chatgpt::config(grant),
            Self::Copilot => copilot::config(grant),
        }
    }
}
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

/// A live sign-in
pub struct Session {
    tokens: Tokens,
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
    verification_url: String,
    expires_at: DateTime<Utc>,
    /// Every prompt showing this code, edited in place once the attempt ends
    prompts: Vec<(ChannelId, MessageId)>,
}

pub struct SubscriptionAuth {
    provider: Provider,
    db: DbPool,
    http: reqwest::Client,
    state: watch::Sender<AuthState>,
    refresh_lock: Mutex<()>,
    sign_in: Mutex<Option<PendingSignIn>>,
}

impl SubscriptionAuth {
    /// Pick up `provider`'s saved sign-in and keep it fresh in the background
    pub async fn load(db: DbPool, provider: Provider) -> Arc<Self> {
        let state = match load_saved(&db, provider).await {
            Ok(Some(session)) => {
                tracing::info!(
                    ?provider,
                    expires_at = %session.tokens.expires_at,
                    "Loaded the saved subscription sign-in"
                );
                AuthState::SignedIn(Arc::new(session))
            }
            Ok(None) => {
                tracing::info!(
                    ?provider,
                    "No saved subscription sign-in; the bot asks for one when it's needed"
                );
                AuthState::SignedOut { reason: None }
            }
            Err(e) => {
                tracing::error!(
                    ?e,
                    ?provider,
                    "Failed to load the saved subscription sign-in"
                );
                AuthState::SignedOut {
                    reason: Some("the saved sign-in couldn't be loaded from the database".into()),
                }
            }
        };

        let auth = Arc::new(Self {
            provider,
            db,
            http: reqwest::Client::new(),
            state: watch::Sender::new(state),
            refresh_lock: Mutex::new(()),
            sign_in: Mutex::new(None),
        });
        tokio::spawn(auth.clone().keep_fresh().instrument(tracing::info_span!(
            "subscription_token_refresher",
            ?provider
        )));
        auth
    }

    pub fn provider(&self) -> Provider {
        self.provider
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
                tracing::warn!(error = %e, "Token refresh failed; using the current token until it expires");
                Ok(session.grant())
            }
            Err(unavailable) => Err(unavailable),
        }
    }

    /// The provider refused the current token, which every request carries: refresh it, unless
    /// another caller is doing so already, and return the new grant
    pub async fn recover_from_unauthorized(&self) -> Result<Grant, Unavailable> {
        let session = self.session()?;
        self.refresh(&session).await.map(|fresh| fresh.grant())
    }

    /// The provider's model `name`. Every request carries the newest token, so a run outlives the
    /// `grant` it started on as long as the background refresh keeps up.
    pub fn model(&self, grant: &Grant, name: &str) -> AgentModel {
        client(
            self.provider.config(grant),
            self.http.clone(),
            Some(self.state.subscribe()),
        )
        .completion(name)
        .erase()
    }

    /// Tell the channel why the subscription can't be used. Signed out, that is a sign-in prompt: a new
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
                post(http, channel_id, &Notice::refresh_failing(self.provider, e)).await;
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
        let provider = self.provider;
        let _guard = self.refresh_lock.lock().await;
        let current = self.session()?;
        if current.tokens.access_token != stale.tokens.access_token {
            return Ok(current);
        }

        match provider.refresh(&self.http, &current.tokens).await {
            Ok(tokens) => {
                let session = Arc::new(Session {
                    tokens,
                    refreshed_at: Utc::now(),
                });
                if let Err(e) = self.save(&session).await {
                    // The old refresh token is spent, so a restart before the next good save
                    // needs a new sign-in
                    tracing::error!(?e, "Failed to save the refreshed sign-in");
                }
                tracing::info!(
                    expires_at = %session.tokens.expires_at,
                    "Refreshed the access token"
                );
                self.state
                    .send_replace(AuthState::SignedIn(session.clone()));
                Ok(session)
            }
            Err(RefreshError::Rejected(reason)) => {
                tracing::warn!(%reason, issuer = provider.issuer(), "The refresh token was rejected");
                if let Err(e) = self.forget().await {
                    tracing::error!(?e, "Failed to delete the dead sign-in");
                }
                self.state.send_replace(AuthState::SignedOut {
                    reason: Some(reason.clone()),
                });
                Err(Unavailable::SignedOut {
                    reason: Some(reason),
                })
            }
            Err(RefreshError::Transient(e)) => Err(Unavailable::RefreshFailing(e)),
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
                    tracing::warn!(error = %e, ?retry_in, "Token refresh failed; retrying");
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
                let notice = Notice::sign_in(
                    self.provider,
                    reason,
                    &pending.user_code,
                    &pending.verification_url,
                    pending.expires_at,
                );
                if let Some(message_id) = post(http, channel_id, &notice).await {
                    pending.prompts.push((channel_id, message_id));
                }
            }
            return;
        }

        let provider = self.provider;
        let code = match provider.request_device_code(&self.http).await {
            Ok(code) => code,
            Err(e) => {
                tracing::error!(error = %e, ?provider, "Failed to start a device-code sign-in");
                post(http, channel_id, &Notice::sign_in_unavailable(provider, &e)).await;
                return;
            }
        };

        let expires_at =
            Utc::now() + TimeDelta::from_std(code.ttl).unwrap_or(TimeDelta::minutes(15));
        let notice = Notice::sign_in(
            provider,
            reason,
            &code.user_code,
            &code.verification_url,
            expires_at,
        );
        let prompt = post(http, channel_id, &notice).await;
        *sign_in = Some(PendingSignIn {
            user_code: code.user_code.clone(),
            verification_url: code.verification_url.clone(),
            expires_at,
            prompts: prompt.map(|id| (channel_id, id)).into_iter().collect(),
        });
        tracing::info!(%expires_at, ?provider, "Started a device-code sign-in");

        tokio::spawn(
            self.clone()
                .finish_sign_in(http.clone(), code, expires_at)
                .instrument(tracing::info_span!("subscription_sign_in", ?provider)),
        );
    }

    async fn finish_sign_in(
        self: Arc<Self>,
        http: Arc<Http>,
        mut code: DeviceCode,
        expires_at: DateTime<Utc>,
    ) {
        let provider = self.provider;
        let outcome = match self.wait_for_approval(&mut code, expires_at).await {
            Ok(tokens) => {
                let session = Arc::new(Session {
                    tokens,
                    refreshed_at: Utc::now(),
                });
                let plan = provider.plan(&session.tokens);
                let saved = {
                    let _guard = self.refresh_lock.lock().await;
                    let saved = self
                        .save(&session)
                        .await
                        .inspect_err(|e| tracing::error!(?e, "Failed to save the new sign-in"))
                        .is_ok();
                    self.state.send_replace(AuthState::SignedIn(session));
                    saved
                };
                tracing::info!(?plan, "Signed in");
                Notice::signed_in(provider, plan.as_deref(), saved)
            }
            Err(SignInError::Expired) => {
                tracing::info!("The sign-in code expired unused");
                Notice::code_expired()
            }
            Err(SignInError::Failed(e)) => {
                tracing::warn!(error = %e, "Sign-in failed");
                Notice::sign_in_failed(provider, &e)
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
        code: &mut DeviceCode,
        expires_at: DateTime<Utc>,
    ) -> Result<Tokens, SignInError> {
        let mut network_failures = 0;
        loop {
            tokio::time::sleep(code.poll_interval).await;
            if Utc::now() >= expires_at {
                return Err(SignInError::Expired);
            }

            match self.provider.poll_device_code(&self.http, code).await {
                Ok(PollOutcome::Pending) => network_failures = 0,
                Ok(PollOutcome::Approved(tokens)) => return Ok(tokens),
                Err(PollError::Network(e)) => {
                    network_failures += 1;
                    tracing::warn!(error = %e, network_failures, "Polling the sign-in failed");
                    if network_failures >= MAX_POLL_NETWORK_FAILURES {
                        return Err(SignInError::Failed(format!(
                            "Lost the connection to {} while waiting for the code: {e}",
                            self.provider.issuer()
                        )));
                    }
                }
                Err(e @ PollError::Rejected(_)) => {
                    return Err(SignInError::Failed(e.to_string()));
                }
            }
        }
    }

    async fn save(&self, session: &Session) -> eyre::Result<()> {
        use crate::schema::subscription_auth;

        let row = SavedSignIn {
            provider: self.provider.key().to_string(),
            access_token: session.tokens.access_token.clone(),
            refresh_token: session.tokens.refresh_token.clone(),
            id_token: session.tokens.id_token.clone(),
            account_id: session.tokens.account_id.clone(),
            expires_at: session.tokens.expires_at,
            refreshed_at: session.refreshed_at,
        };
        let mut conn = self.db.get().await.context("No database connection")?;
        diesel::insert_into(subscription_auth::table)
            .values(&row)
            .on_conflict(subscription_auth::provider)
            .do_update()
            .set(&row)
            .execute(&mut conn)
            .await
            .context("Failed to upsert the sign-in")?;
        Ok(())
    }

    async fn forget(&self) -> eyre::Result<()> {
        use crate::schema::subscription_auth;

        let mut conn = self.db.get().await.context("No database connection")?;
        diesel::delete(subscription_auth::table.find(self.provider.key()))
            .execute(&mut conn)
            .await
            .context("Failed to delete the sign-in")?;
        Ok(())
    }

    /// Explain a failed run in the channel when the subscription is what failed. The agent's own
    /// failures, like running out of turns, stay in the logs.
    pub async fn report_run_failure(
        &self,
        http: &Arc<Http>,
        channel_id: ChannelId,
        error: &PromptError,
    ) {
        if let Some(notice) = Notice::run_failure(self.provider, error) {
            post(http, channel_id, &notice).await;
        }
    }
}

/// A client on `http` whose requests carry the newest token of `auth`, if given
fn client(
    config: OpenAIConfig,
    http: reqwest::Client,
    auth: Option<watch::Receiver<AuthState>>,
) -> OpenAI {
    let http = DynHttpClient::new(ReqwestClient::from(http));
    let http = match auth {
        Some(auth) => http.with_middleware(FreshToken(auth)),
        None => http,
    };
    config.connect(http)
}

/// Swaps the newest access token into every request: rig authenticates every request of a run
/// with the token the run started on, which an hour-long run can outlive.
struct FreshToken(watch::Receiver<AuthState>);

impl HttpMiddleware for FreshToken {
    fn before_request_headers<'a>(
        &'a self,
        _exchange: Exchange<'a>,
        headers: &'a mut HeaderMap,
    ) -> WasmBoxedFuture<'a, http_client::Result<()>> {
        // Signed out, the request goes with the old token and fails the way a run expects
        let grant = match &*self.0.borrow() {
            AuthState::SignedIn(session) => Some(session.grant()),
            AuthState::SignedOut { .. } => None,
        };
        if let Some(grant) = grant {
            match HeaderValue::from_str(&format!("Bearer {}", grant.access_token)) {
                Ok(value) => {
                    headers.insert("authorization", value);
                }
                Err(e) => tracing::warn!(?e, "Unusable access token"),
            }
            if let Some(account_id) = grant.account_id
                && let Ok(value) = HeaderValue::from_str(&account_id)
            {
                headers.insert("ChatGPT-Account-Id", value);
            }
        }
        Box::pin(async { Ok(()) })
    }
}

#[derive(Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = crate::schema::subscription_auth)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(treat_none_as_null = true)]
struct SavedSignIn {
    provider: String,
    access_token: String,
    refresh_token: String,
    id_token: Option<String>,
    account_id: Option<String>,
    expires_at: DateTime<Utc>,
    refreshed_at: DateTime<Utc>,
}

async fn load_saved(db: &DbPool, provider: Provider) -> eyre::Result<Option<Session>> {
    use crate::schema::subscription_auth;

    let mut conn = db.get().await.context("No database connection")?;
    let row = subscription_auth::table
        .find(provider.key())
        .select(SavedSignIn::as_select())
        .first(&mut conn)
        .await
        .optional()
        .context("Failed to read the sign-in")?;
    Ok(row.map(|row| Session {
        tokens: Tokens {
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

/// A status message from the bot about the subscription itself. Posted as an embed with no text
/// content, which keeps it out of the conversation the agent sees.
struct Notice {
    title: String,
    body: String,
    colour: Colour,
    /// A code for the user to type, shown on its own
    code: Option<String>,
    /// A button linking to a page
    link: Option<(&'static str, String)>,
}

impl Notice {
    fn sign_in(
        provider: Provider,
        reason: Option<&str>,
        user_code: &str,
        url: &str,
        expires_at: DateTime<Utc>,
    ) -> Self {
        let title = provider.title();
        let lead = match reason {
            Some(reason) => format!("I need a new {title} sign-in: {reason}."),
            None => format!("I run on a {title} subscription and nobody has signed me in yet."),
        };
        Self {
            title: format!("{title} sign-in needed"),
            body: format!(
                "{lead}\n\n\
                 1. Open {url}\n\
                 2. Enter the code below. It expires <t:{expiry}:R>.\n\n\
                 {help} I'll get to the waiting messages as soon as the sign-in goes through.",
                expiry = expires_at.timestamp(),
                help = provider.sign_in_help(),
            ),
            colour: Colour::GOLD,
            code: Some(user_code.to_string()),
            link: Some(("Open the sign-in page", url.to_string())),
        }
    }

    fn signed_in(provider: Provider, plan: Option<&str>, saved: bool) -> Self {
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
        Self::plain(
            format!("Signed in to {}", provider.title()),
            body,
            Colour::DARK_GREEN,
        )
    }

    fn code_expired() -> Self {
        Self::plain(
            "Sign-in code expired".to_string(),
            "Nobody entered the code in time. Mention me to get a new one.".to_string(),
            Colour::DARK_GREY,
        )
    }

    fn sign_in_failed(provider: Provider, error: &str) -> Self {
        Self::plain(
            format!("{} sign-in failed", provider.title()),
            format!("{}\n\nMention me to try again.", detail(error)),
            Colour::RED,
        )
    }

    fn sign_in_unavailable(provider: Provider, error: &str) -> Self {
        Self::plain(
            format!("Couldn't start a {} sign-in", provider.title()),
            format!("{}\n\nMention me to try again.", detail(error)),
            Colour::RED,
        )
    }

    fn refresh_failing(provider: Provider, error: &str) -> Self {
        Self::plain(
            format!("Can't reach {} right now", provider.title()),
            format!(
                "My sign-in needs a refresh and it keeps failing: {}\n\nI'll keep retrying in \
                 the background and answer as soon as it works.",
                detail(error)
            ),
            Colour::ORANGE,
        )
    }

    fn run_failure(provider: Provider, error: &PromptError) -> Option<Self> {
        let again = "Mention me to try again.";
        let title = provider.title();
        if let Some(status) = error.provider_response_status() {
            let body = error.provider_response_body().unwrap_or_default();
            if let Some(resets_at) = usage_limit_reset(body) {
                let until = resets_at
                    .map(|at| format!(" until <t:{}:R>", at.timestamp()))
                    .unwrap_or_else(|| " for now".to_string());
                return Some(Self::plain(
                    format!("{title} usage limit reached"),
                    format!(
                        "The {title} subscription I run on is out of usage{until}. Mention me \
                         again after that."
                    ),
                    Colour::ORANGE,
                ));
            }
            let heading = if status == StatusCode::UNAUTHORIZED {
                format!("{title} rejected my sign-in")
            } else {
                format!("{title} request failed")
            };
            return Some(Self::plain(
                heading,
                format!("HTTP {status}: {}\n\n{again}", oauth::error_message(body)),
                Colour::RED,
            ));
        }

        let unreachable = match error {
            PromptError::Provider(ProviderError::Http(_)) => true,
            PromptError::Provider(_) => false,
            PromptError::Report(report) => match report.kind {
                ErrorKind::Http => true,
                ErrorKind::Json
                | ErrorKind::Url
                | ErrorKind::Request
                | ErrorKind::Response
                | ErrorKind::Provider
                | ErrorKind::ProviderResponse => false,
                _ => return None,
            },
            _ => return None,
        };
        let heading = if unreachable {
            format!("Couldn't reach {title}")
        } else {
            format!("{title} returned an error")
        };
        Some(Self::plain(
            heading,
            format!("{}\n\n{again}", detail(&error.to_string())),
            Colour::RED,
        ))
    }

    fn plain(title: String, body: String, colour: Colour) -> Self {
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
            .title(&self.title)
            .description(&self.body)
            .colour(self.colour);
        match &self.code {
            Some(code) => embed.field("Code", format!("`{code}`"), false),
            None => embed,
        }
    }

    fn components(&self) -> Vec<CreateActionRow> {
        self.link
            .as_ref()
            .map(|(label, url)| {
                vec![CreateActionRow::Buttons(vec![
                    CreateButton::new_link(url).label(*label),
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

/// `Some` when an error body reports the subscription's usage limit, holding the reset time
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
                "Failed to post a subscription notice as an embed; trying plain text"
            );
            channel_id
                .send_message(http, CreateMessage::new().content(notice.text()))
                .await
                .inspect_err(|e| tracing::error!(?e, "Failed to post a subscription notice"))
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
            "Failed to edit a subscription notice as an embed; trying plain text"
        );
        let edit = EditMessage::new().content(notice.text()).components(vec![]);
        if let Err(e) = channel_id.edit_message(http, message_id, edit).await {
            tracing::error!(?e, "Failed to edit a subscription notice");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::constants::{CHATGPT_RESPONDER_MODEL, COPILOT_RESPONDER_MODEL};
    use rig::completion::{CompletionRequest, GenerationOptions};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    /// A turn as the ChatGPT backend streams it when the model has nothing to add: no output
    /// events, and an empty `output` on the terminal event as always
    const EMPTY_TURN_SSE: &str = r#"data: {"type":"response.completed","response":{"id":"resp_1","object":"response","created_at":1,"status":"completed","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"model":"gpt-6-luna","usage":{"input_tokens":1,"input_tokens_details":{"cached_tokens":0},"output_tokens":0,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":1},"output":[],"tools":[]}}

data: [DONE]

"#;

    /// Answers `count` HTTP requests with `sse`. Returns the base URL it listens on and the raw
    /// requests it has received so far.
    async fn serve(
        sse: &'static str,
        count: usize,
    ) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a local port");
        let address = listener.local_addr().expect("local address");
        let received = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = received.clone();
        tokio::spawn(async move {
            for _ in 0..count {
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
                log.lock()
                    .expect("request log")
                    .push(String::from_utf8_lossy(&request).into_owned());
                let content_type = if sse.starts_with("data:") {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{sse}",
                    sse.len()
                );
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write response");
            }
        });
        (format!("http://{address}"), received)
    }

    fn model_at(url: String, auth: Option<watch::Receiver<AuthState>>) -> AgentModel {
        let grant = Grant {
            access_token: "token".to_string(),
            account_id: None,
        };
        client(
            Provider::Chatgpt.config(&grant).with_base_url(url),
            reqwest::Client::new(),
            auth,
        )
        .completion(CHATGPT_RESPONDER_MODEL)
        .erase()
    }

    fn request(session: &str) -> CompletionRequest {
        CompletionRequest::new("hi").options(GenerationOptions::default().cache_key(session))
    }

    fn headers<'a>(request: &'a str, wanted: &str) -> Vec<&'a str> {
        request
            .lines()
            .filter_map(|line| line.split_once(':'))
            .filter(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.trim())
            .collect()
    }

    #[tokio::test]
    async fn requests_of_a_session_share_its_session_id() {
        let (url, received) = serve(EMPTY_TURN_SSE, 2).await;
        let model = model_at(url, None);
        for _ in 0..2 {
            model
                .call(request("discord-channel-1-watcher"))
                .await
                .expect("an empty turn");
        }

        let received = received.lock().expect("request log");
        assert_eq!(received.len(), 2);
        for request in received.iter() {
            assert_eq!(
                headers(request, "session-id"),
                ["discord-channel-1-watcher"],
                "{request}"
            );
        }
    }

    #[tokio::test]
    async fn requests_carry_the_newest_token() {
        let (url, received) = serve(EMPTY_TURN_SSE, 1).await;
        let start = Utc::now();
        let state = watch::Sender::new(AuthState::SignedIn(Arc::new(session(
            start,
            start + TimeDelta::days(8),
        ))));
        let model = model_at(url, Some(state.subscribe()));

        let mut refreshed = session(start, start + TimeDelta::days(8));
        refreshed.tokens.access_token = "refreshed".to_string();
        refreshed.tokens.account_id = Some("account".to_string());
        state.send_replace(AuthState::SignedIn(Arc::new(refreshed)));
        model.call(request("session")).await.expect("an empty turn");

        let received = received.lock().expect("request log");
        assert_eq!(headers(&received[0], "authorization"), ["Bearer refreshed"]);
        assert_eq!(headers(&received[0], "chatgpt-account-id"), ["account"]);
    }

    /// For this bot an empty turn is how a run normally ends: the reply already went out through
    /// `send_discord_message` and the model has nothing to add
    #[tokio::test]
    async fn an_empty_chatgpt_turn_ends_the_run() {
        let (url, _) = serve(EMPTY_TURN_SSE, 1).await;
        let response = model_at(url, None)
            .call(request("session"))
            .await
            .expect("an empty turn");
        assert!(response.choice.is_empty());
    }

    /// A Chat Completions reply, the route Copilot serves non-Codex models on
    const CHAT_REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gemini-4-hydrogen","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;

    #[tokio::test]
    async fn copilot_requests_carry_the_newest_token_as_vs_code() {
        let (url, received) = serve(CHAT_REPLY, 1).await;
        let start = Utc::now();
        let state = watch::Sender::new(AuthState::SignedIn(Arc::new(session(
            start,
            start + TimeDelta::minutes(30),
        ))));
        let grant = Grant {
            access_token: "tid=old".to_string(),
            account_id: None,
        };
        let model = client(
            Provider::Copilot.config(&grant).with_base_url(url),
            reqwest::Client::new(),
            Some(state.subscribe()),
        )
        .completion(COPILOT_RESPONDER_MODEL)
        .erase();

        let mut refreshed = session(start, start + TimeDelta::minutes(30));
        refreshed.tokens.access_token = "tid=new".to_string();
        state.send_replace(AuthState::SignedIn(Arc::new(refreshed)));
        let response = model
            .call(CompletionRequest::new("hi"))
            .await
            .expect("a reply");
        assert!(!response.choice.is_empty());

        let received = received.lock().expect("request log");
        assert!(
            received[0].starts_with("POST /chat/completions "),
            "{}",
            received[0]
        );
        assert_eq!(headers(&received[0], "authorization"), ["Bearer tid=new"]);
        assert_eq!(
            headers(&received[0], "copilot-integration-id"),
            ["vscode-chat"]
        );
        assert_eq!(headers(&received[0], "x-initiator"), ["user"]);
    }

    fn session(refreshed_at: DateTime<Utc>, expires_at: DateTime<Utc>) -> Session {
        Session {
            tokens: Tokens {
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
