-- CreateTable
CREATE TABLE "User" ("id" TEXT NOT NULL, "email" TEXT NOT NULL, CONSTRAINT "User_pkey" PRIMARY KEY ("id"));
CREATE TABLE "Order" ("id" TEXT NOT NULL, "email" TEXT NOT NULL, "status" TEXT NOT NULL, CONSTRAINT "Order_pkey" PRIMARY KEY ("id"));
-- CreateIndex
CREATE UNIQUE INDEX "User_email_key" ON "User"("email");
CREATE INDEX "Order_status_idx" ON "Order"("status");
