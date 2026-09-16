use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use futures::{
    SinkExt as _, StreamExt,
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
};
use rig::message::Message as RigMessage;
use serenity::all::{ChannelId, Context, GetMessages, MessageId, Typing, UserId};
use tracing::{Instrument as _, instrument};

use crate::discord::{
    agent::{self, AgentSession},
    bot::Guild,
    constants::{
        AGENT_SESSION_TIMEOUT, MESSAGE_CONTEXT_SIZE, MESSAGE_DEBOUNCE_TIMEOUT,
        TYPING_DEBOUNCE_TIMEOUT,
    },
    message::{AttachmentMode, QueuedMessage, discord_message_to_rig_message},
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

    /// Request to immediately run the agent loop even if debounce timers haven't expired or there
    /// are no new messages. Useful for service startup when we want to process any awaiting
    /// messages right away.
    ForceProcess,
}

/// A channel message waiting for the debounce to hand it to the agent
struct PendingMessage {
    message: RigMessage,
    /// Where the fresh-session backfill stops: the API only fills in what is older than the queue
    id: MessageId,
    mentions_bot: bool,
    /// The bot's own message. A live session already holds it as a `send_discord_message` call,
    /// so it only reaches the agent when seeding a fresh session.
    from_bot: bool,
}

struct ChannelState {
    activity: ChannelActivity,
    event_recv: UnboundedReceiver<ChannelEvent>,
    agent: Option<AgentSession>,
    /// When the agent last finished a run. The session expires once the channel goes
    /// `AGENT_SESSION_TIMEOUT` without one, however much the users chat in between.
    last_run_finished: Option<Instant>,

    // The latest discord context received from the event handler.
    // Note that each discord context is bound to a specific event and is destroyed after event
    // handler completes, so we should not rely on it being valid forever.
    discord_ctx: Context,
    bot_user_id: UserId,
    channel_id: ChannelId,
    // All guilds the bot is in
    guilds: Arc<scc::HashMap<serenity::model::id::GuildId, Guild>>,

    // Only process messages when a message mentions the bot, otherwise still queue incoming
    // messages.
    discord_bot_mention_only: bool,

    /// Every message of the channel, the bot's own included, waits here until the debounce
    /// expires. It is the only way into the agent for anything newer than its oldest entry: a
    /// fresh session backfills strictly older messages from the API and appends the queue behind
    /// them, so nothing can reach the agent twice.
    message_queue: Vec<PendingMessage>,
}

impl ChannelState {
    /// Channel messages older than the queue, oldest first, enough to fill `MESSAGE_CONTEXT_SIZE`
    /// together with it. Anchoring on the oldest queued message keeps a message that lands during
    /// the fetch out of the result; it reaches the agent through the queue instead. Attachments
    /// stay placeholders, the agent opens them on demand.
    #[instrument(skip(self))]
    async fn backfill_history(&self) -> Vec<RigMessage> {
        let needed = MESSAGE_CONTEXT_SIZE.saturating_sub(self.message_queue.len());
        if needed == 0 {
            return vec![];
        }

        // Fetch a full window rather than `needed` so empty messages (stickers, bare embeds)
        // don't eat into what the agent gets to see
        let mut page = GetMessages::new().limit(MESSAGE_CONTEXT_SIZE as u8);
        if let Some(oldest) = self.message_queue.first() {
            page = page.before(oldest.id);
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
        let mut history = Vec::with_capacity(needed);
        for msg in messages
            .iter()
            .filter(|m| !m.content.trim().is_empty() || !m.attachments.is_empty())
            .take(needed)
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

    async fn main_loop(
        mut self,
        shared_vectordb_client: Option<tools::SharedVectorClient>,
        firecrawl: Option<tools::Firecrawl>,
        openai_api_key: String,
    ) {
        loop {
            // Whether anything queued deserves a run: the bot's own messages never do, and in
            // mention-only mode neither do user messages that don't mention it. Checked over the
            // whole queue, not just the newest entry, because users keep typing after the mention.
            let has_pending_prompt = self
                .message_queue
                .iter()
                .any(|m| !m.from_bot && (!self.discord_bot_mention_only || m.mentions_bot));
            let timer = if has_pending_prompt {
                tokio::time::sleep_until(
                    self.activity
                        .next_processing_time()
                        .unwrap_or_else(Instant::now)
                        .into(),
                )
            } else {
                tokio::time::sleep(Duration::from_secs(u64::MAX))
            };

            let (timer_expired, force_process) = tokio::select! {
                event = self.event_recv.next() => {
                    if let Some(event) = event {
                        match event {
                            ChannelEvent::Message(msg, ctx) => {
                                self.discord_ctx = ctx;
                                let from_bot = msg.message.author.id == self.bot_user_id;
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

                                let mentions_bot =
                                    !from_bot && msg.message.mentions_user_id(self.bot_user_id);

                                let message = discord_message_to_rig_message(
                                    &msg.message,
                                    self.bot_user_id,
                                    &guild,
                                    AttachmentMode::Inline,
                                ).await;

                                self.message_queue.push(PendingMessage {
                                    message,
                                    id: msg.message.id,
                                    mentions_bot,
                                    from_bot,
                                });
                                // Keep only the newest window: in mention-only mode the backlog
                                // would otherwise grow without bound
                                if self.message_queue.len() > MESSAGE_CONTEXT_SIZE {
                                    self.message_queue.drain(0..self.message_queue.len() - MESSAGE_CONTEXT_SIZE);
                                }

                                (false, false)
                            }
                            ChannelEvent::Typing(uid, ctx) => {
                                self.discord_ctx = ctx;
                                if uid == self.bot_user_id {
                                    // Ignore typing events from the bot itself
                                    continue;
                                }
                                self.activity.update_typing();
                                (false, false)
                            }
                            ChannelEvent::ForceProcess => {
                                (false, true)
                            }
                        }
                    }
                    else {
                        tracing::info!("Channel event receiver closed, exiting main loop");
                        break;
                    }
                }
                _ = timer => (true, false),
            };

            // The timer is only armed while something queued deserves a run, so its expiry is
            // enough on its own
            if !force_process && !timer_expired {
                continue;
            }

            let span = tracing::span!(tracing::Level::INFO, "process_discord_message");
            let ran = async {
                let _typing = Typing::start(self.discord_ctx.http.clone(), self.channel_id);

                // Idle since the bot last ran, not since the last message: in mention-only mode
                // the messages keep coming while nobody involves the bot
                if self
                    .last_run_finished
                    .is_some_and(|t| t.elapsed() > AGENT_SESSION_TIMEOUT)
                {
                    self.agent = None;
                }

                let fresh_session = self.agent.is_none();
                let agent = match self.agent.as_mut() {
                    Some(agent) => agent,
                    None => {
                        let history = self.backfill_history().await;
                        match agent::create_agent_session(
                            &self.discord_ctx,
                            self.channel_id,
                            &openai_api_key,
                            shared_vectordb_client.clone(),
                            firecrawl.clone(),
                            history,
                        ) {
                            Ok(session) => self.agent.insert(session),
                            Err(e) => {
                                tracing::error!(?e, "Failed to create agent session for channel");
                                return false;
                            }
                        }
                    }
                };

                // A live session already holds the bot's replies as tool calls; only a fresh one
                // needs them to see what it said
                agent.add_messages(
                    self.message_queue
                        .drain(..)
                        .filter(|m| fresh_session || !m.from_bot)
                        .map(|m| m.message)
                        .collect(),
                );

                let _ = agent.execute_agent_multi_turn().await.inspect_err(|e| {
                    tracing::error!(?e, "Error executing agent session in channel main loop",);
                });
                self.last_run_finished = Some(Instant::now());
                true
            }
            .instrument(span)
            .await;
            if !ran {
                continue;
            }
        }
    }
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
        openai_api_key: String,
        shared_vectordb_client: Option<tools::SharedVectorClient>,
        firecrawl: Option<tools::Firecrawl>,
        discord_bot_mention_only: bool,
        guilds: Arc<scc::HashMap<serenity::model::id::GuildId, Guild>>,
    ) -> Self {
        let (event_send, event_recv) = futures::channel::mpsc::unbounded();

        let bot_user_id = discord_ctx.cache.current_user().id;

        let state = ChannelState {
            activity: ChannelActivity::new(),
            event_recv,
            agent: None,
            last_run_finished: None,
            bot_user_id,
            discord_ctx: discord_ctx.clone(),
            message_queue: vec![],
            channel_id,
            discord_bot_mention_only,
            guilds,
        };

        let main_loop_handle = tokio::spawn(
            state
                .main_loop(shared_vectordb_client, firecrawl, openai_api_key)
                .instrument(tracing::info_span!(
                    "channel_main_loop",
                    channel_id = channel_id.get(),
                    discord_bot_mention_only
                )),
        );

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
