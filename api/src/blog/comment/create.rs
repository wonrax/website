use axum::{
    Json, debug_handler,
    extract::{Path, State},
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde::{Deserialize, Serialize};

use crate::{
    App,
    blog::models::{NewBlogComment, NewBlogPost},
    error::AppError,
    identity::{AuthUser, models::identity::Traits},
    real_ip::ClientIp,
    schema::{blog_comments, blog_posts, identities},
};

use crate::blog::comment::Comment;

#[debug_handler]
pub async fn create_comment(
    State(ctx): State<App>,
    Path(slug): Path<String>,
    ClientIp(ip): ClientIp,
    AuthUser(auth_user): AuthUser,
    crate::json::Json(mut comment): crate::json::Json<CommentSubmission>,
) -> Result<Json<Comment>, AppError> {
    comment
        .validate()
        .map_err(|e| (e, axum::http::StatusCode::BAD_REQUEST))?;

    let mut conn = ctx.diesel.get().await?;

    // check if the post exists, otherwise create it
    let post_exists = blog_posts::table
        .filter(blog_posts::category.eq("blog"))
        .filter(blog_posts::slug.eq(&slug))
        .select(blog_posts::id)
        .first::<i32>(&mut conn)
        .await
        .optional()?;

    let post_id = if let Some(id) = post_exists {
        id
    } else {
        let new_post = NewBlogPost {
            category: "blog".to_string(),
            slug: slug.clone(),
            title: None,
        };

        diesel::insert_into(blog_posts::table)
            .values(&new_post)
            .on_conflict((blog_posts::category, blog_posts::slug))
            .do_nothing()
            .execute(&mut conn)
            .await?;

        blog_posts::table
            .filter(blog_posts::category.eq("blog"))
            .filter(blog_posts::slug.eq(&slug))
            .select(blog_posts::id)
            .first(&mut conn)
            .await?
    };

    // check if the parent comment actually belongs to the post
    if let Some(parent_id) = comment.parent_id {
        let parent_exists = blog_comments::table
            .filter(blog_comments::id.eq(parent_id))
            .filter(blog_comments::post_id.eq(post_id))
            .select(blog_comments::id)
            .first::<i32>(&mut conn)
            .await
            .optional()?;

        if parent_exists.is_none() {
            return Err("You're replying to the comment that does not belong to this post".into());
        }
    }

    let new_comment = NewBlogComment {
        author_ip: ip.to_string(),
        // Name comes from the linked identity's traits at read time; no
        // per-comment override is accepted now that auth is required.
        author_name: None,
        author_email: None,
        identity_id: Some(auth_user.id),
        content: comment.content.clone(),
        post_id,
        parent_id: comment.parent_id,
    };

    let resulting_comment = diesel::insert_into(blog_comments::table)
        .values(&new_comment)
        .returning((
            blog_comments::id,
            blog_comments::content,
            blog_comments::parent_id,
            blog_comments::created_at,
        ))
        .get_result::<(i32, String, Option<i32>, chrono::NaiveDateTime)>(&mut conn)
        .await?;

    let identity_traits = identities::table
        .filter(identities::id.eq(auth_user.id))
        .select(identities::traits)
        .first::<serde_json::Value>(&mut conn)
        .await
        .optional()?;

    let author_name = identity_traits
        .and_then(|traits| serde_json::from_value::<Traits>(traits).ok())
        .and_then(|t| t.name)
        .unwrap_or_else(|| {
            tracing::error!("No name in traits for identity ID `{}`", auth_user.id);
            "No name".into()
        });

    Ok(Json(Comment {
        id: resulting_comment.0,
        author_name,
        content: resulting_comment.1,
        parent_id: resulting_comment.2,
        created_at: resulting_comment.3,
        votes: 0,
        depth: -1,
    }))
}

#[derive(Deserialize, Serialize)]
pub struct CommentSubmission {
    content: String,
    parent_id: Option<i32>,
}

impl CommentSubmission {
    fn validate(&mut self) -> Result<(), &'static str> {
        self.content = self.content.trim().to_string();
        if self.content.len() > 5000 {
            return Err("Content too long (max 5000 characters)");
        }

        if self.content.is_empty() {
            return Err("No content provided");
        }

        Ok(())
    }
}
