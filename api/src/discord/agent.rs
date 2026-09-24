use crate::discord::{
    chatgpt::{ChatgptAuth, Grant},
    constants::{
        CHATGPT_CONTEXT_WINDOW, CHATGPT_RESPONDER_MODEL, CHATGPT_WATCHER_MODEL,
        COMPACTION_THRESHOLD_PERCENT, GEMINI_CONTEXT_WINDOW, GEMINI_MODEL, MAX_AGENT_TURNS,
        MEMORY_PROMPT, RECALL_PROMPT, SYSTEM_PROMPT, WATCHER_PROMPT,
    },
    tools::{
        DiscordSendMessageTool, FetchChannelHistoryTool, FetchMessageTool, FetchMessageUserIdsTool,
        FetchPageContentTool, Firecrawl, MemoryDeleteTool, MemoryFindTool, MemoryStoreTool,
        MemoryUpdateTool, ReactToMessageTool, SearchChannelMessagesTool,
        ViewMessageAttachmentsTool, WebSearchTool,
    },
};
use eyre::Context as _;
use rig::{
    client::CompletionClient as _, completion::Message as RigMessage, message::ToolChoice,
    providers::gemini,
};
use rig_agent::{
    Agent, AgentBuilder, ModelHandle,
    agent::WithBuilderTools,
    completion::{Prompt as _, PromptError},
};
use serenity::all::{ChannelId, Context};
use std::sync::Arc;
use tracing::instrument;

use super::tools::SharedVectorClient;

/// The LLM the agents run on, picked at startup by `DISCORD_LLM_BACKEND`
#[derive(Clone)]
pub enum LlmBackend {
    Chatgpt(Arc<ChatgptAuth>),
    Gemini { api_key: String },
}

/// Which of a channel's two agents a model or session is for
#[derive(Clone, Copy, Debug)]
pub enum Role {
    /// Reads every batch to decide whether the bot speaks, and keeps the memories
    Watcher,
    /// Writes the bot's replies
    Responder,
}

/// Names `role`'s sessions in `channel_id` to the provider, which keeps their cached prefix under it
fn session_key(role: Role, channel_id: ChannelId) -> String {
    match role {
        Role::Watcher => format!("discord-channel-{channel_id}-watcher"),
        Role::Responder => format!("discord-channel-{channel_id}"),
    }
}

impl LlmBackend {
    /// The model `role` runs on in `channel_id`. ChatGPT authenticates it as the run's `grant`.
    pub fn model(
        &self,
        role: Role,
        channel_id: ChannelId,
        grant: Option<&Grant>,
    ) -> eyre::Result<ModelHandle> {
        match self {
            Self::Gemini { api_key } => gemini_model(api_key),
            Self::Chatgpt(auth) => {
                let grant = grant.ok_or_else(|| eyre::eyre!("A ChatGPT model needs a sign-in"))?;
                let name = match role {
                    Role::Watcher => CHATGPT_WATCHER_MODEL,
                    Role::Responder => CHATGPT_RESPONDER_MODEL,
                };
                auth.model(grant, name, &session_key(role, channel_id))
            }
        }
    }

    fn context_window(&self) -> u64 {
        match self {
            Self::Chatgpt(_) => CHATGPT_CONTEXT_WINDOW,
            Self::Gemini { .. } => GEMINI_CONTEXT_WINDOW,
        }
    }

    /// Sent to the provider verbatim with every request of `role`'s session in the channel
    fn session_params(&self, role: Role, channel_id: ChannelId) -> Option<serde_json::Value> {
        // Routes each session's requests to where its long, stable prefix is cached
        matches!(self, Self::Chatgpt(_))
            .then(|| serde_json::json!({ "prompt_cache_key": session_key(role, channel_id) }))
    }
}

pub fn gemini_model(api_key: &str) -> Result<ModelHandle, eyre::Error> {
    let client = gemini::Client::new(api_key).context("Failed to create Gemini client")?;
    Ok(ModelHandle::new(client.completion_model(GEMINI_MODEL)))
}

/// How a run may use its tools
#[derive(Clone, Copy, Debug)]
pub enum Turn {
    /// As the model sees fit, over as many model calls as that takes
    Agentic,
    /// Not at all: a single model call that answers in text. The tools stay registered, so the
    /// request keeps the prefix the provider has cached.
    TextOnly,
}

/// Agent session for persistent multi-turn conversations
pub struct AgentSession {
    pub agent: Agent,
    pub conversation_history: Vec<RigMessage>,
    /// Tokens the biggest model call of the last run held, roughly what the next run starts from
    context_tokens: u64,
    context_window: u64,
}

impl AgentSession {
    /// Append messages to the conversation history. Nothing is trimmed on purpose: an
    /// append-only history keeps the prompt prefix stable, so provider prompt caching keeps
    /// hitting. Compaction and the idle timeouts are what bound the session.
    pub fn add_messages(&mut self, messages: Vec<RigMessage>) {
        self.conversation_history.extend(messages);
    }

    /// Whether the session has filled enough of the context window to be compacted
    pub fn needs_compaction(&self) -> bool {
        self.context_tokens * 100 >= self.context_window * COMPACTION_THRESHOLD_PERCENT
    }

    /// Run the agent over the conversation: the newest history entry is the prompt and
    /// everything before it is the history. Returns the text of the final model turn.
    #[instrument(skip(self))]
    pub async fn run(&mut self, turn: Turn) -> Result<String, PromptError> {
        let Some(prompt) = self.conversation_history.pop() else {
            tracing::warn!("Skipping agent run: empty conversation history");
            return Ok(String::new());
        };
        if !matches!(prompt, RigMessage::User { .. }) {
            // Nothing to respond to: the newest message is the bot's own. Happens when a
            // startup `ForceProcess` finds the channel already answered.
            self.conversation_history.push(prompt);
            tracing::debug!("Skipping agent run: newest message is not from a user");
            return Ok(String::new());
        }

        let request = self
            .agent
            .prompt(&prompt)
            .history(self.conversation_history.clone())
            .extended_details();
        let result = match turn {
            Turn::Agentic => request.max_turns(MAX_AGENT_TURNS).await,
            Turn::TextOnly => request.tool_choice(ToolChoice::None).max_turns(1).await,
        };

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
                return Err(e);
            }
        };

        // The ChatGPT wrapper reports an empty closing turn as zero usage, so take the biggest
        // call rather than the last
        let context_tokens = response
            .completion_calls
            .iter()
            .map(|call| call.usage.input_tokens + call.usage.output_tokens)
            .max()
            .unwrap_or(0);
        if context_tokens > 0 {
            self.context_tokens = context_tokens;
        }
        tracing::info!(
            calls = response.completion_calls.len(),
            input_tokens = response.usage.input_tokens,
            cached_input_tokens = response.usage.cached_input_tokens,
            output_tokens = response.usage.output_tokens,
            context_tokens = self.context_tokens,
            "Agent run finished"
        );

        // As of rig 0.39, `with_history` no longer folds the run's messages back into the
        // passed history; the prompt, assistant replies, and tool calls/results come back
        // only via `extended_details`. Persist them ourselves so the next Discord message
        // can see what the agent did, including the replies it already posted.
        self.conversation_history
            .extend(response.messages.unwrap_or_else(|| vec![prompt]));

        Ok(response.output)
    }

    /// Starts the session over from `summary` followed by the latest channel messages, and hands
    /// back the session as it was until now
    pub fn start_over(&mut self, summary: &str, recent: Vec<RigMessage>) -> AgentSession {
        let seed = RigMessage::user(format!(
            "[Summary of the conversation before the messages below]\n{summary}"
        ));
        let history = std::iter::once(seed).chain(recent).collect();
        AgentSession {
            agent: self.agent.clone(),
            conversation_history: std::mem::replace(&mut self.conversation_history, history),
            context_tokens: std::mem::take(&mut self.context_tokens),
            context_window: self.context_window,
        }
    }
}

/// The responder's hold on the channel's memories
pub enum Memories {
    /// No vector database is configured
    Off,
    /// Reads them while the watcher keeps them
    Recall(SharedVectorClient),
    /// Keeps them itself, when the channel has no watcher
    Keep(SharedVectorClient),
}

/// Create the session that writes the channel's replies
pub fn create_responder_session(
    discord_ctx: &Context,
    channel_id: ChannelId,
    llm: &LlmBackend,
    model: ModelHandle,
    memories: Memories,
    firecrawl: Option<Firecrawl>,
    initial_history: Vec<RigMessage>,
) -> Result<AgentSession, eyre::Error> {
    // Create tools with shared context
    let ctx_arc = Arc::new(discord_ctx.clone());
    let bot_user_id = discord_ctx.cache.current_user().id;
    let discord_tool = DiscordSendMessageTool {
        ctx: ctx_arc.clone(),
        channel_id,
    };
    let reaction_tool = ReactToMessageTool {
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
    let preamble = match &memories {
        Memories::Off => SYSTEM_PROMPT.to_string(),
        Memories::Recall(_) => format!("{SYSTEM_PROMPT}\n\n{RECALL_PROMPT}"),
        Memories::Keep(_) => format!("{SYSTEM_PROMPT}\n\n{MEMORY_PROMPT}"),
    };

    let mut agent_builder = AgentBuilder::from_model_handle(model).preamble(&preamble);
    if let Some(params) = llm.session_params(Role::Responder, channel_id) {
        agent_builder = agent_builder.additional_params(params);
    }
    let mut agent_builder = agent_builder
        .tool(discord_tool)
        .tool(reaction_tool)
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

    match memories {
        Memories::Off => {}
        Memories::Recall(client) => {
            agent_builder = agent_builder.tool(MemoryFindTool::new_with_client(
                client,
                channel_id.get(),
                None,
            ));
        }
        Memories::Keep(client) => {
            agent_builder = with_memory_curation(agent_builder, client, channel_id);
            tracing::info!("Memory tools enabled for channel {}", channel_id);
        }
    }

    let agent = agent_builder.build();

    // Store the history in the session rather than initializing the agent with it
    tracing::debug!(
        "Creating new responder session with {} messages of context",
        initial_history.len()
    );

    Ok(AgentSession {
        agent,
        conversation_history: initial_history,
        context_tokens: 0,
        context_window: llm.context_window(),
    })
}

/// Create the session that reads every message of the channel, decides when the responder
/// speaks, and keeps the memories when there is a vector database to keep them in
pub fn create_watcher_session(
    discord_ctx: &Context,
    channel_id: ChannelId,
    llm: &LlmBackend,
    model: ModelHandle,
    vectordb: Option<SharedVectorClient>,
    initial_history: Vec<RigMessage>,
) -> AgentSession {
    let mut agent_builder = AgentBuilder::from_model_handle(model).preamble(WATCHER_PROMPT);
    if let Some(params) = llm.session_params(Role::Watcher, channel_id) {
        agent_builder = agent_builder.additional_params(params);
    }
    let ctx_arc = Arc::new(discord_ctx.clone());
    let mut agent_builder = agent_builder
        // Rereads the conversation behind a memory's `source_message_ids`
        .tool(FetchChannelHistoryTool {
            ctx: ctx_arc.clone(),
            channel_id,
            bot_user_id: discord_ctx.cache.current_user().id,
        })
        .tool(ReactToMessageTool {
            ctx: ctx_arc,
            channel_id,
        });
    if let Some(client) = vectordb {
        agent_builder = with_memory_curation(agent_builder, client, channel_id);
    }

    tracing::debug!(
        "Creating new watcher session with {} messages of context",
        initial_history.len()
    );

    AgentSession {
        agent: agent_builder.build(),
        conversation_history: initial_history,
        context_tokens: 0,
        context_window: llm.context_window(),
    }
}

/// Registers the tools that read and write the channel's memories
fn with_memory_curation(
    agent_builder: AgentBuilder<WithBuilderTools>,
    client: SharedVectorClient,
    channel_id: ChannelId,
) -> AgentBuilder<WithBuilderTools> {
    let channel = channel_id.get();
    agent_builder
        .tool(MemoryStoreTool::new_with_client(client.clone(), channel))
        .tool(MemoryFindTool::new_with_client(
            client.clone(),
            channel,
            None,
        ))
        .tool(MemoryUpdateTool::new_with_client(client.clone(), channel))
        .tool(MemoryDeleteTool::new_with_client(client, channel))
}
