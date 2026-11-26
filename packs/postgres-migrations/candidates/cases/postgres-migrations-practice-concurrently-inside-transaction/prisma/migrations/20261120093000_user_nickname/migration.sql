-- AlterTable
ALTER TABLE "User" ADD COLUMN "nickname" TEXT;
-- CreateIndex
CREATE INDEX CONCURRENTLY "User_nickname_idx" ON "User"("nickname"); -- caught
