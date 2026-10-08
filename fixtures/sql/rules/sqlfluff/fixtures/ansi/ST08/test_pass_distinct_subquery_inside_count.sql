SELECT
    COUNT(
        DISTINCT(
            SELECT ANY_VALUE(id)
            FROM UNNEST(tag) t
        )
    )
FROM
    dataset_name.table_name;
