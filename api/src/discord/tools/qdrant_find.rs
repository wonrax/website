use super::qdrant_shared::{QdrantConfig, QdrantSharedClient, SearchResult};
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Clone)]
pub struct QdrantFindTool {
    pub config: QdrantConfig,
    pub limit: u64,
    pub channel_id: u64, // Discord channel ID
}

impl QdrantFindTool {
    pub fn new_from_config(config: QdrantConfig, channel_id: u64, limit: Option<u64>) -> Self {
        let mut config_with_channel = config;
        config_with_channel.channel_id = Some(channel_id);
        Self {
            config: config_with_channel,
            limit: limit.unwrap_or(5), // Default to top 5 results
            channel_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantFindArgs {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantResult {
    pub point_id: String,
    pub content: String,
    pub score: f32,
    pub metadata: Option<Value>,
    pub timestamp: Option<String>,
}

impl From<SearchResult> for QdrantResult {
    fn from(result: SearchResult) -> Self {
        Self {
            point_id: result.point_id,
            content: result.content,
            score: result.score,
            metadata: result.metadata,
            timestamp: result.timestamp,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantFindOutput {
    pub success: bool,
    pub results: Vec<QdrantResult>,
    pub total_found: usize,
    pub query: String,
    pub collection: String,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Qdrant find error: {0}")]
pub struct QdrantFindError(String);

impl Tool for QdrantFindTool {
    const NAME: &'static str = "qdrant_find";
    type Error = QdrantFindError;
    type Args = QdrantFindArgs;
    type Output = QdrantFindOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let properties = json!({
            "query": {
                "type": "string",
                "description": "Query to search for in the vector database"
            },
            "limit": {
                "type": "integer",
                "description": "Maximum number of results to return (default: 5)",
                "minimum": 1,
                "maximum": 20
            }
        });

        let required = vec!["query"];

        ToolDefinition {
            name: "qdrant_find".to_string(),
            description: format!(
                "Retrieve relevant stored information from channel {} based on semantic similarity. Use this to find past conversations, user preferences, or relevant context.",
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
        let query = args.query.clone();
        let limit = args.limit.unwrap_or(self.limit);

        tracing::info!(
            "Starting qdrant_find for query: '{}' with limit: {}",
            query,
            limit
        );

        // Spawn the async work in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move {
            tracing::debug!("Creating Qdrant client...");

            // Create client for this call with better error handling
            let client = match QdrantSharedClient::new(config).await {
                Ok(client) => {
                    tracing::debug!("Qdrant client created successfully");
                    client
                }
                Err(e) => {
                    tracing::error!("Failed to create Qdrant client: {}", e);
                    return Err(QdrantFindError(format!(
                        "Failed to create Qdrant client: {}",
                        e
                    )));
                }
            };

            tracing::debug!("Starting search operation...");

            // Use None for collection_name since it's hardcoded via channel_id in the config
            let results = match client.search(&query, None, limit).await {
                Ok(results) => {
                    tracing::debug!(
                        "Search completed successfully, found {} results",
                        results.len()
                    );
                    results
                }
                Err(e) => {
                    tracing::error!("Search operation failed (Qdrant may be unavailable): {}", e);
                    return Ok(QdrantFindOutput {
                        success: false,
                        results: vec![],
                        total_found: 0,
                        query: query.clone(),
                        collection: "unknown".to_string(),
                        error: Some(format!("Qdrant unavailable: {}", e)),
                    });
                }
            };

            let collection_used = client
                .get_collection_name(None)
                .unwrap_or_else(|_| "unknown".to_string());

            let qdrant_results: Vec<QdrantResult> =
                results.into_iter().map(QdrantResult::from).collect();
            let total_found = qdrant_results.len();

            tracing::info!(
                "qdrant_find completed: found {} results in collection '{}'",
                total_found,
                collection_used
            );

            Ok::<QdrantFindOutput, QdrantFindError>(QdrantFindOutput {
                success: true,
                results: qdrant_results,
                total_found,
                query: query.clone(),
                collection: collection_used,
                error: None,
            })
        });

        match handle.await {
            Ok(result) => {
                tracing::debug!("qdrant_find task completed successfully");
                result
            }
            Err(e) => {
                tracing::error!("qdrant_find task panicked or was cancelled: {}", e);
                Ok(QdrantFindOutput {
                    success: false,
                    results: vec![],
                    total_found: 0,
                    query: args.query,
                    collection: "unknown".to_string(),
                    error: Some(format!("Task execution error: {}", e)),
                })
            }
        }
    }
}
