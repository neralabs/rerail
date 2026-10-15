SELECT * FROM (
    WITH foo AS (
        SELECT col FROM dbo.tab
    )
    SELECT * FROM foo
) AS a
