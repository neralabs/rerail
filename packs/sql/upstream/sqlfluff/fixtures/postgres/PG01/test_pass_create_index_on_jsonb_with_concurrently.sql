CREATE INDEX CONCURRENTLY idx_foo ON bar (tenant_id, (event ->> 'id'));
