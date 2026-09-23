//! The "Sign in with ChatGPT" OAuth flow as the Codex CLI speaks it: device-code sign-in and
//! refresh-token rotation against auth.openai.com.

use std::time::Duration;

use base64::Engine as _;
use chrono::{DateTime, TimeDelta, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Deserializer};

const ISSUER: &str = "https://auth.openai.com";
/// Codex CLI's public OAuth client, the only one the ChatGPT backend hands subscription tokens to
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Where the user types the device code
pub const VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
/// How long OpenAI honours a device code
pub const DEVICE_CODE_TTL: Duration = Duration::from_secs(15 * 60);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Assumed access token lifetime when neither the JWT nor the response states one. Short on
/// purpose: refreshing early costs one request, expiring unnoticed costs a failed run.
const FALLBACK_TOKEN_LIFETIME: TimeDelta = TimeDelta::hours(1);

/// A pending device-code sign-in
pub struct DeviceCode {
    pub user_code: String,
    device_auth_id: String,
    pub poll_interval: Duration,
}

pub enum PollOutcome {
    /// Nobody has entered the code yet
    Pending,
    Approved(AuthorizationGrant),
}

pub struct AuthorizationGrant {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PollError {
    /// The request didn't make it; the next poll may
    #[error("could not reach OpenAI: {0}")]
    Network(reqwest::Error),
    #[error("{0}")]
    Rejected(String),
}

/// The credentials a successful sign-in or refresh yields
#[derive(Clone)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: Option<String>,
    pub account_id: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    /// OpenAI refused the refresh token; only a new sign-in brings the session back
    #[error("{0}")]
    Rejected(String),
    /// Network trouble, rate limiting or a server error: the same refresh token may work later
    #[error("{0}")]
    Transient(String),
}

pub async fn request_device_code(http: &reqwest::Client) -> Result<DeviceCode, String> {
    #[derive(Deserialize)]
    struct UserCodeResponse {
        device_auth_id: String,
        #[serde(alias = "usercode")]
        user_code: String,
        /// Seconds, sent as a string by the live API
        #[serde(default, deserialize_with = "seconds_from_string_or_number")]
        interval: Option<u64>,
    }

    let response = http
        .post(format!("{ISSUER}/api/accounts/deviceauth/usercode"))
        .timeout(REQUEST_TIMEOUT)
        .json(&serde_json::json!({ "client_id": CLIENT_ID }))
        .send()
        .await
        .map_err(|e| format!("could not reach OpenAI: {e}"))?;

    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        return Err("OpenAI has device code sign-in turned off for Codex clients".to_string());
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "OpenAI answered HTTP {status}: {}",
            error_message(&body)
        ));
    }

    let code: UserCodeResponse = response
        .json()
        .await
        .map_err(|e| format!("unexpected response from OpenAI: {e}"))?;
    Ok(DeviceCode {
        user_code: code.user_code,
        device_auth_id: code.device_auth_id,
        poll_interval: code
            .interval
            .filter(|&secs| secs > 0)
            .map_or(DEFAULT_POLL_INTERVAL, Duration::from_secs),
    })
}

pub async fn poll_device_code(
    http: &reqwest::Client,
    code: &DeviceCode,
) -> Result<PollOutcome, PollError> {
    #[derive(Deserialize)]
    struct ApprovedResponse {
        authorization_code: String,
        code_verifier: String,
    }

    let response = http
        .post(format!("{ISSUER}/api/accounts/deviceauth/token"))
        .timeout(REQUEST_TIMEOUT)
        .json(&serde_json::json!({
            "device_auth_id": code.device_auth_id,
            "user_code": code.user_code,
        }))
        .send()
        .await
        .map_err(PollError::Network)?;

    let status = response.status();
    // Codex treats both as "not approved yet"
    if status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND {
        return Ok(PollOutcome::Pending);
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(PollError::Rejected(format!(
            "OpenAI refused the sign-in (HTTP {status}): {}",
            error_message(&body)
        )));
    }

    let approved: ApprovedResponse = response.json().await.map_err(|e| {
        PollError::Rejected(format!("unexpected sign-in response from OpenAI: {e}"))
    })?;
    Ok(PollOutcome::Approved(AuthorizationGrant {
        authorization_code: approved.authorization_code,
        code_verifier: approved.code_verifier,
    }))
}

/// Trade an approved device code for tokens. The code is single use, so this is never retried.
pub async fn exchange_authorization(
    http: &reqwest::Client,
    grant: AuthorizationGrant,
) -> Result<Tokens, String> {
    let redirect_uri = format!("{ISSUER}/deviceauth/callback");
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("grant_type", "authorization_code"),
            ("code", grant.authorization_code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", CLIENT_ID),
            ("code_verifier", grant.code_verifier.as_str()),
        ])
        .finish();

    let response = http
        .post(format!("{ISSUER}/oauth/token"))
        .timeout(REQUEST_TIMEOUT)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await
        .map_err(|e| format!("could not reach OpenAI to finish signing in: {e}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "OpenAI refused to finish the sign-in (HTTP {status}): {}",
            error_message(&body)
        ));
    }

    let tokens: TokenResponse = serde_json::from_str(&body)
        .map_err(|e| format!("unexpected token response from OpenAI: {e}"))?;
    let refresh_token = tokens
        .refresh_token
        .clone()
        .ok_or("OpenAI issued no refresh token, so the sign-in could not be kept alive")?;
    Ok(tokens.into_tokens(refresh_token, None))
}

/// Rotate `previous` into a fresh access token. OpenAI invalidates the old refresh token on
/// success, so the returned one must replace it before anything else can refresh.
pub async fn refresh(http: &reqwest::Client, previous: &Tokens) -> Result<Tokens, RefreshError> {
    let response = http
        .post(format!("{ISSUER}/oauth/token"))
        .timeout(REQUEST_TIMEOUT)
        .json(&serde_json::json!({
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": previous.refresh_token,
        }))
        .send()
        .await
        .map_err(|e| RefreshError::Transient(format!("could not reach OpenAI: {e}")))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(classify_refresh_failure(status, &body));
    }

    let tokens: TokenResponse = serde_json::from_str(&body).map_err(|e| {
        RefreshError::Transient(format!("unexpected refresh response from OpenAI: {e}"))
    })?;
    let refresh_token = tokens
        .refresh_token
        .clone()
        .unwrap_or_else(|| previous.refresh_token.clone());
    Ok(tokens.into_tokens(refresh_token, Some(previous)))
}

fn classify_refresh_failure(status: StatusCode, body: &str) -> RefreshError {
    let code = error_code(body);
    let reason = match code.as_deref() {
        Some("refresh_token_expired") => "the saved sign-in expired".to_string(),
        Some("refresh_token_reused") => {
            "the saved sign-in was already used by something else (a copy of it elsewhere, or a \
             refresh that never got saved)"
                .to_string()
        }
        Some("refresh_token_invalidated") => {
            "the sign-in was revoked (signed out everywhere, or the password changed)".to_string()
        }
        _ => format!(
            "OpenAI refused the saved sign-in (HTTP {status}): {}",
            error_message(body)
        ),
    };

    let rejected = status == StatusCode::UNAUTHORIZED
        || (status == StatusCode::BAD_REQUEST
            && matches!(
                code.as_deref(),
                Some(
                    "invalid_grant"
                        | "refresh_token_expired"
                        | "refresh_token_reused"
                        | "refresh_token_invalidated"
                )
            ));
    if rejected {
        RefreshError::Rejected(reason)
    } else {
        RefreshError::Transient(format!(
            "token refresh failed (HTTP {status}): {}",
            error_message(body)
        ))
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<i64>,
}

impl TokenResponse {
    /// Refresh responses may leave out the ID token; `previous` fills what they don't restate
    fn into_tokens(self, refresh_token: String, previous: Option<&Tokens>) -> Tokens {
        let id_token = self
            .id_token
            .or_else(|| previous.and_then(|p| p.id_token.clone()));
        let account_id = id_token
            .as_deref()
            .and_then(account_id)
            .or_else(|| account_id(&self.access_token))
            .or_else(|| previous.and_then(|p| p.account_id.clone()));
        let expires_at = jwt_claims(&self.access_token)
            .and_then(|claims| claims.get("exp")?.as_i64())
            .and_then(|exp| DateTime::from_timestamp(exp, 0))
            .or_else(|| {
                self.expires_in
                    .map(|secs| Utc::now() + TimeDelta::seconds(secs))
            })
            .unwrap_or_else(|| Utc::now() + FALLBACK_TOKEN_LIFETIME);

        Tokens {
            access_token: self.access_token,
            refresh_token,
            id_token,
            account_id,
            expires_at,
        }
    }
}

/// The ChatGPT plan behind a sign-in (plus, pro, ...), read from its ID token
pub fn plan_type(id_token: &str) -> Option<String> {
    auth_claim(id_token, "chatgpt_plan_type")
}

fn account_id(token: &str) -> Option<String> {
    auth_claim(token, "chatgpt_account_id")
}

fn auth_claim(token: &str, name: &str) -> Option<String> {
    jwt_claims(token)?
        .get("https://api.openai.com/auth")?
        .get(name)?
        .as_str()
        .map(str::to_string)
}

/// The unverified payload of a JWT. Only used to read our own tokens' metadata.
fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::prelude::BASE64_URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The error code of an OAuth error body: `{"error": "invalid_grant"}` or
/// `{"error": {"code": "refresh_token_expired"}}`
fn error_code(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let error = json.get("error")?;
    error
        .as_str()
        .or_else(|| error.get("code")?.as_str())
        .map(str::to_string)
}

/// The most readable part of an error body, for showing to people
pub fn error_message(body: &str) -> String {
    let json: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let message = json.as_ref().and_then(|json| {
        json.get("error_description")
            .or_else(|| json.get("error")?.get("message"))
            .or_else(|| json.get("detail"))
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

fn seconds_from_string_or_number<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
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

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: serde_json::Value) -> String {
        let encode = |value: &serde_json::Value| {
            base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(value.to_string())
        };
        format!(
            "{}.{}.signature",
            encode(&serde_json::json!({"alg": "none"})),
            encode(&claims)
        )
    }

    #[test]
    fn tokens_take_expiry_and_account_from_the_jwts() {
        let access_token = jwt(serde_json::json!({ "exp": 2_000_000_000 }));
        let id_token = jwt(serde_json::json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct_1",
                "chatgpt_plan_type": "plus"
            }
        }));
        let response = TokenResponse {
            access_token,
            refresh_token: None,
            id_token: Some(id_token.clone()),
            expires_in: Some(60),
        };

        let tokens = response.into_tokens("refresh".to_string(), None);

        assert_eq!(tokens.expires_at.timestamp(), 2_000_000_000);
        assert_eq!(tokens.account_id.as_deref(), Some("acct_1"));
        assert_eq!(plan_type(&id_token).as_deref(), Some("plus"));
    }

    #[test]
    fn refreshed_tokens_keep_what_the_response_leaves_out() {
        let previous = Tokens {
            access_token: "old".to_string(),
            refresh_token: "old-refresh".to_string(),
            id_token: Some("old-id".to_string()),
            account_id: Some("acct_1".to_string()),
            expires_at: Utc::now(),
        };
        let response = TokenResponse {
            access_token: "opaque".to_string(),
            refresh_token: None,
            id_token: None,
            expires_in: Some(3600),
        };

        let tokens = response.into_tokens("old-refresh".to_string(), Some(&previous));

        assert_eq!(tokens.id_token.as_deref(), Some("old-id"));
        assert_eq!(tokens.account_id.as_deref(), Some("acct_1"));
        assert!(tokens.expires_at > Utc::now() + TimeDelta::minutes(59));
    }

    #[test]
    fn only_a_dead_refresh_token_demands_a_new_sign_in() {
        let rejected = |status, body| {
            matches!(
                classify_refresh_failure(status, body),
                RefreshError::Rejected(_)
            )
        };

        assert!(rejected(
            StatusCode::BAD_REQUEST,
            r#"{"error": "invalid_grant"}"#
        ));
        assert!(rejected(
            StatusCode::BAD_REQUEST,
            r#"{"error": {"code": "refresh_token_reused", "message": "reused"}}"#
        ));
        assert!(rejected(StatusCode::UNAUTHORIZED, ""));
        assert!(!rejected(
            StatusCode::BAD_REQUEST,
            r#"{"error": "invalid_request"}"#
        ));
        assert!(!rejected(StatusCode::TOO_MANY_REQUESTS, ""));
        assert!(!rejected(StatusCode::BAD_GATEWAY, "<html>"));
    }

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
        assert_eq!(error_message(""), "no details given");
        assert_eq!(truncate("abcdef", 3), "abc…");
    }
}
