use axum::{
    Json, Router,
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
};
use diesel::prelude::*;
use diesel::sql_types::{Float8, Integer, Jsonb, Nullable, Text, Timestamp};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use eyre::eyre;
use futures_util::stream::StreamExt;
use robotxt::Robots;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};
use tokio::sync::Mutex;
use tokio::time::Instant;
use tokio_stream::wrappers::BroadcastStream;

use crate::{App, error::AppError, recommendation::crawler::MAX_CONCURRENT_FETCHES};

mod crawler;
mod engine;

const MIN_CRAWL_INTERVAL: Duration = Duration::from_mins(10);

pub struct RecommendationSystem {
    pub site_limiter: SiteLimiter,
    pub robots_cache: Mutex<HashMap<String, Robots>>,
    pub events: tokio::sync::broadcast::Sender<FeedEvent>,
    last_crawl_time: Mutex<Option<Instant>>,
    crawl_in_progress: Mutex<bool>,
}

impl RecommendationSystem {
    pub fn new() -> Self {
        let (events, _) = tokio::sync::broadcast::channel(256);
        Self {
            site_limiter: SiteLimiter::new(),
            robots_cache: Mutex::new(HashMap::new()),
            events,
            last_crawl_time: Mutex::new(None),
            crawl_in_progress: Mutex::new(false),
        }
    }
}

impl Default for RecommendationSystem {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SiteLimiter {
    next_allowed: Mutex<HashMap<String, Instant>>,
}

impl SiteLimiter {
    fn new() -> Self {
        Self {
            next_allowed: Mutex::new(HashMap::new()),
        }
    }

    pub async fn wait(&self, domain: &str, delay: Duration) {
        loop {
            let sleep_for = {
                let mut guard = self.next_allowed.lock().await;
                let now = Instant::now();
                match guard.get(domain) {
                    Some(next) if *next > now => Some(*next - now),
                    _ => {
                        guard.insert(domain.to_string(), now + delay);
                        None
                    }
                }
            };

            match sleep_for {
                Some(duration) => tokio::time::sleep(duration).await,
                None => break,
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SourceInfo {
    pub key: String,
    pub score: Option<f64>,
    pub external_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedItem {
    pub id: i32,
    pub title: String,
    pub url: String,
    pub score: f64,
    pub similarity_score: Option<f64>,
    pub submitted_at: Option<chrono::NaiveDateTime>,
    pub sources: Vec<SourceInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedSnapshot {
    pub items: Vec<FeedItem>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RankingPreset {
    #[default]
    Balanced,
    NewerFirst,
    TopFirst,
    SimilarFirst,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceFilter {
    #[default]
    All,
    HackerNews,
    Lobsters,
}

#[derive(Deserialize)]
pub struct FeedQuery {
    offset: Option<i64>,
    limit: Option<u32>,
    #[serde(default)]
    source: SourceFilter,
    #[serde(default)]
    ranking: RankingPreset,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum FeedEvent {
    NewEntries { count: usize },
}

#[derive(QueryableByName, Debug)]
struct RankedRow {
    #[diesel(sql_type = Integer)]
    id: i32,
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Text)]
    url: String,
    #[diesel(sql_type = Timestamp)]
    submitted_at: chrono::NaiveDateTime,
    #[diesel(sql_type = Float8)]
    score: f64,
    #[diesel(sql_type = Nullable<Float8>)]
    similarity_score: Option<f64>,
    #[diesel(sql_type = Nullable<Jsonb>)]
    sources: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserHistorySource {
    pub title: Option<String>,
    pub url: url::Url,
    pub weight: Option<f64>,
}

pub fn route() -> Router<App> {
    Router::<App>::new()
        .route("/feed", get(get_feed_snapshot))
        .route("/feed/stream", get(get_feed_stream))
}

pub fn start_background_crawl(ctx: App) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_hours(8));
        loop {
            interval.tick().await;
            if let Err(err) = run_crawl_and_notify(ctx.clone()).await {
                tracing::warn!(?err, "recommendation crawl failed");
            }
        }
    });
}

async fn get_feed_snapshot(
    State(ctx): State<App>,
    Query(query): Query<FeedQuery>,
) -> Result<Json<FeedSnapshot>, AppError> {
    let limit = query.limit.unwrap_or(20).min(100) as i64;
    let offset = query.offset.unwrap_or(0);

    let crawl_ctx = ctx.clone();
    tokio::spawn(async move {
        if let Err(err) = run_crawl_and_notify(crawl_ctx).await {
            tracing::warn!(?err, "recommendation crawl failed");
        }
    });

    let items = fetch_feed_items(&ctx, limit, offset, query.source, query.ranking).await?;

    let snapshot = FeedSnapshot { items };

    Ok(Json(snapshot))
}

async fn get_feed_stream(
    State(ctx): State<App>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>>, AppError>
{
    let stream = BroadcastStream::new(ctx.recommendation.events.subscribe())
        .filter_map(|event| async move { event.ok() })
        .map(|event| {
            let json = serde_json::to_string(&event).unwrap_or_default();
            Ok(Event::default().data(json))
        });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

async fn fetch_feed_items(
    ctx: &App,
    limit: i64,
    offset: i64,
    source_filter: SourceFilter,
    ranking: RankingPreset,
) -> Result<Vec<FeedItem>, eyre::Error> {
    let mut conn = ctx.diesel.get().await?;

    // RRF k constants for each ranking preset
    // Lower k = more weight given to top-ranked items for that signal
    let (similarity_k, external_k, freshness_k) = match ranking {
        RankingPreset::Balanced => (8.0, 2.0, 3.0),
        RankingPreset::NewerFirst => (20.0, 15.0, 1.0),
        RankingPreset::TopFirst => (20.0, 1.0, 10.0),
        RankingPreset::SimilarFirst => (1.0, 15.0, 15.0),
    };

    // Source filter condition for SQL
    let source_filter_sql = match source_filter {
        SourceFilter::All => String::new(),
        SourceFilter::HackerNews => {
            "AND EXISTS (SELECT 1 FROM online_article_metadata m JOIN online_article_sources s ON s.id = m.source_id WHERE m.online_article_id = i.id AND s.key = 'hacker-news')".to_string()
        }
        SourceFilter::Lobsters => {
            "AND EXISTS (SELECT 1 FROM online_article_metadata m JOIN online_article_sources s ON s.id = m.source_id WHERE m.online_article_id = i.id AND s.key = 'lobsters')".to_string()
        }
    };

    // Reciprocal Rank Fusion (RRF) combines multiple ranking signals by converting each
    // to a rank-based score: 1/(k + rank). This normalizes different scales and reduces
    // the impact of outliers. Each signal has its own k constant for tuning.
    //
    // Signals:
    // - Similarity: vector similarity to user history (weighted by history weight)
    // - External score: ln(hn_points + 1) + ln(lobsters_points + 1) to dampen outliers
    // - Freshness: ln(1 + 1/hours_old) based on earliest submitted_at across sources
    //
    // For each history chunk, we find the nearest feed chunks using HNSW index.
    // This direction allows PostgreSQL to use the index on online_article_chunks.embedding.
    let sql = format!(
        r#"
        WITH history_articles AS (
            SELECT online_article_id, COALESCE(weight, 0.1) AS weight
            FROM user_history
        ),
        feed_items AS (
            SELECT i.id, i.title AS original_title, i.url, i.created_at
            FROM online_articles i
            WHERE NOT EXISTS (SELECT 1 FROM user_history uh WHERE uh.online_article_id = i.id)
            {source_filter_sql}
        ),
        -- For each history chunk, find nearest chunks using HNSW index (no filter inside LATERAL
        -- so the index can be used), then filter to feed_items afterward
        nearest_chunks AS (
            SELECT
                hc.online_article_id AS history_article_id,
                nearest.online_article_id,
                nearest.dist,
                ha.weight
            FROM history_articles ha
            JOIN online_article_chunks hc ON hc.online_article_id = ha.online_article_id
            CROSS JOIN LATERAL (
                SELECT
                    fc.online_article_id,
                    fc.embedding <=> hc.embedding AS dist
                FROM online_article_chunks fc
                WHERE fc.online_article_id != hc.online_article_id
                ORDER BY fc.embedding <=> hc.embedding
                LIMIT 100
            ) nearest
        ),
        nearest_feed AS (
            SELECT
                nc.online_article_id,
                (1 - nc.dist) * nc.weight AS weighted_similarity
            FROM nearest_chunks nc
            JOIN feed_items fi ON fi.id = nc.online_article_id
        ),
        item_similarities AS (
            SELECT
                online_article_id,
                MAX(weighted_similarity) AS similarity
            FROM nearest_feed
            GROUP BY online_article_id
        ),
        -- Aggregate external scores using log dampening: ln(hn + 1) + ln(lobsters + 1)
        item_external_scores AS (
            SELECT
                fi.id AS online_article_id,
                SUM(LN(COALESCE(im.external_score, 0.0) + 1.0)) AS log_external_score
            FROM feed_items fi
            LEFT JOIN online_article_metadata im ON im.online_article_id = fi.id
            GROUP BY fi.id
        ),
        -- Freshness score: ln(1 + 1/hours_old) based on earliest submitted_at across sources
        item_freshness AS (
            SELECT
                fi.id AS online_article_id,
                LN(1.0 + 1.0 / GREATEST(EXTRACT(EPOCH FROM (NOW() - MIN(im.submitted_at))) / 3600.0, 0.01)) AS freshness_score
            FROM feed_items fi
            JOIN online_article_metadata im ON im.online_article_id = fi.id
            GROUP BY fi.id
        ),
        -- Rank by similarity (higher is better)
        similarity_ranked AS (
            SELECT
                online_article_id,
                ROW_NUMBER() OVER (ORDER BY similarity DESC NULLS LAST) AS rank
            FROM item_similarities
        ),
        -- Rank by external score (higher is better)
        external_ranked AS (
            SELECT
                online_article_id,
                ROW_NUMBER() OVER (ORDER BY log_external_score DESC NULLS LAST) AS rank
            FROM item_external_scores
        ),
        -- Rank by freshness (higher is better)
        freshness_ranked AS (
            SELECT
                online_article_id,
                ROW_NUMBER() OVER (ORDER BY freshness_score DESC NULLS LAST) AS rank
            FROM item_freshness
        ),
        -- RRF: combine ranks with 1/(k + rank), k tuned per signal
        ranked AS (
            SELECT
                fi.id,
                fi.original_title,
                fi.url,
                fi.created_at,
                (
                    COALESCE(1.0 / ({similarity_k} + sr.rank), 0.0)
                    + COALESCE(1.0 / ({external_k} + er.rank), 0.0)
                    + COALESCE(1.0 / ({freshness_k} + fr.rank), 0.0)
                )::FLOAT8 AS score,
                ism.similarity AS similarity_score
            FROM feed_items fi
            LEFT JOIN similarity_ranked sr ON sr.online_article_id = fi.id
            LEFT JOIN external_ranked er ON er.online_article_id = fi.id
            LEFT JOIN freshness_ranked fr ON fr.online_article_id = fi.id
            LEFT JOIN item_similarities ism ON ism.online_article_id = fi.id
        ),
        -- Limit results first, then fetch expensive metadata
        limited_ranked AS (
            SELECT * FROM ranked
            ORDER BY score DESC, created_at DESC, id DESC
            LIMIT $1 OFFSET $2
        )
        SELECT
            lr.id,
            COALESCE(
                (SELECT im.metadata->>'editorialized_title'
                 FROM online_article_metadata im
                 WHERE im.online_article_id = lr.id
                   AND im.metadata->>'editorialized_title' IS NOT NULL
                 ORDER BY im.submitted_at
                 LIMIT 1),
                lr.original_title
            ) AS title,
            lr.url,
            (SELECT MIN(im.submitted_at) FROM online_article_metadata im WHERE im.online_article_id = lr.id) AS submitted_at,
            lr.score,
            lr.similarity_score,
            (SELECT JSONB_AGG(JSONB_BUILD_OBJECT(
                'key', s.key,
                'score', im.external_score,
                'external_id', im.metadata->>'external_id'
            ))
            FROM online_article_metadata im
            JOIN online_article_sources s ON s.id = im.source_id
            WHERE im.online_article_id = lr.id) AS sources
        FROM limited_ranked lr
        ORDER BY lr.score DESC, lr.created_at DESC, lr.id DESC
    "#
    );

    let rows = diesel::sql_query(sql)
        .bind::<Integer, _>(limit as i32)
        .bind::<Integer, _>(offset as i32)
        .load(&mut conn)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row: RankedRow| {
            let sources: Vec<SourceInfo> = row
                .sources
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            FeedItem {
                id: row.id,
                title: row.title,
                url: row.url,
                score: row.score,
                similarity_score: row.similarity_score,
                submitted_at: Some(row.submitted_at),
                sources,
            }
        })
        .collect())
}

async fn newest_item_id(ctx: &App) -> Result<Option<i32>, eyre::Error> {
    use crate::schema::online_articles::dsl as articles_dsl;
    let mut conn = ctx.diesel.get().await?;
    let newest = articles_dsl::online_articles
        .select(articles_dsl::id)
        .order(articles_dsl::id.desc())
        .first::<i32>(&mut conn)
        .await
        .optional()?;
    Ok(newest)
}

async fn count_new_items(ctx: &App, since_id: Option<i32>) -> Result<usize, eyre::Error> {
    use crate::schema::online_articles::dsl as articles_dsl;
    let mut conn = ctx.diesel.get().await?;
    let count = match since_id {
        Some(id) => {
            articles_dsl::online_articles
                .filter(articles_dsl::id.gt(id))
                .count()
                .get_result::<i64>(&mut conn)
                .await?
        }
        None => {
            articles_dsl::online_articles
                .count()
                .get_result::<i64>(&mut conn)
                .await?
        }
    };
    Ok(count as usize)
}

async fn run_crawl_and_notify(ctx: App) -> Result<(), eyre::Error> {
    // FIXME: possible race condition when updating in_progress outside lock,
    // consider using atomics
    {
        let mut in_progress = ctx.recommendation.crawl_in_progress.lock().await;
        if *in_progress {
            tracing::debug!("Crawl already in progress, skipping");
            return Ok(());
        }

        let last_crawl = ctx.recommendation.last_crawl_time.lock().await;
        if let Some(last) = *last_crawl
            && last.elapsed() < MIN_CRAWL_INTERVAL
        {
            tracing::debug!("Crawl ran recently, skipping");
            return Ok(());
        }

        *in_progress = true;
    }

    let result = async {
        tracing::debug!("Starting recommendation crawl");
        let newest_id = newest_item_id(&ctx).await?;

        let (history, crawl) = tokio::join!(ensure_user_history(&ctx), crawler::run_crawl(&ctx),);
        let _ = history.inspect_err(|err| {
            tracing::error!(?err, "Failed to ensure user history");
        });
        let _ = crawl.inspect_err(|err| {
            tracing::error!(?err, "Crawl failed");
        });

        let new_items = count_new_items(&ctx, newest_id).await?;
        if new_items > 0 {
            let _ = ctx
                .recommendation
                .events
                .send(FeedEvent::NewEntries { count: new_items });
        }
        Ok::<(), eyre::Error>(())
    }
    .await;

    {
        let mut in_progress = ctx.recommendation.crawl_in_progress.lock().await;
        *in_progress = false;
        let mut last_crawl = ctx.recommendation.last_crawl_time.lock().await;
        *last_crawl = Some(Instant::now());
    }

    result
}

async fn ensure_user_history(ctx: &App) -> Result<usize, eyre::Error> {
    let sources = fetch_user_history_sources(ctx).await?;
    tracing::debug!("Fetched {} user history sources", sources.len());
    if sources.is_empty() {
        tracing::warn!("No user history sources found");
        return Ok(0);
    }

    insert_user_history(ctx, sources).await.inspect(|inserted| {
        tracing::info!("Inserted {} user history entries", inserted);
    })
}

async fn fetch_user_history_sources(ctx: &App) -> Result<Vec<UserHistorySource>, eyre::Error> {
    let raindrop_token = match &ctx.config.raindrop_api_token {
        Some(token) => token,
        None => return Err(eyre!("Raindrop API token not configured")),
    };

    let mut all = Vec::new();
    for collection in ctx.config.recommender_raindrop_collections.iter() {
        let mut page = 0;
        let per_page = 50;

        loop {
            let url = format!(
                "https://api.raindrop.io/rest/v1/raindrops/{}?page={}&perpage={}",
                collection.collection_id, page, per_page
            );

            let resp = ctx
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {}", raindrop_token))
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                tracing::error!(?status, body, "Failed to fetch highlights from Raindrop",);
                break;
            }

            let highlights_response = resp.json::<RaindropHighlightsResponse>().await?;
            if !highlights_response.result {
                break;
            }

            let current_count = highlights_response.items.len();
            all.extend(
                highlights_response
                    .items
                    .into_iter()
                    .map(|entry| (entry, collection.weight))
                    .collect::<Vec<_>>(),
            );
            if current_count < per_page {
                break;
            }
            page += 1;
        }
    }

    let items: Vec<UserHistorySource> = all
        .into_iter()
        .filter_map(|(entry, weight)| match url::Url::parse(&entry.link) {
            Ok(url) => Some(UserHistorySource {
                title: entry.title,
                url,
                weight: Some(weight.into()),
            }),
            Err(err) => {
                tracing::warn!(%entry.link, ?err, "Failed to parse Raindrop highlight URL");
                None
            }
        })
        .collect();

    Ok(items)
}

async fn insert_user_history(
    ctx: &App,
    sources: Vec<UserHistorySource>,
) -> Result<usize, eyre::Error> {
    use crate::schema::online_articles::dsl as articles_dsl;
    use crate::schema::user_history::dsl as history_dsl;

    let mut new_entries: Vec<UserHistorySource> = Vec::new();
    // FIXME: N+1 query
    for source in sources {
        let url = match crawler::canonicalize_url(source.url.clone()) {
            Ok(url) => url,
            Err(err) => {
                tracing::error!(%source.url, ?err, "Failed to canonicalize user history URL");
                continue;
            }
        };

        let mut conn = ctx.diesel.get().await?;

        let existing_item = articles_dsl::online_articles
            .filter(articles_dsl::url.eq(url.as_str()))
            .first::<crate::models::recommendation::OnlineArticle>(&mut conn)
            .await
            .optional()?;

        match existing_item {
            Some(item) => {
                // if the article is already indexed, just add to history
                let existing_history = history_dsl::user_history
                    .filter(history_dsl::online_article_id.eq(item.id))
                    .first::<crate::models::recommendation::UserHistory>(&mut conn)
                    .await
                    .optional()?;
                if existing_history.is_none() {
                    diesel::insert_into(history_dsl::user_history)
                        .values(crate::models::recommendation::NewUserHistory {
                            online_article_id: item.id,
                            weight: source.weight,
                        })
                        .execute(&mut conn)
                        .await?;
                }
            }
            None => {
                new_entries.push(source);
            }
        };
    }

    Ok(futures::stream::iter(new_entries)
        .map(|entry| {
            let ctx = ctx.clone();
            async move {
                let article =
                    crawler::fetch_and_generate_embedding(&ctx, entry.url.clone(), entry.title)
                        .await?;
                let mut conn = ctx.diesel.get().await?;
                let article_id = crawler::insert_article(&mut conn, article, None)
                    .await
                    .map_err(|err| {
                        eyre::eyre!("Failed to insert article {}: {}", entry.url, err)
                    })?;

                // insert into user history
                diesel::insert_into(history_dsl::user_history)
                    .values(crate::models::recommendation::NewUserHistory {
                        online_article_id: article_id,
                        weight: entry.weight,
                    })
                    .execute(&mut conn)
                    .await?;
                Ok::<(), eyre::Error>(())
            }
        })
        .buffer_unordered(MAX_CONCURRENT_FETCHES)
        .filter_map(|result| async {
            match result {
                Ok(ok) => Some(ok),
                Err(err) => {
                    tracing::warn!(?err, "Failed to fetch and insert article");
                    None
                }
            }
        })
        .collect::<Vec<_>>()
        .await
        .len())
}

pub async fn get_or_create_source(
    conn: &mut AsyncPgConnection,
    key: &str,
    name: &str,
    base_url: Option<&str>,
) -> Result<i32, eyre::Error> {
    use crate::schema::online_article_sources::dsl as sources_dsl;

    let existing = sources_dsl::online_article_sources
        .filter(sources_dsl::key.eq(key))
        .first::<crate::models::recommendation::OnlineArticleSource>(conn)
        .await
        .optional()?;

    if let Some(source) = existing {
        return Ok(source.id);
    }

    let new_source = crate::models::recommendation::NewArticleSource {
        key: key.to_string(),
        name: name.to_string(),
        base_url: base_url.map(|s| s.to_string()),
    };

    let inserted = diesel::insert_into(sources_dsl::online_article_sources)
        .values(&new_source)
        .get_result::<crate::models::recommendation::OnlineArticleSource>(conn)
        .await?;

    Ok(inserted.id)
}

#[derive(Debug, Deserialize)]
struct RaindropEntry {
    link: String,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RaindropHighlightsResponse {
    result: bool,
    items: Vec<RaindropEntry>,
}
