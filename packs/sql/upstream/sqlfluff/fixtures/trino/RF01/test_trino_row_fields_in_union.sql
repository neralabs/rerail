SELECT a.metadata.value FROM first_table AS a
UNION ALL
SELECT b.metadata.value FROM second_table AS b;
