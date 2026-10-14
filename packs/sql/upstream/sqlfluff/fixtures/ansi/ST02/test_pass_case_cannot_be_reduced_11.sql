SELECT
    dv_runid,
    CASE
        WHEN LEAD(dv_startdateutc) OVER (
            PARTITION BY rowid ORDER BY dv_startdateutc
        ) IS NULL
        THEN 1
        ELSE 0
    END AS loadstate
FROM d;
