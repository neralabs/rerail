WITH RECURSIVE
outcome0 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,1000 AS work,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome1 AS (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,1000 AS work FROM outcome0 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome1 w CROSS JOIN outcome0 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,w.work AS work WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work,1 AS truth FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome3 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work,1 AS truth FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,NULL AS memo_hit FROM outcome1 d
),
outcome3 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(NULL) THEN -2 ELSE NULL END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(NULL)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome2 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(0,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome4 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.tested>=0 THEN d.tested END AS m0truth,d.work-d.remaining AS m0cost FROM outcome3 d
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome7 d
),
outcome9 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome8 d
),
outcome10 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome9 d
),
outcome11 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome10 d
),
outcome12 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome11 d
),
outcome13 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome12 d
),
outcome14 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome13 d
),
outcome15 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome14 d
),
outcome16 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome15 d
),
outcome17 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome16 d
),
outcome18 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome17 d
),
outcome19 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome18 d
),
outcome20 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome19 d
),
outcome21 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome20 d
),
outcome22 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome21 d
),
outcome23 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome22 d
),
outcome24 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome23 d
),
outcome25 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome24 d
),
outcome26 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome25 d
),
outcome27 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome26 d
),
outcome28 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome27 d
),
outcome29 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome28 d
),
outcome30 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome29 d
),
outcome31 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome30 d
),
outcome32 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome31 d
),
outcome33 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome32 d
),
outcome34 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome33 d
),
outcome35 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome34 d
),
outcome36 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome35 d
),
outcome37 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome36 d
),
outcome38 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome37 d
),
outcome39 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome38 d
),
outcome40 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome39 d
),
outcome41 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome40 d
),
outcome42 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome41 d
),
outcome43 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome42 d
),
outcome44 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome43 d
),
outcome45 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome44 d
),
outcome46 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome45 d
),
outcome47 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome46 d
),
outcome48 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome47 d
),
outcome49 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome48 d
),
outcome50 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome49 d
),
outcome51 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome50 d
),
outcome52 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome51 d
),
outcome53 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome52 d
),
outcome54 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome53 d
),
outcome55 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome54 d
),
outcome56 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome55 d
),
outcome57 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome56 d
),
outcome58 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome57 d
),
outcome59 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome58 d
),
outcome60 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome59 d
),
outcome61 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome60 d
),
outcome62 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome61 d
),
outcome63 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome62 d
),
outcome64 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome63 d
),
outcome65 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome64 d
),
outcome66 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN d.truth>0 AND NOT (0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome65 d
),
outcome67 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome66 d
),
outcome68 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome67 s
) SELECT o.truth,o.work FROM outcome68 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.truth>=0
),
outcome2 AS MATERIALIZED (
SELECT 0 AS clause,o.file,o.node,o.truth,o.work FROM outcome1 o WHERE o.truth=1 OR o.truth<0
),
outcome3 AS MATERIALIZED (
SELECT clause,file,node,truth,work FROM outcome2 WHERE truth<0
)
-- final matches
SELECT m.clause,m.file,m.node,CASE m.clause WHEN 0 THEN json_array() END AS payload,0 AS failure,m.work FROM outcome2 m WHERE m.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome3 f WHERE f.clause=m.clause AND f.file=m.file)
UNION ALL
SELECT clause,file,node,'[]',truth,work FROM outcome3
ORDER BY 1,2,3
