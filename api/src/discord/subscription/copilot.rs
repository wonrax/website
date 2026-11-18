//! The GitHub Copilot sign-in as VS Code's Copilot Chat does it: GitHub's device flow for a GitHub
//! token, which is then traded for a short-lived Copilot token.
//!
//! The GitHub token lasts until revoked and doesn't rotate, and the trade can be repeated as
//! often as needed, so a refresh is just another trade.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use reqwest::StatusCode;
use rig::providers::{copilot, openai::OpenAIConfig};
use serde::Deserialize;

use super::{
    Grant,
    oauth::{
        DeviceCode, PollError, PollOutcome, REQUEST_TIMEOUT, RefreshError, Tokens, error_message,
        poll_interval,
    },
};

/// Copilot Chat's GitHub OAuth app, the one the Copilot token endpoint trusts
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
/// The editor identity the Copilot token endpoint expects; matches what rig sends to the API
const EDITOR_VERSION: &str = "vscode/1.107.0";
const EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
const USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
/// Used when GitHub doesn't say how long a device code lasts
const DEFAULT_DEVICE_CODE_TTL: Duration = Duration::from_secs(15 * 60);
/// Added to the poll interval each time GitHub answers `slow_down`
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);
/// Assumed Copilot token lifetime when the response states none; they last about 30 minutes
const FALLBACK_TOKEN_LIFETIME: TimeDelta = TimeDelta::minutes(20);

const FORM: &str = "application/x-www-form-urlencoded";

fn form(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// rig's configuration for the Copilot API, signed in as `grant`. rig reads the API host for the
/// account's plan from the token itself.
pub fn config(grant: &Grant) -> OpenAIConfig {
    OpenAIConfig::with_key(&copilot::DIALECT, grant.access_token.clone())
}

pub async fn request_device_code(http: &reqwest::Client) -> Result<DeviceCode, String> {
    #[derive(Deserialize)]
    struct DeviceCodeResponse {
        device_code: String,
        user_code: String,
        verification_uri: String,
        expires_in: Option<u64>,
        interval: Option<u64>,
    }

    let response = http
        .post(DEVICE_CODE_URL)
        .timeout(REQUEST_TIMEOUT)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, FORM)
        .body(form(&[("client_id", CLIENT_ID), ("scope", "read:user")]))
        .send()
        .await
        .map_err(|e| format!("could not reach GitHub: {e}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "GitHub answered HTTP {status}: {}",
            error_message(&body)
        ));
    }

    let code: DeviceCodeResponse =
        serde_json::from_str(&body).map_err(|e| format!("unexpected response from GitHub: {e}"))?;
    Ok(DeviceCode {
        user_code: code.user_code,
        verification_url: code.verification_uri,
        ttl: code
            .expires_in
            .map_or(DEFAULT_DEVICE_CODE_TTL, Duration::from_secs),
        poll_interval: poll_interval(code.interval),
        device_secret: code.device_code,
    })
}

/// Whether someone has entered `code` yet, and the tokens it got if so. Slows `code`'s polling
/// down when GitHub asks.
pub async fn poll_device_code(
    http: &reqwest::Client,
    code: &mut DeviceCode,
) -> Result<PollOutcome, PollError> {
    #[derive(Deserialize)]
    struct AccessTokenResponse {
        access_token: Option<String>,
        error: Option<String>,
        error_description: Option<String>,
    }

    let response = http
        .post(ACCESS_TOKEN_URL)
        .timeout(REQUEST_TIMEOUT)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, FORM)
        .body(form(&[
            ("client_id", CLIENT_ID),
            ("device_code", code.device_secret.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ]))
        .send()
        .await
        .map_err(|e| PollError::Network(format!("could not reach GitHub: {e}")))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return Err(PollError::Network(format!(
            "GitHub answered HTTP {status}: {}",
            error_message(&body)
        )));
    }
    // GitHub answers 200 with an `error` field while the code is pending, so the status alone
    // says little
    let answer: AccessTokenResponse = serde_json::from_str(&body).map_err(|_| {
        PollError::Rejected(format!(
            "unexpected sign-in response from GitHub (HTTP {status}): {}",
            error_message(&body)
        ))
    })?;

    if let Some(github_token) = answer.access_token {
        return trade(http, &github_token)
            .await
            .map(PollOutcome::Approved)
            .map_err(|e| PollError::Rejected(e.to_string()));
    }
    match answer.error.as_deref() {
        Some("authorization_pending") => Ok(PollOutcome::Pending),
        Some("slow_down") => {
            code.poll_interval += SLOW_DOWN_STEP;
            Ok(PollOutcome::Pending)
        }
        Some("expired_token") => Err(PollError::Rejected("the code expired".to_string())),
        Some("access_denied") => Err(PollError::Rejected(
            "the sign-in was cancelled on GitHub".to_string(),
        )),
        error => Err(PollError::Rejected(format!(
            "GitHub refused the sign-in: {}",
            answer
                .error_description
                .as_deref()
                .or(error)
                .unwrap_or("no details given")
        ))),
    }
}

/// A fresh Copilot token for the GitHub token `previous` holds
pub async fn refresh(http: &reqwest::Client, previous: &Tokens) -> Result<Tokens, RefreshError> {
    trade(http, &previous.refresh_token).await
}

/// Trade `github_token` for a Copilot token
async fn trade(http: &reqwest::Client, github_token: &str) -> Result<Tokens, RefreshError> {
    let response = http
        .get(COPILOT_TOKEN_URL)
        .timeout(REQUEST_TIMEOUT)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(
            reqwest::header::AUTHORIZATION,
            format!("token {github_token}"),
        )
        .header("editor-version", EDITOR_VERSION)
        .header("editor-plugin-version", EDITOR_PLUGIN_VERSION)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .map_err(|e| RefreshError::Transient(format!("could not reach GitHub: {e}")))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    parse_trade(status, &body, github_token)
}

fn parse_trade(status: StatusCode, body: &str, github_token: &str) -> Result<Tokens, RefreshError> {
    #[derive(Deserialize)]
    struct CopilotTokenResponse {
        token: String,
        /// Unix seconds
        expires_at: Option<i64>,
        /// Seconds until GitHub suggests trading again
        refresh_in: Option<i64>,
    }

    match status {
        // The GitHub token was revoked, or the account has no Copilot plan
        StatusCode::UNAUTHORIZED => {
            return Err(RefreshError::Rejected(
                "GitHub no longer accepts the saved sign-in".to_string(),
            ));
        }
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
            return Err(RefreshError::Rejected(format!(
                "the signed-in GitHub account can't use Copilot: {}",
                error_message(body)
            )));
        }
        status if !status.is_success() => {
            return Err(RefreshError::Transient(format!(
                "getting a Copilot token failed (HTTP {status}): {}",
                error_message(body)
            )));
        }
        _ => {}
    }

    let response: CopilotTokenResponse = serde_json::from_str(body)
        .map_err(|e| RefreshError::Transient(format!("unexpected Copilot token response: {e}")))?;
    let expires_at = response
        .expires_at
        .and_then(|at| DateTime::from_timestamp(at, 0))
        .or_else(|| {
            response
                .refresh_in
                .map(|secs| Utc::now() + TimeDelta::seconds(secs))
        })
        .unwrap_or_else(|| Utc::now() + FALLBACK_TOKEN_LIFETIME);
    Ok(Tokens {
        access_token: response.token,
        refresh_token: github_token.to_string(),
        id_token: None,
        account_id: None,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trade_keeps_the_github_token_and_takes_the_expiry() {
        let body = r#"{"token": "tid=1;exp=2000000000;proxy-ep=proxy.individual.githubcopilot.com", "expires_at": 2000000000, "refresh_in": 1500, "endpoints": {"api": "https://api.individual.githubcopilot.com"}}"#;

        let tokens = parse_trade(StatusCode::OK, body, "gho_github").expect("a Copilot token");

        assert!(tokens.access_token.starts_with("tid=1;"));
        assert_eq!(tokens.refresh_token, "gho_github");
        assert_eq!(tokens.expires_at.timestamp(), 2_000_000_000);
    }

    #[test]
    fn only_a_refused_github_token_demands_a_new_sign_in() {
        let rejected = |status| {
            matches!(
                parse_trade(status, r#"{"message": "nope"}"#, "gho"),
                Err(RefreshError::Rejected(_))
            )
        };

        assert!(rejected(StatusCode::UNAUTHORIZED));
        assert!(rejected(StatusCode::FORBIDDEN));
        assert!(!rejected(StatusCode::TOO_MANY_REQUESTS));
        assert!(!rejected(StatusCode::BAD_GATEWAY));
    }
}
