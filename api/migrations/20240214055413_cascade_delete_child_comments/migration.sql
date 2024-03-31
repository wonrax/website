-- DropForeignKey
ALTER TABLE "blog_comments" DROP CONSTRAINT "blog_comments_parent_id_fkey";

-- AddForeignKey
ALTER TABLE "blog_comments" ADD CONSTRAINT "blog_comments_parent_id_fkey" FOREIGN KEY ("parent_id") REFERENCES "blog_comments"("id") ON DELETE CASCADE ON UPDATE CASCADE;
