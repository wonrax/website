-- The recommender ranks with a taste model over sparse features now, so the
-- chunk embeddings go, along with the article text nothing has stored since
-- the terms column replaced it.
DROP TABLE "online_article_chunks";

ALTER TABLE "online_articles" DROP COLUMN "content_text";

-- CreateTable
CREATE TABLE "recommender_feedback" (
    "online_article_id" INTEGER NOT NULL,
    "impressions" INTEGER NOT NULL DEFAULT 0,
    "last_shown_at" TIMESTAMPTZ(6),
    "clicked_at" TIMESTAMPTZ(6),
    "dismissed_at" TIMESTAMPTZ(6),

    CONSTRAINT "recommender_feedback_pkey" PRIMARY KEY ("online_article_id")
);

-- AddForeignKey
ALTER TABLE "recommender_feedback" ADD CONSTRAINT "recommender_feedback_online_article_id_fkey" FOREIGN KEY ("online_article_id") REFERENCES "online_articles"("id") ON DELETE CASCADE ON UPDATE NO ACTION;
