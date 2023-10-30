use std::{cell::RefCell, collections::HashMap, rc::Rc};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
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
    author_name: String,
    content: String,
    parent_id: Option<i32>,
    created_at: chrono::NaiveDateTime,
    upvote: i32,
}

#[derive(Debug, Clone, Serialize)]
struct CommentView {
    id: i32,
    author_name: String,
    content: String,
    parent_id: Option<i32>,
    created_at: chrono::NaiveDateTime,
    children: Option<Vec<Rc<RefCell<CommentView>>>>,
    upvote: i32,
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

#[derive(Deserialize)]
struct Pagination {
    page_offset: usize,
    page_size: usize,
}

async fn get_blog_post_comments(
    State(ctx): State<APIContext>,
    Path(slug): Path<String>,
    pagination: Query<Pagination>,
) -> Result<Json<Vec<Rc<RefCell<CommentView>>>>, AppError> {
    let rows = sqlx::query_as::<_, Comment>(
        "
        WITH RECURSIVE root_comments AS (
            SELECT
                comments.parent_id,
                comments.id,
                comments.author_name,
                comments.content,
                ARRAY [comments.id],
                0,
                comments.created_at,
                comments.upvote
            FROM comments
            JOIN posts ON (posts.id = comments.post_id)
            WHERE posts.category = 'blog'
            AND posts.slug = $1
            AND comments.parent_id IS NULL
            ORDER BY comments.upvote DESC, comments.created_at
            LIMIT $2 OFFSET $3
        ), t(parent_id, id, author_name, content, root, level, created_at, upvote) AS (
            (
                SELECT * FROM root_comments
            )
            UNION ALL
            SELECT
                comments.parent_id,
                comments.id,
                comments.author_name,
                comments.content,
                array_append(root, comments.id),
                t.level + 1,
                comments.created_at,
                comments.upvote
            FROM t
                JOIN comments ON (comments.parent_id = t.id)
        )
        SELECT * FROM t
        ORDER BY root;
        ",
    )
    .bind(slug)
    .bind(pagination.page_size as i64)
    .bind(pagination.page_offset as i64)
    .fetch_all(&ctx.pool)
    .await?;

    let mut nested = turn_flat_comments_to_nested(rows.clone());
    sort_comments_by_upvote(&mut nested);

    let mut result: Vec<Rc<RefCell<CommentView>>> = vec![];
    for comment in nested {
        depth_first_search(comment.clone(), &mut result);
    }

    // remove all children that are still referenced
    for comment in &result {
        comment.borrow_mut().children = None;
    }

    Ok(Json(result))
}

fn turn_flat_comments_to_nested(comments: Vec<Comment>) -> Vec<Rc<RefCell<CommentView>>> {
    let mut tree = HashMap::<i32, Rc<RefCell<CommentView>>>::new();
    for comment in comments {
        let c = Rc::new(RefCell::new(CommentView {
            id: comment.id,
            author_name: comment.author_name,
            content: comment.content,
            parent_id: comment.parent_id,
            created_at: comment.created_at,
            children: None,
            upvote: comment.upvote,
        }));

        tree.insert(c.borrow().id, c.clone());
        if let Some(parent_id) = c.borrow().parent_id {
            let parent = tree.get(&parent_id);
            if let Some(parent) = parent {
                let mut mut_parent = parent.borrow_mut();
                if let Some(children) = mut_parent.children.as_mut() {
                    children.push(c.clone());
                } else {
                    let children = vec![c.clone()];
                    mut_parent.children = Some(children);
                }
            }
        };
    }

    let mut final_comments: Vec<Rc<RefCell<CommentView>>> = vec![];
    for (_, comment) in &tree {
        if comment.borrow().parent_id.is_none() {
            final_comments.push(comment.clone());
        }
    }

    final_comments
}

fn sort_comments_by_upvote(comments: &mut Vec<Rc<RefCell<CommentView>>>) {
    // sort the array
    comments.sort_unstable_by_key(|k| (-k.borrow().upvote, k.borrow().created_at));

    // sort the children
    for comment in comments {
        if let Some(children) = comment.borrow_mut().children.as_mut() {
            sort_comments_by_upvote(children);
        }
    }
}

fn depth_first_search(
    comment: Rc<RefCell<CommentView>>,
    mut result: &mut Vec<Rc<RefCell<CommentView>>>,
) {
    result.push(comment.clone());
    if let Some(children) = comment.borrow().children.as_ref() {
        for child in children {
            depth_first_search(child.clone(), &mut result);
        }
    }
}
