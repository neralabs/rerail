SELECT a.x
FROM outer_tbl AS a
WHERE a.id IN (
    SELECT b.id FROM t2 AS b JOIN t3 AS c ON b.id = c.id
    WHERE c.id = a.id
)
