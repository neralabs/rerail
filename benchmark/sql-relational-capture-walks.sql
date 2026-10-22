WITH RECURSIVE
outcome0 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node FROM node n WHERE 1 AND n.kind IN (256)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,d.node,1 AS truth,NULL AS c0,0 AS u0 FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.truth<>0 AND NOT (0) THEN CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (256))) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM outcome1 d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.c0 AS c0,d.u0 FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.k,d.file,d.node,d.c0 AS c0 FROM outcome3 d WHERE d.truth<>0 AND NOT (d.u0)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,CASE WHEN origin.id<origin."last" THEN origin.id+1 END AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open FROM outcome4 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE (CASE WHEN origin.id<origin."last" THEN origin.id+1 END) IS NOT NULL
UNION ALL
SELECT w.k,w.file,CASE WHEN w.phase=2 THEN CASE WHEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END)<=w.bound THEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END) END ELSE w.node END,w.bound,(w.phase+1)%3,CASE WHEN w.phase=0 THEN (SELECT (WITH candidate0 AS MATERIALIZED (SELECT 0 AS bound),
candidate1 AS MATERIALIZED (SELECT 1 AS truth,0 AS withheld,0 AS open,bound FROM candidate0),
candidate2 AS MATERIALIZED (SELECT s.*,CASE WHEN s.truth<>0 AND (s.withheld&1)=0 THEN CASE WHEN n.named AND (CASE WHEN (s.bound&1)<>0 OR d.c0 IS NULL THEN 1 ELSE relation_capture_equal(n.file,d.c0,n.id) END) THEN 1 ELSE 0 END END AS tested FROM candidate1 s),
candidate3 AS MATERIALIZED (SELECT CASE WHEN s.truth=0 OR s.tested=0 THEN 0 WHEN s.truth=2 OR s.tested=2 THEN 2 ELSE 1 END AS truth,s.withheld|CASE WHEN s.tested=2 OR (s.tested IS NULL AND s.truth<>0) THEN 1 ELSE 0 END AS withheld,0 AS open,CASE WHEN s.tested=1 THEN s.bound|1 ELSE s.bound END AS bound FROM candidate2 s),
candidate4 AS MATERIALIZED (SELECT s.*,CASE WHEN s.truth<>0 AND (s.withheld&0)=0 THEN CASE WHEN n.kind IN (1) THEN 1 ELSE 0 END END AS tested FROM candidate3 s),
candidate5 AS MATERIALIZED (SELECT CASE WHEN s.truth=0 OR s.tested=0 THEN 0 WHEN s.truth=2 OR s.tested=2 THEN 2 ELSE 1 END AS truth,s.withheld|CASE WHEN s.tested=2 OR (s.tested IS NULL AND s.truth<>0) THEN 0 ELSE 0 END AS withheld,0 AS open,s.bound AS bound FROM candidate4 s),
candidate6 AS MATERIALIZED (SELECT s.*,CASE WHEN s.truth<>0 AND (s.withheld&0)=0 THEN relation_value(1,n.file,n.id) END AS tested FROM candidate5 s),
candidate7 AS MATERIALIZED (SELECT CASE WHEN s.truth=0 OR s.tested=0 THEN 0 WHEN s.truth=2 OR s.tested=2 THEN 2 ELSE 1 END AS truth,s.withheld|CASE WHEN s.tested=2 OR (s.tested IS NULL AND s.truth<>0) THEN 0 ELSE 0 END AS withheld,0 AS open,s.bound AS bound FROM candidate6 s),
candidate8 AS MATERIALIZED (SELECT truth AS truth,bound FROM candidate7) SELECT CASE WHEN truth=1 THEN 1+((bound&1)<<2) ELSE truth END FROM candidate8) FROM outcome4 d WHERE d.k=w.k) ELSE w.own END, CASE WHEN w.phase=1 THEN CASE WHEN (w.own&3)=1 OR (w.own&3)=2 OR w.open THEN 0 ELSE CASE WHEN NOT (NOT EXISTS (SELECT 1 WHERE node_regex(0,n.file,n.id))) THEN 1 ELSE 0 END END ELSE w.stopped END,CASE WHEN w.phase=2 THEN (w.low+((w.own&3)=1)) ELSE w.low END,CASE WHEN w.phase=2 THEN (w.uncertain+((w.own&3)=2)) ELSE w.uncertain END,CASE WHEN w.phase=2 THEN (w.open OR w.stopped=2) ELSE w.open END FROM outcome5 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase<2 OR (NOT ((w.own&3)=1 OR (w.own&3)=2 OR w.open) AND (CASE WHEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END)<=w.bound THEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END) END) IS NOT NULL)
),
outcome6 AS MATERIALIZED (
SELECT w.k,w.node AS witness,w.own AS captured,(w.low+((w.own&3)=1)) AS low,(w.uncertain+((w.own&3)=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome5 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND (((w.own&3)=1 OR (w.own&3)=2 OR w.open) OR (CASE WHEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END)<=w.bound THEN (CASE WHEN w.stopped=0 THEN n.id+1 ELSE n."last"+1 END) END) IS NULL)
),
outcome7 AS MATERIALIZED (
SELECT d.k,CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END AS truth,CASE WHEN (t.captured&4)<>0 THEN t.witness ELSE d.c0 END AS c0 FROM outcome4 d LEFT JOIN outcome6 t ON t.k=d.k
),
outcome8 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR o.truth=0 THEN 0 WHEN d.truth=2 OR o.truth=2 THEN 2 ELSE 1 END AS truth,CASE WHEN o.truth=1 THEN o.c0 ELSE d.c0 END AS c0,(d.u0 OR o.truth=2 OR (o.k IS NULL AND d.truth<>0)) AS u0 FROM outcome3 d LEFT JOIN outcome7 o ON o.k=d.k
),
outcome9 AS MATERIALIZED (
SELECT s.k,s.truth,s.c0 AS c0 FROM outcome8 s
)
-- final matches
SELECT 0,d.file,d.node,json_array(o.c0) FROM outcome0 d JOIN outcome9 o ON o.k=d.k WHERE o.truth=1 ORDER BY d.file,d.node
