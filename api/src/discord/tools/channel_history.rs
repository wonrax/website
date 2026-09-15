use crate::discord::message::{format_message_compact, parse_message_id};
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::{ChannelId, Context, GetMessages, UserId};
use std::sync::Arc;
use thiserror::Error;

/// Page size when the agent doesn't ask for one
const DEFAULT_LIMIT: u32 = 30;
/// Discord's per-request cap
const MAX_LIMIT: u32 = 100;

#[derive(Debug, Clone)]
pub struct FetchChannelHistoryTool {
    pub ctx: Arc<Context>,
    pub channel_id: ChannelId,
    pub bot_user_id: UserId,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryDirection {
    Before,
    After,
    Around,
}

impl HistoryDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
            Self::Around => "around",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchChannelHistoryArgs {
    pub anchor_message_id: String,
    pub direction: HistoryDirection,

    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchChannelHistoryOutput {
    pub success: bool,
    /// Formatted messages, oldest first
    pub messages: String,
    pub count: usize,
    /// Cursors come from the raw page rather than the rendered messages so paging
    /// continues past messages that were filtered out of `messages`
    pub oldest_message_id: Option<String>,
    pub newest_message_id: Option<String>,
    pub more_available: bool,
    pub error: Option<String>,
}

impl FetchChannelHistoryOutput {
    fn failure(error: String) -> Self {
        Self {
            success: false,
            messages: String::new(),
            count: 0,
            oldest_message_id: None,
            newest_message_id: None,
            more_available: false,
            error: Some(error),
        }
    }
}

#[derive(Debug, Error)]
#[error("Fetch channel history error: {0}")]
pub struct FetchChannelHistoryError(String);

impl Tool for FetchChannelHistoryTool {
    const NAME: &'static str = "fetch_channel_history";
    type Error = FetchChannelHistoryError;
    type Args = FetchChannelHistoryArgs;
    type Output = FetchChannelHistoryOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Fetch messages from this channel's history that are outside your context. Your context only holds the most recent stretch of the channel, so use this when the thread under discussion began earlier, when a reply points at a message you can't see, or when you need to check when you last spoke. Results use the same header format as your context, oldest first, with your own messages tagged [you]. Attachments are listed by name; view_message_attachments shows them. Keep paging by passing the returned oldest_message_id (or newest_message_id) as the next anchor while more_available is true."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "anchor_message_id": {
                        "type": "string",
                        "description": "The ID from a [#ID] message header to page from."
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["before", "after", "around"],
                        "description": "before: messages older than the anchor (the usual choice, anchored at the oldest message you can see). after: messages newer than the anchor. around: both sides of the anchor, for the context of a message someone replied to."
                    },
                    "limit": {
                        "type": ["integer", "null"],
                        "description": "Messages to fetch, 1-100. Default 30."
                    }
                },
                "required": ["anchor_message_id", "direction", "limit"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let anchor = match parse_message_id("anchor_message_id", &args.anchor_message_id) {
            Ok(anchor) => anchor,
            Err(error) => return Ok(FetchChannelHistoryOutput::failure(error)),
        };
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

        // MAX_LIMIT fits in a u8, so the cast cannot truncate
        let page = GetMessages::new().limit(limit as u8);
        let page = match args.direction {
            HistoryDirection::Before => page.before(anchor),
            HistoryDirection::After => page.after(anchor),
            HistoryDirection::Around => page.around(anchor),
        };

        let ctx = self.ctx.clone();
        let channel_id = self.channel_id;

        // Spawn the Discord API operations in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move { channel_id.messages(&ctx.http, page).await });

        let mut fetched = match handle.await {
            Ok(Ok(messages)) => messages,
            Ok(Err(e)) => {
                tracing::error!(?e, "Failed to fetch channel history");
                return Ok(FetchChannelHistoryOutput::failure(e.to_string()));
            }
            Err(e) => {
                tracing::error!(?e, "Task join error while fetching channel history");
                return Ok(FetchChannelHistoryOutput::failure(format!(
                    "Task execution failed: {e}"
                )));
            }
        };

        // Discord returns newest first; the agent reads oldest first like its context
        fetched.sort_by_key(|m| m.id.get());

        let more_available = fetched.len() >= limit as usize;
        let oldest_message_id = fetched.first().map(|m| m.id.to_string());
        let newest_message_id = fetched.last().map(|m| m.id.to_string());

        let rendered: Vec<String> = fetched
            .iter()
            .filter(|m| !m.content.trim().is_empty() || !m.attachments.is_empty())
            .map(|m| format_message_compact(m, self.bot_user_id))
            .collect();
        let count = rendered.len();

        let messages = if fetched.is_empty() {
            format!(
                "[no messages {} message {}]",
                args.direction.as_str(),
                anchor
            )
        } else {
            rendered.join("\n")
        };

        tracing::debug!(
            direction = args.direction.as_str(),
            anchor = anchor.get(),
            limit,
            count,
            "fetch_channel_history completed"
        );

        Ok(FetchChannelHistoryOutput {
            success: true,
            messages,
            count,
            oldest_message_id,
            newest_message_id,
            more_available,
            error: None,
        })
    }
}
