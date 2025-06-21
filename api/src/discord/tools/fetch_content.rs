use crate::discord::constants::URL_FETCH_TIMEOUT_SECS;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct FetchPageContentTool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchPageContentArgs {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchPageContentOutput {
    pub content: String,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Fetch page content error: {0}")]
pub struct FetchPageContentError(String);

impl Tool for FetchPageContentTool {
    const NAME: &'static str = "fetch_page_content";
    type Error = FetchPageContentError;
    type Args = FetchPageContentArgs;
    type Output = FetchPageContentOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "fetch_page_content".to_string(),
            description:
                "Fetch and parse content from a web page URL. Returns the main content as text."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch content from"
                    }
                },
                "required": ["url"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match fetch_url_content_and_parse(&args.url).await {
            Ok(content) => Ok(FetchPageContentOutput {
                content,
                success: true,
                error: None,
            }),
            Err(e) => Ok(FetchPageContentOutput {
                content: "[Failed to fetch content]".to_string(),
                success: false,
                error: Some(e.to_string()),
            }),
        }
    }
}

/// Fetches content from a URL, attempts to convert HTML to Markdown.
async fn fetch_url_content_and_parse(url_str: &str) -> Result<String, eyre::Error> {
    use article_scraper::{ArticleScraper, Readability};
    use reqwest::Client;
    use url::Url;

    let scraper = ArticleScraper::new(None).await;
    let url = Url::parse(url_str)?;
    let client = Client::builder().timeout(URL_FETCH_TIMEOUT_SECS).build()?;

    let article = scraper
        .parse(&url, false, &client, None)
        .await
        .map_err(|e| eyre::eyre!("Failed to scrape article for {url_str}: {e}"))?;

    let mut result = String::new();
    if let Some(title) = article.title {
        result.push_str(&format!("# {}\n\n", title.trim()));
    }
    if let Some(html) = article.html {
        let content = Readability::extract(&html, None).await?;
        result.push_str(&content);
    }
    if result.is_empty() {
        Ok("[No readable article content found]".to_string())
    } else {
        Ok(result)
    }
}
