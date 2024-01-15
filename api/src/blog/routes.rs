use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::Response,
    routing::{get, post},
    Router,
};

use crate::APIContext;

use super::comment::{create::create_comment, get::get_comments};

pub fn route() -> Router<APIContext> {
    Router::<APIContext>::new()
        .route("/:slug/comments", get(get_comments))
        .route("/:slug/comments", post(create_comment))
        .layer(axum::middleware::from_fn(client_ip))
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
