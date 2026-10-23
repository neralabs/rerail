WITH RECURSIVE
outcome0 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,10000 AS work,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome1 AS (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,10000 AS work FROM outcome0 GROUP BY file
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
SELECT d.k,d.file,d.node,d.work FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN 1 THEN 1 END AS truth1 FROM outcome1 d
),
outcome3 AS (
SELECT d.k,d.file,d.node,d.work AS work,NULL AS hit5,NULL AS memoCost6,COALESCE(COALESCE(d.truth1>0 AND NOT (0),0),0) AS reached3,d.work AS start4,d.truth1 AS truth1 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN NOT d.reached3 THEN d.work WHEN d.hit5 IS NOT NULL THEN MAX(0,d.work-d.memoCost6) ELSE MAX(0,d.work-1) END AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,CASE WHEN NOT d.reached3 THEN NULL WHEN d.hit5 IS NOT NULL THEN CASE WHEN d.work<d.memoCost6 THEN -2 ELSE d.hit5 END WHEN d.work=0 THEN -2 END AS truth7 FROM outcome3 d
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.reached3 AND d.hit5 IS NULL AND d.start4>0 THEN 1 END AS truth8 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN COALESCE(d.truth8>0 AND NOT (0),0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(0,n.file,n.id))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome5 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome7 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.tested AS truth10,d.truth7 AS truth7,d.truth8 AS truth8 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.truth8 IS NULL THEN NULL WHEN d.truth8<0 THEN d.truth8 WHEN d.truth10<0 THEN d.truth10 WHEN d.truth8=0 OR d.truth10=0 THEN 0 WHEN d.truth8=2 OR d.truth10=2 THEN 2 ELSE 1 END AS truth8 FROM outcome7 d
),
outcome9 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN COALESCE(d.truth8>0 AND NOT (0),0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(1,n.file,n.id))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome8 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome10 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.tested AS truth11,d.truth7 AS truth7,d.truth8 AS truth8 FROM outcome9 d
),
outcome11 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.truth8 IS NULL THEN NULL WHEN d.truth8<0 THEN d.truth8 WHEN d.truth11<0 THEN d.truth11 WHEN d.truth8=0 OR d.truth11=0 THEN 0 WHEN d.truth8=2 OR d.truth11=2 THEN 2 ELSE 1 END AS truth8 FROM outcome10 d
),
outcome12 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN COALESCE(d.truth8>0 AND NOT (0),0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(2,n.file,n.id))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome11 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome13 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.tested AS truth12,d.truth7 AS truth7,d.truth8 AS truth8 FROM outcome12 d
),
outcome14 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.truth8 IS NULL THEN NULL WHEN d.truth8<0 THEN d.truth8 WHEN d.truth12<0 THEN d.truth12 WHEN d.truth8=0 OR d.truth12=0 THEN 0 WHEN d.truth8=2 OR d.truth12=2 THEN 2 ELSE 1 END AS truth8 FROM outcome13 d
),
outcome15 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN COALESCE(d.truth8>0 AND NOT (0),0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(3,n.file,n.id))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome14 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome16 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.tested AS truth13,d.truth7 AS truth7,d.truth8 AS truth8 FROM outcome15 d
),
outcome17 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.truth8 IS NULL THEN NULL WHEN d.truth8<0 THEN d.truth8 WHEN d.truth13<0 THEN d.truth13 WHEN d.truth8=0 OR d.truth13=0 THEN 0 WHEN d.truth8=2 OR d.truth13=2 THEN 2 ELSE 1 END AS truth8 FROM outcome16 d
),
outcome18 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN COALESCE(d.truth8>0 AND NOT (0),0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(4,n.file,n.id))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome17 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome19 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.tested AS truth14,d.truth7 AS truth7,d.truth8 AS truth8 FROM outcome18 d
),
outcome20 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.truth8 IS NULL THEN NULL WHEN d.truth8<0 THEN d.truth8 WHEN d.truth14<0 THEN d.truth14 WHEN d.truth8=0 OR d.truth14=0 THEN 0 WHEN d.truth8=2 OR d.truth14=2 THEN 2 ELSE 1 END AS truth8 FROM outcome19 d
),
outcome21 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN COALESCE(d.truth8>0 AND NOT (0),0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(5,n.file,n.id))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome20 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome22 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.tested AS truth15,d.truth7 AS truth7,d.truth8 AS truth8 FROM outcome21 d
),
outcome23 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.truth8 IS NULL THEN NULL WHEN d.truth8<0 THEN d.truth8 WHEN d.truth15<0 THEN d.truth15 WHEN d.truth8=0 OR d.truth15=0 THEN 0 WHEN d.truth8=2 OR d.truth15=2 THEN 2 ELSE 1 END AS truth8 FROM outcome22 d
),
outcome24 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN COALESCE(d.truth8>0 AND NOT (0),0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(6,n.file,n.id))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome23 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome25 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.tested AS truth16,d.truth7 AS truth7,d.truth8 AS truth8 FROM outcome24 d
),
outcome26 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.truth8 IS NULL THEN NULL WHEN d.truth8<0 THEN d.truth8 WHEN d.truth16<0 THEN d.truth16 WHEN d.truth8=0 OR d.truth16=0 THEN 0 WHEN d.truth8=2 OR d.truth16=2 THEN 2 ELSE 1 END AS truth8 FROM outcome25 d
),
outcome27 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN COALESCE(d.truth8>0 AND NOT (0),0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(7,n.file,n.id))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome26 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome28 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.tested AS truth17,d.truth7 AS truth7,d.truth8 AS truth8 FROM outcome27 d
),
outcome29 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.truth8 IS NULL THEN NULL WHEN d.truth8<0 THEN d.truth8 WHEN d.truth17<0 THEN d.truth17 WHEN d.truth8=0 OR d.truth17=0 THEN 0 WHEN d.truth8=2 OR d.truth17=2 THEN 2 ELSE 1 END AS truth8 FROM outcome28 d
),
outcome30 AS MATERIALIZED (
SELECT d.*,NULL AS memo_hit FROM outcome29 d
),
outcome31 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth8>0 AND NOT (0),0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(NULL) THEN -2 ELSE NULL END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (COALESCE(d.truth8>0 AND NOT (0),0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(NULL)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome30 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(8,d.file,d.node,CASE WHEN ((COALESCE(d.truth8>0 AND NOT (0),0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((COALESCE(d.truth8>0 AND NOT (0),0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome32 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.tested>=0 THEN d.tested END AS m0truth,d.work-d.remaining AS m0cost FROM outcome31 d
),
outcome33 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.tested AS truth18,d.truth7 AS truth7,d.truth8 AS truth8,d.m0truth,d.m0cost FROM outcome32 d
),
outcome34 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.truth8 IS NULL THEN NULL WHEN d.truth8<0 THEN d.truth8 WHEN d.truth18<0 THEN d.truth18 WHEN d.truth8=0 OR d.truth18=0 THEN 0 WHEN d.truth8=2 OR d.truth18=2 THEN 2 ELSE 1 END AS truth8,d.m0truth,d.m0cost FROM outcome33 d
),
outcome35 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,d.truth8 AS truth8,d.m0truth,d.m0cost FROM outcome34 d
),
outcome36 AS (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,CASE WHEN d.reached3 AND d.hit5 IS NULL AND d.start4>0 THEN d.truth8 ELSE d.truth7 END AS truth7,d.m0truth,d.m0cost FROM outcome35 d
),
outcome37 AS (
SELECT d.*,CASE WHEN d.truth7>=0 THEN d.truth7 END AS m1truth,d.start4-d.work AS m1cost FROM outcome36 d
),
outcome38 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome37 d
),
outcome39 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth7<0 THEN d.truth7 WHEN d.truth1=0 OR d.truth7=0 THEN 0 WHEN d.truth1=2 OR d.truth7=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome38 d
),
outcome40 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome39 d
),
outcome41 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth19,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome40 d
),
outcome42 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth19<0 THEN d.truth19 WHEN d.truth1=0 OR d.truth19=0 THEN 0 WHEN d.truth1=2 OR d.truth19=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome41 d
),
outcome43 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome42 d
),
outcome44 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth20,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome43 d
),
outcome45 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth20<0 THEN d.truth20 WHEN d.truth1=0 OR d.truth20=0 THEN 0 WHEN d.truth1=2 OR d.truth20=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome44 d
),
outcome46 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome45 d
),
outcome47 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth21,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome46 d
),
outcome48 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth21<0 THEN d.truth21 WHEN d.truth1=0 OR d.truth21=0 THEN 0 WHEN d.truth1=2 OR d.truth21=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome47 d
),
outcome49 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome48 d
),
outcome50 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth22,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome49 d
),
outcome51 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth22<0 THEN d.truth22 WHEN d.truth1=0 OR d.truth22=0 THEN 0 WHEN d.truth1=2 OR d.truth22=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome50 d
),
outcome52 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome51 d
),
outcome53 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth23,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome52 d
),
outcome54 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth23<0 THEN d.truth23 WHEN d.truth1=0 OR d.truth23=0 THEN 0 WHEN d.truth1=2 OR d.truth23=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome53 d
),
outcome55 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome54 d
),
outcome56 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth24,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome55 d
),
outcome57 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth24<0 THEN d.truth24 WHEN d.truth1=0 OR d.truth24=0 THEN 0 WHEN d.truth1=2 OR d.truth24=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome56 d
),
outcome58 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome57 d
),
outcome59 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth25,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome58 d
),
outcome60 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth25<0 THEN d.truth25 WHEN d.truth1=0 OR d.truth25=0 THEN 0 WHEN d.truth1=2 OR d.truth25=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome59 d
),
outcome61 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome60 d
),
outcome62 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth26,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome61 d
),
outcome63 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth26<0 THEN d.truth26 WHEN d.truth1=0 OR d.truth26=0 THEN 0 WHEN d.truth1=2 OR d.truth26=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome62 d
),
outcome64 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome63 d
),
outcome65 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth27,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome64 d
),
outcome66 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth27<0 THEN d.truth27 WHEN d.truth1=0 OR d.truth27=0 THEN 0 WHEN d.truth1=2 OR d.truth27=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome65 d
),
outcome67 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome66 d
),
outcome68 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth28,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome67 d
),
outcome69 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth28<0 THEN d.truth28 WHEN d.truth1=0 OR d.truth28=0 THEN 0 WHEN d.truth1=2 OR d.truth28=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome68 d
),
outcome70 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome69 d
),
outcome71 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth29,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome70 d
),
outcome72 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth29<0 THEN d.truth29 WHEN d.truth1=0 OR d.truth29=0 THEN 0 WHEN d.truth1=2 OR d.truth29=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome71 d
),
outcome73 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome72 d
),
outcome74 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth30,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome73 d
),
outcome75 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth30<0 THEN d.truth30 WHEN d.truth1=0 OR d.truth30=0 THEN 0 WHEN d.truth1=2 OR d.truth30=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome74 d
),
outcome76 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome75 d
),
outcome77 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth31,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome76 d
),
outcome78 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth31<0 THEN d.truth31 WHEN d.truth1=0 OR d.truth31=0 THEN 0 WHEN d.truth1=2 OR d.truth31=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome77 d
),
outcome79 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome78 d
),
outcome80 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth32,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome79 d
),
outcome81 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth32<0 THEN d.truth32 WHEN d.truth1=0 OR d.truth32=0 THEN 0 WHEN d.truth1=2 OR d.truth32=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome80 d
),
outcome82 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome81 d
),
outcome83 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth33,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome82 d
),
outcome84 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth33<0 THEN d.truth33 WHEN d.truth1=0 OR d.truth33=0 THEN 0 WHEN d.truth1=2 OR d.truth33=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome83 d
),
outcome85 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome84 d
),
outcome86 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth34,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome85 d
),
outcome87 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth34<0 THEN d.truth34 WHEN d.truth1=0 OR d.truth34=0 THEN 0 WHEN d.truth1=2 OR d.truth34=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome86 d
),
outcome88 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome87 d
),
outcome89 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth35,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome88 d
),
outcome90 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth35<0 THEN d.truth35 WHEN d.truth1=0 OR d.truth35=0 THEN 0 WHEN d.truth1=2 OR d.truth35=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome89 d
),
outcome91 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome90 d
),
outcome92 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth36,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome91 d
),
outcome93 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth36<0 THEN d.truth36 WHEN d.truth1=0 OR d.truth36=0 THEN 0 WHEN d.truth1=2 OR d.truth36=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome92 d
),
outcome94 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome93 d
),
outcome95 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth37,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome94 d
),
outcome96 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth37<0 THEN d.truth37 WHEN d.truth1=0 OR d.truth37=0 THEN 0 WHEN d.truth1=2 OR d.truth37=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome95 d
),
outcome97 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome96 d
),
outcome98 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth38,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome97 d
),
outcome99 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth38<0 THEN d.truth38 WHEN d.truth1=0 OR d.truth38=0 THEN 0 WHEN d.truth1=2 OR d.truth38=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome98 d
),
outcome100 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome99 d
),
outcome101 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth39,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome100 d
),
outcome102 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth39<0 THEN d.truth39 WHEN d.truth1=0 OR d.truth39=0 THEN 0 WHEN d.truth1=2 OR d.truth39=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome101 d
),
outcome103 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome102 d
),
outcome104 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth40,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome103 d
),
outcome105 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth40<0 THEN d.truth40 WHEN d.truth1=0 OR d.truth40=0 THEN 0 WHEN d.truth1=2 OR d.truth40=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome104 d
),
outcome106 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome105 d
),
outcome107 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth41,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome106 d
),
outcome108 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth41<0 THEN d.truth41 WHEN d.truth1=0 OR d.truth41=0 THEN 0 WHEN d.truth1=2 OR d.truth41=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome107 d
),
outcome109 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome108 d
),
outcome110 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth42,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome109 d
),
outcome111 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth42<0 THEN d.truth42 WHEN d.truth1=0 OR d.truth42=0 THEN 0 WHEN d.truth1=2 OR d.truth42=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome110 d
),
outcome112 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome111 d
),
outcome113 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth43,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome112 d
),
outcome114 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth43<0 THEN d.truth43 WHEN d.truth1=0 OR d.truth43=0 THEN 0 WHEN d.truth1=2 OR d.truth43=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome113 d
),
outcome115 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome114 d
),
outcome116 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth44,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome115 d
),
outcome117 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth44<0 THEN d.truth44 WHEN d.truth1=0 OR d.truth44=0 THEN 0 WHEN d.truth1=2 OR d.truth44=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome116 d
),
outcome118 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome117 d
),
outcome119 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth45,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome118 d
),
outcome120 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth45<0 THEN d.truth45 WHEN d.truth1=0 OR d.truth45=0 THEN 0 WHEN d.truth1=2 OR d.truth45=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome119 d
),
outcome121 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome120 d
),
outcome122 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth46,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome121 d
),
outcome123 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth46<0 THEN d.truth46 WHEN d.truth1=0 OR d.truth46=0 THEN 0 WHEN d.truth1=2 OR d.truth46=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome122 d
),
outcome124 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome123 d
),
outcome125 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth47,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome124 d
),
outcome126 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth47<0 THEN d.truth47 WHEN d.truth1=0 OR d.truth47=0 THEN 0 WHEN d.truth1=2 OR d.truth47=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome125 d
),
outcome127 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome126 d
),
outcome128 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth48,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome127 d
),
outcome129 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth48<0 THEN d.truth48 WHEN d.truth1=0 OR d.truth48=0 THEN 0 WHEN d.truth1=2 OR d.truth48=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome128 d
),
outcome130 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome129 d
),
outcome131 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth49 FROM outcome130 d
),
outcome132 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth49<0 THEN d.truth49 WHEN d.truth1=0 OR d.truth49=0 THEN 0 WHEN d.truth1=2 OR d.truth49=2 THEN 2 ELSE 1 END AS truth1 FROM outcome131 d
),
outcome133 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1 FROM outcome132 d
),
outcome134 AS MATERIALIZED (
SELECT d.k,d.work,d.truth1 AS truth FROM outcome133 d
) SELECT o.truth,o.work FROM outcome134 o)
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
