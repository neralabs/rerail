SELECT * FROM db.sc.tbl2
WHERE a NOT IN (SELECT a FROM db.sc.tbl1)
