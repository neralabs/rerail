.timer on
PRAGMA cache_size=-1000000; PRAGMA temp_store=MEMORY;
CREATE TABLE bind AS SELECT i.file, CASE WHEN i.alias<>'' THEN i.alias ELSE i.name END AS local, r.def FROM imp i JOIN imp_res2 r ON r.imp=i.id;
CREATE INDEX bind_fl ON bind(file, local);
CREATE INDEX IF NOT EXISTS call_file_callee ON call(file, callee);
CREATE INDEX IF NOT EXISTS def_file_scope_name ON def(file, scope, name);
CREATE TABLE edge3 AS
 SELECT c.id call, d.id def FROM call c JOIN def d ON d.file=c.file AND d.scope='' AND d.name=c.callee
 UNION SELECT c.id, b.def FROM call c JOIN bind b ON b.file=c.file AND b.local=c.callee;
SELECT count(*) FROM edge3;
