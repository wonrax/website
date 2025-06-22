use super::qdrant_shared::{QdrantConfig, QdrantSharedClient};
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Clone)]
pub struct QdrantUpdateTool {
    pub config: QdrantConfig,
    pub channel_id: u64, // Discord channel ID
}

impl QdrantUpdateTool {
    pub fn new_from_config(config: QdrantConfig, channel_id: u64) -> Self {
        let mut config_with_channel = config;
        config_with_channel.channel_id = Some(channel_id);
        Self {
            config: config_with_channel,
            channel_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantUpdateArgs {
    pub point_id: String,
    pub information: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantUpdateOutput {
    pub success: bool,
    pub point_id: String,
    pub message: String,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Qdrant update error: {0}")]
pub struct QdrantUpdateError(String);

impl Tool for QdrantUpdateTool {
    const NAME: &'static str = "qdrant_update";
    type Error = QdrantUpdateError;
    type Args = QdrantUpdateArgs;
    type Output = QdrantUpdateOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let properties = json!({
            "point_id": {
                "type": "string",
                "description": "The point ID of the existing memory to update (obtained from qdrant_find results)"
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
            name: "qdrant_update".to_string(),
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
        // Clone config and args for the async operation
        let config = self.config.clone();
        let point_id = args.point_id.clone();
        let information = args.information.clone();
        let metadata = args.metadata.clone();

        // Spawn the async work in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move {
            // Create client for this call
            let client = QdrantSharedClient::new(config)
                .await
                .map_err(|e| QdrantUpdateError(format!("Failed to create Qdrant client: {}", e)))?;

            // Use None for collection_name since it's hardcoded via channel_id in the config
            client
                .update(&point_id, &information, None, metadata)
                .await
                .map_err(|e| QdrantUpdateError(format!("Failed to update information: {}", e)))?;

            let collection_used = client
                .get_collection_name(None)
                .unwrap_or_else(|_| "unknown".to_string());

            Ok::<QdrantUpdateOutput, QdrantUpdateError>(QdrantUpdateOutput {
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
            Err(e) => Ok(QdrantUpdateOutput {
                success: false,
                point_id: args.point_id,
                message: "Failed to update information".to_string(),
                error: Some(format!("Task execution error: {}", e)),
            }),
        }
    }
}
