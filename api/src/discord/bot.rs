use crate::discord::{
    channel::{ChannelEvent, ChannelHandle},
    constants::WHITELIST_CHANNELS,
    message::QueuedMessage,
};
use arc_swap::ArcSwap;
use serenity::all::{ChannelId, Message, Ready, TypingStartEvent};
use serenity::async_trait;
use serenity::prelude::*;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, MutexGuard};

use super::tools::SharedVectorClient;

pub struct DiscordEventHandler {
    channel_handles: Arc<Mutex<HashMap<ChannelId, ChannelHandle>>>,

    shared_vectordb_client: Option<SharedVectorClient>,
    openai_api_key: String,
    whitelist_channels: Vec<ChannelId>,
    bot_user_id: ArcSwap<Option<serenity::model::id::UserId>>,
    discord_bot_mention_only: bool,
}

impl DiscordEventHandler {
    pub async fn new(server_config: crate::config::ServerConfig) -> Self {
        let shared_vectordb_client = match &server_config.vector_db {
            Some(conf) => SharedVectorClient::new(conf.clone())
                .await
                .inspect_err(|e| {
                    tracing::error!(
                        "Failed to create shared vector client, defaulting to None: {}",
                        e
                    );
                })
                .ok(),
            None => None,
        };

        Self {
            channel_handles: Arc::new(Mutex::new(HashMap::new())),
            whitelist_channels: (server_config.discord_whitelist_channels.as_ref())
                .unwrap_or(&WHITELIST_CHANNELS.to_vec())
                .iter()
                .map(|id| ChannelId::new(*id))
                .collect(),
            shared_vectordb_client,
            bot_user_id: ArcSwap::new(Arc::new(None)),
            openai_api_key: server_config.openai_api_key.clone().unwrap_or_default(),
            discord_bot_mention_only: server_config.discord_mention_only,
        }
    }

    /// Initialize agent sessions for all whitelisted channels on startup
    /// This helps recover conversation context after server restarts
    pub async fn initialize_channels(&self, ctx: &Context) -> Result<(), eyre::Error> {
        tracing::info!("Initializing agent sessions for whitelisted channels on startup...");

        for channel_id in &self.whitelist_channels {
            let channel_id = *channel_id;
            // Check if channel has recent activity (messages in the last hour)
            match self.has_recent_activity(ctx, channel_id).await {
                Ok(true) => {
                    self.get_or_create_channel_handle(
                        &mut self.channel_handles.lock().await,
                        channel_id,
                        ctx.clone(),
                    )
                    .send_event(ChannelEvent::ForceProcess)
                    .await
                    .inspect_err(|e| {
                        tracing::error!(
                            "Failed to send ForceProcess event to channel {} upon \
                                reevaluating recent conversation on service startup: {}",
                            channel_id,
                            e
                        );
                    })?;
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
                                && msg.author.id
                                    != self.bot_user_id.load().as_ref().unwrap_or_default()
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

    fn get_or_create_channel_handle<'a>(
        &self,
        lock: &'a mut MutexGuard<'_, HashMap<ChannelId, ChannelHandle>>,
        channel_id: ChannelId,
        discord_ctx: Context,
    ) -> &'a mut ChannelHandle {
        lock.entry(channel_id).or_insert_with(|| {
            ChannelHandle::new(
                discord_ctx,
                channel_id,
                self.openai_api_key.clone(),
                self.shared_vectordb_client.clone(),
                self.discord_bot_mention_only,
            )
        })
    }
}

#[async_trait]
impl EventHandler for DiscordEventHandler {
    async fn message(&self, ctx: Context, msg: Message) {
        if !self.whitelist_channels.contains(&msg.channel_id) {
            return;
        }

        let _ = self
            .get_or_create_channel_handle(
                &mut self.channel_handles.lock().await,
                msg.channel_id,
                ctx.clone(),
            )
            .send_event(ChannelEvent::Message(QueuedMessage { message: msg }))
            .await
            .inspect_err(|e| {
                tracing::error!(?e, "Failed to send Message event");
            });
    }

    async fn typing_start(&self, ctx: Context, event: TypingStartEvent) {
        if !self.whitelist_channels.contains(&event.channel_id) {
            return;
        }

        let _ = self
            .get_or_create_channel_handle(
                &mut self.channel_handles.lock().await,
                event.channel_id,
                ctx.clone(),
            )
            .send_event(ChannelEvent::Typing(event.user_id))
            .await
            .inspect_err(|e| {
                tracing::error!(
                    "Failed to send Typing event to channel {}: {}",
                    event.channel_id,
                    e
                );
            });
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);

        // Store bot user ID for mention detection
        self.bot_user_id.store(Arc::new(Some(ready.user.id)));

        if self.discord_bot_mention_only {
            tracing::info!("Bot is in mention-only mode - will only respond to mentions");
        } else {
            tracing::info!("Bot is in auto mode - will process all messages");
        }

        // Initialize agent sessions for active channels after startup
        if let Err(e) = self.initialize_channels(&ctx).await {
            tracing::error!("Failed to initialize channels on startup: {}", e);
        }
    }
}
