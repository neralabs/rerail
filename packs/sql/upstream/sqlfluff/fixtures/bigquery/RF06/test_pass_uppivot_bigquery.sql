SELECT
    *
FROM model
UNPIVOT(
    (A, B)
    FOR year
    IN ((C, D) AS 'year_2011', (E, F) AS 'year_2012'));
