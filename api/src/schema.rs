// @generated automatically by Diesel CLI.

#[allow(unused_imports)]
use diesel::sql_types::*;
pub type PgVector = pgvector::sql_types::Vector;

diesel::table! {
    _prisma_migrations (id) {
        #[max_length = 36]
        id -> Varchar,
        #[max_length = 64]
        checksum -> Varchar,
        finished_at -> Nullable<Timestamptz>,
        #[max_length = 255]
        migration_name -> Varchar,
        logs -> Nullable<Text>,
        rolled_back_at -> Nullable<Timestamptz>,
        started_at -> Timestamptz,
        applied_steps_count -> Int4,
    }
}

diesel::table! {
    blog_comment_votes (id) {
        id -> Int4,
        comment_id -> Int4,
        ip -> Nullable<Text>,
        indentity_id -> Nullable<Int4>,
        score -> Int4,
        created_at -> Timestamp,
    }
}

diesel::table! {
    blog_comments (id) {
        id -> Int4,
        author_ip -> Text,
        author_name -> Nullable<Text>,
        author_email -> Nullable<Text>,
        identity_id -> Nullable<Int4>,
        content -> Text,
        post_id -> Int4,
        parent_id -> Nullable<Int4>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    blog_posts (id) {
        id -> Int4,
        category -> Text,
        slug -> Text,
        title -> Nullable<Text>,
    }
}

diesel::table! {
    counters (id) {
        id -> Int4,
        key -> Text,
        name -> Text,
        count -> Int8,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    identities (id) {
        id -> Int4,
        traits -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    identity_credential_types (id) {
        id -> Int4,
        #[max_length = 64]
        name -> Varchar,
        created_at -> Timestamp,
    }
}

diesel::table! {
    identity_credentials (id) {
        id -> Int4,
        credential -> Nullable<Jsonb>,
        credential_type_id -> Int4,
        identity_id -> Int4,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    online_article_chunks (id) {
        id -> Int4,
        online_article_id -> Int4,
        embedding -> crate::schema::PgVector,
        created_at -> Timestamp,
    }
}

diesel::table! {
    online_article_metadata (id) {
        id -> Int4,
        online_article_id -> Int4,
        source_id -> Int4,
        external_score -> Nullable<Float8>,
        metadata -> Nullable<Jsonb>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    online_articles (id) {
        id -> Int4,
        url -> Text,
        title -> Text,
        content_text -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    sessions (id) {
        id -> Int4,
        #[max_length = 133]
        token -> Varchar,
        active -> Bool,
        issued_at -> Timestamp,
        expires_at -> Timestamp,
        identity_id -> Int4,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    online_article_sources (id) {
        id -> Int4,
        key -> Text,
        name -> Text,
        base_url -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    user_history (id) {
        id -> Int4,
        online_article_id -> Int4,
        weight -> Nullable<Float8>,
        added_at -> Timestamp,
    }
}

diesel::joinable!(blog_comment_votes -> blog_comments (comment_id));
diesel::joinable!(blog_comments -> blog_posts (post_id));
diesel::joinable!(blog_comments -> identities (identity_id));
diesel::joinable!(identity_credentials -> identities (identity_id));
diesel::joinable!(identity_credentials -> identity_credential_types (credential_type_id));
diesel::joinable!(online_article_chunks -> online_articles (online_article_id));
diesel::joinable!(online_article_metadata -> online_articles (online_article_id));
diesel::joinable!(online_article_metadata -> online_article_sources (source_id));
diesel::joinable!(sessions -> identities (identity_id));
diesel::joinable!(user_history -> online_articles (online_article_id));

diesel::allow_tables_to_appear_in_same_query!(
    _prisma_migrations,
    blog_comment_votes,
    blog_comments,
    blog_posts,
    counters,
    identities,
    identity_credential_types,
    identity_credentials,
    online_article_chunks,
    online_article_metadata,
    online_articles,
    sessions,
    online_article_sources,
    user_history,
);
