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
use sqlx::{error::DatabaseError, FromRow};

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
    depth: i32,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
struct CommentView {
    id: i32,
    author_name: String,
    content: String,
    parent_id: Option<i32>,
    created_at: chrono::NaiveDateTime,
    children: Option<Vec<Rc<RefCell<CommentView>>>>,
    upvote: i32,
    depth: usize,
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
) -> Result<Json<Vec<CommentView>>, AppError> {
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
        ), t(parent_id, id, author_name, content, root, depth, created_at, upvote) AS (
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
                t.depth + 1,
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

    // let result = tree_based_sorting(rows);
    let result = array_based_sorting(rows);

    Ok(Json(result))
}

fn tree_based_sorting(comments: Vec<Comment>) -> Vec<CommentView> {
    let mut nested = turn_flat_comments_to_nested(comments);
    sort_comments_by_upvote(&mut nested);

    let mut result: Vec<CommentView> = vec![];
    for comment in nested {
        depth_first_search(comment.clone(), &mut result);
    }

    // remove all children that are still referenced
    for comment in &mut result {
        comment.children = None;
    }

    result
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
            depth: comment.depth as usize,
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

fn depth_first_search(comment: Rc<RefCell<CommentView>>, mut result: &mut Vec<CommentView>) {
    result.push(comment.borrow_mut().to_owned());
    if let Some(children) = comment.borrow().children.as_ref() {
        for child in children {
            depth_first_search(child.clone(), &mut result);
        }
    }
}

fn array_based_sorting(comments: Vec<Comment>) -> Vec<CommentView> {
    sort_array(comments)
        .into_iter()
        .map(|comment| CommentView {
            id: comment.id,
            author_name: comment.author_name,
            content: comment.content,
            parent_id: comment.parent_id,
            created_at: comment.created_at,
            children: None,
            upvote: comment.upvote,
            depth: comment.depth as usize,
        })
        .collect()
}

fn sort_array(comments: Vec<Comment>) -> Vec<Comment> {
    if comments.len() == 0 {
        return vec![];
    }
    let mut result = vec![];

    let mut top_level_comments_children: HashMap<i32, Vec<Comment>> = HashMap::new();

    let mut i = 0;
    let mut current_root_comment_id = comments[0].id;
    let root_depth = comments[0].depth;
    while i < comments.len() {
        let comment = &comments[i];
        if comment.depth > root_depth {
            top_level_comments_children
                .entry(current_root_comment_id)
                .or_insert(vec![])
                .push(comment.clone());
        } else {
            current_root_comment_id = comment.id;
            result.push(comment.clone());
            top_level_comments_children.insert(comment.id, vec![]);
        }
        i += 1;
    }

    result.sort_unstable_by_key(|k| (-k.upvote, k.created_at));

    let mut curr = 0;
    for top_level_comment in result.clone() {
        let children = top_level_comments_children
            .remove(&top_level_comment.id)
            .unwrap();
        let children_length = children.len();
        let sorted_children = sort_array(children);
        for (k, child_comment) in sorted_children.iter().enumerate() {
            result.insert(k + curr + 1, child_comment.to_owned());
        }
        curr += children_length + 1;
    }

    result
}

#[cfg(test)]
mod test {
    use rand::Rng;

    use super::Comment;

    #[test]
    fn test_correctness_using_two_implementations() {
        let comments = generate_comments(10000, 5);
        let tree_based_sorted_comments = super::tree_based_sorting(comments.clone());
        let array_based_sorted_comments = super::array_based_sorting(comments.clone());

        assert_eq!(tree_based_sorted_comments, array_based_sorted_comments);
    }

    // Generate comments randomly
    fn generate_comments(n: usize, max_depth: usize) -> Vec<Comment> {
        let mut comments = vec![];
        for i in 0..n {
            let depth = rand::thread_rng().gen_range(0..max_depth + 1);
            let comment = Comment {
                id: i as i32,
                author_name: "author".to_string(),
                content: "content".to_string(),
                parent_id: None,
                created_at: chrono::offset::Local::now().naive_local(),
                upvote: 0,
                depth: depth as i32,
            };
            comments.push(comment);
        }
        comments
    }
}
