use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::{ChannelId, Context, CreateMessage};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct DiscordSendMessageTool {
    pub ctx: Arc<Context>,
    pub channel_id: ChannelId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordSendMessageArgs {
    pub content: String,
    /// Message ID to reply to (if reply is true). If not provided, replies to the most recent message.
    /// Uses float to accommodate JSON number type to side-step this error:
    /// Toolset error: ToolCallError: ToolCallError: JsonError: invalid type: floating point
    /// `1.4601911394024858e+18`, expected u64 at line 1 column 691
    #[serde(default)]
    pub reply_to_message_id: Option<f64>,
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
                    "reply_to_message_id": {
                        "type": "number",
                        "description": "The Discord message ID to reply to."
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
        let content = args.content.clone();

        // Spawn the Discord API operations in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move {
            let mut message_builder = CreateMessage::new().content(&content);

            if let Some(target_message_id) = args.reply_to_message_id
                && let Ok(original_msg) = channel_id
                    .message(&ctx.http, target_message_id as u64)
                    .await
            {
                message_builder = message_builder.reference_message(&original_msg);
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
