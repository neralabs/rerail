# NOTES: the boot group (bootstrap and schema checks)

Seven checks, written by hand in `.rr`. Scores are `rrlab dl` on the recorded batch 9 and 10 events, matched on
(path, line, code). "Reader" is the engine's reader (`rr-census`, `CLAUSES=1`): yes means the file parses and every
clause lowers.

| Check | Python lines | `.rr` lines | rrlab dl (exact / events) | Reader |
|---|---|---|---|---|
| bootstrap_cache_health | 370 | 53 | 16 / 16 | yes |
| bootstrap_lazy_sql | 390 | 16 + 54 (lib) | 2 / 2 | yes |
| bootstrap_eager_resolve | 12 | 13 (+ the same lib) | 2 / 2 | yes |
| bootstrap_engine_invariants | 2,163 | 415 + 412 (generated tables) | 143 / 145: file scan 74 / 75, fake executor 10 / 11, coverage 59 / 59 | yes |
| schema_bootstrap_contracts | 387 | 97 | 16 / 16 | yes |
| schema_architecture | 232 + 239 (plugin) | 94 | 10 / 10 | yes |
| schema_introspection_data_paths | 612 | 51 + 27 (generated tables) | 54 / 54 | yes |

bootstrap_engine_invariants takes over 115 s in one run, so it is scored in pieces (`--entry`, and `--test` for the
coverage entry). Events whose expected result is an exception (the linter raises SyntaxError on a direct call) are
not counted: a policy withholds an unparsable document.

Shared files:
- `lib/data-layer/boot-python.rr` (79 lines): how these scripts read Python by spelling. `callee_name` (a Name id or
  Attribute attr callee), `deco_word` (a decorator's name, through a call), `def_walk` (ast.walk of a def:
  decorators, signature, nested defs), and the scripts' `_annotation_names`: `ann_word`, `ann_word_s` (a
  whole-string annotation parsed), `ann_top`, `ann_subscript`, `ann_value_word`, `ann_slice_word`. Used by six of
  the seven checks.
- `lib/data-layer/boot-lazy-sql.rr` (54 lines): the BLB findings, shared by bootstrap-lazy-sql and
  bootstrap-eager-resolve, which are the same report under two names.
- `data-layer/bootstrap-engine-invariants.tables.rr`, from `gen-bootstrap-engine-invariants.py`: every module-level
  frozenset of the linter as `bei_set(SET, x)`, under the linter's own names (32 sets, 409 rows).
- `data-layer/schema-introspection-data-paths.tables.rr`, from `gen-schema-introspection-data-paths.py`: the probe
  names, SQL markers, authority words, path markers and primitive definitions. Its `--check` also compares the
  rule's `.scope include` lines with DATA_PATH_PREFIXES.

Both generators print `ok` with `--check`.

## Misses and harness notes

schema_introspection_data_paths: its `_collect_violations` entry returns Violation objects with no code, while the
plugin reports every one as **SIDP001**. The rule reports SIDP001, as the plugin and the JSON port do, and rrlab dl
names the codeless records after the rule's one code.

Remaining misses:
- **bootstrap_engine_invariants, `test_engine_private_helper_uses_selected_function`**
  (`_collect_fake_executor_violations`, BEI007 at bootstrap.py:2): an entry emulation. The direct call checks one
  def wherever it is. Through the linter's file scan, as in this rule, that check runs only in engine files, and
  bootstrap.py is not one in this event. A harness limit.
- **bootstrap_engine_invariants, `test_engine_tree_and_raw_source_are_independent`** (BEI035 at bootstrap.py:1): an
  entry emulation. The test passes `source=` text (`# SchemaTransactionSidecar`) that differs from the parsed `tree=`
  (`x=1`), and the option `rawSources` carries it. A rule reads one file, so its text and tree agree. A harness limit.

## Choices worth knowing

- **schema_architecture** is two scripts under one name. The suite's plugin (SAP000 … SAP005) and the standalone
  `lint_schema_architecture.py` (three rule ids) are both in the rule. The standalone script's findings have no
  code, so the rule reports them as `schema_architecture` with the rule id as a `tag`, which matches the recording.
  One `.scope include` covers the plugin's three directories and the script's two files outside them. So each half
  also runs on the other's files: the plugin's checks on sql_schema_services.py and sql_versioning_services.py, the
  script's in all of l2000_connection. This is a deliberate difference.
- **schema_introspection_data_paths:** the "allowed path markers" (`/schema/`, `_metadata`, `/bootstrap`, …) are
  tested in the rule, as `contains(marker, "/" + path)`, as the linter does. The 14 markers would take 28 globs in
  `.scope exclude`, and those would only approximate the substring test. The empty, unsigned approval list (and
  its SIDP000) is left out: SIDP000 needs a non-empty list.
- **bootstrap_engine_invariants:**
  - The engine files are the two defaults, plus any path in the options `bootstrapPaths` / `enginePaths`, which
    stand for the tests' monkeypatching of BOOTSTRAP_FILES and BOOTSTRAP_ENGINE_FILES. A list option has no row
    when it is empty, so the defaults always count. In the real repository that is what the linter does too.
  - Forbidden calls are table-driven: `bei_watch(family, code, def)` and `bei_ban(family, name)`, one family per
    linter collector. The shape checks (`@pure_lazy`, `-> LazyResult[…]`, keyword-only refs) share
    `bei_lazy_surface`.
  - `-> LazyResult[…]` is read from the tree. The linter tests whether `ast.unparse(returns)` starts with
    "LazyResult[". It does exactly when the annotation's leftmost operand is a subscript of the bare name
    LazyResult, so `-> LazyResult[int] | None` passes (bei_leftmost).
  - BEI007's zero-count guard is an enclosing ast If whose test compares `….ZERO_APPLICATION_TERMINALS`. An `elif`
    is an If whose orelse holds every later clause, so a call in a later `elif` or the `else` is guarded too
    (bei_if_holds).
  - BEI020 has a second branch in the linter, a Name `fallback_schema_ref`, which is dead code: the collector is
    only ever passed Call nodes. The rule leaves it out. Running the linter on
    `def _application_sql_assertions_for_plan(plan): fallback_schema_ref = 1` in an engine coordinator reports
    nothing.
  - BEI032's base is a Name or Attribute: `class E(PipelineLifecycleEffect[int])` is not an effect, because the
    linter's `_attribute_name` gives "" for a Subscript. "In the class" includes its decorators, as ast.walk does.
  - Argument values are read with their parentheses taken off (bei_kw, bei_arg0), and positional counts include
    `*xs` (bei_npos), as ast's Call.args does.
  - Review counterexamples: the linter itself, run on the four review trees, agrees with the rule on every finding
    except the match capture below.
  - At 415 lines the file is long. That is 45 codes from a 2,163-line linter, in six families; the families are
    the structure, and a library would not shorten it.
- **Strings:** constant text tests (BEI036's stub markers, SIDP001's SQL markers) read `str_part`, the str
  Constants as CPython builds them, f-string pieces included. The rrlab facts give only an f-string's first piece.
  No event depends on a later one.
- **Annotations** (schema_bootstrap_contracts): a whole-string annotation is parsed with `parsed(s, "annotation", r)`;
  if it does not parse, the string is its own word, as `_annotation_names` has it. tree-sitter writes
  `Sequence[T]` in an annotation as a `generic_type`, so `ann_subscript` reads both that form and `subscript`.

## Library and core change requests

**python/ast.rr, `ast_name`: match captures are not Names.** In `case x:`, `case [a, *r]:`,
`case P(k=b) as c:` CPython binds x, a, r, b and c as strings (MatchAs.name, MatchStar.name), not as ast.Name.
A dotted value `case C.RED:` and the class `P` are Names. `ast_name` counts the captures, so BEI002 reports
`case _publish_static_naming_rows_direct:`, and the linter does not. Proposed addition to ast.rr, next to `not_name`:

```
// Match captures: CPython keeps a capture's name as a string (MatchAs.name, MatchStar.name, MatchMapping.rest),
// not a Name. `case x`, `case [a, *r]`, `case P(k=b) as c`, `case {**rest}` bind x, a, r, b, c, rest; the
// value `C.RED` and the class `P` stay Names, and a keyword pattern's key (k) is a string too.
.decl capture_name(n: node)
capture_name(n) :- (kind(p, "case_pattern") ; kind(p, "keyword_pattern")), child(p, dn, _), kind(dn, "dotted_name"),
    child(dn, n, 0), !child(dn, _, 1).
capture_name(n) :- kind(p, "keyword_pattern"), child(p, n, 0), kind(n, "identifier").
capture_name(n) :- (kind(s, "splat_pattern") ; kind(s, "dictionary_splat_pattern")), child(s, n, 0),
    kind(n, "identifier"), inside(s, _, "case_clause").
capture_name(n) :- kind(a, "as_pattern"), inside(a, _, "case_clause"), child(a, n, 1), kind(n, "identifier").
not_name(n) :- capture_name(n).
```

The tree shapes are tree-sitter-python's, checked with rrlab on these four cases. The rule does not work around
this.

## Not expressible

Nothing. Every code of the seven linters is in a rule.
