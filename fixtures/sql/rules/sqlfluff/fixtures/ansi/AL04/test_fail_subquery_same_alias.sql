SELECT *
FROM tbl AS t
WHERE t.val IN (SELECT t.val FROM tbl2 AS t)
