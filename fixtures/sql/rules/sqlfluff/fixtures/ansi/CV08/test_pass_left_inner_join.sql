SELECT
    foo.col1,
    bar.col2,
    baz.col3
FROM bar
LEFT JOIN foo
    ON foo.bar_id = bar.id
INNER JOIN baz
    ON foo.baz_id = baz.id;
