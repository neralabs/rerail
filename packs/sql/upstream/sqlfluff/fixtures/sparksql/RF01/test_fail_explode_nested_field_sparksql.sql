SELECT
  tbl.a AS a_new,
  EXPLODE(tbl.b.c) AS a_b_new
FROM test AS tbl
