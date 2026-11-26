-- Run in a maintenance window: the table is locked first, so nothing after it waits for another session
BEGIN;
LOCK TABLE accounts IN ACCESS EXCLUSIVE MODE;
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS run_order bigint NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS accounts_run_order_idx ON accounts (run_order);
COMMIT;
