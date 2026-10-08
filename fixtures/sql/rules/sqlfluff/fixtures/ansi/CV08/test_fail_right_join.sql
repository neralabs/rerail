SELECT
    foo.col1,
    bar.col2
FROM foo
RIGHT JOIN bar
    ON foo.bar_id = bar.id;
