CREATE TABLE audit_log (id bigserial PRIMARY KEY, at timestamptz NOT NULL);
CREATE INDEX audit_log_at_idx ON audit_log (at);
CREATE INDEX orders_created_idx -- caught
  -- a comment naming `orders` in backticks is no sign of MySQL
  ON orders (created_at);
