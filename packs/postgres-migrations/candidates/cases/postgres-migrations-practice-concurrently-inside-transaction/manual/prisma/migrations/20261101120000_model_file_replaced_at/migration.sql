-- Applied by hand, one statement at a time: this repository runs no `prisma migrate deploy`.
ALTER TABLE "ModelFile" ADD COLUMN IF NOT EXISTS "replacedAt" timestamptz;
CREATE INDEX CONCURRENTLY IF NOT EXISTS "ModelFile_replacedAt_idx" ON "ModelFile" ("replacedAt") WHERE "replacedAt" IS NOT NULL;

-- psql still refuses this one: the explicit transaction wraps it
BEGIN;
DROP INDEX CONCURRENTLY IF EXISTS "ModelFile_old_idx"; -- caught
COMMIT;
