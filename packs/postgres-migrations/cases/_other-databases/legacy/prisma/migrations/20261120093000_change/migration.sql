-- Statements every rule of the pack reports on a PostgreSQL table that already exists; this folder is another
-- database's, as its ORM declares, so none is reported.
ALTER TABLE "User" ADD COLUMN "flag" BOOLEAN;
CREATE INDEX "User_email_idx" ON "User"("email");
ALTER TABLE "Post" ADD CONSTRAINT "Post_authorId_fkey" FOREIGN KEY ("authorId") REFERENCES "User"("id");
ALTER TABLE "User" ALTER COLUMN "name" SET NOT NULL;
ALTER TABLE "User" ADD COLUMN "token" TEXT DEFAULT gen_random_uuid();
ALTER TABLE "User" ALTER COLUMN "age" SET DATA TYPE BIGINT;
ALTER TABLE "User" DROP COLUMN "bio", ADD COLUMN "bioText" TEXT;
ALTER TABLE "User" DROP COLUMN "legacy";
CREATE INDEX CONCURRENTLY "User_age_idx" ON "User"("age");
ALTER TABLE "User" ADD COLUMN "code" TEXT NOT NULL;
