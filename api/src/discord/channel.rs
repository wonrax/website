use std::time::{Duration, Instant};

use futures::{
    SinkExt as _, StreamExt,
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
};
use rig::message::Message as RigMessage;
use serenity::all::{ChannelId, Context, Message, Typing, UserId};

use crate::discord::{
    agent::{self, AgentSession},
    constants::{
        AGENT_SESSION_TIMEOUT, MESSAGE_CONTEXT_SIZE, MESSAGE_DEBOUNCE_TIMEOUT,
        TYPING_DEBOUNCE_TIMEOUT,
    },
    message::{QueuedMessage, discord_message_to_rig_message, queued_messages_to_rig_messages},
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
    /// 1. Enough time passed since last message (MESSAGE_DEBOUNCE_TIMEOUT_MS)
    /// 2. Enough time passed since last typing (TYPING_DEBOUNCE_TIMEOUT_MS)
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
    Message(QueuedMessage),

    /// A typing event has been received in the channel
    Typing(UserId),

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
    // Note that each discord context is binded to a specific event and is destroyed after event
    // handler completes, so we should not rely on it being valid forever.
    discord_ctx: Context,
    bot_user_id: serenity::model::id::UserId,
    channel_id: ChannelId,

    // Only process messages when a message mentions the bot, otherwise still queue incoming
    // messages.
    discord_bot_mention_only: bool,

    // Queue the incoming messages and only add them to the agent when debounced. This is because
    // the AgentSession::add_messages handle context trimming which retains at most N new messages.
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

        let mut rig_messages = Vec::new();

        // Process messages chronologically (oldest first)
        for msg in fetched_messages.into_iter().rev() {
            let rig_message = discord_message_to_rig_message(&msg, self.bot_user_id);
            rig_messages.push(rig_message);
        }

        rig_messages
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
    ) -> Self {
        let (event_send, event_recv) = futures::channel::mpsc::unbounded();

        let bot_user_id = discord_ctx.cache.current_user().id;

        let mut state = ChannelState {
            activity: ChannelActivity::new(),
            event_recv,
            agent: None,
            bot_user_id,
            discord_ctx: discord_ctx.clone(),
            message_queue: vec![],
            channel_id,
            discord_bot_mention_only,
        };

        let main_loop_handle = tokio::spawn(async move {
            loop {
                let timer = if !state.message_queue.is_empty()
                    && (!state.discord_bot_mention_only
                        || state
                            .message_queue
                            .iter()
                            // Check if any queued message mentions the bot and not just the last
                            // one because the user can send immediate subsequent messages after mentioning
                            // the bot
                            .any(|m| m.message.mentions_user_id(state.bot_user_id)))
                {
                    tokio::time::sleep_until(
                        state
                            .activity
                            .next_processing_time()
                            .unwrap_or_else(Instant::now)
                            .into(),
                    )
                } else {
                    tokio::time::sleep(Duration::from_secs(u64::MAX))
                };

                let (timer_expired, force_process) = tokio::select! {
                    event = state.event_recv.next() => {
                        if let Some(event) = event {
                            match event {
                                ChannelEvent::Message(msg) => {
                                    if msg.message.author.id == state.bot_user_id {
                                        // No need to process messages from the bot itself, it will
                                        // be represented as a tool call in the conversation
                                        // history, so duplicating it here would be redundant.
                                        continue;
                                    }
                                    state.activity.update_message();
                                    state.message_queue.push(msg);
                                    (false, false)
                                }
                                ChannelEvent::Typing(uid) => {
                                    if uid == state.bot_user_id {
                                        // Ignore typing events from the bot itself
                                        continue;
                                    }
                                    state.activity.update_typing();
                                    (false, false)
                                }
                                ChannelEvent::ForceProcess => {
                                    (false, true)
                                }
                            }
                        }
                        else {
                            tracing::info!(
                                ?channel_id,
                                "Channel event receiver closed, exiting main loop"
                            );
                            break;
                        }
                    }
                    _ = timer => (true, false),
                };

                if !force_process {
                    if !timer_expired {
                        continue;
                    }

                    if state.message_queue.is_empty() {
                        continue;
                    }
                }

                let _typing = Typing::start(state.discord_ctx.http.clone(), channel_id);

                if state
                    .activity
                    .last_activity()
                    .is_some_and(|t| t.elapsed() > AGENT_SESSION_TIMEOUT)
                {
                    state.agent = None;
                }

                if state.agent.is_none() {
                    state.agent = Some(agent::create_agent_session(
                        &state.discord_ctx,
                        channel_id,
                        &openai_api_key,
                        shared_vectordb_client.clone(),
                        state.build_conversation_history().await,
                    ));
                }

                let agent = state.agent.as_mut().unwrap();

                agent.add_messages(queued_messages_to_rig_messages(
                    state
                        .message_queue
                        // Cap to MESSAGE_CONTEXT_SIZE most recent messages because if
                        // discord_bot_mention_only is true, we may have a large backlog
                        .split_at(if state.message_queue.len() > MESSAGE_CONTEXT_SIZE {
                            state.message_queue.len() - MESSAGE_CONTEXT_SIZE
                        } else {
                            0
                        })
                        .1,
                    Some(state.bot_user_id),
                ));

                state.message_queue.clear();

                let _ = agent.execute_agent_multi_turn().await.inspect_err(|e| {
                    tracing::error!(?e, "Error executing agent session in channel main loop",);
                });
            }
        });

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
