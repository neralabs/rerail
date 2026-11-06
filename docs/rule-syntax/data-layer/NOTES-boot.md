# NOTES: the boot group (bootstrap and schema checks)

Seven checks, written by hand in `.rr`. Scores are `rrlab dl` on the recorded batch 9 and 10 events, matched on
(path, line, code). "Reader" is the engine's reader (`rr-census`, `CLAUSES=1`): yes means the file parses and every
clause lowers.

| Check | Python lines | `.rr` lines | rrlab dl (exact / events) | Reader |
|---|---|---|---|---|
| bootstrap_cache_health | 370 | 53 | 15 / 16 | yes |
| bootstrap_lazy_sql | 390 | 16 + 54 (lib) | 2 / 2 (+1 skipped) | yes |
| bootstrap_eager_resolve | 12 | 13 (+ the same lib) | 2 / 2 | yes |
| bootstrap_engine_invariants | 2,163 | 385 + 412 (generated tables) | 143 / 145 (+1 skipped): file scan 74 / 75, fake executor 10 / 11, coverage 59 / 59 | yes |
| schema_bootstrap_contracts | 387 | 97 | 16 / 16 (+1 skipped) | yes |
| schema_architecture | 232 + 239 (plugin) | 94 | 10 / 10 (+1 skipped) | yes |
| schema_introspection_data_paths | 612 | 51 + 27 (generated tables) | 54 / 54 with the code remap below (9 / 54 as recorded) | yes |

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

The harness (`tools/rrlab/dl.py`) has two gaps. I scored through a patched copy in my scratch directory; dl.py
itself is unchanged.
- **Exception records crash dl.py.** A record whose `expected` is an exception (`{'exception': {'type':
  'SyntaxError', …}}`) makes `events()` fail with `TypeError: string indices must be integers` for
  bootstrap_lazy_sql, schema_architecture, schema_bootstrap_contracts and bootstrap_engine_invariants. The patched
  copy skips those records; they are the "+1 skipped" in the table. In each case the linter raises SyntaxError from
  a direct call. A policy withholds an unparsable document and gives no finding, unless it says `.on syntax_error`.
- **Codeless records.** dl.py gives a record without a `code` field the check's name as its code. That is right for
  schema_architecture's standalone script: its findings have a rule id and no code (see below). It is wrong for
  schema_introspection_data_paths: its `_collect_violations` entry returns Violation objects that carry call_name,
  not a code, while the plugin reports every one as **SIDP001**. The rule reports SIDP001, as the plugin and the
  JSON port do. With the recorded codes remapped (`schema_introspection_data_paths` → `SIDP001`), all 54 events are
  exact; as recorded, 9 are (the 9 with no findings). A fix in dl.py: give such an entry's records the code that the
  check's plugin reports.

Remaining misses:
- **bootstrap_cache_health, `test_cache_parser_error_is_a_diagnostic`** (BCH000 at line 1): a harness gap.
  The rule says `.on syntax_error report BCH000 at line 1`, which is the linter's extraction-failure finding, but
  rrlab does not implement `.on syntax_error`. schema_architecture's SAP000 is written the same way, and no
  recorded event exercises it.
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
  - `-> LazyResult[…]` is read from the tree: the return annotation is a subscript of the bare name LazyResult.
    The linter tests whether `ast.unparse(returns)` starts with "LazyResult[", and that is the same test.
  - BEI007's zero-count guard is an ancestor `if` (or `elif`) whose test compares `….ZERO_APPLICATION_TERMINALS`.
    An `elif` two or more steps down its chain is not seen as inside the first `elif`'s If, as it is in ast. No
    event exercises this.
  - At 385 lines the file is long. That is 45 codes from a 2,163-line linter, in six families; the families are
    the structure, and a library would not shorten it.
- **Strings:** constant text tests (BEI036's stub markers, SIDP001's SQL markers) read `str_part`, the str
  Constants as CPython builds them, f-string pieces included. The rrlab facts give only an f-string's first piece.
  No event depends on a later one.
- **Annotations** (schema_bootstrap_contracts): a whole-string annotation is parsed with `parsed(s, "annotation", r)`;
  if it does not parse, the string is its own word, as `_annotation_names` has it. tree-sitter writes
  `Sequence[T]` in an annotation as a `generic_type`, so `ann_subscript` reads both that form and `subscript`.

## Library and core change requests

None. All new vocabulary is in the two library files above.

## Not expressible

Nothing. Every code of the seven linters is in a rule.
