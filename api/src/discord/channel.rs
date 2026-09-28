use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use futures::{
    SinkExt as _, StreamExt,
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
};
use rig::message::Message as RigMessage;
use rig_agent::completion::PromptError;
use serenity::all::{
    ChannelId, Context, GetMessages, GuildId, Message, MessageId, MessageUpdateEvent, Reaction,
    ReactionType, Typing, UserId,
};
use tokio::sync::watch;
use tracing::{Instrument as _, instrument};

use crate::discord::{
    agent::{self, AgentSession, LlmBackend, Memories, Role, Seed, Turn},
    bot::Guild,
    chatgpt::{self, AuthState, Grant},
    constants::{
        AGENT_SESSION_TIMEOUT, COMPACTION_KEPT_MESSAGES, COMPACTION_PROMPT, MEMORY_PASS_PROMPT,
        MESSAGE_CONTEXT_SIZE, MESSAGE_DEBOUNCE_TIMEOUT, TYPING_DEBOUNCE_TIMEOUT,
        WATCHER_SESSION_TIMEOUT,
    },
    memory::{ChannelMemory, MemorySystem},
    message::{self, AttachmentMode, QueuedMessage, discord_message_to_rig_message},
    tools,
};

/// Dual-timestamp activity tracker for proper debouncing
#[derive(Debug)]
struct ChannelActivity {
    /// When the last message occurred
    last_message: Option<Instant>,
    /// When the last typing event occurred
    last_typing: Option<Instant>,
}

impl ChannelActivity {
    fn new() -> Self {
        Self {
            last_message: None,
            last_typing: None,
        }
    }

    fn update_message(&mut self) {
        self.last_message = Some(Instant::now());
    }

    fn update_typing(&mut self) {
        self.last_typing = Some(Instant::now());
    }

    /// Calculate when we can next process messages
    /// We need both conditions satisfied:
    /// 1. Enough time passed since last message (`MESSAGE_DEBOUNCE_TIMEOUT`)
    /// 2. Enough time passed since last typing (`TYPING_DEBOUNCE_TIMEOUT`)
    fn next_processing_time(&self) -> Option<Instant> {
        let message_deadline = self.last_message.map(|t| t + MESSAGE_DEBOUNCE_TIMEOUT);
        let typing_deadline = self.last_typing.map(|t| t + TYPING_DEBOUNCE_TIMEOUT);

        match (message_deadline, typing_deadline) {
            (Some(m), Some(t)) => Some(m.max(t)),
            (Some(m), None) => Some(m),
            (None, Some(t)) => Some(t),
            (None, None) => None,
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum ChannelEvent {
    /// A new message has been received in the channel
    Message(QueuedMessage, Context),

    /// A typing event has been received in the channel
    Typing(UserId, Context),

    /// Request to immediately process the channel even if debounce timers haven't expired or there
    /// are no new messages. Useful for service startup when we want to process any awaiting
    /// messages right away. `addressed` sends it straight to the responder, as a message pinging
    /// the bot would; otherwise the watcher decides.
    ForceProcess { addressed: bool },

    /// Someone reacted to a message of the channel
    Reaction(Reaction, Context),

    /// Someone took a reaction back
    ReactionRemoved(Reaction),

    /// A message of the channel changed
    Edit(MessageUpdateEvent, Context),

    /// Messages of the channel were deleted
    Deletion(Vec<MessageId>),
}

/// A channel message waiting for the debounce to hand it to the agents
struct PendingMessage {
    message: RigMessage,
    /// As Discord sent it, to render again when it's edited before the batch goes out
    raw: Box<Message>,
    /// A user message that pings the bot or replies to it
    addresses_bot: bool,
    /// Who it brings into the conversation, for their memory profiles
    people: Vec<UserId>,
    /// The bot's own message. A live responder already holds it as a `send_discord_message` call,
    /// so it only reaches the responder when seeding a fresh session. The watcher reads it always.
    from_bot: bool,
}

/// Something that happened to a message the agents may have read: a reaction, an edit, or a
/// deletion. It reaches them with the next batch, appended like the messages so their cached
/// prefix stays intact, and never makes a batch on its own.
struct PendingEvent {
    /// When it happened, as a snowflake
    at: MessageId,
    /// How the agents read it
    line: String,
    /// The reaction it is, which taking back before the batch goes out cancels
    reaction: Option<ReactionKey>,
}

#[derive(PartialEq, Eq)]
struct ReactionKey {
    user_id: UserId,
    message_id: MessageId,
    emoji: ReactionType,
}

/// What waits in the queue for the debounce
enum Pending {
    Message(PendingMessage),
    Event(PendingEvent),
}

impl Pending {
    /// Where a fresh session's backfill stops when this opens the batch: the API only fills in
    /// what is older than the queue
    fn anchor(&self) -> MessageId {
        match self {
            Self::Message(message) => message.raw.id,
            Self::Event(event) => event.at,
        }
    }

    fn message(&self) -> Option<&PendingMessage> {
        match self {
            Self::Message(message) => Some(message),
            Self::Event(_) => None,
        }
    }
}

/// Messages and events a queue holds at most. Messages alone are capped at the context window;
/// this keeps a reaction spree from growing it without bound.
const MAX_QUEUED: usize = 3 * MESSAGE_CONTEXT_SIZE;

/// Recent messages the agents may hold, by ID, to name them in what happens to them
#[derive(Default)]
struct KnownMessages(BTreeMap<MessageId, KnownMessage>);

struct KnownMessage {
    author: String,
    /// As the agents last read it
    content: String,
}

/// Messages `KnownMessages` remembers, the oldest forgotten first. A session can outgrow it, but
/// what happens to messages that old rarely matters to the conversation.
const MAX_KNOWN_MESSAGES: usize = 1000;

impl KnownMessages {
    fn add(&mut self, msg: &Message) {
        self.0.insert(
            msg.id,
            KnownMessage {
                author: msg.author.name.clone(),
                content: msg.content.clone(),
            },
        );
        while self.0.len() > MAX_KNOWN_MESSAGES {
            self.0.pop_first();
        }
    }
}

/// What a batch's runs authenticate with
struct RunAuth {
    /// The ChatGPT token; `None` on other backends
    grant: Option<Grant>,
}

enum RunError {
    /// ChatGPT has no usable sign-in any more, and the channel has been told
    SignedOut,
    Failed(PromptError),
}

struct ChannelState {
    activity: ChannelActivity,
    event_recv: UnboundedReceiver<ChannelEvent>,
    /// Writes the replies
    responder: Option<AgentSession>,
    /// Reads every message, decides when the responder runs, and keeps the memories. Never
    /// started in mention-only mode.
    watcher: Option<AgentSession>,
    /// When the watcher last got messages. Once the channel stays quiet for
    /// `WATCHER_SESSION_TIMEOUT`, the conversation is over: the watcher updates the memories from
    /// it and its session ends.
    watcher_last_input: Option<Instant>,
    /// ChatGPT has no usable sign-in, and the channel has said so. Runs wait for the sign-in to
    /// change, or for a new message asking for another try.
    awaiting_auth: bool,
    /// The responder lost its sign-in mid-run. The prompt it didn't get to is still the newest
    /// entry of its session, and the next processing runs it regardless of what the batch holds.
    unanswered: bool,
    /// ChatGPT sign-in changes; `None` on other backends
    auth_updates: Option<watch::Receiver<AuthState>>,
    /// When the responder last finished a run. Its session expires once the channel goes
    /// `AGENT_SESSION_TIMEOUT` without one, however much the users chat in between. `None`, and
    /// so never expiring, while an `unanswered` run waits for its retry.
    last_run_finished: Option<Instant>,

    llm: LlmBackend,
    memory: MemorySystem,
    firecrawl: Option<tools::Firecrawl>,
    /// The server the channel belongs to, which keeps its memories. Looked up once.
    guild_id: Option<GuildId>,

    // The latest discord context received from the event handler.
    // Note that each discord context is bound to a specific event and is destroyed after event
    // handler completes, so we should not rely on it being valid forever.
    discord_ctx: Context,
    bot_user_id: UserId,
    channel_id: ChannelId,
    // All guilds the bot is in
    guilds: Arc<scc::HashMap<serenity::model::id::GuildId, Guild>>,

    // Only respond when a message addresses the bot, and run no watcher: nothing costs a model
    // call until then. Incoming messages still queue up for context.
    discord_bot_mention_only: bool,

    /// Every message of the channel, the bot's own included, waits here until the debounce
    /// expires, along with what happens to messages meanwhile. It is the only way into the agents
    /// for anything newer than its oldest entry: a fresh session backfills strictly older
    /// messages from the API and appends the queue behind them, so nothing can reach an agent
    /// twice.
    message_queue: Vec<Pending>,
    /// The messages the agents may hold, for the events about them
    known: KnownMessages,
}

impl ChannelState {
    /// Up to `count` channel messages from right before `before` (the newest when `None`), oldest
    /// first. Callers anchor on the oldest message they are about to add, which keeps a message
    /// that lands during the fetch out of the result; it reaches the agent through the queue
    /// instead. Attachments stay placeholders, the agent opens them on demand. Comes with the
    /// people the messages bring into the conversation.
    #[instrument(skip(self))]
    async fn backfill_history(
        &mut self,
        before: Option<MessageId>,
        count: usize,
    ) -> (Vec<RigMessage>, HashSet<UserId>) {
        if count == 0 {
            return (vec![], HashSet::new());
        }

        // Fetch a full window rather than `count` so empty messages (stickers, bare embeds)
        // don't eat into what the agent gets to see
        let mut page = GetMessages::new().limit(MESSAGE_CONTEXT_SIZE as u8);
        if let Some(before) = before {
            page = page.before(before);
        }
        let messages = match self.channel_id.messages(&self.discord_ctx.http, page).await {
            Ok(messages) => messages,
            Err(e) => {
                tracing::error!(
                    ?e,
                    "Failed to backfill channel history; seeding the session from the queue alone"
                );
                return (vec![], HashSet::new());
            }
        };

        // Newest first from the API
        let mut history = Vec::with_capacity(count);
        let mut people = HashSet::new();
        for msg in messages
            .iter()
            .filter(|m| !m.content.trim().is_empty() || !m.attachments.is_empty())
            .take(count)
        {
            people.extend(message::participants(msg, self.bot_user_id));
            self.known.add(msg);
            history.push(
                discord_message_to_rig_message(
                    msg,
                    self.bot_user_id,
                    None,
                    AttachmentMode::Placeholder,
                )
                .await,
            );
        }
        history.reverse();
        (history, people)
    }

    /// The server the channel belongs to, looked up the first time it's needed
    async fn resolve_guild(&mut self) {
        if self.guild_id.is_some() {
            return;
        }
        match self.channel_id.to_channel(&self.discord_ctx.http).await {
            Ok(channel) => self.guild_id = channel.guild().map(|channel| channel.guild_id),
            Err(e) => tracing::error!(
                ?e,
                "Failed to look up the channel's server; no memories this time"
            ),
        }
    }

    /// A message as the agents read it, with its author's presence in the server
    async fn render(&self, msg: &Message) -> RigMessage {
        // Read up front: the cache entry stays locked while it's held
        let presence = self
            .guild_id
            .and_then(|guild_id| self.guilds.get_sync(&guild_id))
            .and_then(|guild| message::presence(msg.author.id, guild.get()));
        discord_message_to_rig_message(
            msg,
            self.bot_user_id,
            presence.as_deref(),
            AttachmentMode::Inline,
        )
        .await
    }

    /// Queues a channel message
    async fn queue_message(&mut self, msg: Message) {
        let from_bot = msg.author.id == self.bot_user_id;
        let addresses_bot = !from_bot && message::addresses(&msg, self.bot_user_id);
        let message = self.render(&msg).await;
        self.known.add(&msg);
        self.enqueue(Pending::Message(PendingMessage {
            message,
            addresses_bot,
            people: message::participants(&msg, self.bot_user_id),
            from_bot,
            raw: Box::new(msg),
        }));
    }

    fn enqueue(&mut self, entry: Pending) {
        self.message_queue.push(entry);
        trim_queue(&mut self.message_queue);
    }

    fn queue_event(&mut self, line: String, reaction: Option<ReactionKey>) {
        self.enqueue(Pending::Event(PendingEvent {
            at: message::snowflake_now(),
            line,
            reaction,
        }));
    }

    /// Queues a reaction. The bot's own are left out: the responder reacts through a tool, and
    /// the rest are the memory's 🧠 marks.
    async fn queue_reaction(&mut self, reaction: Reaction) {
        let Some(user_id) = reaction.user_id.filter(|id| *id != self.bot_user_id) else {
            return;
        };
        let reactor = match &reaction.member {
            Some(member) => member.user.name.clone(),
            None => match user_id.to_user(&self.discord_ctx).await {
                Ok(user) => user.name,
                Err(e) => {
                    tracing::warn!(?e, "Failed to look up who reacted");
                    return;
                }
            },
        };
        let line = message::reaction_line(
            &reactor,
            &reaction.emoji,
            self.known
                .0
                .get(&reaction.message_id)
                .map(|m| m.author.as_str()),
            reaction.message_id,
        );
        self.queue_event(
            line,
            Some(ReactionKey {
                user_id,
                message_id: reaction.message_id,
                emoji: reaction.emoji,
            }),
        );
    }

    /// Drops a queued reaction taken back before the batch goes out. One the agents already read
    /// stays: taking a reaction back rarely changes what the conversation means.
    fn unqueue_reaction(&mut self, reaction: Reaction) {
        let Some(user_id) = reaction.user_id else {
            return;
        };
        let key = ReactionKey {
            user_id,
            message_id: reaction.message_id,
            emoji: reaction.emoji,
        };
        if let Some(position) = self.message_queue.iter().position(
            |entry| matches!(entry, Pending::Event(event) if event.reaction.as_ref() == Some(&key)),
        ) {
            self.message_queue.remove(position);
        }
    }

    /// A queued message is rendered again, and the edit of one the agents may hold is queued.
    /// Updates that leave the text alone, like Discord adding a link preview, are no edit.
    async fn queue_edit(&mut self, update: MessageUpdateEvent) {
        let Some(content) = update.content.clone() else {
            return;
        };

        let queued = self.message_queue.iter_mut().find_map(|entry| match entry {
            Pending::Message(queued) if queued.raw.id == update.id => Some(queued),
            _ => None,
        });
        if let Some(queued) = queued {
            if queued.raw.content == content {
                return;
            }
            update.apply_to_message(&mut queued.raw);
            let msg = Message::clone(&queued.raw);
            let from_bot = queued.from_bot;
            let message = self.render(&msg).await;
            self.known.add(&msg);
            // Looked up again, since rendering needed all of `self`
            if let Some(Pending::Message(queued)) = self
                .message_queue
                .iter_mut()
                .find(|entry| matches!(entry, Pending::Message(m) if m.raw.id == msg.id))
            {
                queued.message = message;
                queued.addresses_bot = !from_bot && message::addresses(&msg, self.bot_user_id);
                queued.people = message::participants(&msg, self.bot_user_id);
            }
            return;
        }

        let Some(known) = self.known.0.get_mut(&update.id) else {
            return;
        };
        if known.content == content {
            return;
        }
        known.content = content;
        let line = message::edit_line(
            &known.author,
            update.id,
            self.channel_id,
            &known.content,
            update.attachments.as_deref().unwrap_or_default(),
        );
        self.queue_event(line, None);
    }

    /// A queued message is dropped before the agents read it, and the deletion of one they may
    /// hold is queued
    fn queue_deletions(&mut self, message_ids: &[MessageId]) {
        for id in message_ids {
            let known = self.known.0.remove(id);
            if let Some(position) = self
                .message_queue
                .iter()
                .position(|entry| matches!(entry, Pending::Message(m) if m.raw.id == *id))
            {
                self.message_queue.remove(position);
            } else if let Some(known) = known {
                self.queue_event(message::deletion_line(&known.author, *id), None);
            }
        }
    }

    /// The channel's memories, once its server is known
    fn memory(&self) -> Option<ChannelMemory> {
        Some(self.memory.channel(self.guild_id?, self.channel_id))
    }

    /// What a fresh session starts from: `history`, and memory notes on the channel and `people`
    async fn seed(&self, history: Vec<RigMessage>, people: HashSet<UserId>) -> Seed {
        let Some(memory) = self.memory() else {
            return Seed {
                history,
                notes: String::new(),
                people: HashSet::new(),
            };
        };
        let wanted: Vec<UserId> = people.iter().copied().collect();
        match memory
            .store
            .notes(memory.guild_id, Some(self.channel_id), &wanted)
            .await
        {
            Ok(notes) => Seed {
                history,
                notes: notes.render(chrono::Utc::now().date_naive()),
                people,
            },
            Err(e) => {
                // Nobody counts as noted, so each person's profile gets another try when they speak
                tracing::error!(
                    ?e,
                    "Failed to load the memory notes; the session starts without them"
                );
                Seed {
                    history,
                    notes: String::new(),
                    people: HashSet::new(),
                }
            }
        }
    }

    /// Hands `session` the profiles of those of `people` it hasn't seen, in a message ahead of the
    /// ones that bring them in
    async fn note_newcomers(
        &self,
        session: &mut AgentSession,
        people: impl IntoIterator<Item = UserId>,
    ) {
        let Some(memory) = self.memory() else {
            return;
        };
        let newcomers: Vec<UserId> = people
            .into_iter()
            .filter(|id| session.noted_people.insert(*id))
            .collect();
        if newcomers.is_empty() {
            return;
        }
        match memory.store.notes(memory.guild_id, None, &newcomers).await {
            Ok(notes) => {
                if let Some(text) = notes.render_newcomers(chrono::Utc::now().date_naive()) {
                    session.add_messages(vec![RigMessage::user(text)]);
                }
            }
            Err(e) => {
                tracing::error!(
                    ?e,
                    "Failed to load the profiles of people joining the conversation"
                );
                // Another try when they next speak
                for id in &newcomers {
                    session.noted_people.remove(id);
                }
            }
        }
    }

    /// The sign-in for the next runs. `None` when ChatGPT has none usable, which the channel is
    /// then told about and waits out.
    async fn authorize(&mut self) -> Option<RunAuth> {
        let LlmBackend::Chatgpt(auth) = &self.llm else {
            return Some(RunAuth { grant: None });
        };
        let auth = auth.clone();
        // Taken as seen before looking, so a change during the checks still wakes us
        if let Some(updates) = self.auth_updates.as_mut() {
            updates.mark_unchanged();
        }
        match auth.access().await {
            Ok(grant) => Some(RunAuth { grant: Some(grant) }),
            Err(unavailable) => {
                self.awaiting_auth = true;
                auth.report_unavailable(&self.discord_ctx.http, self.channel_id, &unavailable)
                    .await;
                None
            }
        }
    }

    /// Runs `session`, and when ChatGPT refuses a token that should have been good, refreshes it
    /// and gives the run one more go
    async fn run_session(
        &mut self,
        session: &mut AgentSession,
        turn: Turn,
        role: Role,
        run_auth: &mut RunAuth,
    ) -> Result<String, RunError> {
        let result = session.run(turn).await;
        let (Err(e), LlmBackend::Chatgpt(auth), Some(grant)) =
            (&result, &self.llm, &run_auth.grant)
        else {
            return result.map_err(RunError::Failed);
        };
        if !chatgpt::is_unauthorized(e) {
            return result.map_err(RunError::Failed);
        }

        tracing::warn!("ChatGPT rejected the access token; refreshing and retrying");
        let auth = auth.clone();
        match auth.recover_from_unauthorized(grant).await {
            Ok(grant) => match self.llm.model(role, self.channel_id, Some(&grant)) {
                Ok(handle) => {
                    session.agent.set_model_handle(handle);
                    run_auth.grant = Some(grant);
                    session.run(turn).await.map_err(RunError::Failed)
                }
                Err(e) => {
                    tracing::error!(?e, "Failed to create the ChatGPT model");
                    result.map_err(RunError::Failed)
                }
            },
            Err(unavailable) => {
                self.awaiting_auth = true;
                auth.report_unavailable(&self.discord_ctx.http, self.channel_id, &unavailable)
                    .await;
                Err(RunError::SignedOut)
            }
        }
    }

    /// Summarizes `session` and starts it over from the summary, followed by the latest
    /// `COMPACTION_KEPT_MESSAGES` messages before `before`. Hands back the session as it was, or
    /// `None` when no summary came, leaving the caller to replace the session.
    async fn compact(
        &mut self,
        session: &mut AgentSession,
        role: Role,
        before: Option<MessageId>,
        run_auth: &mut RunAuth,
    ) -> Option<AgentSession> {
        tracing::info!(?role, "Compacting the session");
        session.add_messages(vec![RigMessage::user(COMPACTION_PROMPT)]);
        let summary = match self
            .run_session(session, Turn::TextOnly, role, run_auth)
            .await
        {
            Ok(summary) if !summary.trim().is_empty() => summary,
            Ok(_) => {
                tracing::warn!(?role, "Compaction produced no summary");
                return None;
            }
            Err(RunError::SignedOut) => return None,
            Err(RunError::Failed(e)) => {
                tracing::error!(?e, ?role, "Failed to summarize the session for compaction");
                return None;
            }
        };

        let (mut recent, people) = self
            .backfill_history(before, COMPACTION_KEPT_MESSAGES)
            .await;
        if matches!(role, Role::Watcher) {
            recent = recent.iter().map(message::observed).collect();
        }
        let finished = session.start_over(&summary, recent);
        self.note_newcomers(session, people).await;
        Some(finished)
    }

    /// Hands the queued messages to the agents. The watcher reads all of them; the responder
    /// runs when one addresses the bot or the watcher calls for it.
    async fn process(&mut self, forced_address: bool) {
        let Some(mut run_auth) = self.authorize().await else {
            return;
        };
        self.resolve_guild().await;

        let batch: Vec<Pending> = self.message_queue.drain(..).collect();
        let addressed = forced_address
            || std::mem::take(&mut self.unanswered)
            || batch
                .iter()
                .filter_map(Pending::message)
                .any(|m| m.addresses_bot);

        let respond = if self.discord_bot_mention_only {
            addressed
        } else {
            self.watch(&batch, addressed, &mut run_auth).await
        };

        if respond {
            self.respond(batch, &mut run_auth).await;
        } else {
            self.pass_to_responder(batch).await;
        }
    }

    /// Feeds the batch to the watcher and asks whether the responder should run. A batch that
    /// addresses the bot runs it regardless, so the watcher only reads that one.
    async fn watch(&mut self, batch: &[Pending], addressed: bool, run_auth: &mut RunAuth) -> bool {
        let before = batch.first().map(Pending::anchor);
        let model = match self
            .llm
            .model(Role::Watcher, self.channel_id, run_auth.grant.as_ref())
        {
            Ok(model) => model,
            Err(e) => {
                tracing::error!(?e, "Failed to create the watcher model");
                return addressed;
            }
        };

        let mut watcher = match self.watcher.take() {
            Some(mut watcher) if watcher.needs_compaction() => {
                watcher.agent.set_model_handle(model.clone());
                match self
                    .compact(&mut watcher, Role::Watcher, before, run_auth)
                    .await
                {
                    Some(finished) => {
                        self.spawn_memory_pass(finished);
                        watcher
                    }
                    None => {
                        self.spawn_memory_pass(watcher);
                        self.new_watcher(model, batch).await
                    }
                }
            }
            Some(mut watcher) => {
                watcher.agent.set_model_handle(model);
                watcher
            }
            None => self.new_watcher(model, batch).await,
        };

        self.note_newcomers(&mut watcher, batch_people(batch)).await;
        watcher.add_messages(observed(batch));
        self.watcher_last_input = Some(Instant::now());

        let respond = if addressed {
            true
        } else {
            match self
                .run_session(&mut watcher, Turn::TextOnly, Role::Watcher, run_auth)
                .await
            {
                Ok(answer) => {
                    let respond = wants_response(&answer);
                    tracing::info!(respond, answer = answer.trim(), "Watcher decided");
                    respond
                }
                Err(RunError::SignedOut) => false,
                Err(RunError::Failed(e)) => {
                    tracing::error!(?e, "Watcher run failed; staying quiet");
                    false
                }
            }
        };

        self.watcher = Some(watcher);
        respond
    }

    /// A watcher session seeded with the messages right before `batch`
    async fn new_watcher(
        &mut self,
        model: rig_agent::ModelHandle,
        batch: &[Pending],
    ) -> AgentSession {
        let (history, mut people) = self
            .backfill_history(
                batch.first().map(Pending::anchor),
                MESSAGE_CONTEXT_SIZE.saturating_sub(batch_messages(batch)),
            )
            .await;
        people.extend(batch_people(batch));
        let history = history.iter().map(message::observed).collect();
        agent::create_watcher_session(
            &self.discord_ctx,
            self.channel_id,
            &self.llm,
            model,
            self.memory(),
            self.seed(history, people).await,
        )
    }

    /// Ends the watcher's session once the channel has gone quiet, updating the memories from it
    async fn retire_watcher(&mut self) {
        let Some(mut watcher) = self.watcher.take() else {
            return;
        };
        self.watcher_last_input = None;
        if self.memory().is_none() {
            return;
        }

        // No sign-in prompt for this: nobody is waiting on it
        let grant = match &self.llm {
            LlmBackend::Chatgpt(auth) => match auth.access().await {
                Ok(grant) => Some(grant),
                Err(_) => {
                    // Kept for the next idle deadline, so the conversation's memories outlast
                    // the sign-in outage
                    tracing::warn!("Postponing the watcher's memory pass: ChatGPT has no sign-in");
                    self.watcher = Some(watcher);
                    self.watcher_last_input = Some(Instant::now());
                    return;
                }
            },
            LlmBackend::Gemini { .. } => None,
        };

        // Quiet means nobody else spoke, but the bot's last replies may still wait in the queue,
        // and so may what happened to the messages since
        watcher.add_messages(observed(&self.message_queue));
        match self
            .llm
            .model(Role::Watcher, self.channel_id, grant.as_ref())
        {
            Ok(model) => {
                watcher.agent.set_model_handle(model);
                self.spawn_memory_pass(watcher);
            }
            Err(e) => tracing::error!(?e, "Failed to create the watcher model for its memory pass"),
        }
    }

    /// Updates the memories from a finished watcher session in the background: nothing waits on
    /// it, and the channel's next watcher can't write memories until its own session ends
    fn spawn_memory_pass(&self, mut session: AgentSession) {
        if self.memory().is_none() {
            return;
        }
        tokio::spawn(
            async move {
                session.add_messages(vec![RigMessage::user(MEMORY_PASS_PROMPT)]);
                if let Err(e) = session.run(Turn::Agentic).await {
                    tracing::error!(?e, "Watcher memory pass failed");
                }
            }
            .instrument(tracing::info_span!(
                "watcher_memory_pass",
                channel_id = self.channel_id.get()
            )),
        );
    }

    /// Runs the responder over the batch. Typing shows from here on, compaction included: someone
    /// is waiting on the reply now, which isn't so while only the watcher runs.
    async fn respond(&mut self, batch: Vec<Pending>, run_auth: &mut RunAuth) {
        let typing = Typing::start(self.discord_ctx.http.clone(), self.channel_id);
        let Some((mut responder, seeded)) = self.ready_responder(&batch, run_auth).await else {
            return;
        };
        self.note_newcomers(&mut responder, batch_people(&batch))
            .await;

        // A live session already holds the bot's replies as tool calls; only a freshly seeded
        // one needs them to see what it said
        responder.add_messages(agent_input(&batch, |m| seeded || !m.from_bot));

        let result = self
            .run_session(&mut responder, Turn::Agentic, Role::Responder, run_auth)
            .await;
        typing.stop();

        self.unanswered = matches!(result, Err(RunError::SignedOut));
        if let Err(RunError::Failed(e)) = &result {
            tracing::error!(?e, "Error executing agent session in channel main loop");
            if matches!(self.llm, LlmBackend::Chatgpt(_)) {
                chatgpt::report_run_failure(&self.discord_ctx.http, self.channel_id, e).await;
            }
        }
        self.responder = Some(responder);
        // An unanswered session must not expire before its retry, however long the sign-in takes
        self.last_run_finished = (!self.unanswered).then(Instant::now);
    }

    /// The responder session for `batch`: the live one, compacted first when due, or a fresh one.
    /// The flag tells whether it was seeded just now.
    async fn ready_responder(
        &mut self,
        batch: &[Pending],
        run_auth: &mut RunAuth,
    ) -> Option<(AgentSession, bool)> {
        let before = batch.first().map(Pending::anchor);
        let model = match self
            .llm
            .model(Role::Responder, self.channel_id, run_auth.grant.as_ref())
        {
            Ok(model) => model,
            Err(e) => {
                tracing::error!(?e, "Failed to create the responder model");
                return None;
            }
        };

        if let Some(mut responder) = self.live_responder() {
            // A refreshed ChatGPT token only reaches a live session this way
            responder.agent.set_model_handle(model.clone());
            if !responder.needs_compaction() {
                return Some((responder, false));
            }
            if self
                .compact(&mut responder, Role::Responder, before, run_auth)
                .await
                .is_some()
            {
                return Some((responder, true));
            }
        }

        let (history, mut people) = self
            .backfill_history(
                before,
                MESSAGE_CONTEXT_SIZE.saturating_sub(batch_messages(batch)),
            )
            .await;
        people.extend(batch_people(batch));
        let seed = self.seed(history, people).await;
        let memories = match self.memory() {
            None => Memories::Off,
            Some(memory) if self.discord_bot_mention_only => Memories::Keep(memory),
            Some(memory) => Memories::Recall(memory),
        };
        match agent::create_responder_session(
            &self.discord_ctx,
            self.channel_id,
            &self.llm,
            model,
            memories,
            self.firecrawl.clone(),
            seed,
        ) {
            Ok(session) => Some((session, true)),
            Err(e) => {
                tracing::error!(?e, "Failed to create agent session for channel");
                None
            }
        }
    }

    /// The responder's session, unless it has gone unused long enough to expire. Idle since the
    /// bot last ran, not since the last message: the messages keep coming while nobody involves
    /// the bot.
    fn live_responder(&mut self) -> Option<AgentSession> {
        let responder = self.responder.take()?;
        let expired = self
            .last_run_finished
            .is_some_and(|t| t.elapsed() > AGENT_SESSION_TIMEOUT);
        (!expired).then_some(responder)
    }

    /// Keeps a live responder up to date with a batch it didn't run for, so its next run has no
    /// hole where these messages were. Appending leaves its cached prefix intact.
    async fn pass_to_responder(&mut self, batch: Vec<Pending>) {
        let mut responder = self.live_responder();
        if let Some(responder) = responder.as_mut() {
            self.note_newcomers(responder, batch_people(&batch)).await;
            responder.add_messages(agent_input(&batch, |m| !m.from_bot));
        }
        self.responder = responder;
    }

    async fn main_loop(mut self) {
        loop {
            // Whether anything queued deserves a run: the bot's own messages never do, and in
            // mention-only mode neither do user messages that don't address it. Checked over the
            // whole queue, not just the newest entry, because users keep typing after the mention.
            let has_pending_prompt = self
                .message_queue
                .iter()
                .filter_map(Pending::message)
                .any(|m| !m.from_bot && (!self.discord_bot_mention_only || m.addresses_bot));
            let timer = if has_pending_prompt && !self.awaiting_auth {
                tokio::time::sleep_until(
                    self.activity
                        .next_processing_time()
                        .unwrap_or_else(Instant::now)
                        .into(),
                )
            } else {
                tokio::time::sleep(Duration::from_secs(u64::MAX))
            };
            // A channel with a batch waiting for its debounce isn't quiet, whenever the watcher last
            // ran
            let watcher_idle = sleep_until_some(
                self.watcher_last_input
                    .filter(|_| self.watcher.is_some() && !has_pending_prompt)
                    .map(|t| t + WATCHER_SESSION_TIMEOUT),
            );

            let wake = tokio::select! {
                event = self.event_recv.next() => {
                    if let Some(event) = event {
                        match event {
                            ChannelEvent::Message(msg, ctx) => {
                                self.discord_ctx = ctx;
                                let from_bot = msg.message.author.id == self.bot_user_id;
                                // The bot's text-less posts are its ChatGPT notices, not conversation
                                if from_bot && msg.message.content.trim().is_empty() {
                                    continue;
                                }
                                if !from_bot {
                                    // The bot's own replies don't hold the debounce open
                                    self.activity.update_message();
                                }

                                if let Some(guild_id) = msg.message.guild_id {
                                    self.guild_id = Some(guild_id);
                                }
                                // Someone asking again gets another try, e.g. a new sign-in code
                                // once the last one expired
                                if !from_bot
                                    && (!self.discord_bot_mention_only
                                        || message::addresses(&msg.message, self.bot_user_id))
                                {
                                    self.awaiting_auth = false;
                                }

                                self.queue_message(msg.message).await;
                                Wake::Idle
                            }
                            ChannelEvent::Typing(uid, ctx) => {
                                self.discord_ctx = ctx;
                                if uid == self.bot_user_id {
                                    // Ignore typing events from the bot itself
                                    continue;
                                }
                                self.activity.update_typing();
                                Wake::Idle
                            }
                            ChannelEvent::ForceProcess { addressed } => Wake::Process { addressed },
                            ChannelEvent::Reaction(reaction, ctx) => {
                                self.discord_ctx = ctx;
                                self.queue_reaction(reaction).await;
                                Wake::Idle
                            }
                            ChannelEvent::ReactionRemoved(reaction) => {
                                self.unqueue_reaction(reaction);
                                Wake::Idle
                            }
                            ChannelEvent::Edit(update, ctx) => {
                                self.discord_ctx = ctx;
                                self.queue_edit(update).await;
                                Wake::Idle
                            }
                            ChannelEvent::Deletion(message_ids) => {
                                self.queue_deletions(&message_ids);
                                Wake::Idle
                            }
                        }
                    }
                    else {
                        tracing::info!("Channel event receiver closed, exiting main loop");
                        break;
                    }
                }
                // The timer is only armed while something queued deserves a run
                _ = timer => Wake::Process { addressed: false },
                // Signed in, refreshed, or signed out for good: worth another look either way
                _ = auth_changed(self.auth_updates.as_mut()), if self.awaiting_auth => {
                    self.awaiting_auth = false;
                    Wake::Process { addressed: false }
                }
                _ = watcher_idle => Wake::WatcherIdle,
            };

            match wake {
                Wake::Idle => {}
                Wake::Process { addressed } => {
                    self.process(addressed)
                        .instrument(tracing::span!(
                            tracing::Level::INFO,
                            "process_discord_message"
                        ))
                        .await;
                }
                Wake::WatcherIdle => self.retire_watcher().await,
            }
        }
    }
}

/// What woke the channel's main loop up
enum Wake {
    /// Nothing to run yet
    Idle,
    Process {
        addressed: bool,
    },
    /// The watcher went `WATCHER_SESSION_TIMEOUT` without new messages
    WatcherIdle,
}

/// Everyone the messages of `batch` bring into the conversation
fn batch_people(batch: &[Pending]) -> Vec<UserId> {
    batch
        .iter()
        .filter_map(Pending::message)
        .flat_map(|m| m.people.iter().copied())
        .collect()
}

fn batch_messages(batch: &[Pending]) -> usize {
    batch.iter().filter_map(Pending::message).count()
}

/// Keeps only the newest window of the queue: in mention-only mode the backlog would otherwise
/// grow without bound
fn trim_queue(queue: &mut Vec<Pending>) {
    while batch_messages(queue) > MESSAGE_CONTEXT_SIZE || queue.len() > MAX_QUEUED {
        queue.remove(0);
    }
}

/// `batch` as an agent reads it: the messages `keep` lets through, each on its own, and the events
/// between them gathered into one message per stretch
fn agent_input(batch: &[Pending], keep: impl Fn(&PendingMessage) -> bool) -> Vec<RigMessage> {
    let mut input = Vec::new();
    let mut events: Vec<&str> = Vec::new();
    for entry in batch {
        match entry {
            Pending::Event(event) => events.push(&event.line),
            Pending::Message(message) if keep(message) => {
                if !events.is_empty() {
                    input.push(RigMessage::user(events.join("\n")));
                    events.clear();
                }
                input.push(message.message.clone());
            }
            Pending::Message(_) => {}
        }
    }
    if !events.is_empty() {
        input.push(RigMessage::user(events.join("\n")));
    }
    input
}

/// `batch` as the watcher reads it, the bot's messages included
fn observed(batch: &[Pending]) -> Vec<RigMessage> {
    agent_input(batch, |_| true)
        .iter()
        .map(message::observed)
        .collect()
}

/// Whether the watcher's answer calls for the responder: it opens with RESPOND or PASS
fn wants_response(answer: &str) -> bool {
    answer
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .get(..7)
        .is_some_and(|word| word.eq_ignore_ascii_case("respond"))
}

pub struct ChannelHandle {
    event_send: UnboundedSender<ChannelEvent>,

    #[allow(dead_code, reason = "TODO: handle shutdown, restarts, etc.")]
    main_loop_handle: tokio::task::JoinHandle<()>,
}

impl ChannelHandle {
    pub fn new(
        discord_ctx: Context,
        channel_id: ChannelId,
        llm: LlmBackend,
        memory: MemorySystem,
        firecrawl: Option<tools::Firecrawl>,
        discord_bot_mention_only: bool,
        guilds: Arc<scc::HashMap<serenity::model::id::GuildId, Guild>>,
    ) -> Self {
        let (event_send, event_recv) = futures::channel::mpsc::unbounded();

        let bot_user_id = discord_ctx.cache.current_user().id;

        let state = ChannelState {
            activity: ChannelActivity::new(),
            event_recv,
            responder: None,
            watcher: None,
            watcher_last_input: None,
            awaiting_auth: false,
            unanswered: false,
            auth_updates: match &llm {
                LlmBackend::Chatgpt(auth) => Some(auth.subscribe()),
                LlmBackend::Gemini { .. } => None,
            },
            last_run_finished: None,
            llm,
            memory,
            firecrawl,
            guild_id: None,
            bot_user_id,
            discord_ctx: discord_ctx.clone(),
            message_queue: vec![],
            known: KnownMessages::default(),
            channel_id,
            discord_bot_mention_only,
            guilds,
        };

        let main_loop_handle = tokio::spawn(state.main_loop().instrument(tracing::info_span!(
            "channel_main_loop",
            channel_id = channel_id.get(),
            discord_bot_mention_only
        )));

        Self {
            event_send,
            main_loop_handle,
        }
    }

    pub async fn send_event(&mut self, event: ChannelEvent) -> Result<(), eyre::Error> {
        self.event_send
            .send(event)
            .await
            .map_err(|e| eyre::eyre!(e))
    }
}

/// Resolves when the ChatGPT sign-in changes, and never without one to watch
async fn auth_changed(updates: Option<&mut watch::Receiver<AuthState>>) {
    if let Some(updates) = updates
        && updates.changed().await.is_ok()
    {
        return;
    }
    std::future::pending().await
}

/// Resolves at `deadline`, and never without one
async fn sleep_until_some(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_answers_open_with_the_decision() {
        for answer in [
            "RESPOND: nobody answered the question",
            "`RESPOND` — bait worth taking",
            "respond",
            "  **Respond** they're asking the bot",
        ] {
            assert!(wants_response(answer), "{answer}");
        }
        for answer in ["PASS: chatter", "", "RESPONSE", "I'd say RESPOND", "rés"] {
            assert!(!wants_response(answer), "{answer}");
        }
    }

    fn message(id: u64, from_bot: bool) -> Pending {
        let raw: Message = serde_json::from_value(serde_json::json!({
            "id": id.to_string(),
            "channel_id": "1",
            "author": { "id": "2", "username": "wonrax" },
            "content": format!("message {id}"),
            "timestamp": "2026-09-14T12:03:30.469Z",
            "tts": false,
            "mention_everyone": false,
            "mentions": [],
            "mention_roles": [],
            "attachments": [],
            "embeds": [],
            "pinned": false,
            "type": 0
        }))
        .expect("test message should deserialize");
        Pending::Message(PendingMessage {
            message: RigMessage::user(raw.content.clone()),
            raw: Box::new(raw),
            addresses_bot: false,
            people: vec![],
            from_bot,
        })
    }

    fn event(line: &str) -> Pending {
        Pending::Event(PendingEvent {
            at: MessageId::new(1),
            line: line.to_string(),
            reaction: None,
        })
    }

    fn texts(input: &[RigMessage]) -> Vec<String> {
        input
            .iter()
            .map(|message| match message {
                RigMessage::User { content } => content
                    .iter()
                    .filter_map(|part| match part {
                        rig::message::UserContent::Text(text) => Some(text.text.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn events_between_messages_arrive_as_one_message() {
        let batch = vec![
            event("[reaction] a"),
            message(10, false),
            event("[edit] b"),
            message(11, true),
            event("[deletion] c"),
            message(12, false),
        ];
        assert_eq!(
            texts(&agent_input(&batch, |m| !m.from_bot)),
            vec![
                "[reaction] a",
                "message 10",
                "[edit] b\n[deletion] c",
                "message 12"
            ]
        );
        assert_eq!(agent_input(&batch, |_| true).len(), 6);
    }

    #[test]
    fn the_queue_keeps_the_newest_window() {
        let mut queue: Vec<Pending> = (0..MESSAGE_CONTEXT_SIZE as u64 + 2)
            .map(|id| message(id + 1, false))
            .collect();
        trim_queue(&mut queue);
        assert_eq!(batch_messages(&queue), MESSAGE_CONTEXT_SIZE);
        assert_eq!(queue.first().map(Pending::anchor), Some(MessageId::new(3)));

        let mut queue: Vec<Pending> = (0..MAX_QUEUED + 5).map(|_| event("[reaction]")).collect();
        queue.push(message(99, false));
        trim_queue(&mut queue);
        assert_eq!(queue.len(), MAX_QUEUED);
        assert_eq!(queue.last().map(Pending::anchor), Some(MessageId::new(99)));
    }
}
