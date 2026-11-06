# Group `conn`: connection and graph-path checks

Seven data-layer checks, hand-written in `.rr`. They are scored with `tools/rrlab/rrlab dl <check>` on the
data-layer port's recorded linter calls. The reader column is the final-shape engine's reader
(`rr-census` with `CLAUSES=1`).

| Check | Python lines | `.rr` lines | rrlab dl (exact / events) | Reader lowers |
|---|---:|---:|---|---|
| connection_mode_split | 390 | 92 | 12 / 13 | yes |
| legacy_callback_quarantine | 515 | 92 | 11 / 11 | yes |
| connection_structural_boundaries | 874 | 156 | 14 / 15 | yes |
| connection_service_architecture | 923 | 335 | 32 / 32 | yes |
| graphql_plan_ownership | 446 | 81 | 43 / 50 | yes |
| bounded_composed_graph_paths | 387 | 213 | 66 / 66 | yes |
| graph_lazy_sql_plan | 862 | 160 | 115 / 115 | yes |
| **total** | 4,397 | 1,129 | 293 / 302 | |

The JSON port of bounded_composed_graph_paths is 380 KB. The `.rr` is 213 lines: it reads physical lines
(`src_line`), as the linter's `(?m)^` regexes do, and keeps its text shapes (class blocks, four-space methods and
decorators, field lines).

## Misses

- **connection_mode_split, `test_mode_source_gate_uses_supplied_probe` (`lint_source`):** a harness limit. The
  test passes `probeSource: "pass\n"` to the source-trigger gate, while the file on disk defines
  `class DatabaseConnection`. The gate is a no-op on real input, because every reported name contains a trigger,
  so the rule has no gate and reports the class.
- **connection_structural_boundaries, `test_legacy_lifecycle_helper_preserves_first_definition_and_method_order`
  (`_lifecycle_violations`):** a deliberate difference. That entry is the linter's legacy AST mode, which takes
  the *first* top-level class of a name and the first method of a name. The linter's fact mode (`lint`, the
  plugin) keeps the *last* class of a name, with all its methods. The rule follows fact mode, which
  `test_structural_duplicate_and_base_controls` checks. The two modes disagree on the same file: with two
  `_ConnectionLifecycleMixin` classes, legacy mode reports lines 2 and 3, and fact mode reports line 6.
- **graphql_plan_ownership, the 7 GPO000 events:** a harness gap. The rule says
  `.on syntax_error report GPO000 at line 1`, which rrlab does not model (README, "Known gaps": `.on`). Every
  other event is exact.

## Deliberate differences

- **connection_service_architecture, CONN601:** reported on `impl/read_through/connection.py` when the policy
  reads that file. The linter's `lint()` reads that file only for CONN103, so its CONN601 check (gated on that
  exact path) never fires in production. Its own test (`test_read_through_getattr_delegation_is_forbidden`)
  shows the intent.
- **connection_structural_boundaries, the options `connectionPath`, `lifecyclePath` and `sqlPath`:** the rule
  takes the three l2000/l2200 paths from these options, with the linter's constants as defaults, as the tests do
  when they move them.

## Reader refusals

None remain. Two forms were changed so that they lower:
- bounded_composed_graph_paths, `code {c} in "BCGP0[0-1][0-9]"`: the reader does not parse this continuation line,
  so it was dropped. The codes are constants in the rule anyway.
- bounded_composed_graph_paths, recursion over `l + 1` / `l - 1` (decorator runs, blank lines above a def): the
  reader refuses these as an `UnboundedValueCycle`, although `bcgp_line` bounds them. They are written as counts
  instead: "no line strictly between a and b that is not `    @…`".

## Harness fixes made (tools/rrlab/dl.py)

`events()` crashed on four shapes of `expected` in the complete-api records, so I changed how it reads them:
- a finding without `code` (connection_mode_split's `Violation`) takes the check name, which is the plugin's code;
- `line: None` (connection_service_architecture's file-level `ArchitectureError`) is line 1, as the plugin
  reports it;
- `file_path` / `line_number` (graph_lazy_sql_plan's `Violation`) are read as `path` / `line`;
- an `expected` that is not a list (`{"exception": …}`, from the conflicting-legacy-modes test) is skipped, as
  other events the upstream run did not pass are.

## Harness gaps

- rrlab does not model `.on syntax_error report CODE [at line 1]`. It is a small change in `rrlab.py`: one finding
  per document that has a `syntax_error` row, with the directive's code, at line 1 or at the error's line. It
  would lift graphql_plan_ownership to 50 / 50, and it would help the other groups that use the directive
  (bootstrap-cache-health, lazy-adapter-ownership, schema-architecture and others).
- rrlab's lowering refuses `split_dot` with a `_` column (`Underscore in binary relation`).
  connection_mode_split uses a capture instead.

## Library and core changes

None requested. I added no library file. Each linter has its own variant of "a decorator's name" (Name only,
Name or Attribute, through calls or not), of "a class's methods" (first or last of a name, top-level classes
only, merged across same-named classes) and of "a docstring-free body". So the rules keep short local helpers,
with prefixes `cms_`, `lcb_`, `csb_`, `csa_`, `gpo_`, `bcgp_` and `glsp_`. If a later group needs the same
variants, these are candidates for a `lib/data-layer/conn-classes.rr`: `top_class`, `direct_method_last`,
`decorator_last_name`.

## Not expressible

Nothing. All seven checks are written in full.
