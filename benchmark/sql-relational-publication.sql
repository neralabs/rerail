WITH RECURSIVE
outcome0 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome1 AS (
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
outcome3 AS (
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
outcome5 AS (
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
outcome7 AS (
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
outcome9 AS (
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
outcome11 AS (
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
outcome13 AS (
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
outcome15 AS (
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
outcome17 AS (
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
outcome19 AS (
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
outcome21 AS (
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
outcome23 AS (
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
outcome25 AS (
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
outcome27 AS (
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
outcome29 AS (
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
outcome31 AS (
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
outcome33 AS (
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
outcome35 AS (
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
outcome37 AS (
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
outcome39 AS (
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
outcome41 AS (
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
outcome43 AS (
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
outcome45 AS (
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
outcome47 AS (
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
outcome49 AS (
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
outcome51 AS (
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
outcome53 AS (
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
outcome55 AS (
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
outcome57 AS (
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
outcome59 AS (
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
outcome61 AS (
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
outcome63 AS (
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
SELECT 0 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome1 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 1 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome3 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 2 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome5 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 3 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome7 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 4 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome9 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 5 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome11 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 6 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome13 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 7 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome15 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 8 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome17 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 9 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome19 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 10 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome21 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 11 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome23 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 12 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome25 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 13 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome27 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 14 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome29 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 15 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome31 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 16 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome33 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 17 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome35 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 18 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome37 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 19 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome39 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 20 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome41 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 21 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome43 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 22 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome45 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 23 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome47 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 24 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome49 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 25 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome51 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 26 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome53 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 27 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome55 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 28 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome57 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 29 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome59 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 30 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome61 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 31 AS clause,o.file,o.node,o.truth,o.c0 AS v0 FROM outcome63 o WHERE o.truth=1 OR o.truth<0
),
outcome65 AS MATERIALIZED (
SELECT clause,file,node,truth FROM outcome64 WHERE truth<0
)
-- final matches
SELECT m.clause,m.file,m.node,CASE m.clause WHEN 0 THEN json_array(m.v0) WHEN 1 THEN json_array(m.v0) WHEN 2 THEN json_array(m.v0) WHEN 3 THEN json_array(m.v0) WHEN 4 THEN json_array(m.v0) WHEN 5 THEN json_array(m.v0) WHEN 6 THEN json_array(m.v0) WHEN 7 THEN json_array(m.v0) WHEN 8 THEN json_array(m.v0) WHEN 9 THEN json_array(m.v0) WHEN 10 THEN json_array(m.v0) WHEN 11 THEN json_array(m.v0) WHEN 12 THEN json_array(m.v0) WHEN 13 THEN json_array(m.v0) WHEN 14 THEN json_array(m.v0) WHEN 15 THEN json_array(m.v0) WHEN 16 THEN json_array(m.v0) WHEN 17 THEN json_array(m.v0) WHEN 18 THEN json_array(m.v0) WHEN 19 THEN json_array(m.v0) WHEN 20 THEN json_array(m.v0) WHEN 21 THEN json_array(m.v0) WHEN 22 THEN json_array(m.v0) WHEN 23 THEN json_array(m.v0) WHEN 24 THEN json_array(m.v0) WHEN 25 THEN json_array(m.v0) WHEN 26 THEN json_array(m.v0) WHEN 27 THEN json_array(m.v0) WHEN 28 THEN json_array(m.v0) WHEN 29 THEN json_array(m.v0) WHEN 30 THEN json_array(m.v0) WHEN 31 THEN json_array(m.v0) END AS payload,0 AS failure FROM outcome64 m WHERE m.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome65 f WHERE f.clause=m.clause AND f.file=m.file)
UNION ALL
SELECT clause,file,node,'[]',truth FROM outcome65
ORDER BY 1,2,3
