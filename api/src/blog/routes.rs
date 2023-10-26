use std::{
    collections::HashMap,
    rc::Rc,
};

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

#[derive(FromRow, Debug, Serialize, Clone)]
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

#[derive(Debug, Clone, Serialize)]
struct CommentIntermediate {
    id: i32,
    author_name: String,
    content: String,
    parent_id: Option<i32>,
    created_at: chrono::NaiveDateTime,
    children: Option<Vec<Rc<CommentIntermediate>>>,
}

#[derive(Debug, Serialize)]
struct CommentView {
    author_name: String,
    content: String,
    created_at: String,
    children: Option<Vec<CommentView>>,
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
                },
            ),
        };

        (status_code, Json(json!(error_response))).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::DatabaseError(e)
    }
}

async fn get_blog_post_comments(
    State(ctx): State<APIContext>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<Rc<CommentIntermediate>>>, AppError> {
    let rows = sqlx::query_as::<_, Comment>(
        "
        SELECT comments.* FROM comments
        JOIN posts ON posts.id = comments.post_id
        WHERE posts.slug = $1;
        ",
    )
    .bind(slug)
    .fetch_all(&ctx.pool)
    .await?;

    let nested = turn_flat_comments_to_nested(rows.clone());

    Ok(Json(nested))
}

fn turn_flat_comments_to_nested(mut comments: Vec<Comment>) -> Vec<Rc<CommentIntermediate>> {
    comments.sort_by(|a, b| a.id.cmp(&b.id));

    let mut tree = HashMap::<i32, Rc<CommentIntermediate>>::new();
    for comment in comments {
        let c = Rc::new(CommentIntermediate {
            id: comment.id,
            author_name: comment.author_name,
            content: comment.content,
            parent_id: comment.parent_id,
            created_at: comment.created_at,
            children: None,
        });

        tree.insert(c.id, c.clone());
        println!("cid {:?}", c.id);
        if let Some(parent_id) = c.parent_id {
            let parent = tree.get_mut(&parent_id);
            if let Some(parent) = parent {
                let mut_parent = Rc::make_mut(parent);
                if let Some(children) = mut_parent.children.as_mut() {
                    children.push(c.clone());
                } else {
                    let children = vec![c.clone()];
                    mut_parent.children = Some(children);
                }
                println!("children of {:?}: {:?}", parent_id, mut_parent.children);
            }
        }
    }

    let mut final_comments: Vec<Rc<CommentIntermediate>> = vec![];
    for (_, comment) in &tree {
        if comment.parent_id.is_none() {
            final_comments.push(comment.clone());
        }
    }

    println!("tree {:?}", tree);

    final_comments
}
