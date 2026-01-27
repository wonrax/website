use std::iter::zip;

use base64::Engine as _;
use futures::StreamExt;
use rig::{
    OneOrMany,
    completion::Message as RigMessage,
    message::{ImageDetail, ImageMediaType, MimeType, UserContent},
};
use scc::hash_map::OccupiedEntry;
use serenity::all::{GuildId, Message};

use crate::discord::bot::Guild;

// Message queue item for debouncing
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub message: Message,
}

/// Helper function to format Discord message content with message ID, timestamp, and username
/// with optional bot user ID for accurate mention detection
fn format_message_content_with_bot_id(
    msg: &Message,
    bot_user_id: Option<serenity::model::id::UserId>,
    guild: &Option<OccupiedEntry<'_, GuildId, Guild>>,
) -> String {
    const MAX_REF_MSG_LEN: usize = 100; // Maximum length for referenced message preview

    let timestamp_str = msg.timestamp.to_rfc3339();
    let author_name = &msg.author.name;
    let message_id = msg.id.get();

    // Check if message mentions the bot
    let mentions_bot = bot_user_id
        .map(|bot_id| msg.mentions.iter().any(|u| u.id == bot_id))
        .unwrap_or(false);

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

    let user_mentions: String = msg
        .mentions
        .iter()
        .map(|user| format!("@{}: {}", user.id, user.name))
        .collect::<Vec<_>>()
        .join("; ");

    let user_presence = if let Some(entry) = guild {
        let guild_data = entry.get();
        let presence_info = guild_data.presences.get_sync(&msg.author.id);
        match presence_info {
            Some(activities) if !activities.is_empty() => activities
                .iter()
                .map(|act| {
                    format!(
                        "{:?} {} ({}--{})",
                        act.kind,
                        act.name,
                        act.details.as_ref().map_or("No details", |d| d.as_str()),
                        act.state.as_ref().map_or("No state", |s| s.as_str())
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
            _ => "None".to_string(),
        }
    } else {
        "None".to_string()
    };

    // Build context block
    let context_block = format!(
        "\n
<<context>>
* Replied To: [{}]
* Mentions/Replies Bot: [{}]
* Users mentioned in message: [{}]
* User presence info: [{}]
<</context>>",
        referenced_message_preview, mentions_bot, user_mentions, user_presence
    );

    let base_message = if msg.content.is_empty() && !msg.attachments.is_empty() {
        // Handle attachments (images, files, etc.)
        format!(
            "[Message ID: {}] [{}] {} (@{}): [Attachment: {}]",
            message_id,
            timestamp_str.unwrap_or_else(|| "N/A".to_string()),
            author_name,
            msg.author.id,
            msg.attachments
                .iter()
                .map(|a| a.filename.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        format!(
            "[Message ID: {}] [{}] {} (@{}): {}",
            message_id,
            timestamp_str.unwrap_or_else(|| "N/A".to_string()),
            author_name,
            msg.author.id,
            msg.content
        )
    };

    format!("{}{}", base_message, context_block)
}

/// Helper function to convert a Discord message to a RigMessage
pub async fn discord_message_to_rig_message(
    msg: &Message,
    bot_user_id: serenity::model::id::UserId,
    guild: &Option<OccupiedEntry<'_, GuildId, Guild>>,
) -> RigMessage {
    let is_bot_message = msg.author.id == bot_user_id;

    if is_bot_message {
        // For bot messages, just use text content
        let content = format_message_content_with_bot_id(msg, Some(bot_user_id), guild);
        RigMessage::assistant(content)
    } else {
        // For user messages, handle both text and images
        let mut content_parts = Vec::new();

        // Add text content first
        let text_content = format_message_content_with_bot_id(msg, Some(bot_user_id), guild);
        content_parts.push(UserContent::text(text_content.clone()));

        // fetch images in batch
        let images_iter = msg.attachments.iter().filter_map(|attachment| {
            attachment
                .content_type
                .as_ref()
                .and_then(|ct| ImageMediaType::from_mime_type(ct))
                .map(|media_type| (&attachment.proxy_url, media_type))
        });

        let images: Vec<_> = futures::stream::iter(images_iter.clone())
            .then(|(url, _)| reqwest::get(url))
            .filter_map(async |resp| match resp {
                Ok(r) if r.status().is_success() => Some(r.bytes()),
                Ok(r) => {
                    tracing::error!(
                        status = r.status().as_u16(),
                        "Failed to fetch image from Discord attachment due to non-success status"
                    );

                    None
                }
                Err(error) => {
                    tracing::error!(
                        ?error,
                        "Failed to fetch image from Discord attachment due to HTTP error"
                    );

                    None
                }
            })
            .filter_map(async |bytes_result| match bytes_result.await {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    tracing::error!(
                        ?error,
                        "Failed to read image bytes from Discord attachment response"
                    );

                    None
                }
            })
            .collect()
            .await;

        content_parts.extend(
            zip(images, images_iter).map(|(image_bytes, (_, media_type))| {
                UserContent::image_base64(
                    base64::prelude::BASE64_STANDARD.encode(&image_bytes),
                    Some(media_type),
                    Some(ImageDetail::Auto),
                )
            }),
        );

        match OneOrMany::many(content_parts) {
            Ok(content) => RigMessage::from(content),
            Err(_) => RigMessage::user(text_content), // Fallback to text-only if content list is empty
        }
    }
}
