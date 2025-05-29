use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestMessageContentPartImage, ChatCompletionRequestMessageContentPartText,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        ChatCompletionRequestUserMessageContentPart, CreateChatCompletionRequestArgs, ImageUrl,
        ReasoningEffort,
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
const LAYER1_MODEL: &str = "gpt-4.1-mini";
const LAYER2_MODEL: &str = "o4-mini";
const LAYER1_TEMPERATURE: f32 = 0.3;
const LAYER2_TEMPERATURE: f32 = 0.75;
const LAYER1_MAX_TOKENS: u16 = 300;
const LAYER2_MAX_TOKENS: u16 = 4096;
const RESPONSE_THRESHOLD: i32 = 8;
const URL_FETCH_TIMEOUT_SECS: Duration = Duration::from_secs(15);
const MAX_REF_MSG_LEN: usize = 50; // Max length for referenced message preview
const MAX_ASSISTANT_RESPONSE_MESSAGE_COUNT: usize = 1;
const DISCORD_BOT_NAME: &str = "The Irony Himself";

// Regex to remove timestamp and author prefix and context if the bot accidentally outputs it
static CLEAN_MESSAGE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?xms) # Enable comments, insignificant whitespace, multiline mode, and dotall mode
        # --- Option 1: Generic Timestamp/Author prefix at the START of the string ---
        (
            ^           # Anchor to the beginning of the entire string
            \[          # Literal opening bracket
            .*?         # *ANY* character (.), matched 0+ times (*), non-greedily (?)
                        # This will match everything inside the brackets, regardless of format
            \]          # Literal closing bracket
            \s+         # Whitespace after timestamp bracket
            [^:]+       # Author name (one or more characters that are NOT a colon)
            :           # Colon after author
            \s+         # Whitespace after colon
        )
        | # --- OR ---
        # --- Option 2: Context block ANYWHERE in the string ---
        (
            \s*           # Optional leading whitespace before the block
            <<context>>   # The literal opening tag
            .*?           # Any character (including newline due to 's' flag), matched non-greedily
            <</context>>  # The literal closing tag
            \s*           # Optional trailing whitespace after the block
        )
        ",
    )
    .expect("Invalid regex for cleaning message")
});

const COMMON_SYSTEM_CONTEXT: &str = formatcp!(
    r#"[CONTEXT]
Process Discord messages chronologically (oldest first). Messages:
- Role: 'user' or 'assistant' (assistant = YOU as "{DISCORD_BOT_NAME}")
- Format: [ISO_TIMESTAMP] Author: text (may include images/fetched links)
- <<context>> tags contain metadata (mentions/replies)

**CRITICAL RULES:**
1. Messages starting with [TIMESTAMP] {{{DISCORD_BOT_NAME}}} are YOUR OWN - align responses and avoid repetition
2. Treat '!'-prefixed mentions as commands (e.g., "! be silent") that override normal behavior
3. Parse full content: timestamps, authors, text, images, links, and <<context>> blocks"#
);

const LAYER1_SYSTEM_PROMPT: &str = formatcp!(
    r#"[ROLE] Discord Response Analyst

[TASK]
Decide if "{DISCORD_BOT_NAME}" should respond to the **final message** (casual/fun channel vibe). Key factors:

* 🚨 **DIRECT ENGAGEMENT**: Mentions bot or asks question? (STRONG response indicator)
* ⚠️ **BOT ACTIVITY**: Were you last speaker? (Avoid responding unless directly addressed)
* 💡 **INSIGHT POTENTIAL**: Can you add 1-2 sentence valuable perspective such as facts or trivia about the current topic?
* 😂 **HUMOR POTENTIAL**: Clear opening for witty/sarcastic remark?
* ❌ **REPETITION RISK**: If topic/insight exists in history, LOWER score substantially
* ⚠️ **COMMANDS**: Adjust for '!' commands (e.g., "! silent" → lower score)

[OUTPUT FORMAT]
Output EXACTLY this structure (no extra text):
Insight: <1-2 sentences OR "None">
HumorTopic: <Brief joke idea OR "None">
Score: <0-10 (↑ for mentions, ↓ for commands/repetition)>
Respond: <"Yes" if score >= {RESPONSE_THRESHOLD} OR bot mentioned (unless command overrides), else "No">"#
);

fn generate_layer2_system_prompt(insight: Option<&str>, humor_topic: Option<&str>) -> String {
    let task_guidance = match (humor_topic, insight) {
        (_, Some(i)) if i != "None" => {
            format!("PRIMARY GOAL: Casually share this insight: '{}'", i)
        }
        (Some(h), _) if h != "None" => {
            format!("PRIMARY GOAL: Make witty/sarcastic comment about: '{}'", h)
        }
        _ => "PRIMARY GOAL: Engage naturally with latest message".to_string(),
    };

    format!(
        r#"[PERSONA]
You ARE "{DISCORD_BOT_NAME}" - witty, sarcastic, Gen-Z style Discord bot

[CRITICAL RESPONSE RULES]
1. **ONE MESSAGE DEFAULT**: 99% of responses should be single message
2. **MULTI-MESSAGE EXCEPTION (RARE)**:
   - First message MUST contain core response
   - Second and more ONLY if:
     a) First was complex (e.g., code) AND
     b) Follow-up is DISTINCTLY new (never rephrasing)
3. **STOPPING MECHANISM**: Output "[END]" when nothing new to add
4. **AVOID**:
   - Repetition of history/your own messages
   - "Assistant:" prefixes
   - Overly helpful/corrective tones without wit
   - Generic AI phrases

[TONE/GEN-Z STYLE]
- Casual slang (ded, slay, sheesh, npc, vibe check)
- Savage/ironic emojis (💀,⚡️,👀)
- Pop-culture/meme references
- Lowcaps optional

[TASK]
{task_guidance}. Respond to LATEST message while following ALL rules above.

[OUTPUT]
Raw Discord message ONLY (no prefixes/explanation, do not include the <<context>> block)"#
    )
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
#[derive(Debug, Default)]
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
            let key = key.trim().to_ascii_lowercase();
            let value_trimmed = value.trim();

            match key.as_str() {
                "score" => {
                    result.score = value_trimmed.parse::<i32>().unwrap_or(0);
                }
                "insight" => {
                    if !value_trimmed.is_empty()
                        && !value_trimmed.to_ascii_lowercase().starts_with("none")
                    {
                        result.insight = Some(value_trimmed.to_string());
                    }
                }
                "humortopic" => {
                    if !value_trimmed.is_empty()
                        && !value_trimmed.to_ascii_lowercase().starts_with("none")
                    {
                        result.humor_topic = Some(value_trimmed.to_string());
                    }
                }
                "respond" => {
                    result.should_respond = value_trimmed.to_ascii_lowercase().starts_with("yes");
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

    tracing::debug!(?layer1_messages, "layer 1 message");

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

    tracing::debug!(layer1_output_content, "layer 1 response");

    let analysis_result = parse_layer1_output(layer1_output_content);

    tracing::debug!(?analysis_result, "Parsed Layer 1 analysis result");

    // --- Layer 2: Generate Response ---
    if analysis_result.should_respond {
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

        let mut is_first_response = true; // Flag to control replying vs sending
        let mut response_message_count = 0;

        'outer: loop {
            if response_message_count >= MAX_ASSISTANT_RESPONSE_MESSAGE_COUNT {
                tracing::debug!("Layer 2 message history exceeded limit, breaking loop.");
                break;
            }

            if !is_first_response {
                // Add a small delay to prevent rate limiting or flooding
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }

            let layer2_request = CreateChatCompletionRequestArgs::default()
                .model(LAYER2_MODEL)
                .messages(layer2_messages.clone())
                .max_completion_tokens(25000u32) // including reasoning tokens
                .n(1)
                .reasoning_effort(ReasoningEffort::High)
                // Commented due to not compatible with current model (prev: gpt-4.1)
                // .max_tokens(LAYER2_MAX_TOKENS)
                // .temperature(LAYER2_TEMPERATURE)
                // .stop(["[END]".to_string()])
                .build()?;

            tracing::debug!(?layer2_request, "Layer 2 request");

            let layer2_response = match openai_client.chat().create(layer2_request).await {
                Ok(res) => res,
                Err(e) => {
                    tracing::error!(error = %e, "Layer 2 OpenAI API call failed during loop");
                    break;
                }
            };

            let assistant_response_content = layer2_response
                .choices
                .first()
                .and_then(|c| c.message.content.as_deref())
                .unwrap_or("")
                .trim();

            tracing::debug!(
                raw_response = assistant_response_content,
                "Layer 2 raw response in loop"
            );

            if assistant_response_content == "[END]" {
                tracing::debug!("Received '[END]' signal. Terminating response generation.");
                break;
            }

            let final_response = CLEAN_MESSAGE_REGEX
                .replace_all(assistant_response_content, "")
                .into_owned();

            if final_response.is_empty() {
                tracing::warn!("Layer 2 generated an empty response after cleaning (and not '[END]'). Stopping.");
                break;
            }

            // Split the response into parts on blank lines (two newlines)
            let parts: Vec<&str> = final_response
                .split("\n\n")
                .map(|part| part.trim())
                .filter(|part| !part.is_empty())
                .collect();

            // Send each part as a separate message
            for part in parts {
                if part == "[END]" {
                    tracing::debug!("Received '[END]' signal. Terminating response generation.");
                    break 'outer;
                }

                let builder = if is_first_response {
                    CreateMessage::new().reference_message(&msg).content(part)
                } else {
                    tokio::time::sleep(Duration::from_millis(part.len() as u64 * 20)).await;
                    CreateMessage::new().content(part)
                };

                if let Err(e) = msg.channel_id.send_message(&ctx.http, builder).await {
                    tracing::error!(error = %e, "Failed to send Layer 2 message part to Discord");
                    break;
                }

                is_first_response = false;
                response_message_count += 1;
            }

            // --- Update History for Next Iteration ---
            let assistant_message = ChatCompletionRequestAssistantMessageArgs::default()
                .content(format!(
                    "[{}] {}: {}",
                    chrono::Utc::now().to_rfc3339(),
                    DISCORD_BOT_NAME,
                    final_response
                ))
                .build()?
                .into();

            layer2_messages.push(assistant_message);
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
