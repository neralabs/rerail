-- A storage parameter beside an action that takes ACCESS EXCLUSIVE: the statement takes ACCESS EXCLUSIVE
ALTER TABLE "visits" SET (autovacuum_enabled = false), ADD COLUMN "note" text; -- caught
