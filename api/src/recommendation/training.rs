use std::sync::{Arc, atomic::Ordering};

use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Bool, Float8, Integer, Jsonb, Nullable, Text};
use diesel_async::RunQueryDsl;

use crate::App;

use super::{
    model::{self, Example, TasteModel},
    parse_sources, sources_json_sql,
};

/// An article shown this many times without a click counts as skipped.
pub const SKIPPED_AFTER_IMPRESSIONS: i32 = 3;

/// Raindrop collection weights are 0.1 to 0.8, this puts a great read above a
/// click.
const HISTORY_WEIGHT_SCALE: f64 = 2.0;
const CLICK_WEIGHT: f64 = 1.0;
const DISMISS_WEIGHT: f64 = 1.5;
const SKIP_WEIGHT: f64 = 0.3;

const BACKGROUND_PER_HISTORY_ITEM: usize = 3;
const MIN_BACKGROUND_SAMPLE: usize = 200;
const MAX_BACKGROUND_SAMPLE: usize = 3000;

#[derive(QueryableByName)]
struct HistoryRow {
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Text)]
    url: String,
    #[diesel(sql_type = Nullable<Jsonb>)]
    recommender_terms: Option<serde_json::Value>,
    #[diesel(sql_type = Float8)]
    weight: f64,
}

#[derive(QueryableByName)]
struct FeedbackRow {
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Text)]
    url: String,
    #[diesel(sql_type = Nullable<Jsonb>)]
    recommender_terms: Option<serde_json::Value>,
    #[diesel(sql_type = Nullable<Jsonb>)]
    sources: Option<serde_json::Value>,
    #[diesel(sql_type = Bool)]
    clicked: bool,
    #[diesel(sql_type = Bool)]
    dismissed: bool,
}

#[derive(QueryableByName)]
struct BackgroundRow {
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Text)]
    url: String,
    #[diesel(sql_type = Nullable<Jsonb>)]
    recommender_terms: Option<serde_json::Value>,
}

/// The current model, trained on the spot if nothing has trained one yet.
pub async fn taste_model(ctx: &App) -> Result<Arc<TasteModel>, eyre::Error> {
    if let Some(model) = ctx.recommendation.taste_model.load_full() {
        return Ok(model);
    }

    let _training = ctx.recommendation.training.lock().await;
    if let Some(model) = ctx.recommendation.taste_model.load_full() {
        return Ok(model);
    }

    train_and_store(ctx).await
}

/// Retrains in the background. Calls made while a retrain is still waiting to
/// start fold into it, since it reads their feedback anyway.
pub fn schedule_retrain(ctx: &App) {
    if ctx
        .recommendation
        .retrain_queued
        .swap(true, Ordering::AcqRel)
    {
        return;
    }

    let ctx = ctx.clone();
    tokio::spawn(async move {
        let _training = ctx.recommendation.training.lock().await;
        ctx.recommendation
            .retrain_queued
            .store(false, Ordering::Release);

        if let Err(err) = train_and_store(&ctx).await {
            tracing::warn!(?err, "Failed to retrain the taste model");
        }
    });
}

/// Callers hold `RecommendationSystem::training` so two trainings never race
/// to store their model.
async fn train_and_store(ctx: &App) -> Result<Arc<TasteModel>, eyre::Error> {
    let examples = load_examples(ctx).await?;

    let model = tokio::task::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let model = TasteModel::train(&examples);
        tracing::info!(
            examples = examples.len(),
            elapsed = ?started.elapsed(),
            "Trained the taste model"
        );
        model
    })
    .await?;

    let model = Arc::new(model);
    ctx.recommendation.taste_model.store(Some(model.clone()));
    Ok(model)
}

/// Positives are the Raindrop bookmarks and clicked feed rows. Negatives are
/// dismissed and skipped rows, plus a background sample of older crawled
/// articles that stands in for "a typical front page story" until there's
/// enough feedback to go on.
async fn load_examples(ctx: &App) -> Result<Vec<Example>, eyre::Error> {
    let mut conn = ctx.diesel.get().await?;

    let history: Vec<HistoryRow> = diesel::sql_query(
        r#"
        SELECT oa.title, oa.url, oa.recommender_terms, COALESCE(uh.weight, 0.1)::FLOAT8 AS weight
        FROM user_history uh
        JOIN online_articles oa ON oa.id = uh.online_article_id
        "#,
    )
    .load(&mut conn)
    .await?;

    let feedback: Vec<FeedbackRow> = diesel::sql_query(format!(
        r#"
        SELECT
            oa.title,
            oa.url,
            oa.recommender_terms,
            {sources} AS sources,
            f.clicked_at IS NOT NULL AS clicked,
            f.dismissed_at IS NOT NULL AS dismissed
        FROM recommender_feedback f
        JOIN online_articles oa ON oa.id = f.online_article_id
        WHERE NOT EXISTS (SELECT 1 FROM user_history uh WHERE uh.online_article_id = oa.id)
            AND (f.clicked_at IS NOT NULL OR f.dismissed_at IS NOT NULL OR f.impressions >= $1)
        "#,
        sources = sources_json_sql("oa.id"),
    ))
    .bind::<Integer, _>(SKIPPED_AFTER_IMPRESSIONS)
    .load(&mut conn)
    .await?;

    // Hashing the id keeps the sample stable between retrains, and leaving out
    // the last few days keeps today's candidates out of their own negatives.
    let background: Vec<BackgroundRow> = if history.is_empty() {
        Vec::new()
    } else {
        let sample_size = (history.len() * BACKGROUND_PER_HISTORY_ITEM)
            .clamp(MIN_BACKGROUND_SAMPLE, MAX_BACKGROUND_SAMPLE);

        diesel::sql_query(
            r#"
            SELECT oa.title, oa.url, oa.recommender_terms
            FROM online_articles oa
            WHERE oa.created_at < NOW() - INTERVAL '3 days'
                AND NOT EXISTS (SELECT 1 FROM user_history uh WHERE uh.online_article_id = oa.id)
                AND NOT EXISTS (SELECT 1 FROM recommender_feedback f WHERE f.online_article_id = oa.id)
            ORDER BY MD5(oa.id::TEXT)
            LIMIT $1
            "#,
        )
        .bind::<BigInt, _>(i64::try_from(sample_size)?)
        .load(&mut conn)
        .await?
    };

    drop(conn);

    let mut examples = Vec::with_capacity(history.len() + feedback.len() + background.len());

    let history_weight = history
        .iter()
        .map(|row| row.weight * HISTORY_WEIGHT_SCALE)
        .sum::<f64>();
    examples.extend(history.into_iter().map(|row| Example {
        features: model::content_features(
            &row.url,
            &model::article_terms(&row.title, row.recommender_terms.as_ref()),
        ),
        label: true,
        weight: row.weight * HISTORY_WEIGHT_SCALE,
    }));

    // The background sample as a whole weighs as much as the history, so a
    // prediction of 0.5 means "as likely a bookmark as a random story".
    let background_weight = history_weight / background.len().max(1) as f64;
    examples.extend(background.into_iter().map(|row| Example {
        features: model::content_features(
            &row.url,
            &model::article_terms(&row.title, row.recommender_terms.as_ref()),
        ),
        label: false,
        weight: background_weight,
    }));

    examples.extend(feedback.into_iter().map(|row| {
        let (label, weight) = if row.dismissed {
            (false, DISMISS_WEIGHT)
        } else if row.clicked {
            (true, CLICK_WEIGHT)
        } else {
            (false, SKIP_WEIGHT)
        };

        Example {
            features: model::feed_features(
                &row.url,
                &model::article_terms(&row.title, row.recommender_terms.as_ref()),
                &parse_sources(row.sources),
            ),
            label,
            weight,
        }
    }));

    Ok(examples)
}
