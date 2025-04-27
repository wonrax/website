use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestMessageContentPartImage, ChatCompletionRequestMessageContentPartText,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        ChatCompletionRequestUserMessageContentPart, CreateChatCompletionRequestArgs, ImageUrl,
    },
    Client as OpenAIClient,
};
use const_format::formatcp;
use futures_util::StreamExt;
use regex::Regex;
use serenity::all::{ChannelId, CreateMessage, Message, Ready, Typing};
use serenity::async_trait;
use serenity::prelude::*;
use std::{sync::LazyLock, time::Duration};
use tracing;

const WHITELIST_CHANNELS: [u64; 2] = [1133997981637554188, 1119652436102086809];
const MESSAGE_CONTEXT_SIZE: usize = 30;
const LAYER1_MODEL: &str = "gpt-4.1-nano";
const LAYER2_MODEL: &str = "gpt-4.1";
const LAYER1_TEMPERATURE: f32 = 0.3;
const LAYER2_TEMPERATURE: f32 = 0.75;
const LAYER1_MAX_TOKENS: u16 = 300;
const LAYER2_MAX_TOKENS: u16 = 4096;
const RESPONSE_THRESHOLD: i32 = 7;
const URL_FETCH_TIMEOUT_SECS: Duration = Duration::from_secs(15);
const MAX_REF_MSG_LEN: usize = 50; // Max length for referenced message preview

// Regex to remove timestamp and author prefix if the bot accidentally outputs it
static CLEAN_MESSAGE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z\]\s+[^:]+:\s+")
        .expect("Invalid regex for cleaning message")
});

const COMMON_SYSTEM_CONTEXT: &str = formatcp!(
    r#"[CONTEXT]
You are processing a sequence of Discord messages provided chronologically (oldest first).
Each message object has a 'role' ('user' or 'assistant'). 'Assistant' messages are from the bot you are acting as or analyzing ("The Irony Himself").
Message content starts with metadata followed by the actual message:
1.  An ISO 8601 timestamp in brackets (e.g., '[2023-10-27T10:30:00Z]').
2.  The author's Discord username followed by a colon (e.g., 'JohnDoe: ').
3.  The message text, potentially including fetched link content or image URLs.
4.  Additional context within "<<context>>...<</context>>" tags, like bot mentions or reply info.

User message 'content' can be complex:
- Simple text following metadata.
- An array of text parts (each with metadata) and image URLs (`ImageUrl`).
- Text parts may include fetched link content: '[Fetched Link Content: URL]...[End Fetched Link Content]'.

Interpret the full message content, considering timestamps, author, text, images, fetched links, and the <<context>> block. Use timestamps and authorship to gauge flow and relevance.

**IMPORTANT**: If a user message mentions the bot and starts with '!', treat it as a potential command that might override standard behavior (e.g., "! be silent"). Factor this into your analysis/response."#
);

const LAYER1_SYSTEM_PROMPT: &str = formatcp!(
    r#"[ROLE] Discord Conversation Analyst

[CONTEXT]
You will be given a sequence of User and Assistant messages representing a Discord conversation history (oldest first). 'Assistant' messages are from the bot ("The Irony Himself"), 'User' messages are from others. Analyze the **final message** in the sequence.

[TASK]
Evaluate if "The Irony Himself" should respond, considering the channel's casual, fun, friendly vibe. Consider:
*   Direct Engagement: Is the last message a question to the bot? Does it mention the bot? (Greatly increases response chance).
*   Relevance & Flow: Does it continue the immediate topic? Is it engaging?
*   Engagement Potential: Opportunity to add value, humor, or continue naturally?
*   Bot Activity: Was 'Assistant' the last/penultimate speaker? (Lean against responding unless directly engaged).
*   Information Value: Can a *brief* (1-2 sentence) interesting fact/perspective fit the vibe?
*   Context/Correction: Does the last message miss crucial context, contain errors (in a debate), or misunderstand concepts?
*   Humor Potential: Clear opportunity for witty/sarcastic comment on the *last message* or *current topic*?
*   Commands: Does the last message seem like a command to the bot (e.g., starting with '!' after mention)? Adjust score/decision accordingly.

Note: Avoid replying to yourself ('Assistant' as the last message). Detect irony/sarcasm.

[OUTPUT FORMAT]
Output *only* in this format, each key on a new line. No extra text.
Insight: <One or two sentences for the potential insight, OR "None". Separate multiple insights with a semicolon.>
HumorTopic: <Brief topic/idea for a joke relevant to the last message, OR "None". Separate multiple ideas with a semicolon.>
Score: <0-10 score reflecting need/opportunity. Increase for direct mentions. Decrease for commands like "be silent".>
Respond: <"Yes" if score >= {RESPONSE_THRESHOLD} (or bot mentioned and score is not drastically lowered by a command), otherwise "No">
"#
);

fn generate_layer2_system_prompt(insight: Option<&str>, humor_topic: Option<&str>) -> String {
    let task_guidance = match (humor_topic, insight) {
        (Some(topic), _) if topic != "None" => {
            format!(r#"
Your main goal is to generate a witty, sarcastic, or funny response related to: '{}'. Keep it relevant to the latest message.

[TONE & STYLE INSTRUCTIONS]
• 1–2 sentences typically.
• (Optional) End with 1-2 savage/ironic emoji.
• References: memes, dev pains, pop culture sass, gen alpha brain rot etc. Feel free to be edgy/politically incorrect if contextually appropriate for a closed channel.

Sample Vibe Check Outputs (adapt beyond compsci):
*   "this code's got more memory leaks than your mama's weight bro 💀"
*   "these unit tests running slower than blizzard's sexual harassment investigations ⏳⚖️"
*   "this inheritance hierarchy more fucked up than elon's twitter algo 🌐🪓"
*   "who dereferenced null? must be that intern who still uses java 8 ☕️🧟"
*   "our pipeline more broken than crypto bros after ftx collapsed 💸📉""#, topic)
        },
        (_, Some(valuable_insight)) if valuable_insight != "None" => {
            format!(r#"
Your main goal is to briefly share this interesting point/perspective casually: '{}'. Ensure natural flow.

[TONE & STYLE INSTRUCTIONS]
*   **BE YOUR PERSONA:** Witty, maybe ironic/sarcastic, be mean if necessary.
*   **Be Casual:** Natural language. Avoid formality, preaching, teaching unless that *is* the witty angle.
*   **Be Relevant:** Respond to the *latest* message(s).
*   **Be Concise (usually):** Adapt length. Short/punchy often good, but detail is okay if needed. Match chat style (e.g., lowercase, slang).
*   **Use Markdown Subtly:** `*italic*`, `**bold**`, `` `code` `` sparingly. 1-2 relevant emojis okay.
*   **AVOID:** Generic AI sound, being overly helpful/corrective unless witty, asking for confirmation unless truly unclear/risky."#, valuable_insight)
        },
        _ => "Your main goal is to engage naturally with the latest message. Keep the conversation flowing fun and friendly. Be witty or add irony if appropriate.".to_string(),
    };

    format!(
        r#"[PERSONA]
You ARE the Discord bot "The Irony Himself". Witty, sarcastic, friendly, casual. Part of a fun, informal community.

[CONTEXT]
You are seeing recent conversation history (User/Assistant messages) chronologically. Generate the *next* message as 'Assistant'.

[TASK GUIDANCE]
{task_guidance}

[STYLE - GEN Z]
speak like a gen z. informal tone, slang, abbreviations, lowcaps often preferred. make it sound hip.

example gen z slang:
asl, based, basic, beat your face, bestie, bet, big yikes, boujee, bussin', clapback, dank, ded, drip, glow-up, goat., hits diff, ijbol, i oop, it's giving..., iykyk, let him cook, l+ratio, lit, moot/moots, npc, ok boomer, opp, out of pocket, period/perioduh, sheesh, shook, simp, situationship, sksksk, slaps, slay, soft-launch, stan, sus, tea, understood the assignment, valid, vibe check, wig, yeet

[OUTPUT INSTRUCTIONS]
*   Output *only* the raw message content for Discord.
*   NO "Assistant:", your name, or other prefixes/explanations.
*   Just the chat message text. Use blank lines for separation if needed.
*   Do NOT include the <<context>> block in the final response."#
    )
    .trim()
    .into()
}

/// Fetches content from a URL, attempts to convert HTML to Markdown.
async fn fetch_url_content_and_parse(url_str: &str) -> Result<String, eyre::Error> {
    let client = reqwest::Client::builder()
        .timeout(URL_FETCH_TIMEOUT_SECS)
        .build()?;

    let response = client.get(url_str).send().await?;

    if !response.status().is_success() {
        return Err(eyre::eyre!(
            "Failed to fetch URL {}: Status {}",
            url_str,
            response.status()
        ));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|val| val.to_str().ok())
        .unwrap_or("");

    // Only process HTML content to avoid parsing binary files etc.
    if !content_type.starts_with("text/html") {
        return Ok("[Non-HTML content, skipped]".into());
    }

    let site_content = response.text().await?;

    htmd::convert(&site_content).map_err(Into::into)
}

/// Turn the discord chat history into LLM user and assistant messages.
async fn build_history_messages(
    ctx: &Context,
    channel_id: ChannelId,
    message_context_size: usize,
) -> Result<Vec<ChatCompletionRequestMessage>, eyre::Error> {
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

    let mut history_messages: Vec<ChatCompletionRequestMessage> =
        Vec::with_capacity(fetched_messages.len());

    // Process messages chronologically (oldest first)
    for msg in fetched_messages.into_iter().rev() {
        let author_name = &msg.author.name;
        let is_bot_message = msg.author.id == bot_user_id;
        let mentions_or_replies_to_bot = msg.mentions_user_id(bot_user_id)
            || msg
                .referenced_message
                .as_ref()
                .is_some_and(|m| m.author.id == bot_user_id);

        let mut content_parts: Vec<ChatCompletionRequestUserMessageContentPart> = Vec::new();
        let mut current_text = String::new();

        // Start with metadata
        // TODO: use relative time to emphasize time relevance.
        let timestamp_str = msg.timestamp.to_rfc3339(); // Use standard format
        current_text.push_str(&format!(
            "[{}] {}: ",
            timestamp_str.unwrap_or("N/A".into()),
            author_name
        ));

        // Add main message content
        current_text.push_str(&msg.content);

        // Process attachments for images
        for attachment in &msg.attachments {
            if attachment
                .content_type
                .as_ref()
                .map_or(false, |ct| ct.starts_with("image/"))
            {
                if !current_text.is_empty() {
                    content_parts.push(ChatCompletionRequestUserMessageContentPart::Text(
                        ChatCompletionRequestMessageContentPartText { text: current_text },
                    ));

                    // Reset for possible text after image like context and linked content
                    current_text = String::new();
                }
                content_parts.push(ChatCompletionRequestUserMessageContentPart::ImageUrl(
                    ChatCompletionRequestMessageContentPartImage {
                        image_url: ImageUrl {
                            url: attachment.proxy_url.clone(),
                            detail: None, // Auto detail is usually fine
                        },
                    },
                ));
            }
        }

        let fetched_links_text = futures::stream::iter(
            msg.content
                .split_whitespace()
                .map(ToString::to_string)
                .filter(|word| word.starts_with("http://") || word.starts_with("https://")),
        )
        .map(|url| async move {
            let result = fetch_url_content_and_parse(&url).await;
            (url, result)
        })
        .buffered(1)
        .filter_map(|(url, result)| async move {
            match result {
                Ok(md_content) if !md_content.is_empty() => Some(format!(
                    "\n\n[Fetched Link Content: {}]\n{}\n[End Fetched Link Content]",
                    url,
                    md_content.trim()
                )),
                Ok(_) => {
                    // Skip successfully fetched but empty content
                    None
                }
                Err(e) => {
                    tracing::warn!(url = %url, error = %e, "Failed to fetch or parse link content");
                    Some(format!("\n[Could not fetch link: {}]", url))
                }
            }
        })
        .collect::<String>()
        .await;

        if !fetched_links_text.is_empty() {
            if !current_text.is_empty() && !current_text.ends_with('\n') {
                current_text.push('\n');
            }
            current_text.push_str(&fetched_links_text);
        }

        // Add context block
        let referenced_message_preview = msg
            .referenced_message
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
                    m.content
                };
                format!("{}: {}", m.author.name, content_preview)
            })
            .unwrap_or("None".into());

        current_text.push_str(&format!(
            "\n\n<<context>>\n* Replied To: [{}]\n* Mentions/Replies Bot: [{}]\n<</context>>",
            referenced_message_preview, mentions_or_replies_to_bot,
        ));

        // Add any remaining text as the last part
        if !current_text.is_empty() || content_parts.is_empty() {
            // Ensure at least one part if no images
            content_parts.push(ChatCompletionRequestUserMessageContentPart::Text(
                ChatCompletionRequestMessageContentPartText { text: current_text },
            ));
        }

        if is_bot_message {
            // Combine text parts for Assistant messages (images from bot ignored for now)
            let assistant_content = content_parts
                .into_iter()
                .filter_map(|part| match part {
                    ChatCompletionRequestUserMessageContentPart::Text(t) => Some(t.text),
                    _ => None, // Ignore images in bot's own history messages
                })
                .collect::<String>();

            if !assistant_content.trim().is_empty() {
                history_messages.push(
                    ChatCompletionRequestAssistantMessageArgs::default()
                        .content(assistant_content)
                        .build()? // Handle potential build errors
                        .into(),
                );
            }
        } else {
            history_messages.push(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(content_parts)
                    .build()?
                    .into(),
            );
        }
    }

    Ok(history_messages)
}

/// Represents the parsed output of the Layer 1 analysis.
#[derive(Default)]
struct Layer1AnalysisResult {
    score: i32,
    insight: Option<String>,
    humor_topic: Option<String>,
    should_respond: bool,
}

/// Parses the structured output from the Layer 1 LLM call.
fn parse_layer1_output(output: &str) -> Layer1AnalysisResult {
    let mut result = Layer1AnalysisResult::default();

    for line in output.lines() {
        let trimmed_line = line.trim();

        if trimmed_line.is_empty() {
            continue;
        }

        // Split into key and value based on the first colon
        if let Some((key, value)) = trimmed_line.split_once(':') {
            let key = key.trim();
            let value_trimmed = value.trim();

            match key {
                "Score" => {
                    result.score = value_trimmed.parse::<i32>().unwrap_or(0);
                }
                "Insight" => {
                    if !value_trimmed.is_empty() && !value_trimmed.eq_ignore_ascii_case("None") {
                        result.insight = Some(value_trimmed.to_string());
                    }
                }
                "HumorTopic" => {
                    if !value_trimmed.is_empty() && !value_trimmed.eq_ignore_ascii_case("None") {
                        result.humor_topic = Some(value_trimmed.to_string());
                    }
                }
                "Respond" => {
                    result.should_respond = value_trimmed.eq_ignore_ascii_case("Yes");
                }
                k => {
                    tracing::warn!(key = %k, "Unexpected key in Layer 1 output: {}", key);
                }
            }
        } else {
            tracing::warn!(%line, "Unexpected line format in Layer 1 output");
        }
    }

    result
}

async fn handle_message(
    openai_client: &OpenAIClient<OpenAIConfig>,
    ctx: Context,
    msg: Message,
) -> Result<(), eyre::Error> {
    // Ignore messages from the bot itself or from other bots
    if msg.author.bot {
        return Ok(());
    }

    let base_history = build_history_messages(&ctx, msg.channel_id, MESSAGE_CONTEXT_SIZE).await?;

    let common_system_message: ChatCompletionRequestMessage =
        ChatCompletionRequestSystemMessageArgs::default()
            .content(COMMON_SYSTEM_CONTEXT)
            .build()?
            .into();

    // --- Layer 1: Analyze if Bot Should Respond ---
    let layer1_system_message = ChatCompletionRequestSystemMessageArgs::default()
        .content(LAYER1_SYSTEM_PROMPT)
        .build()?
        .into();

    let mut layer1_messages = vec![common_system_message.clone(), layer1_system_message];
    layer1_messages.extend(base_history.clone()); // Clone history for Layer 1

    let layer1_request = CreateChatCompletionRequestArgs::default()
        .model(LAYER1_MODEL)
        .messages(layer1_messages)
        .max_tokens(LAYER1_MAX_TOKENS)
        .temperature(LAYER1_TEMPERATURE)
        .build()?;

    let layer1_response = openai_client.chat().create(layer1_request).await?;

    let layer1_output_content = layer1_response
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("");

    let analysis_result = parse_layer1_output(layer1_output_content);

    // --- Layer 2: Generate Response ---
    if analysis_result.should_respond && analysis_result.score >= RESPONSE_THRESHOLD {
        let _typing = Typing::start(ctx.http.clone(), msg.channel_id);

        let layer2_system_prompt = generate_layer2_system_prompt(
            analysis_result.insight.as_deref(),
            analysis_result.humor_topic.as_deref(),
        );
        let layer2_system_message = ChatCompletionRequestSystemMessageArgs::default()
            .content(layer2_system_prompt)
            .build()?
            .into();

        let mut layer2_messages = vec![common_system_message, layer2_system_message];
        layer2_messages.extend(base_history);

        let layer2_request = CreateChatCompletionRequestArgs::default()
            .model(LAYER2_MODEL)
            .messages(layer2_messages)
            .max_tokens(LAYER2_MAX_TOKENS)
            .temperature(LAYER2_TEMPERATURE)
            .build()?;

        let layer2_response = openai_client.chat().create(layer2_request).await?;

        if let Some(response_content) = layer2_response
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
        {
            let final_response = CLEAN_MESSAGE_REGEX
                .replace(response_content.trim(), "")
                .into_owned();

            if !final_response.is_empty() {
                msg.channel_id
                    .send_message(
                        &ctx.http,
                        CreateMessage::new()
                            .reference_message(&msg) // Reply to the original message
                            .content(final_response),
                    )
                    .await?;
            } else {
                tracing::warn!("Layer 2 generated an empty response after cleaning.");
            }
        }
    }

    Ok(())
}

pub struct Handler {
    pub openai_client: OpenAIClient<OpenAIConfig>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if !WHITELIST_CHANNELS.contains(&msg.channel_id.get()) {
            return;
        }

        if let Err(why) = handle_message(&self.openai_client, ctx, msg).await {
            tracing::error!(error = %why, "Error handling Discord message");
        }
    }

    async fn ready(&self, _: Context, ready: Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);
    }
}
