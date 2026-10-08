SELECT
    foo.col1,
    bar.col2,
    baz.col3
FROM foo
RIGHT JOIN bar
    ON foo.bar_id = bar.id
INNER JOIN baz
    ON foo.baz_id = baz.id;
