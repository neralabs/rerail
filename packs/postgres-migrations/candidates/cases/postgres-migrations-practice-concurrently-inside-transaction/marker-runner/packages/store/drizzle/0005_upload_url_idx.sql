-- no-transaction
-- A marker the package's runner does not read (it reads its own, prefixed one): the file runs in BEGIN ... COMMIT
CREATE INDEX CONCURRENTLY "upload_url_idx" ON "upload" USING btree ("url"); -- caught
