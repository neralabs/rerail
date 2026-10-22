WITH RECURSIVE
needed14(file,node) AS MATERIALIZED (SELECT file,id FROM node WHERE kind IN (197) UNION SELECT p.file,p.id FROM needed14 k CROSS JOIN node n ON n.file=k.file AND n.id=k.node CROSS JOIN node p ON p.file=n.file AND p.id=n.parent WHERE p.kind IN (226)),
bounds7(file,node,last) AS MATERIALIZED (SELECT k.file,k.node,n."last" FROM r7 k CROSS JOIN node n ON n.file=k.file AND n.id=k.node),
regions7(file,node,last) AS MATERIALIZED (SELECT b.file,b.node,b.last FROM bounds7 b WHERE NOT EXISTS (SELECT 1 FROM bounds7 outer_region WHERE outer_region.file=b.file AND outer_region.node<b.node AND outer_region.last>=b.node)),
-- frontier outputs
r0(file,node) AS (
SELECT file,node FROM r7
UNION
SELECT file,node FROM r9
UNION
SELECT file,node FROM r6
UNION
SELECT file,node FROM r16
UNION
SELECT file,node FROM r16
UNION
SELECT file,node FROM r16
UNION
SELECT file,node FROM r16
UNION
SELECT file,node FROM r4
UNION
SELECT file,node FROM r10
UNION
SELECT file,node FROM r16
),
-- utility[0]
r1(file,node) AS MATERIALIZED (
SELECT a0n0.file,a0n0.id FROM node AS a0n0 WHERE a0n0.kind IN (145)
UNION
SELECT a1n0.file,a1n0.id FROM node AS a1n0 WHERE a1n0.kind IN (197) AND NOT EXISTS (SELECT 1 WHERE EXISTS (SELECT 1 FROM r14 k WHERE k.file=a1n0.file AND k.node=a1n0.id))
),
-- origin candidates
r3(file,node) AS MATERIALIZED (
SELECT file,node FROM r1
),
-- utility[3]
r4(file,node) AS MATERIALIZED (
SELECT DISTINCT a3n0.file,a3n0.id FROM node AS a3n0 WHERE a3n0.kind IN (137,138,221,222,223,224)
),
-- origin search stop
r5(file,node) AS MATERIALIZED (
SELECT a4n0.file,a4n0.id FROM node AS a4n0 WHERE a4n0.kind IN (145)
UNION
SELECT a5n0.file,a5n0.id FROM node AS a5n0 WHERE a5n0.kind IN (197) AND NOT EXISTS (SELECT 1 WHERE EXISTS (SELECT 1 FROM r14 k WHERE k.file=a5n0.file AND k.node=a5n0.id))
),
-- binding search stop
r6(file,node) AS MATERIALIZED (
SELECT DISTINCT a7n0.file,a7n0.id FROM node AS a7n0 WHERE a7n0.kind IN (145,197)
),
walk7(file,origin,node,distance) AS MATERIALIZED (SELECT n.file,k.node,n.parent,1 FROM r3 k CROSS JOIN node n ON n.file=k.file AND n.id=k.node WHERE n.parent IS NOT NULL UNION ALL SELECT w.file,w.origin,n.parent,w.distance+1 FROM walk7 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE n.parent IS NOT NULL AND NOT EXISTS (SELECT 1 FROM r5 s WHERE s.file=w.file AND s.node=w.node)),
pairs7(file,origin,node,distance) AS MATERIALIZED (SELECT w.* FROM walk7 w JOIN r4 f ON f.file=w.file AND f.node=w.node),
r7(file,node) AS MATERIALIZED (SELECT DISTINCT file,node FROM pairs7),
-- demanded binding candidates
r9(file,node) AS MATERIALIZED (
SELECT DISTINCT a8n0.file,a8n0.id FROM r15 AS seed8 CROSS JOIN node AS a8n0 ON a8n0.file=seed8.file AND a8n0.id=seed8.node WHERE a8n0.kind IN (1)
),
-- ancestor predicate
r10(file,node) AS MATERIALIZED (
SELECT DISTINCT a9n0.file,a9n0.id FROM node AS a9n0 WHERE a9n0.kind IN (145,197)
),
-- utility[7] for consumer kinds
r14(file,node) AS MATERIALIZED (
SELECT a10n0.file,a10n0.id FROM needed14 AS keys10 CROSS JOIN node AS a10n0 ON a10n0.file=keys10.file AND a10n0.id=keys10.node CROSS JOIN node AS a10n1 ON a10n1.file=a10n0.file AND a10n1.id=a10n0.parent WHERE a10n1.kind IN (125)
UNION
SELECT a11n0.file,a11n0.id FROM needed14 AS keys11 CROSS JOIN node AS a11n0 ON a11n0.file=keys11.file AND a11n0.id=keys11.node CROSS JOIN node AS a11n1 ON a11n1.file=a11n0.file AND a11n1.id=a11n0.parent CROSS JOIN node AS a11n2 ON a11n2.file=a11n0.file AND a11n2.id=a11n0.next WHERE a11n1.kind IN (207) AND a11n2.kind IN (158,224)
UNION
SELECT a12n0.file,a12n0.id FROM needed14 AS keys12 CROSS JOIN node AS a12n0 ON a12n0.file=keys12.file AND a12n0.id=keys12.node CROSS JOIN node AS a12n1 ON a12n1.file=a12n0.file AND a12n1.id=a12n0.parent CROSS JOIN node AS a12n2 ON a12n2.file=a12n1.file AND a12n2.id=a12n1.prev WHERE a12n1.kind IN (158) AND a12n2.kind IN (1) AND node_regex(0,a12n2.file,a12n2.id)
UNION
SELECT a13n0.file,a13n0.id FROM needed14 AS keys13 CROSS JOIN node AS a13n0 ON a13n0.file=keys13.file AND a13n0.id=keys13.node CROSS JOIN node AS a13n1 ON a13n1.file=a13n0.file AND a13n1.id=a13n0.parent CROSS JOIN node AS a13n2 ON a13n2.file=a13n1.file AND a13n2.id=a13n1.prev WHERE a13n1.kind IN (158) AND relation_pattern(1,a13n2.file,a13n2.id) AND a13n2.kind IN (204)
UNION
SELECT a14n0.file,a14n0.id FROM needed14 AS keys14 CROSS JOIN node AS a14n0 ON a14n0.file=keys14.file AND a14n0.id=keys14.node CROSS JOIN node AS a14n1 ON a14n1.file=a14n0.file AND a14n1.id=a14n0.parent CROSS JOIN node AS a14n2 ON a14n2.file=a14n1.file AND a14n2.parent=a14n1.id CROSS JOIN node AS a14n3 ON a14n3.file=a14n2.file AND a14n3.id=a14n2.next CROSS JOIN node AS a14n4 ON a14n4.file=a14n0.file AND a14n4.id=a14n0.prev WHERE a14n1.kind IN (215) AND a14n2.kind IN (1) AND node_regex(2,a14n2.file,a14n2.id) AND node_regex(3,a14n3.file,a14n3.id) AND node_regex(3,a14n4.file,a14n4.id)
UNION
SELECT a15n0.file,a15n0.id FROM r14 AS seed15 CROSS JOIN node AS a15n1 ON a15n1.file=seed15.file AND a15n1.id=seed15.node CROSS JOIN node AS a15n0 ON a15n0.file=a15n1.file AND a15n0.parent=a15n1.id WHERE a15n1.kind IN (226) AND EXISTS (SELECT 1 FROM needed14 k WHERE k.file=a15n0.file AND k.node=a15n0.id)
),
-- utility[9] within r7
r15(file,node) AS MATERIALIZED (
SELECT a16n0.file,a16n0.id FROM regions7 AS domain16 CROSS JOIN node AS a16n1 ON a16n1.file=domain16.file AND a16n1.id>=domain16.node AND a16n1.id<=domain16.last CROSS JOIN node AS a16n0 ON a16n0.file=a16n1.file AND a16n0.parent=a16n1.id WHERE NOT EXISTS (SELECT 1 FROM (SELECT 1) AS unit17 CROSS JOIN node AS a17n1 ON a17n1.file=a16n0.file AND a17n1.id=a16n0.prev WHERE node_regex(4,a17n1.file,a17n1.id)) AND a16n1.kind IN (199,200)
UNION
SELECT a18n0.file,a18n0.id FROM regions7 AS domain18 CROSS JOIN node AS a18n1 ON a18n1.file=domain18.file AND a18n1.id>=domain18.node AND a18n1.id<=domain18.last CROSS JOIN node AS a18n0 ON a18n0.file=a18n1.file AND a18n0.parent=a18n1.id CROSS JOIN node AS a18n2 ON a18n2.file=a18n0.file AND a18n2.id=a18n0.prev WHERE a18n1.kind IN (137,228) AND node_regex(5,a18n2.file,a18n2.id)
UNION
SELECT a19n0.file,a19n0.id FROM r15 AS seed19 CROSS JOIN node AS a19n1 ON a19n1.file=seed19.file AND a19n1.id=seed19.node CROSS JOIN node AS a19n0 ON a19n0.file=a19n1.file AND a19n0.parent=a19n1.id WHERE a19n1.kind IN (180,181,184,201,216,218,226)
),
-- utility[8] for bound candidates
r16(file,node) AS MATERIALIZED (
SELECT a20n0.file,a20n0.id FROM r15 AS seed20 CROSS JOIN node AS a20n0 ON a20n0.file=seed20.file AND a20n0.id=seed20.node
UNION
SELECT a21n0.file,a21n0.id FROM regions7 AS domain21 CROSS JOIN node AS a21n1 ON a21n1.file=domain21.file AND a21n1.id>=domain21.node AND a21n1.id<=domain21.last CROSS JOIN node AS a21n0 ON a21n0.file=a21n1.file AND a21n0.parent=a21n1.id WHERE NOT EXISTS (SELECT 1 FROM (SELECT 1) AS unit22 CROSS JOIN node AS a22n1 ON a22n1.file=a21n0.file AND a22n1.id=a21n0.prev WHERE node_regex(4,a22n1.file,a22n1.id)) AND a21n1.kind IN (123)
UNION
SELECT a23n0.file,a23n0.id FROM regions7 AS domain23 CROSS JOIN node AS a23n1 ON a23n1.file=domain23.file AND a23n1.id>=domain23.node AND a23n1.id<=domain23.last CROSS JOIN node AS a23n0 ON a23n0.file=a23n1.file AND a23n0.parent=a23n1.id CROSS JOIN node AS a23n2 ON a23n2.file=a23n1.file AND a23n2.id=a23n1.parent WHERE a23n0.kind IN (273) AND a23n1.kind IN (186) AND a23n2.kind IN (144)
UNION
SELECT a24n0.file,a24n0.id FROM r16 AS seed24 CROSS JOIN node AS a24n1 ON a24n1.file=seed24.file AND a24n1.id=seed24.node CROSS JOIN node AS a24n0 ON a24n0.file=a24n1.file AND a24n0.parent=a24n1.id WHERE a24n1.kind IN (184,216,218,226,273)
),
binding_seeds(file,node,atom) AS MATERIALIZED (SELECT n.file,n.id,relation_leaf_atom(n.file,n.id,n.atom) FROM r9 t CROSS JOIN node n ON n.file=t.file AND n.id=t.node WHERE 1 AND n.named),
binding_walk(file,atom,witness,node) AS (SELECT n.file,t.atom,t.node,n.parent FROM binding_seeds t CROSS JOIN node n ON n.file=t.file AND n.id=t.node WHERE n.parent IS NOT NULL UNION ALL SELECT w.file,w.atom,w.witness,n.parent FROM binding_walk w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE n.parent IS NOT NULL AND NOT EXISTS (SELECT 1 FROM r6 s WHERE s.file=w.file AND s.node=w.node)),
binding_choices(file,node,atom,witness) AS MATERIALIZED (SELECT w.file,w.node,w.atom,MIN(w.witness) FROM binding_walk w JOIN r7 k ON k.file=w.file AND k.node=w.node GROUP BY w.file,w.node,w.atom),
origin_choices AS (SELECT p.file,p.origin,b.atom,b.witness,p.node AS ancestor,p.distance,ROW_NUMBER() OVER (PARTITION BY p.file,p.origin,b.atom ORDER BY p.distance) AS choice FROM pairs7 p JOIN binding_choices b ON b.file=p.file AND b.node=p.node),
capture_demand AS MATERIALIZED (SELECT file,origin,atom,witness,ancestor,distance FROM origin_choices WHERE choice=1),
answer0(file,node,atom,witness) AS (SELECT NULL,NULL,NULL,NULL WHERE 0),
answer1(file,node,atom,witness) AS MATERIALIZED (SELECT file,origin,atom,witness FROM capture_demand),
answer3_walk(file,origin,node,state) AS MATERIALIZED (SELECT DISTINCT file,node,node,0 FROM answer1 UNION SELECT w.file,w.origin,n2.id,1 FROM answer3_walk w CROSS JOIN node n0 ON n0.file=w.file AND n0.id=w.node CROSS JOIN node n1 ON n1.file=n0.file AND n1.parent=n0.id CROSS JOIN node n2 ON n2.file=n1.file AND n2.parent=n1.id WHERE w.state=0 AND 1 AND (1 AND (n1.kind IN (146,147))) AND 1 UNION SELECT w.file,w.origin,n1.id,1 FROM answer3_walk w CROSS JOIN node n0 ON n0.file=w.file AND n0.id=w.node CROSS JOIN node n1 ON n1.file=n0.file AND n1.parent=n0.id WHERE w.state=1 AND (1 AND (n0.kind IN (182,183,208))) AND (1 AND (NOT (NOT EXISTS (SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM (SELECT 1) AS unit1 CROSS JOIN node AS a1n1 ON a1n1.file=n1.file AND a1n1.id=n1.prev WHERE node_regex(4,a1n1.file,a1n1.id))))))),
answer3(file,origin,atom) AS MATERIALIZED (SELECT w.file,w.origin,relation_leaf_atom(n1.file,n1.id,n1.atom) AS atom FROM answer3_walk w CROSS JOIN node n0 ON n0.file=w.file AND n0.id=w.node CROSS JOIN node n1 ON n1.file=n0.file AND n1.id>n0.id AND n1.id<=n0."last" WHERE w.state=0 AND 1 AND (1 AND (n1.kind IN (1) AND NOT (NOT EXISTS (SELECT 1 WHERE EXISTS (SELECT 1 FROM r16 k WHERE k.file=n1.file AND k.node=n1.id))))) AND n1.named UNION SELECT w.file,w.origin,relation_leaf_atom(n0.file,n0.id,n0.atom) AS atom FROM answer3_walk w CROSS JOIN node n0 ON n0.file=w.file AND n0.id=w.node WHERE w.state=1 AND (1 AND (n0.kind IN (1))) AND n0.named UNION SELECT w.file,w.origin,relation_leaf_atom(n1.file,n1.id,n1.atom) AS atom FROM answer3_walk w CROSS JOIN node n0 ON n0.file=w.file AND n0.id=w.node CROSS JOIN node n1 ON n1.file=n0.file AND n1.parent=n0.id WHERE w.state=1 AND (1 AND (n0.kind IN (184,185))) AND (1 AND (n1.kind IN (1))) AND n1.named),
answer2(file,node,atom,witness) AS MATERIALIZED (SELECT a.file,a.node,a.atom,a.witness FROM answer1 a CROSS JOIN node n ON n.file=a.file AND n.id=a.node WHERE 1 AND n.kind IN (197) AND NOT EXISTS (SELECT 1 FROM answer3 p WHERE p.file=a.file AND p.origin=a.node AND p.atom=a.atom)),
answer4(file,node,atom,witness) AS MATERIALIZED (SELECT file,node,atom,witness FROM (SELECT a.file,n.id AS node,a.atom,a.witness,ROW_NUMBER() OVER (PARTITION BY a.file,n.id,a.atom ORDER BY a.node DESC) AS choice FROM answer2 a CROSS JOIN node b ON b.file=a.file AND b.id=a.node CROSS JOIN node v ON v.file=a.file AND v.id=a.atom CROSS JOIN node n ON n.file=b.file AND n.parent=b.id AND n.thash=v.thash WHERE NOT (NOT EXISTS (SELECT 1 FROM (SELECT 1) AS unit0 CROSS JOIN node AS a0n1 ON a0n1.file=n.file AND a0n1.id=n.prev WHERE node_regex(6,a0n1.file,a0n1.id))) AND NOT (NOT EXISTS (SELECT 1 WHERE EXISTS (SELECT 1 FROM r4 k CROSS JOIN node ancestor ON ancestor.file=k.file AND ancestor.id=k.node WHERE k.file=n.file AND ancestor.id<n.id AND ancestor."last">=n.id))) AND NOT (NOT EXISTS (SELECT 1 WHERE EXISTS (SELECT 1 FROM r10 k CROSS JOIN node ancestor ON ancestor.file=k.file AND ancestor.id=k.node WHERE k.file=n.file AND ancestor.id<n.id AND ancestor."last">=n.id))) AND n.kind IN (1) AND (n.kind IN (1) AND NOT (NOT EXISTS (SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM (SELECT 1) AS unit1 CROSS JOIN node AS a1n1 ON a1n1.file=n.file AND a1n1.id=n.parent CROSS JOIN node AS a1n2 ON a1n2.file=n.file AND a1n2.id=n.prev WHERE a1n1.kind IN (204) AND node_regex(7,a1n2.file,a1n2.id) UNION SELECT 1 FROM (SELECT 1) AS unit2 CROSS JOIN node AS a2n1 ON a2n1.file=n.file AND a2n1.id=n.parent CROSS JOIN node AS a2n2 ON a2n2.file=n.file AND a2n2.id=n.next WHERE a2n1.kind IN (215) AND node_regex(3,a2n2.file,a2n2.id) UNION SELECT 1 FROM (SELECT 1) AS unit3 CROSS JOIN node AS a3n1 ON a3n1.file=n.file AND a3n1.id=n.parent WHERE a3n1.kind IN (145) UNION SELECT 1 FROM (SELECT 1) AS unit4 CROSS JOIN node AS a4n1 ON a4n1.file=n.file AND a4n1.id=n.parent WHERE a4n1.kind IN (154) UNION SELECT 1 FROM (SELECT 1) AS unit5 CROSS JOIN node AS a5n1 ON a5n1.file=n.file AND a5n1.id=n.parent WHERE a5n1.kind IN (146) UNION SELECT 1 FROM (SELECT 1) AS unit6 CROSS JOIN node AS a6n1 ON a6n1.file=n.file AND a6n1.id=n.parent WHERE a6n1.kind IN (147) UNION SELECT 1 FROM (SELECT 1) AS unit7 CROSS JOIN node AS a7n1 ON a7n1.file=n.file AND a7n1.id=n.parent CROSS JOIN node AS a7n2 ON a7n2.file=a7n1.file AND a7n2.id=a7n1.parent WHERE a7n1.kind IN (184) AND a7n2.kind IN (146,147,208) UNION SELECT 1 FROM (SELECT 1) AS unit8 CROSS JOIN node AS a8n1 ON a8n1.file=n.file AND a8n1.id=n.parent WHERE a8n1.kind IN (185) UNION SELECT 1 FROM (SELECT 1) AS unit9 CROSS JOIN node AS a9n1 ON a9n1.file=n.file AND a9n1.id=n.parent WHERE a9n1.kind IN (163) UNION SELECT 1 FROM (SELECT 1) AS unit10 CROSS JOIN node AS a10n1 ON a10n1.file=n.file AND a10n1.id=n.parent WHERE a10n1.kind IN (117) UNION SELECT 1 FROM (SELECT 1) AS unit11 CROSS JOIN node AS a11n1 ON a11n1.file=n.file AND a11n1.id=n.parent WHERE a11n1.kind IN (150) UNION SELECT 1 FROM (SELECT 1) AS unit12 CROSS JOIN node AS a12n1 ON a12n1.file=n.file AND a12n1.id=n.parent WHERE a12n1.kind IN (151) UNION SELECT 1 FROM (SELECT 1) AS unit13 CROSS JOIN node AS a13n1 ON a13n1.file=n.file AND a13n1.id=n.parent WHERE a13n1.kind IN (172) UNION SELECT 1 FROM (SELECT 1) AS unit14 CROSS JOIN node AS a14n1 ON a14n1.file=n.file AND a14n1.id=n.parent WHERE a14n1.kind IN (173) UNION SELECT 1 FROM (SELECT 1) AS unit15 CROSS JOIN node AS a15n1 ON a15n1.file=n.file AND a15n1.id=n.parent WHERE a15n1.kind IN (155) UNION SELECT 1 FROM (SELECT 1) AS unit16 CROSS JOIN node AS a16n1 ON a16n1.file=n.file AND a16n1.id=n.parent CROSS JOIN node AS a16n2 ON a16n2.file=a16n1.file AND a16n2.id=a16n1.parent CROSS JOIN node AS a16n3 ON a16n3.file=a16n2.file AND a16n3.id=a16n2.parent WHERE a16n1.kind IN (273) AND a16n2.kind IN (186) AND a16n3.kind IN (140) UNION SELECT 1 FROM (SELECT 1) AS unit17 CROSS JOIN node AS a17n1 ON a17n1.file=n.file AND a17n1.id=n.parent CROSS JOIN node AS a17n2 ON a17n2.file=a17n1.file AND a17n2.id=a17n1.parent WHERE a17n1.kind IN (186) AND a17n2.kind IN (164) UNION SELECT 1 FROM (SELECT 1) AS unit18 CROSS JOIN node AS a18n1 ON a18n1.file=n.file AND a18n1.id=n.parent WHERE NOT EXISTS (SELECT 1 FROM (SELECT 1) AS unit19 CROSS JOIN node AS a19n1 ON a19n1.file=n.file AND a19n1.id=n.prev WHERE node_regex(4,a19n1.file,a19n1.id)) AND a18n1.kind IN (182,183,208) UNION SELECT 1 WHERE EXISTS (SELECT 1 FROM r16 k WHERE k.file=n.file AND k.node=n.id) UNION SELECT 1 FROM (SELECT 1) AS unit21 CROSS JOIN node AS a21n1 ON a21n1.file=n.file AND a21n1.id=n.parent WHERE a21n1.kind IN (126))))) AND n.named AND relation_leaf_atom(n.file,n.id,n.atom)=a.atom AND 1) WHERE choice=1),
answer5(file,node,atom,witness) AS MATERIALIZED (SELECT file,node,atom,witness FROM (SELECT *,ROW_NUMBER() OVER (PARTITION BY file,node,atom ORDER BY branch) AS choice FROM (SELECT file,node,atom,witness,0 AS branch FROM answer0 UNION ALL SELECT file,node,atom,witness,1 AS branch FROM answer4)) WHERE choice=1),
answer6(file,node,atom,witness) AS MATERIALIZED (SELECT a.file,a.node,a.atom,a.witness FROM answer1 a CROSS JOIN node n ON n.file=a.file AND n.id=a.node WHERE 1 AND n.kind IN (145) AND NOT EXISTS (SELECT 1 FROM answer3 p WHERE p.file=a.file AND p.origin=a.node AND p.atom=a.atom)),
answer7(file,node,atom,witness) AS MATERIALIZED (SELECT file,node,atom,witness FROM (SELECT a.file,n.id AS node,a.atom,a.witness,ROW_NUMBER() OVER (PARTITION BY a.file,n.id,a.atom ORDER BY a.node DESC) AS choice FROM answer6 a CROSS JOIN node b ON b.file=a.file AND b.id=a.node CROSS JOIN node n ON n.file=b.file AND n.parent=b.id WHERE n.kind IN (161) AND 1) WHERE choice=1),
answer8(file,node,atom,witness) AS MATERIALIZED (SELECT file,node,atom,witness FROM (SELECT a.file,n.id AS node,a.atom,a.witness,ROW_NUMBER() OVER (PARTITION BY a.file,n.id,a.atom ORDER BY a.node DESC) AS choice FROM answer2 a CROSS JOIN node b ON b.file=a.file AND b.id=a.node CROSS JOIN node n ON n.file=b.file AND n.parent=b.id WHERE NOT (NOT EXISTS (SELECT 1 FROM (SELECT 1) AS unit0 CROSS JOIN node AS a0n1 ON a0n1.file=n.file AND a0n1.id=n.prev WHERE node_regex(6,a0n1.file,a0n1.id))) AND 1) WHERE choice=1),
answer9(file,node,atom,witness) AS MATERIALIZED (SELECT file,node,atom,witness FROM (SELECT *,ROW_NUMBER() OVER (PARTITION BY file,node,atom ORDER BY branch) AS choice FROM (SELECT file,node,atom,witness,0 AS branch FROM answer7 UNION ALL SELECT file,node,atom,witness,1 AS branch FROM answer8)) WHERE choice=1),
answer10(file,node,atom,witness) AS MATERIALIZED (SELECT file,node,atom,witness FROM (SELECT a.file,n.id AS node,a.atom,a.witness,ROW_NUMBER() OVER (PARTITION BY a.file,n.id,a.atom ORDER BY a.node DESC) AS choice FROM answer9 a CROSS JOIN node b ON b.file=a.file AND b.id=a.node CROSS JOIN node v ON v.file=a.file AND v.id=a.atom CROSS JOIN node n ON n.file=b.file AND n.id>b.id AND n.id<=b."last" AND n.thash=v.thash WHERE NOT (NOT EXISTS (SELECT 1 WHERE EXISTS (SELECT 1 FROM r4 k CROSS JOIN node ancestor ON ancestor.file=k.file AND ancestor.id=k.node WHERE k.file=n.file AND ancestor.id<n.id AND ancestor."last">=n.id))) AND NOT (NOT EXISTS (SELECT 1 WHERE EXISTS (SELECT 1 FROM r10 k CROSS JOIN node ancestor ON ancestor.file=k.file AND ancestor.id=k.node WHERE k.file=n.file AND ancestor.id<n.id AND ancestor."last">=n.id))) AND n.kind IN (1) AND (n.kind IN (1) AND NOT (NOT EXISTS (SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM (SELECT 1) AS unit1 CROSS JOIN node AS a1n1 ON a1n1.file=n.file AND a1n1.id=n.parent CROSS JOIN node AS a1n2 ON a1n2.file=n.file AND a1n2.id=n.prev WHERE a1n1.kind IN (204) AND node_regex(7,a1n2.file,a1n2.id) UNION SELECT 1 FROM (SELECT 1) AS unit2 CROSS JOIN node AS a2n1 ON a2n1.file=n.file AND a2n1.id=n.parent CROSS JOIN node AS a2n2 ON a2n2.file=n.file AND a2n2.id=n.next WHERE a2n1.kind IN (215) AND node_regex(3,a2n2.file,a2n2.id) UNION SELECT 1 FROM (SELECT 1) AS unit3 CROSS JOIN node AS a3n1 ON a3n1.file=n.file AND a3n1.id=n.parent WHERE a3n1.kind IN (145) UNION SELECT 1 FROM (SELECT 1) AS unit4 CROSS JOIN node AS a4n1 ON a4n1.file=n.file AND a4n1.id=n.parent WHERE a4n1.kind IN (154) UNION SELECT 1 FROM (SELECT 1) AS unit5 CROSS JOIN node AS a5n1 ON a5n1.file=n.file AND a5n1.id=n.parent WHERE a5n1.kind IN (146) UNION SELECT 1 FROM (SELECT 1) AS unit6 CROSS JOIN node AS a6n1 ON a6n1.file=n.file AND a6n1.id=n.parent WHERE a6n1.kind IN (147) UNION SELECT 1 FROM (SELECT 1) AS unit7 CROSS JOIN node AS a7n1 ON a7n1.file=n.file AND a7n1.id=n.parent CROSS JOIN node AS a7n2 ON a7n2.file=a7n1.file AND a7n2.id=a7n1.parent WHERE a7n1.kind IN (184) AND a7n2.kind IN (146,147,208) UNION SELECT 1 FROM (SELECT 1) AS unit8 CROSS JOIN node AS a8n1 ON a8n1.file=n.file AND a8n1.id=n.parent WHERE a8n1.kind IN (185) UNION SELECT 1 FROM (SELECT 1) AS unit9 CROSS JOIN node AS a9n1 ON a9n1.file=n.file AND a9n1.id=n.parent WHERE a9n1.kind IN (163) UNION SELECT 1 FROM (SELECT 1) AS unit10 CROSS JOIN node AS a10n1 ON a10n1.file=n.file AND a10n1.id=n.parent WHERE a10n1.kind IN (117) UNION SELECT 1 FROM (SELECT 1) AS unit11 CROSS JOIN node AS a11n1 ON a11n1.file=n.file AND a11n1.id=n.parent WHERE a11n1.kind IN (150) UNION SELECT 1 FROM (SELECT 1) AS unit12 CROSS JOIN node AS a12n1 ON a12n1.file=n.file AND a12n1.id=n.parent WHERE a12n1.kind IN (151) UNION SELECT 1 FROM (SELECT 1) AS unit13 CROSS JOIN node AS a13n1 ON a13n1.file=n.file AND a13n1.id=n.parent WHERE a13n1.kind IN (172) UNION SELECT 1 FROM (SELECT 1) AS unit14 CROSS JOIN node AS a14n1 ON a14n1.file=n.file AND a14n1.id=n.parent WHERE a14n1.kind IN (173) UNION SELECT 1 FROM (SELECT 1) AS unit15 CROSS JOIN node AS a15n1 ON a15n1.file=n.file AND a15n1.id=n.parent WHERE a15n1.kind IN (155) UNION SELECT 1 FROM (SELECT 1) AS unit16 CROSS JOIN node AS a16n1 ON a16n1.file=n.file AND a16n1.id=n.parent CROSS JOIN node AS a16n2 ON a16n2.file=a16n1.file AND a16n2.id=a16n1.parent CROSS JOIN node AS a16n3 ON a16n3.file=a16n2.file AND a16n3.id=a16n2.parent WHERE a16n1.kind IN (273) AND a16n2.kind IN (186) AND a16n3.kind IN (140) UNION SELECT 1 FROM (SELECT 1) AS unit17 CROSS JOIN node AS a17n1 ON a17n1.file=n.file AND a17n1.id=n.parent CROSS JOIN node AS a17n2 ON a17n2.file=a17n1.file AND a17n2.id=a17n1.parent WHERE a17n1.kind IN (186) AND a17n2.kind IN (164) UNION SELECT 1 FROM (SELECT 1) AS unit18 CROSS JOIN node AS a18n1 ON a18n1.file=n.file AND a18n1.id=n.parent WHERE NOT EXISTS (SELECT 1 FROM (SELECT 1) AS unit19 CROSS JOIN node AS a19n1 ON a19n1.file=n.file AND a19n1.id=n.prev WHERE node_regex(4,a19n1.file,a19n1.id)) AND a18n1.kind IN (182,183,208) UNION SELECT 1 WHERE EXISTS (SELECT 1 FROM r16 k WHERE k.file=n.file AND k.node=n.id) UNION SELECT 1 FROM (SELECT 1) AS unit21 CROSS JOIN node AS a21n1 ON a21n1.file=n.file AND a21n1.id=n.parent WHERE a21n1.kind IN (126))))) AND n.named AND relation_leaf_atom(n.file,n.id,n.atom)=a.atom AND 1) WHERE choice=1),
answer11(file,node,atom,witness) AS MATERIALIZED (SELECT file,node,atom,witness FROM (SELECT *,ROW_NUMBER() OVER (PARTITION BY file,node,atom ORDER BY branch) AS choice FROM (SELECT file,node,atom,witness,0 AS branch FROM answer5 UNION ALL SELECT file,node,atom,witness,1 AS branch FROM answer10)) WHERE choice=1)
SELECT 0,file,node,json_array(witness) FROM answer11 ORDER BY file,node
