-- CreateTable
CREATE TABLE "Order" (
    "id" TEXT NOT NULL,
    "email" TEXT NOT NULL,
    CONSTRAINT "Order_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "Customer" (
    "id" TEXT NOT NULL,
    "name" TEXT NOT NULL
);

-- CreateIndex
CREATE UNIQUE INDEX "Order_email_key" ON "Order"("email");

-- CreateTable: a partitioned parent
CREATE TABLE "Event" (
    "id" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "at" TIMESTAMP(3) NOT NULL
) PARTITION BY RANGE ("at");

CREATE TABLE "Event_2024" PARTITION OF "Event" FOR VALUES FROM ('2024-01-01') TO ('2025-01-01');
