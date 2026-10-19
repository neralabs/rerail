SELECT
  a.test1,
  a.test2,
  b.test3
FROM table1 AS a,
  LATERAL flatten(input => some_field) AS b,
  LATERAL flatten(input => b.value) AS c,
  LATERAL flatten(input => c.value) AS d,
  LATERAL flatten(input => d.value) AS e,
  LATERAL flatten(input => e.value) AS f;
