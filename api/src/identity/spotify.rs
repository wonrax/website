use std::{
    collections::{HashMap, HashSet},
    ops::Deref,
    sync::Arc,
    time::Duration,
};

use axum::{
    Json,
    extract::{Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use diesel::deserialize::Queryable;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use eyre::eyre;
use rspotify::{
    AuthCodeSpotify, Token,
    clients::{BaseClient, OAuthClient},
    model::Id,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{OnceCell, RwLock};

use crate::{
    App,
    config::SpotifyOauth,
    error::{ApiRequestError, AppError, Error},
    identity::models::credential::{IdentityCredential, NewIdentityCredential},
};

use super::AuthUser;

/// The credentials being persisted in the database
#[derive(Queryable, Deserialize, Serialize)]
pub struct SpotifyCredentials {
    pub user_id: String,
    pub display_name: String,
    pub refresh_token: String,
    pub scopes: Vec<String>,
    pub provider: String,
}

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
    State(ctx): State<App>,
    Query(queries): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    let return_to = queries.get("return_to");
    let redirect_uri = ctx.config.site_url.clone()
        + "/link/spotify"
        + match return_to {
            Some(return_to) => "?return_to=".to_string() + return_to,
            None => "".into(),
        }
        .as_str();

    let spotify_client = create_spotify_client(ctx, Some(redirect_uri));

    let url = spotify_client
        .get_authorize_url(false)
        .map_err(|e| eyre!(e).wrap_err("couldn't build spotify authorize url"))?;

    Ok((axum::http::StatusCode::FOUND, [(header::LOCATION, url)]).into_response())
}

#[axum::debug_handler]
pub async fn handle_spotify_callback(
    State(ctx): State<App>,
    Query(queries): Query<HashMap<String, String>>,
    AuthUser(i): AuthUser,
) -> Result<impl IntoResponse, AppError> {
    if i.id != ctx.config.owner_identity_id {
        Err(SpotifyConnectError::NotPermitted)?
    }

    let code = queries
        .get("code")
        .ok_or(("No `code` in query parameters", StatusCode::BAD_REQUEST))?;

    let return_to = queries.get("return_to");
    let redirect_uri = ctx.config.site_url.clone()
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
        .map_err(|_| "could not take the lock on token")?;

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
            .map_err(|e| eyre!(e).wrap_err("couldn't serialize spotify credentials"))?,
    );

    {
        use crate::schema::{identity_credential_types, identity_credentials};

        let mut conn = ctx.diesel.get().await?;

        let credential_type_id = identity_credential_types::table
            .filter(identity_credential_types::name.eq("oauth"))
            .select(identity_credential_types::id)
            .first::<i32>(&mut conn)
            .await?;

        let new_credential = NewIdentityCredential {
            credential: credential.credential,
            credential_type_id,
            identity_id: i.id,
            created_at: credential.created_at,
            updated_at: credential.updated_at,
        };

        diesel::insert_into(identity_credentials::table)
            .values(&new_credential)
            .execute(&mut conn)
            .await?;
    }

    Ok(())
}

/// Cache spotify client that ties to my account, so that you don't have to
/// create a new client for every request. It also reuses the access token and
/// only refreshed when expired, contrast to having to request for access token
/// every time the client is newly created.
static SPOTIFY_CLIENT: OnceCell<AuthCodeSpotify> = OnceCell::const_new();

static CURRENTLY_PLAYING_CACHE: OnceCell<RwLock<(Arc<CurrentlyPlaying>, std::time::Instant)>> =
    OnceCell::const_new();

#[derive(Clone, Serialize)]
struct CurrentlyPlaying {
    is_playing: bool,
    item: Option<rspotify::model::PlayableItem>,
    currently_playing_type: Option<String>,
}

#[axum::debug_handler]
pub async fn get_currently_playing(State(s): State<App>) -> Result<impl IntoResponse, AppError> {
    async fn fetch_cp(s: &App) -> Result<CurrentlyPlaying, AppError> {
        let user_id = s.config.owner_identity_id;

        let client = SPOTIFY_CLIENT
            .get_or_try_init(|| async { create_my_authorized_spotify_client(s, user_id).await })
            .await?;

        let cp = client
            .current_playing(None, None::<&[_]>)
            .await
            .map_err(|e| eyre!(e).wrap_err("could not get currently playing of user"))?;

        let cp = match cp {
            Some(cp) => CurrentlyPlaying {
                is_playing: cp.is_playing,
                item: cp.item,
                currently_playing_type: Some(format!("{:?}", cp.currently_playing_type)),
            },
            None => CurrentlyPlaying {
                is_playing: false,
                item: None,
                currently_playing_type: None,
            },
        };

        Ok(cp)
    }

    let lock = CURRENTLY_PLAYING_CACHE
        .get_or_try_init(|| async {
            Ok::<_, AppError>(RwLock::new((
                Arc::new(fetch_cp(&s).await?),
                std::time::Instant::now(),
            )))
        })
        .await?;

    let cache = lock.read().await;

    let cp = if cache.1.elapsed() > Duration::from_secs(1) {
        drop(cache);
        let mut cache = lock.write().await;
        *cache = (Arc::new(fetch_cp(&s).await?), std::time::Instant::now());
        cache.0.deref().clone()
    } else {
        cache.0.deref().clone()
    };

    Ok(Json(cp))
}

fn create_spotify_client(ctx: App, redirect_uri: Option<String>) -> rspotify::AuthCodeSpotify {
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

async fn create_my_authorized_spotify_client(
    s: &App,
    user_id: i32,
) -> Result<AuthCodeSpotify, Error> {
    use crate::schema::identity_credential_types;
    use crate::schema::identity_credentials::dsl::*;
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    tracing::info!("Creating global spotify client with user ID {user_id}");

    // the user's spotify refresh token
    let refresh_token: Option<String> = identity_credentials
        .select(credential.retrieve_as_text("refresh_token"))
        .inner_join(
            identity_credential_types::table.on(credential_type_id
                .eq(identity_credential_types::id)
                .and(identity_credential_types::name.eq("oauth"))),
        )
        .filter(identity_id.eq(user_id))
        .filter(credential.contains(serde_json::json!({
            "provider": "spotify"
        })))
        .first(&mut s.diesel.get().await.unwrap())
        .await
        .map_err(|e| {
            eyre!(e).wrap_err(format!(
                "could not fetch spotify credential for user {user_id}"
            ))
        })?;

    match refresh_token {
        Some(refresh_token) => {
            let client = create_spotify_client(s.clone(), None);
            let mut tok = client
                .token
                .lock()
                .await
                .map_err(|_| eyre!("could not take the lock on token"))?;

            *tok = Some(Token {
                refresh_token: Some(refresh_token),
                ..Default::default()
            });

            drop(tok);

            client
                .refresh_token()
                .await
                .map_err(|e| eyre!(e).wrap_err("could not refresh token"))?;

            Ok(client)
        }
        None => Err(eyre!(
            "Couldn't find refresh_token field in Spotify credentials"
        ))?,
    }
}
