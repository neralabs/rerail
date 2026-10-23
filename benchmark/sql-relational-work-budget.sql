WITH RECURSIVE
outcome0 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,193 AS work,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (256)
),
outcome1 AS (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,193 AS work,NULL AS c0 FROM outcome0 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1,r.c2 FROM outcome1 w CROSS JOIN outcome0 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1,r.c2) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,w.work AS work WHERE 1
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work,1 AS truth,NULL AS c0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (256))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining,r.c2 AS e0 FROM outcome3 d CROSS JOIN relation_row r ON (r.c0,r.c1,r.c2) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work,d.c0 AS c0 WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,CASE WHEN origin.id<origin."last" THEN origin.id+1 END AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE (CASE WHEN origin.id<origin."last" THEN origin.id+1 END) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, CASE WHEN w.stopped<0 THEN 0 WHEN w.work=0 THEN -2 ELSE COALESCE(r.c0,0) END,w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,w.work-1) FROM node probe LEFT JOIN relation_value_budget(0,w.file,n.id,CASE WHEN (w.stopped>=0 AND w.work>0) AND ((probe.kind IN (110))) THEN w.work-1 END) v ON 1 WHERE probe.file=w.file AND probe.id=n.id AND (w.stopped>=0 AND w.work>0)) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,CASE WHEN (n.next)<=w.bound THEN (n.next) END,w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR ((w.low+(w.own=1))>65)) AND (CASE WHEN (n.next)<=w.bound THEN (n.next) END) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR ((w.low+(w.own=1))>65)) OR (CASE WHEN (n.next)<=w.bound THEN (n.next) END) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN (NOT COALESCE(t.open,0) AND (COALESCE(t.low,0)+COALESCE(t.uncertain,0))<0) OR COALESCE(t.low,0)>65 THEN 0 WHEN COALESCE(t.low,0)>=0 AND (NOT COALESCE(t.open,0) AND (COALESCE(t.low,0)+COALESCE(t.uncertain,0))<=65) AND (NOT COALESCE(t.open,0) AND COALESCE(t.low,0)=(COALESCE(t.low,0)+COALESCE(t.uncertain,0))) THEN 1 ELSE 2 END END AS truth,COALESCE(t.low,0)+(0) AS c0 FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work,o.c0 AS c0 FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.e0 ELSE d.c0 END AS c0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth,s.c0 AS c0 FROM outcome5 s
) SELECT o.truth,o.work,o.c0 AS c0 FROM outcome6 o) WHERE w.truth>=0
),
outcome2 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,193 AS work,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (110)
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.work,1 AS truth,NULL AS c0,0 AS u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN d.truth>0 AND NOT (d.u0) THEN CASE WHEN n.named AND (CASE WHEN d.c0 IS NULL THEN 1 ELSE relation_capture_equal(d.file,d.c0,d.node) END) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome3 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,CASE WHEN d.tested=1 THEN d.node ELSE d.c0 END AS c0,(d.u0 OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u0 FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,d.work AS remaining,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (110))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome5 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT s.k,s.work,s.truth,s.c0 AS c0 FROM outcome7 s
),
outcome9 AS MATERIALIZED (
SELECT 0 AS clause,o.file,o.node,o.truth,o.work,o.c0 AS v0 FROM outcome1 o WHERE o.truth=1 OR o.truth<0
UNION ALL
SELECT 1 AS clause,d.file,d.node,o.truth,o.work,o.c0 AS v0 FROM outcome2 d JOIN outcome8 o ON o.k=d.k WHERE o.truth=1 OR o.truth<0
),
outcome10 AS MATERIALIZED (
SELECT clause,file,node,truth,work FROM outcome9 WHERE truth<0
)
-- final matches
SELECT m.clause,m.file,m.node,CASE m.clause WHEN 0 THEN json_array(CAST(m.v0 AS TEXT)) WHEN 1 THEN json_array(m.v0) END AS payload,0 AS failure,m.work FROM outcome9 m WHERE m.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome10 f WHERE f.clause=m.clause AND f.file=m.file)
UNION ALL
SELECT clause,file,node,'[]',truth,work FROM outcome10
ORDER BY 1,2,3
