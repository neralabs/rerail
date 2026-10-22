SELECT *
FROM #store as s
INNER JOIN #store_sales AS ss
QUALIFY row_number() OVER (PARTITION BY ss_sold_date ORDER BY ss_sales_price DESC) <= 2
