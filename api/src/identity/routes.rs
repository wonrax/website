use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::Duration;

use crate::{
    App,
    config::GitHubOauth,
    error::{ApiRequestError, AppError},
    identity::models::{
        credential::{IdentityCredential, NewIdentityCredential},
        identity::{Identity, NewIdentity, Traits},
        session::{NewSession, Session},
    },
};

use super::{
    AuthenticationError, COOKIE_NAME, MaybeAuthUser,
    connected_apps::get_connected_apps,
    spotify::{get_currently_playing, handle_spotify_callback, handle_spotify_connect_request},
};

pub fn route() -> Router<App> {
    // TODO rate limit these public endpoints
    Router::<App>::new()
        .route("/me", get(handle_whoami))
        .route("/link/apps", get(get_connected_apps))
        .route("/is_auth", get(is_auth))
        .route("/logout", post(logout))
        .route("/login/github", get(handle_oauth_github_request))
        .route("/login/github/callback", get(handle_github_oauth_callback))
        .route("/link/spotify", get(handle_spotify_connect_request))
        .route("/link/spotify/callback", get(handle_spotify_callback))
        .route("/currently-playing", get(get_currently_playing))
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

    #[serde(skip_serializing_if = "std::ops::Not::not")]
    site_owner: bool,
}

async fn is_auth(
    State(ctx): State<App>,
    MaybeAuthUser(identity): MaybeAuthUser,
) -> Result<axum::Json<IsAuth>, AppError> {
    Ok(Json(IsAuth {
        is_auth: identity.is_ok(),
        id: identity.as_ref().ok().map(|i| i.id),
        traits: identity
            .as_ref()
            .ok()
            .map(|i| Traits::from(i.traits.clone())),
        site_owner: identity
            .as_ref()
            .ok()
            .map(|i| i.id == ctx.config.owner_identity_id)
            .unwrap_or(false),
    }))
}

async fn handle_whoami(
    MaybeAuthUser(identity): MaybeAuthUser,
) -> Result<axum::Json<WhoamiRespose>, AppError> {
    Ok(axum::Json(WhoamiRespose {
        traits: Traits::from(identity?.traits),
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
    State(ctx): State<App>,
    Query(queries): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    let code = queries
        .get("code")
        .ok_or(("No `code` in query parameters", StatusCode::BAD_REQUEST))?;

    let GitHubOauth {
        client_id: github_client_id,
        client_secret: github_client_secret,
    } = ctx
        .config
        .github_oauth
        .as_ref()
        .expect("GitHub Oauth credentials is not set");

    let code_verify: serde_json::Value = ctx
        .http
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": github_client_id,
            "client_secret": github_client_secret,
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

    let user_info: serde_json::Value = ctx
        .http
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

    // NOTE: some users don't have a name set
    let full_name = user_info["name"].as_str().unwrap_or(user);

    let emails: serde_json::Value = ctx
        .http
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

    let mut identity = {
        use crate::schema::{identities, identity_credentials};

        let mut conn = ctx.diesel.get().await?;

        identities::table
            .inner_join(identity_credentials::table)
            .filter(identity_credentials::credential.contains(json!({
                "provider": "github",
                "user_id": user_id
            })))
            .select(identities::all_columns)
            .first::<Identity>(&mut conn)
            .await
            .ok()
    };

    if identity.is_none() {
        use crate::schema::{identities, identity_credential_types, identity_credentials};

        let mut conn = ctx.diesel.get().await?;

        let new_identity = NewIdentity {
            traits: i.traits.clone(),
            created_at: i.created_at,
            updated_at: i.updated_at,
        };

        let inserted_identity: Identity = diesel::insert_into(identities::table)
            .values(&new_identity)
            .get_result(&mut conn)
            .await?;

        let credential = IdentityCredential::new_oauth_credential(serde_json::json!({
            "user_id": user_id,
            "provider": "github",
        }));

        let new_credential = NewIdentityCredential {
            credential: credential.credential,
            credential_type_id: identity_credential_types::table
                .filter(identity_credential_types::name.eq("oauth"))
                .select(identity_credential_types::id)
                .first::<i32>(&mut conn)
                .await?,
            identity_id: inserted_identity.id,
            created_at: credential.created_at,
            updated_at: credential.updated_at,
        };

        diesel::insert_into(identity_credentials::table)
            .values(&new_credential)
            .execute(&mut conn)
            .await?;

        identity = Some(inserted_identity);
    }

    let identity = identity.unwrap();

    let session = Session::new_with_identity_id(identity.id);

    {
        use crate::schema::sessions;

        let mut conn = ctx.diesel.get().await?;

        let new_session = NewSession {
            token: session.token.clone(),
            active: session.active,
            issued_at: session.issued_at,
            expires_at: session.expires_at,
            identity_id: session.identity_id,
            created_at: session.created_at,
            updated_at: session.updated_at,
        };

        diesel::insert_into(sessions::table)
            .values(&new_session)
            .execute(&mut conn)
            .await?;
    }

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
    State(ctx): State<App>,
    Query(queries): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    let return_to = queries.get("return_to");
    let redirect_uri = ctx.config.site_url.clone()
        + "/login/github"
        + match return_to {
            Some(return_to) => "?return_to=".to_string() + return_to,
            None => "".into(),
        }
        .as_str();

    let GitHubOauth {
        client_id: github_client_id,
        ..
    } = ctx
        .config
        .github_oauth
        .as_ref()
        .expect("GitHub Oauth credentials is not set");

    let url = reqwest::Url::parse_with_params(
        "https://github.com/login/oauth/authorize",
        &[
            ("client_id", github_client_id.as_str()),
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
