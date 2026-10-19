with table_arr as (select [1,2,4,2] as arr)
SELECT arr
FROM table_arr t, t.arr
