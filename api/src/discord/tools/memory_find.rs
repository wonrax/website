use super::vector_client::{SearchResult, SharedVectorClient};
use rig::tool::PortableTool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

/// Results when the agent doesn't ask for a count
const DEFAULT_LIMIT: u64 = 10;
/// Tool results are resent with every model turn for the rest of the session, so pages stay small
const MAX_LIMIT: u64 = 20;

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
            limit: limit.unwrap_or(DEFAULT_LIMIT),
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
    pub stored_at: Option<String>,
    /// As strings, ready to be passed to the message tools
    pub source_message_ids: Vec<String>,
}

impl From<SearchResult> for MemoryResult {
    fn from(result: SearchResult) -> Self {
        Self {
            point_id: result.point_id,
            content: result.content,
            score: result.score,
            stored_at: result.metadata.stored_at,
            source_message_ids: result
                .metadata
                .source_message_ids
                .iter()
                .map(u64::to_string)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFindOutput {
    pub success: bool,
    pub results: Vec<MemoryResult>,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Memory find error: {0}")]
pub struct MemoryFindError(String);

impl PortableTool for MemoryFindTool {
    const NAME: &'static str = "memory_find";
    type Error = MemoryFindError;
    type Args = MemoryFindArgs;
    type Output = MemoryFindOutput;

    fn description(&self) -> String {
        format!(
            "Semantic search over channel {}'s memories: everything you know about these users and this channel. Skip queries already answered in this session's tool history. Each result carries a 0.0-1.0 relevance score, the point_id that memory_update and memory_delete take, and source_message_ids: the messages the memory came from, which fetch_channel_history (direction around) rereads in detail. Retrieval is silent: never announce it in the channel.",
            self.channel_id
        )
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language: a topic from the new messages, an author's username, or, once per session, the channel's chat preferences."
                },
                "limit": {
                    "type": ["integer", "null"],
                    "description": "Result cap (default 10, max 20). Scale it with how much the response depends on what you remember."
                }
            },
            "required": ["query", "limit"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let limit = args.limit.unwrap_or(self.limit).clamp(1, MAX_LIMIT);

        match self
            .client
            .search(&args.query, self.channel_id, limit)
            .await
        {
            Ok(results) => {
                tracing::info!(
                    found = results.len(),
                    channel_id = self.channel_id,
                    query = %args.query,
                    "memory_find completed"
                );
                Ok(MemoryFindOutput {
                    success: true,
                    results: results.into_iter().map(MemoryResult::from).collect(),
                    error: None,
                })
            }
            Err(e) => {
                tracing::error!(error = %e, channel_id = self.channel_id, "memory_find failed");
                Ok(MemoryFindOutput {
                    success: false,
                    results: vec![],
                    error: Some(format!("Vector database unavailable: {e}")),
                })
            }
        }
    }
}
