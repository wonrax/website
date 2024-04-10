use axum::{
    debug_handler,
    extract::{Path, State},
    http::StatusCode,
};

use crate::{error::Error, identity::AuthUser, App};

#[debug_handler]
pub async fn delete_comment(
    State(ctx): State<App>,
    Path((_slug, id)): Path<(String, i32)>,
    AuthUser(auth_user): AuthUser,
) -> Result<(), Error> {
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

    sqlx::query!(
        "
        DELETE FROM blog_comments
        WHERE id = $1;
        ",
        id
    )
    .execute(&ctx.pool)
    .await?;

    Ok(())
}
