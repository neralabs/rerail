UPDATE tbl AS dest SET
    t.title = 'TEST'
WHERE t.id = 101 AND EXISTS (
    SELECT 1 FROM foobar AS tmp WHERE tmp.idx = dest.idx)
