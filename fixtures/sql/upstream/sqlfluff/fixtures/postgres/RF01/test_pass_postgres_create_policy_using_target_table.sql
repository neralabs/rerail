CREATE POLICY policy_1 ON table_1
FOR SELECT TO PUBLIC
USING (
  EXISTS (
    SELECT 1
    FROM table_2
    WHERE
      table_2.col_1 = table_1.id
      AND table_2.col_2 = current_user
  )
)
