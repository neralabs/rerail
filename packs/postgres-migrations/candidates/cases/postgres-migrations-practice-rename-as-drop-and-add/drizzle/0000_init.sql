CREATE TABLE IF NOT EXISTS "users" (
	"id" serial PRIMARY KEY NOT NULL,
	"full_name" varchar(255),
	"email" varchar(320) NOT NULL
);
