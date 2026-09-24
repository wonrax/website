use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use futures::{
    SinkExt as _, StreamExt,
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
};
use rig::message::Message as RigMessage;
use rig_agent::completion::PromptError;
use serenity::all::{ChannelId, Context, GetMessages, MessageId, Typing, UserId};
use tokio::sync::watch;
use tracing::{Instrument as _, instrument};

use crate::discord::{
    agent::{self, AgentSession, LlmBackend, Memories, Role, Turn},
    bot::Guild,
    chatgpt::{self, AuthState, Grant},
    constants::{
        AGENT_SESSION_TIMEOUT, COMPACTION_KEPT_MESSAGES, COMPACTION_PROMPT, MEMORY_PASS_PROMPT,
        MESSAGE_CONTEXT_SIZE, MESSAGE_DEBOUNCE_TIMEOUT, TYPING_DEBOUNCE_TIMEOUT,
        WATCHER_SESSION_TIMEOUT,
    },
    message::{self, AttachmentMode, QueuedMessage, discord_message_to_rig_message},
    tools,
};

/// Dual-timestamp activity tracker for proper debouncing
#[derive(Debug)]
struct ChannelActivity {
    /// When the last message occurred
    last_message: Option<Instant>,
    /// When the last typing event occurred
    last_typing: Option<Instant>,
}

impl ChannelActivity {
    fn new() -> Self {
        Self {
            last_message: None,
            last_typing: None,
        }
    }

    fn update_message(&mut self) {
        self.last_message = Some(Instant::now());
    }

    fn update_typing(&mut self) {
        self.last_typing = Some(Instant::now());
    }

    /// Calculate when we can next process messages
    /// We need both conditions satisfied:
    /// 1. Enough time passed since last message (`MESSAGE_DEBOUNCE_TIMEOUT`)
    /// 2. Enough time passed since last typing (`TYPING_DEBOUNCE_TIMEOUT`)
    fn next_processing_time(&self) -> Option<Instant> {
        let message_deadline = self.last_message.map(|t| t + MESSAGE_DEBOUNCE_TIMEOUT);
        let typing_deadline = self.last_typing.map(|t| t + TYPING_DEBOUNCE_TIMEOUT);

        match (message_deadline, typing_deadline) {
            (Some(m), Some(t)) => Some(m.max(t)),
            (Some(m), None) => Some(m),
            (None, Some(t)) => Some(t),
            (None, None) => None,
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum ChannelEvent {
    /// A new message has been received in the channel
    Message(QueuedMessage, Context),

    /// A typing event has been received in the channel
    Typing(UserId, Context),

    /// Request to immediately process the channel even if debounce timers haven't expired or there
    /// are no new messages. Useful for service startup when we want to process any awaiting
    /// messages right away. `addressed` sends it straight to the responder, as a message pinging
    /// the bot would; otherwise the watcher decides.
    ForceProcess { addressed: bool },
}

/// A channel message waiting for the debounce to hand it to the agents
struct PendingMessage {
    message: RigMessage,
    /// Where the fresh-session backfill stops: the API only fills in what is older than the queue
    id: MessageId,
    /// A user message that pings the bot or replies to it
    addresses_bot: bool,
    /// The bot's own message. A live responder already holds it as a `send_discord_message` call,
    /// so it only reaches the responder when seeding a fresh session. The watcher reads it always.
    from_bot: bool,
}

/// What a batch's runs authenticate with
struct RunAuth {
    /// The ChatGPT token; `None` on other backends
    grant: Option<Grant>,
}

enum RunError {
    /// ChatGPT has no usable sign-in any more, and the channel has been told
    SignedOut,
    Failed(PromptError),
}

struct ChannelState {
    activity: ChannelActivity,
    event_recv: UnboundedReceiver<ChannelEvent>,
    /// Writes the replies
    responder: Option<AgentSession>,
    /// Reads every message, decides when the responder runs, and keeps the memories. Never
    /// started in mention-only mode.
    watcher: Option<AgentSession>,
    /// When the watcher last got messages. Once the channel stays quiet for
    /// `WATCHER_SESSION_TIMEOUT`, the conversation is over: the watcher updates the memories from
    /// it and its session ends.
    watcher_last_input: Option<Instant>,
    /// ChatGPT has no usable sign-in, and the channel has said so. Runs wait for the sign-in to
    /// change, or for a new message asking for another try.
    awaiting_auth: bool,
    /// The responder lost its sign-in mid-run. The prompt it didn't get to is still the newest
    /// entry of its session, and the next processing runs it regardless of what the batch holds.
    unanswered: bool,
    /// ChatGPT sign-in changes; `None` on other backends
    auth_updates: Option<watch::Receiver<AuthState>>,
    /// When the responder last finished a run. Its session expires once the channel goes
    /// `AGENT_SESSION_TIMEOUT` without one, however much the users chat in between.
    last_run_finished: Option<Instant>,

    llm: LlmBackend,
    vectordb: Option<tools::SharedVectorClient>,
    firecrawl: Option<tools::Firecrawl>,

    // The latest discord context received from the event handler.
    // Note that each discord context is bound to a specific event and is destroyed after event
    // handler completes, so we should not rely on it being valid forever.
    discord_ctx: Context,
    bot_user_id: UserId,
    channel_id: ChannelId,
    // All guilds the bot is in
    guilds: Arc<scc::HashMap<serenity::model::id::GuildId, Guild>>,

    // Only respond when a message addresses the bot, and run no watcher: nothing costs a model
    // call until then. Incoming messages still queue up for context.
    discord_bot_mention_only: bool,

    /// Every message of the channel, the bot's own included, waits here until the debounce
    /// expires. It is the only way into the agents for anything newer than its oldest entry: a
    /// fresh session backfills strictly older messages from the API and appends the queue behind
    /// them, so nothing can reach an agent twice.
    message_queue: Vec<PendingMessage>,
}

impl ChannelState {
    /// Up to `count` channel messages from right before `before` (the newest when `None`), oldest
    /// first. Callers anchor on the oldest message they are about to add, which keeps a message
    /// that lands during the fetch out of the result; it reaches the agent through the queue
    /// instead. Attachments stay placeholders, the agent opens them on demand.
    #[instrument(skip(self))]
    async fn backfill_history(&self, before: Option<MessageId>, count: usize) -> Vec<RigMessage> {
        if count == 0 {
            return vec![];
        }

        // Fetch a full window rather than `count` so empty messages (stickers, bare embeds)
        // don't eat into what the agent gets to see
        let mut page = GetMessages::new().limit(MESSAGE_CONTEXT_SIZE as u8);
        if let Some(before) = before {
            page = page.before(before);
        }
        let messages = match self.channel_id.messages(&self.discord_ctx.http, page).await {
            Ok(messages) => messages,
            Err(e) => {
                tracing::error!(
                    ?e,
                    "Failed to backfill channel history; seeding the session from the queue alone"
                );
                return vec![];
            }
        };

        // Newest first from the API
        let mut history = Vec::with_capacity(count);
        for msg in messages
            .iter()
            .filter(|m| !m.content.trim().is_empty() || !m.attachments.is_empty())
            .take(count)
        {
            history.push(
                discord_message_to_rig_message(
                    msg,
                    self.bot_user_id,
                    &None,
                    AttachmentMode::Placeholder,
                )
                .await,
            );
        }
        history.reverse();
        history
    }

    /// The sign-in for the next runs. `None` when ChatGPT has none usable, which the channel is
    /// then told about and waits out.
    async fn authorize(&mut self) -> Option<RunAuth> {
        let LlmBackend::Chatgpt(auth) = &self.llm else {
            return Some(RunAuth { grant: None });
        };
        let auth = auth.clone();
        // Taken as seen before looking, so a change during the checks still wakes us
        if let Some(updates) = self.auth_updates.as_mut() {
            updates.mark_unchanged();
        }
        match auth.access().await {
            Ok(grant) => Some(RunAuth { grant: Some(grant) }),
            Err(unavailable) => {
                self.awaiting_auth = true;
                auth.report_unavailable(&self.discord_ctx.http, self.channel_id, &unavailable)
                    .await;
                None
            }
        }
    }

    /// Runs `session`, and when ChatGPT refuses a token that should have been good, refreshes it
    /// and gives the run one more go
    async fn run_session(
        &mut self,
        session: &mut AgentSession,
        turn: Turn,
        role: Role,
        run_auth: &mut RunAuth,
    ) -> Result<String, RunError> {
        let result = session.run(turn).await;
        let (Err(e), LlmBackend::Chatgpt(auth), Some(grant)) =
            (&result, &self.llm, &run_auth.grant)
        else {
            return result.map_err(RunError::Failed);
        };
        if !chatgpt::is_unauthorized(e) {
            return result.map_err(RunError::Failed);
        }

        tracing::warn!("ChatGPT rejected the access token; refreshing and retrying");
        let auth = auth.clone();
        match auth.recover_from_unauthorized(grant).await {
            Ok(grant) => match self.llm.model(role, Some(&grant)) {
                Ok(handle) => {
                    session.agent.set_model_handle(handle);
                    run_auth.grant = Some(grant);
                    session.run(turn).await.map_err(RunError::Failed)
                }
                Err(e) => {
                    tracing::error!(?e, "Failed to create the ChatGPT model");
                    result.map_err(RunError::Failed)
                }
            },
            Err(unavailable) => {
                self.awaiting_auth = true;
                auth.report_unavailable(&self.discord_ctx.http, self.channel_id, &unavailable)
                    .await;
                Err(RunError::SignedOut)
            }
        }
    }

    /// Summarizes `session` and starts it over from the summary, followed by the latest
    /// `COMPACTION_KEPT_MESSAGES` messages before `before`. Hands back the session as it was, or
    /// `None` when no summary came, leaving the caller to replace the session.
    async fn compact(
        &mut self,
        session: &mut AgentSession,
        role: Role,
        before: Option<MessageId>,
        run_auth: &mut RunAuth,
    ) -> Option<AgentSession> {
        tracing::info!(?role, "Compacting the session");
        session.add_messages(vec![RigMessage::user(COMPACTION_PROMPT)]);
        let summary = match self
            .run_session(session, Turn::TextOnly, role, run_auth)
            .await
        {
            Ok(summary) if !summary.trim().is_empty() => summary,
            Ok(_) => {
                tracing::warn!(?role, "Compaction produced no summary");
                return None;
            }
            Err(RunError::SignedOut) => return None,
            Err(RunError::Failed(e)) => {
                tracing::error!(?e, ?role, "Failed to summarize the session for compaction");
                return None;
            }
        };

        let mut recent = self
            .backfill_history(before, COMPACTION_KEPT_MESSAGES)
            .await;
        if matches!(role, Role::Watcher) {
            recent = recent.iter().map(message::observed).collect();
        }
        Some(session.start_over(&summary, recent))
    }

    /// Hands the queued messages to the agents. The watcher reads all of them; the responder
    /// runs when one addresses the bot or the watcher calls for it.
    async fn process(&mut self, forced_address: bool) {
        let Some(mut run_auth) = self.authorize().await else {
            return;
        };

        let batch: Vec<PendingMessage> = self.message_queue.drain(..).collect();
        let addressed = forced_address
            || std::mem::take(&mut self.unanswered)
            || batch.iter().any(|m| m.addresses_bot);

        let respond = if self.discord_bot_mention_only {
            addressed
        } else {
            self.watch(&batch, addressed, &mut run_auth).await
        };

        if respond {
            self.respond(batch, &mut run_auth).await;
        } else {
            self.pass_to_responder(batch);
        }
    }

    /// Feeds the batch to the watcher and asks whether the responder should run. A batch that
    /// addresses the bot runs it regardless, so the watcher only reads that one.
    async fn watch(
        &mut self,
        batch: &[PendingMessage],
        addressed: bool,
        run_auth: &mut RunAuth,
    ) -> bool {
        let before = batch.first().map(|m| m.id);
        let model = match self.llm.model(Role::Watcher, run_auth.grant.as_ref()) {
            Ok(model) => model,
            Err(e) => {
                tracing::error!(?e, "Failed to create the watcher model");
                return addressed;
            }
        };

        let mut watcher = match self.watcher.take() {
            Some(mut watcher) if watcher.needs_compaction() => {
                watcher.agent.set_model_handle(model.clone());
                match self
                    .compact(&mut watcher, Role::Watcher, before, run_auth)
                    .await
                {
                    Some(finished) => {
                        self.spawn_memory_pass(finished);
                        watcher
                    }
                    None => {
                        self.spawn_memory_pass(watcher);
                        self.new_watcher(model, before, batch.len()).await
                    }
                }
            }
            Some(mut watcher) => {
                watcher.agent.set_model_handle(model);
                watcher
            }
            None => self.new_watcher(model, before, batch.len()).await,
        };

        watcher.add_messages(
            batch
                .iter()
                .map(|m| message::observed(&m.message))
                .collect(),
        );
        self.watcher_last_input = Some(Instant::now());

        let respond = if addressed {
            true
        } else {
            match self
                .run_session(&mut watcher, Turn::TextOnly, Role::Watcher, run_auth)
                .await
            {
                Ok(answer) => {
                    let respond = wants_response(&answer);
                    tracing::info!(respond, answer = answer.trim(), "Watcher decided");
                    respond
                }
                Err(RunError::SignedOut) => false,
                Err(RunError::Failed(e)) => {
                    tracing::error!(?e, "Watcher run failed; staying quiet");
                    false
                }
            }
        };

        self.watcher = Some(watcher);
        respond
    }

    /// A watcher session seeded with the messages right before a batch of `batch_len` at `before`
    async fn new_watcher(
        &self,
        model: rig_agent::ModelHandle,
        before: Option<MessageId>,
        batch_len: usize,
    ) -> AgentSession {
        let history = self
            .backfill_history(before, MESSAGE_CONTEXT_SIZE.saturating_sub(batch_len))
            .await
            .iter()
            .map(message::observed)
            .collect();
        agent::create_watcher_session(
            &self.discord_ctx,
            self.channel_id,
            &self.llm,
            model,
            self.vectordb.clone(),
            history,
        )
    }

    /// Ends the watcher's session once the channel has gone quiet, updating the memories from it
    async fn retire_watcher(&mut self) {
        let Some(mut watcher) = self.watcher.take() else {
            return;
        };
        self.watcher_last_input = None;
        if self.vectordb.is_none() {
            return;
        }

        // Quiet means nobody else spoke, but the bot's last replies may still wait in the queue
        watcher.add_messages(
            self.message_queue
                .iter()
                .map(|m| message::observed(&m.message))
                .collect(),
        );
        // No sign-in prompt for this: nobody is waiting on it
        let grant = match &self.llm {
            LlmBackend::Chatgpt(auth) => match auth.access().await {
                Ok(grant) => Some(grant),
                Err(_) => {
                    tracing::warn!("Skipping the watcher's memory pass: ChatGPT has no sign-in");
                    return;
                }
            },
            LlmBackend::Gemini { .. } => None,
        };
        match self.llm.model(Role::Watcher, grant.as_ref()) {
            Ok(model) => {
                watcher.agent.set_model_handle(model);
                self.spawn_memory_pass(watcher);
            }
            Err(e) => tracing::error!(?e, "Failed to create the watcher model for its memory pass"),
        }
    }

    /// Updates the memories from a finished watcher session in the background: nothing waits on
    /// it, and the channel's next watcher can't write memories until its own session ends
    fn spawn_memory_pass(&self, mut session: AgentSession) {
        if self.vectordb.is_none() {
            return;
        }
        tokio::spawn(
            async move {
                session.add_messages(vec![RigMessage::user(MEMORY_PASS_PROMPT)]);
                if let Err(e) = session.run(Turn::Agentic).await {
                    tracing::error!(?e, "Watcher memory pass failed");
                }
            }
            .instrument(tracing::info_span!(
                "watcher_memory_pass",
                channel_id = self.channel_id.get()
            )),
        );
    }

    /// Runs the responder over the batch. Typing shows from here on, compaction included: someone
    /// is waiting on the reply now, which isn't so while only the watcher runs.
    async fn respond(&mut self, batch: Vec<PendingMessage>, run_auth: &mut RunAuth) {
        let typing = Typing::start(self.discord_ctx.http.clone(), self.channel_id);
        let before = batch.first().map(|m| m.id);
        let Some((mut responder, seeded)) =
            self.ready_responder(before, batch.len(), run_auth).await
        else {
            return;
        };

        // A live session already holds the bot's replies as tool calls; only a freshly seeded
        // one needs them to see what it said
        responder.add_messages(
            batch
                .into_iter()
                .filter(|m| seeded || !m.from_bot)
                .map(|m| m.message)
                .collect(),
        );

        let result = self
            .run_session(&mut responder, Turn::Agentic, Role::Responder, run_auth)
            .await;
        typing.stop();

        self.unanswered = matches!(result, Err(RunError::SignedOut));
        if let Err(RunError::Failed(e)) = &result {
            tracing::error!(?e, "Error executing agent session in channel main loop");
            if matches!(self.llm, LlmBackend::Chatgpt(_)) {
                chatgpt::report_run_failure(&self.discord_ctx.http, self.channel_id, e).await;
            }
        }
        self.responder = Some(responder);
        self.last_run_finished = Some(Instant::now());
    }

    /// The responder session for a batch of `batch_len` starting at `before`: the live one,
    /// compacted first when due, or a fresh one. The flag tells whether it was seeded just now.
    async fn ready_responder(
        &mut self,
        before: Option<MessageId>,
        batch_len: usize,
        run_auth: &mut RunAuth,
    ) -> Option<(AgentSession, bool)> {
        let model = match self.llm.model(Role::Responder, run_auth.grant.as_ref()) {
            Ok(model) => model,
            Err(e) => {
                tracing::error!(?e, "Failed to create the responder model");
                return None;
            }
        };

        if let Some(mut responder) = self.live_responder() {
            // A refreshed ChatGPT token only reaches a live session this way
            responder.agent.set_model_handle(model.clone());
            if !responder.needs_compaction() {
                return Some((responder, false));
            }
            if self
                .compact(&mut responder, Role::Responder, before, run_auth)
                .await
                .is_some()
            {
                return Some((responder, true));
            }
        }

        let history = self
            .backfill_history(before, MESSAGE_CONTEXT_SIZE.saturating_sub(batch_len))
            .await;
        let memories = match self.vectordb.clone() {
            None => Memories::Off,
            Some(client) if self.discord_bot_mention_only => Memories::Keep(client),
            Some(client) => Memories::Recall(client),
        };
        match agent::create_responder_session(
            &self.discord_ctx,
            self.channel_id,
            &self.llm,
            model,
            memories,
            self.firecrawl.clone(),
            history,
        ) {
            Ok(session) => Some((session, true)),
            Err(e) => {
                tracing::error!(?e, "Failed to create agent session for channel");
                None
            }
        }
    }

    /// The responder's session, unless it has gone unused long enough to expire. Idle since the
    /// bot last ran, not since the last message: the messages keep coming while nobody involves
    /// the bot.
    fn live_responder(&mut self) -> Option<AgentSession> {
        let responder = self.responder.take()?;
        let expired = self
            .last_run_finished
            .is_some_and(|t| t.elapsed() > AGENT_SESSION_TIMEOUT);
        (!expired).then_some(responder)
    }

    /// Keeps a live responder up to date with a batch it didn't run for, so its next run has no
    /// hole where these messages were. Appending leaves its cached prefix intact.
    fn pass_to_responder(&mut self, batch: Vec<PendingMessage>) {
        self.responder = self.live_responder();
        if let Some(responder) = self.responder.as_mut() {
            responder.add_messages(
                batch
                    .into_iter()
                    .filter(|m| !m.from_bot)
                    .map(|m| m.message)
                    .collect(),
            );
        }
    }

    async fn main_loop(mut self) {
        loop {
            // Whether anything queued deserves a run: the bot's own messages never do, and in
            // mention-only mode neither do user messages that don't address it. Checked over the
            // whole queue, not just the newest entry, because users keep typing after the mention.
            let has_pending_prompt = self
                .message_queue
                .iter()
                .any(|m| !m.from_bot && (!self.discord_bot_mention_only || m.addresses_bot));
            let timer = if has_pending_prompt && !self.awaiting_auth {
                tokio::time::sleep_until(
                    self.activity
                        .next_processing_time()
                        .unwrap_or_else(Instant::now)
                        .into(),
                )
            } else {
                tokio::time::sleep(Duration::from_secs(u64::MAX))
            };
            let watcher_idle = sleep_until_some(
                self.watcher_last_input
                    .filter(|_| self.watcher.is_some())
                    .map(|t| t + WATCHER_SESSION_TIMEOUT),
            );

            let wake = tokio::select! {
                event = self.event_recv.next() => {
                    if let Some(event) = event {
                        match event {
                            ChannelEvent::Message(msg, ctx) => {
                                self.discord_ctx = ctx;
                                let from_bot = msg.message.author.id == self.bot_user_id;
                                // The bot's text-less posts are its ChatGPT notices, not conversation
                                if from_bot && msg.message.content.trim().is_empty() {
                                    continue;
                                }
                                if !from_bot {
                                    // The bot's own replies don't hold the debounce open
                                    self.activity.update_message();
                                }

                                let guild = self
                                    .channel_id
                                    .to_channel(self.discord_ctx.http.clone())
                                    .await
                                    .inspect_err(|e| {
                                        tracing::error!(?e, "Failed to fetch channel for guild ID lookup");
                                    })
                                    .ok()
                                    .and_then(|c| c.guild())
                                    .and_then(|g| self.guilds.get_sync(&g.guild_id));

                                let addresses_bot =
                                    !from_bot && message::addresses(&msg.message, self.bot_user_id);
                                // Someone asking again gets another try, e.g. a new sign-in code
                                // once the last one expired
                                if !from_bot && (!self.discord_bot_mention_only || addresses_bot) {
                                    self.awaiting_auth = false;
                                }

                                let message = discord_message_to_rig_message(
                                    &msg.message,
                                    self.bot_user_id,
                                    &guild,
                                    AttachmentMode::Inline,
                                ).await;

                                self.message_queue.push(PendingMessage {
                                    message,
                                    id: msg.message.id,
                                    addresses_bot,
                                    from_bot,
                                });
                                // Keep only the newest window: in mention-only mode the backlog
                                // would otherwise grow without bound
                                if self.message_queue.len() > MESSAGE_CONTEXT_SIZE {
                                    self.message_queue.drain(0..self.message_queue.len() - MESSAGE_CONTEXT_SIZE);
                                }

                                Wake::Idle
                            }
                            ChannelEvent::Typing(uid, ctx) => {
                                self.discord_ctx = ctx;
                                if uid == self.bot_user_id {
                                    // Ignore typing events from the bot itself
                                    continue;
                                }
                                self.activity.update_typing();
                                Wake::Idle
                            }
                            ChannelEvent::ForceProcess { addressed } => Wake::Process { addressed },
                        }
                    }
                    else {
                        tracing::info!("Channel event receiver closed, exiting main loop");
                        break;
                    }
                }
                // The timer is only armed while something queued deserves a run
                _ = timer => Wake::Process { addressed: false },
                // Signed in, refreshed, or signed out for good: worth another look either way
                _ = auth_changed(self.auth_updates.as_mut()), if self.awaiting_auth => {
                    self.awaiting_auth = false;
                    Wake::Process { addressed: false }
                }
                _ = watcher_idle => Wake::WatcherIdle,
            };

            match wake {
                Wake::Idle => {}
                Wake::Process { addressed } => {
                    self.process(addressed)
                        .instrument(tracing::span!(
                            tracing::Level::INFO,
                            "process_discord_message"
                        ))
                        .await;
                }
                Wake::WatcherIdle => self.retire_watcher().await,
            }
        }
    }
}

/// What woke the channel's main loop up
enum Wake {
    /// Nothing to run yet
    Idle,
    Process {
        addressed: bool,
    },
    /// The watcher went `WATCHER_SESSION_TIMEOUT` without new messages
    WatcherIdle,
}

/// Whether the watcher's answer calls for the responder: it opens with RESPOND or PASS
fn wants_response(answer: &str) -> bool {
    answer
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .get(..7)
        .is_some_and(|word| word.eq_ignore_ascii_case("respond"))
}

pub struct ChannelHandle {
    event_send: UnboundedSender<ChannelEvent>,

    #[allow(dead_code, reason = "TODO: handle shutdown, restarts, etc.")]
    main_loop_handle: tokio::task::JoinHandle<()>,
}

impl ChannelHandle {
    pub fn new(
        discord_ctx: Context,
        channel_id: ChannelId,
        llm: LlmBackend,
        vectordb: Option<tools::SharedVectorClient>,
        firecrawl: Option<tools::Firecrawl>,
        discord_bot_mention_only: bool,
        guilds: Arc<scc::HashMap<serenity::model::id::GuildId, Guild>>,
    ) -> Self {
        let (event_send, event_recv) = futures::channel::mpsc::unbounded();

        let bot_user_id = discord_ctx.cache.current_user().id;

        let state = ChannelState {
            activity: ChannelActivity::new(),
            event_recv,
            responder: None,
            watcher: None,
            watcher_last_input: None,
            awaiting_auth: false,
            unanswered: false,
            auth_updates: match &llm {
                LlmBackend::Chatgpt(auth) => Some(auth.subscribe()),
                LlmBackend::Gemini { .. } => None,
            },
            last_run_finished: None,
            llm,
            vectordb,
            firecrawl,
            bot_user_id,
            discord_ctx: discord_ctx.clone(),
            message_queue: vec![],
            channel_id,
            discord_bot_mention_only,
            guilds,
        };

        let main_loop_handle = tokio::spawn(state.main_loop().instrument(tracing::info_span!(
            "channel_main_loop",
            channel_id = channel_id.get(),
            discord_bot_mention_only
        )));

        Self {
            event_send,
            main_loop_handle,
        }
    }

    pub async fn send_event(&mut self, event: ChannelEvent) -> Result<(), eyre::Error> {
        self.event_send
            .send(event)
            .await
            .map_err(|e| eyre::eyre!(e))
    }
}

/// Resolves when the ChatGPT sign-in changes, and never without one to watch
async fn auth_changed(updates: Option<&mut watch::Receiver<AuthState>>) {
    if let Some(updates) = updates
        && updates.changed().await.is_ok()
    {
        return;
    }
    std::future::pending().await
}

/// Resolves at `deadline`, and never without one
async fn sleep_until_some(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::wants_response;

    #[test]
    fn watcher_answers_open_with_the_decision() {
        for answer in [
            "RESPOND: nobody answered the question",
            "`RESPOND` — bait worth taking",
            "respond",
            "  **Respond** they're asking the bot",
        ] {
            assert!(wants_response(answer), "{answer}");
        }
        for answer in ["PASS: chatter", "", "RESPONSE", "I'd say RESPOND", "rés"] {
            assert!(!wants_response(answer), "{answer}");
        }
    }
}
