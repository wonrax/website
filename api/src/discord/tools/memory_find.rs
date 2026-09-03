use super::vector_client::{SearchResult, SharedVectorClient};
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Clone)]
pub struct MemoryFindTool {
    pub client: SharedVectorClient,
    pub limit: u64,
    pub channel_id: u64, // Discord channel ID
}

impl MemoryFindTool {
    pub fn new_with_client(
        client: SharedVectorClient,
        channel_id: u64,
        limit: Option<u64>,
    ) -> Self {
        Self {
            client,
            limit: limit.unwrap_or(10),
            channel_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFindArgs {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryResult {
    pub point_id: String,
    pub content: String,
    pub score: f32,
    pub metadata: Option<Value>,
    pub timestamp: Option<String>,
}

impl From<SearchResult> for MemoryResult {
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
pub struct MemoryFindOutput {
    pub success: bool,
    pub results: Vec<MemoryResult>,
    pub total_found: usize,
    pub query: String,
    pub collection: String,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Memory find error: {0}")]
pub struct MemoryFindError(String);

impl Tool for MemoryFindTool {
    const NAME: &'static str = "memory_find";
    type Error = MemoryFindError;
    type Args = MemoryFindArgs;
    type Output = MemoryFindOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let properties = json!({
            "query": {
                "type": "string",
                "description": "Natural-language query: a topic from the new messages, or an author's username."
            },
            "limit": {
                "type": ["integer", "null"],
                "description": "Result cap (default 10, max 20). Scale it with how much the response depends on what you remember."
            }
        });

        let required = vec!["query", "limit"];

        ToolDefinition {
            name: "memory_find".to_string(),
            description: format!(
                "Semantic search over channel {}'s memories: everything you know about these users and this channel. Query it for the new messages' authors and topics before deciding whether and how to respond, and once per session for the channel's chat preferences. Skip queries already answered in this session's tool history. Each result carries a 0.0-1.0 relevance score and the point id needed by memory_update/memory_delete. Retrieval is silent: never announce it in the channel.",
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
        let query = args.query.clone();
        let limit = args.limit.unwrap_or(self.limit);
        let channel_id = self.channel_id;
        let collection_used = client.get_collection_name(self.channel_id);

        // Spawn the async work in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move {
            // Use None for collection_name since it's hardcoded via channel_id in the config
            let results = match client.search(&query, channel_id, limit).await {
                Ok(results) => results,
                Err(e) => {
                    return Ok(MemoryFindOutput {
                        success: false,
                        results: vec![],
                        total_found: 0,
                        query: query.clone(),
                        collection: "unknown".to_string(),
                        error: Some(format!("Vector database unavailable: {}", e)),
                    });
                }
            };

            let memory_results: Vec<MemoryResult> =
                results.into_iter().map(MemoryResult::from).collect();
            let total_found = memory_results.len();

            tracing::info!(
                "memory_find completed: found {} results in collection '{}' for query '{}'",
                total_found,
                collection_used,
                query
            );

            Ok::<MemoryFindOutput, MemoryFindError>(MemoryFindOutput {
                success: true,
                results: memory_results,
                total_found,
                query: query.clone(),
                collection: collection_used,
                error: None,
            })
        });

        match handle.await {
            Ok(result) => result,
            Err(e) => Ok(MemoryFindOutput {
                success: false,
                results: vec![],
                total_found: 0,
                query: args.query,
                collection: "unknown".to_string(),
                error: Some(format!("Task execution error: {}", e)),
            }),
        }
    }
}
