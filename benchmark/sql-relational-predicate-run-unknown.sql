WITH RECURSIVE
outcome0 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,10000 AS work,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (1)
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
SELECT d.*,d.work AS remaining,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (1))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
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
outcome3 AS (
SELECT d.k,d.file,d.node,d.work AS work,NULL AS hit5,NULL AS memoCost6,COALESCE(COALESCE(d.truth1=0,0),0) AS reached3,d.work AS start4,d.truth1 AS truth1,d.unknown2 AS unknown2 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN NOT d.reached3 THEN d.work WHEN d.hit5 IS NOT NULL THEN MAX(0,d.work-d.memoCost6) ELSE MAX(0,d.work-1) END AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,CASE WHEN NOT d.reached3 THEN NULL WHEN d.hit5 IS NOT NULL THEN CASE WHEN d.work<d.memoCost6 THEN -2 ELSE d.hit5 END WHEN d.work=0 THEN -2 END AS truth7,d.unknown2 AS unknown2 FROM outcome3 d
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.reached3 AND d.hit5 IS NULL AND d.start4>0 THEN 1 END AS truth8,d.unknown2 AS unknown2 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,CASE WHEN COALESCE(d.truth8>0 AND NOT (0),0) THEN CASE WHEN (CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(0,n.file,n.id))) THEN 1 ELSE 0 END)=0 THEN 0 WHEN (CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(1,n.file,n.id))) THEN 1 ELSE 0 END)=0 THEN 0 WHEN (CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(2,n.file,n.id))) THEN 1 ELSE 0 END)=0 THEN 0 WHEN (CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(3,n.file,n.id))) THEN 1 ELSE 0 END)=0 THEN 0 WHEN (CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(4,n.file,n.id))) THEN 1 ELSE 0 END)=0 THEN 0 WHEN (CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(5,n.file,n.id))) THEN 1 ELSE 0 END)=0 THEN 0 WHEN (CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(6,n.file,n.id))) THEN 1 ELSE 0 END)=0 THEN 0 WHEN (CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(7,n.file,n.id))) THEN 1 ELSE 0 END)=0 THEN 0 ELSE 1 END END AS truth10 FROM outcome5 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.truth8 IS NULL THEN NULL WHEN d.truth8<0 THEN d.truth8 WHEN d.truth10<0 THEN d.truth10 WHEN d.truth8=0 OR d.truth10=0 THEN 0 WHEN d.truth8=2 OR d.truth10=2 THEN 2 ELSE 1 END AS truth8,d.unknown2 AS unknown2 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT d.*,NULL AS memo_hit FROM outcome7 d
),
outcome9 AS MATERIALIZED (
SELECT d.*,CASE WHEN NOT (COALESCE(d.truth8>0 AND NOT (0),0)) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<(NULL) THEN -2 ELSE NULL END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT (COALESCE(d.truth8>0 AND NOT (0),0)) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-(NULL)) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome8 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(8,d.file,d.node,CASE WHEN ((COALESCE(d.truth8>0 AND NOT (0),0)) AND d.memo_hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND ((COALESCE(d.truth8>0 AND NOT (0),0)) AND d.memo_hit IS NULL AND d.work>0))
),
outcome10 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.tested>=0 THEN d.tested END AS m0truth,d.work-d.remaining AS m0cost FROM outcome9 d
),
outcome11 AS (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.tested AS truth11,d.truth7 AS truth7,d.truth8 AS truth8,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome10 d
),
outcome12 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,CASE WHEN d.truth8 IS NULL THEN NULL WHEN d.truth8<0 THEN d.truth8 WHEN d.truth11<0 THEN d.truth11 WHEN d.truth8=0 OR d.truth11=0 THEN 0 WHEN d.truth8=2 OR d.truth11=2 THEN 2 ELSE 1 END AS truth8,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome11 d
),
outcome13 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,d.truth7 AS truth7,d.truth8 AS truth8,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome12 d
),
outcome14 AS (
SELECT d.k,d.file,d.node,d.work AS work,d.hit5 AS hit5,d.memoCost6 AS memoCost6,d.reached3 AS reached3,d.start4 AS start4,d.truth1 AS truth1,CASE WHEN d.reached3 AND d.hit5 IS NULL AND d.start4>0 THEN d.truth8 ELSE d.truth7 END AS truth7,d.unknown2 AS unknown2,d.m0truth,d.m0cost FROM outcome13 d
),
outcome15 AS (
SELECT d.*,CASE WHEN d.truth7>=0 THEN d.truth7 END AS m1truth,d.start4-d.work AS m1cost FROM outcome14 d
),
outcome16 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,d.truth1 AS truth1,d.truth7 AS truth7,d.unknown2 AS unknown2,d.m0truth,d.m0cost,d.m1truth,d.m1cost FROM outcome15 d
),
outcome17 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth7 IS NULL THEN d.work ELSE CASE WHEN d.truth7 NOT IN (0,2) THEN d.work WHEN d.m1cost>d.work/31 THEN 0 ELSE d.work-d.m1cost*31 END END AS work,d.truth1 AS truth1,CASE WHEN d.truth7 IN (0,2) AND (d.m1cost>d.work/31) THEN -2 ELSE d.truth7 END AS truth7,d.unknown2 AS unknown2 FROM outcome16 d
),
outcome18 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1 IS NULL THEN NULL WHEN d.truth1<>0 THEN d.truth1 WHEN d.truth7<0 THEN d.truth7 WHEN d.truth7=1 THEN 1 WHEN d.truth7=2 AND 0 THEN 2 ELSE 0 END AS truth1,d.unknown2 OR COALESCE(d.truth7=2,0) AS unknown2 FROM outcome17 d
),
outcome19 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work AS work,CASE WHEN d.truth1=0 AND d.unknown2 THEN 2 ELSE d.truth1 END AS truth1 FROM outcome18 d
),
outcome20 AS MATERIALIZED (
SELECT d.k,d.work,d.truth1 AS truth FROM outcome19 d
) SELECT o.truth,o.work FROM outcome20 o)
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
