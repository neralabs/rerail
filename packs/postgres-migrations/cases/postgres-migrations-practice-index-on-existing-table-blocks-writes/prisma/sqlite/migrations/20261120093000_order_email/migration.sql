-- SQLite has no CONCURRENTLY: the self-hosted SQLite build of the same schema is left alone
CREATE INDEX "Order_email_idx" ON "Order"("email");
