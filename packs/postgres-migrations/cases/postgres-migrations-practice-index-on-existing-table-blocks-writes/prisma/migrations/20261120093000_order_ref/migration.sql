-- AlterTable
ALTER TABLE "Order" ADD COLUMN "ref" TEXT;

-- CreateIndex
CREATE INDEX "Order_email_idx" ON "Order"("email"); -- caught

-- CreateIndex
CREATE UNIQUE INDEX "Order_ref_key" ON "public"."Order"("ref"); -- caught

ALTER TABLE "Order" ADD CONSTRAINT "Order_ref_uq" UNIQUE ("ref"); -- caught

ALTER TABLE "Customer" ADD PRIMARY KEY ("id"); -- caught

ALTER TABLE "Order" ADD COLUMN "code" TEXT UNIQUE; -- caught

-- IF NOT EXISTS changes nothing for Prisma: the index is still built under the lock
CREATE INDEX IF NOT EXISTS "Order_ref_idx" ON "Order"("ref"); -- caught

-- A partitioned parent: reported, with the per-partition fix (no CONCURRENTLY on the parent)
CREATE INDEX "Event_kind_idx" ON "Event"("kind"); -- caught

-- A partition is an ordinary table
CREATE INDEX "Event_2024_kind_idx" ON "Event_2024"("kind"); -- caught

-- A constraint on a partitioned parent has no USING INDEX form: left alone
ALTER TABLE "Event" ADD CONSTRAINT "Event_id_at_key" UNIQUE ("id", "at");
