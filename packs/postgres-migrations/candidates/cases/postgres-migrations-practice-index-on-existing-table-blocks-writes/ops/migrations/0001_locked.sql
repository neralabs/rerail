-- Run with writes stopped: the table is locked first, for the whole transaction
BEGIN;
LOCK TABLE sync_control IN ACCESS EXCLUSIVE MODE;
ALTER TABLE sync_control DROP CONSTRAINT sync_control_pkey;
ALTER TABLE sync_control ADD PRIMARY KEY (workspace_id, sync_id, run_id);
CREATE INDEX IF NOT EXISTS sync_control_sync_id_idx ON sync_control (sync_id);
COMMIT;
