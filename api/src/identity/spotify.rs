use std::collections::{HashMap, HashSet};

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use diesel::deserialize::Queryable;
use rspotify::{
    clients::{BaseClient, OAuthClient},
    model::Id,
    Token,
};
use serde::{Deserialize, Serialize};

use crate::{
    config::SpotifyOauth,
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
    State(ctx): State<APIContext>,
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

    let spotify_client = create_spotify_client(ctx, Some(redirect_uri));

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

    let return_to = queries.get("return_to");
    let site_url: String = std::env::var("SITE_URL").unwrap_or("http://localhost:4321".to_string());
    let redirect_uri = site_url
        + "/link/spotify"
        + match return_to {
            Some(return_to) => "?return_to=".to_string() + return_to,
            None => "".into(),
        }
        .as_str();

    let spotify_client = create_spotify_client(ctx.clone(), Some(redirect_uri));

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

#[derive(Serialize)]
struct CurrentlyPlaying {
    is_playing: bool,
    item: Option<rspotify::model::PlayableItem>,
    currently_playing_type: Option<String>,
}

// TODO cache spotify client to reuse access tokens
#[axum::debug_handler]
pub async fn get_currently_playing(
    State(s): State<APIContext>,
    Path(user_id): Path<i64>,
) -> Result<impl IntoResponse, Error> {
    use crate::schema::identity_credential_types;
    use crate::schema::identity_credentials::dsl::*;
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    // the user's spotify refresh token
    let refresh_token: Option<String> = identity_credentials
        .select(credential.retrieve_as_text("refresh_token"))
        .inner_join(
            identity_credential_types::table.on(credential_type_id
                .eq(identity_credential_types::id)
                .and(identity_credential_types::name.eq("oauth"))),
        )
        .filter(identity_id.eq(user_id as i32))
        .filter(credential.contains(serde_json::json!({
            "provider": "spotify"
        })))
        .first(&mut s.diesel.get().await.unwrap())
        .await
        .map_err(|e| {
            format!(
                "could not fetch spotify credential for user {}: {}",
                user_id, e
            )
        })?;

    match refresh_token {
        Some(refresh_token) => {
            let client = create_spotify_client(s, None);
            let mut tok = client
                .token
                .lock()
                .await
                .map_err(|_| "could not take the lock on token")?;

            *tok = Some(Token {
                refresh_token: Some(refresh_token),
                ..Default::default()
            });

            drop(tok);

            client
                .refresh_token()
                .await
                .map_err(|e| format!("could not refresh token: {}", e))?;

            let cp = client
                .current_playing(None, None::<&[_]>)
                .await
                .map_err(|e| {
                    format!("could not get currently playing of user {}: {}", user_id, e,)
                })?;

            match cp {
                Some(cp) => Ok(Json(CurrentlyPlaying {
                    is_playing: cp.is_playing,
                    item: cp.item,
                    currently_playing_type: Some(format!("{:?}", cp.currently_playing_type)),
                })),
                None => Ok(Json(CurrentlyPlaying {
                    is_playing: false,
                    item: None,
                    currently_playing_type: None,
                })),
            }
        }
        None => Err(("No data", StatusCode::NOT_FOUND))?,
    }
}

fn create_spotify_client(
    ctx: APIContext,
    redirect_uri: Option<String>,
) -> rspotify::AuthCodeSpotify {
    let SpotifyOauth {
        client_id,
        client_secret,
    } = ctx
        .config
        .spotify_oauth
        .as_ref()
        .expect("Spotify Oauth credentials must be set");

    rspotify::AuthCodeSpotify::new(
        rspotify::Credentials {
            id: client_id.clone(),
            secret: Some(client_secret.clone()),
        },
        rspotify::OAuth {
            redirect_uri: redirect_uri.unwrap_or_default(),
            scopes: HashSet::from(["user-read-currently-playing".to_string()]),
            ..Default::default() // to let it generate the state for us
        },
    )
}

/// The credentials being persisted in the database
#[derive(Queryable, Deserialize, Serialize)]
pub struct SpotifyCredentials {
    pub user_id: String,
    pub display_name: String,
    pub refresh_token: String,
    pub scopes: Vec<String>,
    pub provider: String,
}
