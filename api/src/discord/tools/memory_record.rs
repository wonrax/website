//! `record_observations`: appends what an agent learned to the memory log of the channel's server

use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU64,
    sync::Arc,
};

use rig::tool::PortableTool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::{Context, GetMessages, Message, MessageId, ReactionType, User, UserId};
use thiserror::Error;

use crate::discord::{
    memory::{ChannelMemory, NewObservation},
    message::parse_message_ids,
};

const RECORDED_REACTION: &str = "🧠";
/// Messages fetched around each cited one. People split a thought over several messages, so who
/// it's about may only show in the ones next to it.
const SOURCE_CONTEXT: u8 = 10;

#[derive(Clone)]
pub struct RecordObservationsTool {
    pub ctx: Arc<Context>,
    pub memory: ChannelMemory,
    pub bot_user_id: UserId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordObservationsArgs {
    pub observations: Vec<ObservationArgs>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationArgs {
    pub content: String,
    #[serde(default)]
    pub about: Vec<String>,
    #[serde(default)]
    pub keywords: String,
    #[serde(default)]
    pub source_message_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordObservationsOutput {
    pub recorded: usize,
    /// One per observation that wasn't recorded, naming it by its position
    pub errors: Vec<String>,
}

#[derive(Debug, Error)]
#[error("Record observations error: {0}")]
pub struct RecordObservationsError(String);

impl PortableTool for RecordObservationsTool {
    const NAME: &'static str = "record_observations";
    type Error = RecordObservationsError;
    type Args = RecordObservationsArgs;
    type Output = RecordObservationsOutput;

    fn description(&self) -> String {
        "Add observations to the bot's memory log, which the memory notes are periodically \
         rewritten from. An observation is something learned about the people of this server or \
         about this channel that outlasts the conversation it came up in: who someone is, what's \
         going on in their life, what they like, hate, and believe, how they get along, what they \
         want from the bot, the group's running jokes, and anything someone asks the bot to \
         remember. The conversation itself isn't one: what was asked, and what the bot answered \
         or made, stay in the channel's history. Neither is a note that something isn't so: the \
         memory knows only what's recorded, so what's untrue stays out by not being recorded, \
         and a note denying it puts it in. Search the log first and record only what it lacks, \
         or how things stand now when something it holds has changed or proved wrong. Each \
         recorded observation reacts 🧠 to the newest message it cites."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "observations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "The fact as it stands, self-contained enough to make sense a year from now: name the people, keep what someone claimed apart from what is known, and turn relative times into dates (\"next Saturday\" in a message sent 2026-09-24 is 2026-09-26). The day it was observed is recorded with it."
                            },
                            "about": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "The people it is about, by username as the message headers show it or by user ID; it files the observation under their profiles. Empty when it is about the channel as a whole."
                            },
                            "keywords": {
                                "type": "string",
                                "description": "More words search should find it by: nicknames, other spellings, and the key terms in the other language the channel speaks."
                            },
                            "source_message_ids": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "IDs from the [#ID] headers of the messages it comes from, at least one. Later sessions reread the conversation around them."
                            }
                        },
                        "required": ["content", "about", "keywords", "source_message_ids"]
                    }
                }
            },
            "required": ["observations"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let mut errors = Vec::new();

        // The cited messages and the conversation around them, fetched once however many
        // observations cite them
        let mut conversation: HashMap<MessageId, Message> = HashMap::new();
        let mut unavailable: HashMap<MessageId, String> = HashMap::new();
        let cited: Vec<Result<Vec<MessageId>, String>> = args
            .observations
            .iter()
            .map(|o| parse_message_ids("source_message_ids", &o.source_message_ids))
            .collect();
        for id in cited.iter().flatten().flatten() {
            if conversation.contains_key(id) || unavailable.contains_key(id) {
                continue;
            }
            let around = GetMessages::new().around(*id).limit(SOURCE_CONTEXT);
            match self
                .memory
                .channel_id
                .messages(&self.ctx.http, around)
                .await
            {
                Ok(messages) => {
                    let found = messages.iter().any(|m| m.id == *id);
                    for message in messages {
                        conversation.entry(message.id).or_insert(message);
                    }
                    if !found {
                        unavailable.insert(
                            *id,
                            format!("message {id} isn't in this channel or was deleted"),
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(?e, message_id = id.get(), "Failed to fetch a cited message");
                    unavailable.insert(*id, format!("could not fetch message {id}: {e}"));
                }
            }
        }
        let names = NameIndex::new(conversation.values());

        let mut people: HashMap<UserId, String> = HashMap::new();
        let mut recorded: Vec<(NewObservation, MessageId)> = Vec::new();
        for (position, (observation, cited)) in args.observations.into_iter().zip(cited).enumerate()
        {
            let number = position + 1;
            match self
                .prepare(
                    observation,
                    cited,
                    &conversation,
                    &unavailable,
                    &names,
                    &mut people,
                )
                .await
            {
                Ok(prepared) => recorded.push(prepared),
                Err(error) => errors.push(format!("observation {number}: {error}")),
            }
        }
        if recorded.is_empty() {
            return Ok(RecordObservationsOutput {
                recorded: 0,
                errors,
            });
        }

        let (observations, newest): (Vec<NewObservation>, HashSet<MessageId>) =
            recorded.into_iter().unzip();
        if let Err(e) = self
            .memory
            .store
            .record(
                self.memory.guild_id,
                self.memory.channel_id,
                &people,
                &observations,
            )
            .await
        {
            tracing::error!(?e, "record_observations failed");
            errors.push(format!("Nothing was recorded, the database failed: {e}"));
            return Ok(RecordObservationsOutput {
                recorded: 0,
                errors,
            });
        }
        self.memory.dreamer.wake(self.memory.guild_id);
        tracing::info!(
            recorded = observations.len(),
            rejected = errors.len(),
            channel_id = self.memory.channel_id.get(),
            "record_observations completed"
        );

        for message_id in newest {
            if let Err(e) = self
                .memory
                .channel_id
                .create_reaction(
                    &self.ctx.http,
                    message_id,
                    ReactionType::Unicode(RECORDED_REACTION.to_string()),
                )
                .await
            {
                tracing::warn!(
                    ?e,
                    message_id = message_id.get(),
                    "Failed to react to a recorded message"
                );
            }
        }

        Ok(RecordObservationsOutput {
            recorded: observations.len(),
            errors,
        })
    }
}

impl RecordObservationsTool {
    /// The observation ready for the log, with the newest message it cites
    async fn prepare(
        &self,
        observation: ObservationArgs,
        cited: Result<Vec<MessageId>, String>,
        conversation: &HashMap<MessageId, Message>,
        unavailable: &HashMap<MessageId, String>,
        names: &NameIndex,
        people: &mut HashMap<UserId, String>,
    ) -> Result<(NewObservation, MessageId), String> {
        let content = observation.content.trim().to_string();
        if content.is_empty() {
            return Err("content is empty".to_string());
        }
        let cited = cited?;
        if let Some(error) = cited.iter().find_map(|id| unavailable.get(id)) {
            return Err(error.clone());
        }
        let Some(newest) = cited
            .iter()
            .filter_map(|id| conversation.get(id))
            .max_by_key(|m| m.id)
        else {
            return Err("source_message_ids is empty; cite the messages it comes from".to_string());
        };

        let mut about = Vec::new();
        for name in &observation.about {
            let user_id = self.resolve(name, names, people).await?;
            // What's about the bot itself is the channel's lore
            if user_id != self.bot_user_id && !about.contains(&user_id) {
                about.push(user_id);
            }
        }

        let observed_at = chrono::DateTime::from_timestamp(newest.timestamp.unix_timestamp(), 0)
            .unwrap_or_else(chrono::Utc::now);
        Ok((
            NewObservation {
                about,
                content,
                keywords: observation.keywords.trim().to_string(),
                source_message_ids: cited,
                observed_at,
            },
            newest.id,
        ))
    }

    /// The user an `about` entry names: a user ID, someone in the conversation around the cited
    /// messages, or someone the memory already knows in this server
    async fn resolve(
        &self,
        name: &str,
        names: &NameIndex,
        people: &mut HashMap<UserId, String>,
    ) -> Result<UserId, String> {
        let name = name.trim().trim_start_matches('@');
        if let Some(user_id) = name.parse::<u64>().ok().and_then(NonZeroU64::new) {
            let user_id = UserId::from(user_id);
            // Its name is kept when the conversation shows it
            if let Some(user) = names.find_id(user_id) {
                people.insert(user.id, user.name.clone());
            }
            return Ok(user_id);
        }
        if let Some(user) = names.find(name) {
            people.insert(user.id, user.name.clone());
            return Ok(user.id);
        }
        match self
            .memory
            .store
            .find_person(self.memory.guild_id, name)
            .await
        {
            Ok(Some(user_id)) => Ok(user_id),
            Ok(None) => Err(format!(
                "no one called {name:?} wrote or was mentioned around the cited messages, and the memory doesn't know them; cite a message of theirs or give their user ID"
            )),
            Err(e) => Err(format!("could not look up {name:?}: {e}")),
        }
    }
}

/// The users of a set of messages by ID and by every name they go by there: authors, the people
/// they mention, and the authors they reply to
struct NameIndex {
    by_name: HashMap<String, User>,
    by_id: HashMap<UserId, User>,
}

impl NameIndex {
    fn new<'a>(messages: impl Iterator<Item = &'a Message>) -> Self {
        let mut by_name = HashMap::new();
        let mut by_id = HashMap::new();
        for message in messages {
            let mut add = |user: &User, nick: Option<&str>| {
                for name in [Some(user.name.as_str()), user.global_name.as_deref(), nick]
                    .into_iter()
                    .flatten()
                {
                    by_name.insert(name.to_lowercase(), user.clone());
                }
                by_id.insert(user.id, user.clone());
            };
            add(
                &message.author,
                message.member.as_ref().and_then(|m| m.nick.as_deref()),
            );
            for user in &message.mentions {
                add(user, None);
            }
            if let Some(replied) = &message.referenced_message {
                add(&replied.author, None);
            }
        }
        Self { by_name, by_id }
    }

    fn find(&self, name: &str) -> Option<&User> {
        self.by_name.get(&name.to_lowercase())
    }

    fn find_id(&self, user_id: UserId) -> Option<&User> {
        self.by_id.get(&user_id)
    }
}
