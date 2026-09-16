use super::vector_client::SharedVectorClient;
use crate::discord::message::parse_message_ids;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

#[derive(Clone)]
pub struct MemoryStoreTool {
    pub client: SharedVectorClient,
    pub channel_id: u64, // Discord channel ID
}

impl MemoryStoreTool {
    pub fn new_with_client(client: SharedVectorClient, channel_id: u64) -> Self {
        Self { client, channel_id }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStoreArgs {
    pub information: String,
    #[serde(default)]
    pub source_message_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStoreOutput {
    pub success: bool,
    pub point_id: Option<String>,
    pub error: Option<String>,
}

impl MemoryStoreOutput {
    fn failure(error: String) -> Self {
        Self {
            success: false,
            point_id: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Error)]
#[error("Memory store error: {0}")]
pub struct MemoryStoreError(String);

impl Tool for MemoryStoreTool {
    const NAME: &'static str = "memory_store";
    type Error = MemoryStoreError;
    type Args = MemoryStoreArgs;
    type Output = MemoryStoreOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "Save something a future session should know about a user or channel {}. Run memory_find first: when an entry on the same fact exists, memory_update extends it instead of adding a duplicate. After storing, tell the channel in one short line via send_discord_message.",
                self.channel_id
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "information": {
                        "type": "string",
                        "description": "The fact, self-contained enough to make sense months later: name who it is about and what happened."
                    },
                    "source_message_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "IDs from the [#ID] headers of the messages the fact comes from. A later session rereads the conversation around them, so cite the messages that carry the fact rather than the whole batch."
                    }
                },
                "required": ["information", "source_message_ids"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let source_message_ids =
            match parse_message_ids("source_message_ids", &args.source_message_ids) {
                Ok(ids) => ids,
                Err(error) => return Ok(MemoryStoreOutput::failure(error)),
            };

        match self
            .client
            .store(&args.information, self.channel_id, &source_message_ids)
            .await
        {
            Ok(point_id) => {
                tracing::info!(
                    point_id,
                    channel_id = self.channel_id,
                    sources = source_message_ids.len(),
                    "memory_store completed"
                );
                Ok(MemoryStoreOutput {
                    success: true,
                    point_id: Some(point_id),
                    error: None,
                })
            }
            Err(e) => {
                tracing::error!(error = %e, channel_id = self.channel_id, "memory_store failed");
                Ok(MemoryStoreOutput::failure(format!(
                    "Failed to store the memory: {e}"
                )))
            }
        }
    }
}
