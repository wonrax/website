use arc_swap::ArcSwapOption;
use axum::{
    Json, Router,
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use diesel::prelude::*;
use diesel::sql_types::{Bool, Float8, Integer, Jsonb, Nullable, Text, Timestamp};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use eyre::eyre;
use futures_util::stream::StreamExt;
use robotxt::Robots;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::atomic::AtomicBool, time::Duration};
use tokio::sync::Mutex;
use tokio::time::Instant;
use tokio_stream::wrappers::BroadcastStream;

use crate::{App, error::AppError, recommendation::crawler::MAX_CONCURRENT_FETCHES};

mod crawler;
mod feedback;
mod model;
mod training;

const MIN_CRAWL_INTERVAL: Duration = Duration::from_mins(10);
/// Every request ranks the same pool, whatever page it asks for, so pages
/// don't shift against each other.
const CANDIDATE_POOL_SIZE: i32 = 400;
/// Keeps the taste model's odds finite.
const MAX_TASTE_CONFIDENCE: f64 = 0.98;

/// Impressions a row gets before each further one starts to push it down.
const FATIGUE_FREE_IMPRESSIONS: i32 = 2;
const FATIGUE_DECAY: f64 = 0.7;
/// Rows the owner already opened sink but stay in the feed.
const READ_PENALTY: f64 = 0.2;
/// Each row already picked from a domain scales the next one's score by this.
const DOMAIN_REPEAT_DECAY: f64 = 0.75;

pub struct RecommendationSystem {
    pub site_limiter: SiteLimiter,
    pub robots_cache: Mutex<HashMap<String, Robots>>,
    pub events: tokio::sync::broadcast::Sender<FeedEvent>,
    last_crawl_time: Mutex<Option<Instant>>,
    crawl_in_progress: Mutex<bool>,
    taste_model: ArcSwapOption<model::TasteModel>,
    training: Mutex<()>,
    retrain_queued: AtomicBool,
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
            taste_model: ArcSwapOption::empty(),
            training: Mutex::new(()),
            retrain_queued: AtomicBool::new(false),
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
    #[serde(default)]
    pub tags: Vec<String>,
    pub submitter: Option<String>,
    pub comment_count: Option<i64>,
    pub discussion_url: Option<String>,
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
struct CandidateRow {
    #[diesel(sql_type = Integer)]
    id: i32,
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Text)]
    original_title: String,
    #[diesel(sql_type = Text)]
    url: String,
    #[diesel(sql_type = Timestamp)]
    created_at: chrono::NaiveDateTime,
    #[diesel(sql_type = Nullable<Timestamp>)]
    submitted_at: Option<chrono::NaiveDateTime>,
    #[diesel(sql_type = Nullable<Float8>)]
    popularity: Option<f64>,
    #[diesel(sql_type = Nullable<Float8>)]
    freshness_score: Option<f64>,
    #[diesel(sql_type = Nullable<Jsonb>)]
    sources: Option<serde_json::Value>,
    #[diesel(sql_type = Nullable<Jsonb>)]
    recommender_terms: Option<serde_json::Value>,
    #[diesel(sql_type = Integer)]
    impressions: i32,
    #[diesel(sql_type = Bool)]
    clicked: bool,
}

struct RankedItem {
    item: FeedItem,
    domain: Option<String>,
    created_at: chrono::NaiveDateTime,
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
        .route("/feed/feedback", post(feedback::record_feedback))
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
    let offset = offset.max(0);

    // How far each preset leans on taste and popularity. The taste weight is
    // the power the model's odds are raised to. A lower external k gives the
    // pool's most popular stories more of a lead over the rest.
    let (taste_weight, external_k) = match ranking {
        RankingPreset::Balanced => (1.0, 6.0),
        RankingPreset::NewerFirst => (0.5, 15.0),
        RankingPreset::TopFirst => (0.25, 1.0),
        RankingPreset::SimilarFirst => (2.0, 25.0),
    };

    // Freshness decay half-life in hours for each preset
    let freshness_half_life = match ranking {
        RankingPreset::Balanced => 24.0,
        RankingPreset::NewerFirst => 4.0,
        RankingPreset::TopFirst => 24.0,
        RankingPreset::SimilarFirst => 12.0,
    };

    // Source filter condition for feed_items
    let source_filter_sql = match source_filter {
        SourceFilter::All => String::new(),
        SourceFilter::HackerNews => {
            "AND EXISTS (SELECT 1 FROM online_article_metadata m JOIN online_article_sources s ON s.id = m.source_id WHERE m.online_article_id = i.id AND s.key = 'hacker-news')".to_string()
        }
        SourceFilter::Lobsters => {
            "AND EXISTS (SELECT 1 FROM online_article_metadata m JOIN online_article_sources s ON s.id = m.source_id WHERE m.online_article_id = i.id AND s.key = 'lobsters')".to_string()
        }
    };

    // Source filter for popularity - only count the filtered source's score
    let external_score_source_filter = match source_filter {
        SourceFilter::All => String::new(),
        SourceFilter::HackerNews => {
            "JOIN online_article_sources s ON s.id = im.source_id AND s.key = 'hacker-news'"
                .to_string()
        }
        SourceFilter::Lobsters => {
            "JOIN online_article_sources s ON s.id = im.source_id AND s.key = 'lobsters'"
                .to_string()
        }
    };

    // Candidate generation: the freshest well-scored stories that aren't
    // already bookmarked or dismissed. The taste model scores them below, and
    // pagination happens after that so later pages stay consistent.
    let sql = format!(
        r#"
        WITH feed_items AS (
            SELECT i.id, i.title AS original_title, i.url, i.created_at
            FROM online_articles i
            WHERE NOT EXISTS (SELECT 1 FROM user_history uh WHERE uh.online_article_id = i.id)
            AND NOT EXISTS (
                SELECT 1 FROM recommender_feedback f
                WHERE f.online_article_id = i.id AND f.dismissed_at IS NOT NULL
            )
            {source_filter_sql}
        ),
        -- Popularity is a submission's percentile among its source's
        -- submissions, so HN points and Lobsters scores compare. A story on
        -- both sites takes the better one.
        source_percentiles AS (
            SELECT
                im.online_article_id,
                PERCENT_RANK() OVER (
                    PARTITION BY im.source_id
                    ORDER BY COALESCE(im.external_score, 0.0)
                ) AS percentile
            FROM online_article_metadata im
            {external_score_source_filter}
        ),
        item_popularity AS (
            SELECT fi.id AS online_article_id, MAX(sp.percentile) AS popularity
            FROM feed_items fi
            JOIN source_percentiles sp ON sp.online_article_id = fi.id
            GROUP BY fi.id
        ),
        -- Freshness score: exponential decay with configurable half-life
        item_freshness AS (
            SELECT
                fi.id AS online_article_id,
                EXP(-EXTRACT(EPOCH FROM (NOW() - MIN(im.submitted_at))) / 3600.0 * LN(2) / {freshness_half_life} + 3) AS freshness_score
            FROM feed_items fi
            JOIN online_article_metadata im ON im.online_article_id = fi.id
            GROUP BY fi.id
        ),
        -- Rank by popularity across the corpus to pick the pool (higher is better)
        popularity_ranked AS (
            SELECT
                online_article_id,
                popularity,
                ROW_NUMBER() OVER (ORDER BY popularity DESC, online_article_id DESC) AS rank
            FROM item_popularity
        ),
        candidates AS (
            SELECT
                fi.id,
                fi.original_title,
                fi.url,
                fi.created_at,
                pr.popularity,
                ifr.freshness_score
            FROM feed_items fi
            LEFT JOIN popularity_ranked pr ON pr.online_article_id = fi.id
            LEFT JOIN item_freshness ifr ON ifr.online_article_id = fi.id
            ORDER BY (
                COALESCE(1.0 / ({external_k} + pr.rank), 0.0) * COALESCE(ifr.freshness_score, 0.0)
            ) DESC, fi.id DESC
            LIMIT $1
        )
        SELECT
            c.id,
            COALESCE(
                (SELECT im.metadata->>'editorialized_title'
                 FROM online_article_metadata im
                 WHERE im.online_article_id = c.id
                   AND im.metadata->>'editorialized_title' IS NOT NULL
                 ORDER BY im.submitted_at
                 LIMIT 1),
                c.original_title
            ) AS title,
            c.original_title,
            c.url,
            c.created_at,
            (SELECT MIN(im.submitted_at) FROM online_article_metadata im WHERE im.online_article_id = c.id) AS submitted_at,
            c.popularity::FLOAT8 AS popularity,
            c.freshness_score::FLOAT8 AS freshness_score,
            {sources} AS sources,
            oa.recommender_terms,
            COALESCE(f.impressions, 0) AS impressions,
            f.clicked_at IS NOT NULL AS clicked
        FROM candidates c
        JOIN online_articles oa ON oa.id = c.id
        LEFT JOIN recommender_feedback f ON f.online_article_id = c.id
        ORDER BY c.id
    "#,
        sources = sources_json_sql("c.id"),
    );

    let taste_model = training::taste_model(ctx).await?;

    let mut conn = ctx.diesel.get().await?;
    let rows: Vec<CandidateRow> = diesel::sql_query(sql)
        .bind::<Integer, _>(CANDIDATE_POOL_SIZE)
        .load(&mut conn)
        .await?;
    drop(conn);

    // Popularity counts by its rank within the pool, so its pull doesn't
    // depend on how many articles the corpus has piled up.
    let popularity_ranks =
        rank_descending(&rows.iter().map(|row| row.popularity).collect::<Vec<_>>());

    // Popularity scaled by the taste model's odds, then decayed by age and by
    // how often the owner has already seen or opened the row.
    let ranked = rows
        .into_iter()
        .zip(popularity_ranks)
        .map(|(mut row, popularity_rank)| {
            let sources = parse_sources(row.sources.take());
            let terms = model::article_terms(&row.original_title, row.recommender_terms.as_ref());
            let taste = taste_model.predict(&model::feed_features(&row.url, &terms, &sources));

            let popularity = popularity_rank.map_or(0.0, |rank| 1.0 / (external_k + rank as f64));
            let score = popularity
                * taste_factor(taste, taste_weight)
                * row.freshness_score.unwrap_or(0.0)
                * fatigue(row.impressions, row.clicked);

            RankedItem {
                domain: model::article_domain(&row.url),
                created_at: row.created_at,
                item: FeedItem {
                    id: row.id,
                    title: row.title,
                    url: row.url,
                    score,
                    similarity_score: taste,
                    submitted_at: row.submitted_at,
                    sources,
                },
            }
        })
        .collect::<Vec<_>>();

    Ok(diversify_by_domain(ranked)
        .into_iter()
        .skip(usize::try_from(offset)?)
        .take(usize::try_from(limit)?)
        .map(|ranked| ranked.item)
        .collect())
}

/// SQL for the JSON array of an article's HN and Lobsters submissions, in the
/// shape of [`SourceInfo`].
fn sources_json_sql(article_id: &str) -> String {
    format!(
        r#"(SELECT JSONB_AGG(JSONB_BUILD_OBJECT(
            'key', s.key,
            'score', im.external_score,
            'external_id', im.metadata->>'external_id',
            'tags', COALESCE(im.metadata->'tags', '[]'::JSONB),
            'submitter', im.metadata->>'submitter',
            'comment_count', (im.metadata->>'comment_count')::BIGINT,
            'discussion_url', im.metadata->>'discussion_url'
        ))
        FROM online_article_metadata im
        JOIN online_article_sources s ON s.id = im.source_id
        WHERE im.online_article_id = {article_id})"#
    )
}

fn parse_sources(value: Option<serde_json::Value>) -> Vec<SourceInfo> {
    value
        .and_then(|value| {
            serde_json::from_value(value)
                .inspect_err(|err| tracing::warn!(?err, "Failed to parse article sources"))
                .ok()
        })
        .unwrap_or_default()
}

/// 1-based rank of each value, highest first. `None`s stay unranked.
fn rank_descending(values: &[Option<f64>]) -> Vec<Option<usize>> {
    let mut order = values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| value.map(|value| (index, value)))
        .collect::<Vec<_>>();
    order.sort_by(|(_, left), (_, right)| right.total_cmp(left));

    let mut ranks = vec![None; values.len()];
    for (rank, (index, _)) in order.into_iter().enumerate() {
        if let Some(slot) = ranks.get_mut(index) {
            *slot = Some(rank + 1);
        }
    }
    ranks
}

/// The taste model's odds that the owner wants the row, raised to the
/// preset's taste weight. Unlike a rank, this keeps the model's confidence: a
/// model that can't tell the rows apart (every prediction near 0.5) leaves
/// them where popularity and age put them.
fn taste_factor(taste: Option<f64>, weight: f64) -> f64 {
    taste.map_or(1.0, |probability| {
        let probability = probability.clamp(1.0 - MAX_TASTE_CONFIDENCE, MAX_TASTE_CONFIDENCE);
        (probability / (1.0 - probability)).powf(weight)
    })
}

fn fatigue(impressions: i32, clicked: bool) -> f64 {
    if clicked {
        return READ_PENALTY;
    }

    FATIGUE_DECAY.powi((impressions - FATIGUE_FREE_IMPRESSIONS).max(0))
}

/// Orders items by score, but each time a domain is picked, its remaining
/// items lose some score, so one busy site can't fill a whole page.
fn diversify_by_domain(mut remaining: Vec<RankedItem>) -> Vec<RankedItem> {
    let mut picked_per_domain = HashMap::<String, i32>::new();
    let mut ordered = Vec::with_capacity(remaining.len());

    loop {
        let adjusted_score = |candidate: &RankedItem| {
            let repeats = candidate
                .domain
                .as_ref()
                .and_then(|domain| picked_per_domain.get(domain))
                .copied()
                .unwrap_or(0);
            candidate.item.score * DOMAIN_REPEAT_DECAY.powi(repeats)
        };

        let best = remaining
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| {
                adjusted_score(left)
                    .total_cmp(&adjusted_score(right))
                    .then_with(|| left.created_at.cmp(&right.created_at))
                    .then_with(|| left.item.id.cmp(&right.item.id))
            })
            .map(|(index, _)| index);
        let Some(best) = best else {
            break;
        };

        let mut picked = remaining.swap_remove(best);
        picked.item.score = adjusted_score(&picked);
        if let Some(domain) = &picked.domain {
            *picked_per_domain.entry(domain.clone()).or_insert(0) += 1;
        }
        ordered.push(picked);
    }

    ordered
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

        // New bookmarks, and rows that were skipped often enough since the
        // last crawl, change the training data.
        training::schedule_retrain(&ctx);

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
    let mut articles_to_backfill = HashMap::new();
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
                if crawler::needs_recommender_backfill(&item) {
                    articles_to_backfill.insert(item.id, item.clone());
                }

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

    let articles_to_backfill = articles_to_backfill.into_values().collect::<Vec<_>>();
    if !articles_to_backfill.is_empty() {
        tracing::debug!(
            "Backfilling recommender content for {} existing history articles",
            articles_to_backfill.len()
        );

        futures::stream::iter(articles_to_backfill)
            .map(|article| {
                let ctx = ctx.clone();
                async move {
                    crawler::backfill_recommender_fields(&ctx, article)
                        .await
                        .map(|_| ())
                }
            })
            .buffer_unordered(MAX_CONCURRENT_FETCHES)
            .filter_map(|result| async {
                match result {
                    Ok(ok) => Some(ok),
                    Err(err) => {
                        tracing::warn!(?err, "Failed to backfill recommender fields");
                        None
                    }
                }
            })
            .collect::<Vec<_>>()
            .await;
    }

    Ok(futures::stream::iter(new_entries)
        .map(|entry| {
            let ctx = ctx.clone();
            async move {
                let article = crawler::fetch_article(&ctx, entry.url.clone(), entry.title).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked(id: i32, domain: &str, score: f64) -> RankedItem {
        RankedItem {
            item: FeedItem {
                id,
                title: String::new(),
                url: format!("https://{domain}/{id}"),
                score,
                similarity_score: None,
                submitted_at: None,
                sources: Vec::new(),
            },
            domain: Some(domain.to_string()),
            created_at: chrono::NaiveDateTime::default(),
        }
    }

    #[test]
    fn ranks_highest_first_and_skips_missing() {
        assert_eq!(
            rank_descending(&[Some(0.2), None, Some(0.9), Some(0.5)]),
            vec![Some(3), None, Some(1), Some(2)]
        );
    }

    #[test]
    fn taste_factor_follows_the_models_confidence() {
        assert_eq!(taste_factor(None, 2.0), 1.0);
        assert!((taste_factor(Some(0.5), 2.0) - 1.0).abs() < 1e-12);
        assert!(taste_factor(Some(0.9), 1.0) > taste_factor(Some(0.6), 1.0));
        assert!(taste_factor(Some(0.9), 2.0) > taste_factor(Some(0.9), 1.0));
        assert!(taste_factor(Some(0.1), 1.0) < 1.0);
        assert!(taste_factor(Some(1.0), 2.0).is_finite());
        assert!(taste_factor(Some(0.0), 2.0) > 0.0);
    }

    #[test]
    fn fatigue_spares_the_first_impressions() {
        assert_eq!(fatigue(0, false), 1.0);
        assert_eq!(fatigue(FATIGUE_FREE_IMPRESSIONS, false), 1.0);
        assert!(fatigue(FATIGUE_FREE_IMPRESSIONS + 1, false) < 1.0);
        assert_eq!(fatigue(0, true), READ_PENALTY);
    }

    #[test]
    fn spreads_out_a_dominant_domain() {
        let ordered = diversify_by_domain(vec![
            ranked(1, "github.com", 1.0),
            ranked(2, "github.com", 0.95),
            ranked(3, "github.com", 0.9),
            ranked(4, "blog.dev", 0.8),
        ]);

        let ids = ordered
            .iter()
            .map(|ranked| ranked.item.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![1, 4, 2, 3]);
        assert!(
            ordered
                .windows(2)
                .all(|pair| matches!(pair, [left, right] if left.item.score >= right.item.score))
        );
    }
}
