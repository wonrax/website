use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::{ChannelId, CreateMessage, Context};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct DiscordSendMessageTool {
    pub ctx: Arc<Context>,
    pub channel_id: ChannelId,
    pub reply_to_message_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordSendMessageArgs {
    pub content: String,
    #[serde(default)]
    pub reply: bool,
    /// Message ID to reply to (if reply is true). If not provided, replies to the most recent message.
    #[serde(default)]
    pub reply_to_message_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordSendMessageOutput {
    pub success: bool,
    pub message_id: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Discord send message error: {0}")]
pub struct DiscordSendMessageError(String);

impl Tool for DiscordSendMessageTool {
    const NAME: &'static str = "send_discord_message";
    type Error = DiscordSendMessageError;
    type Args = DiscordSendMessageArgs;
    type Output = DiscordSendMessageOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "send_discord_message".to_string(),
            description: "Send a message to the Discord channel. Use this to respond to users."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The message content to send"
                    },
                    "reply": {
                        "type": "boolean",
                        "description": "Whether to reply to a specific message (true) or send a standalone message (false)",
                        "default": false
                    },
                    "reply_to_message_id": {
                        "type": "number",
                        "description": "The Discord message ID to reply to (only used when reply=true). If not provided when reply=true, will reply to the most recent message."
                    }
                },
                "required": ["content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Clone values to move into the spawned task
        let ctx = self.ctx.clone();
        let channel_id = self.channel_id;
        let default_reply_to_message_id = self.reply_to_message_id;
        let content = args.content.clone();
        let reply = args.reply;

        // Use the message ID from args if provided, otherwise fall back to default
        let target_message_id = if reply {
            args.reply_to_message_id.or(default_reply_to_message_id)
        } else {
            None
        };

        // Spawn the Discord API operations in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move {
            let mut message_builder = CreateMessage::new().content(&content);

            if reply && target_message_id.is_some() {
                if let Ok(original_msg) = channel_id
                    .message(&ctx.http, target_message_id.unwrap())
                    .await
                {
                    message_builder = message_builder.reference_message(&original_msg);
                }
            }

            channel_id.send_message(&ctx.http, message_builder).await
        });

        match handle.await {
            Ok(Ok(sent_message)) => {
                tracing::debug!("Sent Discord message: {}", args.content);
                Ok(DiscordSendMessageOutput {
                    success: true,
                    message_id: Some(sent_message.id.get()),
                    error: None,
                })
            }
            Ok(Err(e)) => {
                tracing::error!("Failed to send Discord message: {}", e);
                Ok(DiscordSendMessageOutput {
                    success: false,
                    message_id: None,
                    error: Some(e.to_string()),
                })
            }
            Err(e) => {
                tracing::error!("Task join error while sending Discord message: {}", e);
                Ok(DiscordSendMessageOutput {
                    success: false,
                    message_id: None,
                    error: Some(format!("Task execution failed: {}", e)),
                })
            }
        }
    }
}
