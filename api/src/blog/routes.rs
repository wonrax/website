use axum::{
    routing::{delete, get, patch, post},
    Router,
};

use crate::APIContext;

use super::comment::{
    create::create_comment, delete::delete_comment, get::get_comments, patch::patch_comment,
};

pub fn route() -> Router<APIContext> {
    // TODO rate limit these public endpoints
    Router::<APIContext>::new()
        .route("/:slug/comments", get(get_comments))
        .route("/:slug/comments", post(create_comment))
        .route("/:slug/comments/:id", patch(patch_comment))
        .route("/:slug/comments/:id", delete(delete_comment))
}
