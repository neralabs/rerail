SET lock_timeout = '5s';

-- AlterTable
ALTER TABLE "Invoice" ADD COLUMN "dueAt" TIMESTAMP(3);
