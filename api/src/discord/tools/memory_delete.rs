use super::vector_client::SharedVectorClient;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Clone)]
pub struct MemoryDeleteTool {
    pub client: SharedVectorClient,
    pub channel_id: u64, // Discord channel ID
}

impl MemoryDeleteTool {
    pub fn new_with_client(client: SharedVectorClient, channel_id: u64) -> Self {
        Self { client, channel_id }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDeleteArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub where_metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub where_document: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDeleteOutput {
    pub success: bool,
    pub deleted_count: Option<u32>,
    pub message: String,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Memory delete error: {0}")]
pub struct MemoryDeleteError(String);

impl Tool for MemoryDeleteTool {
    const NAME: &'static str = "memory_delete";
    type Error = MemoryDeleteError;
    type Args = MemoryDeleteArgs;
    type Output = MemoryDeleteOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let properties = json!({
            "ids": {
                "type": "array",
                "items": {
                    "type": "string"
                },
                "description": "Array of memory IDs to delete. Use this when you know specific memory IDs to remove."
            },
            "where_metadata": {
                "type": "object",
                "description": "Filter for deletion by metadata. E.g. {\"timestamp\": {\"$lt\": \"2023-01-01\"}} to delete old memories.",
                "additionalProperties": true
            },
            "where_document": {
                "type": "object",
                "description": "Filter for deletion by document content. E.g. {\"$contains\": \"some text\"} to delete memories containing specific text.",
                "additionalProperties": true
            }
        });

        ToolDefinition {
            name: "memory_delete".to_string(),
            description: format!(
                "Delete stored memories from the vector database for channel {}. Use this to remove outdated, incorrect, or no longer relevant information. You can delete by specific IDs, metadata filters, or document content filters. BE CAREFUL - deletions are permanent.",
                self.channel_id
            ),
            parameters: json!({
                "type": "object",
                "properties": properties,
                "required": []
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let client = self.client.clone();
        let channel_id = self.channel_id;
        let collection_used = client.get_collection_name(self.channel_id);

        // Validate that at least one deletion criteria is provided
        if args.ids.is_none() && args.where_metadata.is_none() && args.where_document.is_none() {
            return Ok(MemoryDeleteOutput {
                success: false,
                deleted_count: None,
                message: "No deletion criteria provided. Must specify at least one of: ids, where_metadata, or where_document".to_string(),
                error: Some("Missing deletion criteria".to_string()),
            });
        }

        // Convert Vec<String> to Vec<&str> for the delete API - clone for thread safety
        let ids_refs: Option<Vec<String>> = args.ids.clone();
        let where_metadata = args.where_metadata.clone();
        let where_document = args.where_document.clone();

        // Spawn the async work in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move {
            // Convert to string slices inside the async block
            let ids_slice: Option<Vec<&str>> = ids_refs
                .as_ref()
                .map(|ids| ids.iter().map(|s| s.as_str()).collect());

            match client
                .delete(channel_id, ids_slice, where_metadata, where_document)
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        "memory_delete completed successfully: deleted from collection '{}'",
                        collection_used
                    );

                    Ok::<MemoryDeleteOutput, MemoryDeleteError>(MemoryDeleteOutput {
                        success: true,
                        deleted_count: None, // ChromaDB doesn't return count in current implementation
                        message: format!(
                            "Memory deletion completed successfully in collection '{}'",
                            collection_used
                        ),
                        error: None,
                    })
                }
                Err(e) => {
                    tracing::error!(
                        "Delete operation failed (vector database may be unavailable): {}",
                        e
                    );
                    Ok(MemoryDeleteOutput {
                        success: false,
                        deleted_count: None,
                        message:
                            "Failed to delete memories - vector database server may be unavailable"
                                .to_string(),
                        error: Some(format!("Vector database error: {}", e)),
                    })
                }
            }
        });

        match handle.await {
            Ok(result) => result,
            Err(e) => Ok(MemoryDeleteOutput {
                success: false,
                deleted_count: None,
                message: "Failed to delete memories".to_string(),
                error: Some(format!("Task execution error: {}", e)),
            }),
        }
    }
}
