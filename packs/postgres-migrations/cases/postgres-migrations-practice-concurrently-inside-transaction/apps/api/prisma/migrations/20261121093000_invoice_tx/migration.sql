-- Keep the column and its index together
BEGIN;
ALTER TABLE "Invoice" ADD COLUMN "dueAt" TIMESTAMP(3);
CREATE INDEX CONCURRENTLY "Invoice_dueAt_idx" ON "Invoice"("dueAt"); -- caught
COMMIT;
