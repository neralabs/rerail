-- A statement inside a function body runs when the function is called, not when the migration runs.
CREATE OR REPLACE FUNCTION archive_partition(p TEXT) RETURNS void AS $$
BEGIN
  EXECUTE 'ALTER TABLE ' || p || ' ALTER COLUMN "archivedAt" SET NOT NULL';
END;
$$ LANGUAGE plpgsql;
