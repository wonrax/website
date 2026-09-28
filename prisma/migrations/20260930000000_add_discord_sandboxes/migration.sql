-- CreateTable
CREATE TABLE "discord_sandboxes" (
    "channel_id" BIGINT NOT NULL,
    "last_used_at" TIMESTAMPTZ(6) NOT NULL,

    CONSTRAINT "discord_sandboxes_pkey" PRIMARY KEY ("channel_id")
);
