use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
    routing::{get, post},
    Extension, RequestExt, Router,
};

use crate::{identity::routes::COOKIE_NAME, APIContext};

use super::comment::{create::create_comment, get::get_comments};

pub fn route(state: APIContext) -> Router<APIContext> {
    // TODO rate limit these public endpoints
    Router::<APIContext>::new()
        .route("/:slug/comments", get(get_comments))
        .route("/:slug/comments", post(create_comment))
        .layer(axum::middleware::from_fn(client_ip))
        .layer(axum::middleware::from_fn_with_state(state, auth_user))
}

#[derive(Clone)]
pub struct ClientIp {
    pub ip: String, // TODO use IpAddr for correctness
}

async fn client_ip(mut req: Request, next: Next) -> Result<Response, StatusCode> {
    let ip: String = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next().map(|v| v.to_string()))
        .unwrap_or_else(|| {
            req.extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .unwrap()
                .0
                .ip()
                .to_string()
        });

    req.extensions_mut().insert(ClientIp { ip });

    Ok(next.run(req).await)
}

async fn auth_user(
    State(ctx): State<APIContext>,
    jar: axum_extra::extract::cookie::CookieJar,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let session_token = jar.get(COOKIE_NAME).map(|c| c.value());

    match session_token {
        Some(token) => {
            let identity = sqlx::query!(
                "
                SELECT i.id
                FROM sessions s JOIN identities i
                ON s.identity_id = i.id
                WHERE s.token = $1
                AND s.active = true
                AND s.expires_at > CURRENT_TIMESTAMP
                AND s.issued_at <= CURRENT_TIMESTAMP;
                ",
                token
            )
            .fetch_one(&ctx.pool)
            .await
            .ok();

            match identity {
                Some(identity) => {
                    req.extensions_mut()
                        .insert(Some(AuthUser { id: identity.id }));
                }
                None => {
                    // return unauthorized
                    return Ok(Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body("unauthorized".into())
                        .unwrap());
                }
            }
        }
        None => {
            req.extensions_mut().insert(Option::<AuthUser>::None);
        }
    }

    Ok(next.run(req).await)
}

#[derive(Clone, Debug)]
pub struct AuthUser {
    pub id: i32,
}
