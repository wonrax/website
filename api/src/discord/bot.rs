use crate::config::DiscordLlmBackend;
use crate::discord::{
    agent::LlmBackend,
    channel::{ChannelEvent, ChannelHandle},
    chatgpt::{ChatgptAuth, DbPool},
    constants::{MESSAGE_CONTEXT_SIZE, WHITELIST_CHANNELS},
    message::{self, QueuedMessage},
};
use arc_swap::ArcSwap;
use async_trait::async_trait;
use scc::hash_map::OccupiedEntry;
use serenity::all::{
    Activity, ChannelId, GuildId, Message, Presence, Ready, TypingStartEvent, UserId,
};
use serenity::prelude::*;
use std::sync::Arc;
use tracing::instrument;

use super::tools::{Firecrawl, SharedVectorClient};

pub(crate) struct Guild {
    pub presences: scc::HashMap<UserId, Vec<Activity>>,
}

pub struct DiscordEventHandler {
    channel_handles: Arc<scc::HashMap<ChannelId, ChannelHandle>>,
    guilds: Arc<scc::HashMap<GuildId, Guild>>,

    shared_vectordb_client: Option<SharedVectorClient>,
    firecrawl: Option<Firecrawl>,
    llm: LlmBackend,
    whitelist_channels: Vec<ChannelId>,
    bot_user_id: ArcSwap<Option<serenity::model::id::UserId>>,
    discord_bot_mention_only: bool,
}

impl DiscordEventHandler {
    pub async fn new(server_config: crate::config::ServerConfig, db: DbPool) -> Self {
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

        let firecrawl = match server_config.firecrawl_api_key.clone() {
            Some(api_key) => Firecrawl::new(api_key)
                .inspect_err(|e| tracing::error!(?e, "Failed to create the Firecrawl client"))
                .ok(),
            None => {
                tracing::warn!(
                    "FIRECRAWL_API_KEY is not set; the agent will have no web search or page fetching"
                );
                None
            }
        };

        let llm = match server_config.discord_llm_backend {
            DiscordLlmBackend::Chatgpt => LlmBackend::Chatgpt(ChatgptAuth::load(db).await),
            DiscordLlmBackend::Gemini => LlmBackend::Gemini {
                api_key: server_config.openai_api_key.clone().unwrap_or_default(),
            },
        };
        tracing::info!(backend = ?server_config.discord_llm_backend, "Discord agent LLM backend");

        Self {
            channel_handles: Arc::new(scc::HashMap::new()),
            guilds: Arc::new(scc::HashMap::new()),
            whitelist_channels: (server_config.discord_whitelist_channels.as_ref())
                .unwrap_or(&WHITELIST_CHANNELS.to_vec())
                .iter()
                .map(|id| ChannelId::new(*id))
                .collect(),
            shared_vectordb_client,
            firecrawl,
            bot_user_id: ArcSwap::from_pointee(None),
            llm,
            discord_bot_mention_only: server_config.discord_mention_only,
        }
    }

    /// Initialize agent sessions for all whitelisted channels on startup
    /// This helps recover conversation context after server restarts
    #[instrument(skip(self, ctx))]
    pub async fn initialize_channels(&self, ctx: &Context) -> Result<(), eyre::Error> {
        tracing::info!("Initializing agent sessions for whitelisted channels on startup...");

        for channel_id in &self.whitelist_channels {
            let channel_id = *channel_id;

            // An unanswered mention goes straight to the responder. Otherwise, in auto mode, recent
            // activity is the watcher's to judge.
            let addressed = match self.has_recent_mention(ctx, channel_id).await {
                Ok(addressed) => addressed,
                Err(e) => {
                    tracing::error!(
                        "Failed to check recent mentions for channel {}: {}",
                        channel_id,
                        e
                    );
                    false
                }
            };
            let should_process = addressed
                || (!self.discord_bot_mention_only
                    && self
                        .has_recent_activity(ctx, channel_id)
                        .await
                        .inspect_err(|e| {
                            tracing::error!(
                                "Failed to check recent activity for channel {}: {}",
                                channel_id,
                                e
                            );
                        })
                        .unwrap_or(false));
            if !should_process {
                tracing::debug!(
                    "Skipping channel {} - nothing recent to process",
                    channel_id
                );
                continue;
            }

            self.get_or_create_channel(channel_id, ctx.clone())
                .send_event(ChannelEvent::ForceProcess { addressed })
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

        tracing::info!("Channel initialization complete");
        Ok(())
    }

    /// Whether the last MESSAGE_CONTEXT_SIZE messages hold a mention of the bot, or a reply to
    /// it, that the bot has not posted since. A newer message of its own means it already handled
    /// the mention before this restart, and running the agent again would answer it twice.
    #[instrument(skip(self, ctx))]
    async fn has_recent_mention(
        &self,
        ctx: &Context,
        channel_id: ChannelId,
    ) -> Result<bool, eyre::Error> {
        use serenity::futures::StreamExt;

        let bot_user_id = self.bot_user_id.load();
        let bot_id = match bot_user_id.as_ref() {
            Some(id) => *id,
            None => return Ok(false),
        };

        // Newest first, so the first message that is either the bot's own or a mention decides
        let mut messages = std::pin::pin!(
            channel_id
                .messages_iter(&ctx.http)
                .take(MESSAGE_CONTEXT_SIZE)
        );
        while let Some(msg) = messages.next().await {
            let Ok(msg) = msg else {
                continue;
            };

            if msg.author.id == bot_id {
                return Ok(false);
            }
            if message::addresses(&msg, bot_id) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Check if a channel has recent activity (messages within the last hour)
    #[instrument(skip(self, ctx))]
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
                                && self
                                    .bot_user_id
                                    .load()
                                    .as_ref()
                                    .is_none_or(|id| msg.author.id != id)
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

    fn get_or_create_channel<'a>(
        &'a self,
        channel_id: ChannelId,
        discord_ctx: Context,
    ) -> OccupiedEntry<'a, ChannelId, ChannelHandle> {
        self.channel_handles
            .entry_sync(channel_id)
            .or_insert_with(|| {
                ChannelHandle::new(
                    discord_ctx,
                    channel_id,
                    self.llm.clone(),
                    self.shared_vectordb_client.clone(),
                    self.firecrawl.clone(),
                    self.discord_bot_mention_only,
                    self.guilds.clone(),
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
            .get_or_create_channel(msg.channel_id, ctx.clone())
            .send_event(ChannelEvent::Message(QueuedMessage { message: msg }, ctx))
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
            .get_or_create_channel(event.channel_id, ctx.clone())
            .send_event(ChannelEvent::Typing(event.user_id, ctx))
            .await
            .inspect_err(|e| {
                tracing::error!(
                    "Failed to send Typing event to channel {}: {}",
                    event.channel_id,
                    e
                );
            });
    }

    async fn presence_update(&self, _ctx: Context, new_presence: Presence) {
        // TODO: add whitelist guild config and check here

        let guild_id = if let Some(guild_id) = new_presence.guild_id {
            guild_id
        } else {
            tracing::warn!(
                user_id = new_presence.user.id.get(),
                "Received presence update without guild ID",
            );
            return;
        };

        let _ = self
            .guilds
            .entry_sync(guild_id)
            .or_insert_with(|| Guild {
                presences: scc::HashMap::new(),
            })
            .presences
            .upsert_sync(new_presence.user.id, new_presence.activities);
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);

        // Store bot user ID for mention detection
        self.bot_user_id.store(Arc::new(Some(ready.user.id)));

        if self.discord_bot_mention_only {
            tracing::info!("Bot is in mention-only mode - will only respond to mentions");
        } else {
            tracing::info!("Bot is in auto mode - the watcher decides when to respond");
        }

        // Initialize agent sessions for active channels after startup
        if let Err(e) = self.initialize_channels(&ctx).await {
            tracing::error!("Failed to initialize channels on startup: {}", e);
        }
    }
}
