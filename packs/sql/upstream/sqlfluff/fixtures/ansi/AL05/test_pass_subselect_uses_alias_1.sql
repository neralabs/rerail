SELECT
    col1,
    (
        SELECT count(*)
        FROM base
        WHERE a.col2 = base.col2
    )
FROM
    without_dup AS a
