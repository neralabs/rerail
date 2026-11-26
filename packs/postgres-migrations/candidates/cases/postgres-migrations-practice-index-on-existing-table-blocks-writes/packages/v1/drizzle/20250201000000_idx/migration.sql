-- drizzle-kit 1.0's folder layout (a snapshot.json beside each migration.sql, no meta/_journal.json) is Drizzle's
CREATE INDEX "users_name_idx" ON "users" USING btree ("name"); -- caught
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "users_name_id_idx" ON "users" USING btree ("name", "id");
