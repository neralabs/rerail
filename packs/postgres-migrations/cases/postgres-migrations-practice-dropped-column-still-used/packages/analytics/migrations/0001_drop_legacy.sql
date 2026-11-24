ALTER TABLE "events" DROP COLUMN IF EXISTS "legacy_source";--> statement-breakpoint -- caught
ALTER TABLE "events" DROP COLUMN "plan";--> statement-breakpoint -- caught
ALTER TABLE events DROP COLUMN referrer;
