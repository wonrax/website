use rig::tool::PortableTool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::{ChannelId, Context, CreateMessage, MessageId};
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

    #[serde(default)]
    pub reply_to_message_id: Option<String>,
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

impl PortableTool for DiscordSendMessageTool {
    const NAME: &'static str = "send_discord_message";
    type Error = DiscordSendMessageError;
    type Args = DiscordSendMessageArgs;
    type Output = DiscordSendMessageOutput;

    fn description(&self) -> String {
        "Send a message to the Discord channel. This is the only way users see anything you produce; raw text output never reaches Discord. To ping someone write <@USER_ID> with an ID from fetch_message_user_ids; a bare name does not ping. Several short messages beat one wall of text when the channel is chatting in short bursts."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Message body. Discord markdown is supported; use it sparingly."
                },
                "reply_to_message_id": {
                    "type": ["string", "null"],
                    "description": "The ID from the [#ID] header of the message being answered. Set it when the reply targets a specific message rather than the channel at large; null otherwise."
                }
            },
            "required": ["content", "reply_to_message_id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Clone values to move into the spawned task
        let ctx = self.ctx.clone();
        let channel_id = self.channel_id;
        let content = args.content.clone();

        // Spawn the Discord API operations in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move {
            let mut message_builder = CreateMessage::new().content(&content);

            if let Some(reply_to_message_id) = args.reply_to_message_id
                && let Some(target_message_id) = reply_to_message_id.parse::<u64>().ok()
                && let Ok(original_msg) = channel_id
                    .message(&ctx.http, MessageId::new(target_message_id))
                    .await
                    .inspect_err(|e| {
                        tracing::error!(
                            "Failed to fetch original message with ID {}: {}",
                            target_message_id,
                            e
                        )
                    })
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
