WITH RECURSIVE
-- clause
r0(file,node) AS (
SELECT file,node FROM r1
),
component1(state,file,node) AS MATERIALIZED (
SELECT 1,a0n0.file,a0n0.id FROM node AS a0n0 WHERE a0n0.kind IN (122,199)
UNION
SELECT 1,a1n0.file,a1n0.id FROM component1 AS seed1 CROSS JOIN node AS a1n1 ON a1n1.file=seed1.file AND a1n1.id=seed1.node CROSS JOIN node AS a1n0 ON a1n0.file=a1n1.file AND a1n0.parent=a1n1.id WHERE seed1.state=2
UNION
SELECT 1,a2n0.file,a2n0.id FROM component1 AS seed2 CROSS JOIN node AS a2n1 ON a2n1.file=seed2.file AND a2n1.id=seed2.node CROSS JOIN node AS a2n0 ON a2n0.file=a2n1.file AND a2n0.parent=a2n1.id WHERE seed2.state=3
UNION
SELECT 2,a3n0.file,a3n0.id FROM component1 AS seed3 CROSS JOIN node AS a3n1 ON a3n1.file=seed3.file AND a3n1.id=seed3.node CROSS JOIN node AS a3n0 ON a3n0.file=a3n1.file AND a3n0.parent=a3n1.id WHERE seed3.state=1
UNION
SELECT 3,a4n0.file,a4n0.id FROM component1 AS seed4 CROSS JOIN node AS a4n1 ON a4n1.file=seed4.file AND a4n1.id=seed4.node CROSS JOIN node AS a4n0 ON a4n0.file=a4n1.file AND a4n0.parent=a4n1.id WHERE seed4.state=1
),
-- utility[2]
r1(file,node) AS (SELECT file,node FROM component1 WHERE state=1),
-- utility[0]
r2(file,node) AS (SELECT file,node FROM component1 WHERE state=2),
-- utility[1]
r3(file,node) AS (SELECT file,node FROM component1 WHERE state=3)
SELECT 0,file,node,json_array() FROM r0 ORDER BY file,node
