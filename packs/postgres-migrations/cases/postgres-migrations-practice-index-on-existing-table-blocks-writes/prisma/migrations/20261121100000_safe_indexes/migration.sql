-- Built without blocking writes
CREATE INDEX CONCURRENTLY "Order_notes_idx" ON "Order"("notes");

-- Attaching an index that already exists takes a brief lock only
ALTER TABLE "Order" ADD CONSTRAINT "Order_ref_uq" UNIQUE USING INDEX "Order_ref_key";
ALTER TABLE "Customer" ADD CONSTRAINT "Customer_pkey" PRIMARY KEY USING INDEX "Customer_id_key";

-- A table created in this migration is empty
CREATE TABLE "Invoice" (
    "id" TEXT NOT NULL,
    "orderId" TEXT NOT NULL,
    CONSTRAINT "Invoice_pkey" PRIMARY KEY ("id")
);
CREATE INDEX "Invoice_orderId_idx" ON "Invoice"("orderId");
ALTER TABLE public."Invoice" ADD CONSTRAINT "Invoice_orderId_key" UNIQUE ("orderId");

-- A materialized view created here is new too
CREATE MATERIALIZED VIEW "OrderStats" AS SELECT "email", count(*) AS "n" FROM "Order" GROUP BY "email";
CREATE UNIQUE INDEX "OrderStats_email_key" ON "OrderStats"("email");

-- Emptied just before
TRUNCATE "Draft";
CREATE INDEX "Draft_title_idx" ON "Draft"("title");

-- A partitioned parent's index shell: each partition is indexed on its own
CREATE INDEX "Event_at_idx" ON ONLY "Event"("at");

-- An exclusion constraint has no USING INDEX form to steer to
ALTER TABLE "Booking" ADD CONSTRAINT "Booking_no_overlap" EXCLUDE USING gist ("room" WITH =, "during" WITH &&);

-- Prisma's primary-key swap: the type change rewrites the table under its lock anyway
ALTER TABLE "Customer" DROP CONSTRAINT "Customer_pkey",
ALTER COLUMN "id" SET DATA TYPE TEXT,
ADD CONSTRAINT "Customer_pkey" PRIMARY KEY ("id");

-- A NOT NULL column with no default is added to an empty table only, so the index after it builds on no rows
ALTER TABLE "Coupon" ADD COLUMN "code" TEXT NOT NULL;
CREATE UNIQUE INDEX "Coupon_code_key" ON "Coupon"("code");
