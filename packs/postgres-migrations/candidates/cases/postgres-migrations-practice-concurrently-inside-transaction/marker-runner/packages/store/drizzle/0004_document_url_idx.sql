-- acme:no-transaction
-- Reason: the index builds on a live table, so the package's runner applies this file outside a transaction
CREATE INDEX CONCURRENTLY "document_url_idx" ON "document" USING btree ("url");--> statement-breakpoint
CREATE INDEX CONCURRENTLY "document_owner_idx" ON "document" USING btree ("ownerId");
