-- CreateTable
CREATE TABLE "blog_posts" (
    "id" SERIAL NOT NULL,
    "category" TEXT NOT NULL,
    "slug" TEXT NOT NULL,
    "title" TEXT,

    CONSTRAINT "blog_posts_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "blog_comments" (
    "id" SERIAL NOT NULL,
    "author_ip" TEXT NOT NULL,
    "author_name" TEXT,
    "author_email" TEXT,
    "identity_id" INTEGER,
    "content" TEXT NOT NULL,
    "post_id" INTEGER NOT NULL,
    "parent_id" INTEGER,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "blog_comments_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "blog_comment_votes" (
    "id" SERIAL NOT NULL,
    "comment_id" INTEGER NOT NULL,
    "ip" TEXT,
    "indentity_id" INTEGER,
    "score" INTEGER NOT NULL DEFAULT 1,
    "created_at" TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "blog_comment_votes_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "identities" (
    "id" SERIAL NOT NULL,
    "traits" JSONB NOT NULL DEFAULT '{}',
    "created_at" TIMESTAMP(6) NOT NULL,
    "updated_at" TIMESTAMP(6) NOT NULL,

    CONSTRAINT "identities_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "identity_credential_types" (
    "id" SERIAL NOT NULL,
    "name" VARCHAR(64) NOT NULL,
    "created_at" TIMESTAMP(6) NOT NULL,

    CONSTRAINT "identity_credential_types_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "identity_credentials" (
    "id" SERIAL NOT NULL,
    "credential" JSONB,
    "credential_type_id" INTEGER NOT NULL,
    "identity_id" INTEGER NOT NULL,
    "created_at" TIMESTAMP(6) NOT NULL,
    "updated_at" TIMESTAMP(6) NOT NULL,

    CONSTRAINT "identity_credentials_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "sessions" (
    "id" SERIAL NOT NULL,
    "token" VARCHAR(133) NOT NULL,
    "active" BOOLEAN NOT NULL,
    "issued_at" TIMESTAMP(6) NOT NULL,
    "expires_at" TIMESTAMP(6) NOT NULL,
    "identity_id" INTEGER NOT NULL,
    "created_at" TIMESTAMP(6) NOT NULL,
    "updated_at" TIMESTAMP(6) NOT NULL,

    CONSTRAINT "sessions_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE UNIQUE INDEX "blog_posts_category_slug_key" ON "blog_posts"("category", "slug");

-- CreateIndex
CREATE UNIQUE INDEX "identity_credential_types_name_key" ON "identity_credential_types"("name");

-- CreateIndex
CREATE UNIQUE INDEX "identity_credentials_credential_key" ON "identity_credentials"("credential");

-- CreateIndex
CREATE INDEX "identity_credentials_credential_idx" ON "identity_credentials" USING GIN ("credential");

-- CreateIndex
CREATE UNIQUE INDEX "sessions_token_key" ON "sessions"("token");

-- AddForeignKey
ALTER TABLE "blog_comments" ADD CONSTRAINT "blog_comments_identity_id_fkey" FOREIGN KEY ("identity_id") REFERENCES "identities"("id") ON DELETE NO ACTION ON UPDATE NO ACTION;

-- AddForeignKey
ALTER TABLE "blog_comments" ADD CONSTRAINT "blog_comments_parent_id_fkey" FOREIGN KEY ("parent_id") REFERENCES "blog_comments"("id") ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "blog_comments" ADD CONSTRAINT "blog_comments_post_id_fkey" FOREIGN KEY ("post_id") REFERENCES "blog_posts"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "blog_comment_votes" ADD CONSTRAINT "blog_comment_votes_comment_id_fkey" FOREIGN KEY ("comment_id") REFERENCES "blog_comments"("id") ON DELETE NO ACTION ON UPDATE NO ACTION;

-- AddForeignKey
ALTER TABLE "identity_credentials" ADD CONSTRAINT "identity_credentials_credential_type_id_fkey" FOREIGN KEY ("credential_type_id") REFERENCES "identity_credential_types"("id") ON DELETE NO ACTION ON UPDATE NO ACTION;

-- AddForeignKey
ALTER TABLE "identity_credentials" ADD CONSTRAINT "identity_credentials_identity_id_fkey" FOREIGN KEY ("identity_id") REFERENCES "identities"("id") ON DELETE NO ACTION ON UPDATE NO ACTION;

-- AddForeignKey
ALTER TABLE "sessions" ADD CONSTRAINT "sessions_identity_id_fkey" FOREIGN KEY ("identity_id") REFERENCES "identities"("id") ON DELETE NO ACTION ON UPDATE NO ACTION;
