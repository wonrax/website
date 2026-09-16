use crate::discord::{
    constants::{MAX_AGENT_TURNS, MEMORY_PROMPT, SYSTEM_PROMPT},
    tools::{
        DiscordSendMessageTool, FetchChannelHistoryTool, FetchMessageTool, FetchMessageUserIdsTool,
        FetchPageContentTool, Firecrawl, SearchChannelMessagesTool, ViewMessageAttachmentsTool,
        WebSearchTool,
    },
};
use eyre::Context as _;
use rig::{
    agent::Agent,
    client::CompletionClient,
    completion::{Message as RigMessage, Prompt},
    providers::gemini::{Client, completion::CompletionModel},
};
use serenity::all::{ChannelId, Context};
use std::sync::Arc;
use tracing::instrument;

use super::tools::SharedVectorClient;

/// Agent session for persistent multi-turn conversations
pub struct AgentSession {
    pub agent: Agent<CompletionModel>,
    pub conversation_history: Vec<RigMessage>,
}

impl AgentSession {
    pub fn new(agent: Agent<CompletionModel>, initial_history: Vec<RigMessage>) -> Self {
        Self {
            agent,
            conversation_history: initial_history,
        }
    }

    /// Append messages to the conversation history. Nothing is trimmed on purpose: an
    /// append-only history keeps the prompt prefix stable, so provider prompt caching keeps
    /// hitting. The idle timeout (`AGENT_SESSION_TIMEOUT`) is what bounds the session.
    pub fn add_messages(&mut self, messages: Vec<RigMessage>) {
        self.conversation_history.extend(messages);
    }

    /// Run the agent over the conversation: the newest history entry is the prompt and
    /// everything before it is the history. rig drives the tool-call loop itself, up to
    /// `MAX_AGENT_TURNS` model calls.
    #[instrument(skip(self))]
    pub async fn execute_agent_multi_turn(&mut self) -> Result<(), eyre::Error> {
        let Some(prompt) = self.conversation_history.pop() else {
            return Err(eyre::eyre!("Empty conversation history"));
        };
        if !matches!(prompt, RigMessage::User { .. }) {
            // Nothing to respond to: the newest message is the bot's own. Happens when a
            // startup `ForceProcess` finds the channel already answered.
            self.conversation_history.push(prompt);
            tracing::debug!("Skipping agent run: newest message is not from a user");
            return Ok(());
        }

        let result = self
            .agent
            .prompt(&prompt)
            .with_history(&self.conversation_history)
            .max_turns(MAX_AGENT_TURNS)
            .extended_details()
            .await;

        let response = match result {
            Ok(response) => response,
            Err(e) => {
                self.conversation_history.push(prompt);
                // remove all tool calls and tool results in case of this error:
                // "The following tool_call_ids did not have response messages: call_UZH253hv9o9RYVHjRxS"
                self.conversation_history.retain(|msg| match msg {
                    RigMessage::System { .. } => true,
                    RigMessage::User { content } => !content
                        .iter()
                        .any(|c| matches!(c, rig::message::UserContent::ToolResult(_))),
                    RigMessage::Assistant { content, .. } => !content
                        .iter()
                        .any(|c| matches!(c, rig::message::AssistantContent::ToolCall(_))),
                });
                return Err(e.into());
            }
        };

        // As of rig 0.39, `with_history` no longer folds the run's messages back into the
        // passed history; the prompt, assistant replies, and tool calls/results come back
        // only via `extended_details`. Persist them ourselves so the next Discord message
        // can see what the agent did, including the replies it already posted.
        self.conversation_history
            .extend(response.messages.unwrap_or_else(|| vec![prompt]));

        Ok(())
    }
}

/// Create a new agent session for a channel
pub fn create_agent_session(
    discord_ctx: &Context,
    channel_id: ChannelId,
    openai_api_key: &str,
    shared_vectordb_client: Option<SharedVectorClient>,
    firecrawl: Option<Firecrawl>,
    initial_history: Vec<RigMessage>,
) -> Result<AgentSession, eyre::Error> {
    // Create Gemini client and build agent
    let llm_client = Client::new(openai_api_key).context("Failed to create Gemini client")?;

    // Create tools with shared context
    let ctx_arc = Arc::new(discord_ctx.clone());
    let bot_user_id = discord_ctx.cache.current_user().id;
    let discord_tool = DiscordSendMessageTool {
        ctx: ctx_arc.clone(),
        channel_id,
    };
    let history_tool = FetchChannelHistoryTool {
        ctx: ctx_arc.clone(),
        channel_id,
        bot_user_id,
    };
    let message_tool = FetchMessageTool {
        ctx: ctx_arc.clone(),
        channel_id,
        bot_user_id,
    };
    let user_ids_tool = FetchMessageUserIdsTool {
        ctx: ctx_arc.clone(),
        channel_id,
    };
    let attachments_tool = ViewMessageAttachmentsTool {
        ctx: ctx_arc.clone(),
        channel_id,
    };
    let search_tool = SearchChannelMessagesTool::new(ctx_arc.clone(), channel_id, bot_user_id)
        .context("Failed to build the Discord search client")?;

    // Godbolt tools
    let gb_compile = crate::discord::tools::Godbolt;
    let gb_langs = crate::discord::tools::GodboltLanguages;
    let gb_compilers = crate::discord::tools::GodboltCompilers;
    let gb_libs = crate::discord::tools::GodboltLibraries;
    let gb_formats = crate::discord::tools::GodboltFormats;
    let gb_format = crate::discord::tools::GodboltFormat;
    let gb_asm = crate::discord::tools::GodboltAsmDoc;
    let gb_ver = crate::discord::tools::GodboltVersion;

    // The memory guidance only applies when the memory tools below are registered
    let preamble = if shared_vectordb_client.is_some() {
        format!("{SYSTEM_PROMPT}\n\n{MEMORY_PROMPT}")
    } else {
        SYSTEM_PROMPT.to_string()
    };

    let mut agent_builder = llm_client
        .agent("gemini-3.8-flash")
        .preamble(&preamble)
        .tool(discord_tool)
        .tool(history_tool)
        .tool(message_tool)
        .tool(user_ids_tool)
        .tool(attachments_tool)
        .tool(search_tool)
        .tool(gb_compile)
        .tool(gb_langs)
        .tool(gb_compilers)
        .tool(gb_libs)
        .tool(gb_formats)
        .tool(gb_format)
        .tool(gb_asm)
        .tool(gb_ver);

    if let Some(firecrawl) = firecrawl {
        agent_builder = agent_builder
            .tool(WebSearchTool {
                firecrawl: firecrawl.clone(),
            })
            .tool(FetchPageContentTool { firecrawl });
    }

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
        // OpenAI params
        // .additional_params(json!({
        //     "max_completion_tokens": 4096,
        //     "reasoning_effort": "medium",
        //     "verbosity": "low"
        // }))
        .build();

    // Store the history in the session rather than initializing the agent with it
    tracing::debug!(
        "Creating new agent session with {} messages of context",
        initial_history.len()
    );

    Ok(AgentSession::new(agent, initial_history))
}
