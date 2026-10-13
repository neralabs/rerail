SELECT a.x
FROM tbl_a AS a
INNER JOIN ((tbl_b AS b
    INNER JOIN tbl_c AS c
        ON b.id = c.id))
    ON a.id = b.id;
