SELECT missing.value FROM first_table AS a
UNION ALL
SELECT b.id FROM second_table AS b;
