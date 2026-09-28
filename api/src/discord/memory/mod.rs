//! What the bot remembers across sessions. Agents append observations to a log; a background
//! dreamer distills the log into docs, a profile per user of a server and the lore of each
//! channel, which sessions read in their instructions.

pub mod dream;
mod store;

use chrono::{DateTime, Datelike as _, NaiveDate, Utc};
use serenity::all::{ChannelId, GuildId, MessageId, UserId};

use crate::discord::{agent::LlmBackend, chatgpt::DbPool, constants::DISCORD_BOT_NAME};

pub use dream::Dreamer;
pub use store::{FoundObservation, MemoryStore, NewObservation};

/// The memories of every server the bot is in: where they're kept, and what distills them
#[derive(Clone)]
pub struct MemorySystem {
    store: MemoryStore,
    dreamer: Dreamer,
}

impl MemorySystem {
    /// Starts the dreamer, which rewrites the docs on `llm`
    pub fn start(db: DbPool, llm: LlmBackend) -> Self {
        let store = MemoryStore::new(db);
        let dreamer = Dreamer::spawn(store.clone(), llm);
        Self { store, dreamer }
    }

    /// Takes deleted messages out of the evidence of the observations citing them. An
    /// observation left without any is withdrawn: search stops finding it, and the docs let go
    /// of it at their next dream.
    pub async fn forget_messages(
        &self,
        guild_id: GuildId,
        channel_id: ChannelId,
        message_ids: &[MessageId],
    ) {
        match self
            .store
            .withdraw_sources(guild_id, channel_id, message_ids)
            .await
        {
            Ok(0) => {}
            Ok(withdrawn) => {
                tracing::info!(
                    withdrawn,
                    channel_id = channel_id.get(),
                    "Withdrew memories whose messages were deleted"
                );
                self.dreamer.wake(guild_id);
            }
            Err(e) => tracing::error!(?e, "Failed to forget deleted messages"),
        }
    }

    pub fn channel(&self, guild_id: GuildId, channel_id: ChannelId) -> ChannelMemory {
        ChannelMemory {
            store: self.store.clone(),
            dreamer: self.dreamer.clone(),
            guild_id,
            channel_id,
        }
    }
}

/// A channel's way to its memories
#[derive(Clone)]
pub struct ChannelMemory {
    pub store: MemoryStore,
    pub dreamer: Dreamer,
    pub guild_id: GuildId,
    pub channel_id: ChannelId,
}

/// What a doc is about
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    /// A user, across the channels of the server
    User(UserId),
    /// A channel's lore
    Channel(ChannelId),
}

/// A doc as a session reads it
#[derive(Debug)]
pub struct Note {
    pub content: String,
    pub written_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct Profile {
    pub user_id: UserId,
    /// As last seen in the server
    pub name: Option<String>,
    pub note: Note,
}

/// The docs a session reads
#[derive(Debug, Default)]
pub struct Notes {
    pub lore: Option<Note>,
    pub profiles: Vec<Profile>,
}

impl Notes {
    /// The section a fresh session's instructions end with
    pub fn render(&self, today: NaiveDate) -> String {
        let mut text = format!(
            "[MEMORY NOTES] What {DISCORD_BOT_NAME} knows about this channel and the people in the \
             conversation. Today is {today}; dates carry their distance from it."
        );
        if self.lore.is_none() && self.profiles.is_empty() {
            text.push_str(" Nothing is known yet.");
        }
        if let Some(lore) = &self.lore {
            text.push_str("\n\n");
            text.push_str(&lore_section(lore, today));
        }
        for profile in &self.profiles {
            text.push_str("\n\n");
            text.push_str(&profile_section(profile, today));
        }
        text
    }

    /// The profiles of people who joined a conversation in progress, as a message of their own.
    /// `None` when none of them has one.
    pub fn render_newcomers(&self, today: NaiveDate) -> Option<String> {
        if self.profiles.is_empty() {
            return None;
        }
        let mut text = format!(
            "[MEMORY NOTES] Profiles of people who just joined the conversation. Today is {today}."
        );
        for profile in &self.profiles {
            text.push_str("\n\n");
            text.push_str(&profile_section(profile, today));
        }
        Some(text)
    }
}

fn lore_section(lore: &Note, today: NaiveDate) -> String {
    format!(
        "<channel_lore updated=\"{}\">\n{}\n</channel_lore>",
        lore.written_at.date_naive(),
        mark_dates(&lore.content, today)
    )
}

fn profile_section(profile: &Profile, today: NaiveDate) -> String {
    format!(
        "<profile of=\"{}\" user_id=\"{}\" updated=\"{}\">\n{}\n</profile>",
        profile.name.as_deref().unwrap_or("unknown"),
        profile.user_id.get(),
        profile.note.written_at.date_naive(),
        mark_dates(&profile.note.content, today)
    )
}

/// Follows each date (2026-07-14) or month (2026-07) in `text` with how far it is from `today`,
/// "2026-07 (2 months ago)", since models are unreliable at date arithmetic. Timestamps and
/// digits that run on are left alone.
fn mark_dates(text: &str, today: NaiveDate) -> String {
    let bytes = text.as_bytes();
    let mut marked = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        let starts_token = i == 0
            || bytes
                .get(i - 1)
                .is_some_and(|b| !b.is_ascii_alphanumeric() && *b != b'-');
        if starts_token
            && let Some((len, distance)) = date_at(bytes.get(i..).unwrap_or_default(), today)
        {
            let end = i + len;
            // Dates are ASCII, so both ends sit on character boundaries
            marked.push_str(text.get(copied..end).unwrap_or_default());
            marked.push_str(&format!(" ({distance})"));
            copied = end;
            i = end;
        } else {
            i += 1;
        }
    }
    marked.push_str(text.get(copied..).unwrap_or_default());
    marked
}

/// The length of the date `bytes` start with and its distance from `today`
fn date_at(bytes: &[u8], today: NaiveDate) -> Option<(usize, String)> {
    let number = |range: std::ops::Range<usize>| -> Option<i32> {
        let digits = bytes.get(range)?;
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(digits).ok()?.parse().ok()
    };
    let byte = |at: usize| bytes.get(at).copied();
    let ends_at = |at: usize| byte(at).is_none_or(|b| !b.is_ascii_alphanumeric() && b != b'-');

    let year = number(0..4).filter(|y| (2000..=2100).contains(y))?;
    if byte(4) != Some(b'-') {
        return None;
    }
    let month = number(5..7).filter(|m| (1..=12).contains(m))?;

    if byte(7) == Some(b'-')
        && let Some(day) = number(8..10)
        && ends_at(10)
    {
        let date = NaiveDate::from_ymd_opt(year, month as u32, day as u32)?;
        return Some((10, days_from(today, date)));
    }
    if ends_at(7) {
        let months = (year * 12 + month) - (today.year() * 12 + today.month() as i32);
        return Some((7, months_from(months)));
    }
    None
}

fn days_from(today: NaiveDate, date: NaiveDate) -> String {
    let days = (date - today).num_days();
    let span = days.unsigned_abs();
    let amount = match span {
        0 => return "today".to_string(),
        1 if days < 0 => return "yesterday".to_string(),
        1 => return "tomorrow".to_string(),
        2..14 => count(span, "day"),
        14..60 => count(span / 7, "week"),
        60..730 => count(span / 30, "month"),
        _ => count(span / 365, "year"),
    };
    relative(days < 0, amount)
}

fn months_from(months: i32) -> String {
    let span = months.unsigned_abs() as u64;
    let amount = match span {
        0 => return "this month".to_string(),
        1 if months < 0 => return "last month".to_string(),
        1 => return "next month".to_string(),
        2..24 => count(span, "month"),
        _ => count(span / 12, "year"),
    };
    relative(months < 0, amount)
}

fn count(n: u64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

fn relative(past: bool, amount: String) -> String {
    if past {
        format!("{amount} ago")
    } else {
        format!("in {amount}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("valid date")
    }

    #[test]
    fn dates_carry_their_distance_from_today() {
        let today = day("2026-09-28");
        assert_eq!(
            mark_dates("went to Đà Lạt on 2026-07-14, back 2026-09-27", today),
            "went to Đà Lạt on 2026-07-14 (2 months ago), back 2026-09-27 (yesterday)"
        );
        assert_eq!(
            mark_dates("trip 2026-10-04; exam 2026-10; moved 2024-01", today),
            "trip 2026-10-04 (in 6 days); exam 2026-10 (next month); moved 2024-01 (2 years ago)"
        );
        assert_eq!(
            mark_dates("today 2026-09-28.", today),
            "today 2026-09-28 (today)."
        );
    }

    #[test]
    fn things_that_only_look_like_dates_stay_put() {
        let today = day("2026-09-28");
        for text in [
            "at 2026-09-14T12:03:30Z",
            "id 12026-09-14",
            "v1-2026-09",
            "2026-13 is no month",
            "1999-01 is out of range",
            "2026-02-30 is no day",
            "2026-091",
        ] {
            assert_eq!(mark_dates(text, today), text, "{text}");
        }
    }

    #[test]
    fn notes_render_as_tagged_sections() {
        let written_at = DateTime::parse_from_rfc3339("2026-09-20T10:00:00Z")
            .expect("valid time")
            .with_timezone(&Utc);
        let notes = Notes {
            lore: Some(Note {
                content: "Running joke since 2026-08: the bot is \"thằng ngáo\"".to_string(),
                written_at,
            }),
            profiles: vec![Profile {
                user_id: UserId::new(350884319360712705),
                name: Some("wonrax".to_string()),
                note: Note {
                    content: "- Vegetarian".to_string(),
                    written_at,
                },
            }],
        };
        let text = notes.render(day("2026-09-28"));
        assert!(text.starts_with("[MEMORY NOTES] What "), "{text}");
        assert!(
            text.contains(
                "<channel_lore updated=\"2026-09-20\">\nRunning joke since 2026-08 (last month)"
            ),
            "{text}"
        );
        assert!(
            text.ends_with(
                "<profile of=\"wonrax\" user_id=\"350884319360712705\" updated=\"2026-09-20\">\n- Vegetarian\n</profile>"
            ),
            "{text}"
        );

        assert!(
            Notes::default()
                .render(day("2026-09-28"))
                .ends_with("Nothing is known yet.")
        );
        assert!(
            Notes::default()
                .render_newcomers(day("2026-09-28"))
                .is_none()
        );
    }
}
