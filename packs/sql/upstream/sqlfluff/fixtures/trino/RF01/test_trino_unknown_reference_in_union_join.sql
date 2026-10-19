(SELECT missing.value FROM first_table AS a
JOIN second_table AS b ON a.id = b.id)
UNION ALL
(SELECT c.id FROM third_table AS c);
