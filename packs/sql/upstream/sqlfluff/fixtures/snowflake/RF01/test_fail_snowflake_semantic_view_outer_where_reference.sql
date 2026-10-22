SELECT sv.order_date
FROM SEMANTIC_VIEW(
    analytics.semantic.sales_analysis
    DIMENSIONS orders.order_date
) AS sv
WHERE missing.order_date > sv.order_date
