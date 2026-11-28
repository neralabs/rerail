SELECT set_config('lock_timeout', '4s', false);
ALTER TABLE orders ADD COLUMN external_ref text;
