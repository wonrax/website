use core::panic;
use std::{collections::HashMap, time::SystemTime};

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap},
    response::IntoResponse,
    routing::get,
    Router,
};
use axum_extra::extract::{cookie::Expiration, CookieJar};
use chrono::Duration;

use crate::{error::AppError, APIContext};

use super::models::{
    credential::IdentityCredential,
    identity::{Identity, Traits},
    session::Session,
};

pub fn route() -> Router<APIContext> {
    // TODO rate limit these public endpoints
    Router::<APIContext>::new()
        .route("/me", get(handle_whoami))
        .route("/oidc/callback/github", get(handle_github_oauth_callback))
        .route("/login/oidc/github", get(handle_oidc_github_request))
    //
}

#[derive(serde::Serialize)]
pub struct WhoamiRespose {
    traits: Traits,
}

const COOKIE_NAME: &str = "auth_token";

#[axum::debug_handler]
pub async fn handle_whoami(
    State(ctx): State<APIContext>,
    jar: axum_extra::extract::cookie::CookieJar,
) -> Result<axum::Json<WhoamiRespose>, AppError> {
    // TODO implement and use an additional shorter cookie length and expiry
    // a.k.a. session token which will be cleared on browser close. This helps
    // speed up the auth process by comparing a shorter token instead of the
    // longer one. The longer one will be used to refresh the shorter one thus
    // has a longer expiry.
    let session_token: &str = jar.get(COOKIE_NAME).ok_or("no cookie in header")?.value();

    let identity = sqlx::query_as!(
        Identity,
        "
        SELECT i.*
        FROM sessions s JOIN identities i
        ON s.identity_id = i.id
        WHERE s.token = $1
        AND s.active = true
        AND s.expires_at > CURRENT_TIMESTAMP
        AND s.issued_at <= CURRENT_TIMESTAMP;
        ",
        session_token
    )
    .fetch_one(&ctx.pool)
    .await
    .map_err(|_| "invalid session token")?;

    Ok(axum::Json(WhoamiRespose {
        traits: identity.traits,
    }))
}

#[axum::debug_handler]
pub async fn handle_github_oauth_callback(
    State(ctx): State<APIContext>,
    Query(queries): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    let code = queries.get("code").ok_or("no code in query")?;

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
        .await
        .map_err(|_| "server network err")?
        .json()
        .await
        .map_err(|_| "json parse reponse from github err")?;

    let access_token = code_verify["access_token"].as_str();

    if access_token.is_none() {
        return Err("no access token".into());
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
        .await
        .map_err(|_| "network err")?
        .json()
        .await
        .map_err(|_| "login successfully, but user info from github json parse err")?;

    // TODO use a struct to deserialize into instead of this
    let user = user_info["login"]
        .as_str()
        .ok_or("github returned unexpected response")?;
    let user_id = user_info["id"]
        .as_i64()
        .ok_or("github returned unexpected response")?;
    let full_name = user_info["name"]
        .as_str()
        .ok_or("github returned unexpected response")?;

    let emails: serde_json::Value = reqwest::Client::new()
        .get("https://api.github.com/user/emails")
        .header("User-Agent", "reqwest")
        .header("Accept", "application/json")
        .header(
            "Authorization",
            "Bearer ".to_string() + access_token.unwrap(),
        )
        .send()
        .await
        .map_err(|_| "network err")?
        .json()
        .await
        .map_err(|_| "emails json parse err")?;

    // filter the email that is primary, verified:
    let email = emails
        .as_array()
        .ok_or("emails is not an array")?
        .iter()
        .find(|email| {
            email["primary"].as_bool().unwrap_or(false)
                && email["verified"].as_bool().unwrap_or(false)
        })
        .ok_or("no valid email found for this github account")?
        .get("email")
        .ok_or("valid email found, but couldn't extract it")?
        .as_str()
        .ok_or("valid email found, but couldn't extract it")?;

    let i = Identity::new_with_traits(Traits {
        name: Some(full_name.to_owned()),
        email: email.to_owned(),
    });

    // check if the user already exists
    // TODO this will not scale without a proper index, either create one (not
    // sure if it's efficient with jsonb) or use a different table for each
    // identifier
    let mut identity = sqlx::query_as!(
        Identity,
        "
        SELECT i.*
        FROM identities i JOIN identity_credentials ic
        ON i.id = ic.identity_id
        WHERE ic.credential->>'oidc_provider' = 'github'
        AND (ic.credential->>'user_id')::BIGINT = $1;
        ",
        user_id
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

        let credential = IdentityCredential::new_oidc_credential(serde_json::json!({
            "oidc_provider": "github",
            "user_id": user_id
        }));

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
            (SELECT id FROM identity_credential_types WHERE name = 'oidc'),
            $2,
            $3,
            $4
        );
        ",
            &credential.credential,
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

    Ok((
        // [(
        //     "Set-Cookie",
        //     // TODO get max age from session.expires_at
        //     // TODO use axum::extract::Cookie to build this
        //     format!(
        //         "{}={}; Secure; HttpOnly; Max-Age=3600; Path=/",
        //         COOKIE_NAME, session.token
        //     ),
        // )],
        CookieJar::new().add(auth_cookie),
        axum::Json(serde_json::json!({
            "github_user": user,
            "full_name": identity.traits.name,
            "email": identity.traits.email,
        })),
    ))
}

#[axum::debug_handler]
pub async fn handle_oidc_github_request() -> impl IntoResponse {
    // TODO move this to shared config
    let github_oauth_client_id: String = std::env::var("GITHUB_OAUTH_CLIENT_ID")
        .expect("GITHUB_OAUTH_CLIENT_ID is not set in .env file");
    let url = reqwest::Url::parse_with_params(
        "https://github.com/login/oauth/authorize",
        &[
            ("client_id", github_oauth_client_id.as_str()),
            ("scope", "user:email"),
        ],
    )
    .unwrap()
    .to_string();

    (axum::http::StatusCode::FOUND, [(header::LOCATION, url)]).into_response()
}
