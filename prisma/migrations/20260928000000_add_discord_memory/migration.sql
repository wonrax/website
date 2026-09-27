-- Accent-insensitive full-text search and trigram fuzzy matching over the memories
CREATE EXTENSION IF NOT EXISTS unaccent;
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- `simple` with the accents folded, so "tuan" finds "Tuấn". ASCII words have none to fold.
CREATE TEXT SEARCH CONFIGURATION simple_unaccent (COPY = simple);
ALTER TEXT SEARCH CONFIGURATION simple_unaccent
    ALTER MAPPING FOR hword, hword_part, word WITH unaccent, simple;

-- unaccent() is only STABLE, which generated columns don't accept
CREATE FUNCTION immutable_unaccent(text) RETURNS text
    LANGUAGE sql IMMUTABLE PARALLEL SAFE STRICT
    RETURN public.unaccent('public.unaccent'::regdictionary, $1);

-- CreateTable
CREATE TABLE "discord_memory_observations" (
    "id" BIGSERIAL NOT NULL,
    "guild_id" BIGINT NOT NULL,
    "channel_id" BIGINT NOT NULL,
    "about_user_ids" BIGINT[] NOT NULL DEFAULT '{}',
    "content" TEXT NOT NULL,
    "keywords" TEXT NOT NULL DEFAULT '',
    "source_message_ids" BIGINT[] NOT NULL DEFAULT '{}',
    "observed_at" TIMESTAMPTZ(6) NOT NULL,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- Words as written weigh A and accent-folded ones B, so an exact match outranks a folded one
    "search_vector" TSVECTOR GENERATED ALWAYS AS (
        setweight(to_tsvector('simple', "content" || ' ' || "keywords"), 'A')
        || setweight(to_tsvector('simple_unaccent', "content" || ' ' || "keywords"), 'B')
    ) STORED,
    "search_text" TEXT GENERATED ALWAYS AS (
        lower(immutable_unaccent("content" || ' ' || "keywords"))
    ) STORED,

    CONSTRAINT "discord_memory_observations_pkey" PRIMARY KEY ("id")
);

CREATE INDEX "discord_memory_observations_guild_id_channel_id_idx" ON "discord_memory_observations"("guild_id", "channel_id");
CREATE INDEX "discord_memory_observations_about_user_ids_idx" ON "discord_memory_observations" USING GIN ("about_user_ids");
CREATE INDEX "discord_memory_observations_search_vector_idx" ON "discord_memory_observations" USING GIN ("search_vector");
CREATE INDEX "discord_memory_observations_search_text_idx" ON "discord_memory_observations" USING GIN ("search_text" gin_trgm_ops);

-- CreateTable
CREATE TABLE "discord_memory_people" (
    "guild_id" BIGINT NOT NULL,
    "user_id" BIGINT NOT NULL,
    "name" TEXT NOT NULL,
    "updated_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "discord_memory_people_pkey" PRIMARY KEY ("guild_id", "user_id")
);

-- CreateTable
CREATE TABLE "discord_memory_docs" (
    "id" BIGSERIAL NOT NULL,
    "guild_id" BIGINT NOT NULL,
    "kind" TEXT NOT NULL CHECK ("kind" IN ('user', 'channel')),
    "subject_id" BIGINT NOT NULL,
    "content" TEXT NOT NULL,
    "dreamed_through" BIGINT NOT NULL,
    "revisit_on" DATE,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "discord_memory_docs_pkey" PRIMARY KEY ("id")
);

CREATE INDEX "discord_memory_docs_guild_id_kind_subject_id_id_idx" ON "discord_memory_docs"("guild_id", "kind", "subject_id", "id" DESC);
