ALTER TABLE "invoices" ADD COLUMN "number" serial8 NOT NULL;--> statement-breakpoint -- caught
CREATE SEQUENCE IF NOT EXISTS "receipt_no_seq";--> statement-breakpoint
ALTER TABLE "receipts" ADD COLUMN IF NOT EXISTS "no" integer DEFAULT nextval('receipt_no_seq'); -- caught
