SELECT table1.c1
FROM
    table1 AS tbl1
INNER JOIN table2 AS tbl2 ON (tbl2.col2 = tbl1.col2)
INNER JOIN table3 AS tbl3 ON (tbl3.col3 = tbl2.col3)
