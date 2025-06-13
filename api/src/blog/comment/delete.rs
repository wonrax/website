use axum::{
    debug_handler,
    extract::{Path, State},
    http::StatusCode,
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::{error::AppError, identity::AuthUser, schema::blog_comments, App};

#[debug_handler]
pub async fn delete_comment(
    State(ctx): State<App>,
    Path((_slug, id)): Path<(String, i32)>,
    AuthUser(auth_user): AuthUser,
) -> Result<(), AppError> {
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

    diesel::delete(blog_comments::table.filter(blog_comments::id.eq(id)))
        .execute(&mut conn)
        .await?;

    Ok(())
}
