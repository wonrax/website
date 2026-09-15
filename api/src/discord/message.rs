use base64::Engine as _;
use rig::{
    OneOrMany,
    completion::Message as RigMessage,
    message::{ImageDetail, ImageMediaType, MimeType, UserContent},
};
use scc::hash_map::OccupiedEntry;
use serenity::all::{
    Activity, ActivityType, Attachment, ChannelId, Context, GuildId, Message, MessageId, UserId,
};

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

/// `[#id] [timestamp] Author`, the header the agent uses to reply to, page from, and attribute
/// messages. Shared by the live context and the lookup tools so IDs look the same everywhere.
/// The author's user ID is left out on purpose: `fetch_message_user_ids` hands it out on demand.
fn message_header(msg: &Message) -> String {
    format!(
        "[#{}] [{}] {}",
        msg.id.get(),
        msg.timestamp
            .to_rfc3339()
            .unwrap_or_else(|| "N/A".to_string()),
        msg.author.name,
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

/// `Author (#id)` of the message this one replies to. No content preview: the agent fetches the
/// message with `fetch_message` when the reply target matters.
fn reply_reference(msg: &Message) -> Option<String> {
    msg.referenced_message
        .as_ref()
        .map(|m| format!("{} (#{})", m.author.name, m.id.get()))
}

/// Names of the users the message pings, without IDs
fn mentioned_names(msg: &Message) -> Option<String> {
    if msg.mentions.is_empty() {
        return None;
    }
    Some(
        msg.mentions
            .iter()
            .map(|user| user.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// The author's current activities from the guild presence cache, when it has any
fn presence(author: UserId, guild: &Option<OccupiedEntry<'_, GuildId, Guild>>) -> Option<String> {
    let entry = guild.as_ref()?.get().presences.get_sync(&author)?;
    let activities = entry.get();
    if activities.is_empty() {
        return None;
    }
    Some(
        activities
            .iter()
            .map(describe_activity)
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn describe_activity(activity: &Activity) -> String {
    // A custom status carries its text in `state` and a fixed "Custom Status" name
    if matches!(activity.kind, ActivityType::Custom) {
        return format!(
            "Status: {}",
            activity.state.as_deref().unwrap_or(&activity.name)
        );
    }
    let mut text = format!("{:?} {}", activity.kind, activity.name);
    let extra: Vec<&str> = activity
        .details
        .as_deref()
        .into_iter()
        .chain(activity.state.as_deref())
        .collect();
    if !extra.is_empty() {
        text.push_str(&format!(" ({})", extra.join("; ")));
    }
    text
}

/// The message line followed by a `<<context>>` block with the reply target, mentions, and the
/// author's presence. Lines with nothing to say are dropped, and so is the block when none remain.
fn format_message_with_context(
    msg: &Message,
    guild: &Option<OccupiedEntry<'_, GuildId, Guild>>,
) -> String {
    let mut text = message_line(msg, false);
    let context: Vec<String> = [
        reply_reference(msg).map(|reply| format!("Replied To: {reply}")),
        mentioned_names(msg).map(|names| format!("Mentions: {names}")),
        presence(msg.author.id, guild).map(|activities| format!("Presence: {activities}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !context.is_empty() {
        text.push_str("\n<<context>>\n");
        text.push_str(&context.join("\n"));
        text.push_str("\n<</context>>");
    }
    text
}

/// Rendering for messages the agent fetches on demand: the same header as the live context,
/// no context block, and the bot's own messages tagged `[you]`.
pub fn format_message_compact(msg: &Message, bot_user_id: UserId) -> String {
    let mut line = message_line(msg, msg.author.id == bot_user_id);
    if let Some(reply) = reply_reference(msg) {
        line.push_str("\n  ↪ replying to ");
        line.push_str(&reply);
    }
    line
}

/// Parses the ID the agent copied from a `[#id]` header, with or without the brackets and hash.
/// The error tells it that retrying with the same value is pointless.
pub fn parse_message_id(param: &str, raw: &str) -> Result<MessageId, String> {
    raw.trim()
        .trim_matches(|c| c == '[' || c == ']' || c == '#')
        .parse::<u64>()
        .map(MessageId::new)
        .map_err(|_| {
            format!(
                "{param} must be the digits of a [#ID] message header, got {raw:?}; retrying with the same value will not help"
            )
        })
}

/// One message of the channel, for the lookup tools
pub async fn fetch_channel_message(
    ctx: &Context,
    channel_id: ChannelId,
    message_id: MessageId,
) -> Result<Message, String> {
    channel_id
        .message(&ctx.http, message_id)
        .await
        .map_err(|e| {
            tracing::error!(?e, message_id = message_id.get(), "Failed to fetch message");
            format!(
                "could not fetch message {message_id}: {e}; it may have been deleted or belong to another channel"
            )
        })
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
    let text_content = format_message_with_context(msg, guild);

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn user(id: u64, name: &str) -> Value {
        json!({ "id": id.to_string(), "username": name })
    }

    /// The fields serenity requires to deserialize a `Message`
    fn message(id: u64, author: Value, content: &str) -> Value {
        json!({
            "id": id.to_string(),
            "channel_id": "1",
            "author": author,
            "content": content,
            "timestamp": "2026-09-14T12:03:30.469Z",
            "tts": false,
            "mention_everyone": false,
            "mentions": [],
            "mention_roles": [],
            "attachments": [],
            "embeds": [],
            "pinned": false,
            "type": 0
        })
    }

    fn parse(value: Value) -> Message {
        serde_json::from_value(value).expect("test message should deserialize")
    }

    #[test]
    fn plain_message_is_one_line_without_ids() {
        let msg = parse(message(
            1549027814278434918,
            user(350884319360712705, "wonrax"),
            "hello",
        ));
        let text = format_message_with_context(&msg, &None);

        assert!(
            text.starts_with("[#1549027814278434918] [2026-09-14T12:03:30"),
            "{text}"
        );
        assert!(text.ends_with("] wonrax: hello"), "{text}");
        assert!(!text.contains("350884319360712705"), "{text}");
        assert!(!text.contains("<<context>>"), "{text}");
    }

    #[test]
    fn reply_and_mentions_render_as_names_and_message_ids() {
        let bot = user(1364822809259544586, "The Irony Himself");
        let mut value = message(
            2,
            user(350884319360712705, "wonrax"),
            "<@1364822809259544586> m ngáo à",
        );
        value["mentions"] = json!([bot]);
        value["type"] = json!(19);
        value["referenced_message"] = message(1549023175818354742, bot, "thì khỏi gửi");
        let text = format_message_with_context(&parse(value), &None);

        let expected = "\n<<context>>\n\
            Replied To: The Irony Himself (#1549023175818354742)\n\
            Mentions: The Irony Himself\n\
            <</context>>";
        assert!(text.ends_with(expected), "{text}");
        assert!(!text.contains("thì khỏi gửi"), "{text}");
    }

    #[test]
    fn compact_format_tags_the_bot_and_names_the_reply_target() {
        let bot_id = UserId::new(1364822809259544586);
        let mut value = message(3, user(bot_id.get(), "The Irony Himself"), "lol");
        value["referenced_message"] = message(2, user(350884319360712705, "wonrax"), "m ngáo à");
        let text = format_message_compact(&parse(value), bot_id);

        assert!(
            text.ends_with("] The Irony Himself [you]: lol\n  ↪ replying to wonrax (#2)"),
            "{text}"
        );
    }

    #[test]
    fn message_ids_parse_with_or_without_header_decoration() {
        for raw in ["42", " 42 ", "#42", "[#42]"] {
            let id = parse_message_id("message_id", raw).expect(raw);
            assert_eq!(id.get(), 42, "{raw}");
        }

        let error = parse_message_id("message_id", "forty-two").expect_err("not an ID");
        assert!(
            error.starts_with("message_id must be the digits"),
            "{error}"
        );
    }
}
