SELECT
    col_name,
    val
FROM my_tbl
UNPIVOT (val FOR col_name IN (a, b))
