use crate::discord::{
    constants::{AGENT_SESSION_TIMEOUT, MAX_AGENT_TURNS, MESSAGE_CONTEXT_SIZE, SYSTEM_PROMPT},
    message::build_conversation_history,
    tools::{DiscordSendMessageTool, FetchPageContentTool},
};
use rig::{
    agent::Agent,
    client::CompletionClient,
    completion::{Message as RigMessage, Prompt},
    providers::openai,
};
use serde_json::json;
use serenity::all::{ChannelId, Context};
use std::{sync::Arc, time::Instant};

// Agent session for persistent multi-turn conversations
pub struct AgentSession {
    pub agent: Agent<openai::CompletionModel>,
    pub conversation_history: Vec<RigMessage>,
    pub last_activity: Instant,
}

impl AgentSession {
    pub fn new(agent: Agent<openai::CompletionModel>, initial_history: Vec<RigMessage>) -> Self {
        Self {
            agent,
            conversation_history: initial_history,
            last_activity: Instant::now(),
        }
    }

    pub fn update_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn add_messages(&mut self, messages: Vec<RigMessage>) {
        self.conversation_history.extend(messages);

        // Keep conversation history manageable - limit to 2x MESSAGE_CONTEXT_SIZE
        let max_history = MESSAGE_CONTEXT_SIZE * 2;
        if self.conversation_history.len() > max_history {
            let excess = self.conversation_history.len() - max_history;
            self.conversation_history.drain(0..excess);
        }
    }

    pub fn is_expired(&self) -> bool {
        self.last_activity.elapsed() > AGENT_SESSION_TIMEOUT
    }
}

/// Create a new agent session for a channel
pub async fn create_agent_session(
    ctx: &Context,
    channel_id: ChannelId,
    context_size: usize,
    openai_api_key: &str,
    server_config: &crate::config::ServerConfig,
) -> Result<AgentSession, eyre::Error> {
    // Create OpenAI client and build agent
    let openai_client = openai::Client::new(openai_api_key);

    // Build conversation history for context
    let history = build_conversation_history(ctx, channel_id, context_size).await?;

    // Create tools with shared context
    let ctx_arc = Arc::new(ctx.clone());
    let discord_tool = DiscordSendMessageTool {
        ctx: ctx_arc.clone(),
        channel_id,
        reply_to_message_id: None, // Will be set per interaction
    };
    let fetch_tool = FetchPageContentTool;

    // Create memory tools if Qdrant is configured
    let mut agent_builder = openai_client
        .agent("o3")
        .preamble(SYSTEM_PROMPT)
        .tool(discord_tool)
        .tool(fetch_tool);

    // Add memory tools if Qdrant configuration exists
    if let Some(qdrant_config) = &server_config.qdrant {
        let memory_config = crate::discord::tools::QdrantConfig {
            url: qdrant_config.url.clone(),
            api_key: qdrant_config.api_key.clone(),
            default_collection: qdrant_config
                .default_collection
                .clone()
                .or_else(|| Some("discord_memory".to_string())),
            channel_id: Some(channel_id.get()), // Add channel ID to config
        };

        // Create a single shared Qdrant client for this channel
        let shared_client = crate::discord::tools::QdrantSharedClient::new_shared(memory_config)
            .await
            .map_err(|e| eyre::eyre!("Failed to create shared Qdrant client: {}", e))?;

        let store_tool = crate::discord::tools::QdrantStoreTool::new_with_client(
            shared_client.clone(),
            channel_id.get(),
        );
        let find_tool = crate::discord::tools::QdrantFindTool::new_with_client(
            shared_client.clone(),
            channel_id.get(),
            Some(5),
        );
        let update_tool = crate::discord::tools::QdrantUpdateTool::new_with_client(
            shared_client,
            channel_id.get(),
        );

        agent_builder = agent_builder
            .tool(store_tool)
            .tool(find_tool)
            .tool(update_tool);

        tracing::info!(
            "Memory tools enabled for channel {} with Qdrant at {}",
            channel_id,
            qdrant_config.url
        );
    } else {
        tracing::debug!("Memory tools not configured - Qdrant settings missing");
    }

    let agent = agent_builder
        .additional_params(json!({
            "max_completion_tokens": 4096,
            "reasoning_effort": "high"
        }))
        .build();

    // Store the history in the session rather than initializing the agent with it
    tracing::debug!(
        "Creating new agent session with {} messages of context",
        history.len()
    );

    Ok(AgentSession::new(agent, history))
}

/// Helper to execute agent multi-turn reasoning and handle the response
pub async fn execute_agent_interaction(
    session: &mut AgentSession,
    messages_count: usize,
    channel_id: ChannelId,
) -> Result<(), eyre::Error> {
    if session.conversation_history.is_empty() {
        return Ok(());
    }

    match session
        .agent
        .prompt("")
        .with_history(&mut session.conversation_history)
        .multi_turn(MAX_AGENT_TURNS)
        .await
    {
        Ok(response) => {
            tracing::debug!(
                "Agent processed {} new messages for channel {}: {}",
                messages_count,
                channel_id,
                response
            );

            // Add the agent's response to the conversation history
            session
                .conversation_history
                .push(RigMessage::assistant(response));
        }
        Err(e) => {
            tracing::error!(
                "Agent error processing {} messages for channel {}: {}",
                messages_count,
                channel_id,
                e
            );
            return Err(e.into());
        }
    }

    session.update_activity();
    Ok(())
}
