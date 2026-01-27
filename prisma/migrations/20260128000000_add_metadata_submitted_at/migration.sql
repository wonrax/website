-- Add submitted_at column to online_article_metadata for storing the actual submission time from HN/Lobsters
-- Backfill existing rows with created_at as fallback
ALTER TABLE online_article_metadata ADD COLUMN submitted_at TIMESTAMP;
UPDATE online_article_metadata SET submitted_at = created_at WHERE submitted_at IS NULL;
ALTER TABLE online_article_metadata ALTER COLUMN submitted_at SET NOT NULL;

-- Index for freshness queries
CREATE INDEX idx_online_article_metadata_submitted_at ON online_article_metadata(submitted_at);
