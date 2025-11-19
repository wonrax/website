use crate::discord::{
    constants::{MAX_AGENT_TURNS, MESSAGE_CONTEXT_SIZE, SYSTEM_PROMPT},
    tools::{DiscordSendMessageTool, FetchPageContentTool, WebSearchTool},
};
use rig::{
    agent::Agent,
    completion::{Message as RigMessage, Prompt},
    providers::openai,
};
use serde_json::json;
use serenity::all::{ChannelId, Context};
use std::sync::Arc;
use tracing::instrument;

use super::tools::SharedVectorClient;

/// Agent session for persistent multi-turn conversations
pub struct AgentSession {
    pub agent: Agent<openai::CompletionModel>,
    pub conversation_history: Vec<RigMessage>,
}

impl AgentSession {
    pub fn new(agent: Agent<openai::CompletionModel>, initial_history: Vec<RigMessage>) -> Self {
        Self {
            agent,
            conversation_history: initial_history,
        }
    }

    /// Add messages to the conversation history, trimming excess if needed but new messages are
    /// always kept
    pub fn add_messages(&mut self, messages: Vec<RigMessage>) {
        let max_history = (MESSAGE_CONTEXT_SIZE * 2).max(messages.len());

        self.conversation_history.extend(messages);

        // TODO: leverage prompt caching to reduce cost
        // https://platform.openai.com/docs/guides/prompt-caching
        if self.conversation_history.len() > max_history {
            let excess = self.conversation_history.len() - max_history;
            self.conversation_history.drain(0..excess);
        }
    }

    /// Execute agent multi-turn conversation
    #[instrument(skip(self))]
    pub async fn execute_agent_multi_turn(&mut self) -> Result<(), eyre::Error> {
        if self.conversation_history.is_empty() {
            return Err(eyre::eyre!("Empty conversation history"));
        }

        for i in 0..MAX_AGENT_TURNS {
            let response = self
            .agent
            .prompt(if i == 0 {
                "[SYSTEM]: New messages are added, respond appropriately. Output [END] if no further action is needed."
            } else {
                "[SYSTEM]: Continue processing the conversation. Output [END] if no further action is needed."
            })
            .with_history(&mut self.conversation_history)
            .multi_turn(MAX_AGENT_TURNS)
            .await
            .inspect_err(|_| {
                // remove all tool calls and tool results in case of this error:
                // "The following tool_call_ids did not have response messages: call_UZH253hv9o9RYVHjRxS"
                self.conversation_history.retain(|msg| match msg {
                    RigMessage::User { content } => {
                        !content.iter().any(|c| matches!(c, rig::message::UserContent::ToolResult(_)))
                    }
                    RigMessage::Assistant { content, .. } => {
                        !content.iter().any(|c| matches!(c, rig::message::AssistantContent::ToolCall(_)))
                    }
                });
            })?;

            if response.trim().ends_with("[END]") {
                break;
            }
        }

        Ok(())
    }
}

/// Create a new agent session for a channel
pub fn create_agent_session(
    discord_ctx: &Context,
    channel_id: ChannelId,
    openai_api_key: &str,
    shared_vectordb_client: Option<SharedVectorClient>,
    initial_history: Vec<RigMessage>,
) -> AgentSession {
    // Create OpenAI client and build agent
    let openai_client = openai::Client::new(openai_api_key);
    let completion_model = openai::CompletionModel::new(openai_client, "gpt-5-mini");

    // Create tools with shared context
    let ctx_arc = Arc::new(discord_ctx.clone());
    let discord_tool = DiscordSendMessageTool {
        ctx: ctx_arc.clone(),
        channel_id,
        reply_to_message_id: None, // Will be set per interaction
    };
    let fetch_tool = FetchPageContentTool;
    let web_search_tool = WebSearchTool;

    // Godbolt tools
    let gb_compile = crate::discord::tools::Godbolt;
    let gb_langs = crate::discord::tools::GodboltLanguages;
    let gb_compilers = crate::discord::tools::GodboltCompilers;
    let gb_libs = crate::discord::tools::GodboltLibraries;
    let gb_formats = crate::discord::tools::GodboltFormats;
    let gb_format = crate::discord::tools::GodboltFormat;
    let gb_asm = crate::discord::tools::GodboltAsmDoc;
    let gb_ver = crate::discord::tools::GodboltVersion;

    // Create memory tools if Qdrant is configured
    let mut agent_builder = completion_model
        .into_agent_builder()
        .preamble(SYSTEM_PROMPT)
        .tool(discord_tool)
        .tool(fetch_tool)
        .tool(web_search_tool)
        .tool(gb_compile)
        .tool(gb_langs)
        .tool(gb_compilers)
        .tool(gb_libs)
        .tool(gb_formats)
        .tool(gb_format)
        .tool(gb_asm)
        .tool(gb_ver);

    if let Some(shared_vectordb_client) = shared_vectordb_client {
        let store_tool = crate::discord::tools::MemoryStoreTool::new_with_client(
            shared_vectordb_client.clone(),
            channel_id.get(),
        );
        let find_tool = crate::discord::tools::MemoryFindTool::new_with_client(
            shared_vectordb_client.clone(),
            channel_id.get(),
            None,
        );
        let update_tool = crate::discord::tools::MemoryUpdateTool::new_with_client(
            shared_vectordb_client.clone(),
            channel_id.get(),
        );
        let delete_tool = crate::discord::tools::MemoryDeleteTool::new_with_client(
            shared_vectordb_client,
            channel_id.get(),
        );

        agent_builder = agent_builder
            .tool(store_tool)
            .tool(find_tool)
            .tool(update_tool)
            .tool(delete_tool);

        tracing::info!("Memory tools enabled for channel {}", channel_id,);
    };

    let agent = agent_builder
        .additional_params(json!({
            "max_completion_tokens": 4096,
            "reasoning_effort": "medium",
            "verbosity": "low"
        }))
        .build();

    // Store the history in the session rather than initializing the agent with it
    tracing::debug!(
        "Creating new agent session with {} messages of context",
        initial_history.len()
    );

    AgentSession::new(agent, initial_history)
}
