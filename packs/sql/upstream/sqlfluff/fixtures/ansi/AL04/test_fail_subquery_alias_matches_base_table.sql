SELECT *
FROM tbl
WHERE val IN (SELECT tbl.val FROM tbl2 AS tbl)
