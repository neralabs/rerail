ALTER TABLE "orders" ALTER COLUMN "customer_id" SET NOT NULL;--> statement-breakpoint -- caught
CREATE TABLE IF NOT EXISTS "invoices" (
	"id" serial PRIMARY KEY NOT NULL,
	"order_id" integer
);
--> statement-breakpoint
ALTER TABLE invoices ALTER order_id SET NOT NULL;
