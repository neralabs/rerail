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
SELECT d.k,CASE WHEN EXISTS (SELECT 1 FROM node n WHERE n.file=d.file AND n.parent=origin.id AND n.kind IN (1) AND ((NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (1)))) AND (NOT node_regex(0,n.file,n.id)))) THEN 1 WHEN EXISTS (SELECT 1 FROM node n WHERE n.file=d.file AND n.parent=origin.id AND n.kind IN (1) AND (NOT (((NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (1)))) AND (NOT node_regex(0,n.file,n.id))) OR ((NOT (NOT (NOT EXISTS (SELECT 1 WHERE n.kind IN (1))))))))) THEN 2 ELSE 0 END AS truth FROM outcome4 d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node
),
outcome6 AS MATERIALIZED (
SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR o.truth=0 THEN 0 WHEN d.truth=2 OR o.truth=2 THEN 2 ELSE 1 END AS truth FROM outcome3 d LEFT JOIN outcome5 o ON o.k=d.k
),
outcome7 AS MATERIALIZED (
SELECT s.k,s.truth FROM outcome6 s
)
-- final matches
SELECT 0,d.file,d.node,json_array() FROM outcome0 d JOIN outcome7 o ON o.k=d.k WHERE o.truth=1 ORDER BY d.file,d.node
