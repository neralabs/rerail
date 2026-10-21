SELECT * FROM db.sc.tbl2
WHERE a NOT IN (SELECT tbl2.a FROM db.sc.tbl1)
