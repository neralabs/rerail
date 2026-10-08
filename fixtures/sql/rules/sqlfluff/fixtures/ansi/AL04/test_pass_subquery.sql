SELECT 1
FROM (
    select 1
    from table_1 as a
    join table_2 as b on a.pk = b.pk
) AS a
