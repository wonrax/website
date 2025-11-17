use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use futures::{
    SinkExt as _, StreamExt,
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
};
use rig::message::Message as RigMessage;
use serenity::all::{ChannelId, Context, Message, Typing, UserId};
use tracing::Instrument as _;

use crate::discord::{
    agent::{self, AgentSession},
    bot::Guild,
    constants::{
        AGENT_SESSION_TIMEOUT, MESSAGE_CONTEXT_SIZE, MESSAGE_DEBOUNCE_TIMEOUT,
        TYPING_DEBOUNCE_TIMEOUT,
    },
    message::{QueuedMessage, discord_message_to_rig_message, format_message_content_with_bot_id},
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

    fn last_activity(&self) -> Option<Instant> {
        match (self.last_message, self.last_typing) {
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

struct ChannelState {
    activity: ChannelActivity,
    event_recv: UnboundedReceiver<ChannelEvent>,
    agent: Option<AgentSession>,

    // The latest discord context received from the event handler.
    // Note that each discord context is bound to a specific event and is destroyed after event
    // handler completes, so we should not rely on it being valid forever.
    discord_ctx: Context,
    bot_user_id: serenity::model::id::UserId,
    channel_id: ChannelId,
    // All guilds the bot is in
    guilds: Arc<scc::HashMap<serenity::model::id::GuildId, Guild>>,

    // Only process messages when a message mentions the bot, otherwise still queue incoming
    // messages.
    discord_bot_mention_only: bool,

    // Queue the incoming messages and only add them to the agent when debounced. This is because
    // the AgentSession::add_messages handles context trimming which retains at most N new messages.
    // We want to avoid trimming unhandled messages if called repeatedly.
    message_queue: Vec<QueuedMessage>,
}

impl ChannelState {
    /// Build conversation history for agent context
    async fn build_conversation_history(&self) -> Vec<RigMessage> {
        let fetched_messages: Vec<Message> = self
            .channel_id
            .messages_iter(&self.discord_ctx.http)
            .filter_map(|m| async {
                m.ok()
                    .filter(|msg| !msg.content.trim().is_empty() || !msg.attachments.is_empty())
            })
            .take(MESSAGE_CONTEXT_SIZE)
            .collect()
            .await;

        fetched_messages
            .into_iter()
            .rev()
            .map(|m| discord_message_to_rig_message(&m, self.bot_user_id, &None))
            .collect()
    }

    async fn main_loop(
        mut self,
        shared_vectordb_client: Option<tools::SharedVectorClient>,
        openai_api_key: String,
    ) {
        loop {
            let timer = if !self.message_queue.is_empty()
                && (!self.discord_bot_mention_only
                    || self
                        .message_queue
                        .iter()
                        // Check if any queued message mentions the bot and not just the last
                        // one because the user can send immediate subsequent messages after mentioning
                        // the bot
                        .any(|m| m.message.mentions_user_id(self.bot_user_id)))
            {
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
                                if msg.message.author.id == self.bot_user_id {
                                    // No need to process messages from the bot itself, it will
                                    // be represented as a tool call in the conversation
                                    // history, so duplicating it here would be redundant.
                                    continue;
                                }
                                self.activity.update_message();
                                self.message_queue.push(msg);
                                (false, false)
                            }
                            ChannelEvent::Typing(uid,ctx) => {
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

            if !force_process {
                if !timer_expired {
                    continue;
                }

                if self.message_queue.is_empty() {
                    continue;
                }
            }

            let _typing = Typing::start(self.discord_ctx.http.clone(), self.channel_id);

            if self
                .activity
                .last_activity()
                .is_some_and(|t| t.elapsed() > AGENT_SESSION_TIMEOUT)
            {
                self.agent = None;
            }

            if self.agent.is_none() {
                self.agent = Some(agent::create_agent_session(
                    &self.discord_ctx,
                    self.channel_id,
                    &openai_api_key,
                    shared_vectordb_client.clone(),
                    self.build_conversation_history().await,
                ));
            }

            let agent = self.agent.as_mut().unwrap();

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

            agent.add_messages(
                self.message_queue
                    // Cap to MESSAGE_CONTEXT_SIZE most recent messages because if
                    // discord_bot_mention_only is true, we may have a large backlog
                    .split_at(if self.message_queue.len() > MESSAGE_CONTEXT_SIZE {
                        self.message_queue.len() - MESSAGE_CONTEXT_SIZE
                    } else {
                        0
                    })
                    .1
                    .iter()
                    .map(|queued_msg| {
                        let content = format_message_content_with_bot_id(
                            &queued_msg.message,
                            Some(self.bot_user_id),
                            &guild,
                        );
                        RigMessage::user(content)
                    })
                    .collect(),
            );

            self.message_queue.clear();

            let _ = agent.execute_agent_multi_turn().await.inspect_err(|e| {
                tracing::error!(?e, "Error executing agent session in channel main loop",);
            });
        }
    }
}

pub struct ChannelHandle {
    event_send: UnboundedSender<ChannelEvent>,
    main_loop_handle: tokio::task::JoinHandle<()>,
}

impl ChannelHandle {
    pub fn new(
        discord_ctx: Context,
        channel_id: ChannelId,
        openai_api_key: String,
        shared_vectordb_client: Option<tools::SharedVectorClient>,
        discord_bot_mention_only: bool,
        guilds: Arc<scc::HashMap<serenity::model::id::GuildId, Guild>>,
    ) -> Self {
        let (event_send, event_recv) = futures::channel::mpsc::unbounded();

        let bot_user_id = discord_ctx.cache.current_user().id;

        let state = ChannelState {
            activity: ChannelActivity::new(),
            event_recv,
            agent: None,
            bot_user_id,
            discord_ctx: discord_ctx.clone(),
            message_queue: vec![],
            channel_id,
            discord_bot_mention_only,
            guilds,
        };

        let main_loop_handle = tokio::spawn(
            state
                .main_loop(shared_vectordb_client, openai_api_key)
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
