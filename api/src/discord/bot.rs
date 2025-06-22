// Discord Bot with Dual-Timestamp Activity Debouncing
//
// This implementation uses dual timestamp tracking to properly handle both message
// and typing debounce requirements with optimal timing.
//
// Requirements:
// 1. Message Debounce: Wait X ms after last message
// 2. Typing Debounce: Wait Y ms after last typing
// 3. Priority Handling: Always wait until BOTH conditions are satisfied
// 4. Efficiency: Sleep only as long as necessary
//
// How it works:
// - Messages call record_message_activity() which updates last_message timestamp
// - Typing calls record_typing_activity() which updates last_typing timestamp
// - Each activity reschedules the timer to wake at max(message_deadline, typing_deadline)
// - Timer sleeps exactly until both debounce conditions are satisfied
// - No premature wake-ups, no permanent skips, optimal efficiency
//
// Benefits:
// 1. Correct dual-timeout handling - respects both message and typing requirements
// 2. Dynamic sleep calculation - never waits longer than necessary
// 3. Proper priority handling - later events extend timeouts appropriately
// 4. Single timer per channel - maintains efficiency and simplicity
// 5. No race conditions - unified reschedule logic handles all cases

use crate::discord::{
    agent::{create_agent_session, execute_agent_interaction, AgentSession},
    constants::{
        MESSAGE_CONTEXT_SIZE, MESSAGE_DEBOUNCE_TIMEOUT, TYPING_DEBOUNCE_TIMEOUT, WHITELIST_CHANNELS,
    },
    message::{queued_messages_to_rig_messages, QueuedMessage},
};
use serenity::all::{ChannelId, Message, Ready, TypingStartEvent};
use serenity::async_trait;
use serenity::prelude::*;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

/// Dual-timestamp activity tracker for proper debouncing
#[derive(Debug)]
pub(crate) struct ChannelActivity {
    /// When the last message occurred
    last_message: Option<Instant>,
    /// When the last typing event occurred
    last_typing: Option<Instant>,
    /// Handle to the current debounce timer
    timer_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ChannelActivity {
    fn new() -> Self {
        Self {
            last_message: None,
            last_typing: None,
            timer_handle: None,
        }
    }

    fn update_message(&mut self) {
        self.last_message = Some(Instant::now());
    }

    fn update_typing(&mut self) {
        self.last_typing = Some(Instant::now());
    }

    fn cancel_timer(&mut self) {
        if let Some(handle) = self.timer_handle.take() {
            handle.abort();
        }
    }

    fn set_timer(&mut self, handle: tokio::task::JoinHandle<()>) {
        self.cancel_timer();
        self.timer_handle = Some(handle);
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
}

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
            execute_agent_interaction(session, messages.len(), channel_id).await?;
        }
    }

    Ok(())
}

pub struct Handler {
    pub message_queue: Arc<Mutex<HashMap<ChannelId, Vec<QueuedMessage>>>>,
    /// Unified activity tracking for simplified debouncing
    pub channel_activity: Arc<Mutex<HashMap<ChannelId, ChannelActivity>>>,
    /// Persistent agent sessions per channel for multi-turn conversations
    pub agent_sessions: Arc<Mutex<HashMap<ChannelId, AgentSession>>>,
    /// Server configuration including OpenAI API key, Qdrant and other services
    pub server_config: crate::config::ServerConfig,
}

impl Handler {
    pub fn new(server_config: crate::config::ServerConfig) -> Self {
        Self {
            message_queue: Arc::new(Mutex::new(HashMap::new())),
            channel_activity: Arc::new(Mutex::new(HashMap::new())),
            agent_sessions: Arc::new(Mutex::new(HashMap::new())),
            server_config,
        }
    }

    /// Initialize agent sessions for all whitelisted channels on startup
    /// This helps recover conversation context after server restarts
    pub async fn initialize_channels(&self, ctx: &Context) -> Result<(), eyre::Error> {
        tracing::info!("Initializing agent sessions for whitelisted channels on startup...");

        for &channel_id_u64 in &WHITELIST_CHANNELS {
            let channel_id = ChannelId::new(channel_id_u64);

            // Check if channel has recent activity (messages in the last hour)
            match self.has_recent_activity(ctx, channel_id).await {
                Ok(true) => {
                    // Initialize agent session for this channel
                    if let Err(e) = self
                        .get_or_create_agent_session(ctx, channel_id, MESSAGE_CONTEXT_SIZE)
                        .await
                    {
                        tracing::error!(
                            "Failed to initialize agent session for channel {}: {}",
                            channel_id,
                            e
                        );
                    } else {
                        tracing::info!("Initialized agent session for channel {}", channel_id);

                        // Evaluate the recent conversation and potentially respond
                        if let Err(e) = self.evaluate_recent_conversation(ctx, channel_id).await {
                            tracing::error!(
                                "Failed to evaluate recent conversation for channel {}: {}",
                                channel_id,
                                e
                            );
                        }
                    }
                }
                Ok(false) => {
                    tracing::debug!("Skipping channel {} - no recent activity", channel_id);
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to check recent activity for channel {}: {}",
                        channel_id,
                        e
                    );
                }
            }
        }

        tracing::info!("Channel initialization complete");
        Ok(())
    }

    /// Check if a channel has recent activity (messages within the last hour)
    async fn has_recent_activity(
        &self,
        ctx: &Context,
        channel_id: ChannelId,
    ) -> Result<bool, eyre::Error> {
        use serenity::futures::StreamExt;

        let one_hour_ago = chrono::Utc::now() - chrono::Duration::hours(1);

        // Check the most recent message
        let has_recent = channel_id
            .messages_iter(&ctx.http)
            .take(1)
            .any(|msg_result| async move {
                match msg_result {
                    Ok(msg) => {
                        // Convert Discord timestamp to chrono DateTime
                        let msg_time = chrono::DateTime::from_timestamp(
                            msg.timestamp.timestamp(),
                            msg.timestamp.timestamp_subsec_nanos(),
                        );

                        if let Some(msg_time) = msg_time {
                            msg_time > one_hour_ago
                        } else {
                            false
                        }
                    }
                    Err(_) => false,
                }
            })
            .await;

        Ok(has_recent)
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
            // Get OpenAI API key from config
            let openai_api_key = self
                .server_config
                .openai_api_key
                .as_ref()
                .ok_or_else(|| eyre::eyre!("OpenAI API key not configured"))?;

            let session = create_agent_session(
                ctx,
                channel_id,
                context_size,
                openai_api_key,
                &self.server_config,
            )
            .await?;
            sessions.insert(channel_id, session);
        } else {
            // Update activity timestamp
            if let Some(session) = sessions.get_mut(&channel_id) {
                session.update_activity();
            }
        }

        Ok(())
    }

    /// Record message activity and schedule processing with proper debouncing
    async fn record_message_activity(&self, ctx: Context, channel_id: ChannelId) {
        let mut activities = self.channel_activity.lock().await;
        let activity = activities
            .entry(channel_id)
            .or_insert_with(ChannelActivity::new);

        activity.update_message();
        self.reschedule_processing(ctx, channel_id, activity).await;
    }

    /// Record typing activity and schedule processing with proper debouncing
    async fn record_typing_activity(&self, ctx: Context, channel_id: ChannelId) {
        let mut activities = self.channel_activity.lock().await;
        let activity = activities
            .entry(channel_id)
            .or_insert_with(ChannelActivity::new);

        activity.update_typing();
        self.reschedule_processing(ctx, channel_id, activity).await;
    }

    /// Reschedule processing timer based on current activity timestamps
    async fn reschedule_processing(
        &self,
        ctx: Context,
        channel_id: ChannelId,
        activity: &mut ChannelActivity,
    ) {
        // Cancel any existing timer
        activity.cancel_timer();

        // Calculate when we should process next
        let next_wake = match activity.next_processing_time() {
            Some(t) => t,
            None => return, // No activity to debounce
        };

        let now = Instant::now();
        let sleep_duration = if next_wake > now {
            next_wake - now
        } else {
            Duration::ZERO
        };

        // Clone necessary state for the async task
        let message_queue = self.message_queue.clone();
        let channel_activity = self.channel_activity.clone();
        let handler = Arc::new(Handler {
            message_queue: self.message_queue.clone(),
            channel_activity: self.channel_activity.clone(),
            agent_sessions: self.agent_sessions.clone(),
            server_config: self.server_config.clone(),
        });

        let handle = tokio::spawn(async move {
            tokio::time::sleep(sleep_duration).await;

            // Process messages if queue not empty
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

            // Clean up timer reference
            {
                let mut activities = channel_activity.lock().await;
                if let Some(activity) = activities.get_mut(&channel_id) {
                    activity.timer_handle = None;
                }
            }
        });

        activity.set_timer(handle);
    }

    /// Evaluate recent conversation on server startup and let the agent decide if it should
    /// respond
    async fn evaluate_recent_conversation(
        &self,
        _ctx: &Context,
        channel_id: ChannelId,
    ) -> Result<(), eyre::Error> {
        let mut sessions = self.agent_sessions.lock().await;
        if let Some(session) = sessions.get_mut(&channel_id) {
            // Only evaluate if there's actual conversation history
            if !session.conversation_history.is_empty() {
                // Check if the last message is from a user (not the bot) and relatively recent
                // We can determine this by checking if the message was created as a user message
                let should_evaluate = session
                    .conversation_history
                    .last()
                    .map(|last_msg| {
                        // Check if this is a user message
                        match last_msg {
                            rig::completion::Message::User { .. } => true,
                            rig::completion::Message::Assistant { .. } => false,
                        }
                    })
                    .unwrap_or(false);

                if should_evaluate {
                    tracing::info!(
                        "Evaluating recent conversation for channel {} with {} messages after restart",
                        channel_id,
                        session.conversation_history.len()
                    );

                    // Let the agent analyze the conversation and decide whether to respond
                    // We pass 0 for messages_count since this is a startup evaluation, not new messages
                    execute_agent_interaction(session, 0, channel_id).await?;
                } else {
                    tracing::debug!(
                        "Skipping evaluation for channel {} - last message was from bot or no messages",
                        channel_id
                    );
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if !WHITELIST_CHANNELS.contains(&msg.channel_id.get()) {
            return;
        }

        if msg.author.bot && msg.author.id == ctx.cache.current_user().id {
            // No need to process messages from the bot itself, it will be represented as a tool
            // call in the conversation history, so duplicating it here would be redundant.
            return;
        }

        let channel_id = msg.channel_id;

        // For human messages: add to queue and record activity (triggers unified debouncing)
        let queued_msg = QueuedMessage { message: msg };

        {
            let mut queue = self.message_queue.lock().await;
            let channel_messages = queue.entry(channel_id).or_insert_with(Vec::new);
            channel_messages.push(queued_msg);

            // Limit queue size per channel
            if channel_messages.len() > 10 {
                channel_messages.remove(0);
            }
        }

        // Record activity which will handle proper debouncing
        self.record_message_activity(ctx, channel_id).await;
    }

    async fn typing_start(&self, ctx: Context, event: TypingStartEvent) {
        if !WHITELIST_CHANNELS.contains(&event.channel_id.get()) {
            return;
        }

        // Don't track bot typing
        let user_id = event.user_id;
        if let Ok(user) = user_id.to_user(&ctx.http).await {
            if user.bot {
                return;
            }
        }

        // Record typing as activity (triggers proper debouncing)
        self.record_typing_activity(ctx, event.channel_id).await;
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);

        // Initialize agent sessions for active channels after startup
        if let Err(e) = self.initialize_channels(&ctx).await {
            tracing::error!("Failed to initialize channels on startup: {}", e);
        }
    }
}
