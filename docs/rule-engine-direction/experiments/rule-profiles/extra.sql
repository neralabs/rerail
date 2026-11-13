.timer on
PRAGMA cache_size=-1000000; PRAGMA temp_store=MEMORY;
CREATE TABLE IF NOT EXISTS contain AS SELECT p.id AS parent, c.id AS child FROM def c JOIN def p ON p.file=c.file AND p.qual=c.scope WHERE c.scope<>'';
CREATE INDEX IF NOT EXISTS contain_parent ON contain(parent);
CREATE TABLE IF NOT EXISTS g AS SELECT caller AS a, callee AS b FROM edge WHERE caller IS NOT NULL AND callee IS NOT NULL UNION SELECT parent, child FROM contain;
CREATE INDEX IF NOT EXISTS g_a ON g(a);
SELECT count(*) FROM g;
WITH RECURSIVE r(id) AS (SELECT id FROM def WHERE scope='' AND name NOT LIKE '\_%' ESCAPE '\' UNION SELECT g.b FROM g JOIN r ON g.a=r.id) SELECT count(*) FROM r;
-- unique-name resolution is loose; scoped resolution: same-file def first, else imported name, else unique global
CREATE TABLE IF NOT EXISTS edge2 AS
 WITH local AS (SELECT c.id call, d.id def FROM call c JOIN def d ON d.file=c.file AND d.scope='' AND d.name=c.callee),
 viaimp AS (SELECT c.id call, r.def FROM call c JOIN imp i ON i.file=c.file AND (CASE WHEN i.alias<>'' THEN i.alias ELSE i.name END)=c.callee JOIN imp_res2 r ON r.imp=i.id)
 SELECT * FROM local UNION SELECT * FROM viaimp;
SELECT count(*) FROM edge2;
