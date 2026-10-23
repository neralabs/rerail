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
outcome7 AS MATERIALIZED (
SELECT d.*,NULL AS memo_hit FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth3>0 AND NOT (0),0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(NULL) THEN -2 ELSE NULL END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (COALESCE(d.truth3>0 AND NOT (0),0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(NULL)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome7 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(1,d.file,d.node,CASE WHEN ((COALESCE(d.truth3>0 AND NOT (0),0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((COALESCE(d.truth3>0 AND NOT (0),0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome9 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.tested>=0 THEN d.tested END AS m0truth,d.work-d.remaining AS m0cost FROM outcome8 d
),
outcome10 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.truth3 AS truth3,d.tested AS truth6,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome9 d
),
outcome11 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1,CASE WHEN d.truth3 IS NULL THEN NULL WHEN d.truth3<0 THEN d.truth3 WHEN d.truth6<0 THEN d.truth6 WHEN d.truth3=0 OR d.truth6=0 THEN 0 WHEN d.truth3=2 OR d.truth6=2 THEN 2 ELSE 1 END AS truth3,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome10 d
),
outcome12 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1,d.truth3 AS truth3,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome11 d
),
outcome13 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth3<0 THEN d.truth3 WHEN d.truth3=1 THEN 1 WHEN d.truth3=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth3=2,0) AS unknown2,d.m0truth,d.m0cost FROM outcome12 d
),
outcome14 AS MATERIALIZED (
SELECT d.*,CASE WHEN COALESCE(d.truth1=0,0) THEN CASE WHEN d.m0truth>=0 THEN 0 END END AS memo_hit FROM outcome13 d
),
outcome15 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(CASE d.memo_hit WHEN 0 THEN d.m0cost END) THEN -2 ELSE CASE d.memo_hit WHEN 0 THEN d.m0truth END END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(CASE d.memo_hit WHEN 0 THEN d.m0cost END)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome14 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(2,d.file,d.node,CASE WHEN ((COALESCE(d.truth1=0,0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((COALESCE(d.truth1=0,0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome16 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.tested>=0 THEN d.tested END AS m1truth,d.work-d.remaining AS m1cost FROM outcome15 d
),
outcome17 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth7,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome16 d
),
outcome18 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth7<0 THEN d.truth7 WHEN d.truth7=1 THEN 1 WHEN d.truth7=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth7=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome17 d
),
outcome19 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome18 d
),
outcome20 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth8,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome19 d
),
outcome21 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth8<0 THEN d.truth8 WHEN d.truth8=1 THEN 1 WHEN d.truth8=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth8=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome20 d
),
outcome22 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome21 d
),
outcome23 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth9,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome22 d
),
outcome24 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth9<0 THEN d.truth9 WHEN d.truth9=1 THEN 1 WHEN d.truth9=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth9=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome23 d
),
outcome25 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome24 d
),
outcome26 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth10,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome25 d
),
outcome27 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth10<0 THEN d.truth10 WHEN d.truth10=1 THEN 1 WHEN d.truth10=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth10=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome26 d
),
outcome28 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome27 d
),
outcome29 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth11,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome28 d
),
outcome30 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth11<0 THEN d.truth11 WHEN d.truth11=1 THEN 1 WHEN d.truth11=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth11=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome29 d
),
outcome31 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome30 d
),
outcome32 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth12,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome31 d
),
outcome33 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth12<0 THEN d.truth12 WHEN d.truth12=1 THEN 1 WHEN d.truth12=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth12=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome32 d
),
outcome34 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome33 d
),
outcome35 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth13,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome34 d
),
outcome36 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth13<0 THEN d.truth13 WHEN d.truth13=1 THEN 1 WHEN d.truth13=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth13=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome35 d
),
outcome37 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome36 d
),
outcome38 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth14,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome37 d
),
outcome39 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth14<0 THEN d.truth14 WHEN d.truth14=1 THEN 1 WHEN d.truth14=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth14=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome38 d
),
outcome40 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome39 d
),
outcome41 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth15,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome40 d
),
outcome42 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth15<0 THEN d.truth15 WHEN d.truth15=1 THEN 1 WHEN d.truth15=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth15=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome41 d
),
outcome43 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome42 d
),
outcome44 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth16,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome43 d
),
outcome45 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth16<0 THEN d.truth16 WHEN d.truth16=1 THEN 1 WHEN d.truth16=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth16=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome44 d
),
outcome46 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome45 d
),
outcome47 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth17,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome46 d
),
outcome48 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth17<0 THEN d.truth17 WHEN d.truth17=1 THEN 1 WHEN d.truth17=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth17=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome47 d
),
outcome49 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome48 d
),
outcome50 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth18,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome49 d
),
outcome51 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth18<0 THEN d.truth18 WHEN d.truth18=1 THEN 1 WHEN d.truth18=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth18=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome50 d
),
outcome52 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome51 d
),
outcome53 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth19,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome52 d
),
outcome54 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth19<0 THEN d.truth19 WHEN d.truth19=1 THEN 1 WHEN d.truth19=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth19=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome53 d
),
outcome55 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome54 d
),
outcome56 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth20,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome55 d
),
outcome57 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth20<0 THEN d.truth20 WHEN d.truth20=1 THEN 1 WHEN d.truth20=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth20=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome56 d
),
outcome58 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome57 d
),
outcome59 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth21,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome58 d
),
outcome60 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth21<0 THEN d.truth21 WHEN d.truth21=1 THEN 1 WHEN d.truth21=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth21=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome59 d
),
outcome61 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome60 d
),
outcome62 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth22,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome61 d
),
outcome63 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth22<0 THEN d.truth22 WHEN d.truth22=1 THEN 1 WHEN d.truth22=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth22=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome62 d
),
outcome64 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome63 d
),
outcome65 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth23,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome64 d
),
outcome66 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth23<0 THEN d.truth23 WHEN d.truth23=1 THEN 1 WHEN d.truth23=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth23=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome65 d
),
outcome67 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome66 d
),
outcome68 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth24,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome67 d
),
outcome69 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth24<0 THEN d.truth24 WHEN d.truth24=1 THEN 1 WHEN d.truth24=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth24=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome68 d
),
outcome70 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome69 d
),
outcome71 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth25,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome70 d
),
outcome72 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth25<0 THEN d.truth25 WHEN d.truth25=1 THEN 1 WHEN d.truth25=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth25=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome71 d
),
outcome73 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome72 d
),
outcome74 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth26,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome73 d
),
outcome75 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth26<0 THEN d.truth26 WHEN d.truth26=1 THEN 1 WHEN d.truth26=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth26=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome74 d
),
outcome76 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome75 d
),
outcome77 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth27,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome76 d
),
outcome78 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth27<0 THEN d.truth27 WHEN d.truth27=1 THEN 1 WHEN d.truth27=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth27=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome77 d
),
outcome79 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome78 d
),
outcome80 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth28,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome79 d
),
outcome81 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth28<0 THEN d.truth28 WHEN d.truth28=1 THEN 1 WHEN d.truth28=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth28=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome80 d
),
outcome82 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome81 d
),
outcome83 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth29,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome82 d
),
outcome84 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth29<0 THEN d.truth29 WHEN d.truth29=1 THEN 1 WHEN d.truth29=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth29=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome83 d
),
outcome85 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome84 d
),
outcome86 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth30,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome85 d
),
outcome87 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth30<0 THEN d.truth30 WHEN d.truth30=1 THEN 1 WHEN d.truth30=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth30=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome86 d
),
outcome88 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome87 d
),
outcome89 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth31,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome88 d
),
outcome90 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth31<0 THEN d.truth31 WHEN d.truth31=1 THEN 1 WHEN d.truth31=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth31=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome89 d
),
outcome91 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome90 d
),
outcome92 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth32,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome91 d
),
outcome93 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth32<0 THEN d.truth32 WHEN d.truth32=1 THEN 1 WHEN d.truth32=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth32=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome92 d
),
outcome94 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome93 d
),
outcome95 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth33,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome94 d
),
outcome96 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth33<0 THEN d.truth33 WHEN d.truth33=1 THEN 1 WHEN d.truth33=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth33=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome95 d
),
outcome97 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome96 d
),
outcome98 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth34,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome97 d
),
outcome99 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth34<0 THEN d.truth34 WHEN d.truth34=1 THEN 1 WHEN d.truth34=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth34=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome98 d
),
outcome100 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome99 d
),
outcome101 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth35,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome100 d
),
outcome102 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth35<0 THEN d.truth35 WHEN d.truth35=1 THEN 1 WHEN d.truth35=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth35=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome101 d
),
outcome103 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome102 d
),
outcome104 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth36,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome103 d
),
outcome105 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth36<0 THEN d.truth36 WHEN d.truth36=1 THEN 1 WHEN d.truth36=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth36=2,0) AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome104 d
),
outcome106 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1=0,0)) THEN NULL WHEN d.work<d.m1cost THEN -2 ELSE d.m1truth END AS tested,CASE WHEN COALESCE(d.truth1=0,0) THEN MAX(0,d.work-d.m1cost) ELSE d.work END AS remaining FROM outcome105 d
),
outcome107 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth37,d.unknown2 AS unknown2 FROM outcome106 d
),
outcome108 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth37<0 THEN d.truth37 WHEN d.truth37=1 THEN 1 WHEN d.truth37=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth37=2,0) AS unknown2 FROM outcome107 d
),
outcome109 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1=0 AND d.unknown2 THEN 2 ELSE d.truth1 END AS truth1 FROM outcome108 d
),
outcome110 AS MATERIALIZED (
SELECT d.k,d.work,d.truth1 AS truth FROM outcome109 d
) SELECT o.truth,o.work FROM outcome110 o)
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
