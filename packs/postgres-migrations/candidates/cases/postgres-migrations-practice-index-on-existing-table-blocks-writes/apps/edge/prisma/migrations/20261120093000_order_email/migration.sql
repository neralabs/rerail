-- Its migration_lock.toml names SQLite, though no folder name says so: left alone
CREATE INDEX "Order_email_idx" ON "Order"("email");
