SELECT
  a.foo
  , b.bar
FROM foo.bar a
INNER JOIN foo.qux 'B'
  ON a.thing = b.thing
;
