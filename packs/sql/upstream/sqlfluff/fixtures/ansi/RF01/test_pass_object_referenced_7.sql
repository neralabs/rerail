UPDATE my_table
SET row_sum = (
    SELECT COUNT(*) AS row_sum
    FROM
        another_table
    WHERE
        another_table.id = my_table.id
)
