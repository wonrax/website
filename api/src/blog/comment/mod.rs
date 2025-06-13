pub mod create;
pub mod delete;
pub mod get;
pub mod patch;

use std::fmt::Debug;

use serde::{Deserialize, Serialize};

// The model that maps to the database table
#[derive(Debug, Serialize, Clone)]
pub struct Comment {
    pub id: i32,
    pub author_name: String,
    pub content: String,
    pub parent_id: Option<i32>,
    pub created_at: chrono::NaiveDateTime,
    pub votes: i64,
    pub depth: i64,
}

// The model that will be returned to the client
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CommentTree {
    pub id: i32,
    pub author_name: String,
    pub content: String,
    pub parent_id: Option<i32>,
    pub created_at: chrono::NaiveDateTime,
    pub children: Option<Vec<CommentTree>>,
    pub upvote: i64,
    pub depth: usize,
    pub is_comment_owner: bool,
    pub is_blog_author: bool,
}
