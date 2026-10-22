SELECT
    ed.shop_id,
    key,
    value
FROM example_data AS ed,
    ed.inventory AS value AT key
