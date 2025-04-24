use std::sync::Arc;

use futures_util::StreamExt;
use openai_api_rs::v1::api::OpenAIClient;
use openai_api_rs::v1::chat_completion::{self, ChatCompletionRequest};
use serenity::all::{CreateMessage, GuildId, Typing};
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;

fn generate_analyst_prompt_score(
    message_history: &str,
    bot_mentioned: bool,
    url: Option<&str>,
    url_content: Option<&str>,
) -> String {
    let url = url.unwrap_or("None");
    let url_content = url_content.unwrap_or("None");
    let message = message_history.split('\n').next().unwrap_or("None");

    format!(
        r#"
[ROLE] Discord Bot Technical Conversation Analyst
Note that you are "The Irony Himself" in the chat history with (is bot: true).

[CONTEXT]
**Message in question:**
{message}

**Mentioned you or not:**
{bot_mentioned}

**Recent Messages (newest first):**
{message_history}

**Linked Content (if any):**
URL: {url}

[URL CONTENT]
{url_content}
[END URL CONTENT]

[END CONTEXT]

[TASK] Evaluate the need for a response to the most recent message in the context. Consider:
• Factual or logical errors (0–3 points)
• Conceptual misunderstandings (0–3 points)
• Security or critical risks (0–5 points)
• Missed crucial background or historical context (0–2 points)
• Debate stalling (0–2 points)
• Negative weighting for recent bot contributions (–2 to –5 points if the bot is too active)
• Comedic potential (0-3 points)

Additionally, identify any “possibly valuable insight”—a short piece of info or perspective (1–2
lines) that the conversation members might appreciate, referencing context or known best practices.

If this prompt contains an image, it's the image from the user's message. You can use that to learn
more about the context of the conversation.

If the message is a question aiming to the bot, it's very likely the bot should answer it
regardless of the score. If the message directly mentions the bot, the score should be
automatically 10 in order to avoid the bot being ignored.

For the comedic potential, consider if the bot's response could be humorous or entertaining added
to the conversation. If so, assign a score between 0 and 3. Generate short, edgy dev humor about
{{concept}}, referencing {{pop_culture_ref}} if provided.

COMEDIC FORMAT:
• 1–2 sentences
• Up to 12 words total if possible
• (Optional) End with 1-2 savage/ironic emoji
• Allowed references: memes, IRL dev pains, mild pop culture sass, gen alpha brain rot etc., feel
free to be super offensive and political incorrect if it fits the context, because this is a closed
Discord channel.

Sample Outputs That Pass the Vibe Check (we only provide examples for compsci-related topics but
feel free to adapt to other topics):
- Memory Leaks
"this code's got more memory leaks than your mama's weight bro 💀"
- Slow Tests
"these unit tests running slower than Blizzard's sexual harassment investigations ⏳⚖️"
- Bad Code
"this inheritance hierarchy more fucked up than Elon's Twitter algo 🌐🪓"
- Null Pointers
"who dereferenced null? must be that intern who still uses Java 8 ☕️🧟"
- CI/CD Failures
"our pipeline more broken than crypto bros after FTX collapsed 💸📉"

Note that Discord partially support markdown so be careful with the formatting so that the text is
not rendered incorrectly, for example if you use `*` or `_` in the text, it will be rendered as
italic or bold.

[OUTPUT FORMAT]
Score: 0–10
ValuableInsight: <One or two sentences only if you see a non-trivial piece of information to add;
otherwise omit.>
Comedic: <Optional, only if you see a chance for a humorous response.>

[EXAMPLE OUTPUT]
––––––––––––––––
Score: 8
ValuableInsight: “You might also highlight how samplesort’s average behavior differs from mergesort.”
Comedic: None

Notes on Multilingual:
• By default, keep the output in English unless explicitly asked otherwise.
• If the conversation is predominantly another language (≥80% non-English), you may provide this
analysis in that language instead."#
    )
    .trim()
    .into()
}

fn generate_analyst_prompt() -> String {
    r#"
[TASK]
Provide a concise correction or deeper insight, referencing the flagged issues or ValuableInsight.
• 1–2 sentences max per issue if possible
• Use Markdown for code samples (e.g., ```rust)
• Neutral, helpful tone, or shift to the channel’s language if appropriate
• Prefer the insight or correction over the joke if possible, we can only choose one

Example Good Output, NOTE THAT ONLY OUTPUT THE RESPONSE (THAT IS THE DISCORD MESSAGE BEING SENT)
THAN OTHER TEXT, DO NOT INCLUDE THE NAME OR ROLE WITH YOUR RESPONSE (e.g. "Bot (is bot: true): " or
"Analyst: "), DO NOT INCLUDE THE SCORE OR THE VALUEABLE ANALYSIS TO THE FINAL RESPONSE:
–––––––––––––––––––
Actually, counting sort is O(n + k), but it only works if k (the range of inputs) isn’t huge. See
the example in the link for details.
-------------------
        "#
    .trim()
    .into()
}

pub struct Handler {
    pub openai_client: Arc<Mutex<OpenAIClient>>,
}

async fn handle_message(
    openai_client: &mut OpenAIClient,
    ctx: Context,
    msg: Message,
) -> Result<(), eyre::Error> {
    // ignore if message is from ourself
    if msg.author.id == ctx.cache.current_user().id {
        return Ok(());
    }

    if msg
        .guild_id
        .is_none_or(|g| g != GuildId::new(968774421668065330))
    {
        return Ok(());
    }

    let _typing = Typing::start(ctx.http.clone(), msg.channel_id);

    const MESSAGE_CONTEXT_SIZE: usize = 20;
    let messages = msg
        .channel_id
        .messages_iter(ctx.http.clone())
        .take(MESSAGE_CONTEXT_SIZE);

    let bot_mentioned = msg
        .mentions
        .iter()
        .any(|user| user.id == ctx.cache.current_user().id);

    let url_in_message = msg
        .content
        .split_whitespace()
        .find(|word| word.starts_with("http://") || word.starts_with("https://"))
        .map(|url| url.to_string());

    // crawl the URL if it exists and convert to markdown using htmd
    let url_content = if let Some(ref url) = url_in_message {
        let site_content = reqwest::get(url)
            .await
            .map_err(|e| eyre::eyre!("Failed to fetch URL content: {e}"))?
            .text()
            .await
            .map_err(|e| eyre::eyre!("Failed to read URL content: {e}"))?;
        let content = htmd::convert(&site_content)
            .map_err(|e| eyre::eyre!("Failed to convert URL content to markdown: {e}"))?;

        Some(content)
    } else {
        None
    };

    let score_prompt = generate_analyst_prompt_score(
        &messages
            .filter_map(async |msg| {
                msg.ok()
                    .and_then(|m| if m.content.is_empty() { None } else { Some(m) })
            })
            .map(|msg| {
                format!(
                    "{} (is bot: {}): {}",
                    msg.author.name, msg.author.bot, msg.content
                )
            })
            .collect::<Vec<_>>()
            .await
            .join("\n"),
        bot_mentioned,
        url_in_message.as_deref(),
        url_content.as_deref(),
    );

    let image = msg
        .attachments
        .iter()
        .find(|attachment| {
            attachment
                .content_type
                .as_ref()
                .map(|ct| ct.starts_with("image/"))
                .unwrap_or(false)
        })
        .map(|attachment| attachment.proxy_url.clone());

    let mut chat = vec![chat_completion::ChatCompletionMessage {
        role: chat_completion::MessageRole::system,
        content: match image {
            Some(image) => chat_completion::Content::ImageUrl(vec![chat_completion::ImageUrl {
                r#type: chat_completion::ContentType::image_url,
                text: Some(score_prompt),
                image_url: Some(chat_completion::ImageUrlType { url: image }),
            }]),
            None => chat_completion::Content::Text(score_prompt),
        },
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    let req = ChatCompletionRequest::new("deepseek-chat".into(), chat.clone());

    let result = openai_client.chat_completion(req).await.unwrap();

    let score_str = &result.choices[0].message.content;
    println!("Score: {score_str:?}");
    let score = score_str
        .as_ref()
        .unwrap_or(&"".to_string())
        .to_ascii_lowercase()
        .split('\n')
        .find(|line| line.trim().starts_with("score:"))
        .map(|line| line.trim().trim_start_matches("score:").trim())
        .and_then(|score_text| score_text.parse::<i32>().ok())
        .unwrap_or(0);

    if score < 3 {
        return Ok(());
    }

    chat.push(chat_completion::ChatCompletionMessage {
        role: chat_completion::MessageRole::assistant,
        content: chat_completion::Content::Text((*score_str).clone().unwrap()),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    });

    chat.push(chat_completion::ChatCompletionMessage {
        role: chat_completion::MessageRole::system,
        content: chat_completion::Content::Text(generate_analyst_prompt()),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    });

    let req = ChatCompletionRequest::new("deepseek-chat".into(), chat.clone());
    let result = openai_client.chat_completion(req).await.unwrap();

    let response = result.choices[0].message.content.as_ref();
    if let Some(response) = response {
        if let Err(why) = msg
            .channel_id
            .send_message(
                &ctx.http,
                CreateMessage::new()
                    .reference_message(&msg)
                    .content(response),
            )
            .await
        {
            return Err(eyre::Error::new(why));
        }
    }

    Ok(())
}

#[async_trait]
impl EventHandler for Handler {
    // Set a handler for the `message` event. This is called whenever a new message is received.
    //
    // Event handlers are dispatched through a threadpool, and so multiple events can be
    // dispatched simultaneously.
    async fn message(&self, ctx: Context, msg: Message) {
        let mut openai_client = self.openai_client.lock().await;
        let _ = handle_message(&mut openai_client, ctx, msg)
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
