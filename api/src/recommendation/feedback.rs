use axum::{Json, extract::State, http::StatusCode};
use diesel::sql_types::{Array, Integer};
use diesel_async::RunQueryDsl;
use serde::Deserialize;

use crate::{App, error::AppError, identity::AuthUser};

use super::training;

const MAX_FEEDBACK_IDS: usize = 100;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FeedbackKind {
    /// The rows scrolled into view.
    Impression,
    /// A link on the row was opened.
    Click,
    /// "Not interested": hides the row and counts as a strong negative.
    Dismiss,
    /// Takes a dismiss back.
    Undismiss,
}

#[derive(Debug, Deserialize)]
pub struct FeedbackRequest {
    kind: FeedbackKind,
    ids: Vec<i32>,
}

/// Records how the owner reacted to feed rows. Anyone else gets a 403 so
/// visitors browsing the public page can't train the owner's feed.
pub async fn record_feedback(
    State(ctx): State<App>,
    AuthUser(identity): AuthUser,
    Json(request): Json<FeedbackRequest>,
) -> Result<StatusCode, AppError> {
    if identity.id != ctx.config.owner_identity_id {
        Err((
            "Only the site owner can give feed feedback",
            StatusCode::FORBIDDEN,
        ))?
    }

    if request.ids.len() > MAX_FEEDBACK_IDS {
        Err((
            "Too many ids in one feedback request",
            StatusCode::BAD_REQUEST,
        ))?
    }

    if request.ids.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }

    // Unknown ids are dropped by the join with online_articles instead of
    // failing the foreign key.
    let sql = match request.kind {
        // Reloading the page shouldn't wear an article out, so an article
        // counts as shown at most once an hour.
        FeedbackKind::Impression => {
            r#"
            INSERT INTO recommender_feedback (online_article_id, impressions, last_shown_at)
            SELECT id, 1, NOW() FROM online_articles WHERE id = ANY($1)
            ON CONFLICT (online_article_id) DO UPDATE SET
                impressions = recommender_feedback.impressions + 1,
                last_shown_at = NOW()
            WHERE recommender_feedback.last_shown_at IS NULL
                OR recommender_feedback.last_shown_at < NOW() - INTERVAL '1 hour'
            "#
        }
        FeedbackKind::Click => {
            r#"
            INSERT INTO recommender_feedback (online_article_id, clicked_at)
            SELECT id, NOW() FROM online_articles WHERE id = ANY($1)
            ON CONFLICT (online_article_id) DO UPDATE SET
                clicked_at = COALESCE(recommender_feedback.clicked_at, NOW())
            "#
        }
        FeedbackKind::Dismiss => {
            r#"
            INSERT INTO recommender_feedback (online_article_id, dismissed_at)
            SELECT id, NOW() FROM online_articles WHERE id = ANY($1)
            ON CONFLICT (online_article_id) DO UPDATE SET dismissed_at = NOW()
            "#
        }
        FeedbackKind::Undismiss => {
            r#"
            UPDATE recommender_feedback SET dismissed_at = NULL
            WHERE online_article_id = ANY($1)
            "#
        }
    };

    let mut conn = ctx.diesel.get().await?;
    diesel::sql_query(sql)
        .bind::<Array<Integer>, _>(&request.ids)
        .execute(&mut conn)
        .await?;

    // Impressions only matter once they pile up into a skip, which the
    // post-crawl retrain picks up.
    if !matches!(request.kind, FeedbackKind::Impression) {
        training::schedule_retrain(&ctx);
    }

    Ok(StatusCode::NO_CONTENT)
}
