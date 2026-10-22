WITH RECURSIVE
outcome0 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node FROM node n WHERE 1 AND n.kind IN (256)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth<>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (256))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.k,d.file,d.node FROM outcome3 d WHERE d.truth<>0 AND NOT (0)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,CASE WHEN origin.id<origin."last" THEN origin.id+1 END AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open FROM outcome4 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE (CASE WHEN origin.id<origin."last" THEN origin.id+1 END) IS NOT NULL
UNION ALL
SELECT w.k,w.file,CASE WHEN w.phase=2 THEN CASE WHEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END)<=w.bound THEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END) END ELSE w.node END,w.bound,(w.phase+1)%3,CASE WHEN w.phase=0 THEN (WITH prior AS MATERIALIZED (SELECT CASE WHEN (n.kind IN (110)) OR (n.kind IN (1)) THEN 1 ELSE 0 END AS t) SELECT CASE WHEN t=0 THEN 0 ELSE (WITH following AS MATERIALIZED (SELECT relation_value(1,n.file,n.id) AS v) SELECT CASE WHEN v=0 THEN 0 WHEN v=2 OR prior.t=2 THEN 2 ELSE v END FROM following) END FROM prior) ELSE w.own END, CASE WHEN w.phase=1 THEN CASE WHEN w.own=1 THEN 0 ELSE CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(0,n.file,n.id))) THEN 1 ELSE 0 END END ELSE w.stopped END,CASE WHEN w.phase=2 THEN (w.low+(w.own=1)) ELSE w.low END,CASE WHEN w.phase=2 THEN (w.uncertain+(w.own=2)) ELSE w.uncertain END,CASE WHEN w.phase=2 THEN (w.open OR w.stopped=2) ELSE w.open END FROM outcome5 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase<2 OR (NOT (w.own=1) AND (CASE WHEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END)<=w.bound THEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END) END) IS NOT NULL)
),
outcome6 AS MATERIALIZED (
SELECT w.k,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome5 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own=1) OR (CASE WHEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END)<=w.bound THEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END) END) IS NULL)
),
outcome7 AS MATERIALIZED (
SELECT d.k,CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END AS truth FROM outcome4 d LEFT JOIN outcome6 t ON t.k=d.k
),
outcome8 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR o.truth=0 THEN 0 WHEN d.truth=2 OR o.truth=2 THEN 2 ELSE 1 END AS truth FROM outcome3 d LEFT JOIN outcome7 o ON o.k=d.k
),
outcome9 AS MATERIALIZED (
SELECT s.k,s.truth FROM outcome8 s
)
-- final matches
SELECT 0,d.file,d.node,json_array() FROM outcome0 d JOIN outcome9 o ON o.k=d.k WHERE o.truth=1 ORDER BY d.file,d.node
