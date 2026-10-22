SELECT a.metadata.nested.value
FROM first_table AS a
JOIN second_table AS b ON a.id = b.id;
