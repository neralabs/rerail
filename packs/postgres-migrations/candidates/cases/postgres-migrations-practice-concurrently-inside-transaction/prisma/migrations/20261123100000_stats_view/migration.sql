-- AlterTable
ALTER TABLE "Order" ADD COLUMN "archived" BOOLEAN NOT NULL DEFAULT false;
-- REFRESH … CONCURRENTLY may run inside a transaction
REFRESH MATERIALIZED VIEW CONCURRENTLY "order_stats";
