CREATE INDEX ON online_article_chunks(online_article_id);

CREATE INDEX ON user_history(online_article_id);

-- Useful if filtering feed_items by recency (e.g. WHERE created_at > NOW() - INTERVAL '14 days')
CREATE INDEX ON online_articles(created_at);
