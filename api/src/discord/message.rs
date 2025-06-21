use crate::discord::constants::DISCORD_BOT_NAME;
use rig::{completion::Message as RigMessage, message::{ImageDetail, UserContent}, OneOrMany};
use serenity::all::{ChannelId, Context, Message};
use serenity::futures::StreamExt;

// Message queue item for debouncing
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub message: Message,
}

/// Build conversation history for agent context
pub async fn build_conversation_history(
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

/// Helper function to format Discord message content with message ID, timestamp, and username
pub fn format_message_content(msg: &Message) -> String {
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
pub fn discord_message_to_rig_message(
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
pub fn queued_messages_to_rig_messages(messages: &[QueuedMessage]) -> Vec<RigMessage> {
    messages
        .iter()
        .map(|queued_msg| {
            let content = format_message_content(&queued_msg.message);
            RigMessage::user(content)
        })
        .collect()
}
