SELECT
    x.json,
    key1,
    value2.field2
FROM table1 AS x,
    UNPIVOT x.json AS value1 AT key1,
    value1.field1 AS value2 AT key2
