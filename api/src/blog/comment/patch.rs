use axum::{
    debug_handler,
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde::Deserialize;

use crate::{
    blog::comment::Comment,
    blog::models::UpdateBlogComment,
    error::AppError,
    identity::AuthUser,
    real_ip::ClientIp,
    schema::{blog_comments, identities},
    App,
};

#[debug_handler]
pub async fn patch_comment(
    State(ctx): State<App>,
    Path((_slug, id)): Path<(String, i32)>,
    ClientIp(_ip): ClientIp,
    AuthUser(auth_user): AuthUser,
    crate::json::Json(mut comment): crate::json::Json<CommentPatch>,
) -> Result<Json<Comment>, AppError> {
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

    let mut conn = ctx.diesel.get().await?;

    let is_owner = blog_comments::table
        .filter(blog_comments::id.eq(id))
        .filter(blog_comments::identity_id.eq(auth_user.id))
        .select(blog_comments::id)
        .first::<i32>(&mut conn)
        .await
        .optional()?;

    if is_owner.is_none() {
        return Err((
            "You are not the owner of this comment",
            StatusCode::FORBIDDEN,
        ))?;
    }

    let update_comment = UpdateBlogComment {
        content: Some(comment.content.clone()),
    };

    let updated_comment = diesel::update(blog_comments::table.filter(blog_comments::id.eq(id)))
        .set(&update_comment)
        .returning((
            blog_comments::id,
            blog_comments::author_name,
            blog_comments::identity_id,
            blog_comments::content,
            blog_comments::parent_id,
            blog_comments::created_at,
        ))
        .get_result::<(
            i32,
            Option<String>,
            Option<i32>,
            String,
            Option<i32>,
            chrono::NaiveDateTime,
        )>(&mut conn)
        .await?;

    let mut author_name = updated_comment.1.clone();

    if author_name.is_none() && updated_comment.2.is_some() {
        let identity_traits = identities::table
            .filter(identities::id.eq(updated_comment.2.unwrap()))
            .select(identities::traits)
            .first::<serde_json::Value>(&mut conn)
            .await
            .optional()?;

        if let Some(traits) = identity_traits {
            let traits: crate::identity::models::identity::Traits =
                serde_json::from_value(traits).map_err(|_| "Invalid traits")?;
            author_name = traits.name;
        }
    }

    Ok(Json(Comment {
        id: updated_comment.0,
        author_name: author_name.unwrap_or_else(|| "Anonymous".to_string()),
        content: updated_comment.3,
        parent_id: updated_comment.4,
        created_at: updated_comment.5,
        votes: 0,
        depth: -1,
    }))
}

#[derive(Deserialize)]
pub struct CommentPatch {
    content: String,
}
