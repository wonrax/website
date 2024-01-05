use axum::{
    routing::{get, post},
    Router,
};

use crate::APIContext;

use super::comment::{create::create_comment, get::get_comments};

pub fn route() -> Router<APIContext> {
    Router::<APIContext>::new()
        .route("/:slug/comments", get(get_comments))
        .route("/:slug/comments", post(create_comment))
}
