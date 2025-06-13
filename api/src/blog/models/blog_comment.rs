use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::Serialize;

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug, Serialize, Clone)]
#[diesel(table_name = crate::schema::blog_comments)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BlogComment {
    pub id: i32,
    pub author_ip: String,
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    pub identity_id: Option<i32>,
    pub content: String,
    pub post_id: i32,
    pub parent_id: Option<i32>,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::blog_comments)]
pub struct NewBlogComment {
    pub author_ip: String,
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    pub identity_id: Option<i32>,
    pub content: String,
    pub post_id: i32,
    pub parent_id: Option<i32>,
}

#[derive(AsChangeset, Debug)]
#[diesel(table_name = crate::schema::blog_comments)]
pub struct UpdateBlogComment {
    pub content: Option<String>,
}
