//! The memories in Postgres: the observation log, the names of the people it is about, and the
//! docs distilled from it

use std::{collections::HashMap, num::NonZeroU64};

use chrono::{DateTime, NaiveDate, Utc};
use diesel::{
    prelude::*,
    sql_types::{Array, BigInt, Bool, Nullable, Text, Timestamptz},
    upsert::excluded,
};
use diesel_async::{AsyncConnection as _, AsyncPgConnection, RunQueryDsl};
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

/// A search of a server's observations: those about its people and those about `channel_id`
pub struct Search<'a> {
    pub guild_id: GuildId,
    pub channel_id: ChannelId,
    pub query: &'a str,
    /// Only what's about every one of them
    pub about: &'a [UserId],
    /// Only what was observed before then
    pub before: Option<DateTime<Utc>>,
    pub limit: i64,
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
    /// What the log gained since the doc was written, in log order: new observations, and the
    /// withdrawals of ones the doc took in
    pub observations: Vec<DreamObservation>,
}

impl DreamInput {
    /// How far into the log the rewritten doc accounts for
    pub fn dreamed_through(&self) -> i64 {
        self.observations
            .last()
            .map(|o| o.position)
            .or(self.doc.as_ref().map(|d| d.dreamed_through))
            .unwrap_or(0)
    }
}

#[derive(QueryableByName)]
pub struct DreamObservation {
    /// Its place in the log: its ID, or its withdrawal's
    #[diesel(sql_type = BigInt)]
    position: i64,
    /// Its source messages were deleted, and the doc should let go of it
    #[diesel(sql_type = Bool)]
    pub withdrawn: bool,
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
struct WithdrawnRow {
    #[diesel(sql_type = Bool)]
    withdrawn: bool,
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

    /// Takes deleted messages of the channel out of the sources of the observations citing them,
    /// and withdraws the observations left with none. Returns how many it withdrew.
    pub async fn withdraw_sources(
        &self,
        guild_id: GuildId,
        channel_id: ChannelId,
        message_ids: &[MessageId],
    ) -> eyre::Result<usize> {
        let mut conn = self.db.get().await.context("No database connection")?;
        let rows: Vec<WithdrawnRow> = diesel::sql_query(
            r#"
            UPDATE discord_memory_observations
            SET source_message_ids = ARRAY(
                    SELECT id FROM unnest(source_message_ids) WITH ORDINALITY AS s(id, position)
                    WHERE id <> ALL($3)
                    ORDER BY position
                ),
                -- The right-hand sides read the row as it was
                withdrawn_at = CASE WHEN source_message_ids <@ $3 THEN now() END,
                withdrawn_seq = CASE WHEN source_message_ids <@ $3
                    THEN nextval(pg_get_serial_sequence('discord_memory_observations', 'id'))
                END
            WHERE guild_id = $1 AND channel_id = $2 AND withdrawn_at IS NULL
                AND source_message_ids && $3
            RETURNING withdrawn_at IS NOT NULL AS withdrawn
            "#,
        )
        .bind::<BigInt, _>(guild_id.get().cast_signed())
        .bind::<BigInt, _>(channel_id.get().cast_signed())
        .bind::<Array<BigInt>, _>(
            message_ids
                .iter()
                .map(|id| id.get().cast_signed())
                .collect::<Vec<_>>(),
        )
        .load(&mut conn)
        .await
        .context("Failed to withdraw the deleted messages from the memories")?;
        Ok(rows.iter().filter(|row| row.withdrawn).count())
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

    pub async fn search(&self, search: &Search<'_>) -> eyre::Result<Vec<FoundObservation>> {
        let mut conn = self.db.get().await.context("No database connection")?;
        search.run(&mut conn).await
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

    /// The docs due a dream, in `guild_id` or in every server: those the log gained anything for
    /// since their last version, the ones that don't exist yet, and those whose revisit date has
    /// come
    pub async fn pending_subjects(
        &self,
        guild_id: Option<GuildId>,
        today: NaiveDate,
    ) -> eyre::Result<Vec<(GuildId, Subject)>> {
        let mut conn = self.db.get().await.context("No database connection")?;
        let rows: Vec<SubjectRow> = diesel::sql_query(
            r#"
            WITH settled AS (
                SELECT id, guild_id, channel_id, about_user_ids,
                    CASE WHEN withdrawn_at < now() - INTERVAL '5 seconds' THEN withdrawn_seq END
                        AS withdrawn_seq
                FROM discord_memory_observations
                WHERE ($1::BIGINT IS NULL OR guild_id = $1)
                    -- Newer ones may still be committing, out of ID order
                    AND created_at < now() - INTERVAL '5 seconds'
            ),
            entries AS (
                SELECT o.guild_id, 'user' AS kind, a.user_id AS subject_id, o.id, o.withdrawn_seq
                FROM settled o CROSS JOIN LATERAL unnest(o.about_user_ids) AS a(user_id)
                UNION ALL
                SELECT guild_id, 'channel', channel_id, id, withdrawn_seq
                FROM settled
            ),
            docs AS (
                SELECT DISTINCT ON (guild_id, kind, subject_id)
                    guild_id, kind, subject_id, dreamed_through, revisit_on
                FROM discord_memory_docs
                ORDER BY guild_id, kind, subject_id, id DESC
            )
            SELECT DISTINCT e.guild_id, e.kind, e.subject_id
            FROM entries e
            LEFT JOIN docs d USING (guild_id, kind, subject_id)
            WHERE CASE
                    WHEN e.withdrawn_seq IS NULL THEN e.id > coalesce(d.dreamed_through, 0)
                    -- A withdrawal only matters to the docs that took the observation in
                    ELSE e.id <= d.dreamed_through AND e.withdrawn_seq > d.dreamed_through
                END
                OR d.revisit_on <= $2
            ORDER BY e.guild_id, e.kind, e.subject_id
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

    /// The doc of `subject` and what the log gained for it since, up to a batch
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

        // A new observation withdrawn before the doc took it in is skipped altogether
        let observations: Vec<DreamObservation> = diesel::sql_query(format!(
            r#"
            SELECT coalesce(o.withdrawn_seq, o.id) AS position,
                o.withdrawn_seq IS NOT NULL AS withdrawn,
                o.content, {ABOUT_NAMES} AS about, o.observed_at
            FROM discord_memory_observations o
            WHERE o.guild_id = $1
                AND o.created_at < now() - INTERVAL '5 seconds'
                AND CASE $3
                    WHEN 'user' THEN $4 = ANY(o.about_user_ids)
                    ELSE o.channel_id = $4
                END
                AND CASE
                    WHEN o.withdrawn_seq IS NULL THEN o.id > $2
                    ELSE o.id <= $2 AND o.withdrawn_seq > $2
                        AND o.withdrawn_at < now() - INTERVAL '5 seconds'
                END
            ORDER BY position
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

impl Search<'_> {
    /// Up to `limit` observations matching the query, best first. Each word, and each two in a
    /// row, which is how most Vietnamese words are spelled, counts by how rare it is among the
    /// observations searched, fully when it matches as written and less when only its
    /// accent-folded form does. Misspelled words match by their trigrams; the two rankings are
    /// fused.
    async fn run(&self, conn: &mut AsyncPgConnection) -> eyre::Result<Vec<FoundObservation>> {
        let sql = format!(
            r#"
            -- Inlined into each use, so both rankings reach their indexes
            WITH scope AS NOT MATERIALIZED (
                SELECT o.id, o.search_vector, o.search_text FROM discord_memory_observations o
                WHERE o.guild_id = $1 AND o.withdrawn_at IS NULL
                    AND (o.channel_id = $2 OR cardinality(o.about_user_ids) > 0)
                    AND o.about_user_ids @> $5
                    AND o.observed_at < coalesce($6, 'infinity')
            ),
            -- The query's words by position, as tsquery literals written and accent-folded
            tokens AS (
                SELECT position,
                    '''' || replace(replace(w.lexeme, '\', '\\'), '''', '''''') || '''' AS written,
                    f.lexeme AS folded_lexeme,
                    '''' || replace(replace(f.lexeme, '\', '\\'), '''', '''''') || '''' AS folded
                FROM (SELECT lexeme, unnest(positions) AS position
                    FROM unnest(to_tsvector('simple', $3))) AS w
                JOIN (SELECT lexeme, unnest(positions) AS position
                    FROM unnest(to_tsvector('simple_unaccent', $3))) AS f USING (position)
            ),
            -- Each word and each two in a row, keyed by their folded form so that what matches
            -- both ways counts once
            grams AS (
                SELECT folded_lexeme AS term, written, folded FROM tokens
                UNION ALL
                SELECT a.folded_lexeme || ' ' || b.folded_lexeme,
                    a.written || ' <-> ' || b.written, a.folded || ' <-> ' || b.folded
                FROM tokens a JOIN tokens b ON b.position = a.position + 1
            ),
            variants AS (
                SELECT term, variant, variant::TSQUERY AS query, max(weight) AS weight FROM (
                    SELECT term, written AS variant, 1.0 AS weight FROM grams
                    UNION ALL
                    SELECT term, folded, 0.5 FROM grams
                ) AS v
                GROUP BY term, variant
            ),
            matches AS (
                SELECT s.id, v.term, v.variant, v.weight
                FROM scope s JOIN variants v ON s.search_vector @@ v.query
            ),
            -- BM25's inverse document frequency: near zero for what most observations contain
            rarity AS (
                SELECT variant, ln(1 + (n.total - count(*) + 0.5) / (count(*) + 0.5)) AS idf
                FROM matches, (SELECT count(*) AS total FROM scope) AS n
                GROUP BY variant, n.total
            ),
            by_words AS (
                SELECT id, row_number() OVER (ORDER BY score DESC, id DESC) AS rank FROM (
                    SELECT id, sum(best) AS score FROM (
                        SELECT m.id, max(m.weight * r.idf) AS best
                        FROM matches m JOIN rarity r USING (variant)
                        GROUP BY m.id, m.term
                    ) AS terms
                    GROUP BY id
                ) AS scored
                ORDER BY rank LIMIT 50
            ),
            by_spelling AS (
                SELECT s.id, row_number() OVER (ORDER BY q.text <<-> s.search_text, s.id DESC) AS rank
                FROM scope s, (SELECT lower(immutable_unaccent($3)) AS text) AS q
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

        let rows: Vec<FoundRow> = diesel::sql_query(sql)
            .bind::<BigInt, _>(self.guild_id.get().cast_signed())
            .bind::<BigInt, _>(self.channel_id.get().cast_signed())
            .bind::<Text, _>(self.query)
            .bind::<BigInt, _>(self.limit)
            .bind::<Array<BigInt>, _>(
                self.about
                    .iter()
                    .map(|id| id.get().cast_signed())
                    .collect::<Vec<_>>(),
            )
            .bind::<Nullable<Timestamptz>, _>(self.before)
            .load(conn)
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
}

#[cfg(test)]
mod tests {
    use diesel_async::{AsyncConnection as _, AsyncPgConnection, SimpleAsyncConnection as _};
    use eyre::Context as _;
    use serenity::all::{ChannelId, GuildId, UserId};

    use super::Search;

    const MIGRATIONS: [&str; 2] = [
        include_str!("../../../../prisma/migrations/20260928000000_add_discord_memory/migration.sql"),
        include_str!(
            "../../../../prisma/migrations/20260929000000_withdraw_discord_memory_observations/migration.sql"
        ),
    ];

    const TOFU: UserId = UserId::new(100);
    const MOCHI: UserId = UserId::new(200);

    const MOONFISH: &str = "tofu went on a date with Hà mới at Moonfish";
    const CHU_DONG: &str = "tofu không chủ động nhắn tin, chỉ đợi Hà mới nhắn trước";
    const GYM: &str = "mochi có động lực đi tập gym mỗi chủ nhật";
    const SUMMER: &str = "mochi thích mùa hạ, nhất là đi biển";

    /// Forty observations dense with common Vietnamese syllables, then a few that stand out,
    /// each newer than the last
    const LOG: &str = r#"
        INSERT INTO discord_memory_observations
            (guild_id, channel_id, about_user_ids, content, keywords, observed_at)
        SELECT 1, 10, '{100}', 'tofu nói anh em không đi chơi cuối tuần, chỉ ở nhà mới vui',
            'không đi chơi cuối tuần ở nhà', '2026-08-01'::TIMESTAMPTZ + i * INTERVAL '1 hour'
        FROM generate_series(1, 40) AS i;

        INSERT INTO discord_memory_observations
            (guild_id, channel_id, about_user_ids, content, keywords, observed_at)
        VALUES
            (1, 10, '{100}', 'tofu went on a date with Hà mới at Moonfish', 'hẹn hò', '2026-09-01'),
            (1, 10, '{100}', 'tofu không chủ động nhắn tin, chỉ đợi Hà mới nhắn trước', '', '2026-09-02'),
            (1, 10, '{200}', 'mochi có động lực đi tập gym mỗi chủ nhật', 'động lực chủ nhật', '2026-09-03'),
            (1, 10, '{200}', 'mochi thích mùa hạ, nhất là đi biển', 'mùa hè', '2026-09-04');
    "#;

    /// The contents `LOG` gives for a search, in the empty database at MEMORY_TEST_DATABASE_URL,
    /// inside a transaction that's never committed
    async fn search(
        query: &str,
        about: &[UserId],
        before: Option<&str>,
    ) -> eyre::Result<Vec<String>> {
        let url = std::env::var("MEMORY_TEST_DATABASE_URL")
            .context("MEMORY_TEST_DATABASE_URL is unset")?;
        let mut conn = AsyncPgConnection::establish(&url).await?;
        conn.begin_test_transaction().await?;
        for sql in MIGRATIONS.into_iter().chain([LOG]) {
            conn.batch_execute(sql).await?;
        }
        let before = before
            .map(|day| format!("{day}T00:00:00Z").parse())
            .transpose()?;
        let found = Search {
            guild_id: GuildId::new(1),
            channel_id: ChannelId::new(10),
            query,
            about,
            before,
            limit: 5,
        }
        .run(&mut conn)
        .await?;
        Ok(found.into_iter().map(|o| o.content).collect())
    }

    #[tokio::test]
    #[ignore = "needs an empty Postgres database at MEMORY_TEST_DATABASE_URL"]
    async fn rare_words_outweigh_common_ones() -> eyre::Result<()> {
        let found = search("tofu Hà mới đi chơi không anh Moonfish tuần", &[], None).await?;
        assert_eq!(found.first().map(String::as_str), Some(MOONFISH), "{found:#?}");
        Ok(())
    }

    #[tokio::test]
    #[ignore = "needs an empty Postgres database at MEMORY_TEST_DATABASE_URL"]
    async fn syllables_in_a_row_count_as_a_word() -> eyre::Result<()> {
        // The gym has both syllables apart, and is newer, which would win a tie
        for query in ["chủ động", "chu dong"] {
            let found = search(query, &[], None).await?;
            assert_eq!(found, [CHU_DONG, GYM], "{query}");
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore = "needs an empty Postgres database at MEMORY_TEST_DATABASE_URL"]
    async fn words_as_written_outrank_their_folded_lookalikes() -> eyre::Result<()> {
        // "hạ" folds to "ha" like "Hà" does, and the summer one is the newest
        let found = search("hà", &[], None).await?;
        assert_eq!(found.first().map(String::as_str), Some(CHU_DONG), "{found:#?}");
        assert!(found.iter().any(|content| content == SUMMER), "{found:#?}");
        Ok(())
    }

    #[tokio::test]
    #[ignore = "needs an empty Postgres database at MEMORY_TEST_DATABASE_URL"]
    async fn about_and_before_narrow_the_search() -> eyre::Result<()> {
        assert_eq!(search("chủ động", &[MOCHI], None).await?, [GYM]);

        let found = search("Hà mới", &[TOFU], Some("2026-09-02")).await?;
        assert_eq!(found.first().map(String::as_str), Some(MOONFISH), "{found:#?}");
        assert!(!found.iter().any(|content| content == CHU_DONG), "{found:#?}");
        Ok(())
    }
}
