SELECT
    CASE
        WHEN item.submitted_timestamp IS NOT NULL
        THEN item.sitting_id
    END
