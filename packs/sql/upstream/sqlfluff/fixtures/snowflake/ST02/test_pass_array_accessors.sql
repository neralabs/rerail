SELECT
CASE
    WHEN genres[0] IS NULL
       THEN 'x'
    ELSE genres
END
AS g
FROM table_t
