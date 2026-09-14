use base64::Engine as _;
use rig::{
    OneOrMany,
    completion::Message as RigMessage,
    message::{ImageDetail, ImageMediaType, MimeType, UserContent},
};
use scc::hash_map::OccupiedEntry;
use serenity::all::{Attachment, GuildId, Message, UserId};

use crate::discord::{bot::Guild, constants::URL_FETCH_TIMEOUT_SECS};

// Message queue item for debouncing
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub message: Message,
}

/// What to do with a message's image attachments when converting it for the agent
#[derive(Debug, Clone, Copy)]
pub enum AttachmentMode {
    /// Download the images into the message so the agent sees them immediately
    Inline,
    /// Leave them as `[Attachment: name]` placeholders the agent opens on demand with
    /// `view_message_attachments`
    Placeholder,
}

/// `[Message ID: id] [timestamp] Author (@author_id)`, the header the agent uses to reply
/// to, page from, and attribute messages. Shared by the live context and the channel history
/// tool so IDs look the same in both.
fn message_header(msg: &Message) -> String {
    format!(
        "[Message ID: {}] [{}] {} (@{})",
        msg.id.get(),
        msg.timestamp
            .to_rfc3339()
            .unwrap_or_else(|| "N/A".to_string()),
        msg.author.name,
        msg.author.id
    )
}

fn attachment_names(msg: &Message) -> String {
    msg.attachments
        .iter()
        .map(|a| a.filename.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `{header}: {content} [Attachment: names]`. Attachments are always named so the agent can
/// open the ones it wasn't shown.
fn message_line(msg: &Message, tag_as_self: bool) -> String {
    let mut line = message_header(msg);
    if tag_as_self {
        line.push_str(" [you]");
    }
    line.push_str(": ");
    line.push_str(&msg.content);
    if !msg.attachments.is_empty() {
        if !msg.content.is_empty() {
            line.push(' ');
        }
        line.push_str(&format!("[Attachment: {}]", attachment_names(msg)));
    }
    line
}

/// `Author (message ID id): first 100 chars` of the message this one replies to. The ID lets
/// the agent fetch the history around it.
fn referenced_message_preview(msg: &Message) -> Option<String> {
    const MAX_REF_MSG_LEN: usize = 100; // Maximum length for referenced message preview

    msg.referenced_message.as_ref().map(|m| {
        let mut chars = m.content.chars();
        let mut content_preview: String = chars.by_ref().take(MAX_REF_MSG_LEN).collect();
        if chars.next().is_some() {
            content_preview.push_str("...");
        }
        format!(
            "{} (message ID {}): {}",
            m.author.name,
            m.id.get(),
            content_preview
        )
    })
}

/// Helper function to format Discord message content with message ID, timestamp, and username
/// with optional bot user ID for accurate mention detection
fn format_message_content_with_bot_id(
    msg: &Message,
    bot_user_id: Option<UserId>,
    guild: &Option<OccupiedEntry<'_, GuildId, Guild>>,
) -> String {
    // Check if message mentions the bot
    let mentions_bot = bot_user_id
        .map(|bot_id| msg.mentions.iter().any(|u| u.id == bot_id))
        .unwrap_or(false);

    let referenced_message_preview =
        referenced_message_preview(msg).unwrap_or_else(|| "None".to_string());

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

    format!(
        "{}\n
<<context>>
* Replied To: [{}]
* Mentions/Replies Bot: [{}]
* Users mentioned in message: [{}]
* User presence info: [{}]
<</context>>",
        message_line(msg, false),
        referenced_message_preview,
        mentions_bot,
        user_mentions,
        user_presence
    )
}

/// Rendering for history the agent fetches on demand: the same header as the live context,
/// no context block, and the bot's own messages tagged `[you]`.
pub fn format_message_compact(msg: &Message, bot_user_id: UserId) -> String {
    let mut line = message_line(msg, msg.author.id == bot_user_id);
    if let Some(reply) = referenced_message_preview(msg) {
        line.push_str("\n  ↪ replying to ");
        line.push_str(&reply);
    }
    line
}

/// The image type of an attachment, when it is one the agent can view
pub fn image_media_type(attachment: &Attachment) -> Option<ImageMediaType> {
    attachment
        .content_type
        .as_deref()
        .and_then(ImageMediaType::from_mime_type)
}

/// Downloads an attachment through Discord's media proxy
pub async fn download_attachment(attachment: &Attachment) -> Result<Vec<u8>, eyre::Error> {
    let client = reqwest::Client::builder()
        .timeout(URL_FETCH_TIMEOUT_SECS)
        .build()?;
    let response = client
        .get(&attachment.proxy_url)
        .send()
        .await?
        .error_for_status()?;
    Ok(response.bytes().await?.to_vec())
}

/// Helper function to convert a Discord message to a RigMessage
pub async fn discord_message_to_rig_message(
    msg: &Message,
    bot_user_id: UserId,
    guild: &Option<OccupiedEntry<'_, GuildId, Guild>>,
    attachments: AttachmentMode,
) -> RigMessage {
    let text_content = format_message_content_with_bot_id(msg, Some(bot_user_id), guild);

    if msg.author.id == bot_user_id {
        // For bot messages, just use text content
        return RigMessage::assistant(text_content);
    }

    let mut content_parts = vec![UserContent::text(text_content.clone())];

    if let AttachmentMode::Inline = attachments {
        for attachment in &msg.attachments {
            let Some(media_type) = image_media_type(attachment) else {
                continue;
            };
            match download_attachment(attachment).await {
                Ok(bytes) => content_parts.push(UserContent::image_base64(
                    base64::prelude::BASE64_STANDARD.encode(&bytes),
                    Some(media_type),
                    Some(ImageDetail::Auto),
                )),
                Err(error) => tracing::error!(
                    ?error,
                    filename = %attachment.filename,
                    "Failed to fetch image from Discord attachment"
                ),
            }
        }
    }

    match OneOrMany::many(content_parts) {
        Ok(content) => RigMessage::from(content),
        Err(_) => RigMessage::user(text_content), // Fallback to text-only if content list is empty
    }
}
