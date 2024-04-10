use axum::{
    debug_handler,
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::{
    error::Error,
    identity::{self, models::identity::Traits, MaybeAuthUser},
    real_ip::ClientIp,
    App,
};

use crate::blog::comment::Comment;

#[debug_handler]
pub async fn create_comment(
    State(ctx): State<App>,
    Path(slug): Path<String>,
    ClientIp(ip): ClientIp,
    MaybeAuthUser(auth_user): MaybeAuthUser,
    crate::json::Json(mut comment): crate::json::Json<CommentSubmission>,
) -> Result<Json<Comment>, Error> {
    if let Err(ref e) = auth_user {
        if matches!(e, identity::AuthenticationError::Unauthorized) {
            return Err(identity::AuthenticationError::Unauthorized.into());
        }
    }

    comment
        .validate(auth_user.is_ok())
        .map_err(|e| (e, axum::http::StatusCode::BAD_REQUEST))?;

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

    let mut resulting_comment = sqlx::query!(
        "
        INSERT INTO blog_comments (
            author_ip,
            author_name,
            author_email,
            identity_id,
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
            $6,
            (SELECT id FROM blog_posts WHERE category = 'blog' AND slug = $7)
        )
        -- TODO fix this hack (e.g. default values in comment struct)
        RETURNING *, 0::int8 as votes, -1 as depth;
        ",
        ip.to_string(),
        comment.author_name,
        comment.author_email,
        auth_user.ok().map(|u| u.id),
        comment.content,
        comment.parent_id,
        &slug,
    )
    .fetch_one(&ctx.pool)
    .await?;

    if resulting_comment.author_name.is_none() {
        let identity = sqlx::query!(
            "
            SELECT traits FROM identities WHERE id = $1;
            ",
            resulting_comment.identity_id
        )
        .fetch_optional(&ctx.pool)
        .await?
        .map(|i| i.traits);

        if let Some(traits) = identity {
            let traits: Traits = serde_json::from_value(traits).map_err(|_| "Invalid traits")?;
            resulting_comment.author_name = Some(traits.name.unwrap_or_else(|| {
                tracing::error!(
                    "No name in traits found for identity ID `{}`",
                    resulting_comment.identity_id.unwrap(),
                );
                "No name".into()
            }));
        }
    }

    Ok(Json(Comment {
        id: resulting_comment.id,
        author_name: resulting_comment.author_name.unwrap(),
        content: resulting_comment.content,
        parent_id: resulting_comment.parent_id,
        created_at: resulting_comment.created_at,
        votes: 0,
        depth: -1,
    }))
}

#[derive(Deserialize, Serialize, FromRow)]
pub struct CommentSubmission {
    author_name: Option<String>,
    author_email: Option<String>,
    content: String,
    parent_id: Option<i32>,
}

impl CommentSubmission {
    fn validate(&mut self, is_auth: bool) -> Result<(), &'static str> {
        if let Some(mut name) = self.author_name.take() {
            name = name.trim().to_string();
            if name.len() < 1 {
                return Err("No author name provided");
            }

            if name.len() > 50 {
                return Err("Author name too long");
            }

            self.author_name = Some(name);
        } else if !is_auth {
            return Err("No author name provided");
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
