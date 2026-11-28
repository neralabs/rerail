-- A rollback script kept beside the migration (a rollback.sql or backfill.sql kept there by hand): Prisma never runs
-- it, so it is not a migration, and nothing in it is reported
ALTER TABLE "Post" ADD COLUMN "legacySlug" TEXT NOT NULL;
