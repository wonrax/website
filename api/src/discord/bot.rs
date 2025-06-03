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
use eyre::Context as _;
use futures_util::StreamExt;
use regex::Regex;
use serenity::all::{ChannelId, CreateMessage, Message, Ready, Typing};
use serenity::async_trait;
use serenity::prelude::*;
use std::{
    sync::{atomic::AtomicBool, LazyLock},
    time::Duration,
};
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
You are processing a sequence of Discord messages provided chronologically (oldest first).
Each message object has a 'role' ('user' or 'assistant'). 'Assistant' messages are from the bot you are acting as or analyzing ("{DISCORD_BOT_NAME}"). This is IMPORTANT because it means if a message starts with [TIMESTAMP] {{{DISCORD_BOT_NAME}}}, the message IS FROM YOU YOURSELF. Use this information to avoid repeating what you've said or adjust behavior accordingly to align with what you've said, or continue responding to what you've left in the middle.
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

[TASK]
Analyze the **final message** in the sequence. Evaluate if "{DISCORD_BOT_NAME}" should respond, considering the channel's casual, fun, friendly vibe. Consider:
-   Direct Engagement: Is the last message a question to the bot? Does it mention the bot? (Greatly increases response chance).
-   Relevance & Flow: Does it continue the immediate topic? Is it engaging?
-   Engagement Potential: Opportunity to add value, humor, or continue naturally?
-   Bot Activity: Was 'Assistant' the last/penultimate speaker? (Lean against responding unless directly engaged).
-   Information Value: Can a *brief* (1-2 sentence) interesting fact/perspective fit the vibe?
-   Context/Correction: Does the last message miss crucial context, contain errors (in a debate), or misunderstand concepts?
-   Humor Potential: Clear opportunity for witty/sarcastic comment on the *last message* or *current topic*?
-   Commands: Does the last message seem like a command to the bot (e.g., starting with '!' after mention)? Adjust score/decision accordingly.

Note: Avoid replying to yourself ('Assistant' as the last message). Detect irony/sarcasm.
**CRITICAL:** Do not repeat or rephrase what have been said in the chat history. If the insight or humour topic is similar to what have been said in this chat, lower the score substantially.

[OUTPUT FORMAT]
You MUST output your analysis *only* in the following format, with each key on a new line. Do NOT add any other explanation or text. DO NOT output in the format of a user or assistant message (that is [timestamp] [author]: [message]), you MUST follow the format below:
Insight: <One or two sentences for the potential insight, OR "None". Separate multiple insights with a semicolon.>
HumorTopic: <Brief topic/idea for a joke relevant to the last message, OR "None". Separate multiple ideas with a semicolon. Lower the score below if this does not have enough potential to crack smiles, the bar is very high in the channel.>
Score: <0-10 score reflecting need/opportunity. Increase for direct mentions. Decrease for commands like "be silent".>
Respond: <"Yes" if score >= {RESPONSE_THRESHOLD} (or bot mentioned and score is not drastically lowered by a command), otherwise "No">
"#
);

fn generate_layer2_system_prompt(insight: Option<&str>, humor_topic: Option<&str>) -> String {
    let task_guidance = match (humor_topic, insight) {
        (_, Some(valuable_insight)) if valuable_insight != "None" => {
            format!(r#"
Your main goal is to briefly share this interesting point/perspective casually: '{}'. Ensure natural flow.

[TONE & STYLE INSTRUCTIONS]
-   **BE YOUR PERSONA:** Witty, maybe ironic/sarcastic, be mean if necessary.
-   **Be Casual:** Natural language. Avoid formality, preaching, teaching unless that *is* the witty angle.
-   **Be Relevant:** Respond to the *latest* message(s), primarily the user message that triggered this response.
-   **Be Concise (usually):** Adapt length. Short/punchy often good, but detail is okay if needed. Match chat style (e.g., lowercase, slang).
-   **Use Markdown Subtly:** `*italic*`, `**bold**`, `` `code` `` sparingly. 1-2 relevant emojis okay."#, valuable_insight)
        },
        (Some(humour_topic), _) if humour_topic != "None" => {
            format!(r#"
Your main goal is to generate a witty, sarcastic, or funny response related to: '{}'. Keep it relevant to the latest message.

[TONE & STYLE INSTRUCTIONS]
• 1–2 sentences typically.
• (Optional) End with 1-2 savage/ironic emoji.
• References: memes, dev pains, pop culture sass, gen alpha brain rot etc. Feel free to be edgy/politically incorrect if contextually appropriate for a closed channel.

Sample Vibe Check Outputs (adapt beyond compsci):
-   "this code's got more memory leaks than your mama's weight bro 💀"
-   "these unit tests running slower than blizzard's sexual harassment investigations ⏳⚖️"
-   "this inheritance hierarchy more fucked up than elon's twitter algo 🌐🪓"
-   "who dereferenced null? must be that intern who still uses java 8 ☕️🧟"
-   "our pipeline more broken than crypto bros after ftx collapsed 💸📉""#, humour_topic)
        },
        _ => "Your main goal is to engage naturally with the latest message. Keep the conversation flowing fun and friendly. Be witty or add irony if appropriate.".to_string(),
    };

    format!(
        r#"[PERSONA]
You ARE the Discord bot "{DISCORD_BOT_NAME}". Witty, sarcastic, friendly, casual. Part of a fun, informal community.

[CONTEXT]
You are seeing recent conversation history (User/Assistant messages) chronologically. Generate the *next* message as 'Assistant'. Remember, messages starting with "[TIMESTAMP] {DISCORD_BOT_NAME}:" are YOUR OWN previous messages in this sequence.

[TASK GUIDANCE]
**RESPONSE LENGTH & STOPPING:**
-   **DEFAULT TO ONE MESSAGE.** Your goal is almost always a single, concise response.
-   **Simple Inputs (e.g., "thanks", "ok", "lol", agreement): Respond ONCE briefly.** Do NOT elaborate or send multiple messages for simple social cues or acknowledgments.
-   **Multi-Message Exception (RARE):** ONLY consider a second message if the *first message* delivered complex information (like code, a detailed explanation) AND you have a *distinctly separate, highly valuable* follow-up point (like a crucial example or critical clarification) that could not fit reasonably in the first.
-   **NEVER send more than TWO messages.** The bar for a second message is extremely high.
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
-   **Prefer stopping early.** If in doubt, output "[END]". Output only "[END]" when you have nothing genuinely *new* and *valuable* to add. Never output "[END]" in a valid message, if you want to stop, output "[END]" in a new message.

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
                    // TODO: bring fetched link content to <<context>> block
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

    let layer1_response = openai_client
        .chat()
        .create(layer1_request.clone())
        .await
        .wrap_err_with(|| format!("{:?}", &layer1_request))
        .wrap_err("Failed to make layer 1 request")?;

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

            let layer2_response = match openai_client.chat().create(layer2_request.clone()).await {
                Ok(res) => res,
                Err(e) => {
                    tracing::error!(error = %e, ?layer2_request, "Layer 2 OpenAI API call failed during loop");
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
    pub error_acked: AtomicBool,
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if !WHITELIST_CHANNELS.contains(&msg.channel_id.get()) {
            return;
        }

        let chan_id = msg.channel_id.clone();

        if let Err(why) = handle_message(&self.openai_client, ctx.clone(), msg).await {
            tracing::error!(error = ?why, "Error handling Discord message");

            if !self.error_acked.load(std::sync::atomic::Ordering::Relaxed) {
                self.error_acked
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                chan_id
                    .send_message(
                        &ctx.http,
                        CreateMessage::new().content(format!(
                            "An error occurred while processing your message:\n```\n{:?}\n```",
                            why
                        )),
                    )
                    .await
                    .inspect_err(|e| {
                        tracing::error!(error = ?e, "Failed to send error message to Discord");
                    });
            }
        } else {
            self.error_acked
                .compare_exchange(
                    true,
                    false,
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                )
                .ok();
        }
    }

    async fn ready(&self, _: Context, ready: Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);
    }
}
