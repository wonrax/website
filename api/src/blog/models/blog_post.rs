use diesel::prelude::*;
use serde::Serialize;

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug, Serialize, Clone)]
#[diesel(table_name = crate::schema::blog_posts)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BlogPost {
    pub id: i32,
    pub category: String,
    pub slug: String,
    pub title: Option<String>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::blog_posts)]
pub struct NewBlogPost {
    pub category: String,
    pub slug: String,
    pub title: Option<String>,
}
