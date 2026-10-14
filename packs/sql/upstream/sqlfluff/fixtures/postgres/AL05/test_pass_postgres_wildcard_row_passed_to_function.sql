SELECT
  to_jsonb(mot.*)
FROM my_table
JOIN my_other_table AS mot ON (TRUE)
