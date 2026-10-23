WITH RECURSIVE
outcome0 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,10000 AS work,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE 1 AND n.kind IN (256)
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
SELECT d.*,d.work AS remaining,CASE WHEN d.truth>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (256))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome3 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (1) AND node_regex(1,candidate.file,candidate.id)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node
),
outcome2 AS MATERIALIZED (
SELECT k,file,node,bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,work FROM outcome1 WHERE node IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome2 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
),
outcome1 AS MATERIALIZED (
SELECT d.k,COALESCE(r.c1,0) AS work,CASE WHEN d.work=0 THEN -2 ELSE r.c0 END AS truth FROM outcome0 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT CASE WHEN (probe.kind IN (1)) AND (NOT (NOT EXISTS (SELECT 1 WHERE node_regex(0,probe.file,probe.id)))) THEN 1 ELSE 0 END,d.work-1 FROM node probe WHERE probe.file=d.file AND probe.id=d.node AND (d.work>0))
) SELECT o.truth,o.work FROM outcome1 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome2 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (1) AND node_regex(1,candidate.file,candidate.id)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome2 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (1) AND node_regex(1,candidate.file,candidate.id)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome3 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome2 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (1) AND node_regex(1,candidate.file,candidate.id)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome4 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome3 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome4 o)
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
