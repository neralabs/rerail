-- AlterTable: stable and constant defaults are stored in the catalog, no rewrite
ALTER TABLE "Order" ADD COLUMN "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
ADD COLUMN "updatedAt" TIMESTAMP(3) NOT NULL DEFAULT now(),
ADD COLUMN "active" BOOLEAN NOT NULL DEFAULT true;

-- AlterTable: a virtual generated column is computed on read (PostgreSQL 18)
ALTER TABLE "Order" ADD COLUMN "label" TEXT GENERATED ALWAYS AS (upper("name")) VIRTUAL;

-- AlterTable: the safe sequence, a default for new rows only
ALTER TABLE "Order" ADD COLUMN "ref" UUID;
ALTER TABLE "Order" ALTER COLUMN "ref" SET DEFAULT gen_random_uuid();

-- AlterTable: a function of unknown volatility
ALTER TABLE "Order" ADD COLUMN "code" TEXT DEFAULT make_order_code();

-- The table is emptied first
TRUNCATE "Draft";
ALTER TABLE "Draft" ADD COLUMN "id" BIGSERIAL;
