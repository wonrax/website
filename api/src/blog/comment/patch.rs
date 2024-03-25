use axum::{
    debug_handler,
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use sqlx::FromRow;

use crate::{
    blog::comment::Comment, error::Error, identity::AuthUser, real_ip::ClientIp, APIContext,
};

#[debug_handler]
pub async fn patch_comment(
    State(ctx): State<APIContext>,
    Path((_slug, id)): Path<(String, i32)>,
    ClientIp(ip): ClientIp,
    AuthUser(auth_user): AuthUser,
    crate::json::Json(mut comment): crate::json::Json<CommentPatch>,
) -> Result<Json<Comment>, Error> {
    comment.content = comment.content.trim().to_string();

    if comment.content.is_empty() {
        return Err(("Content cannot be empty", StatusCode::BAD_REQUEST))?;
    }

    if comment.content.len() > 5000 {
        return Err((
            "Content too long (max 5000 characters)",
            StatusCode::BAD_REQUEST,
        ))?;
    }

    let is_owner = sqlx::query!(
        "
        SELECT EXISTS (
            SELECT 1 FROM blog_comments WHERE id = $1 AND identity_id = $2
        ) AS is_owner;
        ",
        id,
        auth_user.id
    )
    .fetch_one(&ctx.pool)
    .await?
    .is_owner
    .unwrap_or(false);

    if !is_owner {
        return Err((
            "You are not the owner of this comment",
            StatusCode::FORBIDDEN,
        ))?;
    }

    let mut resulting_comment = sqlx::query!(
        "
        UPDATE blog_comments SET (author_ip, content) = ($1, $2)
        WHERE id = $3
        RETURNING *;
        ",
        ip.to_string(),
        comment.content,
        id
    )
    .fetch_one(&ctx.pool)
    .await?;

    if resulting_comment.author_name.is_none() {
        resulting_comment.author_name = auth_user.traits.name;
    }

    Ok(Json(Comment {
        id: resulting_comment.id,
        author_name: resulting_comment.author_name.ok_or("missing author_name")?,
        content: resulting_comment.content,
        parent_id: resulting_comment.parent_id,
        created_at: resulting_comment.created_at,
        votes: 0,
        depth: -1,
    }))
}

#[derive(Deserialize, FromRow)]
pub struct CommentPatch {
    content: String,
}
