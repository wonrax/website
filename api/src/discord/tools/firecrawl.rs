//! Firecrawl API client shared by `web_search` and `fetch_page_content`

use eyre::WrapErr as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{fmt, time::Duration};

const SEARCH_ENDPOINT: &str = "https://api.firecrawl.dev/v2/search";
const SCRAPE_ENDPOINT: &str = "https://api.firecrawl.dev/v2/scrape";
/// Searches usually take a few seconds; this bounds the outliers
const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
/// How long Firecrawl itself may spend loading a page (its default is 60s, long for a chat bot)
const SCRAPE_PAGE_TIMEOUT_MS: u64 = 30_000;
/// Client-side cap on a scrape, past the page timeout so Firecrawl's own error is what we see
const SCRAPE_TIMEOUT: Duration = Duration::from_millis(SCRAPE_PAGE_TIMEOUT_MS + 15_000);

#[derive(Clone)]
pub struct Firecrawl {
    api_key: String,
    client: reqwest::Client,
}

impl fmt::Debug for Firecrawl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Keeps the key out of logs
        f.debug_struct("Firecrawl").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    #[serde(default)]
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ScrapedPage {
    pub title: Option<String>,
    pub markdown: String,
}

/// The common shape of Firecrawl responses; `data` differs per endpoint. Unknown fields are
/// ignored.
#[derive(Deserialize)]
struct Envelope<T> {
    #[serde(default)]
    success: bool,
    data: Option<T>,
    error: Option<String>,
    warning: Option<String>,
    #[serde(rename = "creditsUsed")]
    credits_used: Option<u64>,
}

#[derive(Default, Deserialize)]
struct SearchData {
    #[serde(default)]
    web: Vec<SearchHit>,
}

#[derive(Deserialize)]
struct ScrapeData {
    #[serde(default)]
    markdown: String,
    #[serde(default)]
    metadata: ScrapeMetadata,
    warning: Option<String>,
}

#[derive(Default, Deserialize)]
struct ScrapeMetadata {
    title: Option<StringOrList>,
    #[serde(rename = "statusCode")]
    status_code: Option<u16>,
    error: Option<String>,
}

/// Firecrawl types some metadata fields as either a string or a list of strings
#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrList {
    One(String),
    Many(Vec<String>),
}

impl StringOrList {
    fn into_first(self) -> Option<String> {
        match self {
            Self::One(value) => Some(value),
            Self::Many(values) => values.into_iter().next(),
        }
    }
}

impl Firecrawl {
    pub fn new(api_key: String) -> Result<Self, eyre::Error> {
        let client = reqwest::Client::builder()
            .build()
            .wrap_err("Failed to build the Firecrawl HTTP client")?;
        Ok(Self { api_key, client })
    }

    /// Top web results for a query: title, URL, and snippet, without the page contents
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchHit>, eyre::Error> {
        let envelope: Envelope<SearchData> = self
            .post(
                SEARCH_ENDPOINT,
                json!({ "query": query, "limit": limit }),
                SEARCH_TIMEOUT,
            )
            .await?;
        Ok(envelope.data.unwrap_or_default().web)
    }

    /// A page's main content as markdown
    pub async fn scrape(&self, url: &str) -> Result<ScrapedPage, eyre::Error> {
        let envelope: Envelope<ScrapeData> = self
            .post(
                SCRAPE_ENDPOINT,
                json!({
                    "url": url,
                    "formats": ["markdown"],
                    "onlyMainContent": true,
                    "timeout": SCRAPE_PAGE_TIMEOUT_MS,
                }),
                SCRAPE_TIMEOUT,
            )
            .await?;
        let data = envelope
            .data
            .ok_or_else(|| eyre::eyre!("Firecrawl scrape of {url} returned no data"))?;

        if let Some(warning) = &data.warning {
            tracing::warn!(warning = %warning, url, "Firecrawl scrape warning");
        }
        if let Some(status) = data.metadata.status_code
            && status >= 400
        {
            let detail = data
                .metadata
                .error
                .map(|error| format!(": {error}"))
                .unwrap_or_default();
            return Err(eyre::eyre!("{url} responded with HTTP {status}{detail}"));
        }

        Ok(ScrapedPage {
            title: data
                .metadata
                .title
                .and_then(StringOrList::into_first)
                .filter(|title| !title.trim().is_empty()),
            markdown: data.markdown,
        })
    }

    async fn post<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        body: Value,
        timeout: Duration,
    ) -> Result<Envelope<T>, eyre::Error> {
        let response = self
            .client
            .post(endpoint)
            .bearer_auth(&self.api_key)
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .wrap_err("Firecrawl request failed")?;
        let status = response.status();
        let text = response
            .text()
            .await
            .wrap_err("Failed to read the Firecrawl response")?;
        let parsed: Result<Envelope<T>, _> = serde_json::from_str(&text);

        if !status.is_success() {
            // Firecrawl errors are JSON with an `error` field; anything else (a proxy page, say)
            // is quoted as is
            let detail = parsed
                .ok()
                .and_then(|envelope| envelope.error)
                .unwrap_or_else(|| text.chars().take(200).collect());
            return Err(eyre::eyre!("Firecrawl returned {status}: {detail}"));
        }

        let envelope = parsed.wrap_err("Firecrawl returned an unexpected body")?;
        if !envelope.success {
            return Err(eyre::eyre!(
                "Firecrawl request failed: {}",
                envelope.error.as_deref().unwrap_or("no error message")
            ));
        }
        if let Some(warning) = &envelope.warning {
            tracing::warn!(warning = %warning, endpoint, "Firecrawl warning");
        }
        tracing::debug!(
            endpoint,
            credits_used = envelope.credits_used,
            "Firecrawl request completed"
        );
        Ok(envelope)
    }
}
