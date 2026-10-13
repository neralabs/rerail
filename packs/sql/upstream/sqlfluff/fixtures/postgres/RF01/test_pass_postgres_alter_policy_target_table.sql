ALTER POLICY policy_1 ON table_1
WITH CHECK (table_1.col_1 = current_user AND table_1.col_2 > 0)
