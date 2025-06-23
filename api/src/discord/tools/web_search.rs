use crate::discord::constants::URL_FETCH_TIMEOUT_SECS;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct WebSearchTool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchArgs {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchOutput {
    pub content: String,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Web search error: {0}")]
pub struct WebSearchError(String);

impl Tool for WebSearchTool {
    const NAME: &'static str = "web_search";
    type Error = WebSearchError;
    type Args = WebSearchArgs;
    type Output = WebSearchOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web using DuckDuckGo and extract readable content from the search results page. Returns the search results as readable text content. Use this tool sparingly to avoid being flagged as a bot".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match perform_web_search(&args.query).await {
            Ok(content) => Ok(WebSearchOutput {
                content,
                success: true,
                error: None,
            }),
            Err(e) => Ok(WebSearchOutput {
                content: "[Failed to fetch search results]".to_string(),
                success: false,
                error: Some(e.to_string()),
            }),
        }
    }
}

/// Performs a web search using DuckDuckGo and extracts readable content
async fn perform_web_search(query: &str) -> Result<String, eyre::Error> {
    use article_scraper::{ArticleScraper, Readability};
    use reqwest::Client;
    use url::Url;

    let mut url = Url::parse("https://duckduckgo.com/html")?;
    url.query_pairs_mut().append_pair("q", query);

    let scraper = ArticleScraper::new(None).await;
    let client = Client::builder()
        .timeout(URL_FETCH_TIMEOUT_SECS)
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()?;

    let article = scraper
        .parse(&url, false, &client, None)
        .await
        .map_err(|e| eyre::eyre!("Failed to scrape search results for query '{query}': {e}"))?;

    let mut result = String::new();
    result.push_str(&format!("# Search Results for: {}\n\n", query));

    if let Some(html) = article.html {
        let content = Readability::extract(&html, None).await?;
        let cleaned_content = clean_whitespace(&content);
        result.push_str(&cleaned_content);
    }

    if result.trim() == format!("# Search Results for: {}", query).trim() {
        Ok("[No readable search results found]".to_string())
    } else {
        Ok(result)
    }
}

/// Cleans up multiple consecutive whitespaces, reducing them to single spaces
/// while preserving paragraph breaks (double newlines)
fn clean_whitespace(text: &str) -> String {
    use regex::Regex;

    // First, normalize line endings to \n
    let normalized = text.replace("\r\n", "\n").replace("\r", "\n");

    // Remove excessive whitespace around HTML-like patterns and clean up spacing
    let pre_cleaned = normalized
        .lines()
        .map(|line| line.trim()) // Trim each line
        .filter(|line| !line.is_empty()) // Remove empty lines
        .collect::<Vec<&str>>()
        .join("\n");

    // Now handle paragraph spacing - replace single newlines with spaces, preserve double newlines
    let paragraph_spaced = pre_cleaned.replace('\n', " ");

    // Replace multiple consecutive whitespace with single spaces
    let whitespace_regex = Regex::new(r"\s+").unwrap();
    let cleaned = whitespace_regex.replace_all(&paragraph_spaced, " ");

    // Add back some paragraph structure by looking for sentence endings followed by capital letters
    let sentence_regex = Regex::new(r"([.!?])\s+([A-Z])").unwrap();
    let with_paragraphs = sentence_regex.replace_all(&cleaned, "$1\n\n$2");

    // Trim leading and trailing whitespace
    with_paragraphs.trim().to_string()
}
