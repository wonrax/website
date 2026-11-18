//! What every subscription's sign-in flow hands the shared state machine: a device code to show,
//! the outcome of polling it, and the tokens it ends in.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};

/// How long to wait for a provider's OAuth endpoints
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// A pending device-code sign-in
pub struct DeviceCode {
    pub user_code: String,
    /// Where the user types the code
    pub verification_url: String,
    /// How long the provider honours the code
    pub ttl: Duration,
    /// Raised when the provider asks to poll more slowly
    pub poll_interval: Duration,
    /// What the provider identifies this sign-in by when polled; never shown
    pub(super) device_secret: String,
}

pub enum PollOutcome {
    /// Nobody has entered the code yet
    Pending,
    Approved(Tokens),
}

#[derive(Debug, thiserror::Error)]
pub enum PollError {
    /// The request didn't make it; the next poll may
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Rejected(String),
}

/// The credentials a successful sign-in or refresh yields
#[derive(Clone)]
pub struct Tokens {
    /// What model requests carry
    pub access_token: String,
    /// What gets the next access token. ChatGPT rotates it on every refresh; Copilot's is the
    /// GitHub token, which stays.
    pub refresh_token: String,
    pub id_token: Option<String>,
    pub account_id: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    /// The provider refused the refresh token; only a new sign-in brings the session back
    #[error("{0}")]
    Rejected(String),
    /// Network trouble, rate limiting or a server error: the same refresh token may work later
    #[error("{0}")]
    Transient(String),
}

/// The most readable part of an error body, for showing to people
pub fn error_message(body: &str) -> String {
    let json: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let message = json.as_ref().and_then(|json| {
        json.get("error_description")
            .or_else(|| json.get("error")?.get("message"))
            .or_else(|| json.get("detail"))
            .or_else(|| json.get("message"))
            .or_else(|| json.get("error"))
            .and_then(|value| value.as_str())
    });
    let message = message.unwrap_or(body).trim();
    if message.is_empty() {
        return "no details given".to_string();
    }
    truncate(message, 300)
}

pub fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

pub fn seconds_from_string_or_number<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Seconds {
        Number(u64),
        Text(String),
    }

    Ok(match Option::<Seconds>::deserialize(deserializer)? {
        Some(Seconds::Number(secs)) => Some(secs),
        Some(Seconds::Text(text)) => text.trim().parse().ok(),
        None => None,
    })
}

/// A poll interval in seconds as a provider states it, or the default
pub fn poll_interval(seconds: Option<u64>) -> Duration {
    seconds
        .filter(|&secs| secs > 0)
        .map_or(DEFAULT_POLL_INTERVAL, Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_interval_arrives_as_a_string_or_a_number() {
        #[derive(Deserialize)]
        struct Interval {
            #[serde(default, deserialize_with = "seconds_from_string_or_number")]
            interval: Option<u64>,
        }
        let parse = |json| {
            serde_json::from_str::<Interval>(json)
                .map(|i| i.interval)
                .ok()
        };

        assert_eq!(parse(r#"{"interval": "5"}"#), Some(Some(5)));
        assert_eq!(parse(r#"{"interval": 7}"#), Some(Some(7)));
        assert_eq!(parse(r#"{}"#), Some(None));
    }

    #[test]
    fn error_messages_prefer_the_description() {
        assert_eq!(
            error_message(r#"{"error": "invalid_grant", "error_description": "expired"}"#),
            "expired"
        );
        assert_eq!(
            error_message(r#"{"error": {"message": "slow down"}}"#),
            "slow down"
        );
        assert_eq!(error_message(r#"{"message": "Not Found"}"#), "Not Found");
        assert_eq!(error_message(""), "no details given");
        assert_eq!(truncate("abcdef", 3), "abc…");
    }
}
