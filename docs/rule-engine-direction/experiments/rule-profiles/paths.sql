.timer on
PRAGMA cache_size=-1000000; PRAGMA temp_store=MEMORY;
CREATE TABLE IF NOT EXISTS cg AS SELECT DISTINCT cd.id AS caller, e.def AS callee FROM edge3 e JOIN call c ON c.id=e.call JOIN def cd ON cd.file=c.file AND cd.qual=c.scope JOIN def t ON t.id=e.def WHERE t.file=c.file AND cd.id<>e.def;
CREATE INDEX IF NOT EXISTS cg_callee ON cg(callee);
SELECT count(*) FROM cg;
-- sinks: defs that call execute-like names
CREATE TABLE IF NOT EXISTS sink AS SELECT DISTINCT d.id FROM call c JOIN def d ON d.file=c.file AND d.qual=c.scope WHERE c.last LIKE 'execute%' OR c.last LIKE 'fetch%';
SELECT count(*) FROM sink;
-- set semantics: (fn, origin) closure
WITH RECURSIVE s(fn, origin) AS (SELECT id, id FROM sink UNION SELECT g.caller, s.origin FROM s JOIN cg g ON g.callee=s.fn) SELECT count(*) FROM s;
-- route enumeration (DLRE transitive_sinks shape), depth-limited
WITH RECURSIVE s(fn, origin, route, d) AS (SELECT id, id, '|'||id||'|', 0 FROM sink UNION ALL SELECT g.caller, s.origin, s.route||g.caller||'|', d+1 FROM s JOIN cg g ON g.callee=s.fn WHERE instr(s.route,'|'||g.caller||'|')=0 AND d<12) SELECT count(*), max(d) FROM s;
