SELECT * FROM (
    SELECT col FROM dbo.tab
    UNION
    SELECT -1 AS col
) AS a
