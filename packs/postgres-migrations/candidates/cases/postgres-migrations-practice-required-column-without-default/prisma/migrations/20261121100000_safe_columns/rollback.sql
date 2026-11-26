-- A rollback script kept beside the migration (civitai keeps rollback.sql and backfill.sql there): Prisma never runs
-- it, so it is not a migration, and nothing in it is reported
ALTER TABLE "Post" ADD COLUMN "legacySlug" TEXT NOT NULL;
