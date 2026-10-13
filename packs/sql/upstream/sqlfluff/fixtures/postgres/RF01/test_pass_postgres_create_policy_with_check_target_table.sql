CREATE POLICY policy_1 ON table_1
FOR UPDATE TO PUBLIC
USING (table_1.id = current_user)
WITH CHECK (table_1.id = current_user AND table_1.col_1 IS NOT NULL)
