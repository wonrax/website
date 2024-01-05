use axum::{
    routing::{get, post},
    Router,
};

use crate::APIContext;

use super::comment::{create::submit_comment, get::get_blog_post_comments};

pub fn route() -> Router<APIContext> {
    Router::<APIContext>::new()
        .route("/:slug/comments", get(get_blog_post_comments))
        .route("/:slug/comments", post(submit_comment))
}
