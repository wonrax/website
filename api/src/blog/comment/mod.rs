pub mod create;
pub mod get;

use std::{cell::RefCell, rc::Rc};

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// The model that maps to the database table
#[derive(FromRow, Debug, Serialize, Clone)]
pub struct Comment {
    id: i32,
    author_name: String,
    content: String,
    parent_id: Option<i32>,
    created_at: chrono::NaiveDateTime,
    votes: i64,
    depth: i32,
}

// The model that will be returned to the client
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CommentTree {
    id: i32,
    author_name: String,
    content: String,
    parent_id: Option<i32>,
    created_at: chrono::NaiveDateTime,
    children: Option<Vec<Rc<RefCell<CommentTree>>>>,
    upvote: i64,
    depth: usize,

    #[serde(skip_serializing_if = "std::ops::Not::not")]
    is_comment_owner: bool,

    #[serde(skip_serializing_if = "std::ops::Not::not")]
    is_blog_author: bool,
}
