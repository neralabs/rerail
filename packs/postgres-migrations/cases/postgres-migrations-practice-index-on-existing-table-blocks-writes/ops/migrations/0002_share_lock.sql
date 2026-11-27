-- A SHARE lock stops writes, so the index build adds nothing; the UNIQUE constraint also stops reads
BEGIN;
LOCK TABLE ledger IN SHARE MODE;
CREATE INDEX ledger_account_idx ON ledger (account_id);
ALTER TABLE ledger ADD CONSTRAINT ledger_ref_key UNIQUE (ref); -- caught
COMMIT;
