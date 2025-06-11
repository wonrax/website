use crate::App;
use axum::Json;
use axum::{extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use std::time::Duration;

// Cache duration for highlights and RSS feed (5 minutes)
const CACHE_DURATION: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Deserialize)]
struct RaindropHighlight {
    #[serde(rename = "_id")]
    id: String,
    title: String,
    text: String,
    note: String, // Always a string (empty string if no note)
    #[serde(default = "default_color")]
    color: String, // Default to yellow if missing
    #[serde(rename = "created")]
    created_at: String,
    link: String,
    tags: Vec<String>,
    #[serde(rename = "raindropRef")]
    raindrop_ref: u64,
}

fn default_color() -> String {
    "yellow".to_string()
}

#[derive(Debug, Deserialize)]
struct RaindropHighlightsResponse {
    result: bool,
    items: Vec<RaindropHighlight>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HighlightItem {
    pub id: String,
    pub title: String,
    pub text: String,
    pub note: Option<String>,
    pub color: String,
    pub created_at: String,
    pub link: String,
    pub tags: Vec<String>,
}

pub async fn get_highlights(State(app): State<App>) -> impl IntoResponse {
    let cache_key = "highlights";

    // Check if we have cached data
    if let Some(cached_data) = app
        .great_reads_cache
        .get(&cache_key.to_string())
        .await
    {
        return Json(
            serde_json::from_slice::<Vec<HighlightItem>>(&cached_data).unwrap_or_default(),
        )
        .into_response();
    }

    tracing::info!("Cache miss for highlights, fetching from Raindrop API");

    let raindrop_token = match &app.config.raindrop_api_token {
        Some(token) => token,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Raindrop API token not configured",
            )
                .into_response();
        }
    };

    let collection_id = "55948413"; // Great Reads collection ID

    let mut all_highlights = Vec::new();
    let mut page = 0;
    let per_page = 50; // Raindrop API limit

    loop {
        let url = format!(
            "https://api.raindrop.io/rest/v1/highlights/{}?page={}&perpage={}",
            collection_id, page, per_page
        );

        match app
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", raindrop_token))
            .send()
            .await
        {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!(
                            "Failed to fetch highlights from Raindrop: {}",
                            resp.status()
                        ),
                    )
                        .into_response();
                }

                match resp.json::<RaindropHighlightsResponse>().await {
                    Ok(highlights_response) => {
                        if !highlights_response.result {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Raindrop API returned error result",
                            )
                                .into_response();
                        }

                        let current_count = highlights_response.items.len();
                        all_highlights.extend(highlights_response.items);

                        // If we got fewer items than per_page, we've reached the end
                        if current_count < per_page {
                            break;
                        }

                        page += 1;
                    }
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Failed to parse highlights response: {e:?}"),
                        )
                            .into_response();
                    }
                }
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to fetch highlights: {e:?}"),
                )
                    .into_response();
            }
        }
    }

    let highlights: Vec<HighlightItem> = all_highlights
        .into_iter()
        .map(|h| HighlightItem {
            id: h.id,
            title: h.title,
            text: h.text,
            note: if h.note.is_empty() {
                None
            } else {
                Some(h.note)
            },
            color: h.color,
            created_at: h.created_at,
            link: h.link,
            tags: h.tags,
        })
        .collect();

    // Cache the result
    if let Ok(serialized) = serde_json::to_vec(&highlights) {
        app.great_reads_cache
            .insert(cache_key.to_string(), serialized, CACHE_DURATION)
            .await;
    }

    Json(highlights).into_response()
}

// Keep the old RSS proxy for backwards compatibility during migration
pub async fn proxy_rss(State(app): State<App>) -> impl IntoResponse {
    let cache_key = "rss_feed";

    // Check if we have cached data
    if let Some(cached_data) = app.great_reads_cache.get(&cache_key.to_string()).await {
        let headers = [(axum::http::header::CONTENT_TYPE, "application/xml")];
        return (
            StatusCode::OK,
            headers,
            axum::body::Bytes::from(cached_data.clone()),
        )
            .into_response();
    }

    tracing::info!("Cache miss for RSS feed, fetching from Raindrop");

    let url = "https://bg.raindrop.io/rss/public/55948413";
    match app.http.get(url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let headers = [(axum::http::header::CONTENT_TYPE, "application/xml")];
            let bytes = resp.bytes().await.unwrap_or_default();

            // Cache the result if successful
            if status.is_success() {
                app.great_reads_cache
                    .insert(cache_key.to_string(), bytes.to_vec(), CACHE_DURATION)
                    .await;
            }

            (status, headers, bytes).into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to fetch RSS feed",
        )
            .into_response(),
    }
}
