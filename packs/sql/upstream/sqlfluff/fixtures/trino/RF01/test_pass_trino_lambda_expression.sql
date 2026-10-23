SELECT
    a_column,
    TRANSFORM(array_col, x -> x.example) AS array_col_example
FROM example_table;
