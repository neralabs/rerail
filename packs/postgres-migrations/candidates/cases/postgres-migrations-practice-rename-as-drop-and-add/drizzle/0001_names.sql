ALTER TABLE "users" DROP COLUMN IF EXISTS "full_name";--> statement-breakpoint -- caught
ALTER TABLE "users" ADD COLUMN "name" varchar(255);
