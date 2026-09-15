use crate::discord::message::{fetch_channel_message, format_message_compact, parse_message_id};
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::{ChannelId, Context, UserId};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct FetchMessageTool {
    pub ctx: Arc<Context>,
    pub channel_id: ChannelId,
    pub bot_user_id: UserId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchMessageArgs {
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchMessageOutput {
    pub success: bool,
    /// The message in the same format as the agent's context
    pub message: Option<String>,
    pub error: Option<String>,
}

impl FetchMessageOutput {
    fn failure(error: String) -> Self {
        Self {
            success: false,
            message: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Error)]
#[error("Fetch message error: {0}")]
pub struct FetchMessageError(String);

impl Tool for FetchMessageTool {
    const NAME: &'static str = "fetch_message";
    type Error = FetchMessageError;
    type Args = FetchMessageArgs;
    type Output = FetchMessageOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Read one message of this channel by ID when it is not in your context, typically the target of a Replied To line. Returns it in the same format as your context, attachments listed by name only; view_message_attachments shows them. For a stretch of the conversation use fetch_channel_history instead."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message_id": {
                        "type": "string",
                        "description": "The ID from a [#ID] message header or a Replied To line."
                    }
                },
                "required": ["message_id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let message_id = match parse_message_id("message_id", &args.message_id) {
            Ok(id) => id,
            Err(error) => return Ok(FetchMessageOutput::failure(error)),
        };

        match fetch_channel_message(&self.ctx, self.channel_id, message_id).await {
            Ok(message) => Ok(FetchMessageOutput {
                success: true,
                message: Some(format_message_compact(&message, self.bot_user_id)),
                error: None,
            }),
            Err(error) => Ok(FetchMessageOutput::failure(error)),
        }
    }
}
