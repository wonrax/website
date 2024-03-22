use axum::http::request::Parts;

use crate::{error::Error, APIContext};

use self::models::identity::Identity;

pub mod models;
pub mod routes;

pub const COOKIE_NAME: &str = "auth_token";

#[derive(thiserror::Error, Debug)]
pub enum AuthenticationError {
    #[error("Authentication required, but no cookie `{COOKIE_NAME}` found in headers.")]
    NoCookie,

    #[error(
        "Unauthorized, please check if you're logged in by refreshing the \
         page. This could be due to an expired session or token has became invalid."
    )]
    Unauthorized,
}

pub struct MaybeAuthUser(pub Result<Identity, AuthenticationError>);

#[axum::async_trait]
impl axum::extract::FromRequestParts<APIContext> for MaybeAuthUser {
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &APIContext,
    ) -> Result<Self, Self::Rejection> {
        let jar = axum_extra::extract::cookie::CookieJar::from_headers(&parts.headers);

        // TODO implement and use an additional shorter cookie length and expiry
        // a.k.a. session token which will be cleared on browser close. This helps
        // speed up the auth process by comparing a shorter token instead of the
        // longer one. The longer one will be used to refresh the shorter one thus
        // has a longer expiry.
        let session_token: &str = if let Some(t) = jar.get(COOKIE_NAME) {
            t.value()
        } else {
            return Ok(MaybeAuthUser(Err(AuthenticationError::NoCookie)));
        };

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
        .fetch_optional(&state.pool)
        .await?;

        Ok(MaybeAuthUser(
            identity.ok_or(AuthenticationError::Unauthorized),
        ))
    }
}

pub struct AuthUser(pub Identity);

#[axum::async_trait]
impl axum::extract::FromRequestParts<APIContext> for AuthUser {
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &APIContext,
    ) -> Result<Self, Self::Rejection> {
        let MaybeAuthUser(auth_user) = MaybeAuthUser::from_request_parts(parts, state).await?;

        Ok(AuthUser(auth_user?))
    }
}
