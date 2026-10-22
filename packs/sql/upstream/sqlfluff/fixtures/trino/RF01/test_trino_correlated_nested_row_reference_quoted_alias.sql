SELECT "Ci".id
FROM source_table AS "Ci"
WHERE EXISTS (
    SELECT 1 FROM t2 AS b JOIN t3 AS c ON b.id = c.id
    WHERE b.key = "Ci".metadata.nested.sub_column
)
