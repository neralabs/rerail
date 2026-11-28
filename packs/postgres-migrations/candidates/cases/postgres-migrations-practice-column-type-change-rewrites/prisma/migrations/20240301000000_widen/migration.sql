-- AlterTable: widenings PostgreSQL makes without a rewrite
ALTER TABLE "Event" ALTER COLUMN "name" SET DATA TYPE VARCHAR(255);
ALTER TABLE "Event" ALTER COLUMN "slug" SET DATA TYPE TEXT;
ALTER TABLE "Event" ALTER COLUMN "price" SET DATA TYPE DECIMAL(14,2);
ALTER TABLE "Event" ALTER COLUMN "startsAt" SET DATA TYPE TIMESTAMPTZ(3);
ALTER TABLE "Event" ALTER COLUMN "tags" SET DATA TYPE VARCHAR(50)[];
-- the old type of "legacyCode" is in no earlier migration (history was squashed)
ALTER TABLE "Event" ALTER COLUMN "legacyCode" SET DATA TYPE VARCHAR(10);
-- a USING that only casts the column to its type changes nothing
ALTER TABLE "Event" ALTER COLUMN "slug" TYPE TEXT USING "slug"::TEXT;
-- TEXT to CITEXT is a binary cast
ALTER TABLE "Event" ALTER COLUMN "title" SET DATA TYPE CITEXT;
