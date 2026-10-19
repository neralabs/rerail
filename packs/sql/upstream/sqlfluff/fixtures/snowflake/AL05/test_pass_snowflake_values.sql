SELECT
  thing_1
  , thing_2
FROM VALUES
  ( 'foo', 'bar')
  , ( 'foo', 'bar')
  my_table_alias(thing_1, thing_2)
