//! `memory_search`: keyword search through the memory log, for what the memory notes leave out

use std::num::NonZeroU64;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use rig::tool::PortableTool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::{ChannelId, UserId};
use thiserror::Error;

use crate::discord::memory::{ChannelMemory, FoundObservation, Search};

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
    pub about: Vec<String>,
    #[serde(default)]
    pub before: Option<String>,
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
                    "description": "Names, nicknames, and slang as the chat writes them, and the topic's key words in both English and the chat's language: an observation is written in one and keyed in the other. Any word may match; the rarer it is in the memory the more it counts, and words written next to each other count more together. Accents are optional, and close misspellings still match."
                },
                "about": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only observations about every one of these people, by username or user ID, for when a name alone matches too much. Empty to search everyone."
                },
                "before": {
                    "type": ["string", "null"],
                    "description": "Only observations from before this day (YYYY-MM-DD): how things stood back then, or older ones the newer crowd out."
                },
                "limit": {
                    "type": ["integer", "null"],
                    "description": "Result cap (default 10, max 20)."
                }
            },
            "required": ["query", "about", "before", "limit"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let (about, before) = match self.filters(&args).await {
            Ok(filters) => filters,
            Err(error) => {
                return Ok(MemorySearchOutput {
                    success: false,
                    results: vec![],
                    error: Some(error),
                });
            }
        };

        let search = Search {
            guild_id: self.memory.guild_id,
            channel_id: self.memory.channel_id,
            query: &args.query,
            about: &about,
            before,
            limit: limit.cast_signed(),
        };
        match self.memory.store.search(&search).await {
            Ok(found) => {
                tracing::info!(
                    found = found.len(),
                    channel_id = self.memory.channel_id.get(),
                    query = %args.query,
                    about = ?args.about,
                    before = ?args.before,
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

impl MemorySearchTool {
    /// The people `about` names, and the start of the day `before` names
    async fn filters(
        &self,
        args: &MemorySearchArgs,
    ) -> Result<(Vec<UserId>, Option<DateTime<Utc>>), String> {
        let mut about = Vec::new();
        for name in &args.about {
            let name = name.trim().trim_start_matches('@');
            let user_id = match name.parse::<u64>().ok().and_then(NonZeroU64::new) {
                Some(user_id) => UserId::from(user_id),
                None => match self
                    .memory
                    .store
                    .find_person(self.memory.guild_id, name)
                    .await
                {
                    Ok(Some(user_id)) => user_id,
                    Ok(None) => {
                        return Err(format!(
                            "the memory knows no one called {name:?}; give their user ID"
                        ));
                    }
                    Err(e) => return Err(format!("could not look up {name:?}: {e}")),
                },
            };
            about.push(user_id);
        }

        let before = match args.before.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(day) => {
                let day = NaiveDate::parse_from_str(day, "%Y-%m-%d")
                    .map_err(|_| format!("before is {day:?}, not a day like 2026-09-01"))?;
                Some(day.and_time(NaiveTime::MIN).and_utc())
            }
        };
        Ok((about, before))
    }
}
