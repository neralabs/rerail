ALTER TABLE "audit_log" ADD COLUMN "actor_id" uuid NOT NULL;--> statement-breakpoint -- caught
DELETE FROM "audit_log" WHERE "created_at" < now() - interval '90 days';--> statement-breakpoint
ALTER TABLE "audit_log" ADD COLUMN "tenant_id" uuid;
