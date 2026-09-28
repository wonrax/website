-- An observation whose source messages were all deleted is withdrawn: search stops finding it,
-- and the docs that took it in let go of it at their next dream. `withdrawn_seq` comes from the
-- ID sequence, which orders the withdrawal in the log among the observations, so a doc's
-- `dreamed_through` also tells whether it has seen the withdrawal.
ALTER TABLE "discord_memory_observations"
    ADD COLUMN "withdrawn_at" TIMESTAMPTZ(6),
    ADD COLUMN "withdrawn_seq" BIGINT;

-- Deletions look up the observations citing a message
CREATE INDEX "discord_memory_observations_source_message_ids_idx" ON "discord_memory_observations" USING GIN ("source_message_ids");
