ALTER TABLE "posts" ADD COLUMN "tag" text;--> statement-breakpoint
CREATE INDEX "posts_tag_idx" ON "posts" USING btree ("tag"); -- caught
--> statement-breakpoint
-- The form the Drizzle fix asks for: built CONCURRENTLY by hand before the deploy, the migration only finds it
CREATE INDEX IF NOT EXISTS "posts_slug_tag_idx" ON "posts" USING btree ("slug", "tag");--> statement-breakpoint
CREATE UNIQUE INDEX IF NOT EXISTS "posts_tag_slug_key" ON "posts" USING btree ("tag", "slug");
