UPDATE t1 AS a, t2 AS b
SET a.x = (
    SELECT SUM(c.v)
    FROM t3 AS c
    WHERE c.k = b.id
);
