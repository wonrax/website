use const_format::formatcp;
use rig::prelude::*;
use rig::{
    agent::Agent,
    completion::{Message as RigMessage, Prompt, ToolDefinition},
    message::{ImageDetail, UserContent},
    providers::openai,
    tool::Tool,
    OneOrMany,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::{ChannelId, CreateMessage, Message, Ready};
use serenity::async_trait;
use serenity::futures::StreamExt;
use serenity::prelude::*;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing;

const WHITELIST_CHANNELS: [u64; 2] = [1133997981637554188, 1119652436102086809];
const MESSAGE_CONTEXT_SIZE: usize = 20; // Number of previous messages to load for context
const MESSAGE_DEBOUNCE_TIMEOUT_MS: u64 = 5000; // 5 seconds to collect messages
const URL_FETCH_TIMEOUT_SECS: Duration = Duration::from_secs(15);
const DISCORD_BOT_NAME: &str = "The Irony Himself";
const MAX_AGENT_TURNS: usize = 10; // Maximum turns for multi-turn reasoning
const AGENT_SESSION_TIMEOUT_MINS: u64 = 30; // Reset agent after 30 minutes of inactivity

// Message queue item for debouncing
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    message: Message,
}

// Agent session for persistent multi-turn conversations
pub(crate) struct AgentSession {
    agent: Agent<openai::CompletionModel>,
    conversation_history: Vec<RigMessage>,
    last_activity: Instant,
}

impl AgentSession {
    fn new(agent: Agent<openai::CompletionModel>, initial_history: Vec<RigMessage>) -> Self {
        Self {
            agent,
            conversation_history: initial_history,
            last_activity: Instant::now(),
        }
    }

    fn update_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    fn add_messages(&mut self, messages: Vec<RigMessage>) {
        self.conversation_history.extend(messages);

        // Keep conversation history manageable - limit to 2x MESSAGE_CONTEXT_SIZE
        let max_history = MESSAGE_CONTEXT_SIZE * 2;
        if self.conversation_history.len() > max_history {
            let excess = self.conversation_history.len() - max_history;
            self.conversation_history.drain(0..excess);
        }
    }

    fn is_expired(&self) -> bool {
        self.last_activity.elapsed() > Duration::from_secs(AGENT_SESSION_TIMEOUT_MINS * 60)
    }
}

// Discord Send Message Tool
#[derive(Debug, Clone)]
struct DiscordSendMessageTool {
    ctx: Arc<Context>,
    channel_id: ChannelId,
    reply_to_message_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiscordSendMessageArgs {
    content: String,
    #[serde(default)]
    reply: bool,
    /// Message ID to reply to (if reply is true). If not provided, replies to the most recent message.
    #[serde(default)]
    reply_to_message_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiscordSendMessageOutput {
    success: bool,
    message_id: Option<u64>,
    error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Discord send message error: {0}")]
struct DiscordSendMessageError(String);

impl Tool for DiscordSendMessageTool {
    const NAME: &'static str = "send_discord_message";
    type Error = DiscordSendMessageError;
    type Args = DiscordSendMessageArgs;
    type Output = DiscordSendMessageOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "send_discord_message".to_string(),
            description: "Send a message to the Discord channel. Use this to respond to users."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The message content to send"
                    },
                    "reply": {
                        "type": "boolean",
                        "description": "Whether to reply to a specific message (true) or send a standalone message (false)",
                        "default": false
                    },
                    "reply_to_message_id": {
                        "type": "number",
                        "description": "The Discord message ID to reply to (only used when reply=true). If not provided when reply=true, will reply to the most recent message."
                    }
                },
                "required": ["content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Clone values to move into the spawned task
        let ctx = self.ctx.clone();
        let channel_id = self.channel_id;
        let default_reply_to_message_id = self.reply_to_message_id;
        let content = args.content.clone();
        let reply = args.reply;

        // Use the message ID from args if provided, otherwise fall back to default
        let target_message_id = if reply {
            args.reply_to_message_id.or(default_reply_to_message_id)
        } else {
            None
        };

        // Spawn the Discord API operations in a separate task to avoid Sync issues
        let handle = tokio::spawn(async move {
            let mut message_builder = CreateMessage::new().content(&content);

            if reply && target_message_id.is_some() {
                if let Ok(original_msg) = channel_id
                    .message(&ctx.http, target_message_id.unwrap())
                    .await
                {
                    message_builder = message_builder.reference_message(&original_msg);
                }
            }

            channel_id.send_message(&ctx.http, message_builder).await
        });

        match handle.await {
            Ok(Ok(sent_message)) => {
                tracing::debug!("Sent Discord message: {}", args.content);
                Ok(DiscordSendMessageOutput {
                    success: true,
                    message_id: Some(sent_message.id.get()),
                    error: None,
                })
            }
            Ok(Err(e)) => {
                tracing::error!("Failed to send Discord message: {}", e);
                Ok(DiscordSendMessageOutput {
                    success: false,
                    message_id: None,
                    error: Some(e.to_string()),
                })
            }
            Err(e) => {
                tracing::error!("Task join error while sending Discord message: {}", e);
                Ok(DiscordSendMessageOutput {
                    success: false,
                    message_id: None,
                    error: Some(format!("Task execution failed: {}", e)),
                })
            }
        }
    }
}

// Fetch Page Content Tool
#[derive(Debug, Clone)]
struct FetchPageContentTool;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FetchPageContentArgs {
    url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FetchPageContentOutput {
    content: String,
    success: bool,
    error: Option<String>,
}

#[derive(Debug, Error)]
#[error("Fetch page content error: {0}")]
struct FetchPageContentError(String);

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

/// Build conversation history for agent context
async fn build_conversation_history(
    ctx: &Context,
    channel_id: ChannelId,
    message_context_size: usize,
) -> Result<Vec<RigMessage>, eyre::Error> {
    let bot_user_id = ctx.cache.current_user().id;

    let fetched_messages: Vec<Message> = channel_id
        .messages_iter(&ctx.http)
        .filter_map(|m| async {
            m.ok()
                .filter(|msg| !msg.content.trim().is_empty() || !msg.attachments.is_empty())
        })
        .take(message_context_size)
        .collect()
        .await;

    let mut rig_messages = Vec::new();

    // Process messages chronologically (oldest first)
    for msg in fetched_messages.into_iter().rev() {
        let rig_message = discord_message_to_rig_message(&msg, bot_user_id);
        rig_messages.push(rig_message);
    }

    Ok(rig_messages)
}

/// The agentic message handler using rig with multi-turn support
async fn handle_message_batch(
    ctx: Context,
    messages: Vec<QueuedMessage>,
    handler: Arc<Handler>,
) -> Result<(), eyre::Error> {
    if messages.is_empty() {
        return Ok(());
    }

    let channel_id = messages[0].message.channel_id;

    // Ensure we have an agent session for this channel with enough context to include recent messages
    let context_size = std::cmp::max(MESSAGE_CONTEXT_SIZE, messages.len());
    handler
        .get_or_create_agent_session(&ctx, channel_id, context_size)
        .await?;

    // Add the new messages to the agent's conversation and let it naturally respond
    {
        let mut sessions = handler.agent_sessions.lock().await;
        if let Some(session) = sessions.get_mut(&channel_id) {
            // Convert the batch of new messages to RigMessage format
            let new_messages = queued_messages_to_rig_messages(&messages);

            // Add new messages to the conversation history
            session.add_messages(new_messages);

            // Execute agent interaction with multi-turn reasoning
            let _ = execute_agent_interaction(session, messages.len(), channel_id).await;
        }
    }

    Ok(())
}

pub struct Handler {
    pub message_queue: Arc<Mutex<HashMap<ChannelId, Vec<QueuedMessage>>>>,
    pub openai_api_key: String,
    /// Track pending timers for each channel to avoid duplicate processing
    pub pending_timers: Arc<Mutex<HashMap<ChannelId, tokio::task::JoinHandle<()>>>>,
    /// Persistent agent sessions per channel for multi-turn conversations
    pub agent_sessions: Arc<Mutex<HashMap<ChannelId, AgentSession>>>,
}

impl Handler {
    pub fn new(openai_api_key: String) -> Self {
        Self {
            message_queue: Arc::new(Mutex::new(HashMap::new())),
            openai_api_key,
            pending_timers: Arc::new(Mutex::new(HashMap::new())),
            agent_sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a new Handler instance that shares the same underlying data
    fn clone_for_task(&self) -> Arc<Self> {
        Arc::new(Handler {
            message_queue: self.message_queue.clone(),
            openai_api_key: self.openai_api_key.clone(),
            pending_timers: self.pending_timers.clone(),
            agent_sessions: self.agent_sessions.clone(),
        })
    }

    /// Get or create an agent session for a channel
    async fn get_or_create_agent_session(
        &self,
        ctx: &Context,
        channel_id: ChannelId,
        context_size: usize,
    ) -> Result<(), eyre::Error> {
        let mut sessions = self.agent_sessions.lock().await;

        // Check if session exists and is not expired
        let needs_new_session = sessions
            .get(&channel_id)
            .is_none_or(|session| session.is_expired());

        if needs_new_session {
            // Create OpenAI client and build agent
            let openai_client = openai::Client::new(&self.openai_api_key);

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

            sessions.insert(channel_id, AgentSession::new(agent, history));
        } else {
            // Update activity timestamp
            if let Some(session) = sessions.get_mut(&channel_id) {
                session.update_activity();
            }
        }

        Ok(())
    }

    /// Schedule processing for a channel after the debounce timeout
    async fn schedule_channel_processing(&self, ctx: Context, channel_id: ChannelId) {
        // Cancel any existing timer for this channel
        {
            let mut timers = self.pending_timers.lock().await;
            if let Some(handle) = timers.remove(&channel_id) {
                handle.abort();
            }
        }

        // Create new timer
        let message_queue = self.message_queue.clone();
        let pending_timers = self.pending_timers.clone();
        let handler = self.clone_for_task();

        let handle = tokio::spawn(async move {
            // Wait for the debounce timeout
            tokio::time::sleep(Duration::from_millis(MESSAGE_DEBOUNCE_TIMEOUT_MS)).await;

            // Remove this timer from pending list
            {
                let mut timers = pending_timers.lock().await;
                timers.remove(&channel_id);
            }

            // Process the messages for this channel
            let messages = {
                let mut queue = message_queue.lock().await;
                queue.remove(&channel_id).unwrap_or_default()
            };

            if !messages.is_empty() {
                if let Err(e) = handle_message_batch(ctx, messages, handler).await {
                    tracing::error!(
                        "Error processing message batch for channel {}: {}",
                        channel_id,
                        e
                    );
                }
            }
        });

        // Store the timer handle
        {
            let mut timers = self.pending_timers.lock().await;
            timers.insert(channel_id, handle);
        }
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if !WHITELIST_CHANNELS.contains(&msg.channel_id.get()) {
            return;
        }

        // Ignore messages from bots
        if msg.author.bot {
            return;
        }

        // Add to queue
        let queued_msg = QueuedMessage { message: msg };

        let channel_id = queued_msg.message.channel_id;
        {
            let mut queue = self.message_queue.lock().await;
            let channel_messages = queue.entry(channel_id).or_insert_with(Vec::new);
            channel_messages.push(queued_msg);

            // Limit queue size per channel
            if channel_messages.len() > 10 {
                channel_messages.remove(0);
            }
        }

        // Schedule processing for this channel (this will reset the timer if one exists)
        self.schedule_channel_processing(ctx, channel_id).await;
    }

    async fn ready(&self, _: Context, ready: Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);
    }
}

/// Helper function to format Discord message content with message ID, timestamp, and username
fn format_message_content(msg: &Message) -> String {
    const MAX_REF_MSG_LEN: usize = 100; // Maximum length for referenced message preview

    let timestamp_str = msg.timestamp.to_rfc3339();
    let author_name = &msg.author.name;
    let message_id = msg.id.get();

    // Check if message mentions the bot (we'll need the bot user ID for this)
    // For now, we'll check if the message content contains the bot name
    let mentions_bot = msg.content.contains(DISCORD_BOT_NAME)
        || msg.content.contains("@") && msg.content.to_lowercase().contains("irony");

    // Get referenced message preview
    let referenced_message_preview = msg
        .referenced_message
        .as_ref()
        .map(|m| {
            let content_preview = if m.content.len() > MAX_REF_MSG_LEN {
                format!(
                    "{}...",
                    &m.content[..m
                        .content
                        .char_indices()
                        .nth(MAX_REF_MSG_LEN)
                        .map(|(n, _)| n)
                        .unwrap_or(0)]
                )
            } else {
                m.content.clone()
            };
            format!("{}: {}", m.author.name, content_preview)
        })
        .unwrap_or_else(|| "None".to_string());

    // Build context block
    let context_block = format!(
        "\n\n<<context>>\n* Replied To: [{}]\n* Mentions/Replies Bot: [{}]\n<</context>>",
        referenced_message_preview, mentions_bot
    );

    let base_message = if msg.content.is_empty() && !msg.attachments.is_empty() {
        // Handle attachments (images, files, etc.)
        format!(
            "[Message ID: {}] [{}] {}: [Attachment: {}]",
            message_id,
            timestamp_str.unwrap_or_else(|| "N/A".to_string()),
            author_name,
            msg.attachments
                .iter()
                .map(|a| a.filename.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        format!(
            "[Message ID: {}] [{}] {}: {}",
            message_id,
            timestamp_str.unwrap_or_else(|| "N/A".to_string()),
            author_name,
            msg.content
        )
    };

    format!("{}{}", base_message, context_block)
}

/// Helper function to convert a Discord message to a RigMessage
fn discord_message_to_rig_message(
    msg: &Message,
    bot_user_id: serenity::model::id::UserId,
) -> RigMessage {
    let is_bot_message = msg.author.id == bot_user_id;

    if is_bot_message {
        // For bot messages, just use text content
        let content = format_message_content(msg);
        RigMessage::assistant(content)
    } else {
        // For user messages, handle both text and images
        let mut has_images = false;
        let mut content_parts = Vec::new();

        // Add text content first
        let text_content = format_message_content(msg);
        content_parts.push(UserContent::text(text_content.clone()));

        // Process attachments for images
        for attachment in &msg.attachments {
            if attachment
                .content_type
                .as_ref()
                .is_some_and(|ct| ct.starts_with("image/"))
            {
                content_parts.push(UserContent::image(
                    attachment.proxy_url.clone(),
                    None,                    // format
                    None,                    // media_type
                    Some(ImageDetail::Auto), // detail level
                ));
                has_images = true;
            }
        }

        if has_images {
            match OneOrMany::many(content_parts) {
                Ok(content) => RigMessage::from(content),
                Err(_) => RigMessage::user(text_content), // Fallback to text-only if content list is empty
            }
        } else {
            // If no images, just use the text content
            RigMessage::user(text_content)
        }
    }
}

/// Helper function to convert a batch of QueuedMessages to RigMessages (all as user messages)
fn queued_messages_to_rig_messages(messages: &[QueuedMessage]) -> Vec<RigMessage> {
    messages
        .iter()
        .map(|queued_msg| {
            let content = format_message_content(&queued_msg.message);
            RigMessage::user(content)
        })
        .collect()
}

/// Create the system prompt for the Discord bot agent
const SYSTEM_PROMPT: &str = formatcp!(
    r#"[CONTEXT]
You are processing a sequence of Discord messages provided chronologically (oldest first).
Each message object has a 'role' ('user' or 'assistant'). 'Assistant' messages are from the bot you are acting as or analyzing ("{}"). This is IMPORTANT because it means if a message starts with [Message ID: xxx] [TIMESTAMP] {}, the message IS FROM YOU YOURSELF. Use this information to avoid repeating what you've said or adjust behavior accordingly to align with what you've said, or continue responding to what you've left in the middle.
Message content starts with metadata followed by the actual message:
1.  A Message ID in brackets (e.g., '[Message ID: 123456789]').
2.  An ISO 8601 timestamp in brackets (e.g., '[2023-10-27T10:30:00Z]').
3.  The author's Discord username followed by a colon (e.g., 'JohnDoe: ').
4.  The message text, potentially including images.
5.  Additional context within "<<context>>...<</context>>" tags, like bot mentions or reply info.

Interpret the full message content, considering message IDs, timestamps, author, text, images, fetched links, and the <<context>> block. Use timestamps and authorship to gauge flow and relevance.

**IMPORTANT**: If a user message mentions the bot and starts with '!', treat it as a potential command that might override standard behavior (e.g., "! be silent"). Factor this into your analysis/response.

Each message in the conversation history includes its Discord message ID in the format "[Message ID: 123456789]".
When you want to reply to a specific message, use that message ID in the reply_to_message_id parameter.

[PERSONA]
You ARE the Discord bot "{}". Witty, sarcastic, friendly, casual. Part of a fun, informal community.

Only reply to messages that you find interesting, relevant, or that you can add value to. If a message is too basic, repetitive, or doesn't warrant a response, just ignore it. Don't feel obligated to reply to every message. For example, you can correct misinformation or add valueable insights to the ongoing conversation. Absolutely avoid being too spammy.

[TASK GUIDANCE]
**RESPONSE LENGTH & STOPPING:**
-   **DEFAULT TO ONE MESSAGE.** Your goal is almost always a single, concise response.
-   **Simple Inputs (e.g., "thanks", "ok", "lol", agreement): Respond ONCE briefly.** Do NOT elaborate or send multiple messages for simple social cues or acknowledgments.
-   **Multi-Message Exception (RARE):** ONLY consider a second message if the *first message* delivered complex information (like code, a detailed explanation) AND you have a *distinctly separate, highly valuable* follow-up point (like a crucial example or critical clarification) that could not fit reasonably in the first.
-   **DO NOT REPEAT:** Absolutely avoid generating multiple messages that rephrase the same core idea, sentiment, or acknowledgment. If you or anyone else has said it, move on or stop.

**ABSOLUTELY AVOID:**
-   Starting messages with phrases that just confirm understanding before providing the answer.
-   Generic AI sounds.
-   Being overly helpful/corrective unless witty.
-   Asking for confirmation.

**MULTI-MESSAGE FLOW (Use Sparingly):**
-   Your *first* message MUST contain the main point/answer.
-   **ONLY generate a second message IF you have a *distinctly new* angle, a relevant follow-up question, or a concrete example that significantly adds value beyond the first message.**
-   **DO NOT generate third or subsequent messages unless absolutely necessary to convey critical, distinct information that couldn't fit before.** The bar for continuing is VERY HIGH.
-   **CRITICAL: Avoid generating multiple messages that just rephrase, slightly alter, or elaborate on the *same core idea* or sentiment expressed in your previous message.** Each message needs *substantive novelty*.
-   **Prefer stopping early.** If in doubt, don't send additional messages. Never send follow-up messages that just rephrase or slightly expand on what you already said.

**CRITICAL:** Do not repeat or rephrase what have been said in the chat history. If the insight or humour topic is similar to what have been said in this chat, avoid responding unless you have something genuinely new to add.

[TONE & STYLE INSTRUCTIONS]
-   **BE YOUR PERSONA:** Witty, maybe ironic/sarcastic, be mean if necessary.
-   **Be Casual:** Natural language. Avoid formality, preaching, teaching unless that *is* the witty angle.
-   **Be Relevant:** Respond to the *latest* message(s), primarily the user message that triggered this response.
-   **Be Concise (usually):** Adapt length. Short/punchy often good, but detail is okay if needed. Match chat style (e.g., lowercase, slang).
-   **Use Markdown Subtly:** `*italic*`, `**bold**`, `` `code` `` sparingly. 1-2 relevant emojis okay.

[STYLE - GEN Z]
speak like a gen z. informal tone, slang, abbreviations, lowcaps often preferred. make it sound hip.

example gen z slang:
asl, based, basic, beat your face, bestie, bet, big yikes, boujee, bussin', clapback, dank, ded, drip, glow-up, goat., hits diff, ijbol, i oop, it's giving..., iykyk, let him cook, l+ratio, lit, moot/moots, npc, ok boomer, opp, out of pocket, period/perioduh, sheesh, shook, simp, situationship, sksksk, slaps, slay, soft-launch, stan, sus, tea, understood the assignment, valid, vibe check, wig, yeet

[TOOLS AVAILABLE]
You have access to tools that let you:
- Send messages to Discord (send_discord_message) - use with reply=false for standalone messages, reply=true or reply_to_message_id=<message_id> to reply to recent messages
- Fetch web page content when needed (fetch_page_content)

You can use multi-turn reasoning to:
- Fetch URL content and then provide thoughtful analysis
- Send multiple messages to build a complete response (but prefer single messages)
- Chain multiple tool calls together for complex tasks

[OUTPUT INSTRUCTIONS]
- Use tools to send Discord messages - don't output raw text
- When sending Discord messages, you can reply to recent messages by setting reply=true (the system will automatically determine which message to reply to based on context)
- Be strategic about when to respond - add value or humor to the conversation
- Remember previous interactions in this channel for better continuity"#,
    DISCORD_BOT_NAME,
    DISCORD_BOT_NAME,
    DISCORD_BOT_NAME
);

/// Helper to execute agent multi-turn reasoning and handle the response
async fn execute_agent_interaction(
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
