use crate::discord::message::{fetch_channel_message, parse_message_id};
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::{ChannelId, Context, User};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct FetchMessageUserIdsTool {
    pub ctx: Arc<Context>,
    pub channel_id: ChannelId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchMessageUserIdsArgs {
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRef {
    pub name: String,
    pub user_id: String,
}

impl From<&User> for UserRef {
    fn from(user: &User) -> Self {
        Self {
            name: user.name.clone(),
            user_id: user.id.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchMessageUserIdsOutput {
    pub success: bool,
    pub author: Option<UserRef>,
    pub mentions: Vec<UserRef>,
    pub error: Option<String>,
}

impl FetchMessageUserIdsOutput {
    fn failure(error: String) -> Self {
        Self {
            success: false,
            author: None,
            mentions: vec![],
            error: Some(error),
        }
    }
}

#[derive(Debug, Error)]
#[error("Fetch message user IDs error: {0}")]
pub struct FetchMessageUserIdsError(String);

impl Tool for FetchMessageUserIdsTool {
    const NAME: &'static str = "fetch_message_user_ids";
    type Error = FetchMessageUserIdsError;
    type Args = FetchMessageUserIdsArgs;
    type Output = FetchMessageUserIdsOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Get the user IDs behind a message: its author and every user it mentions. Message headers show names only, so call this before pinging someone with <@USER_ID> in send_discord_message."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message_id": {
                        "type": "string",
                        "description": "The ID from the [#ID] header of a message written by, or mentioning, the user you need."
                    }
                },
                "required": ["message_id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let message_id = match parse_message_id("message_id", &args.message_id) {
            Ok(id) => id,
            Err(error) => return Ok(FetchMessageUserIdsOutput::failure(error)),
        };

        match fetch_channel_message(&self.ctx, self.channel_id, message_id).await {
            Ok(message) => Ok(FetchMessageUserIdsOutput {
                success: true,
                author: Some(UserRef::from(&message.author)),
                mentions: message.mentions.iter().map(UserRef::from).collect(),
                error: None,
            }),
            Err(error) => Ok(FetchMessageUserIdsOutput::failure(error)),
        }
    }
}
