-- drizzle-kit 1.0's folder layout (a snapshot.json beside each migration.sql, no meta/_journal.json) is Drizzle's:
-- its migrator still runs every pending migration in one transaction
CREATE INDEX CONCURRENTLY "users_email_idx" ON "users" USING btree ("email"); -- caught
