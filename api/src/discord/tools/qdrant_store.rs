use super::qdrant_shared::{QdrantConfig, QdrantSharedClient};
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Clone)]
pub struct QdrantStoreTool {
    pub config: QdrantConfig,
    pub channel_id: u64, // Discord channel ID
}

impl QdrantStoreTool {
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
pub struct QdrantStoreArgs {
    pub information: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantStoreOutput {
    pub success: bool,
    pub point_id: Option<String>,
    pub message: String,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Qdrant store error: {0}")]
pub struct QdrantStoreError(String);

impl Tool for QdrantStoreTool {
    const NAME: &'static str = "qdrant_store";
    type Error = QdrantStoreError;
    type Args = QdrantStoreArgs;
    type Output = QdrantStoreOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let properties = json!({
            "information": {
                "type": "string",
                "description": "Information to store in the vector database"
            },
            "metadata": {
                "type": "object",
                "description": "Optional metadata to store with the information",
                "additionalProperties": true
            }
        });

        let required = vec!["information"];

        ToolDefinition {
            name: "qdrant_store".to_string(),
            description: format!(
                "Store information in the vector database for channel {}. Use this to save important details about users, conversations, preferences, or interesting facts for future reference in this channel.",
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
        let information = args.information.clone();
        let metadata = args.metadata.clone();

        tracing::info!(
            "Starting qdrant_store for: '{}'",
            information.chars().take(50).collect::<String>()
        );

        // Spawn the async work in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move {
            // Create client for this call with better error handling
            let client = match QdrantSharedClient::new(config).await {
                Ok(client) => client,
                Err(e) => {
                    return Err(QdrantStoreError(format!(
                        "Failed to create Qdrant client: {}",
                        e
                    )));
                }
            };

            tracing::debug!("Starting store operation...");

            // Use None for collection_name since it's hardcoded via channel_id in the config
            let point_id = match client.store(&information, None, metadata).await {
                Ok(point_id) => {
                    tracing::debug!(
                        "Store operation completed successfully, point_id: {}",
                        point_id
                    );
                    point_id
                }
                Err(e) => {
                    tracing::error!("Store operation failed (Qdrant may be unavailable): {}", e);
                    return Ok(QdrantStoreOutput {
                        success: false,
                        point_id: None,
                        message: "Failed to store information - Qdrant server unavailable"
                            .to_string(),
                        error: Some(format!("Qdrant unavailable: {}", e)),
                    });
                }
            };

            let collection_used = client
                .get_collection_name(None)
                .unwrap_or_else(|_| "unknown".to_string());

            tracing::info!(
                "qdrant_store completed successfully: stored in collection '{}'",
                collection_used
            );

            Ok::<QdrantStoreOutput, QdrantStoreError>(QdrantStoreOutput {
                success: true,
                point_id: Some(point_id),
                message: format!(
                    "Information stored successfully in collection '{}'",
                    collection_used
                ),
                error: None,
            })
        });

        match handle.await {
            Ok(result) => result,
            Err(e) => {
                tracing::error!("qdrant_store task panicked or was cancelled: {}", e);
                Ok(QdrantStoreOutput {
                    success: false,
                    point_id: None,
                    message: "Failed to store information".to_string(),
                    error: Some(format!("Task execution error: {}", e)),
                })
            }
        }
    }
}
