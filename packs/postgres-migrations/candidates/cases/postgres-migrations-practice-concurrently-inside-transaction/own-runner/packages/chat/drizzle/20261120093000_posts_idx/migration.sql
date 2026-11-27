-- The package's own runner (scripts/run-migrations.mjs) runs a migration holding CONCURRENTLY statement by statement,
-- outside a transaction: not reported
CREATE INDEX CONCURRENTLY IF NOT EXISTS "post_author_idx" ON "post" ("authorId");--> statement-breakpoint
CREATE INDEX CONCURRENTLY IF NOT EXISTS "post_created_idx" ON "post" ("createdAt");
