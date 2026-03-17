-- Requires pgvector 0.7.0+ for binary_quantize() and Hamming distance on bit vectors.
DROP INDEX IF EXISTS online_article_chunks_embedding_idx;

DROP INDEX IF EXISTS idx_online_article_chunks_article_embedding;

ALTER TABLE
    online_article_chunks
ALTER COLUMN
    embedding TYPE BIT(384) USING binary_quantize(embedding) :: BIT(384);

CREATE INDEX idx_online_article_chunks_article_embedding ON online_article_chunks(online_article_id) INCLUDE (embedding);
