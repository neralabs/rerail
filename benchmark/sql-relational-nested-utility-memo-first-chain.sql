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
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,NULL AS hit5,NULL AS memoCost6,COALESCE(COALESCE(d.truth1>0 AND NOT (0),0),0) AS reached3,d.work AS start4,d.truth1 AS truth1 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN NOT d.reached3 THEN d.work WHEN d.hit5 IS NOT NULL THEN MAX(0,d.work-d.memoCost6) ELSE MAX(0,d.work-1) END AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,CASE WHEN NOT d.reached3 THEN NULL WHEN d.hit5 IS NOT NULL THEN CASE WHEN d.work<d.memoCost6 THEN -2 ELSE d.hit5 END WHEN d.work=0 THEN -2 END AS truth7 FROM outcome3 d
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,NULL AS hit10,d.hit5 AS hit5,NULL AS memoCost11,d.memoCost6 AS memoCost6,d.reached3 AS reached3,COALESCE(d.reached3 AND d.hit5 IS NULL AND d.start4>0,0) AS reached8,d.start4 AS start4,d.work AS start9,d.truth1 AS truth1,d.truth7 AS truth7 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN NOT d.reached8 THEN d.work WHEN d.hit10 IS NOT NULL THEN MAX(0,d.work-d.memoCost11) ELSE MAX(0,d.work-1) END AS work,d.hit10 AS hit10,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.reached8 AS reached8,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,CASE WHEN NOT d.reached8 THEN NULL WHEN d.hit10 IS NOT NULL THEN CASE WHEN d.work<d.memoCost11 THEN -2 ELSE d.hit10 END WHEN d.work=0 THEN -2 END AS truth12,d.truth7 AS truth7 FROM outcome5 d
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,NULL AS hit15,d.hit5 AS hit5,d.memoCost11 AS memoCost11,NULL AS memoCost16,d.memoCost6 AS memoCost6,COALESCE(d.reached8 AND d.hit10 IS NULL AND d.start9>0,0) AS reached13,d.reached3 AS reached3,d.reached8 AS reached8,d.work AS start14,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth7 AS truth7 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN NOT d.reached13 THEN d.work WHEN d.hit15 IS NOT NULL THEN MAX(0,d.work-d.memoCost16) ELSE MAX(0,d.work-1) END AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,CASE WHEN NOT d.reached13 THEN NULL WHEN d.hit15 IS NOT NULL THEN CASE WHEN d.work<d.memoCost16 THEN -2 ELSE d.hit15 END WHEN d.work=0 THEN -2 END AS truth17,d.truth7 AS truth7 FROM outcome7 d
),
outcome9 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,NULL AS hit20,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,NULL AS memoCost21,d.memoCost6 AS memoCost6,d.reached13 AS reached13,COALESCE(d.reached13 AND d.hit15 IS NULL AND d.start14>0,0) AS reached18,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.work AS start19,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth7 AS truth7 FROM outcome8 d
),
outcome10 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN NOT d.reached18 THEN d.work WHEN d.hit20 IS NOT NULL THEN MAX(0,d.work-d.memoCost21) ELSE MAX(0,d.work-1) END AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,CASE WHEN NOT d.reached18 THEN NULL WHEN d.hit20 IS NOT NULL THEN CASE WHEN d.work<d.memoCost21 THEN -2 ELSE d.hit20 END WHEN d.work=0 THEN -2 END AS truth22,d.truth7 AS truth7 FROM outcome9 d
),
outcome11 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,NULL AS hit25,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,NULL AS memoCost26,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,COALESCE(d.reached18 AND d.hit20 IS NULL AND d.start19>0,0) AS reached23,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.work AS start24,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth7 AS truth7 FROM outcome10 d
),
outcome12 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN NOT d.reached23 THEN d.work WHEN d.hit25 IS NOT NULL THEN MAX(0,d.work-d.memoCost26) ELSE MAX(0,d.work-1) END AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit25 AS hit25,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost26 AS memoCost26,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached23 AS reached23,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start24 AS start24,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,CASE WHEN NOT d.reached23 THEN NULL WHEN d.hit25 IS NOT NULL THEN CASE WHEN d.work<d.memoCost26 THEN -2 ELSE d.hit25 END WHEN d.work=0 THEN -2 END AS truth27,d.truth7 AS truth7 FROM outcome11 d
),
outcome13 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit25 AS hit25,NULL AS hit30,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost26 AS memoCost26,NULL AS memoCost31,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached23 AS reached23,COALESCE(d.reached23 AND d.hit25 IS NULL AND d.start24>0,0) AS reached28,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start24 AS start24,d.work AS start29,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth27 AS truth27,d.truth7 AS truth7 FROM outcome12 d
),
outcome14 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN NOT d.reached28 THEN d.work WHEN d.hit30 IS NOT NULL THEN MAX(0,d.work-d.memoCost31) ELSE MAX(0,d.work-1) END AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit25 AS hit25,d.hit30 AS hit30,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost26 AS memoCost26,d.memoCost31 AS memoCost31,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached23 AS reached23,d.reached28 AS reached28,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start24 AS start24,d.start29 AS start29,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth27 AS truth27,CASE WHEN NOT d.reached28 THEN NULL WHEN d.hit30 IS NOT NULL THEN CASE WHEN d.work<d.memoCost31 THEN -2 ELSE d.hit30 END WHEN d.work=0 THEN -2 END AS truth32,d.truth7 AS truth7 FROM outcome13 d
),
outcome15 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit25 AS hit25,d.hit30 AS hit30,NULL AS hit35,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost26 AS memoCost26,d.memoCost31 AS memoCost31,NULL AS memoCost36,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached23 AS reached23,d.reached28 AS reached28,d.reached3 AS reached3,COALESCE(d.reached28 AND d.hit30 IS NULL AND d.start29>0,0) AS reached33,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start24 AS start24,d.start29 AS start29,d.work AS start34,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth27 AS truth27,d.truth32 AS truth32,d.truth7 AS truth7 FROM outcome14 d
),
outcome16 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN NOT d.reached33 THEN d.work WHEN d.hit35 IS NOT NULL THEN MAX(0,d.work-d.memoCost36) ELSE MAX(0,d.work-1) END AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit25 AS hit25,d.hit30 AS hit30,d.hit35 AS hit35,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost26 AS memoCost26,d.memoCost31 AS memoCost31,d.memoCost36 AS memoCost36,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached23 AS reached23,d.reached28 AS reached28,d.reached3 AS reached3,d.reached33 AS reached33,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start24 AS start24,d.start29 AS start29,d.start34 AS start34,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth27 AS truth27,d.truth32 AS truth32,CASE WHEN NOT d.reached33 THEN NULL WHEN d.hit35 IS NOT NULL THEN CASE WHEN d.work<d.memoCost36 THEN -2 ELSE d.hit35 END WHEN d.work=0 THEN -2 END AS truth37,d.truth7 AS truth7 FROM outcome15 d
),
outcome17 AS MATERIALIZED (
SELECT d.*,NULL AS memo_hit FROM outcome16 d
),
outcome18 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (d.reached33 AND d.hit35 IS NULL AND d.start34>0) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(NULL) THEN -2 ELSE NULL END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (d.reached33 AND d.hit35 IS NULL AND d.start34>0) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(NULL)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome17 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(0,d.file,d.node,CASE WHEN ((d.reached33 AND d.hit35 IS NULL AND d.start34>0) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((d.reached33 AND d.hit35 IS NULL AND d.start34>0) AND d.memo_hit IS NULL AND d.work>0))
),
outcome19 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.tested>=0 THEN d.tested END AS m0truth,d.work-d.remaining AS m0cost FROM outcome18 d
),
outcome20 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit25 AS hit25,d.hit30 AS hit30,d.hit35 AS hit35,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost26 AS memoCost26,d.memoCost31 AS memoCost31,d.memoCost36 AS memoCost36,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached23 AS reached23,d.reached28 AS reached28,d.reached3 AS reached3,d.reached33 AS reached33,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start24 AS start24,d.start29 AS start29,d.start34 AS start34,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth27 AS truth27,d.truth32 AS truth32,d.truth37 AS truth37,d.tested AS truth38,d.truth7 AS truth7,d.m0truth,d.m0cost FROM outcome19 d
),
outcome21 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit25 AS hit25,d.hit30 AS hit30,d.hit35 AS hit35,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost26 AS memoCost26,d.memoCost31 AS memoCost31,d.memoCost36 AS memoCost36,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached23 AS reached23,d.reached28 AS reached28,d.reached3 AS reached3,d.reached33 AS reached33,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start24 AS start24,d.start29 AS start29,d.start34 AS start34,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth27 AS truth27,d.truth32 AS truth32,CASE WHEN d.reached33 AND d.hit35 IS NULL AND d.start34>0 THEN d.truth38 ELSE d.truth37 END AS truth37,d.truth7 AS truth7,d.m0truth,d.m0cost FROM outcome20 d
),
outcome22 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth37>=0 THEN d.truth37 END AS m1truth,d.start34-d.work AS m1cost FROM outcome21 d
),
outcome23 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit25 AS hit25,d.hit30 AS hit30,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost26 AS memoCost26,d.memoCost31 AS memoCost31,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached23 AS reached23,d.reached28 AS reached28,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start24 AS start24,d.start29 AS start29,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth27 AS truth27,d.truth32 AS truth32,d.truth37 AS truth37,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome22 d
),
outcome24 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit25 AS hit25,d.hit30 AS hit30,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost26 AS memoCost26,d.memoCost31 AS memoCost31,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached23 AS reached23,d.reached28 AS reached28,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start24 AS start24,d.start29 AS start29,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth27 AS truth27,CASE WHEN d.reached28 AND d.hit30 IS NULL AND d.start29>0 THEN d.truth37 ELSE d.truth32 END AS truth32,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome23 d
),
outcome25 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth32>=0 THEN d.truth32 END AS m2truth,d.start29-d.work AS m2cost FROM outcome24 d
),
outcome26 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit25 AS hit25,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost26 AS memoCost26,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached23 AS reached23,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start24 AS start24,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth27 AS truth27,d.truth32 AS truth32,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost FROM outcome25 d
),
outcome27 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit25 AS hit25,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost26 AS memoCost26,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached23 AS reached23,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start24 AS start24,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,CASE WHEN d.reached23 AND d.hit25 IS NULL AND d.start24>0 THEN d.truth32 ELSE d.truth27 END AS truth27,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost FROM outcome26 d
),
outcome28 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth27>=0 THEN d.truth27 END AS m3truth,d.start24-d.work AS m3cost FROM outcome27 d
),
outcome29 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth27 AS truth27,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome28 d
),
outcome30 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit20 AS hit20,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost21 AS memoCost21,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached18 AS reached18,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start19 AS start19,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,CASE WHEN d.reached18 AND d.hit20 IS NULL AND d.start19>0 THEN d.truth27 ELSE d.truth22 END AS truth22,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost FROM outcome29 d
),
outcome31 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth22>=0 THEN d.truth22 END AS m4truth,d.start19-d.work AS m4cost FROM outcome30 d
),
outcome32 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth22 AS truth22,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost,d.m4truth,d.m4cost FROM outcome31 d
),
outcome33 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit15 AS hit15,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost16 AS memoCost16,d.memoCost6 AS memoCost6,d.reached13 AS reached13,d.reached3 AS reached3,d.reached8 AS reached8,d.start14 AS start14,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,CASE WHEN d.reached13 AND d.hit15 IS NULL AND d.start14>0 THEN d.truth22 ELSE d.truth17 END AS truth17,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost,d.m4truth,d.m4cost FROM outcome32 d
),
outcome34 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth17>=0 THEN d.truth17 END AS m5truth,d.start14-d.work AS m5cost FROM outcome33 d
),
outcome35 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.reached8 AS reached8,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,d.truth12 AS truth12,d.truth17 AS truth17,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost,d.m4truth,d.m4cost,d.m5truth,d.m5cost FROM outcome34 d
),
outcome36 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit10 AS hit10,d.hit5 AS hit5,d.memoCost11 AS memoCost11,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.reached8 AS reached8,d.start4 AS start4,d.start9 AS start9,d.truth1 AS truth1,CASE WHEN d.reached8 AND d.hit10 IS NULL AND d.start9>0 THEN d.truth17 ELSE d.truth12 END AS truth12,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost,d.m4truth,d.m4cost,d.m5truth,d.m5cost FROM outcome35 d
),
outcome37 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth12>=0 THEN d.truth12 END AS m6truth,d.start9-d.work AS m6cost FROM outcome36 d
),
outcome38 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth12 AS truth12,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost,d.m4truth,d.m4cost,d.m5truth,d.m5cost,d.m6truth,d.m6cost FROM outcome37 d
),
outcome39 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,CASE WHEN d.reached3 AND d.hit5 IS NULL AND d.start4>0 THEN d.truth12 ELSE d.truth7 END AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost,d.m4truth,d.m4cost,d.m5truth,d.m5cost,d.m6truth,d.m6cost FROM outcome38 d
),
outcome40 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth7>=0 THEN d.truth7 END AS m7truth,d.start4-d.work AS m7cost FROM outcome39 d
),
outcome41 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1,d.truth7 AS truth7,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost,d.m4truth,d.m4cost,d.m5truth,d.m5cost,d.m6truth,d.m6cost,d.m7truth,d.m7cost FROM outcome40 d
),
outcome42 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth7<0 THEN d.truth7 WHEN d.truth1=0 OR d.truth7=0 THEN 0 WHEN d.truth1=2 OR d.truth7=2 THEN 2 ELSE 1 END AS truth1,d.m0truth,d.m0cost,d.m1truth,d.m1cost,d.m2truth,d.m2cost,d.m3truth,d.m3cost,d.m4truth,d.m4cost,d.m5truth,d.m5cost,d.m6truth,d.m6cost,d.m7truth,d.m7cost FROM outcome41 d
),
outcome43 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth1>0 AND NOT (0),0)) THEN NULL WHEN d.work<d.m7cost THEN -2 ELSE d.m7truth END AS tested,CASE WHEN COALESCE(d.truth1>0 AND NOT (0),0) THEN MAX(0,d.work-d.m7cost) ELSE d.work END AS remaining FROM outcome42 d
),
outcome44 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.truth1 AS truth1,d.tested AS truth39 FROM outcome43 d
),
outcome45 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<0 THEN d.truth1 WHEN d.truth39<0 THEN d.truth39 WHEN d.truth1=0 OR d.truth39=0 THEN 0 WHEN d.truth1=2 OR d.truth39=2 THEN 2 ELSE 1 END AS truth1 FROM outcome44 d
),
outcome46 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1 FROM outcome45 d
),
outcome47 AS MATERIALIZED (
SELECT d.k,d.work,d.truth1 AS truth FROM outcome46 d
) SELECT o.truth,o.work FROM outcome47 o)
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
