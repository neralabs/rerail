DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'entry' AND column_name = 'note') THEN
    ALTER TABLE entry ADD COLUMN note text;
  END IF;
END
$$;
