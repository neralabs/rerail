-- AlterTable
ALTER TABLE "Inbox" ADD COLUMN "readAt" TIMESTAMP(3); -- caught

SET lock_timeout = '3s';

-- AlterTable
ALTER TABLE "Inbox" ADD COLUMN "archivedAt" TIMESTAMP(3);
