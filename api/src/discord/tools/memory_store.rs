use super::vector_client::SharedVectorClient;
use chrono;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStoreOutput {
    pub success: bool,
    pub point_id: Option<String>,
    pub message: String,
    pub error: Option<String>,
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
            name: "memory_store".to_string(),
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
        let client = self.client.clone();
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
            let point_id = match client.store(&information, channel_id, metadata).await {
                Ok(point_id) => {
                    tracing::debug!(
                        "Store operation completed successfully, point_id: {}",
                        point_id
                    );
                    point_id
                }
                Err(e) => {
                    tracing::error!(
                        "Store operation failed (vector database may be unavailable): {}",
                        e
                    );
                    return Ok(MemoryStoreOutput {
                        success: false,
                        point_id: None,
                        message: "Failed to store information - vector database server unavailable"
                            .to_string(),
                        error: Some(format!("Vector database unavailable: {}", e)),
                    });
                }
            };

            tracing::info!(
                "memory_store completed successfully: stored in collection '{}'",
                collection_used
            );

            Ok::<MemoryStoreOutput, MemoryStoreError>(MemoryStoreOutput {
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
            Err(e) => Ok(MemoryStoreOutput {
                success: false,
                point_id: None,
                message: "Failed to store information".to_string(),
                error: Some(format!("Task execution error: {}", e)),
            }),
        }
    }
}
