-- CreateTable
CREATE TABLE "view_counts" (
    "id" SERIAL NOT NULL,
    "app" VARCHAR(32) NOT NULL,
    "key" VARCHAR(256) NOT NULL,
    "count" BIGINT NOT NULL DEFAULT 0,

    CONSTRAINT "view_counts_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE UNIQUE INDEX "view_counts_app_key" ON "view_counts"("app", "key");
