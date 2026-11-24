ALTER TABLE "orders" ADD COLUMN "total" integer DEFAULT 0 NOT NULL;--> statement-breakpoint
ALTER TABLE "orders" ADD CONSTRAINT "orders_total_check" CHECK ("orders"."total" >= 0);--> statement-breakpoint -- caught
ALTER TABLE orders ADD FOREIGN KEY (user_id) REFERENCES users (id); -- caught
TRUNCATE TABLE "order_drafts";--> statement-breakpoint
ALTER TABLE "order_drafts" ADD CONSTRAINT "order_drafts_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;
