WITH RECURSIVE
outcome0 AS MATERIALIZED (
SELECT relation_memo_reset(0) AS ready
),
outcome1 AS MATERIALIZED (
SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,10000 AS work,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n CROSS JOIN outcome0 init ON init.ready=1 WHERE 1 AND n.kind IN (256)
),
outcome2 AS (
SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,10000 AS work FROM outcome1 GROUP BY file
UNION ALL
SELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1 FROM outcome2 w CROSS JOIN outcome1 d ON d.file=w.file AND d.ordinal=w.ordinal+1 CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
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
SELECT d.k,d.file,d.node,d.work,1 AS truth FROM outcome0 d
),
outcome2 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(0,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome2 d
),
outcome4 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome3 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(1,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome5 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(2,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome7 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome6 d
),
outcome8 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome7 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(3,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome9 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome8 d
),
outcome10 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome9 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(4,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome11 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome10 d
),
outcome12 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome11 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(5,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome13 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome12 d
),
outcome14 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome13 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(6,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome15 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome14 d
),
outcome16 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome15 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(7,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome17 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome16 d
),
outcome18 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome17 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(8,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome19 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome18 d
),
outcome20 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome19 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(9,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome21 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome20 d
),
outcome22 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome21 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(10,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome23 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome22 d
),
outcome24 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome23 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(11,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome25 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome24 d
),
outcome26 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome25 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(12,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome27 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome26 d
),
outcome28 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome27 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(13,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome29 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome28 d
),
outcome30 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome29 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(14,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome31 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome30 d
),
outcome32 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome31 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(15,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome33 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome32 d
),
outcome34 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome33 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(16,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome35 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome34 d
),
outcome36 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome35 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(17,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome37 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome36 d
),
outcome38 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome37 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(18,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome39 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome38 d
),
outcome40 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome39 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(19,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome41 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome40 d
),
outcome42 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome41 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(20,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome43 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome42 d
),
outcome44 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome43 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(21,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome45 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome44 d
),
outcome46 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome45 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(22,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome47 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome46 d
),
outcome48 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome47 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(23,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome49 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome48 d
),
outcome50 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome49 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(24,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome51 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome50 d
),
outcome52 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome51 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(25,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome53 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome52 d
),
outcome54 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome53 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(26,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome55 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome54 d
),
outcome56 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome55 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(27,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome57 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome56 d
),
outcome58 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome57 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(28,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome59 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome58 d
),
outcome60 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome59 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(29,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome61 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome60 d
),
outcome62 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome61 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(30,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome63 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome62 d
),
outcome64 AS MATERIALIZED (
SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM outcome63 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,d.file AS file,d.node AS node,d.work AS work WHERE d.truth>0 AND NOT (0)
),
outcome1 AS MATERIALIZED (
SELECT d.k,d.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1) AS node,origin."last" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open,d.work FROM outcome0 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ((SELECT candidate.id FROM node candidate WHERE candidate.file=origin.file AND candidate.id>origin.id AND candidate.id<=origin."last" AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
UNION ALL
SELECT w.k,w.file,w.node,w.bound,1, COALESCE(r.c0,0),w.stopped,w.low,w.uncertain,w.open,COALESCE(r.c1,w.work) FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node CROSS JOIN outcome0 d ON d.k=w.k CROSS JOIN relation_row r ON (r.c0,r.c1) IS (WITH RECURSIVE outcome0 AS MATERIALIZED (
SELECT 1 AS k,w.file AS file,n.id AS node,w.work AS work WHERE w.stopped>=0
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
SELECT d.*,m.truth AS hit,m.cost AS cost FROM outcome0 d LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node
),
outcome2 AS MATERIALIZED (
SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining FROM outcome1 d CROSS JOIN relation_row r ON (r.c0,r.c1) IS (SELECT COALESCE(v.truth,0),COALESCE(v.work,d.work-1) FROM node probe LEFT JOIN relation_value_budget(31,d.file,d.node,CASE WHEN (d.hit IS NULL AND d.work>0) AND (1) THEN d.work-1 END) v ON 1 WHERE probe.file=d.file AND probe.id=d.node AND (d.hit IS NULL AND d.work>0))
),
outcome3 AS MATERIALIZED (
SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth FROM outcome2 d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS 0 AND m.file IS d.file AND m.node IS d.node AND m.vtruth IS d.tested AND m.vcost IS (d.work-d.remaining)
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.phase=0
UNION ALL
SELECT w.k,w.file,w.node,w.bound,2, w.own,0,w.low,w.uncertain,w.open,w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node  WHERE w.phase=1
UNION ALL
SELECT w.k,w.file,(SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1),w.bound,0,0,0,(w.low+(w.own=1)),(w.uncertain+(w.own=2)),(w.open OR w.stopped=2),w.work FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT (w.own<0 OR w.stopped<0 OR (w.own=1)) AND ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NOT NULL
),
outcome2 AS MATERIALIZED (
SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,(w.low+(w.own=1)) AS low,(w.uncertain+(w.own=2)) AS uncertain,(w.open OR w.stopped=2) AS open FROM outcome1 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND ((w.own<0 OR w.stopped<0 OR (w.own=1)) OR ((SELECT candidate.id FROM node candidate WHERE candidate.file=w.file AND candidate.id>w.node AND candidate.id<=w.bound AND (candidate.kind IN (110)) ORDER BY candidate.id LIMIT 1)) IS NULL)
),
outcome3 AS MATERIALIZED (
SELECT d.k,COALESCE(t.work,d.work) AS work,CASE WHEN t.failure<0 THEN t.failure ELSE CASE WHEN COALESCE(t.low,0)>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END END AS truth FROM outcome0 d LEFT JOIN outcome2 t ON t.k=d.k
) SELECT o.truth,o.work FROM outcome3 o)
),
outcome65 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome64 d
),
outcome66 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome65 s
) SELECT o.truth,o.work FROM outcome66 o)
),
outcome5 AS MATERIALIZED (
SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth FROM outcome4 d
),
outcome6 AS MATERIALIZED (
SELECT s.k,s.work,s.truth FROM outcome5 s
) SELECT o.truth,o.work FROM outcome6 o) WHERE w.truth>=0
),
outcome3 AS MATERIALIZED (
SELECT 0 AS clause,o.file,o.node,o.truth,o.work FROM outcome2 o WHERE o.truth=1 OR o.truth<0
),
outcome4 AS MATERIALIZED (
SELECT clause,file,node,truth,work FROM outcome3 WHERE truth<0
)
-- final matches
SELECT m.clause,m.file,m.node,CASE m.clause WHEN 0 THEN json_array() END AS payload,0 AS failure,m.work FROM outcome3 m WHERE m.truth=1 AND NOT EXISTS (SELECT 1 FROM outcome4 f WHERE f.clause=m.clause AND f.file=m.file)
UNION ALL
SELECT clause,file,node,'[]',truth,work FROM outcome4
ORDER BY 1,2,3
