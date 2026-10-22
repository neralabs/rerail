WITH RECURSIVE
outcome0 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node FROM node n WHERE 1 AND n.kind IN (235)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth<>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (235))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.c0 AS c0 FROM outcome3 d WHERE d.truth<>0 AND NOT (0)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.c0 AS c0 FROM outcome4 d WHERE 1
),
outcome6 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,d.c0 AS c0,0 AS u0 FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.c0 AS c0 FROM outcome6 d WHERE d.truth<>0 AND NOT (d.u0)
),
outcome8 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,d.k AS parent,n.file,n.id AS node,n.id AS rank,d.c0 AS c0 FROM outcome7 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node CROSS JOIN node n ON n.file=origin.file AND n.parent=origin.id WHERE n.kind IN (256)
),
outcome9 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,d.c0 AS c0,0 AS u0 FROM outcome8 d
),
outcome10 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth<>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (256))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome9 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome11 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome10 d
),
outcome12 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.c0 AS c0 FROM outcome11 d WHERE d.truth<>0 AND NOT (d.u0)
),
outcome13 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,d.k AS parent,n.file,n.id AS node,n.id AS rank,d.c0 AS c0 FROM outcome12 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node CROSS JOIN node n ON n.file=origin.file AND n.parent=origin.id WHERE n.kind IN (1)
),
outcome14 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,d.c0 AS c0,0 AS u0 FROM outcome13 d
),
outcome15 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth<>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome14 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome16 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome15 d
),
outcome17 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth<>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (1))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome16 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome18 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome17 d
),
outcome19 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth<>0 AND NOT (0) THEN CASE WHEN node_regex(0,d.file,d.node) THEN 2 ELSE 1 END ELSE NULL END AS tested FROM outcome18 d
),
outcome20 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome19 d
),
outcome21 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome20 s
),
outcome22 AS (
SELECT d.parent,o.truth,o.c0 AS c0,ROW_NUMBER() OVER (PARTITION BY d.parent ORDER BY d.rank) AS choice FROM outcome13 d JOIN outcome21 o ON o.k=d.k WHERE o.truth<>0
),
outcome23 AS MATERIALIZED (
SELECT * FROM outcome22 WHERE choice=1
),
outcome24 AS MATERIALIZED (
SELECT d.k,COALESCE(o.truth,0) AS truth,CASE WHEN o.truth=1 THEN o.c0 ELSE d.c0 END AS c0 FROM outcome12 d LEFT JOIN outcome23 o ON o.parent=d.k
),
outcome25 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR o.truth=0 THEN 0 WHEN d.truth=2 OR o.truth=2 THEN 2 ELSE 1 END AS truth,CASE WHEN o.truth=1 THEN o.c0 ELSE d.c0 END AS c0,(d.u0 OR o.truth=2 OR (o.k IS NULL AND d.truth<>0)) AS u0 FROM outcome11 d LEFT JOIN outcome24 o ON o.k=d.k
),
outcome26 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome25 s
),
outcome27 AS (
SELECT d.parent,o.truth,o.c0 AS c0,ROW_NUMBER() OVER (PARTITION BY d.parent ORDER BY d.rank) AS choice FROM outcome8 d JOIN outcome26 o ON o.k=d.k WHERE o.truth<>0
),
outcome28 AS MATERIALIZED (
SELECT * FROM outcome27 WHERE choice=1
),
outcome29 AS MATERIALIZED (
SELECT d.k,COALESCE(o.truth,0) AS truth,CASE WHEN o.truth=1 THEN o.c0 ELSE d.c0 END AS c0 FROM outcome7 d LEFT JOIN outcome28 o ON o.parent=d.k
),
outcome30 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR o.truth=0 THEN 0 WHEN d.truth=2 OR o.truth=2 THEN 2 ELSE 1 END AS truth,CASE WHEN o.truth=1 THEN o.c0 ELSE d.c0 END AS c0,(d.u0 OR o.truth=2 OR (o.k IS NULL AND d.truth<>0)) AS u0 FROM outcome6 d LEFT JOIN outcome29 o ON o.k=d.k
),
outcome31 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.c0 AS c0 FROM outcome30 d WHERE d.truth<>0 AND NOT (d.u0)
),
outcome32 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,d.k AS parent,n.file,n.id AS node,n.id AS rank,d.c0 AS c0 FROM outcome31 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node CROSS JOIN node n ON n.file=origin.file AND n.id>origin.id AND n.id<=origin."last" WHERE n.kind IN (1)
),
outcome33 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,d.c0 AS c0,0 AS u0 FROM outcome32 d
),
outcome34 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth<>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome33 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome35 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome34 d
),
outcome36 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth<>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (1))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome35 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome37 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome36 d
),
outcome38 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth<>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(1,n.file,n.id))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome37 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome39 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome38 d
),
outcome40 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome39 s
),
outcome41 AS (
SELECT d.parent,o.truth,o.c0 AS c0,ROW_NUMBER() OVER (PARTITION BY d.parent ORDER BY d.rank) AS choice FROM outcome32 d JOIN outcome40 o ON o.k=d.k WHERE o.truth<>0
),
outcome42 AS MATERIALIZED (
SELECT * FROM outcome41 WHERE choice=1
),
outcome43 AS MATERIALIZED (
SELECT d.k,COALESCE(o.truth,0) AS truth,CASE WHEN o.truth=1 THEN o.c0 ELSE d.c0 END AS c0 FROM outcome31 d LEFT JOIN outcome42 o ON o.parent=d.k
),
outcome44 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR o.truth=0 THEN 0 WHEN d.truth=2 OR o.truth=2 THEN 2 ELSE 1 END AS truth,CASE WHEN o.truth=1 THEN o.c0 ELSE d.c0 END AS c0,(d.u0 OR o.truth=2 OR (o.k IS NULL AND d.truth<>0)) AS u0 FROM outcome30 d LEFT JOIN outcome43 o ON o.k=d.k
),
outcome45 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome44 s
),
outcome46 AS MATERIALIZED (
SELECT k,CASE truth WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE 2 END AS truth FROM outcome45
),
outcome47 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR o.truth=0 THEN 0 WHEN d.truth=2 OR o.truth=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0 FROM outcome3 d LEFT JOIN outcome46 o ON o.k=d.k
),
outcome48 AS MATERIALIZED (
SELECT s.k,s.truth FROM outcome47 s
)
-- final matches
SELECT 0,d.file,d.node,json_array() FROM outcome0 d JOIN outcome48 o ON o.k=d.k WHERE o.truth=1 ORDER BY d.file,d.node
