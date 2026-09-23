//! `search_channel_messages`: Discord's guild message search, scoped to the agent's channel.
//! Discord opened the endpoint to bots in March 2026 and serenity 0.12 has no route for it, so
//! the request goes straight to the REST API with the bot's token.

use crate::discord::{
    constants::URL_FETCH_TIMEOUT_SECS,
    message::{format_message_compact, snowflake_at},
};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use reqwest::{StatusCode, header::AUTHORIZATION};
use rig::tool::PortableTool;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use serenity::all::{ChannelId, Context, GuildId, Message, UserId};
use std::{sync::Arc, time::Duration};
use thiserror::Error;

/// Hits per page when the agent doesn't ask for a count
const DEFAULT_LIMIT: u32 = 10;
/// Discord's per-request cap
const MAX_LIMIT: u32 = 25;
/// Discord's cap on `offset`
const MAX_OFFSET: u32 = 9975;
/// Discord's cap on `content`
const MAX_QUERY_CHARS: usize = 1024;
/// Tries per search: the index may answer 202 (not ready) or 429 with a `retry_after`
const MAX_ATTEMPTS: u32 = 3;
/// Bounds on one `retry_after` wait so a slow index neither stalls the agent nor gets hammered
const MIN_RETRY_WAIT: Duration = Duration::from_millis(500);
const MAX_RETRY_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct SearchChannelMessagesTool {
    pub ctx: Arc<Context>,
    pub channel_id: ChannelId,
    pub bot_user_id: UserId,
    client: reqwest::Client,
}

impl SearchChannelMessagesTool {
    pub fn new(
        ctx: Arc<Context>,
        channel_id: ChannelId,
        bot_user_id: UserId,
    ) -> Result<Self, reqwest::Error> {
        // Discord wants the DiscordBot user agent on every REST request
        let client = reqwest::Client::builder()
            .user_agent(serenity::constants::USER_AGENT)
            .timeout(URL_FETCH_TIMEOUT_SECS)
            .build()?;
        Ok(Self {
            ctx,
            channel_id,
            bot_user_id,
            client,
        })
    }

    /// One search with bounded retries for a not-yet-ready index (202) and rate limits (429)
    async fn request(
        &self,
        guild_id: GuildId,
        params: &[(&str, String)],
    ) -> Result<SearchResponse, String> {
        let url = url::Url::parse_with_params(
            &format!("https://discord.com/api/v10/guilds/{guild_id}/messages/search"),
            params,
        )
        .map_err(|e| format!("could not build the search URL: {e}"))?;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let response = self
                .client
                .get(url.clone())
                .header(AUTHORIZATION, self.ctx.http.token())
                .send()
                .await
                .map_err(|e| format!("search request failed: {e}"))?;
            let status = response.status();
            let body: Value = response
                .json()
                .await
                .map_err(|e| format!("Discord answered {status} with an unreadable body: {e}"))?;
            let detail = body
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no details")
                .to_string();

            if status == StatusCode::ACCEPTED || status == StatusCode::TOO_MANY_REQUESTS {
                let cause = if status == StatusCode::ACCEPTED {
                    "Discord is still indexing this server"
                } else {
                    "Discord rate limited the search"
                };
                if attempt >= MAX_ATTEMPTS {
                    return Err(format!("{cause} ({detail}); try again in a minute"));
                }
                let wait = body
                    .get("retry_after")
                    .and_then(Value::as_f64)
                    .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
                    .unwrap_or(MAX_RETRY_WAIT)
                    .clamp(MIN_RETRY_WAIT, MAX_RETRY_WAIT);
                tracing::debug!(%status, ?wait, attempt, "Search not answered yet, waiting");
                tokio::time::sleep(wait).await;
                continue;
            }
            if !status.is_success() {
                return Err(format!("Discord refused the search ({status}): {detail}"));
            }
            return serde_json::from_value(body)
                .map_err(|e| format!("could not parse Discord's search response: {e}"));
        }
    }

    fn render(&self, message: &Message) -> String {
        let mut line = format_message_compact(message, self.bot_user_id);
        if message.channel_id != self.channel_id {
            // Hits can sit in one of this channel's threads, which the channel-bound tools
            // can't open
            line.push_str(&format!(" [in thread {}]", message.channel_id));
        }
        line
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchOrder {
    Relevance,
    NewestFirst,
    OldestFirst,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchChannelMessagesArgs {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub author_id: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub sort_by: Option<SearchOrder>,
    #[serde(default)]
    pub offset: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchChannelMessagesOutput {
    pub success: bool,
    /// Formatted hits in the requested order
    pub messages: String,
    pub count: usize,
    pub total_results: u64,
    /// `offset` for the next page, when more hits remain
    pub next_offset: Option<u32>,
    pub error: Option<String>,
}

impl SearchChannelMessagesOutput {
    fn failure(error: String) -> Self {
        Self {
            success: false,
            messages: String::new(),
            count: 0,
            total_results: 0,
            next_offset: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Error)]
#[error("Search channel messages error: {0}")]
pub struct SearchChannelMessagesError(String);

/// What Discord returns for a search. Hits come nested one level deep for historical reasons.
#[derive(Debug, Deserialize)]
struct SearchResponse {
    total_results: u64,
    messages: Vec<Vec<Value>>,
}

/// Parses a time the agent copied from a message header (RFC 3339) or wrote as a date
fn parse_time(param: &str, raw: &str) -> Result<DateTime<Utc>, String> {
    if let Ok(time) = DateTime::parse_from_rfc3339(raw) {
        return Ok(time.with_timezone(&Utc));
    }
    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Ok(date.and_time(NaiveTime::MIN).and_utc());
    }
    Err(format!(
        "{param} must be an ISO 8601 timestamp as in message headers, or a YYYY-MM-DD date, got {raw:?}; retrying with the same value will not help"
    ))
}

/// Flattens Discord's nested hit arrays into messages, counting the ones that don't deserialize
fn parse_hits(messages: Vec<Vec<Value>>) -> (Vec<Message>, usize) {
    let mut unreadable = 0;
    let hits = messages
        .into_iter()
        .flatten()
        .filter_map(|value| {
            serde_json::from_value::<Message>(value)
                .map_err(|e| {
                    unreadable += 1;
                    tracing::warn!(?e, "Skipping a search hit that did not deserialize");
                })
                .ok()
        })
        .collect();
    (hits, unreadable)
}

/// Positions, not returned hits, drive paging: Discord may return fewer hits than `limit` and
/// says not to count on the array length
fn next_offset(offset: u32, limit: u32, total_results: u64) -> Option<u32> {
    let next = offset + limit;
    (u64::from(next) < total_results && next <= MAX_OFFSET).then_some(next)
}

impl PortableTool for SearchChannelMessagesTool {
    const NAME: &'static str = "search_channel_messages";
    type Error = SearchChannelMessagesError;
    type Args = SearchChannelMessagesArgs;
    type Output = SearchChannelMessagesOutput;

    fn description(&self) -> String {
        "Search this channel's whole history for messages outside your context, by words, author, or time. Reach for it when someone refers to something said earlier, when a memory names a topic but you need what was actually said, or to check whether something came up before. Hits use the same format and IDs as your context, so fetch_channel_history (direction around) rereads the conversation around one. Matching is by whole words, not meaning; what you know about people and topics lives in memory_find."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": ["string", "null"],
                    "description": "Words the message must contain, in the language it was written in. Word matching, not meaning: distinctive words, not a question. null to filter by author or time only."
                },
                "author_id": {
                    "type": ["string", "null"],
                    "description": "Discord user ID of the author (from fetch_message_user_ids), or null for any author."
                },
                "after": {
                    "type": ["string", "null"],
                    "description": "Only messages sent after this time: an ISO 8601 timestamp as in message headers, or a YYYY-MM-DD date (its start, UTC). null for no lower bound."
                },
                "before": {
                    "type": ["string", "null"],
                    "description": "Only messages sent before this time, same formats as after. null for no upper bound."
                },
                "sort_by": {
                    "type": ["string", "null"],
                    "enum": ["relevance", "newest_first", "oldest_first"],
                    "description": "null means relevance when query is set, newest_first otherwise."
                },
                "offset": {
                    "type": ["integer", "null"],
                    "description": "Hits to skip, for paging: the next_offset of the previous page. null starts from the first hit."
                },
                "limit": {
                    "type": ["integer", "null"],
                    "description": "Hits per page, 1-25. Default 10."
                }
            },
            "required": ["query", "author_id", "after", "before", "sort_by", "offset", "limit"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let offset = args.offset.unwrap_or(0).min(MAX_OFFSET);
        let query: Option<String> = args
            .query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .map(|query| query.chars().take(MAX_QUERY_CHARS).collect());

        let mut params: Vec<(&str, String)> = vec![
            ("channel_id", self.channel_id.to_string()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
            // Discord skips age-restricted channels by default, and this may be one
            ("include_nsfw", "true".to_string()),
        ];
        if let Some(query) = &query {
            params.push(("content", query.clone()));
        }
        if let Some(author_id) = args
            .author_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            // Tolerate the mention form the agent writes in messages
            let digits = author_id.trim_start_matches("<@").trim_end_matches('>');
            match digits.parse::<u64>() {
                Ok(id) => params.push(("author_id", id.to_string())),
                Err(_) => {
                    return Ok(SearchChannelMessagesOutput::failure(format!(
                        "author_id must be the digits of a Discord user ID, got {author_id:?}; fetch_message_user_ids has them, and retrying with the same value will not help"
                    )));
                }
            }
        }
        for (param, raw, key) in [
            ("after", args.after.as_deref(), "min_id"),
            ("before", args.before.as_deref(), "max_id"),
        ] {
            let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
                continue;
            };
            match parse_time(param, raw) {
                Ok(time) => params.push((key, snowflake_at(time).to_string())),
                Err(error) => return Ok(SearchChannelMessagesOutput::failure(error)),
            }
        }
        let order = args.sort_by.unwrap_or(if query.is_some() {
            SearchOrder::Relevance
        } else {
            SearchOrder::NewestFirst
        });
        match order {
            SearchOrder::Relevance => params.push(("sort_by", "relevance".to_string())),
            SearchOrder::NewestFirst => params.extend([
                ("sort_by", "timestamp".to_string()),
                ("sort_order", "desc".to_string()),
            ]),
            SearchOrder::OldestFirst => params.extend([
                ("sort_by", "timestamp".to_string()),
                ("sort_order", "asc".to_string()),
            ]),
        }

        // The endpoint is guild-wide; the channel filter above scopes it
        let guild_id = match self.channel_id.to_channel(&self.ctx.http).await {
            Ok(channel) => match channel.guild() {
                Some(channel) => channel.guild_id,
                None => {
                    return Ok(SearchChannelMessagesOutput::failure(
                        "search only works in server channels, and this is a direct message"
                            .to_string(),
                    ));
                }
            },
            Err(e) => {
                tracing::error!(?e, "Failed to resolve the channel's guild for search");
                return Ok(SearchChannelMessagesOutput::failure(format!(
                    "could not resolve this channel's server: {e}"
                )));
            }
        };

        let response = match self.request(guild_id, &params).await {
            Ok(response) => response,
            Err(error) => {
                tracing::error!(%error, "search_channel_messages failed");
                return Ok(SearchChannelMessagesOutput::failure(error));
            }
        };

        let (hits, unreadable) = parse_hits(response.messages);
        let mut rendered: Vec<String> = hits.iter().map(|hit| self.render(hit)).collect();
        if unreadable > 0 {
            rendered.push(format!(
                "[{unreadable} hits could not be read and were left out]"
            ));
        }
        let messages = if rendered.is_empty() {
            "[no messages match]".to_string()
        } else {
            rendered.join("\n")
        };

        tracing::debug!(
            query = query.as_deref().unwrap_or(""),
            offset,
            limit,
            count = hits.len(),
            total_results = response.total_results,
            "search_channel_messages completed"
        );

        Ok(SearchChannelMessagesOutput {
            success: true,
            messages,
            count: hits.len(),
            total_results: response.total_results,
            next_offset: next_offset(offset, limit, response.total_results),
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_parse_as_rfc3339_or_dates() {
        let at = parse_time("after", "2026-09-14T12:03:30.469Z").expect("rfc3339");
        assert_eq!(at.to_rfc3339(), "2026-09-14T12:03:30.469+00:00");

        let day = parse_time("before", "2026-09-14").expect("date");
        assert_eq!(day.to_rfc3339(), "2026-09-14T00:00:00+00:00");

        let error = parse_time("after", "yesterday").expect_err("not a time");
        assert!(error.starts_with("after must be"), "{error}");
    }

    #[test]
    fn paging_advances_by_limit_until_the_total() {
        assert_eq!(next_offset(0, 10, 25), Some(10));
        assert_eq!(next_offset(20, 10, 25), None);
        assert_eq!(next_offset(9970, 10, 100_000), None);
    }

    #[test]
    fn hits_flatten_and_unreadable_ones_are_counted() {
        let message = json!({
            "id": "2",
            "channel_id": "1",
            "author": { "id": "3", "username": "wonrax" },
            "content": "hello",
            "timestamp": "2026-09-14T12:03:30.469Z",
            "tts": false,
            "mention_everyone": false,
            "mentions": [],
            "mention_roles": [],
            "attachments": [],
            "embeds": [],
            "pinned": false,
            "type": 0
        });
        let (hits, unreadable) = parse_hits(vec![vec![message], vec![json!({ "id": "nope" })]]);

        assert_eq!(
            hits.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(),
            vec!["hello"]
        );
        assert_eq!(unreadable, 1);
    }
}
