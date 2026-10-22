SELECT source_table.id
FROM catalog.schema.source_table
WHERE EXISTS (
    SELECT 1 FROM t2 AS b JOIN t3 AS c ON b.id = c.id
    WHERE b.key = catalog.schema.source_table.metadata.sub_column
        OR b.key = schema.source_table.metadata.sub_column
        OR b.key = source_table.metadata.sub_column
)
