CREATE TABLE IF NOT EXISTS "events" (
	"id" serial PRIMARY KEY NOT NULL,
	"legacy_source" text,
	"plan" text,
	"referrer" text,
	"created_at" timestamp DEFAULT now() NOT NULL
);
