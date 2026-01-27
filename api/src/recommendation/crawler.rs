use std::time::Duration;

use crate::App;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use eyre::{OptionExt, eyre};
use futures::stream::StreamExt;
use robotxt::Robots;
use serde::Deserialize;

use super::get_or_create_source;

async fn upsert_metadata(
    conn: &mut AsyncPgConnection,
    online_article_id: i32,
    source_id: i32,
    external_score: Option<f64>,
    metadata_json: serde_json::Value,
    submitted_at: chrono::NaiveDateTime,
) -> Result<(), diesel::result::Error> {
    use crate::schema::online_article_metadata::dsl as metadata_dsl;

    let updated = diesel::update(metadata_dsl::online_article_metadata)
        .filter(metadata_dsl::online_article_id.eq(online_article_id))
        .filter(metadata_dsl::source_id.eq(source_id))
        .set((
            metadata_dsl::external_score.eq(external_score),
            metadata_dsl::metadata.eq(&metadata_json),
            metadata_dsl::submitted_at.eq(submitted_at),
        ))
        .execute(conn)
        .await?;

    if updated == 0 {
        let new_metadata = crate::models::recommendation::NewArticleMetadata {
            online_article_id,
            source_id,
            external_score,
            metadata: Some(metadata_json),
            submitted_at,
        };
        diesel::insert_into(metadata_dsl::online_article_metadata)
            .values(&new_metadata)
            .execute(conn)
            .await?;
    }

    Ok(())
}

pub const MAX_CONCURRENT_FETCHES: usize = 4;
const ROBOTS_USER_AGENT: &str = "wrx-recommendation-bot";
const DEFAULT_CRAWL_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug)]
pub struct SourceEntry {
    pub source_id: i32,
    pub title: Option<String>,
    pub url: url::Url,
    pub external_score: Option<f64>,
    pub submitted_at: chrono::NaiveDateTime,
    pub external_id: String,
}

#[derive(Debug)]
pub struct FetchedArticle {
    url: url::Url,
    title: String,
    embeddings: Vec<pgvector::Vector>,
}

#[tracing::instrument(skip(ctx))]
pub async fn run_crawl(ctx: &App) -> Result<(), eyre::Error> {
    tracing::debug!("Starting crawl job");

    let mut entries = fetch_lobsters(ctx)
        .await
        .inspect_err(|err| {
            tracing::error!(?err, "Failed to fetch entries from Lobsters");
        })
        .unwrap_or_default();
    entries.extend(
        fetch_hackernews(ctx)
            .await
            .inspect_err(|err| {
                tracing::error!(?err, "Failed to fetch entries from Hacker News");
            })
            .unwrap_or_default(),
    );

    tracing::debug!("Fetched {} total entries from sources", entries.len());

    // First pass: filter out already-existing URLs
    let mut conn = ctx.diesel.get().await?;
    let mut new_entries = Vec::new();
    // FIXME: N+1 query
    for entry in entries {
        let url = match canonicalize_url(entry.url.clone()) {
            Ok(u) => u,
            Err(err) => {
                tracing::warn!(url = %entry.url, ?err, "Failed to canonicalize URL");
                continue;
            }
        };

        use crate::schema::online_articles::dsl as online_articles_dsl;
        let existing = online_articles_dsl::online_articles
            .filter(online_articles_dsl::url.eq(url.as_str()))
            .first::<crate::models::recommendation::OnlineArticle>(&mut conn)
            .await
            .optional()?;

        if let Some(existing) = existing {
            // Update metadata for existing item (score, editorialized title, external_id, submitted_at)
            let metadata_json = serde_json::json!({
                "editorialized_title": entry.title,
                "external_id": entry.external_id,
            });
            upsert_metadata(
                &mut conn,
                existing.id,
                entry.source_id,
                entry.external_score,
                metadata_json,
                entry.submitted_at,
            )
            .await?;
        } else {
            new_entries.push(SourceEntry { url, ..entry });
        }
    }
    // Release connection before HTTP fetches
    drop(conn);

    if new_entries.is_empty() {
        tracing::debug!("No new entries to process");
        return Ok(());
    }

    tracing::debug!("Processing {} new entries", new_entries.len());

    futures::stream::iter(new_entries)
        .map(|entry| {
            let ctx = ctx.clone();
            async move {
                let article =
                    fetch_and_generate_embedding(&ctx, entry.url.clone(), entry.title.clone())
                        .await?;
                let mut conn = ctx.diesel.get().await?;
                insert_article(&mut conn, article, Some(&entry)).await
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
        .await;

    Ok(())
}

#[tracing::instrument(skip(ctx, url))]
pub async fn fetch_and_generate_embedding(
    ctx: &App,
    url: url::Url,
    title: Option<String>,
) -> Result<FetchedArticle, eyre::Error> {
    let (real_title, markdown) = fetch_markdown(ctx, &url)
        .await
        .inspect_err(|err| {
            tracing::warn!(
                url = %url,
                ?err,
                "Failed to fetch and parse article, falling back to using title only"
            );
        })
        .unwrap_or_else(|_| (title.clone(), title.clone().unwrap_or_default()));

    let title = real_title
        .or(title)
        .ok_or_eyre("couldn't extract title from the article, maybe manually supply one")?;

    let embeddings = super::engine::generate_embeddings(&title, &markdown).await?;

    Ok(FetchedArticle {
        url,
        title,
        embeddings,
    })
}

#[tracing::instrument(skip_all)]
pub async fn insert_article(
    conn: &mut diesel_async::AsyncPgConnection,
    article: FetchedArticle,
    source_entry: Option<&SourceEntry>,
) -> Result<i32, eyre::Error> {
    use crate::schema::online_article_chunks::dsl as chunks_dsl;
    use crate::schema::online_article_metadata::dsl as metadata_dsl;
    use crate::schema::online_articles::dsl as articles_dsl;
    use diesel_async::AsyncConnection;

    let canonical_url = canonicalize_url(article.url.clone())?;

    Ok(conn
        .transaction(|conn| {
            Box::pin(async move {
                let new_item = crate::models::recommendation::NewOnlineArticle {
                    url: canonical_url.to_string(),
                    title: article.title,
                    // useful when debugging though takes up space, so optional
                    content_text: None,
                };

                let article_id = diesel::insert_into(articles_dsl::online_articles)
                    .values(&new_item)
                    .returning(articles_dsl::id)
                    .get_result::<i32>(conn)
                    .await?;

                let chunk_rows: Vec<crate::models::recommendation::NewArticleChunk> = article
                    .embeddings
                    .iter()
                    .map(|embedding| crate::models::recommendation::NewArticleChunk {
                        online_article_id: article_id,
                        embedding: embedding.clone(),
                    })
                    .collect();

                diesel::insert_into(chunks_dsl::online_article_chunks)
                    .values(&chunk_rows)
                    .execute(conn)
                    .await?;

                if let Some(source_entry) = source_entry {
                    let metadata_json = serde_json::json!({
                        "editorialized_title": source_entry.title,
                        "external_id": source_entry.external_id,
                    });
                    let new_metadata = crate::models::recommendation::NewArticleMetadata {
                        online_article_id: article_id,
                        source_id: source_entry.source_id,
                        external_score: source_entry.external_score,
                        metadata: Some(metadata_json),
                        submitted_at: source_entry.submitted_at,
                    };
                    diesel::insert_into(metadata_dsl::online_article_metadata)
                        .values(&new_metadata)
                        .execute(conn)
                        .await?;
                }

                Ok::<_, diesel::result::Error>(article_id)
            })
        })
        .await?)
}

pub fn canonicalize_url(mut url: url::Url) -> Result<url::Url, eyre::Error> {
    url.set_fragment(None);
    if url.path().ends_with('/') && url.path() != "/" {
        let trimmed = url.path().trim_end_matches('/').to_string();
        url.set_path(&trimmed);
    }
    Ok(url)
}

async fn get_robots_info(ctx: &App, url: &url::Url) -> Result<Robots, eyre::Error> {
    let host = url
        .host_str()
        .ok_or_else(|| eyre!("missing host"))?
        .to_string();

    {
        let cache = ctx.recommendation.robots_cache.lock().await;
        if let Some(info) = cache.get(&host).cloned() {
            return Ok(info);
        }
    }

    ctx.recommendation
        .site_limiter
        .wait(&host, DEFAULT_CRAWL_DELAY)
        .await;

    let base = url::Url::parse(&format!("{}://{}/", url.scheme(), host))?;
    let robots_url = robotxt::create_url(&base).map_err(|err| eyre!(err))?;
    let body = match ctx.http.get(robots_url).send().await {
        Ok(resp) => resp.text().await.unwrap_or_default(),
        Err(_) => String::new(),
    };

    let robots = if body.is_empty() {
        Robots::from_always(true, ROBOTS_USER_AGENT)
    } else {
        Robots::from_bytes(body.as_bytes(), ROBOTS_USER_AGENT)
    };

    // FIXME: use retainer cache with expiration
    {
        let mut cache = ctx.recommendation.robots_cache.lock().await;
        cache.insert(host, robots.clone());
    }

    Ok(robots)
}

async fn fetch_markdown(
    ctx: &App,
    url: &url::Url,
) -> Result<(Option<String>, String), eyre::Error> {
    let domain = url.host_str().ok_or_else(|| eyre!("missing host"))?;

    let robots = get_robots_info(ctx, url).await?;
    if !robots.is_absolute_allowed(url) {
        return Err(eyre!("robots.txt disallows crawling this URL"));
    }

    ctx.recommendation
        .site_limiter
        .wait(domain, robots.crawl_delay().unwrap_or(DEFAULT_CRAWL_DELAY))
        .await;

    let article = article_scraper::ArticleScraper::new(None)
        .await
        .parse(url, false, &ctx.http, None)
        .await?;

    let markdown = html_to_markdown_rs::convert(
        article
            .html
            .as_ref()
            .ok_or_else(|| eyre!("no html content found"))?,
        None,
    )?;

    Ok((article.title, markdown))
}

async fn fetch_lobsters(ctx: &App) -> Result<Vec<SourceEntry>, eyre::Error> {
    #[derive(Deserialize)]
    struct LobstersEntry {
        short_id: String,
        title: String,
        url: String,
        score: i64,
        created_at: String,
    }

    let conn = &mut ctx.diesel.get().await?;
    let lobsters_source_id =
        get_or_create_source(conn, "lobsters", "Lobsters", Some("https://lobste.rs/")).await?;
    let url = "https://lobste.rs/hottest.json";

    let mut entries = Vec::new();
    for page in 1..=2 {
        let response = ctx.http.get(format!("{url}/?page={page}")).send().await?;
        let resp: Vec<LobstersEntry> = response.json().await?;

        let new_entries = resp
            .into_iter()
            .filter_map(|entry| {
                let url = url::Url::parse(&entry.url).inspect_err(|err| {
                tracing::warn!(url = %entry.url, ?err, "Failed to parse URL from Lobsters entry")
            }).ok()?;

                let submitted_at = chrono::DateTime::parse_from_rfc3339(&entry.created_at)
                    .inspect_err(|err| {
                        tracing::warn!(created_at = %entry.created_at, ?err, "Failed to parse created_at from Lobsters entry")
                    })
                    .ok()?
                    .naive_utc();

                url.scheme().starts_with("http").then_some(SourceEntry {
                    source_id: lobsters_source_id,
                    title: Some(entry.title),
                    url,
                    external_score: Some(entry.score as f64),
                    submitted_at,
                    external_id: entry.short_id,
                })
            })
            .collect::<Vec<_>>();

        entries.extend(new_entries);
    }

    Ok(entries)
}

async fn fetch_hackernews(ctx: &App) -> Result<Vec<SourceEntry>, eyre::Error> {
    #[derive(Deserialize)]
    struct HNItem {
        title: String,
        url: Option<String>,
        score: i64,
        r#type: String,
        time: i64,
    }

    let conn = &mut ctx.diesel.get().await?;
    let hn_source_id = get_or_create_source(
        conn,
        "hacker-news",
        "Hacker News",
        Some("https://news.ycombinator.com/"),
    )
    .await?;

    let top_stories_resp = ctx
        .http
        .get("https://hacker-news.firebaseio.com/v0/topstories.json")
        .send()
        .await?;
    let top_story_ids: Vec<i64> = top_stories_resp.json().await?;
    let mut entries = Vec::new();
    for story_id in top_story_ids.into_iter().take(64) {
        let item_resp = ctx
            .http
            .get(format!(
                "https://hacker-news.firebaseio.com/v0/item/{}.json",
                story_id
            ))
            .send()
            .await?;
        let item: HNItem = item_resp.json().await?;

        if item.r#type != "story" {
            continue;
        }

        if let Some(url_str) = item.url {
            let url = url::Url::parse(&url_str).inspect_err(|err| {
                tracing::warn!(url = %url_str, ?err, "Failed to parse URL from Hacker News item")
            }).ok();

            let submitted_at =
                chrono::DateTime::from_timestamp(item.time, 0).map(|dt| dt.naive_utc());

            if let Some(url) = url
                && url.scheme().starts_with("http")
                && let Some(submitted_at) = submitted_at
            {
                entries.push(SourceEntry {
                    source_id: hn_source_id,
                    title: Some(item.title),
                    url,
                    external_score: Some(item.score as f64),
                    submitted_at,
                    external_id: story_id.to_string(),
                });
            }
        }
    }
    Ok(entries)
}
