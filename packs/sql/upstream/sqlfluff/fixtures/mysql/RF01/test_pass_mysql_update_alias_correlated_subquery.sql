UPDATE llx_product AS p
SET p.stock = (
    SELECT SUM(ps.reel)
    FROM llx_product_stock AS ps
    WHERE ps.fk_product = p.rowid
);
