-- Python module paths of a worktree's files, from the package markers of the files the rule's
-- scope admits, as native's workspace derives them (workspace/ingest.rs:313-323 `module_under`:
-- `python_module_in` over the workspace's markers; a workspace holds only in-scope files,
-- ingest.rs:279-292), then `FileInput::new` (model.rs:755-771: Ruff's stem fallback). The
-- relations `file.module`/`file.package` projections and the reference graph read (front/rule.rs
-- compiles these stages ahead of the clause's own; INVARIANTS I2).
-- Inputs: member(__path), one row per member content (every file of the worktree), and the
-- constant `rule_scope(kind, glob)`: the rule's `scope.include`/`scope.exclude` globs.
-- Native paths below are relative to native/src/code_rule_dsl/. Each stage is one statement;
-- `-- index:` lines are speed hints for the SQLite harness (linker_check.py) only.

-- ===================================================================== scope
-- stage scope_glob
-- The rule's `scope` as regexes: globset with literal_separator (compile/mod.rs:251): `**/` any
-- directories, `/**` anything below, `*` within one segment.
SELECT g.kind, g.glob,
       '^' || replace(replace(replace(replace(replace(replace(g.glob, '.', '\.'), '**/', '%DIRS%'), '/**', '%BELOW%'),
                      '*', '[^/]*'), '%DIRS%', '(?:.*/)?'), '%BELOW%', '/.*') || '$' AS re
FROM rule_scope g

-- stage in_scope
-- The workspace's files: a workspace holds (and so links) only files some installed rule's scope
-- matches (workspace/ingest.rs:279-292 file_needs_update); here the clause's rule's scope, which
-- also filters the edges by source path (relational.rs:639,658).
SELECT m.__path AS path FROM member m
WHERE EXISTS (SELECT 1 FROM scope_glob g WHERE g.kind = 'include' AND rx_matches(m.__path, g.re))
  AND NOT EXISTS (SELECT 1 FROM scope_glob g WHERE g.kind = 'exclude' AND rx_matches(m.__path, g.re))
-- index: path

-- ===================================================================== module paths
-- stage package_dir
-- model.rs:847 package_marker_directory: `pkg/__init__.py` marks `pkg`; a root `__init__.py` marks ''.
SELECT DISTINCT CASE WHEN path = '__init__.py' THEN '' ELSE substr(path, 1, length(path) - 12) END AS dir
FROM in_scope WHERE path = '__init__.py' OR substr(path, -12) = '/__init__.py'
-- index: dir

-- stage file_name
-- A member's directory (`Path::parent`, '' at the top) and file name.
SELECT path, coalesce(rx_extract(path, '^(.*)/[^/]*$', 1), '') AS dir, rx_extract(path, '[^/]*$', 0) AS file,
       -- file_stem (module_path.rs:13; model.rs:825): the last extension off unless the dot leads.
       coalesce(rx_extract(rx_extract(path, '[^/]*$', 0), '^(.+)\.[^.]*$', 1), rx_extract(path, '[^/]*$', 0)) AS stem
FROM in_scope
-- index: path

-- stage package_climb
-- model.rs:811-818: from the file's directory, climb while each directory holds an `__init__.py`.
WITH RECURSIVE climb(path, dir) AS (
  SELECT f.path, f.dir FROM file_name f JOIN package_dir p ON p.dir = f.dir
  UNION
  SELECT c.path, coalesce(rx_extract(c.dir, '^(.*)/[^/]*$', 1), '')
  FROM climb c JOIN package_dir p ON p.dir = coalesce(rx_extract(c.dir, '^(.*)/[^/]*$', 1), '')
  WHERE c.dir <> ''
)
SELECT path, dir FROM climb
-- index: path

-- stage package_root
-- The highest directory the climb reaches (the shortest; a file's climb dirs are its ancestors,
-- so their lengths differ).
SELECT c.path, c.dir FROM package_climb c
JOIN (SELECT path, min(length(dir)) AS len FROM package_climb GROUP BY path) m ON m.path = c.path AND m.len = length(c.dir)
-- index: path

-- stage module_rel
-- model.rs:819-824: names from the package root's parent down; root '' has no parent (None).
SELECT f.path, f.stem,
       CASE WHEN r.dir IS NULL OR r.dir = '' THEN NULL
            WHEN instr(r.dir, '/') = 0 THEN f.path
            ELSE substr(f.path, length(rx_extract(r.dir, '^(.*)/[^/]*$', 1)) + 2) END AS rel
FROM file_name f
LEFT JOIN package_root r ON r.path = f.path
-- index: path

-- stage module_name_part
-- model.rs:820-829: the relative path's components, the last one by its stem.
WITH RECURSIVE part(path, i, name, rest) AS (
  SELECT path, 0, CASE WHEN instr(rel, '/') > 0 THEN substr(rel, 1, instr(rel, '/') - 1) ELSE stem END,
         CASE WHEN instr(rel, '/') > 0 THEN substr(rel, instr(rel, '/') + 1) END
  FROM module_rel WHERE rel IS NOT NULL
  UNION
  SELECT p.path, p.i + 1,
         CASE WHEN instr(p.rest, '/') > 0 THEN substr(p.rest, 1, instr(p.rest, '/') - 1) ELSE r.stem END,
         CASE WHEN instr(p.rest, '/') > 0 THEN substr(p.rest, instr(p.rest, '/') + 1) END
  FROM part p JOIN module_rel r ON r.path = p.path WHERE p.rest IS NOT NULL
)
SELECT path, i, name FROM part
-- index: path, i

-- stage module_path
-- The file's ModulePath (model.rs:806-838 then FileInput::new model.rs:761-771): its package
-- names when every name is non-empty, else Ruff's stem fallback (module_path.rs `stem`).
SELECT r.path, 1 AS package, (SELECT count(*) FROM module_name_part p WHERE p.path = r.path) AS n
FROM module_rel r
WHERE r.rel IS NOT NULL AND NOT EXISTS (SELECT 1 FROM module_name_part p WHERE p.path = r.path AND p.name = '')
UNION ALL
SELECT r.path, 0, 1 FROM module_rel r
WHERE (r.rel IS NULL OR EXISTS (SELECT 1 FROM module_name_part p WHERE p.path = r.path AND p.name = ''))
  AND r.stem <> ''
-- index: path

-- stage module_segment
-- ModulePath.segments.
SELECT p.path, p.i, p.name FROM module_name_part p JOIN module_path m ON m.path = p.path AND m.package = 1
UNION ALL
SELECT m.path, 0, f.stem FROM module_path m JOIN file_name f ON f.path = m.path WHERE m.package = 0
-- index: path, i

-- stage module_text
-- ModulePath::text (module_path.rs:57): the segments joined by '.', built one segment per step.
WITH RECURSIVE q(path, count, text) AS (
  SELECT s.path, 1, s.name FROM module_segment s WHERE s.i = 0
  UNION
  SELECT q.path, q.count + 1, q.text || '.' || s.name
  FROM q JOIN module_segment s ON s.path = q.path AND s.i = q.count
)
SELECT q.path, q.count, q.text FROM q
-- index: path, count

-- stage module_of
-- The `file.module` and `file.package` projections (eval/evidence.rs:181-186): `text()` and
-- `package_text()` (the segments without the last), NULL where the file has no module.
SELECT m.path, t.text AS module, coalesce(p.text, '') AS package
FROM module_path m JOIN module_text t ON t.path = m.path AND t.count = m.n
LEFT JOIN module_text p ON p.path = m.path AND p.count = m.n - 1
-- index: path

