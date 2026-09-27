//! `memory_search`: keyword search through the memory log, for what the memory notes leave out

use rig::tool::PortableTool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use serenity::all::ChannelId;

use crate::discord::memory::{ChannelMemory, FoundObservation};

/// Results when the agent doesn't ask for a count
const DEFAULT_LIMIT: u64 = 10;
/// Tool results are resent with every model turn for the rest of the session, so pages stay small
const MAX_LIMIT: u64 = 20;

#[derive(Clone)]
pub struct MemorySearchTool {
    pub memory: ChannelMemory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchArgs {
    pub query: String,
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryResult {
    pub content: String,
    /// Names of the people it is about
    pub about: Vec<String>,
    pub observed_on: String,
    /// As strings, ready to be passed to the message tools. Only for what was recorded in this
    /// channel: the message tools can't reach the others.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_message_ids: Vec<String>,
}

impl MemoryResult {
    fn new(found: FoundObservation, channel_id: ChannelId) -> Self {
        let source_message_ids = if found.channel_id == channel_id {
            found
                .source_message_ids
                .iter()
                .map(u64::to_string)
                .collect()
        } else {
            vec![]
        };
        Self {
            content: found.content,
            about: found.about,
            observed_on: found.observed_at.date_naive().to_string(),
            source_message_ids,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchOutput {
    pub success: bool,
    pub results: Vec<MemoryResult>,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Memory search error: {0}")]
pub struct MemorySearchError(String);

impl PortableTool for MemorySearchTool {
    const NAME: &'static str = "memory_search";
    type Error = MemorySearchError;
    type Args = MemorySearchArgs;
    type Output = MemorySearchOutput;

    fn description(&self) -> String {
        "Search every observation recorded about the people of this server and about this \
         channel, for what the memory notes leave out: older, rarer, or finer details. It \
         matches words, not meaning, best matches first. Results recorded in this channel carry \
         source_message_ids that reopen the original conversation with fetch_channel_history \
         (direction around)."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "A few distinctive words (names, nicknames, places, things) in the language the chat used them; any of them may match, accents are optional, and close misspellings still match. When nothing comes back, other words or the other language may find it."
                },
                "limit": {
                    "type": ["integer", "null"],
                    "description": "Result cap (default 10, max 20)."
                }
            },
            "required": ["query", "limit"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

        match self
            .memory
            .store
            .search(
                self.memory.guild_id,
                self.memory.channel_id,
                &args.query,
                limit.cast_signed(),
            )
            .await
        {
            Ok(found) => {
                tracing::info!(
                    found = found.len(),
                    channel_id = self.memory.channel_id.get(),
                    query = %args.query,
                    "memory_search completed"
                );
                Ok(MemorySearchOutput {
                    success: true,
                    results: found
                        .into_iter()
                        .map(|found| MemoryResult::new(found, self.memory.channel_id))
                        .collect(),
                    error: None,
                })
            }
            Err(e) => {
                tracing::error!(
                    ?e,
                    channel_id = self.memory.channel_id.get(),
                    "memory_search failed"
                );
                Ok(MemorySearchOutput {
                    success: false,
                    results: vec![],
                    error: Some(format!("The memory is unavailable: {e}")),
                })
            }
        }
    }
}
