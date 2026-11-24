-- CreateIndex: alone in its migration, so Prisma sends no batch and PostgreSQL opens no transaction
CREATE INDEX CONCURRENTLY "Order_email_idx" ON "Order"("email");
