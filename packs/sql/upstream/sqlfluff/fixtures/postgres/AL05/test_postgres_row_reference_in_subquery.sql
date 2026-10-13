SELECT
    (x).key
FROM (
    SELECT EACH(HSTORE(t)) AS x
    FROM member_plan AS t
) AS q
