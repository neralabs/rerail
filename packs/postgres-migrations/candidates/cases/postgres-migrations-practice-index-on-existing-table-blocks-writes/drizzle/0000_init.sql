CREATE TABLE IF NOT EXISTS "posts" (
	"id" serial PRIMARY KEY NOT NULL,
	"slug" text NOT NULL
);
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "posts_slug_idx" ON "posts" USING btree ("slug");
