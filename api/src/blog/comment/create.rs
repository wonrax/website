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
    identity::{self, MaybeAuthUser, models::identity::Traits},
    real_ip::ClientIp,
    schema::{blog_comments, blog_posts, identities},
};

use crate::blog::comment::Comment;

#[debug_handler]
pub async fn create_comment(
    State(ctx): State<App>,
    Path(slug): Path<String>,
    ClientIp(ip): ClientIp,
    MaybeAuthUser(auth_user): MaybeAuthUser,
    crate::json::Json(mut comment): crate::json::Json<CommentSubmission>,
) -> Result<Json<Comment>, AppError> {
    if let Err(ref e) = auth_user
        && matches!(e, identity::AuthenticationError::Unauthorized)
    {
        return Err(identity::AuthenticationError::Unauthorized.into());
    }

    comment
        .validate(auth_user.is_ok())
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
        author_name: comment.author_name.clone(),
        author_email: comment.author_email.clone(),
        identity_id: auth_user.ok().map(|u| u.id),
        content: comment.content.clone(),
        post_id,
        parent_id: comment.parent_id,
    };

    let resulting_comment = diesel::insert_into(blog_comments::table)
        .values(&new_comment)
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

    let mut author_name = resulting_comment.1.clone();

    if author_name.is_none() && resulting_comment.2.is_some() {
        let identity_traits = identities::table
            .filter(identities::id.eq(resulting_comment.2.unwrap()))
            .select(identities::traits)
            .first::<serde_json::Value>(&mut conn)
            .await
            .optional()?;

        if let Some(traits) = identity_traits {
            let traits: Traits = serde_json::from_value(traits).map_err(|_| "Invalid traits")?;
            author_name = traits.name.or_else(|| {
                tracing::error!(
                    "No name in traits found for identity ID `{}`",
                    resulting_comment.2.unwrap(),
                );
                Some("No name".into())
            });
        }
    }

    Ok(Json(Comment {
        id: resulting_comment.0,
        author_name: author_name.unwrap_or_else(|| "Anonymous".to_string()),
        content: resulting_comment.3,
        parent_id: resulting_comment.4,
        created_at: resulting_comment.5,
        votes: 0,
        depth: -1,
    }))
}

#[derive(Deserialize, Serialize)]
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
            if name.is_empty() {
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

        if self.content.is_empty() {
            return Err("No content provided");
        }

        if let Some(email) = self.author_email.take() {
            self.author_email = Some(email.trim().to_lowercase());

            if email.len() > 50 {
                return Err("Email too long");
            }

            if email.is_empty() {
                return Err("No email provided");
            }

            if !email.contains('@') {
                return Err("Invalid email");
            }
        }

        Ok(())
    }
}
