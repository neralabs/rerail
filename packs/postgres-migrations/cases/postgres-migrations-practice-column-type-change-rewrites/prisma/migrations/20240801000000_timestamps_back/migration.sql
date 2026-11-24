-- AlterTable: TIMESTAMPTZ back to TIMESTAMP skips the rewrite under TimeZone UTC, as the other direction does: left
-- alone alike
ALTER TABLE "Event" ALTER COLUMN "startsAt" SET DATA TYPE TIMESTAMP(3);
