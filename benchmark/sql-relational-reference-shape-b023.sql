WITH RECURSIVE
needed11(file,node) AS MATERIALIZED (SELECT file,id FROM node WHERE kind IN (197) UNION SELECT p.file,p.id FROM needed11 k CROSS JOIN node n ON n.file=k.file AND n.id=k.node CROSS JOIN node p ON p.file=n.file AND p.id=n.parent WHERE p.kind IN (226)),
bounds5(file,node,last) AS MATERIALIZED (SELECT k.file,k.node,n."last" FROM r5 k CROSS JOIN node n ON n.file=k.file AND n.id=k.node),
regions5(file,node,last) AS MATERIALIZED (SELECT b.file,b.node,b.last FROM bounds5 b WHERE NOT EXISTS (SELECT 1 FROM bounds5 outer_region WHERE outer_region.file=b.file AND outer_region.node<b.node AND outer_region.last>=b.node)),
-- utility[0]
r1(file,node) AS MATERIALIZED (
SELECT a0n0.file,a0n0.id FROM node AS a0n0 WHERE a0n0.kind IN (145)
UNION
SELECT a1n0.file,a1n0.id FROM node AS a1n0 WHERE a1n0.kind IN (197) AND NOT (EXISTS (SELECT 1 FROM r11 k WHERE k.file=a1n0.file AND k.node=a1n0.id))
),
-- utility[3]
r3(file,node) AS MATERIALIZED (
SELECT a3n0.file,a3n0.id FROM node AS a3n0 WHERE a3n0.kind IN (137,138,221,222,223,224)
),
-- binding search stop
r4(file,node) AS MATERIALIZED (
SELECT a4n0.file,a4n0.id FROM node AS a4n0 WHERE a4n0.kind IN (145,197)
),
walk5(file,origin,node,distance) AS (SELECT n.file,k.node,n.parent,1 FROM r1 k CROSS JOIN node n ON n.file=k.file AND n.id=k.node WHERE n.parent IS NOT NULL UNION ALL SELECT w.file,w.origin,n.parent,w.distance+1 FROM walk5 w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE n.parent IS NOT NULL AND NOT EXISTS (SELECT 1 FROM r1 s WHERE s.file=w.file AND s.node=w.node)),
pairs5(file,origin,node,distance) AS MATERIALIZED (SELECT w.* FROM walk5 w JOIN r3 f ON f.file=w.file AND f.node=w.node),
r5(file,node) AS MATERIALIZED (SELECT DISTINCT file,node FROM pairs5),
-- demanded binding candidates
r7(file,node) AS MATERIALIZED (
SELECT a5n0.file,a5n0.id FROM r12 AS seed5 CROSS JOIN node AS a5n0 ON a5n0.file=seed5.file AND a5n0.id=seed5.node WHERE a5n0.kind IN (1)
),
-- utility[7] for consumer kinds
r11(file,node) AS MATERIALIZED (
SELECT DISTINCT seed.file,seed.id FROM needed11 k CROSS JOIN node seed ON seed.file=k.file AND seed.id=k.node LEFT JOIN node bound0 ON bound0.file=seed.file AND bound0.id=seed.parent LEFT JOIN node bound1 ON bound1.file=seed.file AND bound1.id=seed.next LEFT JOIN node bound2 ON bound2.file=seed.file AND bound2.id=seed.prev LEFT JOIN node bound3 ON bound3.file=bound0.file AND bound3.id=bound0.prev WHERE 1 AND ((COALESCE(bound0.kind IN (207),0) AND COALESCE(bound1.kind IN (158,224),0)) OR (CASE WHEN bound0.id IS NOT NULL THEN (bound0.kind IN (215) AND EXISTS (SELECT 1 FROM (SELECT 1) AS unit0 CROSS JOIN node AS a0n1 ON a0n1.file=bound0.file AND a0n1.parent=bound0.id CROSS JOIN node AS a0n2 ON a0n2.file=a0n1.file AND a0n2.id=a0n1.next WHERE a0n1.kind IN (1) AND node_regex(2,a0n1.file,a0n1.id) AND node_regex(3,a0n2.file,a0n2.id))) ELSE 0 END AND CASE WHEN bound2.id IS NOT NULL THEN node_regex(3,bound2.file,bound2.id) ELSE 0 END) OR CASE WHEN bound0.id IS NOT NULL THEN ((bound0.kind IN (158) AND CASE WHEN bound3.id IS NOT NULL THEN (bound3.kind IN (1) AND node_regex(0,bound3.file,bound3.id)) ELSE 0 END) OR (bound0.kind IN (158) AND CASE WHEN bound3.id IS NOT NULL THEN (relation_pattern(1,bound3.file,bound3.id) AND bound3.kind IN (204)) ELSE 0 END) OR bound0.kind IN (125)) ELSE 0 END)
UNION
SELECT a6n0.file,a6n0.id FROM r11 AS seed6 CROSS JOIN node AS a6n1 ON a6n1.file=seed6.file AND a6n1.id=seed6.node CROSS JOIN node AS a6n0 ON a6n0.file=a6n1.file AND a6n0.parent=a6n1.id WHERE a6n1.kind IN (226) AND EXISTS (SELECT 1 FROM needed11 k WHERE k.file=a6n0.file AND k.node=a6n0.id)
),
-- utility[9] within r5
r12(file,node) AS MATERIALIZED (
SELECT DISTINCT seed.file,seed.id FROM regions5 d CROSS JOIN node parent ON parent.file=d.file AND parent.id>=d.node AND parent.id<=d.last CROSS JOIN node seed ON seed.file=parent.file AND seed.parent=parent.id LEFT JOIN node bound1 ON bound1.file=seed.file AND bound1.id=seed.prev WHERE parent.kind IN (137,199,200,228) AND ((NOT (COALESCE(bound1."end">bound1.start,0)) AND parent.kind IN (199,200)) OR (parent.kind IN (137,228) AND CASE WHEN bound1.id IS NOT NULL THEN node_regex(5,bound1.file,bound1.id) ELSE 0 END))
UNION
SELECT a7n0.file,a7n0.id FROM r12 AS seed7 CROSS JOIN node AS a7n1 ON a7n1.file=seed7.file AND a7n1.id=seed7.node CROSS JOIN node AS a7n0 ON a7n0.file=a7n1.file AND a7n0.parent=a7n1.id WHERE a7n1.kind IN (180,181,184,201,216,218,226)
),
-- utility[8] for bound candidates
r13(file,node) AS MATERIALIZED (
SELECT DISTINCT seed.file,seed.id FROM regions5 d CROSS JOIN node parent ON parent.file=d.file AND parent.id>=d.node AND parent.id<=d.last CROSS JOIN node seed ON seed.file=parent.file AND seed.parent=parent.id LEFT JOIN node bound1 ON bound1.file=seed.file AND bound1.id=seed.prev LEFT JOIN node bound2 ON bound2.file=parent.file AND bound2.id=parent.parent WHERE parent.kind IN (123,186) AND ((NOT (COALESCE(bound1."end">bound1.start,0)) AND parent.kind IN (123)) OR (seed.kind IN (273) AND (parent.kind IN (186) AND COALESCE(bound2.kind IN (144),0))))
UNION
SELECT file,node FROM r12
UNION
SELECT a8n0.file,a8n0.id FROM r13 AS seed8 CROSS JOIN node AS a8n1 ON a8n1.file=seed8.file AND a8n1.id=seed8.node CROSS JOIN node AS a8n0 ON a8n0.file=a8n1.file AND a8n0.parent=a8n1.id WHERE a8n1.kind IN (184,216,218,226,273)
),
binding_seeds(file,node,atom) AS (SELECT n.file,n.id,relation_leaf_atom(n.file,n.id,n.atom) FROM r7 t CROSS JOIN node n ON n.file=t.file AND n.id=t.node WHERE 1 AND n.named),
binding_walk(file,atom,witness,node) AS (SELECT n.file,t.atom,t.node,n.parent FROM binding_seeds t CROSS JOIN node n ON n.file=t.file AND n.id=t.node WHERE n.parent IS NOT NULL UNION ALL SELECT w.file,w.atom,w.witness,n.parent FROM binding_walk w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE n.parent IS NOT NULL AND NOT EXISTS (SELECT 1 FROM r4 s WHERE s.file=w.file AND s.node=w.node)),
binding_choices(file,node,atom,witness) AS MATERIALIZED (SELECT w.file,w.node,w.atom,MIN(w.witness) FROM binding_walk w JOIN r5 k ON k.file=w.file AND k.node=w.node GROUP BY w.file,w.node,w.atom),
origin_choices AS (SELECT p.file,p.origin,b.atom,b.witness,p.node AS ancestor,p.distance,ROW_NUMBER() OVER (PARTITION BY p.file,p.origin,b.atom ORDER BY p.distance) AS choice FROM pairs5 p JOIN binding_choices b ON b.file=p.file AND b.node=p.node),
capture_demand AS MATERIALIZED (SELECT file,origin,atom,witness,ancestor,distance FROM origin_choices WHERE choice=1),
answer0(file,node,atom,witness) AS (SELECT file,origin,atom,witness FROM capture_demand),
answer2(file,node,atom) AS MATERIALIZED (SELECT p.file,p.id,relation_leaf_atom(p.file,p.id,p.atom) FROM r13 k CROSS JOIN node p ON p.file=k.file AND p.id=k.node WHERE p.kind IN (1) AND p.named),
answer3_walk(file,origin,node,state) AS MATERIALIZED (SELECT DISTINCT w.file,w.node,n2.id,1 FROM answer0 w CROSS JOIN node n0 ON n0.file=w.file AND n0.id=w.node CROSS JOIN node n1 ON n1.file=n0.file AND n1.parent=n0.id CROSS JOIN node n2 ON n2.file=n1.file AND n2.parent=n1.id WHERE (n1.kind IN (146,147)) UNION SELECT w.file,w.origin,n1.id,1 FROM answer3_walk w CROSS JOIN node n0 ON n0.file=w.file AND n0.id=w.node CROSS JOIN node n1 ON n1.file=n0.file AND n1.parent=n0.id LEFT JOIN node bound0 ON bound0.file=n1.file AND bound0.id=n1.prev WHERE w.state=1 AND (n0.kind IN (184,185) AND n1.kind IN (1) OR n0.kind IN (182,183,208) AND NOT (COALESCE(bound0."end">bound0.start,0)))),
answer3(file,origin,atom) AS MATERIALIZED (SELECT w.file,w.origin,relation_leaf_atom(n0.file,n0.id,n0.atom) AS atom FROM answer3_walk w CROSS JOIN node n0 ON n0.file=w.file AND n0.id=w.node WHERE w.state=1 AND (n0.kind IN (1)) AND n0.named),
answer1(file,node,atom,witness) AS MATERIALIZED (SELECT a.file,a.node,a.atom,a.witness FROM answer0 a CROSS JOIN node n ON n.file=a.file AND n.id=a.node WHERE 1 AND NOT EXISTS (SELECT 1 FROM answer2 p WHERE p.file=a.file AND p.atom=a.atom AND p.node>a.node AND p.node<=n."last") AND NOT EXISTS (SELECT 1 FROM answer3 p WHERE p.file=a.file AND p.origin=a.node AND p.atom=a.atom)),
answer4(file,node,atom,witness) AS MATERIALIZED (SELECT a.file,n.id,a.atom,a.witness FROM answer1 a CROSS JOIN node b ON b.file=a.file AND b.id=a.node CROSS JOIN node n ON n.file=b.file AND n.parent=b.id LEFT JOIN node bound0 ON bound0.file=n.file AND bound0.id=n.prev WHERE ((b.kind IN (145) AND n.kind IN (161)) OR (b.kind IN (197) AND CASE WHEN bound0.id IS NOT NULL THEN node_regex(6,bound0.file,bound0.id) ELSE 0 END))),
answer5(file,node,atom,witness) AS MATERIALIZED (SELECT file,node,atom,witness FROM (SELECT a.file,n.id AS node,a.atom,a.witness,ROW_NUMBER() OVER (PARTITION BY a.file,n.id,a.atom ORDER BY a.node DESC) AS choice FROM answer4 a CROSS JOIN node b ON b.file=a.file AND b.id=a.node CROSS JOIN node v ON v.file=a.file AND v.id=a.atom CROSS JOIN node n ON n.file=b.file AND n.id>=b.id AND n.id<=b."last" AND n.thash=v.thash LEFT JOIN node bound0 ON bound0.file=n.file AND bound0.id=n.parent LEFT JOIN node bound1 ON bound1.file=n.file AND bound1.id=n.prev LEFT JOIN node bound2 ON bound2.file=n.file AND bound2.id=n.next LEFT JOIN node bound3 ON bound3.file=bound0.file AND bound3.id=bound0.parent LEFT JOIN node bound4 ON bound4.file=bound3.file AND bound4.id=bound3.parent WHERE n.kind IN (1) AND NOT (((COALESCE(bound0.kind IN (204),0) AND CASE WHEN bound1.id IS NOT NULL THEN node_regex(7,bound1.file,bound1.id) ELSE 0 END) OR (COALESCE(bound0.kind IN (215),0) AND CASE WHEN bound2.id IS NOT NULL THEN node_regex(3,bound2.file,bound2.id) ELSE 0 END) OR (NOT (COALESCE(bound1."end">bound1.start,0)) AND COALESCE(bound0.kind IN (182,183,208),0)) OR EXISTS (SELECT 1 FROM r13 k WHERE k.file=n.file AND k.node=n.id) OR CASE WHEN bound0.id IS NOT NULL THEN ((bound0.kind IN (184) AND COALESCE(bound3.kind IN (146,147,208),0)) OR (bound0.kind IN (273) AND CASE WHEN bound3.id IS NOT NULL THEN (bound3.kind IN (186) AND COALESCE(bound4.kind IN (140),0)) ELSE 0 END) OR (bound0.kind IN (186) AND COALESCE(bound3.kind IN (164),0)) OR bound0.kind IN (117,126,145,146,147,150,151,154,155,163,172,173,185)) ELSE 0 END)) AND n.named AND relation_leaf_atom(n.file,n.id,n.atom)=a.atom) WHERE choice=1)
SELECT 0,file,node,json_array(witness) FROM answer5 ORDER BY file,node
