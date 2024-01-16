use axum::{
    debug_handler,
    extract::{Path, State},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::{blog::routes::ClientIp, error::AppError, APIContext};

use crate::blog::comment::Comment;

#[debug_handler]
pub async fn create_comment(
    State(ctx): State<APIContext>,
    Path(slug): Path<String>,
    Extension(ip): Extension<ClientIp>,
    Json(mut comment): Json<CommentSubmission>,
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
            author_email,
            content,
            parent_id,
            post_id
        )
        VALUES (
            $1, 
            $2, 
            $3, 
            $4, 
            $5,
            (SELECT id FROM blog_posts WHERE category = 'blog' AND slug = $6)
        )
        -- TODO fix this hack (e.g. default values in comment struct)
        RETURNING *, 0::int8 as votes, -1 as depth;
        ",
    )
    .bind(ip.ip)
    .bind(comment.author_name)
    .bind(comment.author_email)
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
    author_email: Option<String>,
    content: String,
    parent_id: Option<i32>,
}

impl CommentSubmission {
    fn validate(&mut self) -> Result<(), &'static str> {
        self.author_name = self.author_name.trim().to_string();
        if self.author_name.len() < 1 {
            return Err("No author name provided");
        }

        if self.author_name.len() > 50 {
            return Err("Author name too long");
        }

        self.content = self.content.trim().to_string();
        if self.content.len() > 5000 {
            return Err("Content too long (max 5000 characters)");
        }

        if self.content.len() < 1 {
            return Err("No content provided");
        }

        if let Some(email) = self.author_email.take() {
            self.author_email = Some(email.trim().to_lowercase());

            if email.len() > 50 {
                return Err("Email too long");
            }

            if email.len() < 1 {
                return Err("No email provided");
            }

            if !email.contains('@') {
                return Err("Invalid email");
            }
        }

        Ok(())
    }
}
