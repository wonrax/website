use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::Serialize;

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug, Serialize, Clone)]
#[diesel(table_name = crate::schema::blog_comment_votes)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BlogCommentVote {
    pub id: i32,
    pub comment_id: i32,
    pub ip: Option<String>,
    pub indentity_id: Option<i32>, // Note: keeping the typo from schema
    pub score: i32,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::blog_comment_votes)]
pub struct NewBlogCommentVote {
    pub comment_id: i32,
    pub ip: Option<String>,
    pub indentity_id: Option<i32>,
    pub score: i32,
}
