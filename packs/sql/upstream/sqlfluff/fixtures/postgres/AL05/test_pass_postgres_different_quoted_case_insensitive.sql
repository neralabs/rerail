SELECT
  a.col1
  , B.id
FROM prd.tbl1 a
INNER JOIN tbl1 "b"
  ON a.col1 = B.id
;
