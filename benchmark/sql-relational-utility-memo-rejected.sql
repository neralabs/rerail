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
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome6 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(1,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome8 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome7 d
),
outcome9 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome8 d
),
outcome10 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome9 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(2,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome11 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome10 d
),
outcome12 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome11 d
),
outcome13 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome12 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(3,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome14 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome13 d
),
outcome15 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome14 d
),
outcome16 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome15 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(4,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome17 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome16 d
),
outcome18 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome17 d
),
outcome19 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome18 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(5,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome20 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome19 d
),
outcome21 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome20 d
),
outcome22 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome21 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(6,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome23 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome22 d
),
outcome24 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome23 d
),
outcome25 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome24 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(7,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome26 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome25 d
),
outcome27 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome26 d
),
outcome28 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome27 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(8,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome29 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome28 d
),
outcome30 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome29 d
),
outcome31 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome30 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(9,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome32 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome31 d
),
outcome33 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome32 d
),
outcome34 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome33 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(10,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome35 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome34 d
),
outcome36 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome35 d
),
outcome37 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome36 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(11,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome38 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome37 d
),
outcome39 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome38 d
),
outcome40 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome39 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(12,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome41 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome40 d
),
outcome42 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome41 d
),
outcome43 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome42 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(13,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome44 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome43 d
),
outcome45 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome44 d
),
outcome46 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome45 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(14,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome47 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome46 d
),
outcome48 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome47 d
),
outcome49 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome48 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(15,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome50 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome49 d
),
outcome51 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome50 d
),
outcome52 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome51 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(16,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome53 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome52 d
),
outcome54 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome53 d
),
outcome55 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome54 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(17,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome56 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome55 d
),
outcome57 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome56 d
),
outcome58 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome57 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(18,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome59 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome58 d
),
outcome60 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome59 d
),
outcome61 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome60 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(19,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome62 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome61 d
),
outcome63 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome62 d
),
outcome64 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome63 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(20,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome65 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome64 d
),
outcome66 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome65 d
),
outcome67 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome66 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(21,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome68 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome67 d
),
outcome69 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome68 d
),
outcome70 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome69 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(22,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome71 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome70 d
),
outcome72 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome71 d
),
outcome73 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome72 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(23,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome74 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome73 d
),
outcome75 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome74 d
),
outcome76 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome75 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(24,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome77 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome76 d
),
outcome78 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome77 d
),
outcome79 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome78 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(25,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome80 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome79 d
),
outcome81 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome80 d
),
outcome82 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome81 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(26,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome83 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome82 d
),
outcome84 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome83 d
),
outcome85 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome84 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(27,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome86 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome85 d
),
outcome87 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome86 d
),
outcome88 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome87 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(28,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome89 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome88 d
),
outcome90 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome89 d
),
outcome91 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome90 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(29,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome92 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome91 d
),
outcome93 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome92 d
),
outcome94 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome93 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(30,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome95 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome94 d
),
outcome96 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome95 d
),
outcome97 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.truth>0 AND NOT (0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome96 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(31,d.file,d.node,CASE WHEN ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.truth>0 AND NOT (0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome98 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.m0truth,d.m0cost FROM outcome97 d
),
outcome99 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome98 s
) SELECT o.truth,o.work FROM outcome99 o)
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
