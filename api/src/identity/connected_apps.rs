use axum::{extract::State, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde::Serialize;

#[derive(Queryable, Selectable)]
#[diesel(table_name = crate::schema::identity_credentials)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct IdentityCredentials {
    id: i32,
    credential: Option<serde_json::Value>,
    credential_type_id: i32,
    identity_id: i32,
    created_at: chrono::NaiveDateTime,
    updated_at: chrono::NaiveDateTime,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = crate::schema::identity_credential_types)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct IdentityCredentialTypes {
    id: i32,
    name: String,
    created_at: chrono::NaiveDateTime,
}

use crate::{error::Error, APIContext};

use super::{routes::GitHubCredentials, spotify::SpotifyCredentials, AuthUser};

#[derive(Serialize)]
struct ConnectedApps {
    spotify: Option<Spotify>,
    github: Option<GitHub>,
}

#[derive(Serialize)]
struct Spotify {
    display_name: String,
    added_on: DateTime<Utc>,
}

#[derive(Serialize)]
struct GitHub {
    user_id: i64,
    added_on: DateTime<Utc>,
}

pub async fn get_connected_apps(
    State(s): State<APIContext>,
    AuthUser(i): AuthUser,
) -> Result<impl IntoResponse, Error> {
    let conn = &mut s
        .diesel
        .get()
        .await
        .map_err(|_| "could not get diesel pool conn")?;

    let connections: Vec<IdentityCredentials> = {
        use crate::schema::identity_credential_types;
        use crate::schema::identity_credentials::dsl::*;

        let query = identity_credentials
            .select(IdentityCredentials::as_select())
            .inner_join(
                identity_credential_types::table.on(credential_type_id
                    .eq(identity_credential_types::id)
                    .and(identity_credential_types::name.eq("oauth"))),
            )
            .filter(identity_id.eq(i.id))
            .filter(
                credential
                    .contains(serde_json::json!({
                        "provider": "spotify"
                    }))
                    .or(credential.contains(serde_json::json!({
                        "provider": "github"
                    }))),
            );

        query
            .load(conn)
            .await
            .map_err(|_| "could not query connected apps")?
    };

    let github = connections
        .iter()
        .filter(|c| {
            if let Some(c) = &c.credential {
                c.as_object()
                    .unwrap()
                    .get("provider")
                    .map_or(false, |p| p == "github")
            } else {
                false
            }
        })
        .map(|c| GitHub {
            user_id: serde_json::from_value::<GitHubCredentials>(c.credential.clone().unwrap())
                .unwrap()
                .user_id,
            added_on: c.created_at.and_utc(),
        })
        .next();

    let spotify = connections
        .iter()
        .filter(|c| {
            if let Some(c) = &c.credential {
                c.as_object()
                    .unwrap()
                    .get("provider")
                    .map_or(false, |p| p == "spotify")
            } else {
                false
            }
        })
        .map(|c| Spotify {
            display_name: serde_json::from_value::<SpotifyCredentials>(
                c.credential.to_owned().unwrap(),
            )
            .unwrap()
            .display_name,
            added_on: c.created_at.and_utc(),
        })
        .next();

    Ok(Json(ConnectedApps { github, spotify }))
}
