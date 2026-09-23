use super::vector_client::{SharedVectorClient, VectorClientError};
use crate::discord::message::parse_message_ids;
use rig::tool::PortableTool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

#[derive(Clone)]
pub struct MemoryUpdateTool {
    pub client: SharedVectorClient,
    pub channel_id: u64, // Discord channel ID
}

impl MemoryUpdateTool {
    pub fn new_with_client(client: SharedVectorClient, channel_id: u64) -> Self {
        Self { client, channel_id }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUpdateArgs {
    pub point_id: String,
    pub information: String,
    #[serde(default)]
    pub source_message_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUpdateOutput {
    pub success: bool,
    pub point_id: String,
    /// Every message the memory cites after the update
    pub source_message_ids: Vec<String>,
    pub error: Option<String>,
}

impl MemoryUpdateOutput {
    fn failure(point_id: String, error: String) -> Self {
        Self {
            success: false,
            point_id,
            source_message_ids: vec![],
            error: Some(error),
        }
    }
}

#[derive(Debug, Error)]
#[error("Memory update error: {0}")]
pub struct MemoryUpdateError(String);

impl PortableTool for MemoryUpdateTool {
    const NAME: &'static str = "memory_update";
    type Error = MemoryUpdateError;
    type Args = MemoryUpdateArgs;
    type Output = MemoryUpdateOutput;

    fn description(&self) -> String {
        format!(
            "Extend or correct an existing memory of channel {}: the new text replaces the old, and the messages you cite join the ones it already has. After updating, tell the channel in one short line via send_discord_message.",
            self.channel_id
        )
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "point_id": {
                    "type": "string",
                    "description": "The point_id of the memory, from memory_find."
                },
                "information": {
                    "type": "string",
                    "description": "The complete updated fact. It replaces the old text, so carry over whatever still holds."
                },
                "source_message_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "IDs from the [#ID] headers of the messages behind this update. Added to the sources the memory already cites."
                }
            },
            "required": ["point_id", "information", "source_message_ids"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let source_message_ids =
            match parse_message_ids("source_message_ids", &args.source_message_ids) {
                Ok(ids) => ids,
                Err(error) => return Ok(MemoryUpdateOutput::failure(args.point_id, error)),
            };

        match self
            .client
            .update(
                &args.point_id,
                &args.information,
                self.channel_id,
                &source_message_ids,
            )
            .await
        {
            Ok(metadata) => {
                tracing::info!(
                    point_id = %args.point_id,
                    channel_id = self.channel_id,
                    sources = metadata.source_message_ids.len(),
                    "memory_update completed"
                );
                Ok(MemoryUpdateOutput {
                    success: true,
                    point_id: args.point_id,
                    source_message_ids: metadata
                        .source_message_ids
                        .iter()
                        .map(u64::to_string)
                        .collect(),
                    error: None,
                })
            }
            Err(VectorClientError::NotFound(_)) => Ok(MemoryUpdateOutput::failure(
                args.point_id.clone(),
                format!(
                    "no memory has the point_id {:?}; ids come from memory_find, and retrying with the same one will not help",
                    args.point_id
                ),
            )),
            Err(e) => {
                tracing::error!(error = %e, point_id = %args.point_id, "memory_update failed");
                Ok(MemoryUpdateOutput::failure(
                    args.point_id,
                    format!("Failed to update the memory: {e}"),
                ))
            }
        }
    }
}
