-- This package's runner wraps every migration in one transaction (it mentions CONCURRENTLY only in a comment), and
-- the other package's runner, which would not, does not run this folder
CREATE INDEX CONCURRENTLY "invoice_due_idx" ON "invoice" USING btree ("dueAt"); -- caught
