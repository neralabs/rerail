SELECT
    color,
    cnt
FROM count_by_color UNPIVOT (
    cnt FOR color IN (red, green, blue)
)
