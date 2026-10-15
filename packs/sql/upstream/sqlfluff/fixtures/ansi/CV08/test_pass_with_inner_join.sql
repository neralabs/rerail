WITH cte AS (
    SELECT
        foo.col1,
        bar.col2
    FROM bar
    INNER JOIN foo
        ON foo.bar_id = bar.id
)

SELECT
    col1,
    col2
FROM cte;
