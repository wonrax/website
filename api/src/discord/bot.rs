use std::{sync::Arc, time::Duration};

use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestMessageContentPartImage, ChatCompletionRequestMessageContentPartText,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        ChatCompletionRequestUserMessageContent, ChatCompletionRequestUserMessageContentPart,
        CreateChatCompletionRequestArgs, ImageUrl,
    },
    Client as OpenAIClient,
};
use futures_util::StreamExt;
use regex::Regex;
use reqwest::Url;
use serenity::all::{ChannelId, CreateMessage, Typing};
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;

async fn fetch_url_content_and_parse(url_str: &str) -> Result<String, eyre::Error> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
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

    if !content_type.starts_with("text/html") {
        tracing::debug!(
            "Skipping non-HTML content for URL: {} (not HTML content)",
            url_str
        );
        return Ok("[Empty]".into());
    }

    let site_content = response.text().await?;

    let md_content = htmd::convert(&site_content).inspect_err(|e| {
        tracing::error!(
            "Failed to convert URL {} content to markdown: {}",
            url_str,
            e
        )
    });

    Ok(md_content.unwrap_or("[Empty]".into()))
}

// Generates the SYSTEM prompt content for the Layer 1 analysis task
fn generate_layer1_system_prompt() -> String {
    format!(
        r#"
[ROLE] Discord Conversation Analyst

[CONTEXT]
You will be given a sequence of User and Assistant messages representing a Discord conversation history, ordered chronologically (oldest first). The 'Assistant' messages are from the bot ("The Irony Himself"), and 'User' messages are from others.

[TASK]
Analyze the **final message** in the provided sequence. Evaluate if the bot "The Irony Himself" should respond, considering the channel's casual, fun, and friendly vibe.
Your analysis should consider:
*   Direct Engagement: Is the last message a question to the bot? Does it mention the bot?
*   Relevance & Flow: Does it continue the immediate prior topic? Is it engaging?
*   Engagement Potential: Is there an opportunity to add value, humor, or continue the conversation naturally?
*   Bot Activity: Was 'Assistant' the last or second-to-last speaker? (If so, lean against responding unless directly engaged).
*   Information Value: Is there a *brief* (1-2 sentence) interesting fact, perspective, or clarification that fits the casual vibe?
*   The last message missed crucial background or historical context, contains factual or logical errors while in a debate, or conceptual misunderstandings?
*   Humor Potential: Is there a clear opportunity for a witty, sarcastic, or funny comment relevant to the *last message* or *current topic*?

Note that if the lastest message is from you (The Irony Himself), avoid replying to yourself so that we don't go into recursive mode.
Also try your best to detect irony and sarcasm in user messages, don't take everything too seriously.

[OUTPUT FORMAT]
You MUST output your analysis *only* in the following format, with each key on a new line. Do NOT add any other explanation or text.
Score: <0-10 score reflecting the need/opportunity to respond. Score 10 if bot mentioned.>
Insight: <One or two sentences for the potential insight, OR the literal word "None">
HumorTopic: <Brief topic/idea for a joke relevant to the last message, OR the literal word "None">
Respond: <"Yes" if score is >= 7 (or bot mentioned), otherwise "No">
"#
    )
    .trim()
    .into()
}
// Generates the SYSTEM prompt content for the Layer 2 response generation task
fn generate_layer2_system_prompt(
    insight: Option<&str>,     // Parsed from Layer 1 output
    humor_topic: Option<&str>, // Parsed from Layer 1 output
) -> String {
    let task_guidance = match (humor_topic, insight) {
        (Some(topic), _) if topic != "None" => {
            format!(r#"
Your main goal is to generate a witty, sarcastic, or funny response related to: '{}'. Keep it relevant to the latest message.

[TONE & STYLE INSTRUCTIONS]
• 1–2 sentences
• (Optional) End with 1-2 savage/ironic emoji
• Allowed references: memes, IRL dev pains, mild pop culture sass, gen alpha brain rot etc., feel free to be super offensive and political incorrect if it fits the context, because this is a closed Discord channel.

Sample Outputs That Pass the Vibe Check (we only provide examples for compsci-related topics but feel free to adapt to other topics):
*   "this code's got more memory leaks than your mama's weight bro 💀"
*   "these unit tests running slower than Blizzard's sexual harassment investigations ⏳⚖️"
*   "this inheritance hierarchy more fucked up than Elon's Twitter algo 🌐🪓"
*   "who dereferenced null? must be that intern who still uses Java 8 ☕️🧟"
*   "our pipeline more broken than crypto bros after FTX collapsed 💸📉""#, topic)
        },
        (_, Some(valuable_insight)) if valuable_insight != "None" => {
            format!(r#"
Your main goal is to briefly share this interesting point or perspective in a casual, friendly way: '{}'. Make sure it flows naturally.

[TONE & STYLE INSTRUCTIONS]
*   **BE YOUR PERSONA:** Witty, maybe a bit ironic/sarcastic, be mean if necessary.
*   **Be Casual:** Use natural language. Avoid formality, preaching, or sounding like a teacher.
*   **Be Relevant:** Respond to the *latest* message(s) in the history.
*   **Be Concise (usually):** Adapt length to the chat flow. Short and punchy is often good, however feel free to make a detailed response when needed. For example, write in all lowercase if everybody is doing so, or use slang and jargons without formal consideration.
*   **Use Markdown Subtly:** `*italic*`, `**bold**`, `` `code` `` sparingly if needed. 1-2 relevant emojis are okay.
*   **AVOID:** Sounding like generic AI, being overly helpful/corrective unless that *is* the witty angle, asking for confirmation unless the user request is truly unclear or risky."#, valuable_insight)
        },
        _ => "Your main goal is to engage naturally with the latest message. Keep the conversation flowing in a fun and friendly way. Be witty or add a touch of irony if appropriate.".to_string(),
    };

    format!(
        r#"
[PERSONA]
You ARE the Discord bot "The Irony Himself". Your personality is witty, sarcastic, friendly, and casual. You are part of a fun, informal community channel. You are participating in the conversation provided.

[CONTEXT]
You are seeing the recent conversation history (User/Assistant messages) chronologically. You need to generate the *next* message in the sequence, acting as the 'Assistant'.

[TASK GUIDANCE]
{task_guidance}

speak like a gen z. the answer must be in an informal tone, use slang, abbreviations, and anything that can make the message sound hip. specially use gen z slang (as opposed to millenials). the list below has a  list of gen z slang. also, speak in lowcaps.

here are some example slang terms you can use:
1. **Asl**: Shortened version of "as hell."
2. **Based**: Having the quality of being oneself and not caring about others' views; agreement with an opinion.
3. **Basic**: Preferring mainstream products, trends, and music.
4. **Beat your face**: To apply makeup.
5. **Bestie**: Short for 'best friend'.
6. **Bet**: An affirmation; agreement, akin to saying "yes" or "it's on."
7. **Big yikes**: An exclamation for something embarrassing or cringeworthy.
9. **Boujee**: Describing someone high-class or materialistic.
10. **Bussin'**: Describing food that tastes very good.
12. **Clapback**: A swift and witty response to an insult or critique.
13. **Dank**: Refers to an ironically good internet meme.
14. **Ded**: Hyperbolic way of saying something is extremely funny.
15. **Drip**: Trendy, high-class fashion.
16. **Glow-up**: A significant improvement in one's appearance or confidence.
17. **G.O.A.T.**: Acronym for "greatest of all time."
18. **Hits different**: Describing something that is better in a peculiar way.
19. **IJBOL**: An acronym for "I just burst out laughing."
20. **I oop**: Expression of shock, embarrassment, or amusement.
21. **It's giving…**: Used to describe the vibe or essence of something.
22. **Iykyk**: Acronym for "If you know, you know," referring to inside jokes.
23. **Let him cook**: Allow someone to proceed uninterrupted.
24. **L+Ratio**: An online insult combining "L" for loss and "ratio" referring to social media metrics.
25. **Lit**: Describes something exciting or excellent.
26. **Moot/Moots**: Short for "mutuals" or "mutual followers."
27. **NPC**: Someone perceived as not thinking for themselves or acting robotically.
28. **OK Boomer**: A pejorative used to dismiss or mock outdated attitudes, often associated with the Baby Boomer generation.
29. **Opp**: Short for opposition or enemies.
30. **Out of pocket**: Describing behavior that is considered excessive or inappropriate.
31. **Period/Perioduh**: Used to emphasize a statement.
32. **Sheesh**: An exclamation of praise or admiration.
33. **Shook**: Feeling shocked or surprised.
34. **Simp**: Someone who is overly affectionate or behaves in a sycophantic way, often in pursuit of a romantic relationship.
35. **Situationship**: An ambiguous romantic relationship that lacks clear definition.
36. **Sksksk**: An expression of amusement or laughter.
37. **Slaps**: Describing something, particularly music, that is of high quality.
38. **Slay**: To do something exceptionally well.
39. **Soft-launch**: To hint at a relationship discreetly on social media.
40. **Stan**: To support something, or someone, fervently.
41. **Sus**: Short for suspect or suspicious.
42. **Tea**: Gossip.
43. **Understood the assignment**: To perform well or meet expectations.
44. **Valid**: Describing something as acceptable or reasonable.
45. **Vibe check**: An assessment of someone's mood or attitude.
46. **Wig**: An exclamation used when something is done exceptionally well.
47. **Yeet**: To throw something with force; an exclamation of excitement.

[COMEDIC FORMAT]

[OUTPUT INSTRUCTIONS]
*   Output *only* the raw message content you want to send to Discord.
*   Do NOT include "Assistant:", your name, or any other prefix or explanation.
*   Just the text of the chat message.
"#
    )
    .trim()
    .into()
}
// --- Helper function to build the history message sequence ---
async fn build_history_messages(
    ctx: &Context,
    channel_id: ChannelId,
    message_context_size: usize,
) -> Result<Vec<ChatCompletionRequestMessage>, eyre::Error> {
    let bot_user_id = ctx.cache.current_user().id;

    let fetched_messages: Vec<Message> = channel_id
        .messages_iter(ctx.http.clone())
        .take(message_context_size)
        .filter_map(|m| async {
            m.ok().and_then(|m| {
                if m.content.trim().is_empty() && m.attachments.is_empty() {
                    None
                } else {
                    Some(m)
                }
            })
        })
        .collect()
        .await;

    let mut history_messages: Vec<ChatCompletionRequestMessage> = Vec::new();

    for msg in fetched_messages.into_iter().rev() {
        // Chronological order
        let author_name = msg.author.name.clone();
        let is_bot_message = msg.author.id == bot_user_id;

        // --- Content Parts ---
        let mut content_parts: Vec<ChatCompletionRequestUserMessageContentPart> = Vec::new();
        let mut accumulated_text = String::new();

        // TODO: use relative time instead to emphasize the importance
        let timestamp_str = format!("[{}] ", msg.timestamp.to_rfc3339().unwrap_or("".into()));
        accumulated_text.push_str(&timestamp_str);

        let author_prefix = format!("{}: ", author_name); // Format author name prefix
        accumulated_text.push_str(&author_prefix); // Add it after timestamp

        if !msg.content.is_empty() {
            accumulated_text.push_str(&msg.content);
        }

        let mut has_images = false;
        for attachment in &msg.attachments {
            if attachment
                .content_type
                .as_ref()
                .map_or(false, |ct| ct.starts_with("image/"))
            {
                has_images = true;
                // Add accumulated text as a Text part before the Image part
                if !accumulated_text.is_empty() {
                    content_parts.push(ChatCompletionRequestUserMessageContentPart::Text(
                        ChatCompletionRequestMessageContentPartText {
                            text: accumulated_text.clone(),
                        },
                    ));
                    accumulated_text.clear(); // Reset for any text *after* this image
                }
                content_parts.push(ChatCompletionRequestUserMessageContentPart::ImageUrl(
                    ChatCompletionRequestMessageContentPartImage {
                        image_url: ImageUrl {
                            url: attachment.proxy_url.clone(),
                            detail: None,
                        },
                    },
                ));
            }
        }

        let words = msg.content.split_whitespace();
        let mut fetched_links_text = String::new(); // Accumulate link text separately
        for word in words {
            if word.starts_with("http://") || word.starts_with("https://") {
                if let Ok(parsed_url) = Url::parse(word) {
                    match fetch_url_content_and_parse(parsed_url.as_str()).await {
                        Ok(md_content) => {
                            // Add clear delimiters and newlines for readability
                            fetched_links_text.push_str(&format!(
                                "\n\n[Fetched Link Content: {}]\n{}\n[End Fetched Link Content]",
                                parsed_url,
                                md_content.trim()
                            ));
                        }
                        Err(_) => {
                            fetched_links_text
                                .push_str(&format!("\n[Could not fetch link: {}]", parsed_url));
                        }
                    }
                }
            }
        }
        if !fetched_links_text.is_empty() {
            // Ensure there's a space or newline before adding link blocks if original text exists
            if !accumulated_text.is_empty()
                && !accumulated_text.ends_with('\n')
                && !accumulated_text.ends_with(' ')
            {
                accumulated_text.push(' ');
            }
            accumulated_text.push_str(&fetched_links_text);
        }

        // Add any remaining accumulated text as the last text part
        if !accumulated_text.is_empty() {
            content_parts.push(ChatCompletionRequestUserMessageContentPart::Text(
                ChatCompletionRequestMessageContentPartText {
                    text: accumulated_text,
                },
            ));
        }

        // --- Build the Message Object ---
        if content_parts.is_empty() {
            continue; // Skip if absolutely nothing to send
        }

        if is_bot_message {
            // Combine all text parts for assistant message (NOTE: ignore images in bot history?)
            let assistant_content = content_parts
                .iter()
                .filter_map(|part| {
                    if let ChatCompletionRequestUserMessageContentPart::Text(text_part) = part {
                        Some(text_part.text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<&str>>()
                .join("");

            if !assistant_content.trim().is_empty() {
                history_messages.push(
                    ChatCompletionRequestAssistantMessageArgs::default()
                        .content(assistant_content)
                        .build()?
                        .into(),
                );
            }
        } else {
            // User message
            let user_content = if has_images || content_parts.len() > 1 {
                ChatCompletionRequestUserMessageContent::Array(content_parts)
            } else {
                // Should only be one Text part if no images and links didn't create new parts
                if let Some(ChatCompletionRequestUserMessageContentPart::Text(text_part)) =
                    content_parts.first()
                {
                    ChatCompletionRequestUserMessageContent::Text(text_part.text.clone())
                } else {
                    continue; // Skip if something went wrong
                }
            };

            history_messages.push(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(user_content)
                    .build()?
                    .into(),
            );
        }
    }

    Ok(history_messages)
}

// --- System Prompt Updates ---

fn generate_layer_system_prompt_context() -> String {
    format!(
        r#"
[CONTEXT]
You are processing a sequence of Discord messages provided chronologically (oldest first).
Each message object has a 'role' ('user' or 'assistant'). 'Assistant' messages are from the bot you are acting as or analyzing ("The Irony Himself").
Message content starts with metadata followed by the actual message:
1.  An ISO 8601 timestamp in brackets (e.g., '[2023-10-27T10:30:00Z]').
2.  The author's Discord username followed by a colon (e.g., 'JohnDoe: ').
3.  The message text, potentially including fetched link content or image URLs.

User message 'content' can be complex beyond the initial metadata:
- It may be simple text following the metadata.
- It may be an array containing text parts (each starting with metadata) and image URLs (`ImageUrl`).
- Text parts may include fetched content from links, marked like: '[Fetched Link Content: URL]...[End Fetched Link Content]'.

Interpret the full message content, including timestamps, author names (from the prefix), text, images (via URL), and fetched link data, in the context of the conversation history. Use timestamps and authorship to gauge flow and relevance.
"#
    )
}

async fn handle_message(
    openai_client: &OpenAIClient<OpenAIConfig>,
    ctx: Context,
    msg: Message,
) -> Result<(), eyre::Error> {
    if msg.author.id == ctx.cache.current_user().id {
        return Ok(()); // Ignore messages from the bot itself
    }

    const MESSAGE_CONTEXT_SIZE: usize = 30; // Adjust as needed

    let base_history =
        match build_history_messages(&ctx, msg.channel_id, MESSAGE_CONTEXT_SIZE).await {
            Ok(history) => history,
            Err(e) => {
                tracing::error!("Error building message history: {}. Aborting.", e);
                return Ok(());
            }
        };

    let common_system_prompt_content = generate_layer_system_prompt_context();
    let common_system_message: ChatCompletionRequestMessage =
        ChatCompletionRequestSystemMessageArgs::default()
            .content(common_system_prompt_content)
            .build()?
            .into();

    // --- Layer 1 ---
    let layer1_system_prompt_content = generate_layer1_system_prompt();
    let layer1_system_message = ChatCompletionRequestSystemMessageArgs::default()
        .content(layer1_system_prompt_content)
        .build()?
        .into();
    let mut layer1_input_messages = vec![common_system_message.clone(), layer1_system_message];
    layer1_input_messages.extend(base_history.clone());
    let layer1_request = CreateChatCompletionRequestArgs::default()
        .model("gpt-4.1-mini") // Vision capable
        .messages(layer1_input_messages)
        .max_tokens(150u16)
        .temperature(0.2)
        .build()?;
    let layer1_response = openai_client.chat().create(layer1_request).await?;
    let layer1_output = layer1_response.choices[0]
        .message
        .content
        .as_deref()
        .unwrap_or("");

    // --- Parse Layer 1 ---
    // ... (parsing logic) ...
    let mut score = 0;
    let mut insight: Option<String> = None;
    let mut humor_topic: Option<String> = None;
    let mut should_respond = false;
    let threshold = 3;
    for line in layer1_output.lines() {
        /* ... parsing logic ... */
        let trimmed_line = line.trim();
        if let Some(s) = trimmed_line.strip_prefix("Score:") {
            score = s.trim().parse::<i32>().unwrap_or(0);
        } else if let Some(i) = trimmed_line.strip_prefix("Insight:") {
            let val = i.trim();
            if !val.eq_ignore_ascii_case("None") {
                insight = Some(val.to_string());
            }
        } else if let Some(h) = trimmed_line.strip_prefix("HumorTopic:") {
            let val = h.trim();
            if !val.eq_ignore_ascii_case("None") {
                humor_topic = Some(val.to_string());
            }
        } else if let Some(r) = trimmed_line.strip_prefix("Respond:") {
            should_respond = r.trim().eq_ignore_ascii_case("Yes");
        }
    }

    let bot_id = ctx.cache.current_user().id;

    // --- Decision Gate ---
    let bot_mentioned = msg.author.id != bot_id // Ignore messages from the bot itself
        && (msg.mentions_user_id(bot_id)
            || msg
                .referenced_message
                .as_ref()
                .is_some_and(|m| m.author.id == ctx.cache.current_user().id));

    if bot_mentioned || (should_respond && score >= threshold) {
        let _typing = Typing::start(ctx.http.clone(), msg.channel_id);

        // --- Layer 2 ---
        let layer2_system_prompt_content =
            generate_layer2_system_prompt(insight.as_deref(), humor_topic.as_deref());
        let layer2_system_message = ChatCompletionRequestSystemMessageArgs::default()
            .content(layer2_system_prompt_content)
            .build()?
            .into();
        let mut layer2_input_messages = vec![common_system_message, layer2_system_message];
        layer2_input_messages.extend(base_history); // Use same rich history
        let layer2_request = CreateChatCompletionRequestArgs::default()
            .model("gpt-4.1") // Vision capable
            .messages(layer2_input_messages)
            .max_tokens(4096u16)
            .temperature(0.75)
            .build()?;
        let layer2_response = openai_client.chat().create(layer2_request).await?;

        // --- Send Response ---
        if let Some(response_content) = layer2_response.choices[0].message.content.as_deref() {
            let trimmed_response = response_content.trim();
            if !trimmed_response.is_empty() {
                fn clean_message(message: &str) -> String {
                    let re =
                        Regex::new(r"^\[\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z\]\s+[^:]+:\s+")
                            .unwrap();
                    re.replace(message, "").into_owned()
                }

                msg.channel_id
                    .send_message(
                        &ctx.http,
                        CreateMessage::new()
                            .reference_message(&msg)
                            .content(clean_message(trimmed_response)),
                    )
                    .await?;
            }
        }
    }

    Ok(())
}

pub struct Handler {
    pub openai_client: Arc<OpenAIClient<OpenAIConfig>>,
}

#[async_trait]
impl EventHandler for Handler {
    // Set a handler for the `message` event. This is called whenever a new message is received.
    //
    // Event handlers are dispatched through a threadpool, and so multiple events can be
    // dispatched simultaneously.
    async fn message(&self, ctx: Context, msg: Message) {
        let _ = handle_message(&self.openai_client, ctx, msg)
            .await
            .inspect_err(|why| {
                tracing::error!("Error handling Discord message: {why}");
            });
    }

    // Set a handler to be called on the `ready` event. This is called when a shard is booted, and
    // a READY payload is sent by Discord. This payload contains data like the current user's guild
    // Ids, current user data, private channels, and more.
    //
    // In this case, just print what the current user's username is.
    async fn ready(&self, _: Context, ready: Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);
    }
}
