-- The bot can sign in to more than one subscription, one row each
ALTER TABLE "chatgpt_auth" RENAME TO "subscription_auth";
ALTER TABLE "subscription_auth" ADD COLUMN "provider" TEXT;
UPDATE "subscription_auth" SET "provider" = 'chatgpt';
ALTER TABLE "subscription_auth" ALTER COLUMN "provider" SET NOT NULL;
ALTER TABLE "subscription_auth" DROP CONSTRAINT "chatgpt_auth_pkey";
ALTER TABLE "subscription_auth" DROP COLUMN "id";
ALTER TABLE "subscription_auth" ADD CONSTRAINT "subscription_auth_pkey" PRIMARY KEY ("provider");
