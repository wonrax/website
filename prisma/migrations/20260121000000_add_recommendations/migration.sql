CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE online_article_sources (
    id SERIAL PRIMARY KEY,
    "key" TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    base_url TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE online_articles (
    id SERIAL PRIMARY KEY,
    url TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    content_text TEXT,
    created_at TIMESTAMP DEFAULT NOW()
);

CREATE TABLE online_article_metadata (
    id SERIAL PRIMARY KEY,
    online_article_id INTEGER NOT NULL REFERENCES online_articles(id) ON DELETE CASCADE,
    source_id INTEGER NOT NULL REFERENCES online_article_sources(id) ON DELETE CASCADE,
    external_score FLOAT,
    metadata JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    UNIQUE(online_article_id, source_id)
);

CREATE INDEX ON online_article_metadata(online_article_id);

CREATE INDEX ON online_article_metadata(source_id);

CREATE TABLE online_article_chunks (
    id SERIAL PRIMARY KEY,
    online_article_id INTEGER NOT NULL REFERENCES online_articles(id) ON DELETE CASCADE,
    embedding vector(384) NOT NULL,
    created_at TIMESTAMP DEFAULT NOW()
);

CREATE INDEX ON online_article_chunks USING hnsw (embedding vector_cosine_ops);

CREATE TABLE user_history (
    id SERIAL PRIMARY KEY,
    online_article_id INTEGER NOT NULL REFERENCES online_articles(id),
    weight FLOAT DEFAULT 0.0,
    added_at TIMESTAMP DEFAULT NOW()
);
