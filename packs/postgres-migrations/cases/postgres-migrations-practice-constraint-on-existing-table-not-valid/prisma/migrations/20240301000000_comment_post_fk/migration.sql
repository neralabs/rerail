-- AlterTable
ALTER TABLE "Comment" ADD COLUMN "authorId" TEXT;

-- AddForeignKey
ALTER TABLE "Comment" ADD CONSTRAINT "Comment_postId_fkey" FOREIGN KEY ("postId") REFERENCES "Post"("id") ON DELETE CASCADE ON UPDATE CASCADE; -- caught

-- AddForeignKey
ALTER TABLE "Comment" ADD CONSTRAINT "Comment_authorId_fkey" FOREIGN KEY ("authorId") REFERENCES "User"("id") NOT VALID;

-- ValidateForeignKey
ALTER TABLE "Comment" VALIDATE CONSTRAINT "Comment_authorId_fkey";

-- AddCheck
ALTER TABLE "Comment" ADD CONSTRAINT "Comment_body_len" CHECK (length("body") <= 10000); -- caught

-- AddCheck
ALTER TABLE "User" ADD CONSTRAINT "User_email_lower" CHECK ("email" = lower("email")) NOT VALID;

-- AddUnique
ALTER TABLE "User" ADD CONSTRAINT "User_email_key" UNIQUE ("email");
