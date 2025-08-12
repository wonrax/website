use super::vector_client::SharedVectorClient;
use chrono;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUpdateOutput {
    pub success: bool,
    pub point_id: String,
    pub message: String,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Memory update error: {0}")]
pub struct MemoryUpdateError(String);

impl Tool for MemoryUpdateTool {
    const NAME: &'static str = "memory_update";
    type Error = MemoryUpdateError;
    type Args = MemoryUpdateArgs;
    type Output = MemoryUpdateOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let properties = json!({
            "point_id": {
                "type": "string",
                "description": "The point ID of the existing memory to update (obtained from memory_find results)"
            },
            "information": {
                "type": "string",
                "description": "Updated information to replace the existing memory content"
            },
            "metadata": {
                "type": "object",
                "description": "Optional updated metadata to store with the information",
                "additionalProperties": true
            }
        });

        let required = vec!["point_id", "information"];

        ToolDefinition {
            name: "memory_update".to_string(),
            description: format!(
                "Update existing information in the vector database for channel {}. Use this to modify or correct previously stored memories based on new information or corrections.",
                self.channel_id
            ),
            parameters: json!({
                "type": "object",
                "properties": properties,
                "required": required
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let client = self.client.clone();
        let point_id = args.point_id.clone();
        let information = args.information.clone();
        let mut metadata = args.metadata.unwrap_or_else(|| serde_json::json!({}));
        let channel_id = self.channel_id;
        let collection_used = client.get_collection_name(self.channel_id);

        // Add timestamp to metadata
        if let serde_json::Value::Object(ref mut obj) = metadata {
            obj.insert(
                "timestamp".to_string(),
                serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
            );
        }
        let metadata = Some(metadata);

        // Spawn the async work in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move {
            // Use None for collection_name since it's hardcoded via channel_id in the config
            client
                .update(&point_id, &information, channel_id, metadata)
                .await
                .map_err(|e| MemoryUpdateError(format!("Failed to update information: {}", e)))?;

            tracing::debug!(
                "Update operation completed successfully, point_id: {}",
                point_id,
            );

            Ok::<MemoryUpdateOutput, MemoryUpdateError>(MemoryUpdateOutput {
                success: true,
                point_id: point_id.clone(),
                message: format!(
                    "Information updated successfully in collection '{}'",
                    collection_used
                ),
                error: None,
            })
        });

        match handle.await {
            Ok(result) => result,
            Err(e) => Ok(MemoryUpdateOutput {
                success: false,
                point_id: args.point_id,
                message: "Failed to update information".to_string(),
                error: Some(format!("Task execution error: {}", e)),
            }),
        }
    }
}
