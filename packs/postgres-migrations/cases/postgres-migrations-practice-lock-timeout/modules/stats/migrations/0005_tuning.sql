ALTER TABLE "visits" SET (
	autovacuum_vacuum_insert_scale_factor = 0.01,
	toast.autovacuum_enabled = false
);
ALTER TABLE "visits" RESET (fillfactor);
ALTER TABLE "events" ALTER COLUMN "kind" SET STATISTICS 500;
ALTER TABLE "events" ALTER COLUMN "kind" SET (n_distinct = -0.5);
ALTER TABLE "events" CLUSTER ON "events_pkey";
ALTER TABLE "events" VALIDATE CONSTRAINT "events_kind_check";
-- Each statement above takes SHARE UPDATE EXCLUSIVE, which blocks no read or write; this one takes ACCESS EXCLUSIVE
ALTER TABLE "visits" SET (user_catalog_table = true); -- caught
