use crate::discord::{
    constants::{AGENT_SESSION_TIMEOUT_MINS, MAX_AGENT_TURNS, MESSAGE_CONTEXT_SIZE, SYSTEM_PROMPT},
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
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

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
        self.last_activity.elapsed() > Duration::from_secs(AGENT_SESSION_TIMEOUT_MINS * 60)
    }
}

/// Create a new agent session for a channel
pub async fn create_agent_session(
    ctx: &Context,
    channel_id: ChannelId,
    context_size: usize,
    openai_api_key: &str,
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

    let agent = openai_client
        .agent("o4-mini")
        .preamble(SYSTEM_PROMPT)
        .tool(discord_tool)
        .tool(fetch_tool)
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

    let mut history_clone = session.conversation_history.clone();
    match session
        .agent
        .prompt("")
        .with_history(&mut history_clone)
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
