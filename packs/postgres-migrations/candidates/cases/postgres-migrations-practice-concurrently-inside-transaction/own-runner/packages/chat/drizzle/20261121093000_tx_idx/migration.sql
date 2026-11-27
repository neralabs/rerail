-- The same runner, but the file opens its own transaction: PostgreSQL refuses it whatever runs the file
BEGIN;--> statement-breakpoint
CREATE INDEX CONCURRENTLY "post_title_idx" ON "post" ("title");--> statement-breakpoint -- caught
COMMIT;
