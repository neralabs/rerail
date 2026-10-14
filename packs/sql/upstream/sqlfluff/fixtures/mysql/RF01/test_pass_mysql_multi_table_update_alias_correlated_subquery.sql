UPDATE t1 AS a
INNER JOIN t2 AS b ON a.id = b.id
SET a.x = (
    SELECT SUM(c.v)
    FROM t3 AS c
    WHERE c.k = b.id
);
