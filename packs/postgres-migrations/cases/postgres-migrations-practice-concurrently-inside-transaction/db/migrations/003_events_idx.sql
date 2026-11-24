-- Plain SQL migration, applied one statement at a time by scripts/migrate.ts
CREATE INDEX CONCURRENTLY IF NOT EXISTS "events_created_at_idx" ON "events" ("created_at");
ALTER TABLE "events" ADD COLUMN "source" text;

BEGIN;
ALTER TABLE "sessions" ADD COLUMN "ip" inet;
CREATE INDEX CONCURRENTLY "sessions_ip_idx" ON "sessions" ("ip"); -- caught
COMMIT;
