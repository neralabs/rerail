CREATE INDEX CONCURRENTLY "user_email_idx" ON "user" USING btree ("email");--> statement-breakpoint -- caught
ALTER TABLE "user" ADD COLUMN "nickname" text;--> statement-breakpoint
ALTER TABLE "events" DETACH PARTITION "events_2023" CONCURRENTLY; -- caught
