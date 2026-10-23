-- Native's Python reference graph (`reference_edges`, refs mode) as SQL stages.
-- Inputs: member(path, hash) and the cgn export's `as_references` relations per content
-- (cgn_<rel>(hash, variant, stub, ...), every variant). Native paths below are relative to
-- native/src/code_rule_dsl/. Each stage is one statement; `-- index:` lines are speed hints.

-- ===================================================================== scope
-- stage scope_glob
-- The dead-code rule's `scope` (packs/data-layer/candidates/rules/data-layer-internal-dead-code.json)
-- as regexes: globset with literal_separator (compile/mod.rs:251): `**/` any directories, `/**`
-- anything below, `*` within one segment.
SELECT g.kind, g.glob,
       '^' || replace(replace(replace(replace(replace(replace(g.glob, '.', '\.'), '**/', '%DIRS%'), '/**', '%BELOW%'),
                      '*', '[^/]*'), '%DIRS%', '(?:.*/)?'), '%BELOW%', '/.*') || '$' AS re
FROM (SELECT 'include' AS kind, value AS glob FROM json_each('["api/**/*.py","impl/**/*.py","data_layer/**/*.py","_internal/**/*.py","l1000_cache/**/*.py","l1000_schema/**/*.py","l1050_model_registry/**/*.py","l1050_polymorphism/**/*.py","l1100_exceptions/**/*.py","l1200_capabilities/**/*.py","l1200_content_hash/**/*.py","l1200_json/**/*.py","l1200_versioned_ids/**/*.py","l1250_security/**/*.py","l1300_session/**/*.py","l1350_foundational_types/**/*.py","l1400_config/**/*.py","l1400_types/**/*.py","l1450_index_policy_contracts/**/*.py","l1450_keys/**/*.py","l1470_matching_contracts/**/*.py","l1500_llm/**/*.py","l1900_retry/**/*.py","l2000_connection/**/*.py","l2050_transaction/**/*.py","l2055_lazy_result/**/*.py","l2060_query_builder/**/*.py","l2070_batching/**/*.py","l2080_lazy_orm/**/*.py","l2090_byte_tracking/**/*.py","l2090_response/**/*.py","l2091_rtt_tracking/**/*.py","l2092_timing_tracking/**/*.py","l2100_include/**/*.py","l2110_limits/**/*.py","l2150_prepared_cache/**/*.py","l2200_notification/**/*.py","l2500_binary_diff/**/*.py","l3000_schema_builder/**/*.py","l3100_schema_introspector/**/*.py","l3150_schema_diff/**/*.py","l3200_metadata/**/*.py","l3250_migrations/**/*.py","l3260_startup/**/*.py","l3500_bulk_ops/**/*.py","l4000_permissions/**/*.py","l4050_background_reaper/**/*.py","l4100_queue/**/*.py","l4150_scheduler/**/*.py","l4170_inbound_bridge/**/*.py","l4200_event_pipeline/**/*.py","l4250_timers/**/*.py","l4300_ipc/**/*.py","l4300_timers/**/*.py","l4350_timer_driver/**/*.py","l4400_scheduler/**/*.py","l4500_trust/**/*.py","l5000_search_index_policy/**/*.py","l5000_subscriptions/**/*.py","l5100_fts/**/*.py","l5100_llm_router/**/*.py","l5175_chunking/**/*.py","l5195_embedding_compute_tables/**/*.py","l5200_vector_index/**/*.py","l5250_matching/**/*.py","l5300_config_compatibility/**/*.py","l6000_log_table/**/*.py","l6300_config/**/*.py","l6500_evidence/**/*.py","l6510_credentials/**/*.py","l6520_auth/**/*.py","l6530_managed_secrets/**/*.py","l6600_cleanup/**/*.py","l7000_versioned_schema/**/*.py","l7100_versioned_operations/**/*.py","l7200_versioned_transactions/**/*.py","l7250_versioned_operations/**/*.py","l7350_versioned_timer_driver/**/*.py","l7390_versioned_diff_entrypoint/**/*.py","l7400_versioned_diff/**/*.py","l7450_versioned_revert/**/*.py","l7500_commit_triggers/**/*.py","l7800_versioned_merge/**/*.py","l7900_versioned_rebase/**/*.py","l8000_admin/**/*.py","l8100_ops_analytics/**/*.py","l9600_graphs/**/*.py","l9700_graph_query/**/*.py","l9710_graph_algorithms/**/*.py","l9720_sparql/**/*.py","l9750_graphql/**/*.py","l9800_rippling/**/*.py","l9850_workflow/**/*.py","l9875_mcp/**/*.py","l9900_auto_api/**/*.py"]')
      UNION ALL
      SELECT 'exclude', value FROM json_each('["**/.git/**","**/.venv/**","**/tmp/**","**/__pycache__/**","**/tests/**"]')) g

-- stage in_scope
-- The workspace's files: a workspace holds (and so links) only files some installed rule's scope
-- matches (workspace/ingest.rs:279-292 file_needs_update); with the dead-code rule that is its
-- scope, which also filters the edges by source path (relational.rs:639,658).
SELECT m.path, m.hash FROM member m
WHERE EXISTS (SELECT 1 FROM scope_glob g WHERE g.kind = 'include' AND rx_matches(m.path, g.re))
  AND NOT EXISTS (SELECT 1 FROM scope_glob g WHERE g.kind = 'exclude' AND rx_matches(m.path, g.re))
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

-- stage module_rel
-- model.rs:819-824: names from the package root's parent down; root '' has no parent (None).
SELECT f.path, f.stem,
       CASE WHEN r.dir IS NULL OR r.dir = '' THEN NULL
            WHEN instr(r.dir, '/') = 0 THEN f.path
            ELSE substr(f.path, length(rx_extract(r.dir, '^(.*)/[^/]*$', 1)) + 2) END AS rel
FROM file_name f
LEFT JOIN package_climb r ON r.path = f.path
  AND NOT EXISTS (SELECT 1 FROM package_climb o WHERE o.path = r.path AND length(o.dir) < length(r.dir))
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

-- stage module_prefix
-- module_path.rs `qualified(count)`: the first `count` segments, directories by their stem,
-- the file's whole; NULL when one keeps a dot.
WITH RECURSIVE q(path, count, text) AS (
  SELECT path, 0, '' FROM module_path
  UNION
  SELECT q.path, q.count + 1,
         CASE WHEN q.text IS NULL THEN NULL
              WHEN instr(CASE WHEN s.i = m.n - 1 THEN s.name
                              ELSE coalesce(rx_extract(s.name, '^(.+)\.[^.]*$', 1), s.name) END, '.') > 0 THEN NULL
              WHEN q.count = 0 THEN CASE WHEN s.i = m.n - 1 THEN s.name ELSE coalesce(rx_extract(s.name, '^(.+)\.[^.]*$', 1), s.name) END
              ELSE q.text || '.' || CASE WHEN s.i = m.n - 1 THEN s.name ELSE coalesce(rx_extract(s.name, '^(.+)\.[^.]*$', 1), s.name) END END
  FROM q JOIN module_segment s ON s.path = q.path AND s.i = q.count JOIN module_path m ON m.path = q.path
)
SELECT path, count, text FROM q
-- index: path, count

-- stage file_variant
-- The export's symbolic variant the real module selects (cgnative.rs:73 `Variant::of`).
SELECT mb.path, mb.hash, m.n,
       CASE WHEN m.package = 0 THEN 'unpackaged'
            WHEN m.n > 1 AND (SELECT s.name FROM module_segment s WHERE s.path = m.path AND s.i = m.n - 1) = '__init__' THEN 'init'
            ELSE 'module' END AS variant,
       CASE WHEN m.package = 0 THEN 1 ELSE 17 END AS symlen,
       CASE WHEN substr(mb.path, -4) = '.pyi' THEN '1' ELSE '0' END AS stub
FROM in_scope mb JOIN module_path m ON m.path = mb.path
-- index: path
-- index: hash, variant, stub

-- ===================================================================== refusals: paths
-- stage path_refusal
-- Files whose summary the export cannot give exactly (cgnative.rs:114-120, SCHEMA § 3.4).
SELECT m.path, 'module: none (empty stem)' AS construct FROM in_scope m
WHERE NOT EXISTS (SELECT 1 FROM module_path p WHERE p.path = m.path)
UNION
SELECT s.path, 'module: named like a module native compares names with (abc, typing, typing_extensions, functools)'
FROM module_segment s WHERE s.i = 0 AND s.name IN ('abc', 'typing', 'typing_extensions', 'functools')
UNION
SELECT path, 'module: deeper than the symbolic module (17 segments)' FROM module_path WHERE n > 17
UNION
SELECT path, 'module: a name is . or .. (ModulePath::names rejects it)' FROM module_segment WHERE name IN ('.', '..')
UNION
-- path_module's identifier test (closure.rs:521-527) is Unicode `is_alphabetic`; the stage's is ASCII.
SELECT path, 'path_module: non-ASCII path' FROM in_scope WHERE rx_matches(path, '[^\x00-\x7F]')

-- ===================================================================== substitution
-- stage raw_name
-- Every exported string of a member's variant holding a sentinel token.
SELECT DISTINCT v.path, x.raw FROM file_variant v JOIN (
  SELECT hash, variant, stub, name AS raw FROM cgn_def
  UNION ALL SELECT hash, variant, stub, name FROM cgn_edge
  UNION ALL SELECT hash, variant, stub, "key" FROM cgn_alias
  UNION ALL SELECT hash, variant, stub, target FROM cgn_alias
  UNION ALL SELECT hash, variant, stub, name FROM cgn_opaque
  UNION ALL SELECT hash, variant, stub, name FROM cgn_module
  UNION ALL SELECT hash, variant, stub, "key" FROM cgn_reference_alias WHERE instr("key", '.<base-slot>.') > 0
  UNION ALL SELECT hash, variant, stub, target FROM cgn_reference_alias WHERE instr("key", '.<base-slot>.') > 0
) x ON x.hash = v.hash AND x.variant = v.variant AND x.stub = v.stub
WHERE rx_matches(x.raw, '\x01')
  AND NOT EXISTS (SELECT 1 FROM path_refusal r WHERE r.path = v.path)

-- stage name_sub
-- cgnative.rs:163 `substitute`: token \x01<r> is the real module's `qualified(r - symbolic
-- length + real length)`; an empty prefix takes its following '.'.
WITH RECURSIVE s(path, raw, done, rest, bad) AS (
  SELECT path, raw, '', raw, NULL FROM raw_name
  UNION
  SELECT s.path, s.raw,
         s.done || rx_extract(s.rest, '^([^\x01]*)\x01', 1) || coalesce(q.text, ''),
         CASE WHEN q.text = '' AND substr(rx_extract(s.rest, '\x01[0-9]+(.*)$', 1), 1, 1) = '.'
              THEN substr(rx_extract(s.rest, '\x01[0-9]+(.*)$', 1), 2)
              ELSE rx_extract(s.rest, '\x01[0-9]+(.*)$', 1) END,
         CASE WHEN CAST(rx_extract(s.rest, '\x01([0-9]+)', 1) AS INTEGER) - v.symlen + v.n < 0
                   THEN 'substitution: relative import above the real package root'
              WHEN q.text IS NULL THEN 'substitution: real module segment keeps a dot' END
  FROM s JOIN file_variant v ON v.path = s.path
  LEFT JOIN module_prefix q ON q.path = s.path
       AND q.count = CAST(rx_extract(s.rest, '\x01([0-9]+)', 1) AS INTEGER) - v.symlen + v.n
  WHERE s.bad IS NULL AND rx_matches(s.rest, '\x01')
)
SELECT path, raw, CASE WHEN bad IS NULL THEN done || rest END AS name, bad FROM s
WHERE bad IS NOT NULL OR NOT rx_matches(rest, '\x01')
-- index: path, raw

-- stage refused_path
-- A file whose summary cannot be substituted is not linked (no guess of its names).
SELECT path, construct FROM path_refusal
UNION SELECT DISTINCT path, bad FROM name_sub WHERE bad IS NOT NULL
-- index: path

-- stage linked_file
-- The files linked: members with a variant and an exact substitution.
SELECT v.path, v.hash, v.variant, v.stub FROM file_variant v
WHERE NOT EXISTS (SELECT 1 FROM refused_path r WHERE r.path = v.path)
-- index: path
-- index: hash, variant, stub

-- ===================================================================== substituted summaries
-- stage s_def
-- summary.rs:100 Def, after as_references (summary.rs:198-208: OPEN cleared; SHADOWED cleared on
-- the last def of a qualname) — as exported.
SELECT f.path, CAST(d.idx AS INTEGER) AS idx, coalesce(n.name, d.name) AS name,
       CAST(d.start AS INTEGER) AS start, CAST(d."end" AS INTEGER) AS stop, CAST(d.flags AS INTEGER) AS flags
FROM linked_file f JOIN cgn_def d ON d.hash = f.hash AND d.variant = f.variant AND d.stub = f.stub
LEFT JOIN name_sub n ON n.path = f.path AND n.raw = d.name
-- index: path, idx
-- index: path, start
-- index: name

-- stage s_edge
-- summary.edges in refs mode: references plus siblingLinks rows (summary.rs:128, 182-199).
SELECT DISTINCT f.path, CAST(e."def" AS INTEGER) AS idx, coalesce(n.name, e.name) AS name
FROM linked_file f JOIN cgn_edge e ON e.hash = f.hash AND e.variant = f.variant AND e.stub = f.stub
LEFT JOIN name_sub n ON n.path = f.path AND n.raw = e.name
-- index: name
-- index: path, idx

-- stage s_alias
-- summary.aliases in refs mode (rebuilt from reference_aliases, summary.rs:161-180).
SELECT DISTINCT f.path, coalesce(k.name, a."key") AS key, coalesce(t.name, a.target) AS target
FROM linked_file f JOIN cgn_alias a ON a.hash = f.hash AND a.variant = f.variant AND a.stub = f.stub
LEFT JOIN name_sub k ON k.path = f.path AND k.raw = a."key"
LEFT JOIN name_sub t ON t.path = f.path AND t.raw = a.target
-- index: path, key

-- stage s_opaque
SELECT DISTINCT f.path, coalesce(n.name, o.name) AS name
FROM linked_file f JOIN cgn_opaque o ON o.hash = f.hash AND o.variant = f.variant AND o.stub = f.stub
LEFT JOIN name_sub n ON n.path = f.path AND n.raw = o.name

-- stage s_module
-- summary.module: ModulePath::import_name (module_path.rs).
SELECT f.path, coalesce(n.name, m.name) AS name
FROM linked_file f JOIN cgn_module m ON m.hash = f.hash AND m.variant = f.variant AND m.stub = f.stub
LEFT JOIN name_sub n ON n.path = f.path AND n.raw = m.name
-- index: path

-- stage s_open
SELECT f.path, CAST(o."open" AS INTEGER) AS open
FROM linked_file f JOIN cgn_open o ON o.hash = f.hash AND o.variant = f.variant AND o.stub = f.stub

-- stage s_base_slot
-- `{class}.<base-slot>.{classStart}:{ordinal}` → `{module}.<base>.{short}` (summary.rs:1087-1095):
-- the ordinal is the zero-padded position among the file's reference aliases, so it orders a
-- class's bases as declared.
SELECT DISTINCT f.path, coalesce(k.name, a."key") AS key, coalesce(t.name, a.target) AS target
FROM linked_file f JOIN cgn_reference_alias a ON a.hash = f.hash AND a.variant = f.variant AND a.stub = f.stub
LEFT JOIN name_sub k ON k.path = f.path AND k.raw = a."key"
LEFT JOIN name_sub t ON t.path = f.path AND t.raw = a.target
WHERE instr(a."key", '.<base-slot>.') > 0

-- stage s_super
-- as_references (summary.rs:139-144, 171-176): a base slot whose class start is the start of the
-- last class def of that name becomes a BASE super, in slot order.
SELECT b.path, substr(b.key, 1, instr(b.key, '.<base-slot>.') - 1) AS cls, b.target,
       substr(b.key, instr(b.key, '.<base-slot>.') + 13 + instr(substr(b.key, instr(b.key, '.<base-slot>.') + 13), ':')) AS ord
FROM s_base_slot b
WHERE instr(substr(b.key, instr(b.key, '.<base-slot>.') + 13), ':') > 0
  AND CAST(substr(b.key, instr(b.key, '.<base-slot>.') + 13,
                  instr(substr(b.key, instr(b.key, '.<base-slot>.') + 13), ':') - 1) AS INTEGER)
      = (SELECT d.start FROM s_def d WHERE d.path = b.path AND (d.flags / 2) % 2 = 1
           AND d.name = substr(b.key, 1, instr(b.key, '.<base-slot>.') - 1)
           AND NOT EXISTS (SELECT 1 FROM s_def l WHERE l.path = d.path AND l.name = d.name
                             AND (l.flags / 2) % 2 = 1 AND l.idx > d.idx))

-- ===================================================================== name tables
-- stage path_alt
-- closure.rs:513-531 path_module: strip .py/.pyi, drop a trailing __init__, keep the longest
-- suffix of identifier segments; only where it differs from the module (closure.rs:134-135).
SELECT x.path, x.alt FROM (
  SELECT b.path, replace(rx_extract(b.stripped, '(?:^|/)((?:[_A-Za-z][_A-Za-z0-9]*/)*[_A-Za-z][_A-Za-z0-9]*)$', 1), '/', '.') AS alt
  FROM (SELECT path, CASE WHEN instr(p, '/') > 0 AND substr(p, -9) = '/__init__' THEN substr(p, 1, length(p) - 9) ELSE p END AS stripped
        FROM (SELECT path, CASE WHEN substr(path, -3) = '.py' THEN substr(path, 1, length(path) - 3)
                                WHEN substr(path, -4) = '.pyi' THEN substr(path, 1, length(path) - 4) END AS p
              FROM linked_file) WHERE p IS NOT NULL) b
) x
WHERE x.alt IS NOT NULL AND NOT EXISTS (SELECT 1 FROM s_module m WHERE m.path = x.path AND m.name = x.alt)
-- index: path

-- stage spelling
-- Contribution::of `names` (closure.rs:136-145): a name, and where it starts with the module (then
-- end or '.'), the same name under the path's spelling.
SELECT DISTINCT x.path, x.name, x.name AS key FROM (
  SELECT path, name FROM s_def UNION SELECT path, key FROM s_alias UNION SELECT path, name FROM s_opaque
  UNION SELECT path, cls FROM s_super) x
UNION
SELECT x.path, x.name, a.alt || substr(x.name, length(m.name) + 1) FROM (
  SELECT path, name FROM s_def UNION SELECT path, key FROM s_alias UNION SELECT path, name FROM s_opaque
  UNION SELECT path, cls FROM s_super) x
JOIN s_module m ON m.path = x.path JOIN path_alt a ON a.path = x.path
WHERE x.name = m.name OR substr(x.name, 1, length(m.name) + 1) = m.name || '.'
-- index: path, name

-- stage by_name
-- Graph.by_name (closure.rs:146-154; language/adapter/calls.rs:149: a SHADOWED def has no key).
SELECT s.key, d.path, d.idx, d.flags, d.name FROM s_def d JOIN spelling s ON s.path = d.path AND s.name = d.name
WHERE (d.flags / 64) % 2 = 0
-- index: key

-- stage aliases
-- Graph.aliases (closure.rs:172-180): keys under both spellings, targets as written.
SELECT DISTINCT s.key, a.path, a.target FROM s_alias a JOIN spelling s ON s.path = a.path AND s.name = a.key
-- index: key
-- index: target

-- stage opaque
SELECT DISTINCT s.key FROM s_opaque o JOIN spelling s ON s.path = o.path AND s.name = o.name
-- index: key

-- stage modules
-- Graph.modules (closure.rs:191-197): the module and the path's spelling; stars are empty in refs mode.
SELECT m.name, m.path FROM s_module m
UNION SELECT a.alt, a.path FROM path_alt a
-- index: name

-- stage supers
-- Graph.supers (closure.rs:155-171), BASE only in refs mode; `ord` is the declared order.
SELECT DISTINCT s.key AS cls, x.path, x.target, x.ord FROM s_super x JOIN spelling s ON s.path = x.path AND s.name = x.cls
-- index: cls
-- index: target

-- stage class_key
-- Names whose by_name entries are all classes.
SELECT key FROM by_name GROUP BY key HAVING min((flags / 2) % 2) = 1
-- index: key

-- stage method_def
-- by_name entries that are not classes (reference_method, closure.rs:2146-2154).
SELECT key, path, idx FROM by_name WHERE (flags / 2) % 2 = 0
-- index: key

-- stage reference_class
-- Graph.reference_classes (closure.rs:773-798): every class def of every file by its last segment;
-- class_of uses a short name with exactly one entry.
SELECT rx_extract(d.name, '[^.]*$', 0) AS short, min(d.name) AS name
FROM s_def d WHERE (d.flags / 2) % 2 = 1
GROUP BY rx_extract(d.name, '[^.]*$', 0) HAVING count(*) = 1
-- index: short

-- stage own_name
-- closure.rs:2497 own_name: the def name of `by_name[key].first()`; a key whose entries name
-- different defs depends on slot order (refused where used, `own_name_refusal`).
SELECT key, min(name) AS name, count(DISTINCT name) AS names FROM by_name GROUP BY key
-- index: key

-- ===================================================================== class resolution
-- stage base_target
-- class_noting for `<base>` names (closure.rs:2288-2311), at hops 0 (reference_bases):
-- aliases `{module}.<imported>.{short}` then `{module}.<class>.{short}`, the first target that is a
-- workspace class; else the one class workspace-wide of that short name.
SELECT t.target, k.rank, a.target AS cls
FROM (SELECT DISTINCT target, substr(target, 1, instr(target, '.<base>.') - 1) AS module,
             substr(target, instr(target, '.<base>.') + 8) AS short
      FROM supers WHERE instr(target, '.<base>.') > 0) t
JOIN (SELECT 0 AS rank, '<imported>' AS kind UNION ALL SELECT 1, '<class>') k
JOIN aliases a ON a.key = t.module || '.' || k.kind || '.' || t.short
JOIN class_key c ON c.key = a.target
-- index: target

-- stage base_class
-- The resolved base (Mro::Workspace) of each `<base>` target; `several` > 1 is a first-of-several
-- choice native makes by alias entry order (refused).
SELECT t.target,
       CASE WHEN EXISTS (SELECT 1 FROM base_target b WHERE b.target = t.target)
            THEN (SELECT min(b.cls) FROM base_target b WHERE b.target = t.target
                    AND b.rank = (SELECT min(rank) FROM base_target x WHERE x.target = t.target))
            ELSE (SELECT r.name FROM reference_class r WHERE r.short = substr(t.target, instr(t.target, '.<base>.') + 8)) END AS cls,
       (SELECT count(DISTINCT b.cls) FROM base_target b WHERE b.target = t.target
          AND b.rank = (SELECT min(rank) FROM base_target x WHERE x.target = t.target)) AS several
FROM (SELECT DISTINCT target FROM supers WHERE instr(target, '.<base>.') > 0) t
-- index: target

-- stage bases
-- reference_bases (closure.rs:2106-2120): a class's BASE supers resolved to workspace classes,
-- in declared order.
SELECT s.cls, s.ord, b.cls AS base FROM supers s JOIN base_class b ON b.target = s.target
WHERE b.cls IS NOT NULL AND b.several <= 1
-- index: cls

-- stage subclass
-- python_classes / python_class (closure.rs:2380-2494): a class (own name equal to its key) is
-- a subclass of each workspace base's own name.
SELECT DISTINCT o2.name AS cls, s.cls AS child
FROM supers s JOIN own_name o ON o.key = s.cls AND o.name = s.cls
JOIN base_class b ON b.target = s.target AND b.cls IS NOT NULL AND b.several <= 1
JOIN own_name o2 ON o2.key = b.cls
-- index: cls

-- stage marker_class
-- Class parts of the `<self>`, `<super>`, `<field>` names the summaries reference.
SELECT DISTINCT substr(name, 1, instr(name, '.<self>.') - 1) AS cls FROM s_edge WHERE instr(name, '.<self>.') > 0
UNION SELECT substr(name, 1, instr(name, '.<super>.') - 1) FROM s_edge WHERE instr(name, '.<super>.') > 0
UNION SELECT substr(name, 1, instr(name, '.<field>.') - 1) FROM s_edge WHERE instr(name, '.<field>.') > 0

-- stage class_root
-- Every class whose reference_order a link may read.
SELECT key AS cls FROM class_key UNION SELECT cls FROM supers UNION SELECT base FROM bases
UNION SELECT cls FROM marker_class UNION SELECT name FROM reference_class
-- index: cls

-- stage order_dist
-- reference_order (closure.rs:2122-2135) is breadth-first over reference_bases with a visited set:
-- a class's place is first its distance from the root ...
WITH RECURSIVE r(root, cls, dist) AS (
  SELECT cls, cls, 0 FROM class_root
  UNION
  SELECT r.root, b.base, r.dist + 1 FROM r JOIN bases b ON b.cls = r.cls WHERE r.dist < 64
)
SELECT root, cls, min(dist) AS dist FROM r GROUP BY root, cls
-- index: root, cls

-- stage reference_order
-- ... then, among classes at that distance, the least sequence of declared base ordinals along a
-- shortest path (the queue's order). (dist, seq) is the BFS position.
WITH RECURSIVE p(root, cls, dist, seq) AS (
  SELECT cls, cls, 0, '' FROM class_root
  UNION
  SELECT p.root, b.base, p.dist + 1, p.seq || b.ord
  FROM p JOIN bases b ON b.cls = p.cls
  JOIN order_dist d ON d.root = p.root AND d.cls = b.base AND d.dist = p.dist + 1
)
SELECT root, cls, dist, min(seq) AS seq FROM p GROUP BY root, cls, dist
-- index: root, cls
-- index: root, dist, seq

-- stage imported_alias
-- `{module}.<imported>.{short}` aliases, by the name `module.short` whose lookup reads them.
SELECT key, target, substr(key, 1, instr(key, '.<imported>.') - 1) || '.' || substr(key, instr(key, '.<imported>.') + 12) AS name
FROM aliases WHERE instr(key, '.<imported>.') > 0 AND instr(substr(key, instr(key, '.<imported>.') + 12), '.') = 0
-- index: target

-- stage fq_class
-- reference_fq_class (closure.rs:2228-2246): a name is a class through by_name (all classes) or
-- `<imported>` aliases, `k` hops in; it is found at hops H iff H + k <= 16.
WITH RECURSIVE f(name, cls, k) AS (
  SELECT key, key, 0 FROM class_key
  UNION
  SELECT a.name, f.cls, f.k + 1 FROM f JOIN imported_alias a ON a.target = f.name
  WHERE f.k < 17 AND NOT EXISTS (SELECT 1 FROM by_name b WHERE b.key = a.name)
)
SELECT name, cls, min(k) AS k FROM f GROUP BY name, cls
-- index: name

-- stage type_key
-- reference_type (closure.rs:2248-2268): per alias key `{module}.<imported|class>.{short}`, the class
-- its first target with a class gives (`several` > 1: a first-of-several choice, refused).
SELECT a.key, min(f.cls) AS cls, min(f.k) AS k, count(DISTINCT f.cls) AS several
FROM aliases a JOIN fq_class f ON f.name = a.target
WHERE instr(a.key, '.<imported>.') > 0 OR instr(a.key, '.<class>.') > 0
GROUP BY a.key
-- index: key

-- stage type_unique
-- The keys with one class (the others are refused).
SELECT key, cls, k FROM type_key WHERE several = 1
-- index: key

-- stage digit
-- Positions 1..2048 (prefix enumeration); longer names are refused (`linker_refusal`).
WITH RECURSIVE n(i) AS (SELECT 1 UNION SELECT i + 1 FROM n WHERE i < 2048) SELECT i FROM n
-- index: i

-- stage field_alias
-- Aliases a constructor field may read: `{base}.<init-parameter>.{param}` and `{base}.{field}`
-- for every class whose order a field lookup walks (closure.rs:1890-1926).
SELECT a.key, a.target FROM aliases a
WHERE instr(a.key, '.<init-parameter>.') > 0
   OR EXISTS (SELECT 1 FROM class_root c WHERE c.cls = rx_extract(a.key, '^(.*)\.[^.]*$', 1))
-- index: key

-- stage class_of_walk
-- class_of / class_noting for a field alias's target (closure.rs:2272-2362), one hop per row.
WITH RECURSIVE w(start, cur, k) AS (
  SELECT DISTINCT target, target, 0 FROM field_alias WHERE substr(target, 1, 13) <> '<annotation>.'
  UNION
  SELECT w.start, x.next, w.k + 1 FROM w JOIN (
    -- an alias of the name itself, if all its entries agree
    SELECT a.key AS cur, min(a.target) AS next FROM aliases a GROUP BY a.key HAVING count(DISTINCT a.target) = 1
  ) x ON x.cur = w.cur
  WHERE w.k < 17 AND instr(w.cur, '.<base>.') = 0
    AND NOT EXISTS (SELECT 1 FROM by_name b WHERE b.key = w.cur)
    AND NOT EXISTS (SELECT 1 FROM opaque o WHERE o.key = w.cur)
  UNION
  -- else the longest prefix any table names, when that is an alias all of whose entries agree
  SELECT w.start, x.target || substr(w.cur, d.i), w.k + 1
  FROM w JOIN digit d ON d.i < length(w.cur) AND substr(w.cur, d.i, 1) = '.'
  JOIN (SELECT a.key, min(a.target) AS target FROM aliases a GROUP BY a.key HAVING count(DISTINCT a.target) = 1) x
    ON x.key = substr(w.cur, 1, d.i - 1) AND x.target <> x.key
  WHERE w.k < 17 AND instr(w.cur, '.<base>.') = 0
    AND NOT EXISTS (SELECT 1 FROM by_name b WHERE b.key = w.cur)
    AND NOT EXISTS (SELECT 1 FROM opaque o WHERE o.key = w.cur)
    AND NOT EXISTS (SELECT 1 FROM aliases a WHERE a.key = w.cur)
    AND NOT EXISTS (SELECT 1 FROM digit e WHERE e.i > d.i AND e.i < length(w.cur) AND substr(w.cur, e.i, 1) = '.'
                      AND (EXISTS (SELECT 1 FROM aliases a WHERE a.key = substr(w.cur, 1, e.i - 1)
                                     GROUP BY a.key HAVING count(DISTINCT a.target) = 1 AND min(a.target) <> a.key)
                           OR EXISTS (SELECT 1 FROM by_name b WHERE b.key = substr(w.cur, 1, e.i - 1))
                           OR EXISTS (SELECT 1 FROM opaque o WHERE o.key = substr(w.cur, 1, e.i - 1))
                           OR EXISTS (SELECT 1 FROM modules m WHERE m.name = substr(w.cur, 1, e.i - 1))))
)
SELECT start, cur, k FROM w
-- index: cur

-- stage class_of
-- Where the walk ends: a `<base>` name, or a by_name name of classes, is a workspace class
-- (`k` hops in: found at hops H iff H + k <= 16).
SELECT w.start, b.cls, w.k FROM class_of_walk w JOIN base_class b ON b.target = w.cur
WHERE instr(w.cur, '.<base>.') > 0 AND b.cls IS NOT NULL AND b.several <= 1
UNION
SELECT w.start, w.cur, w.k FROM class_of_walk w JOIN class_key c ON c.key = w.cur
WHERE instr(w.cur, '.<base>.') = 0
-- index: start

-- ===================================================================== link closure
-- stage link_reach
-- Every name a link may read, from the summaries' edge names: a superset of link_python's
-- recursion (closure.rs:1685-1887), each step with its least hop cost; names past 19 hops in
-- cannot yield a definition (MAX_HOPS 16 plus at most three unchecked hops).
WITH RECURSIVE r(name, d) AS (
  SELECT DISTINCT name, 0 FROM s_edge
  UNION
  -- `<bare>` / `<unqualified>` fallbacks (closure.rs:1736-1742, 1755-1761)
  SELECT substr(r.name, instr(r.name, '.<bare>.') + 8 + instr(substr(r.name, instr(r.name, '.<bare>.') + 8), '=>') + 1), r.d + 1
  FROM r WHERE r.d < 19 AND instr(r.name, '.<bare>.') > 0 AND instr(substr(r.name, instr(r.name, '.<bare>.') + 8), '=>') > 0
  UNION
  SELECT substr(r.name, instr(r.name, '.<unqualified>.') + 15 + instr(substr(r.name, instr(r.name, '.<unqualified>.') + 15), '=>') + 1), r.d + 1
  FROM r WHERE r.d < 19 AND instr(r.name, '.<unqualified>.') > 0 AND instr(substr(r.name, instr(r.name, '.<unqualified>.') + 15), '=>') > 0
  UNION
  -- late `^` (closure.rs:1794-1798)
  SELECT rtrim(rtrim(r.name, '^'), '.'), r.d + 1 FROM r WHERE r.d < 19 AND substr(r.name, -1) = '^'
  UNION
  -- aliases of the name itself (member, closure.rs:1987-1990)
  SELECT a.target, r.d + 1 FROM r JOIN aliases a ON a.key = r.name WHERE r.d < 19
  UNION
  -- `<declared>` / `<imported>` of a `<bare>` name (closure.rs:1718-1735)
  SELECT a.target, r.d + 2 FROM r JOIN aliases a
    ON a.key IN (substr(r.name, 1, instr(r.name, '.<bare>.') - 1) || '.<declared>.' || substr(substr(r.name, instr(r.name, '.<bare>.') + 8), 1, instr(substr(r.name, instr(r.name, '.<bare>.') + 8), '=>') - 1),
                 substr(r.name, 1, instr(r.name, '.<bare>.') - 1) || '.<imported>.' || substr(substr(r.name, instr(r.name, '.<bare>.') + 8), 1, instr(substr(r.name, instr(r.name, '.<bare>.') + 8), '=>') - 1))
  WHERE r.d < 18 AND instr(r.name, '.<bare>.') > 0 AND instr(substr(r.name, instr(r.name, '.<bare>.') + 8), '=>') > 0
  UNION
  -- the own member of an `<unqualified>` name (closure.rs:1748-1754)
  SELECT a.target, r.d + 2 FROM r JOIN aliases a
    ON a.key = substr(r.name, 1, instr(r.name, '.<unqualified>.') - 1) || '.' || substr(substr(r.name, instr(r.name, '.<unqualified>.') + 15), 1, instr(substr(r.name, instr(r.name, '.<unqualified>.') + 15), '=>') - 1)
  WHERE r.d < 18 AND instr(r.name, '.<unqualified>.') > 0 AND instr(substr(r.name, instr(r.name, '.<unqualified>.') + 15), '=>') > 0
  UNION
  -- the declared-import hop (closure.rs:1814-1824)
  SELECT a.target, r.d + 2 FROM r JOIN aliases a
    ON a.key = rx_extract(r.name, '^(.*)\.[^.]*$', 1) || '.<imported>.' || rx_extract(r.name, '[^.]*$', 0)
  WHERE r.d < 18 AND instr(r.name, '.') > 0
  UNION
  -- prefix aliases (closure.rs:1831-1851)
  SELECT a.target || substr(r.name, d.i), r.d + 1
  FROM r JOIN digit d ON d.i < length(r.name) AND substr(r.name, d.i, 1) = '.'
  JOIN aliases a ON a.key = substr(r.name, 1, d.i - 1)
  WHERE r.d < 19
  UNION
  -- constructor fields of a `<field>` name (closure.rs:1701-1712, 1890-1956)
  SELECT x.cls || '.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') + 1), r.d + 3
  FROM r JOIN reference_order o ON o.root = substr(r.name, 1, instr(r.name, '.<field>.') - 1)
  JOIN field_alias a ON a.key IN (
      o.cls || '.<init-parameter>.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1),
      o.cls || '.<init-parameter>.' || ltrim(substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1), '_'),
      o.cls || '.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1))
  JOIN class_of x ON x.start = a.target
  WHERE r.d < 17 AND instr(r.name, '.<field>.') > 0 AND instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') > 0
  UNION
  -- ... and of an annotated target (`<annotation>.X`, reference_type of the caller's parent)
  SELECT t.cls || '.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') + 1), r.d + 3
  FROM r JOIN reference_order o ON o.root = substr(r.name, 1, instr(r.name, '.<field>.') - 1)
  JOIN field_alias a ON a.key IN (
      o.cls || '.<init-parameter>.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1),
      o.cls || '.<init-parameter>.' || ltrim(substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1), '_'),
      o.cls || '.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1))
  JOIN type_key t ON substr(a.target, 1, 13) = '<annotation>.'
       AND t.key IN (rx_extract(substr(r.name, 1, instr(r.name, '.<field>.') - 1), '^(.*)\.[^.]*$', 1) || '.<imported>.' || substr(a.target, 14),
                     rx_extract(substr(r.name, 1, instr(r.name, '.<field>.') - 1), '^(.*)\.[^.]*$', 1) || '.<class>.' || substr(a.target, 14))
  WHERE r.d < 17 AND instr(r.name, '.<field>.') > 0 AND instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') > 0
  UNION
  -- constructor fields after a class prefix (closure.rs:1852-1859)
  SELECT x.cls || '.' || substr(r.name, instr(substr(r.name, d.i + 1), '.') + d.i + 1), r.d + 3
  FROM r JOIN digit d ON d.i < length(r.name) AND substr(r.name, d.i, 1) = '.'
  JOIN class_key c ON c.key = substr(r.name, 1, d.i - 1)
  JOIN reference_order o ON o.root = c.key
  JOIN field_alias a ON a.key IN (
      o.cls || '.<init-parameter>.' || substr(r.name, d.i + 1, instr(substr(r.name, d.i + 1), '.') - 1),
      o.cls || '.<init-parameter>.' || ltrim(substr(r.name, d.i + 1, instr(substr(r.name, d.i + 1), '.') - 1), '_'),
      o.cls || '.' || substr(r.name, d.i + 1, instr(substr(r.name, d.i + 1), '.') - 1))
  JOIN class_of x ON x.start = a.target
  WHERE r.d < 17 AND instr(substr(r.name, d.i + 1), '.') > 0
  UNION
  SELECT t.cls || '.' || substr(r.name, instr(substr(r.name, d.i + 1), '.') + d.i + 1), r.d + 3
  FROM r JOIN digit d ON d.i < length(r.name) AND substr(r.name, d.i, 1) = '.'
  JOIN class_key c ON c.key = substr(r.name, 1, d.i - 1)
  JOIN reference_order o ON o.root = c.key
  JOIN field_alias a ON a.key IN (
      o.cls || '.<init-parameter>.' || substr(r.name, d.i + 1, instr(substr(r.name, d.i + 1), '.') - 1),
      o.cls || '.<init-parameter>.' || ltrim(substr(r.name, d.i + 1, instr(substr(r.name, d.i + 1), '.') - 1), '_'),
      o.cls || '.' || substr(r.name, d.i + 1, instr(substr(r.name, d.i + 1), '.') - 1))
  JOIN type_key t ON substr(a.target, 1, 13) = '<annotation>.'
       AND t.key IN (rx_extract(c.key, '^(.*)\.[^.]*$', 1) || '.<imported>.' || substr(a.target, 14),
                     rx_extract(c.key, '^(.*)\.[^.]*$', 1) || '.<class>.' || substr(a.target, 14))
  WHERE r.d < 17 AND instr(substr(r.name, d.i + 1), '.') > 0
)
SELECT DISTINCT name FROM r
-- index: name

-- ===================================================================== link_python, exact
-- stage link_form
-- link_python's string guards in their order (closure.rs:1686-1800); the first that holds decides
-- the branch: 1 `<type>`, 2 `<field>`, 3 `<bare>`, 4 `<unqualified>`, 5 `<self>`, 6 `<super>`,
-- 7 late `^`, 8 a plain name. `a`, `b`, `c` are the branch's parts.
SELECT name, form,
       CASE WHEN form IN (1, 2, 3, 4) THEN substr(name, 1, at - 1)
            WHEN form IN (5, 6) THEN substr(name, 1, at - 1)
            WHEN form = 7 THEN rtrim(rtrim(name, '^'), '.')
            ELSE rx_extract(name, '^(.*)\.[^.]*$', 1) END AS a,
       CASE WHEN form IN (1, 2) THEN substr(rest, 1, instr(rest, '.') - 1)
            WHEN form IN (3, 4) THEN substr(rest, 1, instr(rest, '=>') - 1)
            WHEN form IN (5, 6) THEN rest
            WHEN form = 8 AND instr(name, '.') > 0 THEN rx_extract(name, '[^.]*$', 0) END AS b,
       CASE WHEN form IN (1, 2) THEN substr(rest, instr(rest, '.') + 1)
            WHEN form IN (3, 4) THEN substr(rest, instr(rest, '=>') + 2) END AS c
FROM (
  SELECT name, form,
         CASE form WHEN 1 THEN pt WHEN 2 THEN pf WHEN 3 THEN pb WHEN 4 THEN pu WHEN 5 THEN ps WHEN 6 THEN pp END AS at,
         CASE form WHEN 1 THEN substr(name, pt + 8) WHEN 2 THEN substr(name, pf + 9) WHEN 3 THEN substr(name, pb + 8)
                   WHEN 4 THEN substr(name, pu + 15) WHEN 5 THEN substr(name, ps + 8) WHEN 6 THEN substr(name, pp + 9) END AS rest
  FROM (
    SELECT name, pt, pf, pb, pu, ps, pp,
           CASE WHEN pt > 0 AND instr(substr(name, pt + 8), '.') > 0 THEN 1
                WHEN pf > 0 AND instr(substr(name, pf + 9), '.') > 0 THEN 2
                WHEN pu = 0 AND pb > 0 AND instr(substr(name, pb + 8), '=>') > 0 THEN 3
                WHEN pu > 0 AND instr(substr(name, pu + 15), '=>') > 0 THEN 4
                WHEN ps > 0 THEN 5
                WHEN pp > 0 THEN 6
                WHEN substr(name, -1) = '^' THEN 7
                ELSE 8 END AS form
    FROM (SELECT name, instr(name, '.<type>.') AS pt, instr(name, '.<field>.') AS pf, instr(name, '.<bare>.') AS pb,
                 instr(name, '.<unqualified>.') AS pu, instr(name, '.<self>.') AS ps, instr(name, '.<super>.') AS pp
          FROM link_reach)))
-- index: name
-- index: form

-- stage link_plain
-- A plain name (closure.rs:1801-1885): 8 its definitions; 9 the declared-import hop; 10 `member`
-- (aliases or an opaque binding); 11 the longest-prefix scan.
SELECT f.name, f.a, f.b,
       CASE WHEN EXISTS (SELECT 1 FROM by_name x WHERE x.key = f.name) THEN 8
            WHEN f.a IS NOT NULL AND EXISTS (SELECT 1 FROM aliases x WHERE x.key = f.a || '.<imported>.' || f.b) THEN 9
            WHEN EXISTS (SELECT 1 FROM aliases x WHERE x.key = f.name) OR EXISTS (SELECT 1 FROM opaque x WHERE x.key = f.name) THEN 10
            ELSE 11 END AS step
FROM link_form f WHERE f.form = 8
-- index: name
-- index: step

-- stage prefix_match
-- closure.rs:1829-1885: each proper prefix (`lineage().skip(1)`) that ends the scan: aliases with a
-- target that is not the prefix itself nor a workspace module without a definition of that name,
-- a definition, an opaque binding, a module.
SELECT x.name, x.i, x.p, x.rest, x.kind FROM (
  SELECT l.name, d.i, substr(l.name, 1, d.i - 1) AS p, substr(l.name, d.i) AS rest,
         CASE WHEN EXISTS (SELECT 1 FROM aliases a WHERE a.key = substr(l.name, 1, d.i - 1) AND a.target <> a.key
                             AND (NOT EXISTS (SELECT 1 FROM modules m WHERE m.name = a.target)
                                  OR EXISTS (SELECT 1 FROM by_name b WHERE b.key = a.target))) THEN 'alias'
              WHEN EXISTS (SELECT 1 FROM by_name b WHERE b.key = substr(l.name, 1, d.i - 1)) THEN 'def'
              WHEN EXISTS (SELECT 1 FROM opaque o WHERE o.key = substr(l.name, 1, d.i - 1)) THEN 'opaque'
              WHEN EXISTS (SELECT 1 FROM modules m WHERE m.name = substr(l.name, 1, d.i - 1)) THEN 'module' END AS kind
  FROM link_plain l JOIN digit d ON d.i < length(l.name) AND substr(l.name, d.i, 1) = '.'
  WHERE l.step = 11) x
WHERE x.kind IS NOT NULL
-- index: name, i

-- stage prefix_first
-- The longest such prefix.
SELECT m.name, m.p, m.rest, m.kind,
       CASE WHEN m.kind = 'def' AND EXISTS (SELECT 1 FROM class_key c WHERE c.key = m.p) THEN 1 ELSE 0 END AS class
FROM prefix_match m WHERE NOT EXISTS (SELECT 1 FROM prefix_match o WHERE o.name = m.name AND o.i > m.i)
-- index: name

-- stage field_demand
-- reference_field calls (closure.rs:1705, 1858): `<field>` names, and a class prefix followed by
-- `field.member`.
SELECT DISTINCT a AS cls, b AS field, c AS member FROM link_form WHERE form = 2
UNION
SELECT p, substr(rest, 2, instr(substr(rest, 2), '.') - 1), substr(rest, instr(substr(rest, 2), '.') + 2)
FROM prefix_first WHERE kind = 'def' AND class = 1 AND instr(substr(rest, 2), '.') > 0
-- index: cls, field

-- stage field_candidate
-- reference_field (closure.rs:1890-1926) per base in reference_order: the alias key it returns
-- through, if any: `{base}.<init-parameter>.{param}` for param in constructor_names(field) =
-- [field, field with `^_+` removed] (contracts.rs:144) unless all its targets are
-- `<unannotated>` (then `break` to the field), else `{base}.{field}`.
SELECT d.cls, d.field, o.dist, o.seq,
       CASE WHEN EXISTS (SELECT 1 FROM aliases a WHERE a.key = o.cls || '.<init-parameter>.' || d.field) THEN
              CASE WHEN EXISTS (SELECT 1 FROM aliases a WHERE a.key = o.cls || '.<init-parameter>.' || d.field AND a.target <> '<unannotated>')
                   THEN o.cls || '.<init-parameter>.' || d.field
                   WHEN EXISTS (SELECT 1 FROM aliases a WHERE a.key = o.cls || '.' || d.field) THEN o.cls || '.' || d.field END
            WHEN ltrim(d.field, '_') <> d.field AND EXISTS (SELECT 1 FROM aliases a WHERE a.key = o.cls || '.<init-parameter>.' || ltrim(d.field, '_')) THEN
              CASE WHEN EXISTS (SELECT 1 FROM aliases a WHERE a.key = o.cls || '.<init-parameter>.' || ltrim(d.field, '_') AND a.target <> '<unannotated>')
                   THEN o.cls || '.<init-parameter>.' || ltrim(d.field, '_')
                   WHEN EXISTS (SELECT 1 FROM aliases a WHERE a.key = o.cls || '.' || d.field) THEN o.cls || '.' || d.field END
            WHEN EXISTS (SELECT 1 FROM aliases a WHERE a.key = o.cls || '.' || d.field) THEN o.cls || '.' || d.field END AS hit
FROM (SELECT DISTINCT cls, field FROM field_demand) d JOIN reference_order o ON o.root = d.cls
-- index: cls, field

-- stage field_hit
-- The first base in order with a key.
SELECT c.cls, rx_extract(c.cls, '^(.*)\.[^.]*$', 1) AS parent, c.field, c.hit FROM field_candidate c
WHERE c.hit IS NOT NULL AND NOT EXISTS (
  SELECT 1 FROM field_candidate o WHERE o.cls = c.cls AND o.field = c.field AND o.hit IS NOT NULL
    AND (o.dist < c.dist OR (o.dist = c.dist AND o.seq < c.seq)))
-- index: cls, field

-- stage field_class
-- reference_type_members (closure.rs:1928-1956), at hops H = h + 1 of the caller's link: each
-- target `<annotation>.X` is reference_type(parent(class), X, H) — through `<imported>` before
-- `<class>`, each found iff H + 1 + k <= 16 — other targets class_of(target, H + 1). `lo`/`hi`
-- bound the caller's hops h.
SELECT h.cls, h.field, t0.cls AS member_class, 0 AS lo, 14 - t0.k AS hi
FROM field_hit h JOIN aliases a ON a.key = h.hit
JOIN type_unique t0 ON t0.key = h.parent || '.<imported>.' || substr(a.target, 14)
WHERE substr(a.target, 1, 13) = '<annotation>.'
UNION
SELECT h.cls, h.field, t1.cls,
       CASE WHEN t0.key IS NULL THEN 0 ELSE 15 - t0.k END, 14 - t1.k
FROM field_hit h JOIN aliases a ON a.key = h.hit
JOIN type_unique t1 ON t1.key = h.parent || '.<class>.' || substr(a.target, 14)
LEFT JOIN type_key t0 ON t0.key = h.parent || '.<imported>.' || substr(a.target, 14)
WHERE substr(a.target, 1, 13) = '<annotation>.'
UNION
SELECT h.cls, h.field, x.cls, 0, 14 - x.k
FROM field_hit h JOIN aliases a ON a.key = h.hit JOIN class_of x ON x.start = a.target
WHERE substr(a.target, 1, 13) <> '<annotation>.'

-- stage type_demand
-- reference_type for `<type>` names (closure.rs:1686-1699), at the name's own hops h:
-- `<imported>` found iff h + 1 + k <= 16, else `<class>`.
SELECT f.name, t0.cls, 0 AS lo, 15 - t0.k AS hi, f.c AS attr
FROM link_form f JOIN type_unique t0 ON t0.key = f.a || '.<imported>.' || f.b
WHERE f.form = 1
UNION
SELECT f.name, t1.cls, CASE WHEN t0.key IS NULL THEN 0 ELSE 16 - t0.k END, 15 - t1.k, f.c
FROM link_form f JOIN type_unique t1 ON t1.key = f.a || '.<class>.' || f.b
LEFT JOIN type_key t0 ON t0.key = f.a || '.<imported>.' || f.b
WHERE f.form = 1

-- stage method_demand
-- reference_method (closure.rs:2137-2161) lookups: the class, the attribute, whether order[0] is
-- skipped (`<super>`).
SELECT DISTINCT cls AS root, attr, 0 AS skip FROM type_demand
UNION SELECT a, b, 0 FROM link_form WHERE form = 5
UNION SELECT a, b, 1 FROM link_form WHERE form = 6
UNION SELECT p, substr(rest, 2), 0 FROM prefix_first WHERE kind = 'def' AND class = 1 AND instr(substr(rest, 2), '.') = 0
-- index: root, attr, skip

-- stage method_candidate
SELECT d.root, d.attr, d.skip, o.cls, o.dist, o.seq
FROM method_demand d JOIN reference_order o ON o.root = d.root AND (d.skip = 0 OR o.dist > 0)
WHERE EXISTS (SELECT 1 FROM method_def m WHERE m.key = o.cls || '.' || d.attr)
-- index: root, attr, skip

-- stage method_first
-- The first class in BFS order (dist, seq) with a function `{class}.{attr}`.
SELECT c.root, c.attr, c.skip, c.cls FROM method_candidate c
WHERE NOT EXISTS (SELECT 1 FROM method_candidate o WHERE o.root = c.root AND o.attr = c.attr AND o.skip = c.skip
                    AND (o.dist < c.dist OR (o.dist = c.dist AND o.seq < c.seq)))
-- index: root, attr, skip

-- stage compose_candidate
-- `<self>` composition (closure.rs:1771-1782): no class in order has it; for each subclass (by
-- text), each other base of it (declared order), that base's reference_method.
SELECT f.name, s.child, b.ord, o.dist, o.seq, o.cls
FROM link_form f JOIN subclass s ON s.cls = f.a
JOIN bases b ON b.cls = s.child AND b.base <> f.a
JOIN reference_order o ON o.root = b.base
WHERE f.form = 5
  AND NOT EXISTS (SELECT 1 FROM method_first m WHERE m.root = f.a AND m.attr = f.b AND m.skip = 0)
  AND EXISTS (SELECT 1 FROM method_def m WHERE m.key = o.cls || '.' || f.b)
-- index: name

-- stage compose_first
SELECT c.name, c.cls FROM compose_candidate c
WHERE NOT EXISTS (SELECT 1 FROM compose_candidate o WHERE o.name = c.name
                    AND (o.child < c.child OR (o.child = c.child AND (o.ord < c.ord OR (o.ord = c.ord
                         AND (o.dist < c.dist OR (o.dist = c.dist AND o.seq < c.seq)))))))

-- stage link_term
-- Definitions a name links to directly, for its hops h in [lo, hi].
-- `<type>` (closure.rs:1690-1699)
SELECT t.name, m.path, m.idx, t.lo, t.hi FROM type_demand t
JOIN method_first f ON f.root = t.cls AND f.attr = t.attr AND f.skip = 0
JOIN method_def m ON m.key = f.cls || '.' || t.attr
UNION
-- `<unqualified>` whose own member is a function: member(own) (closure.rs:1748-1754, 1961-1994)
SELECT l.name, b.path, b.idx, 0, 99 FROM link_form l JOIN by_name b ON b.key = l.a || '.' || l.b
WHERE l.form = 4 AND EXISTS (SELECT 1 FROM method_def m WHERE m.key = l.a || '.' || l.b)
UNION
-- `<self>` (closure.rs:1763-1783)
SELECT l.name, m.path, m.idx, 0, 99 FROM link_form l
JOIN method_first f ON f.root = l.a AND f.attr = l.b AND f.skip = 0
JOIN method_def m ON m.key = f.cls || '.' || l.b
WHERE l.form = 5
UNION
SELECT c.name, m.path, m.idx, 0, 99 FROM compose_first c JOIN link_form l ON l.name = c.name
JOIN method_def m ON m.key = c.cls || '.' || l.b
UNION
-- `<super>` (closure.rs:1785-1793)
SELECT l.name, m.path, m.idx, 0, 99 FROM link_form l
JOIN method_first f ON f.root = l.a AND f.attr = l.b AND f.skip = 1
JOIN method_def m ON m.key = f.cls || '.' || l.b
WHERE l.form = 6
UNION
-- every definition of the exact name (closure.rs:1801-1813)
SELECT l.name, b.path, b.idx, 0, 99 FROM link_plain l JOIN by_name b ON b.key = l.name WHERE l.step = 8
UNION
-- a class prefix's method (class_member, closure.rs:1860-1864, 2086-2098)
SELECT p.name, m.path, m.idx, 0, 99 FROM prefix_first p
JOIN method_first f ON f.root = p.p AND f.attr = substr(p.rest, 2) AND f.skip = 0
JOIN method_def m ON m.key = f.cls || '.' || substr(p.rest, 2)
WHERE p.kind = 'def' AND p.class = 1 AND instr(substr(p.rest, 2), '.') = 0
-- index: name

-- stage link_kid
-- The names a name's link unions (`union`, closure.rs:2503: each at hops + 1 through link_name,
-- whose check `hops > 16` makes it empty: chk = 1) or follows directly (link_python, no check:
-- chk = 0), `delta` hops on, for the caller's hops h in [lo, hi].
-- `<field>` (closure.rs:1701-1712): reference_field at h + 1, members at h + 3
SELECT f.name, x.member_class || '.' || f.c AS kid, 3 AS delta, 1 AS chk, x.lo, x.hi
FROM link_form f JOIN field_class x ON x.cls = f.a AND x.field = f.b WHERE f.form = 2
UNION
-- `<bare>` (closure.rs:1713-1743)
SELECT f.name, a.target, 2, 1, 0, 99 FROM link_form f JOIN aliases a ON a.key = f.a || '.<declared>.' || f.b
WHERE f.form = 3
UNION
SELECT f.name, a.target, 2, 1, 0, 99 FROM link_form f JOIN aliases a ON a.key = f.a || '.<imported>.' || f.b
WHERE f.form = 3 AND NOT EXISTS (SELECT 1 FROM aliases x WHERE x.key = f.a || '.<declared>.' || f.b)
UNION
SELECT f.name, f.c, 1, 0, 0, 99 FROM link_form f
WHERE f.form = 3 AND f.c <> ''
  AND NOT EXISTS (SELECT 1 FROM aliases x WHERE x.key = f.a || '.<declared>.' || f.b)
  AND NOT EXISTS (SELECT 1 FROM aliases x WHERE x.key = f.a || '.<imported>.' || f.b)
UNION
-- `<unqualified>` (closure.rs:1744-1762): member(own) at h + 1, its aliases at h + 2; else the fallback
SELECT f.name, a.target, 2, 1, 0, 99 FROM link_form f JOIN aliases a ON a.key = f.a || '.' || f.b
WHERE f.form = 4 AND EXISTS (SELECT 1 FROM method_def m WHERE m.key = f.a || '.' || f.b)
UNION
SELECT f.name, f.c, 1, 0, 0, 99 FROM link_form f
WHERE f.form = 4 AND f.c <> '' AND NOT EXISTS (SELECT 1 FROM method_def m WHERE m.key = f.a || '.' || f.b)
UNION
-- late `^` (closure.rs:1794-1798)
SELECT f.name, f.a, 1, 0, 0, 99 FROM link_form f WHERE f.form = 7
UNION
-- the declared-import hop (closure.rs:1814-1824)
SELECT l.name, a.target, 2, 1, 0, 99 FROM link_plain l JOIN aliases a ON a.key = l.a || '.<imported>.' || l.b
WHERE l.step = 9
UNION
-- member's aliases (closure.rs:1987-1990)
SELECT l.name, a.target, 1, 1, 0, 99 FROM link_plain l JOIN aliases a ON a.key = l.name WHERE l.step = 10
UNION
-- a prefix's aliases (closure.rs:1831-1851)
SELECT p.name, a.target || p.rest, 1, 1, 0, 99 FROM prefix_first p JOIN aliases a ON a.key = p.p
WHERE p.kind = 'alias' AND a.target <> a.key
  AND (NOT EXISTS (SELECT 1 FROM modules m WHERE m.name = a.target) OR EXISTS (SELECT 1 FROM by_name b WHERE b.key = a.target))
UNION
-- a class prefix's constructor field (closure.rs:1852-1859)
SELECT p.name, x.member_class || '.' || substr(p.rest, instr(substr(p.rest, 2), '.') + 2), 3, 1, x.lo, x.hi
FROM prefix_first p JOIN field_class x ON x.cls = p.p AND x.field = substr(p.rest, 2, instr(substr(p.rest, 2), '.') - 1)
WHERE p.kind = 'def' AND p.class = 1 AND instr(substr(p.rest, 2), '.') > 0
-- index: kid

-- stage link_result
-- link_name(name, h) (closure.rs:1449): its own definitions and its kids', each a definition for
-- the h in [lo, hi] it holds at (a kid's range shifted by delta; a checked kid needs
-- h + delta <= 16). The ranges shrink along every step, so the recursion ends.
WITH RECURSIVE res(name, path, idx, lo, hi) AS (
  SELECT name, path, idx, lo, hi FROM link_term WHERE lo <= hi
  UNION
  SELECT k.name, r.path, r.idx,
         CASE WHEN k.lo >= r.lo - k.delta THEN k.lo ELSE r.lo - k.delta END,
         CASE WHEN k.chk = 1 AND 16 - k.delta < CASE WHEN k.hi < r.hi - k.delta THEN k.hi ELSE r.hi - k.delta END
              THEN 16 - k.delta
              ELSE CASE WHEN k.hi < r.hi - k.delta THEN k.hi ELSE r.hi - k.delta END END
  FROM res r JOIN link_kid k ON k.kid = r.name
  WHERE CASE WHEN k.lo >= r.lo - k.delta THEN k.lo ELSE r.lo - k.delta END
        <= CASE WHEN k.chk = 1 AND 16 - k.delta < CASE WHEN k.hi < r.hi - k.delta THEN k.hi ELSE r.hi - k.delta END
                THEN 16 - k.delta
                ELSE CASE WHEN k.hi < r.hi - k.delta THEN k.hi ELSE r.hi - k.delta END END
)
SELECT DISTINCT name, path, idx FROM res WHERE lo = 0
-- index: name

-- ===================================================================== output
-- stage reference_graph_incomplete
-- relational.rs:636-643: an open summary (syntax error, no scope table, budget) of an in-scope
-- file fails the clause; one row then, and no edges.
SELECT 'The native reference graph is incomplete' AS reason, count(*) AS open_files
FROM s_open o JOIN in_scope s ON s.path = o.path WHERE o.open <> 0
HAVING count(*) > 0

-- stage reference_edges
-- resolve_edges (closure.rs:1064-1078) and reference_rows (closure.rs:548-583) as the relational
-- table (relational.rs:633-678): each (owner, target) pair once, by source path in scope.
SELECT DISTINCT e.path AS __path, sd.start AS __start, sd.stop AS __end,
       r.path AS target_path, td.start AS target_start, td.stop AS target_end,
       sd.name AS source_name, td.name AS target_name, (td.flags / 2) % 2 AS target_class
FROM s_edge e JOIN in_scope s ON s.path = e.path
JOIN link_result r ON r.name = e.name
JOIN s_def sd ON sd.path = e.path AND sd.idx = e.idx
JOIN s_def td ON td.path = r.path AND td.idx = r.idx
WHERE NOT EXISTS (SELECT 1 FROM reference_graph_incomplete)

-- ===================================================================== refusals
-- stage linker_refusal
-- Where native's answer depends on slot or iteration order, or the export cannot be exact: the
-- files involved, by construct. Never a guess: the stages above drop these choices.
SELECT path, construct FROM refused_path
UNION
SELECT s.path, 'class_of <base>: first of several alias targets that are classes (alias entry order, closure.rs:2296)'
FROM supers s JOIN base_class b ON b.target = s.target WHERE b.several > 1
UNION
SELECT s.path, 'reference_order: one class key''s bases from several files (slot order, closure.rs:2108)'
FROM supers s WHERE (SELECT count(DISTINCT x.path) FROM supers x WHERE x.cls = s.cls) > 1
UNION
SELECT b.path, 'own_name: first of several definitions under one key (slot order, closure.rs:2497)'
FROM own_name o JOIN by_name b ON b.key = o.key
WHERE o.names > 1 AND (EXISTS (SELECT 1 FROM supers s WHERE s.cls = o.key) OR EXISTS (SELECT 1 FROM base_class c WHERE c.cls = o.key))
UNION
SELECT a.path, 'reference_type / reference_fq_class: first of several alias targets that are classes (alias entry order, closure.rs:2245, 2262)'
FROM type_key t JOIN aliases a ON a.key = t.key WHERE t.several > 1
UNION
SELECT s.path, 'reference_order: class hierarchy deeper than 64' FROM order_dist d JOIN supers s ON s.cls = d.root WHERE d.dist >= 64
UNION
SELECT e.path, 'link: name longer than 2048 characters' FROM s_edge e WHERE length(e.name) > 2048
UNION
SELECT e.path, 'reference_order: root not among class_root' FROM s_edge e JOIN link_form f ON f.name = e.name
WHERE f.form IN (2, 5, 6) AND NOT EXISTS (SELECT 1 FROM class_root c WHERE c.cls = f.a)
