use crate::discord::{
    agent::{create_agent_session, execute_agent_interaction, AgentSession},
    constants::{MESSAGE_CONTEXT_SIZE, MESSAGE_DEBOUNCE_TIMEOUT_MS, WHITELIST_CHANNELS},
    message::{queued_messages_to_rig_messages, QueuedMessage},
};
use serenity::all::{ChannelId, Message, Ready};
use serenity::async_trait;
use serenity::prelude::*;
use std::{
    collections::HashMap,
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

/// The agentic message handler using rig with multi-turn support
async fn handle_message_batch(
    ctx: Context,
    messages: Vec<QueuedMessage>,
    handler: Arc<Handler>,
) -> Result<(), eyre::Error> {
    if messages.is_empty() {
        return Ok(());
    }

    let channel_id = messages[0].message.channel_id;

    // Ensure we have an agent session for this channel with enough context to include recent messages
    let context_size = std::cmp::max(MESSAGE_CONTEXT_SIZE, messages.len());
    handler
        .get_or_create_agent_session(&ctx, channel_id, context_size)
        .await?;

    // Add the new messages to the agent's conversation and let it naturally respond
    {
        let mut sessions = handler.agent_sessions.lock().await;
        if let Some(session) = sessions.get_mut(&channel_id) {
            // Convert the batch of new messages to RigMessage format
            let new_messages = queued_messages_to_rig_messages(&messages);

            // Add new messages to the conversation history
            session.add_messages(new_messages);

            // Execute agent interaction with multi-turn reasoning
            let _ = execute_agent_interaction(session, messages.len(), channel_id).await;
        }
    }

    Ok(())
}

pub struct Handler {
    pub message_queue: Arc<Mutex<HashMap<ChannelId, Vec<QueuedMessage>>>>,
    pub openai_api_key: String,
    /// Track pending timers for each channel to avoid duplicate processing
    pub pending_timers: Arc<Mutex<HashMap<ChannelId, tokio::task::JoinHandle<()>>>>,
    /// Persistent agent sessions per channel for multi-turn conversations
    pub agent_sessions: Arc<Mutex<HashMap<ChannelId, AgentSession>>>,
}

impl Handler {
    pub fn new(openai_api_key: String) -> Self {
        Self {
            message_queue: Arc::new(Mutex::new(HashMap::new())),
            openai_api_key,
            pending_timers: Arc::new(Mutex::new(HashMap::new())),
            agent_sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a new Handler instance that shares the same underlying data
    fn clone_for_task(&self) -> Arc<Self> {
        Arc::new(Handler {
            message_queue: self.message_queue.clone(),
            openai_api_key: self.openai_api_key.clone(),
            pending_timers: self.pending_timers.clone(),
            agent_sessions: self.agent_sessions.clone(),
        })
    }

    /// Get or create an agent session for a channel
    async fn get_or_create_agent_session(
        &self,
        ctx: &Context,
        channel_id: ChannelId,
        context_size: usize,
    ) -> Result<(), eyre::Error> {
        let mut sessions = self.agent_sessions.lock().await;

        // Check if session exists and is not expired
        let needs_new_session = sessions
            .get(&channel_id)
            .is_none_or(|session| session.is_expired());

        if needs_new_session {
            let session = create_agent_session(ctx, channel_id, context_size, &self.openai_api_key).await?;
            sessions.insert(channel_id, session);
        } else {
            // Update activity timestamp
            if let Some(session) = sessions.get_mut(&channel_id) {
                session.update_activity();
            }
        }

        Ok(())
    }

    /// Schedule processing for a channel after the debounce timeout
    async fn schedule_channel_processing(&self, ctx: Context, channel_id: ChannelId) {
        // Cancel any existing timer for this channel
        {
            let mut timers = self.pending_timers.lock().await;
            if let Some(handle) = timers.remove(&channel_id) {
                handle.abort();
            }
        }

        // Create new timer
        let message_queue = self.message_queue.clone();
        let pending_timers = self.pending_timers.clone();
        let handler = self.clone_for_task();

        let handle = tokio::spawn(async move {
            // Wait for the debounce timeout
            tokio::time::sleep(Duration::from_millis(MESSAGE_DEBOUNCE_TIMEOUT_MS)).await;

            // Remove this timer from pending list
            {
                let mut timers = pending_timers.lock().await;
                timers.remove(&channel_id);
            }

            // Process the messages for this channel
            let messages = {
                let mut queue = message_queue.lock().await;
                queue.remove(&channel_id).unwrap_or_default()
            };

            if !messages.is_empty() {
                if let Err(e) = handle_message_batch(ctx, messages, handler).await {
                    tracing::error!(
                        "Error processing message batch for channel {}: {}",
                        channel_id,
                        e
                    );
                }
            }
        });

        // Store the timer handle
        {
            let mut timers = self.pending_timers.lock().await;
            timers.insert(channel_id, handle);
        }
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if !WHITELIST_CHANNELS.contains(&msg.channel_id.get()) {
            return;
        }

        // Ignore messages from bots
        if msg.author.bot {
            return;
        }

        // Add to queue
        let queued_msg = QueuedMessage { message: msg };

        let channel_id = queued_msg.message.channel_id;
        {
            let mut queue = self.message_queue.lock().await;
            let channel_messages = queue.entry(channel_id).or_insert_with(Vec::new);
            channel_messages.push(queued_msg);

            // Limit queue size per channel
            if channel_messages.len() > 10 {
                channel_messages.remove(0);
            }
        }

        // Schedule processing for this channel (this will reset the timer if one exists)
        self.schedule_channel_processing(ctx, channel_id).await;
    }

    async fn ready(&self, _: Context, ready: Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);
    }
}
