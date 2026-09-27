//! The memories in Postgres: the observation log, the names of the people it is about, and the
//! docs distilled from it

use std::{collections::HashMap, num::NonZeroU64};

use chrono::{DateTime, NaiveDate, Utc};
use diesel::{
    prelude::*,
    sql_types::{Array, BigInt, Nullable, Text, Timestamptz},
    upsert::excluded,
};
use diesel_async::{AsyncConnection as _, RunQueryDsl};
use eyre::Context as _;
use serenity::all::{ChannelId, GuildId, MessageId, UserId};

use super::{Note, Notes, Profile, Subject};
use crate::{
    discord::chatgpt::DbPool,
    schema::{discord_memory_docs, discord_memory_observations, discord_memory_people},
};

/// Observations a single dream distills at most. A backlog, such as a new doc's whole history,
/// takes several.
const DREAM_BATCH: i64 = 500;

#[derive(Clone)]
pub struct MemoryStore {
    db: DbPool,
}

/// An observation on its way into the log
pub struct NewObservation {
    pub about: Vec<UserId>,
    pub content: String,
    pub keywords: String,
    pub source_message_ids: Vec<MessageId>,
    /// When the newest message it comes from was sent
    pub observed_at: DateTime<Utc>,
}

/// An observation as search finds it
pub struct FoundObservation {
    pub content: String,
    /// Names of the people it is about
    pub about: Vec<String>,
    pub observed_at: DateTime<Utc>,
    /// Where it was recorded, which its source messages belong to. What's about people comes
    /// from every channel of the server.
    pub channel_id: ChannelId,
    pub source_message_ids: Vec<u64>,
}

/// A doc's newest version
pub struct Doc {
    id: i64,
    pub content: String,
    pub dreamed_through: i64,
    pub written_at: DateTime<Utc>,
}

/// What a dream rewrites a doc from
pub struct DreamInput {
    pub doc: Option<Doc>,
    /// The user's name, for a profile
    pub name: Option<String>,
    /// Recorded since the doc was written, oldest first
    pub observations: Vec<DreamObservation>,
}

impl DreamInput {
    /// The newest observation the rewritten doc accounts for
    pub fn dreamed_through(&self) -> i64 {
        self.observations
            .last()
            .map(|o| o.id)
            .or(self.doc.as_ref().map(|d| d.dreamed_through))
            .unwrap_or(0)
    }
}

#[derive(QueryableByName)]
pub struct DreamObservation {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    pub content: String,
    #[diesel(sql_type = Array<Text>)]
    pub about: Vec<String>,
    #[diesel(sql_type = Timestamptz)]
    pub observed_at: DateTime<Utc>,
}

impl Subject {
    fn kind(self) -> &'static str {
        match self {
            Self::User(_) => "user",
            Self::Channel(_) => "channel",
        }
    }

    fn id(self) -> i64 {
        match self {
            Self::User(id) => id.get().cast_signed(),
            Self::Channel(id) => id.get().cast_signed(),
        }
    }

    fn from_row(kind: &str, id: i64) -> Option<Self> {
        let id = NonZeroU64::new(id.cast_unsigned())?;
        match kind {
            "user" => Some(Self::User(id.into())),
            "channel" => Some(Self::Channel(id.into())),
            _ => None,
        }
    }
}

#[derive(Insertable)]
#[diesel(table_name = discord_memory_observations)]
struct ObservationRow<'a> {
    guild_id: i64,
    channel_id: i64,
    about_user_ids: Vec<i64>,
    content: &'a str,
    keywords: &'a str,
    source_message_ids: Vec<i64>,
    observed_at: DateTime<Utc>,
}

#[derive(Insertable)]
#[diesel(table_name = discord_memory_people)]
struct PersonRow<'a> {
    guild_id: i64,
    user_id: i64,
    name: &'a str,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = discord_memory_docs)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct DocRow {
    id: i64,
    content: String,
    dreamed_through: i64,
    created_at: DateTime<Utc>,
}

impl From<DocRow> for Doc {
    fn from(row: DocRow) -> Self {
        Self {
            id: row.id,
            content: row.content,
            dreamed_through: row.dreamed_through,
            written_at: row.created_at,
        }
    }
}

#[derive(QueryableByName)]
struct FoundRow {
    #[diesel(sql_type = Text)]
    content: String,
    #[diesel(sql_type = Array<Text>)]
    about: Vec<String>,
    #[diesel(sql_type = Timestamptz)]
    observed_at: DateTime<Utc>,
    #[diesel(sql_type = BigInt)]
    channel_id: i64,
    #[diesel(sql_type = Array<BigInt>)]
    source_message_ids: Vec<i64>,
}

#[derive(QueryableByName)]
struct NoteRow {
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = BigInt)]
    subject_id: i64,
    #[diesel(sql_type = Text)]
    content: String,
    #[diesel(sql_type = Timestamptz)]
    created_at: DateTime<Utc>,
    #[diesel(sql_type = Nullable<Text>)]
    name: Option<String>,
}

#[derive(QueryableByName)]
struct PersonIdRow {
    #[diesel(sql_type = BigInt)]
    user_id: i64,
}

#[derive(QueryableByName)]
struct SubjectRow {
    #[diesel(sql_type = BigInt)]
    guild_id: i64,
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = BigInt)]
    subject_id: i64,
}

/// The names of the users an observation `o` is about, in order, falling back to their IDs
const ABOUT_NAMES: &str = "ARRAY(
    SELECT coalesce(p.name, a.user_id::TEXT)
    FROM unnest(o.about_user_ids) WITH ORDINALITY AS a(user_id, position)
    LEFT JOIN discord_memory_people p ON p.guild_id = o.guild_id AND p.user_id = a.user_id
    ORDER BY a.position
)";

impl MemoryStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    /// Appends `observations` to the log, and keeps the names of `people` for the server
    pub async fn record(
        &self,
        guild_id: GuildId,
        channel_id: ChannelId,
        people: &HashMap<UserId, String>,
        observations: &[NewObservation],
    ) -> eyre::Result<()> {
        let guild_id = guild_id.get().cast_signed();
        let people: Vec<PersonRow> = people
            .iter()
            .map(|(user_id, name)| PersonRow {
                guild_id,
                user_id: user_id.get().cast_signed(),
                name,
            })
            .collect();
        let observations: Vec<ObservationRow> = observations
            .iter()
            .map(|o| ObservationRow {
                guild_id,
                channel_id: channel_id.get().cast_signed(),
                about_user_ids: o.about.iter().map(|id| id.get().cast_signed()).collect(),
                content: &o.content,
                keywords: &o.keywords,
                source_message_ids: o
                    .source_message_ids
                    .iter()
                    .map(|id| id.get().cast_signed())
                    .collect(),
                observed_at: o.observed_at,
            })
            .collect();

        let mut conn = self.db.get().await.context("No database connection")?;
        conn.transaction::<_, eyre::Report, _>(async |conn| {
            use discord_memory_people::dsl as person;
            if !people.is_empty() {
                diesel::insert_into(person::discord_memory_people)
                    .values(&people)
                    .on_conflict((person::guild_id, person::user_id))
                    .do_update()
                    .set((
                        person::name.eq(excluded(person::name)),
                        person::updated_at.eq(Utc::now()),
                    ))
                    .execute(conn)
                    .await
                    .context("Failed to save the names of the people")?;
            }
            diesel::insert_into(discord_memory_observations::table)
                .values(&observations)
                .execute(conn)
                .await
                .context("Failed to append the observations")?;
            Ok(())
        })
        .await
    }

    /// The user of the server last seen as `name`, ignoring case
    pub async fn find_person(&self, guild_id: GuildId, name: &str) -> eyre::Result<Option<UserId>> {
        let mut conn = self.db.get().await.context("No database connection")?;
        let row: Option<PersonIdRow> = diesel::sql_query(
            "SELECT user_id FROM discord_memory_people
            WHERE guild_id = $1 AND lower(name) = lower($2)
            ORDER BY updated_at DESC
            LIMIT 1",
        )
        .bind::<BigInt, _>(guild_id.get().cast_signed())
        .bind::<Text, _>(name)
        .get_result(&mut conn)
        .await
        .optional()
        .context("Failed to look up a person")?;
        Ok(row
            .and_then(|row| NonZeroU64::new(row.user_id.cast_unsigned()))
            .map(UserId::from))
    }

    /// Up to `limit` observations matching `query`, best first, among those about people of the
    /// server and those about the channel itself. Words match regardless of accents, and
    /// misspelled ones by their trigrams; the two rankings are fused.
    pub async fn search(
        &self,
        guild_id: GuildId,
        channel_id: ChannelId,
        query: &str,
        limit: i64,
    ) -> eyre::Result<Vec<FoundObservation>> {
        let sql = format!(
            r#"
            WITH q AS (
                -- Any of the words rather than all of them. Each matches as written (weight A)
                -- or accent-folded (B), as the search vector stores it.
                SELECT (
                    SELECT string_agg(term, ' | ')::TSQUERY FROM (
                        SELECT '''' || replace(replace(lexeme, '\', '\\'), '''', '''''') || ''':A' AS term
                        FROM unnest(tsvector_to_array(to_tsvector('simple', $3))) AS lexeme
                        UNION
                        SELECT '''' || replace(replace(lexeme, '\', '\\'), '''', '''''') || ''':B'
                        FROM unnest(tsvector_to_array(to_tsvector('simple_unaccent', $3))) AS lexeme
                    ) AS terms
                ) AS words,
                lower(immutable_unaccent($3)) AS text
            ),
            scope AS (
                SELECT o.* FROM discord_memory_observations o
                WHERE o.guild_id = $1 AND (o.channel_id = $2 OR cardinality(o.about_user_ids) > 0)
            ),
            by_words AS (
                SELECT s.id, row_number() OVER (
                    ORDER BY ts_rank_cd(s.search_vector, q.words) DESC, s.id DESC
                ) AS rank
                FROM scope s, q
                WHERE s.search_vector @@ q.words
                ORDER BY rank LIMIT 50
            ),
            by_spelling AS (
                SELECT s.id, row_number() OVER (ORDER BY q.text <<-> s.search_text, s.id DESC) AS rank
                FROM scope s, q
                WHERE q.text <% s.search_text
                ORDER BY rank LIMIT 50
            ),
            fused AS (
                SELECT id, sum(weight / (60 + rank)) AS score FROM (
                    SELECT id, rank, 1.0 AS weight FROM by_words
                    UNION ALL
                    SELECT id, rank, 0.7 FROM by_spelling
                ) AS ranked
                GROUP BY id
            )
            SELECT o.content, {ABOUT_NAMES} AS about, o.observed_at, o.channel_id,
                o.source_message_ids
            FROM fused f
            JOIN discord_memory_observations o USING (id)
            ORDER BY f.score DESC, o.id DESC
            LIMIT $4
            "#
        );

        let mut conn = self.db.get().await.context("No database connection")?;
        let rows: Vec<FoundRow> = diesel::sql_query(sql)
            .bind::<BigInt, _>(guild_id.get().cast_signed())
            .bind::<BigInt, _>(channel_id.get().cast_signed())
            .bind::<Text, _>(query)
            .bind::<BigInt, _>(limit)
            .load(&mut conn)
            .await
            .context("Failed to search the memories")?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                Some(FoundObservation {
                    content: row.content,
                    about: row.about,
                    observed_at: row.observed_at,
                    channel_id: NonZeroU64::new(row.channel_id.cast_unsigned())?.into(),
                    source_message_ids: row
                        .source_message_ids
                        .into_iter()
                        .map(i64::cast_unsigned)
                        .collect(),
                })
            })
            .collect())
    }

    /// The newest docs: the lore of `lore_of` when given, and the profiles of `people` in the
    /// server. Empty docs are left out.
    pub async fn notes(
        &self,
        guild_id: GuildId,
        lore_of: Option<ChannelId>,
        people: &[UserId],
    ) -> eyre::Result<Notes> {
        let mut conn = self.db.get().await.context("No database connection")?;
        let rows: Vec<NoteRow> = diesel::sql_query(
            r#"
            SELECT DISTINCT ON (d.kind, d.subject_id)
                d.kind, d.subject_id, d.content, d.created_at, p.name
            FROM discord_memory_docs d
            LEFT JOIN discord_memory_people p
                ON d.kind = 'user' AND p.guild_id = d.guild_id AND p.user_id = d.subject_id
            WHERE d.guild_id = $1 AND (
                (d.kind = 'channel' AND d.subject_id = $2)
                OR (d.kind = 'user' AND d.subject_id = ANY($3))
            )
            ORDER BY d.kind, d.subject_id, d.id DESC
            "#,
        )
        .bind::<BigInt, _>(guild_id.get().cast_signed())
        .bind::<Nullable<BigInt>, _>(lore_of.map(|id| id.get().cast_signed()))
        .bind::<Array<BigInt>, _>(
            people
                .iter()
                .map(|id| id.get().cast_signed())
                .collect::<Vec<_>>(),
        )
        .load(&mut conn)
        .await
        .context("Failed to load the memory docs")?;

        let mut notes = Notes::default();
        for row in rows {
            if row.content.trim().is_empty() {
                continue;
            }
            let note = Note {
                content: row.content,
                written_at: row.created_at,
            };
            match Subject::from_row(&row.kind, row.subject_id) {
                Some(Subject::Channel(_)) => notes.lore = Some(note),
                Some(Subject::User(user_id)) => notes.profiles.push(Profile {
                    user_id,
                    name: row.name,
                    note,
                }),
                None => {}
            }
        }
        notes.profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(notes)
    }

    /// The docs due a dream, in `guild_id` or in every server: those with observations newer than
    /// their last version, the ones that don't exist yet, and those whose revisit date has come
    pub async fn pending_subjects(
        &self,
        guild_id: Option<GuildId>,
        today: NaiveDate,
    ) -> eyre::Result<Vec<(GuildId, Subject)>> {
        let mut conn = self.db.get().await.context("No database connection")?;
        let rows: Vec<SubjectRow> = diesel::sql_query(
            r#"
            WITH settled AS (
                SELECT * FROM discord_memory_observations
                WHERE ($1::BIGINT IS NULL OR guild_id = $1)
                    -- Newer ones may still be committing, out of ID order
                    AND created_at < now() - INTERVAL '5 seconds'
            ),
            subjects AS (
                SELECT o.guild_id, 'user' AS kind, a.user_id AS subject_id, max(o.id) AS newest
                FROM settled o CROSS JOIN LATERAL unnest(o.about_user_ids) AS a(user_id)
                GROUP BY o.guild_id, a.user_id
                UNION ALL
                SELECT guild_id, 'channel', channel_id, max(id)
                FROM settled
                GROUP BY guild_id, channel_id
            ),
            docs AS (
                SELECT DISTINCT ON (guild_id, kind, subject_id)
                    guild_id, kind, subject_id, dreamed_through, revisit_on
                FROM discord_memory_docs
                ORDER BY guild_id, kind, subject_id, id DESC
            )
            SELECT s.guild_id, s.kind, s.subject_id
            FROM subjects s
            LEFT JOIN docs d USING (guild_id, kind, subject_id)
            WHERE d.subject_id IS NULL OR s.newest > d.dreamed_through OR d.revisit_on <= $2
            ORDER BY s.guild_id, s.kind, s.subject_id
            "#,
        )
        .bind::<Nullable<BigInt>, _>(guild_id.map(|id| id.get().cast_signed()))
        .bind::<diesel::sql_types::Date, _>(today)
        .load(&mut conn)
        .await
        .context("Failed to find the docs due a dream")?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let guild_id = NonZeroU64::new(row.guild_id.cast_unsigned())?;
                Some((
                    guild_id.into(),
                    Subject::from_row(&row.kind, row.subject_id)?,
                ))
            })
            .collect())
    }

    /// The doc of `subject` and the observations recorded since, up to a batch of them
    pub async fn dream_input(
        &self,
        guild_id: GuildId,
        subject: Subject,
    ) -> eyre::Result<DreamInput> {
        use discord_memory_docs::dsl as doc;

        let guild = guild_id.get().cast_signed();
        let mut conn = self.db.get().await.context("No database connection")?;
        let current: Option<Doc> = doc::discord_memory_docs
            .filter(doc::guild_id.eq(guild))
            .filter(doc::kind.eq(subject.kind()))
            .filter(doc::subject_id.eq(subject.id()))
            .order(doc::id.desc())
            .select(DocRow::as_select())
            .first(&mut conn)
            .await
            .optional()
            .context("Failed to load the doc")?
            .map(Doc::from);

        let name = match subject {
            Subject::User(user_id) => {
                use discord_memory_people::dsl as person;
                person::discord_memory_people
                    .filter(person::guild_id.eq(guild))
                    .filter(person::user_id.eq(user_id.get().cast_signed()))
                    .select(person::name)
                    .first(&mut conn)
                    .await
                    .optional()
                    .context("Failed to load the person's name")?
            }
            Subject::Channel(_) => None,
        };

        let observations: Vec<DreamObservation> = diesel::sql_query(format!(
            r#"
            SELECT o.id, o.content, {ABOUT_NAMES} AS about, o.observed_at
            FROM discord_memory_observations o
            WHERE o.guild_id = $1 AND o.id > $2
                AND o.created_at < now() - INTERVAL '5 seconds'
                AND CASE $3
                    WHEN 'user' THEN $4 = ANY(o.about_user_ids)
                    ELSE o.channel_id = $4
                END
            ORDER BY o.id
            LIMIT $5
            "#
        ))
        .bind::<BigInt, _>(guild)
        .bind::<BigInt, _>(current.as_ref().map_or(0, |d| d.dreamed_through))
        .bind::<Text, _>(subject.kind())
        .bind::<BigInt, _>(subject.id())
        .bind::<BigInt, _>(DREAM_BATCH)
        .load(&mut conn)
        .await
        .context("Failed to load the observations to dream")?;

        Ok(DreamInput {
            doc: current,
            name,
            observations,
        })
    }

    /// Stores `content` as the doc of `subject`, distilled from the observations through
    /// `dreamed_through`. Rewriting `current` into the same text keeps its row and only moves its
    /// marks. Returns whether the text changed.
    pub async fn save_doc(
        &self,
        guild_id: GuildId,
        subject: Subject,
        current: Option<&Doc>,
        content: &str,
        dreamed_through: i64,
        revisit_on: Option<NaiveDate>,
    ) -> eyre::Result<bool> {
        use discord_memory_docs::dsl as doc;

        let mut conn = self.db.get().await.context("No database connection")?;
        if let Some(current) = current.filter(|d| d.content == content) {
            diesel::update(doc::discord_memory_docs.find(current.id))
                .set((
                    doc::dreamed_through.eq(dreamed_through),
                    doc::revisit_on.eq(revisit_on),
                ))
                .execute(&mut conn)
                .await
                .context("Failed to move the doc's marks")?;
            return Ok(false);
        }

        diesel::insert_into(doc::discord_memory_docs)
            .values((
                doc::guild_id.eq(guild_id.get().cast_signed()),
                doc::kind.eq(subject.kind()),
                doc::subject_id.eq(subject.id()),
                doc::content.eq(content),
                doc::dreamed_through.eq(dreamed_through),
                doc::revisit_on.eq(revisit_on),
            ))
            .execute(&mut conn)
            .await
            .context("Failed to save the doc")?;
        Ok(true)
    }
}
