-- Covering index for the history_chunks CTE: includes embedding so the join
-- can be satisfied from the index without hitting the main table
CREATE INDEX idx_online_article_chunks_article_embedding 
    ON online_article_chunks(online_article_id) INCLUDE (embedding);

-- Index on user_history weight for faster filtering/sorting by weight
CREATE INDEX idx_user_history_weight ON user_history(weight DESC);
