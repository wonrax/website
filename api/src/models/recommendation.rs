use chrono::NaiveDateTime;
use diesel::prelude::*;
use pgvector::Vector;
use serde::Serialize;

#[derive(Queryable, Selectable, Debug, Serialize, Clone)]
#[diesel(table_name = crate::schema::online_article_sources)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OnlineArticleSource {
    pub id: i32,
    pub key: String,
    pub name: String,
    pub base_url: Option<String>,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::online_article_sources)]
pub struct NewArticleSource {
    pub key: String,
    pub name: String,
    pub base_url: Option<String>,
}

#[derive(Queryable, Selectable, Debug, Serialize, Clone)]
#[diesel(table_name = crate::schema::online_articles)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OnlineArticle {
    pub id: i32,
    pub url: String,
    pub title: String,
    pub content_text: Option<String>,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::online_articles)]
pub struct NewOnlineArticle {
    pub url: String,
    pub title: String,
    pub content_text: Option<String>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::online_article_chunks)]
pub struct NewArticleChunk {
    pub online_article_id: i32,
    pub embedding: Vector,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::online_article_metadata)]
pub struct NewArticleMetadata {
    pub online_article_id: i32,
    pub source_id: i32,
    pub external_score: Option<f64>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Queryable, Selectable, Debug, Serialize, Clone)]
#[diesel(table_name = crate::schema::user_history)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserHistory {
    pub id: i32,
    pub online_article_id: i32,
    pub weight: Option<f64>,
    pub added_at: NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::user_history)]
pub struct NewUserHistory {
    pub online_article_id: i32,
    pub weight: Option<f64>,
}
