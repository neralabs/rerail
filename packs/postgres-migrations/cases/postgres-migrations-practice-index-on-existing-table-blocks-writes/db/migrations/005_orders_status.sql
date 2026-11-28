BEGIN;
ALTER TABLE orders ADD COLUMN status text;
CREATE INDEX orders_status_idx ON orders (status); -- caught
COMMIT;
