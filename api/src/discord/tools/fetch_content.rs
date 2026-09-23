use crate::discord::tools::firecrawl::Firecrawl;
use rig::tool::PortableTool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

/// Characters of markdown returned per call. Tool results are resent with every model turn for
/// the rest of the session, so an unbounded page would inflate every later request. The agent
/// reads on with `start`.
const MAX_CONTENT_CHARS: usize = 20_000;

#[derive(Debug, Clone)]
pub struct FetchPageContentTool {
    pub firecrawl: Firecrawl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchPageContentArgs {
    pub url: String,

    #[serde(default)]
    pub start: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchPageContentOutput {
    pub success: bool,
    pub title: Option<String>,
    /// The page's main content as markdown, cut at MAX_CONTENT_CHARS with a marker
    pub content: String,
    pub error: Option<String>,
}

impl FetchPageContentOutput {
    fn failure(error: String) -> Self {
        Self {
            success: false,
            title: None,
            content: String::new(),
            error: Some(error),
        }
    }
}

#[derive(Debug, Error)]
#[error("Fetch page content error: {0}")]
pub struct FetchPageContentError(String);

impl PortableTool for FetchPageContentTool {
    const NAME: &'static str = "fetch_page_content";
    type Error = FetchPageContentError;
    type Args = FetchPageContentArgs;
    type Output = FetchPageContentOutput;

    fn description(&self) -> String {
        "Read a web page as markdown, main content only: a link someone posted, or a web_search result whose snippet is not enough. Long pages are cut off with a marker that says how to read on."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Absolute http(s) URL of the page."
                },
                "start": {
                    "type": ["integer", "null"],
                    "description": "Character offset to start reading from, to continue past a truncation marker. Default 0."
                }
            },
            "required": ["url", "start"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let is_web_url =
            url::Url::parse(&args.url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
        if !is_web_url {
            return Ok(FetchPageContentOutput::failure(format!(
                "url must be an absolute http(s) URL, got {:?}; retrying with the same value will not help",
                args.url
            )));
        }
        let start = args.start.unwrap_or(0);

        let page = match self.firecrawl.scrape(&args.url).await {
            Ok(page) => page,
            Err(e) => {
                tracing::error!(error = ?e, url = %args.url, "fetch_page_content failed");
                // `{:#}` keeps the cause chain, e.g. the transport error under the wrap
                return Ok(FetchPageContentOutput::failure(format!("{e:#}")));
            }
        };

        if page.markdown.trim().is_empty() {
            return Ok(FetchPageContentOutput::failure(format!(
                "{} has no readable main content",
                args.url
            )));
        }

        let total = page.markdown.chars().count();
        if start >= total {
            return Ok(FetchPageContentOutput::failure(format!(
                "start {start} is past the end of the page, which has {total} characters"
            )));
        }

        Ok(FetchPageContentOutput {
            success: true,
            title: page.title,
            content: window(&page.markdown, start, total),
            error: None,
        })
    }
}

/// `MAX_CONTENT_CHARS` characters of `markdown` from `start`, plus a marker when more follows
fn window(markdown: &str, start: usize, total: usize) -> String {
    let mut content: String = markdown
        .chars()
        .skip(start)
        .take(MAX_CONTENT_CHARS)
        .collect();
    let end = total.min(start + MAX_CONTENT_CHARS);
    if end < total {
        content.push_str(&format!(
            "\n\n[truncated: {} more characters; call again with start = {end} to read on]",
            total - end
        ));
    }
    content
}
