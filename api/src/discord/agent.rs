use crate::discord::{
    chatgpt::{ChatgptAuth, Grant},
    constants::{
        CHATGPT_CONTEXT_WINDOW, CHATGPT_DREAMER_MODEL, CHATGPT_RESPONDER_MODEL,
        CHATGPT_WATCHER_MODEL, COMPACTION_THRESHOLD_PERCENT, GEMINI_CONTEXT_WINDOW, GEMINI_MODEL,
        MAX_AGENT_RUN_DURATION, MAX_AGENT_TURNS, MEMORY_PROMPT, RECALL_PROMPT,
        RUN_CONTEXT_LIMIT_PERCENT, SYSTEM_PROMPT, WATCHER_PROMPT,
    },
    memory::ChannelMemory,
    sandbox::ChannelSandbox,
    tools::{
        DiscordSendMessageTool, FetchChannelHistoryTool, FetchMessageTool, FetchMessageUserIdsTool,
        FetchPageContentTool, Firecrawl, MemorySearchTool, ReactToMessageTool,
        RecordObservationsTool, SandboxRunTool, SandboxViewImageTool, SandboxWriteFileTool,
        SearchChannelMessagesTool, ViewMessageAttachmentsTool, WebSearchTool,
    },
};
use eyre::Context as _;
use rig::{
    client::CompletionClient as _, completion::Message as RigMessage, message::ToolChoice,
    providers::gemini,
};
use rig_agent::{
    Agent, AgentBuilder, ModelHandle,
    agent::{
        AgentHook, CompletionCallAction, CompletionCallEvent, CompletionResponseEvent,
        HookContext, ObservationAction, WithBuilderTools,
    },
    completion::{Prompt as _, PromptError},
};
use serenity::all::{ChannelId, Context, GuildId, UserId};
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
use tracing::instrument;

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

    /// The model that rewrites the memory docs of `guild_id`
    pub fn dreamer_model(
        &self,
        guild_id: GuildId,
        grant: Option<&Grant>,
    ) -> eyre::Result<ModelHandle> {
        match self {
            Self::Gemini { api_key } => gemini_model(api_key),
            Self::Chatgpt(auth) => {
                let grant = grant.ok_or_else(|| eyre::eyre!("A ChatGPT model needs a sign-in"))?;
                auth.model(
                    grant,
                    CHATGPT_DREAMER_MODEL,
                    &format!("discord-guild-{guild_id}-dreamer"),
                )
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

/// Why a run ended without finishing
#[derive(Debug)]
pub enum RunFailure {
    /// It reached one of its limits. What it did stays in the session, so the next run can pick
    /// up from there.
    Stopped(RunLimit),
    Failed(PromptError),
}

/// A limit that stops an agentic run
#[derive(Clone, Copy, Debug)]
pub enum RunLimit {
    /// `MAX_AGENT_TURNS` model calls
    Turns,
    /// `MAX_AGENT_RUN_DURATION`
    Time,
    /// `RUN_CONTEXT_LIMIT_PERCENT` of the context window
    Context,
}

impl RunLimit {
    /// How the stopped session tells the agent
    fn note(self) -> String {
        let reason = match self {
            Self::Turns => format!("it used up its {MAX_AGENT_TURNS} model calls"),
            Self::Time => format!(
                "it ran for {} minutes, its limit",
                MAX_AGENT_RUN_DURATION.as_secs() / 60
            ),
            Self::Context => "your context window is nearly full, so the session will be \
                              summarized before your next run"
                .to_string(),
        };
        format!(
            "[Run stopped] Your run stopped here before you finished: {reason}. If someone asks \
             you to go on, pick up where you left off."
        )
    }
}

/// How `RunGuard` names the limit it stops a run at
const STOPPED_AT_TIME: &str = "run time limit";
const STOPPED_AT_CONTEXT: &str = "context window nearly full";

/// Stops an agentic run before a model call once it has run for `MAX_AGENT_RUN_DURATION`, or when
/// the last call nearly filled the context window. The run ends with the history it has made so
/// far, every tool call answered.
struct RunGuard {
    started: Instant,
    context_window: u64,
    /// Tokens the latest model call held
    context_tokens: Arc<AtomicU64>,
}

impl AgentHook for RunGuard {
    async fn on_completion_call(
        &self,
        _ctx: &HookContext,
        _event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        if self.started.elapsed() >= MAX_AGENT_RUN_DURATION {
            return CompletionCallAction::stop(STOPPED_AT_TIME);
        }
        let context_tokens = self.context_tokens.load(Ordering::Relaxed);
        if context_tokens * 100 >= self.context_window * RUN_CONTEXT_LIMIT_PERCENT {
            return CompletionCallAction::stop(STOPPED_AT_CONTEXT);
        }
        CompletionCallAction::continue_run()
    }

    async fn on_completion_response(
        &self,
        _ctx: &HookContext,
        event: CompletionResponseEvent<'_>,
    ) -> ObservationAction {
        // The ChatGPT wrapper reports an empty closing turn as zero usage
        let tokens = event.usage.input_tokens + event.usage.output_tokens;
        if tokens > 0 {
            self.context_tokens.store(tokens, Ordering::Relaxed);
        }
        ObservationAction::continue_run()
    }
}

/// Agent session for persistent multi-turn conversations
pub struct AgentSession {
    pub agent: Agent,
    pub conversation_history: Vec<RigMessage>,
    /// Tokens the biggest model call of the last run held, roughly what the next run starts from
    context_tokens: u64,
    context_window: u64,
    /// People whose profiles the preamble's memory notes hold
    seeded_people: HashSet<UserId>,
    /// People whose profiles the session holds: the seeded ones, and those noted since in
    /// messages of their own
    pub noted_people: HashSet<UserId>,
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
    pub async fn run(&mut self, turn: Turn) -> Result<String, RunFailure> {
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
        let context_tokens = Arc::new(AtomicU64::new(self.context_tokens));
        let result = match turn {
            Turn::Agentic => {
                request
                    .add_hook(RunGuard {
                        started: Instant::now(),
                        context_window: self.context_window,
                        context_tokens: context_tokens.clone(),
                    })
                    .max_turns(MAX_AGENT_TURNS)
                    .await
            }
            Turn::TextOnly => request.tool_choice(ToolChoice::None).max_turns(1).await,
        };

        let response = match result {
            Ok(response) => response,
            Err(e) => {
                let (limit, history) = match e {
                    PromptError::MaxTurnsError { chat_history, .. } => {
                        (RunLimit::Turns, *chat_history)
                    }
                    PromptError::PromptCancelled {
                        chat_history,
                        reason,
                    } if reason == STOPPED_AT_TIME => (RunLimit::Time, chat_history),
                    PromptError::PromptCancelled {
                        chat_history,
                        reason,
                    } if reason == STOPPED_AT_CONTEXT => (RunLimit::Context, chat_history),
                    e => return Err(RunFailure::Failed(self.drop_failed_run(prompt, e))),
                };
                tracing::warn!(?limit, "Agent run stopped at its limit");
                // Everything before the stop: the history it started from, then the prompt and
                // the run's own turns up to its last tool results
                self.conversation_history = history;
                self.conversation_history
                    .push(RigMessage::user(limit.note()));
                let context_tokens = context_tokens.load(Ordering::Relaxed);
                if context_tokens > 0 {
                    self.context_tokens = context_tokens;
                }
                return Err(RunFailure::Stopped(limit));
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

    /// Puts `prompt` back for a retry after the run over it failed, and hands back the error
    fn drop_failed_run(&mut self, prompt: RigMessage, error: PromptError) -> PromptError {
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
        error
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
            seeded_people: self.seeded_people.clone(),
            // Profiles noted in messages went with the old history
            noted_people: std::mem::replace(&mut self.noted_people, self.seeded_people.clone()),
        }
    }
}

/// The responder's hold on the channel's memories
pub enum Memories {
    /// The channel isn't in a server, which the memories are kept by
    Off,
    /// Reads them while the watcher records them
    Recall(ChannelMemory),
    /// Records them itself, when the channel has no watcher
    Keep(ChannelMemory),
}

impl Memories {
    /// The server of the channel
    fn guild_id(&self) -> Option<GuildId> {
        match self {
            Self::Off => None,
            Self::Recall(memory) | Self::Keep(memory) => Some(memory.guild_id),
        }
    }
}

/// The services behind the responder's optional tools, which are left out without them
#[derive(Clone, Default)]
pub struct ToolBackends {
    pub firecrawl: Option<Firecrawl>,
    pub sandbox: Option<ChannelSandbox>,
}

/// What a fresh session starts from
pub struct Seed {
    /// The channel messages right before the batch that starts it
    pub history: Vec<RigMessage>,
    /// The memory notes its preamble ends with; empty without memories
    pub notes: String,
    /// Whose profiles the notes hold
    pub people: HashSet<UserId>,
}

/// Create the session that writes the channel's replies
pub fn create_responder_session(
    discord_ctx: &Context,
    channel_id: ChannelId,
    llm: &LlmBackend,
    model: ModelHandle,
    memories: Memories,
    backends: ToolBackends,
    seed: Seed,
) -> Result<AgentSession, eyre::Error> {
    let ToolBackends { firecrawl, sandbox } = backends;
    // Create tools with shared context
    let ctx_arc = Arc::new(discord_ctx.clone());
    let bot_user_id = discord_ctx.cache.current_user().id;
    let discord_tool = DiscordSendMessageTool {
        ctx: ctx_arc.clone(),
        channel_id,
        guild_id: memories.guild_id(),
        sandbox: sandbox.clone(),
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
        Memories::Recall(_) => format!("{SYSTEM_PROMPT}\n\n{RECALL_PROMPT}\n\n{}", seed.notes),
        Memories::Keep(_) => format!("{SYSTEM_PROMPT}\n\n{MEMORY_PROMPT}\n\n{}", seed.notes),
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

    if let Some(sandbox) = sandbox {
        agent_builder = agent_builder
            .tool(SandboxRunTool {
                sandbox: sandbox.clone(),
            })
            .tool(SandboxWriteFileTool {
                sandbox: sandbox.clone(),
            })
            .tool(SandboxViewImageTool { sandbox });
    }

    match memories {
        Memories::Off => {}
        Memories::Recall(memory) => {
            agent_builder = agent_builder.tool(MemorySearchTool { memory });
        }
        Memories::Keep(memory) => {
            agent_builder = with_memory_recording(agent_builder, ctx_arc, memory, bot_user_id);
        }
    }

    let agent = agent_builder.build();

    // Store the history in the session rather than initializing the agent with it
    tracing::debug!(
        "Creating new responder session with {} messages of context",
        seed.history.len()
    );

    Ok(AgentSession {
        agent,
        conversation_history: seed.history,
        context_tokens: 0,
        context_window: llm.context_window(),
        noted_people: seed.people.clone(),
        seeded_people: seed.people,
    })
}

/// Create the session that reads every message of the channel, decides when the responder
/// speaks, and records the memories when the channel has them
pub fn create_watcher_session(
    discord_ctx: &Context,
    channel_id: ChannelId,
    llm: &LlmBackend,
    model: ModelHandle,
    memory: Option<ChannelMemory>,
    seed: Seed,
) -> AgentSession {
    let preamble = if memory.is_some() {
        format!("{WATCHER_PROMPT}\n\n{}", seed.notes)
    } else {
        WATCHER_PROMPT.to_string()
    };
    let mut agent_builder = AgentBuilder::from_model_handle(model).preamble(&preamble);
    if let Some(params) = llm.session_params(Role::Watcher, channel_id) {
        agent_builder = agent_builder.additional_params(params);
    }
    let ctx_arc = Arc::new(discord_ctx.clone());
    let bot_user_id = discord_ctx.cache.current_user().id;
    // Rereads the conversation behind the memories' `source_message_ids`
    let mut agent_builder = agent_builder.tool(FetchChannelHistoryTool {
        ctx: ctx_arc.clone(),
        channel_id,
        bot_user_id,
    });
    if let Some(memory) = memory {
        agent_builder = with_memory_recording(agent_builder, ctx_arc, memory, bot_user_id);
    }

    tracing::debug!(
        "Creating new watcher session with {} messages of context",
        seed.history.len()
    );

    AgentSession {
        agent: agent_builder.build(),
        conversation_history: seed.history,
        context_tokens: 0,
        context_window: llm.context_window(),
        noted_people: seed.people.clone(),
        seeded_people: seed.people,
    }
}

/// Registers the tools that record the channel's memories and search them
fn with_memory_recording(
    agent_builder: AgentBuilder<WithBuilderTools>,
    ctx: Arc<Context>,
    memory: ChannelMemory,
    bot_user_id: UserId,
) -> AgentBuilder<WithBuilderTools> {
    agent_builder
        .tool(RecordObservationsTool {
            ctx,
            memory: memory.clone(),
            bot_user_id,
        })
        .tool(MemorySearchTool { memory })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::Usage;
    use rig_agent::test_utils::{MockAddTool, MockCompletionModel, MockTurn};

    const WINDOW: u64 = 1000;

    fn session(turns: impl IntoIterator<Item = MockTurn>) -> AgentSession {
        let model = MockCompletionModel::from_turns(turns);
        AgentSession {
            agent: AgentBuilder::from_model_handle(ModelHandle::new(model))
                .tool(MockAddTool)
                .build(),
            conversation_history: vec![RigMessage::user("before"), RigMessage::user("go")],
            context_tokens: 0,
            context_window: WINDOW,
            seeded_people: HashSet::new(),
            noted_people: HashSet::new(),
        }
    }

    fn add(call: usize) -> MockTurn {
        MockTurn::tool_call(
            format!("call_{call}"),
            "add",
            serde_json::json!({ "x": 1, "y": 2 }),
        )
    }

    fn tool_results(history: &[RigMessage]) -> usize {
        history
            .iter()
            .filter(|message| {
                matches!(message, RigMessage::User { content } if content
                    .iter()
                    .any(|part| matches!(part, rig::message::UserContent::ToolResult(_))))
            })
            .count()
    }

    #[tokio::test]
    async fn a_run_out_of_turns_keeps_its_work() {
        let mut session = session((0..MAX_AGENT_TURNS).map(add));
        let result = session.run(Turn::Agentic).await;

        assert!(matches!(result, Err(RunFailure::Stopped(RunLimit::Turns))));
        let history = &session.conversation_history;
        assert_eq!(tool_results(history), MAX_AGENT_TURNS);
        assert!(matches!(history.last(), Some(RigMessage::User { .. })));
        assert!(matches!(history.first(), Some(RigMessage::User { .. })));
    }

    #[tokio::test]
    async fn a_run_stops_before_overflowing_its_context() {
        let full = Usage {
            input_tokens: WINDOW,
            total_tokens: WINDOW,
            ..Usage::new()
        };
        let mut session = session([add(0).with_usage(full), add(1), add(2)]);
        let result = session.run(Turn::Agentic).await;

        assert!(matches!(result, Err(RunFailure::Stopped(RunLimit::Context))));
        // The first call's tool result made it in; the call that would overflow never went out
        assert_eq!(tool_results(&session.conversation_history), 1);
        assert!(session.needs_compaction());
    }

    #[tokio::test]
    async fn a_failed_run_puts_its_prompt_back() {
        let mut session = session([MockTurn::error("provider down")]);
        let result = session.run(Turn::Agentic).await;

        assert!(matches!(result, Err(RunFailure::Failed(_))));
        assert_eq!(session.conversation_history.len(), 2);
    }
}
