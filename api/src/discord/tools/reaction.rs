use crate::discord::message::parse_message_id;
use rig::tool::PortableTool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::{ChannelId, Context, ReactionType};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ReactToMessageTool {
    pub ctx: Arc<Context>,
    pub channel_id: ChannelId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactToMessageArgs {
    pub message_id: String,
    pub emoji: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactToMessageOutput {
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Reaction error: {0}")]
pub struct ReactToMessageError(String);

impl PortableTool for ReactToMessageTool {
    const NAME: &'static str = "react_to_message";
    type Error = ReactToMessageError;
    type Args = ReactToMessageArgs;
    type Output = ReactToMessageOutput;

    fn description(&self) -> String {
        "Add an emoji reaction to a message in the channel. Sometimes a reaction says it better than a reply, or goes with one.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "message_id": {
                    "type": "string",
                    "description": "The ID from the [#ID] header of the message to react to."
                },
                "emoji": {
                    "type": "string",
                    "description": "A unicode emoji, or a server emoji as <:name:id> (<a:name:id> when animated)."
                }
            },
            "required": ["message_id", "emoji"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let failure = |error: String| {
            Ok(ReactToMessageOutput {
                success: false,
                error: Some(error),
            })
        };

        let message_id = match parse_message_id("message_id", &args.message_id) {
            Ok(id) => id,
            Err(error) => return failure(error),
        };
        let Ok(reaction) = ReactionType::try_from(args.emoji.trim()) else {
            return failure(format!(
                "{:?} is neither a unicode emoji nor a <:name:id> server emoji",
                args.emoji
            ));
        };

        match self
            .channel_id
            .create_reaction(&self.ctx.http, message_id, reaction)
            .await
        {
            Ok(()) => Ok(ReactToMessageOutput {
                success: true,
                error: None,
            }),
            Err(e) => {
                tracing::error!(
                    ?e,
                    message_id = message_id.get(),
                    "Failed to add a reaction"
                );
                failure(format!("Discord refused the reaction: {e}"))
            }
        }
    }
}
