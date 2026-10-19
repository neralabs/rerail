SELECT outer_tbl.x
FROM outer_tbl
WHERE outer_tbl.id IN (
    SELECT bogus.id FROM t2 AS b JOIN t3 AS c ON b.id = c.id
)
