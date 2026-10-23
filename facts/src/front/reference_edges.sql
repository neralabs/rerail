-- Native's Python reference graph (`reference_edges`, refs mode) as SQL stages: the definition
-- of the relation `reference_edges` that a relational clause declaring `references` reads
-- (front/rule.rs compiles these stages ahead of the clause's own, after module_path.sql's;
-- INVARIANTS I2).
-- Inputs: module_path.sql's stages (`in_scope`, `module_path`, `module_segment`, `file_name`)
-- and, per content (SCHEMA § 3.4), each row carrying the content's `__path`:
--   cgn_<rel>(__path, variant, ...)   native's call summary of the content under each symbolic
--                                     variant, after `as_references` with the clause's links,
--                                     in the reading the path selects (`.pyi`: stub); empty for
--                                     contents outside the rule's scope
-- Everything else is membership-dependent.
-- Native paths below are relative to native/src/code_rule_dsl/. Each stage is one statement;
-- `-- index:` lines are speed hints for the SQLite harness (linker_check.py) only.

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
SELECT mb.path, m.n,
       CASE WHEN m.package = 0 THEN 'unpackaged'
            WHEN m.n > 1 AND l.name = '__init__' THEN 'init'
            ELSE 'module' END AS variant,
       CASE WHEN m.package = 0 THEN 1 ELSE 17 END AS symlen
FROM in_scope mb JOIN module_path m ON m.path = mb.path
LEFT JOIN module_segment l ON l.path = m.path AND l.i = m.n - 1
-- index: path
-- index: path, variant

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
  SELECT __path, variant, name AS raw FROM cgn_def
  UNION ALL SELECT __path, variant, name FROM cgn_edge
  UNION ALL SELECT __path, variant, "key" FROM cgn_alias
  UNION ALL SELECT __path, variant, target FROM cgn_alias
  UNION ALL SELECT __path, variant, name FROM cgn_opaque
  UNION ALL SELECT __path, variant, name FROM cgn_module
  UNION ALL SELECT __path, variant, "key" FROM cgn_reference_alias WHERE instr("key", '.<base-slot>.') > 0
  UNION ALL SELECT __path, variant, target FROM cgn_reference_alias WHERE instr("key", '.<base-slot>.') > 0
) x ON x.__path = v.path AND x.variant = v.variant
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
SELECT v.path, v.variant FROM file_variant v
WHERE NOT EXISTS (SELECT 1 FROM refused_path r WHERE r.path = v.path)
-- index: path
-- index: path, variant

-- ===================================================================== substituted summaries
-- stage s_def
-- summary.rs:100 Def, after as_references (summary.rs:198-208: OPEN cleared; SHADOWED cleared on
-- the last def of a qualname) — as exported.
SELECT f.path, CAST(d.idx AS INTEGER) AS idx, coalesce(n.name, d.name) AS name,
       CAST(d.start AS INTEGER) AS start, CAST(d."end" AS INTEGER) AS stop, CAST(d.flags AS INTEGER) AS flags
FROM linked_file f JOIN cgn_def d ON d.__path = f.path AND d.variant = f.variant
LEFT JOIN name_sub n ON n.path = f.path AND n.raw = d.name
-- index: path, idx
-- index: path, start
-- index: name

-- stage s_edge
-- summary.edges in refs mode: references plus siblingLinks rows (summary.rs:128, 182-199).
SELECT DISTINCT f.path, CAST(e."def" AS INTEGER) AS idx, coalesce(n.name, e.name) AS name
FROM linked_file f JOIN cgn_edge e ON e.__path = f.path AND e.variant = f.variant
LEFT JOIN name_sub n ON n.path = f.path AND n.raw = e.name
-- index: name
-- index: path, idx

-- stage s_alias
-- summary.aliases in refs mode (rebuilt from reference_aliases, summary.rs:161-180).
SELECT DISTINCT f.path, coalesce(k.name, a."key") AS key, coalesce(t.name, a.target) AS target
FROM linked_file f JOIN cgn_alias a ON a.__path = f.path AND a.variant = f.variant
LEFT JOIN name_sub k ON k.path = f.path AND k.raw = a."key"
LEFT JOIN name_sub t ON t.path = f.path AND t.raw = a.target
-- index: path, key

-- stage s_opaque
SELECT DISTINCT f.path, coalesce(n.name, o.name) AS name
FROM linked_file f JOIN cgn_opaque o ON o.__path = f.path AND o.variant = f.variant
LEFT JOIN name_sub n ON n.path = f.path AND n.raw = o.name

-- stage s_module
-- summary.module: ModulePath::import_name (module_path.rs).
SELECT f.path, coalesce(n.name, m.name) AS name
FROM linked_file f JOIN cgn_module m ON m.__path = f.path AND m.variant = f.variant
LEFT JOIN name_sub n ON n.path = f.path AND n.raw = m.name
-- index: path

-- stage s_open
SELECT f.path, CAST(o."open" AS INTEGER) AS open
FROM linked_file f JOIN cgn_open o ON o.__path = f.path AND o.variant = f.variant

-- stage s_base_slot
-- `{class}.<base-slot>.{classStart}:{ordinal}` → `{module}.<base>.{short}` (summary.rs:1087-1095):
-- the ordinal is the zero-padded position among the file's reference aliases, so it orders a
-- class's bases as declared.
SELECT DISTINCT f.path, coalesce(k.name, a."key") AS key, coalesce(t.name, a.target) AS target
FROM linked_file f JOIN cgn_reference_alias a ON a.__path = f.path AND a.variant = f.variant
LEFT JOIN name_sub k ON k.path = f.path AND k.raw = a."key"
LEFT JOIN name_sub t ON t.path = f.path AND t.raw = a.target
WHERE instr(a."key", '.<base-slot>.') > 0

-- stage last_class_def
-- The last class def of each name per file (summary.rs:139-144 `class_starts`).
SELECT d.path, d.name, d.start FROM s_def d
JOIN (SELECT path, name, max(idx) AS idx FROM s_def WHERE (flags / 2) % 2 = 1 GROUP BY path, name) l
  ON l.path = d.path AND l.name = d.name AND l.idx = d.idx
-- index: path, name

-- stage s_super
-- as_references (summary.rs:139-144, 171-176): a base slot whose class start is the start of the
-- last class def of that name becomes a BASE super, in slot order.
SELECT b.path, substr(b.key, 1, instr(b.key, '.<base-slot>.') - 1) AS cls, b.target,
       substr(b.key, instr(b.key, '.<base-slot>.') + 13 + instr(substr(b.key, instr(b.key, '.<base-slot>.') + 13), ':')) AS ord
FROM s_base_slot b
JOIN last_class_def c ON c.path = b.path AND c.name = substr(b.key, 1, instr(b.key, '.<base-slot>.') - 1)
WHERE instr(substr(b.key, instr(b.key, '.<base-slot>.') + 13), ':') > 0
  AND CAST(substr(b.key, instr(b.key, '.<base-slot>.') + 13,
                  instr(substr(b.key, instr(b.key, '.<base-slot>.') + 13), ':') - 1) AS INTEGER) = c.start

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
SELECT short, min(name) AS name
FROM (SELECT rx_extract(d.name, '[^.]*$', 0) AS short, d.name FROM s_def d WHERE (d.flags / 2) % 2 = 1)
GROUP BY short HAVING count(*) = 1
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
SELECT t.target, CASE WHEN m.target IS NOT NULL THEN m.cls ELSE r.name END AS cls, coalesce(m.several, 0) AS several
FROM (SELECT DISTINCT target FROM supers WHERE instr(target, '.<base>.') > 0) t
LEFT JOIN (SELECT b.target, min(b.cls) AS cls, count(DISTINCT b.cls) AS several
           FROM base_target b JOIN (SELECT target, min(rank) AS rank FROM base_target GROUP BY target) x
             ON x.target = b.target AND x.rank = b.rank
           GROUP BY b.target) m ON m.target = t.target
-- reference_class has one row per short name (grouped by it).
LEFT JOIN reference_class r ON r.short = substr(t.target, instr(t.target, '.<base>.') + 8)
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

-- stage uniform_alias
-- Alias keys all of whose entries agree, with their target.
SELECT a.key, min(a.target) AS target FROM aliases a GROUP BY a.key HAVING count(DISTINCT a.target) = 1
-- index: key

-- stage named_prefix
-- Names a prefix scan stops at (closure.rs:2321-2335): an alias all of whose entries agree on
-- another name, a definition, an opaque binding, a module.
SELECT key FROM uniform_alias WHERE target <> key
UNION SELECT key FROM by_name UNION SELECT key FROM opaque UNION SELECT name FROM modules
-- index: key

-- stage field_alias
-- Aliases a constructor field may read: `{base}.<init-parameter>.{param}` and `{base}.{field}`
-- for every class whose order a field lookup walks (closure.rs:1890-1926).
SELECT a.key, a.target FROM aliases a
WHERE instr(a.key, '.<init-parameter>.') > 0
   OR EXISTS (SELECT 1 FROM class_root c WHERE c.cls = rx_extract(a.key, '^(.*)\.[^.]*$', 1))
-- index: key

-- stage class_of_walk
-- class_of / class_noting for a field alias's target (closure.rs:2272-2362), one hop per row
-- (`p` NULL). A prefix scan is rows with `p`, the prefixes of `cur` at its dots, longest first:
-- `rx_extract(…, '^(.*)\.')` takes the text before the last dot. A dot that ends `cur` is no
-- split point; the scan goes on past a prefix no table names and stops at the first one that is.
WITH RECURSIVE w(start, cur, k, p) AS (
  SELECT DISTINCT target, target, 0, NULL FROM field_alias WHERE substr(target, 1, 13) <> '<annotation>.'
  UNION
  SELECT w.start, x.target, w.k + 1, NULL FROM w JOIN uniform_alias x ON x.key = w.cur
  -- an alias of the name itself, if all its entries agree
  WHERE w.p IS NULL AND w.k < 17 AND instr(w.cur, '.<base>.') = 0
    AND NOT EXISTS (SELECT 1 FROM by_name b WHERE b.key = w.cur)
    AND NOT EXISTS (SELECT 1 FROM opaque o WHERE o.key = w.cur)
  UNION
  -- else the longest prefix any table names, when that is an alias all of whose entries agree
  SELECT w.start, w.cur, w.k, rx_extract(w.cur, '(?s)^(.*)\.[^.]*$', 1) FROM w
  WHERE w.p IS NULL AND w.k < 17 AND instr(w.cur, '.<base>.') = 0 AND instr(w.cur, '.') > 0
    AND NOT EXISTS (SELECT 1 FROM by_name b WHERE b.key = w.cur)
    AND NOT EXISTS (SELECT 1 FROM opaque o WHERE o.key = w.cur)
    AND NOT EXISTS (SELECT 1 FROM aliases a WHERE a.key = w.cur)
  UNION
  SELECT w.start, w.cur, w.k, rx_extract(w.p, '(?s)^(.*)\.[^.]*$', 1) FROM w
  WHERE w.p IS NOT NULL AND instr(w.p, '.') > 0 AND length(w.p) + 1 >= length(w.cur)
  UNION
  SELECT w.start, w.cur, w.k, rx_extract(w.p, '(?s)^(.*)\.[^.]*$', 1) FROM w
  WHERE w.p IS NOT NULL AND instr(w.p, '.') > 0
    AND NOT EXISTS (SELECT 1 FROM named_prefix n WHERE n.key = w.p)
  UNION
  SELECT w.start, x.target || substr(w.cur, length(w.p) + 1), w.k + 1, NULL
  FROM w JOIN uniform_alias x ON x.key = w.p AND x.target <> x.key
  WHERE w.p IS NOT NULL AND length(w.p) + 1 < length(w.cur)
)
SELECT start, cur, k FROM w WHERE p IS NULL
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
-- stage field_key_form
-- The three alias keys a constructor field `F` of class `C` is read under (closure.rs:1890-1926):
-- 0 `C.<init-parameter>.F`, 1 `C.<init-parameter>.{F without leading _}`, 2 `C.F`.
SELECT 0 AS v UNION ALL SELECT 1 UNION ALL SELECT 2

-- stage type_key_form
-- reference_type's two lookups (closure.rs:2248-2268): `<imported>` then `<class>`.
SELECT '.<imported>.' AS kind UNION ALL SELECT '.<class>.'

-- stage link_reach
-- Every name a link may read, from the summaries' edge names: a superset of link_python's
-- recursion (closure.rs:1685-1887), each step with its least hop cost; names past 19 hops in
-- cannot yield a definition (MAX_HOPS 16 plus at most three unchecked hops). Name rows have `p`
-- NULL; rows with `p` are the name's prefixes at its dots (the text before the last dot of the
-- name, then of the prefix), which the prefix arms read.
WITH RECURSIVE r(name, d, p) AS (
  SELECT DISTINCT name, 0, NULL FROM s_edge
  UNION
  -- `<bare>` / `<unqualified>` fallbacks (closure.rs:1736-1742, 1755-1761)
  SELECT substr(r.name, instr(r.name, '.<bare>.') + 8 + instr(substr(r.name, instr(r.name, '.<bare>.') + 8), '=>') + 1), r.d + 1, NULL
  FROM r WHERE r.p IS NULL AND r.d < 19 AND instr(r.name, '.<bare>.') > 0 AND instr(substr(r.name, instr(r.name, '.<bare>.') + 8), '=>') > 0
  UNION
  SELECT substr(r.name, instr(r.name, '.<unqualified>.') + 15 + instr(substr(r.name, instr(r.name, '.<unqualified>.') + 15), '=>') + 1), r.d + 1, NULL
  FROM r WHERE r.p IS NULL AND r.d < 19 AND instr(r.name, '.<unqualified>.') > 0 AND instr(substr(r.name, instr(r.name, '.<unqualified>.') + 15), '=>') > 0
  UNION
  -- late `^` (closure.rs:1794-1798)
  SELECT rtrim(rtrim(r.name, '^'), '.'), r.d + 1, NULL FROM r WHERE r.p IS NULL AND r.d < 19 AND substr(r.name, -1) = '^'
  UNION
  -- aliases of the name itself (member, closure.rs:1987-1990)
  SELECT a.target, r.d + 1, NULL FROM r JOIN aliases a ON a.key = r.name WHERE r.p IS NULL AND r.d < 19
  UNION
  -- `<declared>` / `<imported>` of a `<bare>` name (closure.rs:1718-1735)
  SELECT a.target, r.d + 2, NULL FROM r JOIN (SELECT '.<declared>.' AS kind UNION ALL SELECT '.<imported>.') k
  JOIN aliases a
    ON a.key = substr(r.name, 1, instr(r.name, '.<bare>.') - 1) || k.kind || substr(substr(r.name, instr(r.name, '.<bare>.') + 8), 1, instr(substr(r.name, instr(r.name, '.<bare>.') + 8), '=>') - 1)
  WHERE r.p IS NULL AND r.d < 18 AND instr(r.name, '.<bare>.') > 0 AND instr(substr(r.name, instr(r.name, '.<bare>.') + 8), '=>') > 0
  UNION
  -- the own member of an `<unqualified>` name (closure.rs:1748-1754)
  SELECT a.target, r.d + 2, NULL FROM r JOIN aliases a
    ON a.key = substr(r.name, 1, instr(r.name, '.<unqualified>.') - 1) || '.' || substr(substr(r.name, instr(r.name, '.<unqualified>.') + 15), 1, instr(substr(r.name, instr(r.name, '.<unqualified>.') + 15), '=>') - 1)
  WHERE r.p IS NULL AND r.d < 18 AND instr(r.name, '.<unqualified>.') > 0 AND instr(substr(r.name, instr(r.name, '.<unqualified>.') + 15), '=>') > 0
  UNION
  -- the declared-import hop (closure.rs:1814-1824)
  SELECT a.target, r.d + 2, NULL FROM r JOIN aliases a
    ON a.key = rx_extract(r.name, '^(.*)\.[^.]*$', 1) || '.<imported>.' || rx_extract(r.name, '[^.]*$', 0)
  WHERE r.p IS NULL AND r.d < 18 AND instr(r.name, '.') > 0
  UNION
  -- the prefixes of a name at its dots, longest first
  SELECT r.name, r.d, rx_extract(coalesce(r.p, r.name), '(?s)^(.*)\.[^.]*$', 1) FROM r
  WHERE r.d < 19 AND instr(coalesce(r.p, r.name), '.') > 0
  UNION
  -- prefix aliases (closure.rs:1831-1851)
  SELECT a.target || substr(r.name, length(r.p) + 1), r.d + 1, NULL
  FROM r JOIN aliases a ON a.key = r.p
  WHERE r.p IS NOT NULL AND length(r.p) + 1 < length(r.name) AND r.d < 19
  UNION
  -- constructor fields of a `<field>` name (closure.rs:1701-1712, 1890-1956)
  SELECT x.cls || '.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') + 1), r.d + 3, NULL
  FROM r JOIN reference_order o ON o.root = substr(r.name, 1, instr(r.name, '.<field>.') - 1)
  JOIN field_key_form k
  JOIN field_alias a ON a.key = o.cls || CASE k.v
      WHEN 0 THEN '.<init-parameter>.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1)
      WHEN 1 THEN '.<init-parameter>.' || ltrim(substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1), '_')
      ELSE '.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1) END
  JOIN class_of x ON x.start = a.target
  WHERE r.p IS NULL AND r.d < 17 AND instr(r.name, '.<field>.') > 0 AND instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') > 0
  UNION
  -- ... and of an annotated target (`<annotation>.X`, reference_type of the caller's parent)
  SELECT t.cls || '.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') + 1), r.d + 3, NULL
  FROM r JOIN reference_order o ON o.root = substr(r.name, 1, instr(r.name, '.<field>.') - 1)
  JOIN field_key_form k
  JOIN field_alias a ON a.key = o.cls || CASE k.v
      WHEN 0 THEN '.<init-parameter>.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1)
      WHEN 1 THEN '.<init-parameter>.' || ltrim(substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1), '_')
      ELSE '.' || substr(substr(r.name, instr(r.name, '.<field>.') + 9), 1, instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') - 1) END
  JOIN type_key_form tk
  JOIN type_key t ON t.key = rx_extract(substr(r.name, 1, instr(r.name, '.<field>.') - 1), '^(.*)\.[^.]*$', 1) || tk.kind || substr(a.target, 14)
  WHERE r.p IS NULL AND r.d < 17 AND instr(r.name, '.<field>.') > 0 AND instr(substr(r.name, instr(r.name, '.<field>.') + 9), '.') > 0
    AND substr(a.target, 1, 13) = '<annotation>.'
  UNION
  -- constructor fields after a class prefix (closure.rs:1852-1859); the prefix's dot is at
  -- length(p) + 1
  SELECT x.cls || '.' || substr(r.name, instr(substr(r.name, length(r.p) + 2), '.') + length(r.p) + 2), r.d + 3, NULL
  FROM r JOIN class_key c ON c.key = r.p
  JOIN reference_order o ON o.root = c.key
  JOIN field_key_form k
  JOIN field_alias a ON a.key = o.cls || CASE k.v
      WHEN 0 THEN '.<init-parameter>.' || substr(r.name, length(r.p) + 2, instr(substr(r.name, length(r.p) + 2), '.') - 1)
      WHEN 1 THEN '.<init-parameter>.' || ltrim(substr(r.name, length(r.p) + 2, instr(substr(r.name, length(r.p) + 2), '.') - 1), '_')
      ELSE '.' || substr(r.name, length(r.p) + 2, instr(substr(r.name, length(r.p) + 2), '.') - 1) END
  JOIN class_of x ON x.start = a.target
  WHERE r.p IS NOT NULL AND r.d < 17 AND length(r.p) + 1 < length(r.name) AND instr(substr(r.name, length(r.p) + 2), '.') > 0
  UNION
  SELECT t.cls || '.' || substr(r.name, instr(substr(r.name, length(r.p) + 2), '.') + length(r.p) + 2), r.d + 3, NULL
  FROM r JOIN class_key c ON c.key = r.p
  JOIN reference_order o ON o.root = c.key
  JOIN field_key_form k
  JOIN field_alias a ON a.key = o.cls || CASE k.v
      WHEN 0 THEN '.<init-parameter>.' || substr(r.name, length(r.p) + 2, instr(substr(r.name, length(r.p) + 2), '.') - 1)
      WHEN 1 THEN '.<init-parameter>.' || ltrim(substr(r.name, length(r.p) + 2, instr(substr(r.name, length(r.p) + 2), '.') - 1), '_')
      ELSE '.' || substr(r.name, length(r.p) + 2, instr(substr(r.name, length(r.p) + 2), '.') - 1) END
  JOIN type_key_form tk
  JOIN type_key t ON t.key = rx_extract(c.key, '^(.*)\.[^.]*$', 1) || tk.kind || substr(a.target, 14)
  WHERE r.p IS NOT NULL AND r.d < 17 AND length(r.p) + 1 < length(r.name) AND instr(substr(r.name, length(r.p) + 2), '.') > 0
    AND substr(a.target, 1, 13) = '<annotation>.'
)
SELECT DISTINCT name FROM r WHERE p IS NULL
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

-- stage prefix_alias
-- Alias keys a prefix scan stops at (closure.rs:1831-1851): an alias whose target is not the
-- prefix itself nor a workspace module without a definition of that name.
SELECT DISTINCT a.key FROM aliases a WHERE a.target <> a.key
  AND (NOT EXISTS (SELECT 1 FROM modules m WHERE m.name = a.target) OR EXISTS (SELECT 1 FROM by_name b WHERE b.key = a.target))
-- index: key

-- stage plain_prefix
-- The proper prefixes of each scanned name at its dots (`lineage().skip(1)`), the dot at `i`
-- (not the name's last character): the text before the last dot, then before the prefix's.
WITH RECURSIVE q(name, p) AS (
  SELECT name, rx_extract(name, '(?s)^(.*)\.[^.]*$', 1) FROM link_plain WHERE step = 11 AND instr(name, '.') > 0
  UNION
  SELECT name, rx_extract(p, '(?s)^(.*)\.[^.]*$', 1) FROM q WHERE instr(p, '.') > 0
)
SELECT name, p, length(p) + 1 AS i FROM q WHERE length(p) + 1 < length(name)
-- index: name

-- stage prefix_match
-- closure.rs:1829-1885: each proper prefix that ends the scan: a stopping alias, a definition, an
-- opaque binding, a module.
SELECT x.name, x.i, x.p, x.rest, x.kind FROM (
  SELECT q.name, q.i, q.p, substr(q.name, q.i) AS rest,
         CASE WHEN EXISTS (SELECT 1 FROM prefix_alias a WHERE a.key = q.p) THEN 'alias'
              WHEN EXISTS (SELECT 1 FROM by_name b WHERE b.key = q.p) THEN 'def'
              WHEN EXISTS (SELECT 1 FROM opaque o WHERE o.key = q.p) THEN 'opaque'
              WHEN EXISTS (SELECT 1 FROM modules m WHERE m.name = q.p) THEN 'module' END AS kind
  FROM plain_prefix q) x
WHERE x.kind IS NOT NULL
-- index: name, i

-- stage prefix_first
-- The longest such prefix.
SELECT m.name, m.p, m.rest, m.kind,
       CASE WHEN m.kind = 'def' AND EXISTS (SELECT 1 FROM class_key c WHERE c.key = m.p) THEN 1 ELSE 0 END AS class
FROM prefix_match m JOIN (SELECT name, max(i) AS i FROM prefix_match GROUP BY name) l ON l.name = m.name AND l.i = m.i
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
JOIN (SELECT cls, field, min(dist) AS dist FROM field_candidate WHERE hit IS NOT NULL GROUP BY cls, field) d
  ON d.cls = c.cls AND d.field = c.field AND d.dist = c.dist
JOIN (SELECT cls, field, dist, min(seq) AS seq FROM field_candidate WHERE hit IS NOT NULL GROUP BY cls, field, dist) s
  ON s.cls = c.cls AND s.field = c.field AND s.dist = c.dist AND s.seq = c.seq
WHERE c.hit IS NOT NULL
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
JOIN (SELECT root, attr, skip, min(dist) AS dist FROM method_candidate GROUP BY root, attr, skip) d
  ON d.root = c.root AND d.attr = c.attr AND d.skip = c.skip AND d.dist = c.dist
JOIN (SELECT root, attr, skip, dist, min(seq) AS seq FROM method_candidate GROUP BY root, attr, skip, dist) s
  ON s.root = c.root AND s.attr = c.attr AND s.skip = c.skip AND s.dist = c.dist AND s.seq = c.seq
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
-- The least (child, ord, dist, seq), one key at a time.
WITH c1 AS (SELECT c.* FROM compose_candidate c
            JOIN (SELECT name, min(child) AS child FROM compose_candidate GROUP BY name) m ON m.name = c.name AND m.child = c.child),
     c2 AS (SELECT c.* FROM c1 c JOIN (SELECT name, min(ord) AS ord FROM c1 GROUP BY name) m ON m.name = c.name AND m.ord = c.ord),
     c3 AS (SELECT c.* FROM c2 c JOIN (SELECT name, min(dist) AS dist FROM c2 GROUP BY name) m ON m.name = c.name AND m.dist = c.dist)
SELECT c.name, c.cls FROM c3 c JOIN (SELECT name, min(seq) AS seq FROM c3 GROUP BY name) m ON m.name = c.name AND m.seq = c.seq

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
-- file fails the clause: one row then, refused by `reference_edges` below.
SELECT 'The native reference graph is incomplete' AS reason, count(*) AS open_files
FROM s_open o JOIN in_scope s ON s.path = o.path WHERE o.open <> 0
HAVING count(*) > 0

-- stage linked_edges
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
SELECT e.path, 'reference_order: root not among class_root' FROM s_edge e JOIN link_form f ON f.name = e.name
WHERE f.form IN (2, 5, 6) AND NOT EXISTS (SELECT 1 FROM class_root c WHERE c.cls = f.a)

-- ===================================================================== output
-- stage reference_edges
-- The edges, or a named refusal (`rerail_refuse` raises its argument, INVARIANTS I9) when any
-- refusal relation holds a row: an incomplete graph fails the clause as native's does.
SELECT __path, __start, __end, target_path, target_start, target_end, source_name, target_name, target_class
FROM linked_edges
UNION ALL
SELECT rerail_refuse('reference graph: ' || construct), NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL
FROM (SELECT construct FROM linker_refusal UNION SELECT reason FROM reference_graph_incomplete)
