use std::{num::NonZeroU64, ops::Range};

use base64::Engine as _;
use chrono::{DateTime, Utc};
use rig::{
    completion::Message as RigMessage,
    message::{AssistantContent, ImageDetail, ImageMediaType, MimeType, UserContent},
};
use serenity::all::{
    Activity, ActivityType, Attachment, ChannelId, Context, GuildId, Message, MessageId,
    ReactionType, UserId,
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

/// `{content} [Attachment: names]`, with links to messages of `channel_id` written as `[#ID]`.
/// Attachments are always named so the agent can open the ones it wasn't shown.
fn message_body(content: &str, attachments: &[Attachment], channel_id: ChannelId) -> String {
    let mut body = shorten_links(content, channel_id);
    if !attachments.is_empty() {
        if !body.is_empty() {
            body.push(' ');
        }
        let names: Vec<&str> = attachments.iter().map(|a| a.filename.as_str()).collect();
        body.push_str(&format!("[Attachment: {}]", names.join(", ")));
    }
    body
}

/// `{header}: {body}`
fn message_line(msg: &Message, tag_as_self: bool) -> String {
    let mut line = message_header(msg);
    if tag_as_self {
        line.push_str(" [you]");
    }
    line.push_str(": ");
    line.push_str(&message_body(
        &msg.content,
        &msg.attachments,
        msg.channel_id,
    ));
    line
}

/// Where a message link points
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MessageLink {
    channel_id: ChannelId,
    message_id: MessageId,
}

/// The message links in `text`, what people paste to cite a message, with the bytes each spans.
/// A link wrapped in `<>` to keep Discord from previewing it spans the brackets too.
fn message_links(text: &str) -> Vec<(Range<usize>, MessageLink)> {
    const PREFIXES: [&str; 4] = [
        "discord.com/channels/",
        "ptb.discord.com/channels/",
        "canary.discord.com/channels/",
        "discordapp.com/channels/",
    ];
    let mut links = Vec::new();
    let mut from = 0;
    while let Some(found) = text.get(from..).and_then(|rest| rest.find("https://")) {
        let start = from + found;
        from = start + "https://".len();
        let rest = text.get(from..).unwrap_or_default();
        let Some(path) = PREFIXES.iter().find_map(|prefix| rest.strip_prefix(prefix)) else {
            continue;
        };
        let Some((link, len)) = parse_link_path(path) else {
            continue;
        };
        // `path` is the tail of `text`
        let mut span = start..text.len() - path.len() + len;
        if text.get(..span.start).is_some_and(|t| t.ends_with('<'))
            && text.get(span.end..).is_some_and(|t| t.starts_with('>'))
        {
            span = span.start - 1..span.end + 1;
        }
        from = span.end;
        links.push((span, link));
    }
    links
}

/// `GUILD/CHANNEL/MESSAGE` at the start of `path` (`@me` for the guild in DMs), and its length
fn parse_link_path(path: &str) -> Option<(MessageLink, usize)> {
    let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
    let guild = if path.starts_with("@me") {
        3
    } else {
        digits(path)
    };
    let rest = path.get(guild..)?.strip_prefix('/').filter(|_| guild > 0)?;
    let channel = digits(rest);
    let channel_id = rest.get(..channel)?.parse::<NonZeroU64>().ok()?;
    let rest = rest.get(channel..)?.strip_prefix('/')?;
    let message = digits(rest);
    let message_id = rest.get(..message)?.parse::<NonZeroU64>().ok()?;
    Some((
        MessageLink {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
        },
        guild + 1 + channel + 1 + message,
    ))
}

/// `text` with its links to messages of `channel_id` written as `[#ID]`, the way the agents see
/// messages and cite them
fn shorten_links(text: &str, channel_id: ChannelId) -> String {
    let mut shortened = String::with_capacity(text.len());
    let mut copied = 0;
    for (span, link) in message_links(text) {
        if link.channel_id != channel_id {
            continue;
        }
        shortened.push_str(text.get(copied..span.start).unwrap_or_default());
        shortened.push_str(&format!("[#{}]", link.message_id));
        copied = span.end;
    }
    shortened.push_str(text.get(copied..).unwrap_or_default());
    shortened
}

/// `text` with each `[#ID]` written as a link to that message of the channel, which Discord shows
/// the way it shows a link a person pasted
pub fn expand_citations(text: &str, guild_id: Option<GuildId>, channel_id: ChannelId) -> String {
    let guild = guild_id.map_or_else(|| "@me".to_string(), |id| id.to_string());
    let mut expanded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("[#") {
        expanded.push_str(rest.get(..start).unwrap_or_default());
        let after = rest.get(start + 2..).unwrap_or_default();
        let len = after.bytes().take_while(u8::is_ascii_digit).count();
        let id = after
            .get(..len)
            .and_then(|digits| digits.parse::<NonZeroU64>().ok())
            .filter(|_| after.get(len..).is_some_and(|t| t.starts_with(']')));
        match id {
            Some(id) => {
                expanded.push_str(&format!(
                    "https://discord.com/channels/{guild}/{channel_id}/{id}"
                ));
                rest = after.get(len + 1..).unwrap_or_default();
            }
            None => {
                expanded.push_str("[#");
                rest = after;
            }
        }
    }
    expanded.push_str(rest);
    expanded
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
pub fn presence(author: UserId, guild: &Guild) -> Option<String> {
    let entry = guild.presences.get_sync(&author)?;
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
/// author's `presence`. Lines with nothing to say are dropped, and so is the block when none remain.
fn format_message_with_context(msg: &Message, presence: Option<&str>) -> String {
    let mut text = message_line(msg, false);
    let context: Vec<String> = [
        reply_reference(msg).map(|reply| format!("Replied To: {reply}")),
        mentioned_names(msg).map(|names| format!("Mentions: {names}")),
        presence.map(|activities| format!("Presence: {activities}")),
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
        .parse::<NonZeroU64>()
        .map(MessageId::from)
        .map_err(|_| {
            format!(
                "{param} must be the digits of a [#ID] message header, got {raw:?}; retrying with the same value will not help"
            )
        })
}

/// `parse_message_id` over a list, for tools that take several IDs; the first bad one fails
/// the whole call
pub fn parse_message_ids(param: &str, raw: &[String]) -> Result<Vec<MessageId>, String> {
    raw.iter().map(|id| parse_message_id(param, id)).collect()
}

/// Milliseconds of 2015-01-01T00:00:00Z, the zero of every snowflake
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// The snowflake a message sent at `at` would get. Discord accepts it as a position in the
/// channel even though no message has it, which is how the tools page and search by time.
pub fn snowflake_at(at: DateTime<Utc>) -> u64 {
    let ms = u64::try_from(at.timestamp_millis()).unwrap_or(0);
    // Masking the high bits first keeps the shift from overflowing on far-future dates
    (ms.saturating_sub(DISCORD_EPOCH_MS) & (u64::MAX >> 22)) << 22
}

/// The snowflake a message sent now would get, which places what happens now among the messages
pub fn snowflake_now() -> MessageId {
    NonZeroU64::new(snowflake_at(Utc::now())).map_or(MessageId::new(1), MessageId::from)
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

/// Whether `msg` pings `user_id` or replies to one of their messages
pub fn addresses(msg: &Message, user_id: UserId) -> bool {
    msg.mentions_user_id(user_id)
        || msg
            .referenced_message
            .as_ref()
            .is_some_and(|replied| replied.author.id == user_id)
}

/// The users a message brings into the conversation: its author, the people it pings, and the
/// author it replies to, leaving out the bot
pub fn participants(msg: &Message, bot_user_id: UserId) -> Vec<UserId> {
    let mut people: Vec<UserId> = std::iter::once(msg.author.id)
        .chain(msg.mentions.iter().map(|user| user.id))
        .chain(
            msg.referenced_message
                .as_ref()
                .map(|replied| replied.author.id),
        )
        .filter(|id| *id != bot_user_id)
        .collect();
    people.sort_unstable();
    people.dedup();
    people
}

/// A channel message as the watcher reads it. The watcher never posts, so the bot's messages are
/// channel input to it like everyone else's rather than turns of its own.
pub fn observed(message: &RigMessage) -> RigMessage {
    match message {
        RigMessage::Assistant { content, .. } => RigMessage::User {
            content: content
                .iter()
                .filter_map(|part| match part {
                    AssistantContent::Text(text) => Some(UserContent::text(text.text.clone())),
                    _ => None,
                })
                .collect(),
        },
        other => other.clone(),
    }
}

/// A reaction as the agents read it, between the messages. `author` wrote the message, when
/// that's known.
pub fn reaction_line(
    reactor: &str,
    emoji: &ReactionType,
    author: Option<&str>,
    message_id: MessageId,
) -> String {
    let message = match author {
        Some(author) => format!("{author}'s message"),
        None => "a message".to_string(),
    };
    format!("[reaction] {reactor} reacted {emoji} to {message} (#{message_id})")
}

/// An edit of a message the agents have seen, with what the message says now
pub fn edit_line(
    author: &str,
    message_id: MessageId,
    channel_id: ChannelId,
    content: &str,
    attachments: &[Attachment],
) -> String {
    format!(
        "[edit] {author} edited their message (#{message_id}): {}",
        message_body(content, attachments, channel_id)
    )
}

/// The deletion of a message the agents have seen
pub fn deletion_line(author: &str, message_id: MessageId) -> String {
    format!("[deletion] {author}'s message (#{message_id}) was deleted")
}

/// Helper function to convert a Discord message to a RigMessage. `presence` is the author's,
/// when known.
pub async fn discord_message_to_rig_message(
    msg: &Message,
    bot_user_id: UserId,
    presence: Option<&str>,
    attachments: AttachmentMode,
) -> RigMessage {
    let text_content = format_message_with_context(msg, presence);

    if msg.author.id == bot_user_id {
        // For bot messages, just use text content
        return RigMessage::assistant(text_content);
    }

    let mut content_parts = vec![UserContent::text(text_content)];

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

    RigMessage::from(content_parts)
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
        let text = format_message_with_context(&msg, None);

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
        let text = format_message_with_context(&parse(value), None);

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

        for raw in ["forty-two", "0"] {
            let error = parse_message_id("message_id", raw).expect_err("not an ID");
            assert!(
                error.starts_with("message_id must be the digits"),
                "{error}"
            );
        }
    }

    #[test]
    fn message_id_lists_fail_on_the_first_bad_id() {
        let ids = parse_message_ids("source_message_ids", &["[#1]".to_string(), "2".to_string()])
            .expect("all IDs");
        assert_eq!(ids, vec![MessageId::new(1), MessageId::new(2)]);

        let error = parse_message_ids("source_message_ids", &["1".to_string(), "x".to_string()])
            .expect_err("one bad ID");
        assert!(error.starts_with("source_message_ids must be"), "{error}");
    }

    #[test]
    fn message_links_are_found_however_people_paste_them() {
        let text = "xem https://discord.com/channels/1/2/3 với \
            <https://ptb.discord.com/channels/@me/4/5>, https://discord.com/channels/1/2 thì không";
        let links: Vec<(String, u64, u64)> = message_links(text)
            .into_iter()
            .map(|(span, link)| {
                (
                    text.get(span).unwrap_or_default().to_string(),
                    link.channel_id.get(),
                    link.message_id.get(),
                )
            })
            .collect();
        assert_eq!(
            links,
            vec![
                ("https://discord.com/channels/1/2/3".to_string(), 2, 3),
                (
                    "<https://ptb.discord.com/channels/@me/4/5>".to_string(),
                    4,
                    5
                ),
            ]
        );
    }

    #[test]
    fn links_to_the_channel_read_as_message_ids() {
        let text = "như đã nói https://discord.com/channels/1/2/3 và \
            <https://discordapp.com/channels/1/2/4>, còn https://discord.com/channels/1/9/5";
        assert_eq!(
            shorten_links(text, ChannelId::new(2)),
            "như đã nói [#3] và [#4], còn https://discord.com/channels/1/9/5"
        );
    }

    #[test]
    fn cited_ids_become_links() {
        let channel_id = ChannelId::new(2);
        assert_eq!(
            expand_citations(
                "như [#1549027814278434918] nói, [#abc] với [#0] và [#12",
                Some(GuildId::new(7)),
                channel_id
            ),
            "như https://discord.com/channels/7/2/1549027814278434918 nói, [#abc] với [#0] và [#12"
        );
        assert_eq!(
            expand_citations("[#5]", None, channel_id),
            "https://discord.com/channels/@me/2/5"
        );
        let sent = expand_citations("[#5]!", Some(GuildId::new(7)), channel_id);
        assert_eq!(shorten_links(&sent, channel_id), "[#5]!");
    }

    #[test]
    fn links_to_messages_of_the_channel_render_as_ids() {
        let msg = parse(message(
            4,
            user(350884319360712705, "wonrax"),
            "như https://discord.com/channels/7/1/3 nói",
        ));
        let text = format_message_with_context(&msg, None);
        assert!(text.ends_with("wonrax: như [#3] nói"), "{text}");
    }

    #[test]
    fn events_name_the_message_they_happened_to() {
        let id = MessageId::new(3);
        let emoji = ReactionType::Unicode("💀".to_string());
        assert_eq!(
            reaction_line("gabins", &emoji, Some("The Irony Himself"), id),
            "[reaction] gabins reacted 💀 to The Irony Himself's message (#3)"
        );
        assert_eq!(
            reaction_line("gabins", &emoji, None, id),
            "[reaction] gabins reacted 💀 to a message (#3)"
        );
        assert_eq!(
            edit_line(
                "wonrax",
                id,
                ChannelId::new(2),
                "như https://discord.com/channels/7/2/1",
                &[]
            ),
            "[edit] wonrax edited their message (#3): như [#1]"
        );
        assert_eq!(
            deletion_line("wonrax", id),
            "[deletion] wonrax's message (#3) was deleted"
        );
    }

    #[test]
    fn snowflakes_round_trip_through_discord_time() {
        let at = DateTime::parse_from_rfc3339("2026-09-14T12:03:30.469Z")
            .expect("valid time")
            .with_timezone(&Utc);
        let id = MessageId::new(snowflake_at(at));
        assert_eq!(id.created_at().timestamp_millis(), at.timestamp_millis());

        // Before Discord existed there is nothing to point at
        assert_eq!(snowflake_at(DateTime::<Utc>::MIN_UTC), 0);
    }
}
