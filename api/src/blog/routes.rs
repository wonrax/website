use std::net::SocketAddr;

use axum::{
    extract::ConnectInfo,
    http::request::Parts,
    routing::{get, post},
    Router,
};

use crate::{error::Error, APIContext};

use super::comment::{create::create_comment, get::get_comments};

pub fn route() -> Router<APIContext> {
    // TODO rate limit these public endpoints
    Router::<APIContext>::new()
        .route("/:slug/comments", get(get_comments))
        .route("/:slug/comments", post(create_comment))
}

#[derive(Clone)]
pub struct ClientIp {
    pub ip: String, // TODO use IpAddr for correctness
}

#[axum::async_trait]
impl axum::extract::FromRequestParts<APIContext> for ClientIp {
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &APIContext,
    ) -> Result<Self, Self::Rejection> {
        let ip = parts
            .headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next().map(|v| v.to_string()))
            .unwrap_or(
                parts
                    .extensions
                    .get::<ConnectInfo<SocketAddr>>()
                    .ok_or("missing ConnectInfo")?
                    .0
                    .ip()
                    .to_string(),
            );

        Ok(ClientIp { ip })
    }
}
