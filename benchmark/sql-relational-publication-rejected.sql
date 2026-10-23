WITH RECURSIVE
outcome0 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome1 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome0 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome1 w CROSS JOIN outcome0 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(0,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome2 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome3 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome2 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome3 w CROSS JOIN outcome2 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(1,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome4 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome5 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome4 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome5 w CROSS JOIN outcome4 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(2,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome6 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome7 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome6 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome7 w CROSS JOIN outcome6 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(3,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome8 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome9 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome8 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome9 w CROSS JOIN outcome8 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(4,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome10 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome11 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome10 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome11 w CROSS JOIN outcome10 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(5,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome12 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome13 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome12 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome13 w CROSS JOIN outcome12 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(6,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome14 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome15 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome14 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome15 w CROSS JOIN outcome14 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(7,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome16 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome17 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome16 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome17 w CROSS JOIN outcome16 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(8,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome18 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome19 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome18 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome19 w CROSS JOIN outcome18 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(9,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome20 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome21 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome20 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome21 w CROSS JOIN outcome20 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(10,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome22 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome23 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome22 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome23 w CROSS JOIN outcome22 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(11,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome24 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome25 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome24 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome25 w CROSS JOIN outcome24 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(12,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome26 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome27 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome26 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome27 w CROSS JOIN outcome26 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(13,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome28 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome29 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome28 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome29 w CROSS JOIN outcome28 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(14,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome30 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome31 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome30 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome31 w CROSS JOIN outcome30 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(15,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome32 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome33 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome32 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome33 w CROSS JOIN outcome32 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(16,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome34 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome35 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome34 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome35 w CROSS JOIN outcome34 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(17,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome36 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome37 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome36 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome37 w CROSS JOIN outcome36 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(18,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome38 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome39 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome38 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome39 w CROSS JOIN outcome38 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(19,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome40 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome41 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome40 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome41 w CROSS JOIN outcome40 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(20,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome42 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome43 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome42 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome43 w CROSS JOIN outcome42 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(21,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome44 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome45 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome44 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome45 w CROSS JOIN outcome44 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(22,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome46 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome47 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome46 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome47 w CROSS JOIN outcome46 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(23,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome48 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome49 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome48 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome49 w CROSS JOIN outcome48 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(24,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome50 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome51 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome50 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome51 w CROSS JOIN outcome50 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(25,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome52 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome53 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome52 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome53 w CROSS JOIN outcome52 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(26,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome54 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome55 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome54 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome55 w CROSS JOIN outcome54 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(27,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome56 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome57 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome56 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome57 w CROSS JOIN outcome56 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(28,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome58 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome59 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome58 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome59 w CROSS JOIN outcome58 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(29,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome60 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome61 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome60 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome61 w CROSS JOIN outcome60 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(30,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome62 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome63 AS MATERIALIZED (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,NULL AS c0 FROM outcome62 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome63 w CROSS JOIN outcome62 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN relation_value(31,d.file,d.node) ELSE NULL END AS tested FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome7 s
) SELECT o.truth,o.c0 AS c0 FROM outcome8 o) WHERE w.truth>=0
),
outcome64 AS MATERIALIZED (
SELECT 0 AS clause,d.file,d.node,o.truth FROM outcome0 d JOIN outcome1 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 1 AS clause,d.file,d.node,o.truth FROM outcome2 d JOIN outcome3 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 2 AS clause,d.file,d.node,o.truth FROM outcome4 d JOIN outcome5 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 3 AS clause,d.file,d.node,o.truth FROM outcome6 d JOIN outcome7 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 4 AS clause,d.file,d.node,o.truth FROM outcome8 d JOIN outcome9 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 5 AS clause,d.file,d.node,o.truth FROM outcome10 d JOIN outcome11 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 6 AS clause,d.file,d.node,o.truth FROM outcome12 d JOIN outcome13 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 7 AS clause,d.file,d.node,o.truth FROM outcome14 d JOIN outcome15 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 8 AS clause,d.file,d.node,o.truth FROM outcome16 d JOIN outcome17 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 9 AS clause,d.file,d.node,o.truth FROM outcome18 d JOIN outcome19 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 10 AS clause,d.file,d.node,o.truth FROM outcome20 d JOIN outcome21 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 11 AS clause,d.file,d.node,o.truth FROM outcome22 d JOIN outcome23 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 12 AS clause,d.file,d.node,o.truth FROM outcome24 d JOIN outcome25 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 13 AS clause,d.file,d.node,o.truth FROM outcome26 d JOIN outcome27 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 14 AS clause,d.file,d.node,o.truth FROM outcome28 d JOIN outcome29 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 15 AS clause,d.file,d.node,o.truth FROM outcome30 d JOIN outcome31 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 16 AS clause,d.file,d.node,o.truth FROM outcome32 d JOIN outcome33 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 17 AS clause,d.file,d.node,o.truth FROM outcome34 d JOIN outcome35 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 18 AS clause,d.file,d.node,o.truth FROM outcome36 d JOIN outcome37 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 19 AS clause,d.file,d.node,o.truth FROM outcome38 d JOIN outcome39 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 20 AS clause,d.file,d.node,o.truth FROM outcome40 d JOIN outcome41 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 21 AS clause,d.file,d.node,o.truth FROM outcome42 d JOIN outcome43 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 22 AS clause,d.file,d.node,o.truth FROM outcome44 d JOIN outcome45 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 23 AS clause,d.file,d.node,o.truth FROM outcome46 d JOIN outcome47 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 24 AS clause,d.file,d.node,o.truth FROM outcome48 d JOIN outcome49 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 25 AS clause,d.file,d.node,o.truth FROM outcome50 d JOIN outcome51 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 26 AS clause,d.file,d.node,o.truth FROM outcome52 d JOIN outcome53 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 27 AS clause,d.file,d.node,o.truth FROM outcome54 d JOIN outcome55 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 28 AS clause,d.file,d.node,o.truth FROM outcome56 d JOIN outcome57 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 29 AS clause,d.file,d.node,o.truth FROM outcome58 d JOIN outcome59 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 30 AS clause,d.file,d.node,o.truth FROM outcome60 d JOIN outcome61 o ON o.k=d.k WHERE o.truth<0
UNION ALL
SELECT 31 AS clause,d.file,d.node,o.truth FROM outcome62 d JOIN outcome63 o ON o.k=d.k WHERE o.truth<0
)
-- final matches
SELECT 0 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome0 d JOIN outcome1 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=0 AND f.file=d.file)
UNION ALL
SELECT 1 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome2 d JOIN outcome3 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=1 AND f.file=d.file)
UNION ALL
SELECT 2 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome4 d JOIN outcome5 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=2 AND f.file=d.file)
UNION ALL
SELECT 3 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome6 d JOIN outcome7 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=3 AND f.file=d.file)
UNION ALL
SELECT 4 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome8 d JOIN outcome9 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=4 AND f.file=d.file)
UNION ALL
SELECT 5 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome10 d JOIN outcome11 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=5 AND f.file=d.file)
UNION ALL
SELECT 6 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome12 d JOIN outcome13 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=6 AND f.file=d.file)
UNION ALL
SELECT 7 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome14 d JOIN outcome15 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=7 AND f.file=d.file)
UNION ALL
SELECT 8 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome16 d JOIN outcome17 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=8 AND f.file=d.file)
UNION ALL
SELECT 9 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome18 d JOIN outcome19 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=9 AND f.file=d.file)
UNION ALL
SELECT 10 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome20 d JOIN outcome21 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=10 AND f.file=d.file)
UNION ALL
SELECT 11 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome22 d JOIN outcome23 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=11 AND f.file=d.file)
UNION ALL
SELECT 12 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome24 d JOIN outcome25 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=12 AND f.file=d.file)
UNION ALL
SELECT 13 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome26 d JOIN outcome27 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=13 AND f.file=d.file)
UNION ALL
SELECT 14 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome28 d JOIN outcome29 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=14 AND f.file=d.file)
UNION ALL
SELECT 15 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome30 d JOIN outcome31 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=15 AND f.file=d.file)
UNION ALL
SELECT 16 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome32 d JOIN outcome33 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=16 AND f.file=d.file)
UNION ALL
SELECT 17 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome34 d JOIN outcome35 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=17 AND f.file=d.file)
UNION ALL
SELECT 18 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome36 d JOIN outcome37 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=18 AND f.file=d.file)
UNION ALL
SELECT 19 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome38 d JOIN outcome39 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=19 AND f.file=d.file)
UNION ALL
SELECT 20 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome40 d JOIN outcome41 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=20 AND f.file=d.file)
UNION ALL
SELECT 21 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome42 d JOIN outcome43 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=21 AND f.file=d.file)
UNION ALL
SELECT 22 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome44 d JOIN outcome45 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=22 AND f.file=d.file)
UNION ALL
SELECT 23 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome46 d JOIN outcome47 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=23 AND f.file=d.file)
UNION ALL
SELECT 24 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome48 d JOIN outcome49 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=24 AND f.file=d.file)
UNION ALL
SELECT 25 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome50 d JOIN outcome51 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=25 AND f.file=d.file)
UNION ALL
SELECT 26 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome52 d JOIN outcome53 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=26 AND f.file=d.file)
UNION ALL
SELECT 27 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome54 d JOIN outcome55 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=27 AND f.file=d.file)
UNION ALL
SELECT 28 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome56 d JOIN outcome57 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=28 AND f.file=d.file)
UNION ALL
SELECT 29 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome58 d JOIN outcome59 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=29 AND f.file=d.file)
UNION ALL
SELECT 30 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome60 d JOIN outcome61 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=30 AND f.file=d.file)
UNION ALL
SELECT 31 AS clause,d.file,d.node,json_array(o.c0) AS payload,0 AS failure FROM outcome62 d JOIN outcome63 o ON o.k=d.k WHERE o.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome64 f WHERE f.clause=31 AND f.file=d.file)
UNION ALL
SELECT clause,file,node,'[]',truth FROM outcome64
ORDER BY 1,2,3
