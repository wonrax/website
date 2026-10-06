// @generated automatically by Diesel CLI.

#[allow(unused_imports)]
use diesel::{query_builder::QueryId, sql_types::*};

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
    chatgpt_auth (id) {
        id -> Int4,
        access_token -> Text,
        refresh_token -> Text,
        id_token -> Nullable<Text>,
        account_id -> Nullable<Text>,
        expires_at -> Timestamptz,
        refreshed_at -> Timestamptz,
        created_at -> Timestamptz,
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
    discord_memory_docs (id) {
        id -> Int8,
        guild_id -> Int8,
        kind -> Text,
        subject_id -> Int8,
        content -> Text,
        dreamed_through -> Int8,
        revisit_on -> Nullable<Date>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    discord_memory_observations (id) {
        id -> Int8,
        guild_id -> Int8,
        channel_id -> Int8,
        about_user_ids -> Array<Int8>,
        content -> Text,
        keywords -> Text,
        source_message_ids -> Array<Int8>,
        observed_at -> Timestamptz,
        created_at -> Timestamptz,
        withdrawn_at -> Nullable<Timestamptz>,
        withdrawn_seq -> Nullable<Int8>,
    }
}

diesel::table! {
    discord_memory_people (guild_id, user_id) {
        guild_id -> Int8,
        user_id -> Int8,
        name -> Text,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    discord_sandboxes (channel_id) {
        channel_id -> Int8,
        last_used_at -> Timestamptz,
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
    online_article_metadata (id) {
        id -> Int4,
        online_article_id -> Int4,
        source_id -> Int4,
        external_score -> Nullable<Float8>,
        metadata -> Nullable<Jsonb>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        submitted_at -> Timestamp,
    }
}

diesel::table! {
    online_articles (id) {
        id -> Int4,
        url -> Text,
        title -> Text,
        recommender_terms -> Nullable<Jsonb>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    recommender_feedback (online_article_id) {
        online_article_id -> Int4,
        impressions -> Int4,
        last_shown_at -> Nullable<Timestamptz>,
        clicked_at -> Nullable<Timestamptz>,
        dismissed_at -> Nullable<Timestamptz>,
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
diesel::joinable!(online_article_metadata -> online_articles (online_article_id));
diesel::joinable!(online_article_metadata -> online_article_sources (source_id));
diesel::joinable!(recommender_feedback -> online_articles (online_article_id));
diesel::joinable!(sessions -> identities (identity_id));
diesel::joinable!(user_history -> online_articles (online_article_id));

diesel::allow_tables_to_appear_in_same_query!(
    _prisma_migrations,
    blog_comment_votes,
    blog_comments,
    blog_posts,
    chatgpt_auth,
    counters,
    discord_memory_docs,
    discord_memory_observations,
    discord_memory_people,
    discord_sandboxes,
    identities,
    identity_credential_types,
    identity_credentials,
    online_article_metadata,
    online_articles,
    recommender_feedback,
    sessions,
    online_article_sources,
    user_history,
);
