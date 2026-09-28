//! Distills the observation log into the memory docs in the background

use std::collections::HashMap;

use chrono::{NaiveDate, Utc};
use rig_agent::{AgentBuilder, completion::Prompt as _};
use serenity::all::GuildId;
use tokio::{sync::mpsc, time::Instant};
use tracing::Instrument as _;

use super::{
    MemoryStore, Subject,
    store::{Doc, DreamInput, DreamObservation},
};
use crate::discord::{
    agent::LlmBackend,
    chatgpt::Grant,
    constants::{DREAM_PROMPT, DREAM_SETTLE, DREAM_SWEEP_INTERVAL},
};

/// Sizes past which a doc is worth a look in the logs, in characters. Not limits: the dream
/// prompt keeps the docs lean by judgment.
const PROFILE_SIZE_NOTICE: usize = 6_000;
const LORE_SIZE_NOTICE: usize = 30_000;
/// A rewrite longer than this is taken for a runaway and dropped, keeping the doc as it was
const DOC_SIZE_LIMIT: usize = 100_000;

/// Keeps the memory docs up to date. New observations wake it, and it dreams once the recording
/// has settled for `DREAM_SETTLE`; a sweep every `DREAM_SWEEP_INTERVAL`, and one at startup,
/// catches the docs whose revisit date has come and the dreams that failed.
#[derive(Clone)]
pub struct Dreamer {
    wake: mpsc::UnboundedSender<GuildId>,
}

impl Dreamer {
    pub fn spawn(store: MemoryStore, llm: LlmBackend) -> Self {
        let (wake, woken) = mpsc::unbounded_channel();
        tokio::spawn(run(store, llm, woken).instrument(tracing::info_span!("memory_dreamer")));
        Self { wake }
    }

    /// Asks for the docs of `guild_id` to catch up with what was just recorded there
    pub fn wake(&self, guild_id: GuildId) {
        if self.wake.send(guild_id).is_err() {
            tracing::error!(
                "The memory dreamer has stopped; the docs won't catch up until a restart"
            );
        }
    }
}

async fn run(store: MemoryStore, llm: LlmBackend, mut woken: mpsc::UnboundedReceiver<GuildId>) {
    // When each woken server's recording has settled
    let mut due: HashMap<GuildId, Instant> = HashMap::new();
    let mut sweep = tokio::time::interval(DREAM_SWEEP_INTERVAL);
    loop {
        let next = due.values().min().copied();
        tokio::select! {
            guild_id = woken.recv() => match guild_id {
                Some(guild_id) => {
                    due.insert(guild_id, Instant::now() + DREAM_SETTLE);
                }
                None => break,
            },
            _ = tokio::time::sleep_until(next.unwrap_or_else(Instant::now)), if next.is_some() => {
                let now = Instant::now();
                let settled: Vec<GuildId> = due
                    .iter()
                    .filter(|(_, at)| **at <= now)
                    .map(|(guild_id, _)| *guild_id)
                    .collect();
                for guild_id in settled {
                    due.remove(&guild_id);
                    dream(&store, &llm, Some(guild_id)).await;
                }
            }
            _ = sweep.tick() => dream(&store, &llm, None).await,
        }
    }
}

/// Rewrites every doc due a dream, in `guild_id` or in every server
async fn dream(store: &MemoryStore, llm: &LlmBackend, guild_id: Option<GuildId>) {
    let today = Utc::now().date_naive();
    let subjects = match store.pending_subjects(guild_id, today).await {
        Ok(subjects) => subjects,
        Err(e) => {
            tracing::error!(?e, "Failed to find the memory docs due a dream");
            return;
        }
    };
    if subjects.is_empty() {
        return;
    }

    let grant = match llm {
        LlmBackend::Chatgpt(auth) => match auth.access().await {
            Ok(grant) => Some(grant),
            Err(_) => {
                // The next wake or sweep finds the same docs still due
                tracing::warn!(
                    due = subjects.len(),
                    "Postponing the memory dreams: ChatGPT has no sign-in"
                );
                return;
            }
        },
        LlmBackend::Gemini { .. } => None,
    };

    for (guild_id, subject) in subjects {
        if let Err(e) = dream_doc(store, llm, grant.as_ref(), guild_id, subject, today).await {
            tracing::error!(
                ?e,
                guild_id = guild_id.get(),
                ?subject,
                "Failed to dream a memory doc"
            );
        }
    }
}

async fn dream_doc(
    store: &MemoryStore,
    llm: &LlmBackend,
    grant: Option<&Grant>,
    guild_id: GuildId,
    subject: Subject,
    today: NaiveDate,
) -> eyre::Result<()> {
    let input = store.dream_input(guild_id, subject).await?;
    let model = llm.dreamer_model(guild_id, grant)?;
    let agent = AgentBuilder::from_model_handle(model)
        .preamble(DREAM_PROMPT)
        .build();
    let answer = agent.prompt(dream_prompt(subject, &input, today)).await?;
    let dreamed = parse_dream(&answer)
        .ok_or_else(|| eyre::eyre!("The dream answered without a <doc>: {answer:.300}"))?;

    let size = dreamed.content.chars().count();
    if size > DOC_SIZE_LIMIT {
        eyre::bail!("The dream wrote {size} characters, past the {DOC_SIZE_LIMIT} limit");
    }
    let notice = match subject {
        Subject::User(_) => PROFILE_SIZE_NOTICE,
        Subject::Channel(_) => LORE_SIZE_NOTICE,
    };
    if size > notice {
        tracing::warn!(
            guild_id = guild_id.get(),
            ?subject,
            size,
            "A memory doc is growing large"
        );
    }

    // A revisit that isn't ahead would have the doc dreamed at every sweep
    let revisit_on = dreamed.revisit_on.filter(|date| *date > today);
    let changed = store
        .save_doc(
            guild_id,
            subject,
            input.doc.as_ref(),
            &dreamed.content,
            input.dreamed_through(),
            revisit_on,
        )
        .await?;
    tracing::info!(
        guild_id = guild_id.get(),
        ?subject,
        changed,
        size,
        observations = input.observations.len(),
        ?revisit_on,
        "Dreamed a memory doc"
    );
    Ok(())
}

/// The request to rewrite `subject`'s doc from `input`
fn dream_prompt(subject: Subject, input: &DreamInput, today: NaiveDate) -> String {
    let mut prompt = match subject {
        Subject::User(user_id) => format!(
            "Profile of {} (user ID {user_id}).",
            input
                .name
                .as_deref()
                .unwrap_or("a user whose name wasn't seen")
        ),
        Subject::Channel(channel_id) => format!("Lore of channel {channel_id}."),
    };
    prompt.push_str(&format!(" Today is {today}.\n\n"));
    match &input.doc {
        Some(Doc {
            content,
            written_at,
            ..
        }) => prompt.push_str(&format!(
            "The doc as written on {}:\n<doc>\n{content}\n</doc>\n\n",
            written_at.date_naive()
        )),
        None => prompt.push_str("There is no doc yet.\n\n"),
    }
    let (withdrawn, recorded): (Vec<&DreamObservation>, Vec<&DreamObservation>) =
        input.observations.iter().partition(|o| o.withdrawn);
    if recorded.is_empty() && withdrawn.is_empty() {
        prompt.push_str("Nothing was recorded since; the doc is due for today's date.");
    }
    if !recorded.is_empty() {
        prompt.push_str("Observations recorded since, oldest first:");
        push_observations(&mut prompt, &recorded);
    }
    if !withdrawn.is_empty() {
        if !recorded.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str(
            "Withdrawn since, because the messages they came from were deleted; take out what \
             rests on them alone:",
        );
        push_observations(&mut prompt, &withdrawn);
    }
    prompt
}

fn push_observations(prompt: &mut String, observations: &[&DreamObservation]) {
    for observation in observations {
        prompt.push_str(&format!("\n- {}", observation.observed_at.date_naive()));
        if !observation.about.is_empty() {
            prompt.push_str(&format!(", about {}", observation.about.join(", ")));
        }
        prompt.push_str(": ");
        prompt.push_str(&observation.content);
    }
}

struct Dreamed {
    content: String,
    revisit_on: Option<NaiveDate>,
}

/// The doc between the answer's last `<doc>` tags, and the `REVISIT: YYYY-MM-DD` line after it
fn parse_dream(answer: &str) -> Option<Dreamed> {
    let start = answer.rfind("<doc>")? + "<doc>".len();
    let rest = answer.get(start..)?;
    let end = rest.find("</doc>")?;
    let content = rest.get(..end)?.trim().to_string();
    let revisit_on = rest
        .get(end + "</doc>".len()..)
        .unwrap_or_default()
        .lines()
        .find_map(|line| {
            let value = line.trim().trim_matches('`').strip_prefix("REVISIT:")?;
            NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok()
        });
    Some(Dreamed {
        content,
        revisit_on,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dreams_answer_with_a_doc_and_a_revisit_date() {
        let dreamed = parse_dream(
            "Here it is.\n<doc>\n- Hates cilantro\n- Going to Đà Lạt on 2026-10-04\n</doc>\n`REVISIT: 2026-10-05`",
        )
        .expect("a doc");
        assert_eq!(
            dreamed.content,
            "- Hates cilantro\n- Going to Đà Lạt on 2026-10-04"
        );
        assert_eq!(dreamed.revisit_on, NaiveDate::from_ymd_opt(2026, 10, 5));

        let dreamed = parse_dream("<doc>\n</doc>\nREVISIT: none").expect("an empty doc");
        assert_eq!(dreamed.content, "");
        assert_eq!(dreamed.revisit_on, None);

        assert!(parse_dream("I couldn't decide").is_none());
        assert!(parse_dream("<doc>never closed").is_none());
    }
}
