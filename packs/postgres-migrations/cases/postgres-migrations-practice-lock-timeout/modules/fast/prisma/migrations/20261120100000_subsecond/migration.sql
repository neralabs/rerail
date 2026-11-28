SET lock_timeout = '0.5s';

-- AlterTable
ALTER TABLE "Fast" ADD COLUMN "seenAt" TIMESTAMP(3);
