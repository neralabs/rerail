SELECT ci.id
FROM source_table AS ci
WHERE EXISTS (
    SELECT 1 FROM t2 AS b JOIN t3 AS c ON b.id = c.id
    WHERE b.key = missing.metadata.sub_column
)
