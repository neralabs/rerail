-- AlterTable
ALTER TABLE "Invoice" ADD COLUMN "paidAt" TIMESTAMP(3);
-- CreateIndex: Prisma 7.4+ sends each statement on its own, so no transaction surrounds it
CREATE INDEX CONCURRENTLY "Invoice_paidAt_idx" ON "Invoice"("paidAt");
