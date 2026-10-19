SELECT
    quarter,
    sales
FROM Produce
UNPIVOT(sales FOR quarter IN (Q1, Q2, Q3, Q4))
