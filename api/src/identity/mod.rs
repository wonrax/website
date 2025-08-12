use axum::http::request::Parts;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::{App, error::AppError};

use self::models::identity::Identity;

mod connected_apps;
mod spotify;

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
impl axum::extract::FromRequestParts<App> for MaybeAuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &App) -> Result<Self, Self::Rejection> {
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

        let identity = {
            use crate::schema::{identities, sessions};

            let mut conn = state.diesel.get().await?;

            sessions::table
                .inner_join(identities::table)
                .filter(sessions::token.eq(session_token))
                .filter(sessions::active.eq(true))
                .filter(sessions::expires_at.gt(diesel::dsl::now))
                .filter(sessions::issued_at.le(diesel::dsl::now))
                .select(identities::all_columns)
                .first::<Identity>(&mut conn)
                .await
                .optional()?
        };

        Ok(MaybeAuthUser(
            identity.ok_or(AuthenticationError::Unauthorized),
        ))
    }
}

pub struct AuthUser(pub Identity);

#[axum::async_trait]
impl axum::extract::FromRequestParts<App> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &App) -> Result<Self, Self::Rejection> {
        let MaybeAuthUser(auth_user) = MaybeAuthUser::from_request_parts(parts, state).await?;

        Ok(AuthUser(auth_user?))
    }
}
