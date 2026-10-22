SELECT order_date, revenue
FROM SEMANTIC_VIEW(
    analytics.semantic.sales_analysis
    DIMENSIONS orders.order_date
    METRICS orders.revenue
    WHERE orders.quantity * orders.unit_price > 100
)
