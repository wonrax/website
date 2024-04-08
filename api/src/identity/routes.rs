use core::panic;
use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::Duration;

use crate::{
    error::{ApiRequestError, Error},
    APIContext,
};

use super::{
    connected_apps::get_connected_apps,
    models::{
        credential::IdentityCredential,
        identity::{Identity, Traits},
        session::Session,
    },
    spotify::{get_currently_playing, handle_spotify_callback, handle_spotify_connect_request},
    AuthenticationError, MaybeAuthUser, COOKIE_NAME,
};

pub fn route() -> Router<APIContext> {
    // TODO rate limit these public endpoints
    Router::<APIContext>::new()
        .route("/me", get(handle_whoami))
        .route("/link/apps", get(get_connected_apps))
        .route("/is_auth", get(is_auth))
        .route("/logout", post(logout))
        .route("/login/github", get(handle_oauth_github_request))
        .route("/login/github/callback", get(handle_github_oauth_callback))
        .route("/link/spotify", get(handle_spotify_connect_request))
        .route("/link/spotify/callback", get(handle_spotify_callback))
        .route(
            "/user/:user_id/currently-playing",
            get(get_currently_playing),
        )
}

#[derive(serde::Serialize)]
pub struct WhoamiRespose {
    traits: Traits,
}

impl ApiRequestError for AuthenticationError {
    fn status_code(&self) -> axum::http::StatusCode {
        match self {
            AuthenticationError::NoCookie => axum::http::StatusCode::BAD_REQUEST,
            AuthenticationError::Unauthorized => axum::http::StatusCode::UNAUTHORIZED,
        }
    }
}

#[derive(serde::Serialize)]
struct IsAuth {
    is_auth: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<i32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    traits: Option<Traits>,
}

async fn is_auth(MaybeAuthUser(identity): MaybeAuthUser) -> Result<axum::Json<IsAuth>, Error> {
    Ok(Json(IsAuth {
        is_auth: identity.is_ok(),
        id: identity.as_ref().ok().map(|i| i.id),
        traits: identity.ok().map(|i| i.traits),
    }))
}

async fn handle_whoami(
    MaybeAuthUser(identity): MaybeAuthUser,
) -> Result<axum::Json<WhoamiRespose>, Error> {
    Ok(axum::Json(WhoamiRespose {
        traits: identity?.traits,
    }))
}

/// The credentials being persisted in the database
#[derive(Deserialize, Serialize)]
pub struct GitHubCredentials {
    pub user_id: i64,
    pub provider: String,
}

#[axum::debug_handler]
pub async fn handle_github_oauth_callback(
    State(ctx): State<APIContext>,
    Query(queries): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, Error> {
    let code = queries
        .get("code")
        .ok_or(("No `code` in query parameters", StatusCode::BAD_REQUEST))?;

    // TODO move this to shared config
    let github_oauth_client_id: String = std::env::var("GITHUB_OAUTH_CLIENT_ID")
        .expect("GITHUB_OAUTH_CLIENT_ID is not set in .env file");
    let github_oauth_client_secret: String = std::env::var("GITHUB_OAUTH_CLIENT_SECRET")
        .expect("GITHUB_OAUTH_CLIENT_SECRET is not set in .env file");

    let code_verify: serde_json::Value = reqwest::Client::new()
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": github_oauth_client_id,
            "client_secret": github_oauth_client_secret,
            "code": code
        }))
        .send()
        .await?
        .json()
        .await?;

    let access_token = code_verify["access_token"].as_str();

    if access_token.is_none() {
        Err(AuthenticationError::Unauthorized)?
    }

    let user_info: serde_json::Value = reqwest::Client::new()
        .get("https://api.github.com/user")
        .header("User-Agent", "reqwest")
        .header("Accept", "application/json")
        .header(
            "Authorization",
            "Bearer ".to_string() + access_token.unwrap(),
        )
        .send()
        .await?
        .json()
        .await?;

    const MISSING_EXPECTED_FIELD: &str = "GitHub returned unexpected response";

    // TODO use a struct to deserialize into instead of this
    let user = user_info["login"].as_str().ok_or(MISSING_EXPECTED_FIELD)?;
    let user_id = user_info["id"].as_i64().ok_or(MISSING_EXPECTED_FIELD)?;
    let full_name = user_info["name"].as_str().ok_or(MISSING_EXPECTED_FIELD)?;

    let emails: serde_json::Value = reqwest::Client::new()
        .get("https://api.github.com/user/emails")
        .header("User-Agent", "reqwest")
        .header("Accept", "application/json")
        .header(
            "Authorization",
            "Bearer ".to_string() + access_token.unwrap(),
        )
        .send()
        .await?
        .json()
        .await?;

    let email = emails
        .as_array()
        .ok_or("emails is not an array")?
        .iter()
        .find(|email| {
            email["primary"].as_bool().unwrap_or(false)
                && email["verified"].as_bool().unwrap_or(false)
        })
        .ok_or((
            "No valid email found for this github account",
            StatusCode::BAD_GATEWAY,
        ))?
        .get("email")
        .ok_or(
            "valid email found, but couldn't extract it because the field `email` does not exist",
        )?
        .as_str()
        .ok_or(format!(
            "valid email found, but couldn't extract it because the field `email` is not a string: {}",
            emails
        ))?;

    let i = Identity::new_with_traits(Traits {
        name: Some(full_name.to_owned()),
        email: Some(email.to_owned()),
    });

    let mut identity = sqlx::query_as!(
        Identity,
        "
        SELECT i.*
        FROM identities i JOIN identity_credentials ic
        ON i.id = ic.identity_id
        WHERE ic.credential @> $1
        ",
        json!({
            "provider": "github",
            "user_id": user_id
        })
    )
    .fetch_one(&ctx.pool)
    .await
    .ok();

    if identity.is_none() {
        let mut tx = ctx.pool.begin().await?;
        let i = sqlx::query_as!(
            Identity,
            "INSERT INTO identities (
			traits,
			created_at,
			updated_at
		)
		VALUES ($1, $2, $3)
        RETURNING *;",
            serde_json::Value::from(&i.traits),
            i.created_at,
            i.updated_at,
        )
        .fetch_one(&mut *tx)
        .await?;

        let credential = IdentityCredential::new_oauth_credential(serde_json::json!({
            "user_id": user_id,
            "provider": "github",
        }));

        // TODO have a credential types cache since it's not going to change
        sqlx::query!(
            "INSERT INTO identity_credentials (
            credential,
            credential_type_id,
            identity_id,
            created_at,
            updated_at
        )
        VALUES (
            $1,
            (SELECT id FROM identity_credential_types WHERE name = $2),
            $3,
            $4,
            $5
        );
        ",
            &credential.credential,
            Into::<&str>::into(credential.credential_type),
            i.id,
            credential.created_at,
            credential.updated_at,
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        identity = Some(i);
    }

    let identity = identity.unwrap();

    let session = Session::new_with_identity_id(identity.id.clone());

    sqlx::query!(
        "
        INSERT INTO sessions (
            token,
            active,
            issued_at,
            expires_at,
            identity_id,
            created_at,
            updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7);",
        session.token,
        session.active,
        session.issued_at,
        session.expires_at,
        session.identity_id,
        session.created_at,
        session.updated_at,
    )
    .execute(&ctx.pool)
    .await?;

    let auth_cookie = axum_extra::extract::cookie::Cookie::build((COOKIE_NAME, session.token))
        .secure(true)
        .http_only(true)
        // TODO consider switching from chrono to time for the whole crate
        .expires(
            time::OffsetDateTime::now_utc()
                + (session.expires_at - session.issued_at).to_std().unwrap(),
        )
        .path("/");

    Ok(CookieJar::new().add(auth_cookie))
}

#[axum::debug_handler]
pub async fn handle_oauth_github_request(
    Query(queries): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, Error> {
    let return_to = queries.get("return_to");
    let site_url: String = std::env::var("SITE_URL").unwrap_or("http://localhost:4321".to_string());
    let redirect_uri = site_url
        + "/login/github"
        + match return_to {
            Some(return_to) => "?return_to=".to_string() + return_to,
            None => "".into(),
        }
        .as_str();

    // TODO move this to shared config
    let github_oauth_client_id: String = std::env::var("GITHUB_OAUTH_CLIENT_ID")
        .expect("GITHUB_OAUTH_CLIENT_ID is not set in .env file");
    let url = reqwest::Url::parse_with_params(
        "https://github.com/login/oauth/authorize",
        &[
            ("client_id", github_oauth_client_id.as_str()),
            ("scope", "user:email"),
            ("redirect_uri", redirect_uri.as_str()),
        ],
    )
    .map_err(|e| format!("Failed to parse url: {}", e))?
    .to_string();

    Ok((axum::http::StatusCode::FOUND, [(header::LOCATION, url)]).into_response())
}

#[axum::debug_handler]
pub async fn logout() -> impl IntoResponse {
    let auth_cookie = axum_extra::extract::cookie::Cookie::build(COOKIE_NAME)
        .secure(true)
        .http_only(true)
        .max_age(Duration::ZERO)
        .path("/");

    CookieJar::new().add(auth_cookie)
}
