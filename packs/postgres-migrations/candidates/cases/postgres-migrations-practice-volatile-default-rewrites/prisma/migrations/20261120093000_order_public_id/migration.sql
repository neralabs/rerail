-- AlterTable
ALTER TABLE "Order" ADD COLUMN "publicId" UUID NOT NULL DEFAULT gen_random_uuid(); -- caught

-- AlterTable
ALTER TABLE "Order" ADD COLUMN "seq" SERIAL NOT NULL; -- caught

-- AlterTable
ALTER TABLE "Order" ADD COLUMN "token" TEXT DEFAULT md5(random()::text), -- caught
ADD COLUMN "number" BIGINT GENERATED ALWAYS AS IDENTITY; -- caught

-- AlterTable
ALTER TABLE "Order" ADD COLUMN "search" TEXT GENERATED ALWAYS AS (lower("name")) STORED; -- caught

-- CreateTable
CREATE TABLE "Tag" (
    "id" UUID NOT NULL DEFAULT gen_random_uuid(),
    "orderId" TEXT NOT NULL,
    CONSTRAINT "Tag_pkey" PRIMARY KEY ("id")
);

-- AlterTable: "Tag" was created above, so it is empty
ALTER TABLE "Tag" ADD COLUMN "seq" SERIAL NOT NULL;
