SELECT sv.order_date
FROM SEMANTIC_VIEW(
    analytics.semantic.sales_analysis
    DIMENSIONS orders.order_date
) AS sv
JOIN date_targets AS targets
    ON missing.order_date = targets.order_date
