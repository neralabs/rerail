ALTER TABLE "Product" ADD CONSTRAINT "Product_price_check" CHECK ("price" >= 0) NOT VALID;
