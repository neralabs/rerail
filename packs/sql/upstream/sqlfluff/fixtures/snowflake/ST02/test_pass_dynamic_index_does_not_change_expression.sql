SELECT CASE
  WHEN genres[uniform(0, 9, random())] IS NOT NULL
  THEN genres[uniform(0, 9, random())]
END
