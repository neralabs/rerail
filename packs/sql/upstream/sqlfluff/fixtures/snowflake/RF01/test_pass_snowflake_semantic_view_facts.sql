SELECT order_id, unit_price
FROM SEMANTIC_VIEW(
    analytics.semantic.sales_analysis
    DIMENSIONS orders.order_id
    FACTS orders.unit_price
)
