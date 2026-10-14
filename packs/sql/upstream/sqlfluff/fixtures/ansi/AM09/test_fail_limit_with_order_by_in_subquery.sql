SELECT *
FROM (
    SELECT *
    FROM foo
    LIMIT 10
) subquery;
