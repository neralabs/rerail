# Lazy group: notes

| check | Python lines | .rr lines | rrlab dl (exact / events) | engine reader |
|---|---|---|---|---|
| lazy_sql_surface | 446 | 125 | 34 / 34 | lowers |
| lazy_adapter_ownership | 529 | 168 | 9 / 9 | lowers |
| lazy_architecture | 3,840 | 459 + 474 library (`lib/data-layer/lazy-analyzer.rr`) + 318 generated table | 197 / 197 for the reach variant (see below); 177 / 197 as written | refuses `dfs_first` and `dfs_first_memo` |

Recorded events after rrlab's filtering: 34 for lazy_sql_surface (36 recorded) and 197 for lazy_architecture (205
recorded).

The generated table is `data-layer/lazy-architecture.tables.rr`. It is built by
`data-layer/gen-lazy-architecture.py`, and `--check` confirms it matches the linter at 2b1504adc. It holds every
module-level string and string-set constant of the script, as `la_set(NAME, v)`.

## How lazy_architecture was scored

rrlab gives `dfs_first` and `dfs_first_memo` no rows, so every finding found by a transitive search is missed. This
is a harness gap. Those misses are the 20 events lost by the file as written: 5 in `_scan_analyzed_files`, 6 in
`ast-component` and 9 in `scan_files`.

To check the edges, targets and roots, I scored a scratch variant. It replaces each
`dfs_first[_memo](g, _, i, edge, target[, roots])` with plain reachability: `i = min j : { edge(g, m, j), reach(m) }`.
The variant scores:

- `scan_files`: 105 / 105
- `scan_files.ast-component`: 43 / 43
- `_scan_analyzed_files`: 6 / 6
- `_scan_deleted_cold_bootstrap_surfaces`: 43 / 43

I scored the last entry with a variant that keeps only the LA-018 text scan. Its events contain the real 5k–15k-line
schema files, and the full analysis of those takes about 38 s per event in rrlab, which goes past the 2-minute cap. The
entry reports only LA-018, and LA-018 does not depend on the analysis.

Reachability matches `dfs_first_memo` whenever no root's search cuts through a node that an earlier root's search
already visited and that is still on the stack. The recorded events never hit that case.

## Deliberate differences

- **lazy_sql_surface: the code is the message.** The direct calls return messages only, so the recorded "code" is the
  message. `LSS(at, c) code {c}` reports the message as the code. The plugin reports every finding as LSS-001.
- **lazy_adapter_ownership: the source-trigger gate is applied.** It is written `lao_doc` (Transaction, PlanScope,
  legacy_, pure_lazy, @lazy). The plugin applies this gate, but the direct `materialize_source_facts` path does not.
  No scored event depends on the difference.
- **lazy_architecture follows `scan_files`, not the plugin:**
  - **No per-symbol dedupe.** The plugin keeps one finding per `LA-xxx::symbol`, but events expect two LA-017
    findings for the same symbol.
  - **LA-018 is reported.** The text scan is part of `scan_files`, not of the plugin.
  - **No source-trigger gate.** "def " is one of its triggers, so it gates almost nothing.
  - **The allowlist applies to every finding,** as `LA-xxx::symbol` keys in
    `scripts/allowlists/lint_lazy_architecture_allowlist.txt`.
  - **The signature is checked through signing.rr.** It is read from the script's `ALLOWLIST_SIGNATURE` line. A bad
    signature gives LA-000 at the script's line 1, which is a gate.
  - **The ratchet becomes a `.budget`** from `.ratchet-baselines/lint_lazy_architecture.json` `/counts`, where every
    ceiling is 0. It reports code LA at the baseline file, while the script reports its own ratchet message.

## Known approximations (lazy_architecture)

- `callable_aliases` follows only plain name targets. A cycle in a chain of name aliases gives no symbol, where the
  script returns the start name.
- The script's recursion and depth caps are ignored.
- "First" means first by `visit_index`, which is tree-sitter preorder. CPython's `ast.walk` order differs in a few
  places, such as dict keys against values, and these only matter when two entries compete.
- `TryStar` (`except*`) blocks are not walked as the script walks them.
- The LA-011 helper-regression branch is unreachable with the script's own tables, so it is not written.
- COVERAGE.md's no-L6 note stands. `scoped_rhs` and `callable_aliases` are lexical: the latest visible assignment,
  found by position.

## Engine reader refusals

- **lazy_architecture:** 9 lines use `dfs_first_memo` and 2 use `dfs_first`, and the reader answers "not lowered
  yet" for all of them. The form is the one COVERAGE.md names for these searches, so I kept it.
- **Rewritten so they lower:**
  - `code {c} in "regex"` on a `.violation` is a syntax error, so I dropped it. It only documented the code range.
  - `@place` outside a `.violation`'s `at` is not lowered, so `la_find` now carries path and line.

## Library and core change requests

- **rrlab:** give `dfs_first` and `dfs_first_memo` rows. The reachability form above would do as a stand-in.
- **rrlab dl:** `--test` recomputes the entry's reported codes from the filtered events only, so a single-test run of
  an entry that reports no codes compares every code. Compute `reported` before the `--test` filter.
- **rr-census:** lower `dfs_first` and `dfs_first_memo`.

## Not expressible

Nothing.
