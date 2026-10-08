SELECT *
FROM (
    SELECT *
    FROM foo
    ORDER BY id
    LIMIT 10
) subquery;
