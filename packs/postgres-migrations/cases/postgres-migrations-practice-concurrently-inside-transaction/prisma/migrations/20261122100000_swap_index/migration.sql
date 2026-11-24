-- Rebuild the status index and drop the old one
CREATE INDEX "Order_status_v2_idx" ON "Order"("status");
DROP INDEX CONCURRENTLY "Order_status_idx"; -- caught
ALTER INDEX "Order_status_v2_idx" RENAME TO "Order_status_idx";
-- Rebuild the bloated email index
REINDEX INDEX CONCURRENTLY "Order_email_idx"; -- caught
