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
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN 1 THEN 0 END AS truth1,0 AS unknown2 FROM outcome1 d
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1,CASE WHEN COALESCE(d.truth1=0,0) THEN 1 END AS truth3,d.unknown2 AS unknown2 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN COALESCE(d.truth3>0 AND NOT (0),0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(0,n.file,n.id))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.truth3 AS truth3,d.tested AS truth5,d.unknown2 AS unknown2 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1,CASE WHEN d.truth3 IS NULL THEN NULL WHEN d.truth3<0 THEN d.truth3 WHEN d.truth5<0 THEN d.truth5 WHEN d.truth3=0 OR d.truth5=0 THEN 0 WHEN d.truth3=2 OR d.truth5=2 THEN 2 ELSE 1 END AS truth3,d.unknown2 AS unknown2 FROM outcome5 d
),
outcome7 AS (
SELECT d.k,d.file,d.node,d.work AS work,NULL AS hit8,NULL AS memoCost9,COALESCE(COALESCE(d.truth3>0 AND NOT (0),0),0) AS reached6,d.work AS start7,d.truth1 AS truth1,d.truth3 AS truth3,d.unknown2 AS unknown2 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN NOT d.reached6 THEN d.work WHEN d.hit8 IS NOT NULL THEN MAX(0,d.work-d.memoCost9) ELSE MAX(0,d.work-1) END AS work,d.hit8 AS hit8,d.memoCost9 AS memoCost9,d.reached6 AS reached6,d.start7 AS start7,d.truth1 AS truth1,CASE WHEN NOT d.reached6 THEN NULL WHEN d.hit8 IS NOT NULL THEN CASE WHEN d.work<d.memoCost9 THEN -2 ELSE d.hit8 END WHEN d.work=0 THEN -2 END AS truth10,d.truth3 AS truth3,d.unknown2 AS unknown2 FROM outcome7 d
),
outcome9 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit8 AS hit8,d.memoCost9 AS memoCost9,d.reached6 AS reached6,d.start7 AS start7,d.truth1 AS truth1,d.truth10 AS truth10,CASE WHEN d.reached6 AND d.hit8 IS NULL AND d.start7>0 THEN 1 END AS truth11,d.truth3 AS truth3,d.unknown2 AS unknown2 FROM outcome8 d
),
outcome10 AS MATERIALIZED (
SELECT d.*,NULL AS memo_hit FROM outcome9 d
),
outcome11 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth11>0 AND NOT (0),0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(NULL) THEN -2 ELSE NULL END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (COALESCE(d.truth11>0 AND NOT (0),0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(NULL)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome10 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(1,d.file,d.node,CASE WHEN ((COALESCE(d.truth11>0 AND NOT (0),0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((COALESCE(d.truth11>0 AND NOT (0),0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome12 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.tested>=0 THEN d.tested END AS m0truth,d.work-d.remaining AS m0cost FROM outcome11 d
),
outcome13 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit8 AS hit8,d.memoCost9 AS memoCost9,d.reached6 AS reached6,d.start7 AS start7,d.truth1 AS truth1,d.truth10 AS truth10,d.truth11 AS truth11,d.tested AS truth13,d.truth3 AS truth3,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome12 d
),
outcome14 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit8 AS hit8,d.memoCost9 AS memoCost9,d.reached6 AS reached6,d.start7 AS start7,d.truth1 AS truth1,d.truth10 AS truth10,CASE WHEN d.truth11 IS NULL THEN NULL WHEN d.truth11<0 THEN d.truth11 WHEN d.truth13<0 THEN d.truth13 WHEN d.truth11=0 OR d.truth13=0 THEN 0 WHEN d.truth11=2 OR d.truth13=2 THEN 2 ELSE 1 END AS truth11,d.truth3 AS truth3,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome13 d
),
outcome15 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth11>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m0cost THEN -2 ELSE d.m0truth END AS tested,CASE WHEN COALESCE(d.truth11>0 AND NOT (0),0) THEN MAX(0,d.work-d.m0cost) ELSE d.work END AS remaining FROM outcome14 d
),
outcome16 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit8 AS hit8,d.memoCost9 AS memoCost9,d.reached6 AS reached6,d.start7 AS start7,d.truth1 AS truth1,d.truth10 AS truth10,d.truth11 AS truth11,d.tested AS truth14,d.truth3 AS truth3,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome15 d
),
outcome17 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit8 AS hit8,d.memoCost9 AS memoCost9,d.reached6 AS reached6,d.start7 AS start7,d.truth1 AS truth1,d.truth10 AS truth10,CASE WHEN d.truth11 IS NULL THEN NULL WHEN d.truth11<0 THEN d.truth11 WHEN d.truth14<0 THEN d.truth14 WHEN d.truth11=0 OR d.truth14=0 THEN 0 WHEN d.truth11=2 OR d.truth14=2 THEN 2 ELSE 1 END AS truth11,d.truth3 AS truth3,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome16 d
),
outcome18 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit8 AS hit8,d.memoCost9 AS memoCost9,d.reached6 AS reached6,d.start7 AS start7,d.truth1 AS truth1,d.truth10 AS truth10,d.truth11 AS truth11,d.truth3 AS truth3,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome17 d
),
outcome19 AS (
SELECT d.k,d.file,d.node,d.work AS work,d.hit8 AS hit8,d.memoCost9 AS memoCost9,d.reached6 AS reached6,d.start7 AS start7,d.truth1 AS truth1,CASE WHEN d.reached6 AND d.hit8 IS NULL AND d.start7>0 THEN d.truth11 ELSE d.truth10 END AS truth10,d.truth3 AS truth3,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome18 d
),
outcome20 AS (
SELECT d.*,CASE WHEN d.truth10>=0 THEN d.truth10 END AS m1truth,d.start7-d.work AS m1cost FROM outcome19 d
),
outcome21 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1,d.truth10 AS truth10,d.truth3 AS truth3,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome20 d
),
outcome22 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1,CASE WHEN d.truth3 IS NULL THEN NULL WHEN d.truth3<0 THEN d.truth3 WHEN d.truth10<0 THEN d.truth10 WHEN d.truth3=0 OR d.truth10=0 THEN 0 WHEN d.truth3=2 OR d.truth10=2 THEN 2 ELSE 1 END AS truth3,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome21 d
),
outcome23 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1,d.truth3 AS truth3,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome22 d
),
outcome24 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth3<0 THEN d.truth3 WHEN d.truth3=1 THEN 1 WHEN d.truth3=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth3=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome23 d
),
outcome25 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN COALESCE(d.truth1=0,0) THEN CASE WHEN d.m1truth>=0 THEN d.m1truth END END AS hit17,CASE WHEN COALESCE(d.truth1=0,0) THEN CASE WHEN d.m1truth>=0 THEN d.m1cost END END AS memoCost18,COALESCE(COALESCE(d.truth1=0,0),0) AS reached15,d.work AS start16,d.truth1 AS truth1,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome24 d
),
outcome26 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN NOT d.reached15 THEN d.work WHEN d.hit17 IS NOT NULL THEN MAX(0,d.work-d.memoCost18) ELSE MAX(0,d.work-1) END AS work,d.hit17 AS hit17,d.memoCost18 AS memoCost18,d.reached15 AS reached15,d.start16 AS start16,d.truth1 AS truth1,CASE WHEN NOT d.reached15 THEN NULL WHEN d.hit17 IS NOT NULL THEN CASE WHEN d.work<d.memoCost18 THEN -2 ELSE d.hit17 END WHEN d.work=0 THEN -2 END AS truth19,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome25 d
),
outcome27 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit17 AS hit17,d.memoCost18 AS memoCost18,d.reached15 AS reached15,d.start16 AS start16,d.truth1 AS truth1,d.truth19 AS truth19,CASE WHEN d.reached15 AND d.hit17 IS NULL AND d.start16>0 THEN 1 END AS truth20,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome26 d
),
outcome28 AS MATERIALIZED (
SELECT d.*,CASE WHEN COALESCE(d.truth20>0 AND NOT (0),0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome27 d
),
outcome29 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth20>0 AND NOT (0),0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (COALESCE(d.truth20>0 AND NOT (0),0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome28 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(3,d.file,d.node,CASE WHEN ((COALESCE(d.truth20>0 AND NOT (0),0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((COALESCE(d.truth20>0 AND NOT (0),0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome30 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.tested>=0 THEN d.tested END AS m2truth,d.work-d.remaining AS m2cost FROM outcome29 d
),
outcome31 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit17 AS hit17,d.memoCost18 AS memoCost18,d.reached15 AS reached15,d.start16 AS start16,d.truth1 AS truth1,d.truth19 AS truth19,d.truth20 AS truth20,d.tested AS truth22,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost FROM outcome30 d
),
outcome32 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit17 AS hit17,d.memoCost18 AS memoCost18,d.reached15 AS reached15,d.start16 AS start16,d.truth1 AS truth1,d.truth19 AS truth19,CASE WHEN d.truth20 IS NULL THEN NULL WHEN d.truth20<0 THEN d.truth20 WHEN d.truth22<0 THEN d.truth22 WHEN d.truth20=0 OR d.truth22=0 THEN 0 WHEN d.truth20=2 OR d.truth22=2 THEN 2 ELSE 1 END AS truth20,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost FROM outcome31 d
),
outcome33 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth20>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m2cost THEN -2 ELSE d.m2truth END AS tested,CASE WHEN COALESCE(d.truth20>0 AND NOT (0),0) THEN MAX(0,d.work-d.m2cost) ELSE d.work END AS remaining FROM outcome32 d
),
outcome34 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit17 AS hit17,d.memoCost18 AS memoCost18,d.reached15 AS reached15,d.start16 AS start16,d.truth1 AS truth1,d.truth19 AS truth19,d.truth20 AS truth20,d.tested AS truth23,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost FROM outcome33 d
),
outcome35 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit17 AS hit17,d.memoCost18 AS memoCost18,d.reached15 AS reached15,d.start16 AS start16,d.truth1 AS truth1,d.truth19 AS truth19,CASE WHEN d.truth20 IS NULL THEN NULL WHEN d.truth20<0 THEN d.truth20 WHEN d.truth23<0 THEN d.truth23 WHEN d.truth20=0 OR d.truth23=0 THEN 0 WHEN d.truth20=2 OR d.truth23=2 THEN 2 ELSE 1 END AS truth20,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost FROM outcome34 d
),
outcome36 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit17 AS hit17,d.memoCost18 AS memoCost18,d.reached15 AS reached15,d.start16 AS start16,d.truth1 AS truth1,d.truth19 AS truth19,d.truth20 AS truth20,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost FROM outcome35 d
),
outcome37 AS (
SELECT d.k,d.file,d.node,d.work AS work,d.hit17 AS hit17,d.memoCost18 AS memoCost18,d.reached15 AS reached15,d.start16 AS start16,d.truth1 AS truth1,CASE WHEN d.reached15 AND d.hit17 IS NULL AND d.start16>0 THEN d.truth20 ELSE d.truth19 END AS truth19,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost FROM outcome36 d
),
outcome38 AS (
SELECT d.*,CASE WHEN d.truth19>=0 THEN d.truth19 END AS m3truth,d.start16-d.work AS m3cost FROM outcome37 d
),
outcome39 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1,d.truth19 AS truth19,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome38 d
),
outcome40 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth19<0 THEN d.truth19 WHEN d.truth19=1 THEN 1 WHEN d.truth19=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth19=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome39 d
),
outcome41 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m3cost THEN -2 ELSE d.m3truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m3cost) ELSE d.work END AS remaining FROM outcome40 d
),
outcome42 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth24,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome41 d
),
outcome43 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth24<0 THEN d.truth24 WHEN d.truth24=1 THEN 1 WHEN d.truth24=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth24=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome42 d
),
outcome44 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m3cost THEN -2 ELSE d.m3truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m3cost) ELSE d.work END AS remaining FROM outcome43 d
),
outcome45 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth25,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome44 d
),
outcome46 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth25<0 THEN d.truth25 WHEN d.truth25=1 THEN 1 WHEN d.truth25=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth25=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome45 d
),
outcome47 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m3cost THEN -2 ELSE d.m3truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m3cost) ELSE d.work END AS remaining FROM outcome46 d
),
outcome48 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth26,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome47 d
),
outcome49 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth26<0 THEN d.truth26 WHEN d.truth26=1 THEN 1 WHEN d.truth26=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth26=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome48 d
),
outcome50 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m3cost THEN -2 ELSE d.m3truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m3cost) ELSE d.work END AS remaining FROM outcome49 d
),
outcome51 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth27,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome50 d
),
outcome52 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth27<0 THEN d.truth27 WHEN d.truth27=1 THEN 1 WHEN d.truth27=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth27=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome51 d
),
outcome53 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m3cost THEN -2 ELSE d.m3truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m3cost) ELSE d.work END AS remaining FROM outcome52 d
),
outcome54 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth28,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome53 d
),
outcome55 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth28<0 THEN d.truth28 WHEN d.truth28=1 THEN 1 WHEN d.truth28=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth28=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome54 d
),
outcome56 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m3cost THEN -2 ELSE d.m3truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m3cost) ELSE d.work END AS remaining FROM outcome55 d
),
outcome57 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth29,d.unknown2 AS unknown2 FROM outcome56 d
),
outcome58 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth29<0 THEN d.truth29 WHEN d.truth29=1 THEN 1 WHEN d.truth29=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth29=2,0) AS unknown2 FROM outcome57 d
),
outcome59 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1=0 AND d.unknown2 THEN 2 ELSE d.truth1 END AS truth1 FROM outcome58 d
),
outcome60 AS MATERIALIZED (
SELECT d.k,d.work,d.truth1 AS truth FROM outcome59 d
) SELECT o.truth,o.work FROM outcome60 o)
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
