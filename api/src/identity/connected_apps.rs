use axum::{extract::State, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::Serialize;

#[derive(Queryable, Selectable, Serialize)]
#[diesel(table_name = crate::schema::identity_credentials)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct IdentityCredentials {
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
pub struct IdentityCredentialTypes {
    id: i32,
    name: String,
}

use crate::{error::Error, APIContext};

use super::AuthUser;

struct Spotify {
    display_name: String,
    added_on: DateTime<chrono::offset::Utc>,
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

    let _credential_type_id: i32 = {
        use crate::schema::identity_credential_types::dsl::*;
        identity_credential_types
            .select(id)
            .filter(name.eq("oauth"))
            .first(conn)
            .await
            .map_err(|e| format!("could not get oauth credential type: {e}"))?
    };

    let connections = {
        use crate::schema::identity_credentials::dsl::*;
        identity_credentials
            .select(IdentityCredentials::as_select())
            .filter(identity_id.eq(i.id))
            .filter(credential_type_id.eq(_credential_type_id))
            .filter(
                credential
                    .contains(&serde_json::json!({
                        "provider": "spotify"
                    }))
                    .or(credential.contains(&serde_json::json!({
                        "provider": "github"
                    }))),
            )
            .load(conn)
            .await
            .map_err(|_| "could not query connected apps")?
    };

    Ok(Json(connections))
}
