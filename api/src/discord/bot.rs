// Discord Bot with Unified Activity Debouncing
//
// This implementation uses a unified debouncing strategy that treats both new messages
// and typing indicators as "activity". Instead of having separate timers for messages
// and typing events, we use a single timer per channel that resets on any activity.
//
// Benefits of this approach:
// 1. Simpler state management - single ChannelActivity struct per channel
// 2. No race conditions between typing and message timers
// 3. More predictable behavior - any activity delays processing
// 4. Reduced memory usage - fewer timer handles and tracking structures
// 5. Easier to reason about - unified "activity debounce" concept
//
// How it works:
// - Any activity (message or typing) calls record_activity()
// - record_activity() updates timestamp and resets the debounce timer
// - Timer expires after MESSAGE_DEBOUNCE_TIMEOUT_MS of inactivity
// - Before processing, we check if recent activity suggests ongoing typing
// - Only process messages if no recent activity detected

use crate::discord::{
    agent::{create_agent_session, execute_agent_interaction, AgentSession},
    constants::{
        MESSAGE_CONTEXT_SIZE, MESSAGE_DEBOUNCE_TIMEOUT_MS, TYPING_DEBOUNCE_TIMEOUT_MS,
        WHITELIST_CHANNELS,
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

/// Simplified activity tracker for unified debouncing
#[derive(Debug)]
pub(crate) struct ChannelActivity {
    /// When the last activity (message or typing) occurred
    last_activity: Instant,
    /// Handle to the current debounce timer
    timer_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ChannelActivity {
    fn new() -> Self {
        Self {
            last_activity: Instant::now(),
            timer_handle: None,
        }
    }

    fn update_activity(&mut self) {
        self.last_activity = Instant::now();
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

    /// Check if recent activity (within typing timeout) suggests someone might still be active
    fn has_recent_activity(&self) -> bool {
        self.last_activity.elapsed() < Duration::from_millis(TYPING_DEBOUNCE_TIMEOUT_MS)
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
            let _ = execute_agent_interaction(session, messages.len(), channel_id).await;
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

    /// Create a new Handler instance that shares the same underlying data
    fn clone_for_task(&self) -> Arc<Self> {
        Arc::new(Handler {
            message_queue: self.message_queue.clone(),
            channel_activity: self.channel_activity.clone(),
            agent_sessions: self.agent_sessions.clone(),
            server_config: self.server_config.clone(),
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

    /// Record activity (message or typing) and schedule processing with unified debouncing
    async fn record_activity(&self, ctx: Context, channel_id: ChannelId) {
        let mut activities = self.channel_activity.lock().await;
        let activity = activities
            .entry(channel_id)
            .or_insert_with(ChannelActivity::new);

        // Update activity timestamp and cancel any existing timer
        activity.update_activity();
        activity.cancel_timer();

        // Create new debounce timer
        let message_queue = self.message_queue.clone();
        let channel_activity = self.channel_activity.clone();
        let handler = self.clone_for_task();

        let handle = tokio::spawn(async move {
            // Wait for the debounce timeout
            tokio::time::sleep(Duration::from_millis(MESSAGE_DEBOUNCE_TIMEOUT_MS)).await;

            // Check if there's been recent activity that suggests someone is still active
            let should_process = {
                let activities = channel_activity.lock().await;
                activities
                    .get(&channel_id)
                    .map(|activity| !activity.has_recent_activity())
                    .unwrap_or(true)
            };

            if !should_process {
                // Recent activity detected, don't process yet
                // The timer will be rescheduled by the next activity
                return;
            }

            // Clean up the timer reference
            {
                let mut activities = channel_activity.lock().await;
                if let Some(activity) = activities.get_mut(&channel_id) {
                    activity.timer_handle = None;
                }
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
        activity.set_timer(handle);
    }

    /// Evaluate recent conversation and let the agent decide if it should respond
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
                    let _ = execute_agent_interaction(session, 0, channel_id).await;
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

        let channel_id = msg.channel_id;

        // If this is our own bot message, add it to conversation history but don't process
        if msg.author.bot && msg.author.id == ctx.cache.current_user().id {
            // Ensure we have an agent session for this channel
            if let Ok(()) = self
                .get_or_create_agent_session(&ctx, channel_id, MESSAGE_CONTEXT_SIZE)
                .await
            {
                let mut sessions = self.agent_sessions.lock().await;
                if let Some(session) = sessions.get_mut(&channel_id) {
                    // Convert bot message to rig format and add to conversation history
                    let queued_msg = QueuedMessage { message: msg };
                    let rig_messages = queued_messages_to_rig_messages(&[queued_msg]);
                    session.add_messages(rig_messages);
                }
            }
            return;
        }

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

        // Record activity which will handle unified debouncing
        self.record_activity(ctx, channel_id).await;
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

        // Record typing as activity (triggers unified debouncing)
        self.record_activity(ctx, event.channel_id).await;
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);

        // Initialize agent sessions for active channels after startup
        if let Err(e) = self.initialize_channels(&ctx).await {
            tracing::error!("Failed to initialize channels on startup: {}", e);
        }
    }
}
