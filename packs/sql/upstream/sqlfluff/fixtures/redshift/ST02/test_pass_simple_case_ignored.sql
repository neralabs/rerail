SELECT
    CASE status_code
        WHEN 100 THEN true
        ELSE false
    END AS is_success
FROM a
