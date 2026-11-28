ALTER TABLE "orders" ALTER COLUMN "total" SET DATA TYPE numeric(12, 2); -- caught
--> statement-breakpoint
ALTER TABLE "orders" ALTER COLUMN "note" SET DATA TYPE text;
