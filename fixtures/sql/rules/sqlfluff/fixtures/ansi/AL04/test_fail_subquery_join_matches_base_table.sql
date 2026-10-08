SELECT tbl.val
FROM tbl
LEFT JOIN (SELECT tbl.val FROM tbl2 AS tbl);
