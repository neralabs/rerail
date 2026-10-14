SELECT col_1
FROM table_a AS a
WHERE NOT EXISTS (SELECT TRUE FROM table_b AS b WHERE `a`.col_4 = b.col_1)
