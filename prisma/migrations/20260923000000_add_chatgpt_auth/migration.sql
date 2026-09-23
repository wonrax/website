-- CreateTable
CREATE TABLE "chatgpt_auth" (
    "id" INTEGER NOT NULL,
    "access_token" TEXT NOT NULL,
    "refresh_token" TEXT NOT NULL,
    "id_token" TEXT,
    "account_id" TEXT,
    "expires_at" TIMESTAMPTZ(6) NOT NULL,
    "refreshed_at" TIMESTAMPTZ(6) NOT NULL,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "chatgpt_auth_pkey" PRIMARY KEY ("id")
);
