use axum::{
    debug_handler,
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::{error::AppError, APIContext};

use crate::blog::comment::Comment;

#[debug_handler]
pub async fn submit_comment(
    State(ctx): State<APIContext>,
    Path(slug): Path<String>,
    Json(comment): Json<CommentSubmission>,
) -> Result<Json<Comment>, AppError> {
    comment.validate()?;
    // check if the post exists, otherwise create it
    let exists = sqlx::query_as::<_, (bool,)>(
        "
        SELECT EXISTS (
            SELECT id FROM blog_posts WHERE category = 'blog' AND slug = $1
        );
        ",
    )
    .bind(&slug)
    .fetch_one(&ctx.pool)
    .await?;

    if exists.0 == false {
        sqlx::query(
            "
            INSERT INTO blog_posts (category, slug)
            VALUES ('blog', $1)
            ON CONFLICT (category, slug) DO NOTHING;
            ",
        )
        .bind(&slug)
        .execute(&ctx.pool)
        .await?;
    }

    // check if the parent comment actually belongs to the post
    if let Some(parent_id) = comment.parent_id {
        let exists = sqlx::query_as::<_, (bool,)>(
            "
            SELECT EXISTS (
                SELECT id FROM blog_comments WHERE id = $1 AND post_id = (
                    SELECT id FROM blog_posts WHERE category = 'blog' AND slug = $2
                )
            );
            ",
        )
        .bind(parent_id)
        .bind(&slug)
        .fetch_one(&ctx.pool)
        .await?;

        if exists.0 == false {
            return Err("You're replying to the comment that does not belong to this post".into());
        }
    }

    let resulting_comment = sqlx::query_as::<_, Comment>(
        "
        INSERT INTO blog_comments (
            author_ip,
            author_name,
            content,
            parent_id,
            post_id
        )
        VALUES (
            '192.169.1.1', 
            $1, 
            $2, 
            $3, 
            (SELECT id FROM blog_posts WHERE category = 'blog' AND slug = $4)
        )
        RETURNING *, -1 as depth;
        ",
    )
    .bind(comment.author_name)
    .bind(comment.content)
    .bind(comment.parent_id)
    .bind(&slug)
    .fetch_one(&ctx.pool)
    .await?;

    Ok(Json(resulting_comment))
}

#[derive(Deserialize, Serialize, FromRow)]
pub struct CommentSubmission {
    author_name: String,
    content: String,
    parent_id: Option<i32>,
}

impl CommentSubmission {
    fn validate(&self) -> Result<(), &'static str> {
        if self.author_name.len() < 1 {
            return Err("No author name provided");
        }

        if self.author_name.len() > 50 {
            return Err("Author name too long");
        }

        if self.content.len() > 1000 {
            return Err("Content too long");
        }

        if self.content.len() < 1 {
            return Err("No content provided");
        }

        Ok(())
    }
}
