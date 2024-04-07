use std::collections::{HashMap, HashSet};

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
};
use rspotify::{clients::OAuthClient, model::Id};
use serde::Serialize;

use crate::{
    error::{ApiRequestError, Error},
    identity::models::credential::IdentityCredential,
    APIContext,
};

use super::AuthUser;

#[derive(thiserror::Error, Debug)]
pub enum SpotifyConnectError {
    #[error("You are not permitted to link Spotify account")]
    NotPermitted,

    #[error("We failed to verify your Spotify connection")]
    ConnectFailed,

    #[error("Authorization succeeded, but couldn't fetch your Spotify account information")]
    MissingUserInfo,
}

impl ApiRequestError for SpotifyConnectError {}

#[axum::debug_handler]
pub async fn handle_spotify_connect_request(
    Query(queries): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, Error> {
    let return_to = queries.get("return_to");
    let site_url: String = std::env::var("SITE_URL").unwrap_or("http://localhost:4321".to_string());
    let redirect_uri = site_url
        + "/link/spotify"
        + match return_to {
            Some(return_to) => "?return_to=".to_string() + return_to,
            None => "".into(),
        }
        .as_str();

    // TODO move this to shared config
    let spotify_oauth_client_id: String = std::env::var("SPOTIFY_OAUTH_CLIENT_ID")
        .expect("SPOTIFY_OAUTH_CLIENT_ID is not set in .env file");

    let spotify_client = rspotify::AuthCodeSpotify::new(
        rspotify::Credentials {
            id: spotify_oauth_client_id,
            secret: None,
        },
        rspotify::OAuth {
            redirect_uri,
            scopes: HashSet::from(["user-read-currently-playing".to_string()]),
            ..Default::default() // to let it generate the state for us
        },
    );

    let url = spotify_client.get_authorize_url(false).unwrap();

    Ok((axum::http::StatusCode::FOUND, [(header::LOCATION, url)]).into_response())
}

#[axum::debug_handler]
pub async fn handle_spotify_callback(
    State(ctx): State<APIContext>,
    Query(queries): Query<HashMap<String, String>>,
    AuthUser(i): AuthUser,
) -> Result<impl IntoResponse, Error> {
    // TODO remove hardcode
    if i.id != 1 {
        Err(SpotifyConnectError::NotPermitted)?
    }

    let code = queries
        .get("code")
        .ok_or(("No `code` in query parameters", StatusCode::BAD_REQUEST))?;

    // TODO move this to shared config
    let spotify_oauth_client_id: String = std::env::var("SPOTIFY_OAUTH_CLIENT_ID")
        .expect("SPOTIFY_OAUTH_CLIENT_ID is not set in .env file");
    let spotify_oauth_client_secret: String = std::env::var("SPOTIFY_OAUTH_CLIENT_SECRET")
        .expect("SPOTIFY_OAUTH_CLIENT_SECRET is not set in .env file");

    let return_to = queries.get("return_to");
    let site_url: String = std::env::var("SITE_URL").unwrap_or("http://localhost:4321".to_string());
    let redirect_uri = site_url
        + "/link/spotify"
        + match return_to {
            Some(return_to) => "?return_to=".to_string() + return_to,
            None => "".into(),
        }
        .as_str();

    let spotify_client = rspotify::AuthCodeSpotify::new(
        rspotify::Credentials {
            id: spotify_oauth_client_id,
            secret: Some(spotify_oauth_client_secret),
        },
        rspotify::OAuth {
            redirect_uri,
            scopes: HashSet::from(["user-read-currently-playing".to_string()]),
            ..Default::default() // to let it generate the state for us
        },
    );

    spotify_client
        .request_token(code)
        .await
        .map_err(|_| SpotifyConnectError::ConnectFailed)?;

    let me = spotify_client
        .me()
        .await
        .map_err(|_| SpotifyConnectError::MissingUserInfo)?;

    let token = spotify_client
        .token
        .lock()
        .await
        .map_err(|e| format!("could not take the lock on token: {:?}", e))?;

    let creds = SpotifyCredentials {
        display_name: me
            .display_name
            .ok_or(SpotifyConnectError::MissingUserInfo)?,
        refresh_token: token
            .as_ref()
            .ok_or("spotify token retrieved, but cannot be found in struct")?
            .refresh_token
            .clone()
            .ok_or("spotify token retrieved, but refresh token is None")?,
        user_id: me.id.id().to_string(),
        scopes: token
            .as_ref()
            .ok_or("spotify token retrieved, but scopes cannot be found in struct")?
            .scopes
            .iter()
            .cloned()
            .collect(),
        provider: "spotify".into(),
    };

    let credential = IdentityCredential::new_oauth_credential(
        serde_json::to_value(creds)
            .map_err(|e| format!("couldn't serialize spotify credentials: {}", e))?,
    );

    sqlx::query!(
        "
        INSERT INTO identity_credentials (
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
    .execute(&ctx.pool)
    .await?;

    Ok(())
}

/// The credentials being persisted in the database
#[derive(Serialize)]
struct SpotifyCredentials {
    user_id: String,
    display_name: String,
    refresh_token: String,
    scopes: Vec<String>,
    provider: String,
}
