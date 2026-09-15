use crate::discord::tools::firecrawl::{Firecrawl, SearchHit};
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

/// Results when the agent doesn't ask for a count
const DEFAULT_LIMIT: u32 = 5;
/// Tool results are resent with every model turn for the rest of the session, so pages stay small
const MAX_LIMIT: u32 = 10;

#[derive(Debug, Clone)]
pub struct WebSearchTool {
    pub firecrawl: Firecrawl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchArgs {
    pub query: String,

    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchOutput {
    pub success: bool,
    pub results: Vec<SearchHit>,
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
            name: Self::NAME.to_string(),
            description: "Search the web and get the top results as title, URL, and snippet. The snippets are not the pages; when a result looks relevant, read it with fetch_page_content."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query, as typed into a search engine. Under 500 characters."
                    },
                    "limit": {
                        "type": ["integer", "null"],
                        "description": "Results to return, 1-10. Default 5."
                    }
                },
                "required": ["query", "limit"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        match self.firecrawl.search(&args.query, limit).await {
            Ok(results) => Ok(WebSearchOutput {
                success: true,
                results,
                error: None,
            }),
            Err(e) => {
                tracing::error!(error = ?e, query = %args.query, "web_search failed");
                Ok(WebSearchOutput {
                    success: false,
                    results: vec![],
                    // `{:#}` keeps the cause chain, e.g. the transport error under the wrap
                    error: Some(format!("{e:#}")),
                })
            }
        }
    }
}
