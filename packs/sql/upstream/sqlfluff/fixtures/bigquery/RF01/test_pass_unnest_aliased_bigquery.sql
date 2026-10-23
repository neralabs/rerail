SELECT foo_bar
FROM foo,
  UNNEST(bar) AS foo_bar
