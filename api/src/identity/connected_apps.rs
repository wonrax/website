use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use diesel::prelude::*;

#[derive(Queryable, Selectable)]
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

use crate::error::Error;

struct Spotify {
    display_name: String,
    added_on: DateTime<chrono::offset::Utc>,
}

pub async fn get_connected_apps() -> Result<impl IntoResponse, Error> {
    use crate::schema::identity_credentials::dsl::*;

    let conn = &mut PgConnection::establish("hehe").unwrap();
    let ic: Option<serde_json::Value> =
        identity_credentials.select(credential).first(conn).unwrap();

    println!("{}", ic.unwrap());

    Ok(())
}
