CREATE INDEX idx_foo ON bar (tenant_id, (event ->> 'id'));
