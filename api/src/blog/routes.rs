use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Serialize;
use serde_json::json;
use sqlx::FromRow;

use crate::APIContext;

pub fn route() -> Router<APIContext> {
    Router::<APIContext>::new().route("/:slug/comments", get(get_blog_post_comments))
}

#[derive(Serialize)]
struct ErrorResponse {
    code: String,
    msg: Option<String>,
}

#[derive(FromRow, Debug, Serialize)]
struct Comment {
    id: i32,
    author_ip: String,
    author_name: String,
    author_email: Option<String>,
    content: String,
    post_id: i32,
    parent_id: Option<i32>,
    created_at: chrono::NaiveDateTime,
}

#[derive(Debug, Serialize)]
struct CommentView {
    author_name: String,
    content: String,
    created_at: String,
    children: Vec<CommentView>,
}

enum AppError {
    DatabaseError(sqlx::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status_code, error_response) = match self {
            AppError::DatabaseError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorResponse {
                    code: "DB_ERR".into(),
                    msg: Some(format!("Fetching data error: {}", e.to_string())),
                })
        };

        (status_code, Json(json!(error_response))).into_response()
    }
}

impl From<sqlx::Error> for AppError
{
    fn from(e: sqlx::Error) -> Self {
        AppError::DatabaseError(e)
    }
}

async fn get_blog_post_comments(
    State(ctx): State<APIContext>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<Comment>>, AppError> {
    let rows = sqlx::query_as::<_, Comment>(
        "
        SELECT * FROM comments
        JOIN posts ON posts.id = comments.post_id
        WHERE posts.slug = $1;
        ",
    )
    .bind(slug)
    .fetch_all(&ctx.pool)
    .await?;

    Ok(Json(rows))
}
