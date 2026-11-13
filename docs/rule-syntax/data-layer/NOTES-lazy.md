# Lazy group: notes

| check | Python lines | .rr lines | rrlab dl (exact / events) | engine reader |
|---|---|---|---|---|
| lazy_sql_surface | 446 | 124 | 36 / 36 | lowers |
| lazy_adapter_ownership | 529 | 168 | 9 / 9 | lowers |
| lazy_architecture | 3,840 | 484 + 503 library (`lib/data-layer/lazy-analyzer.rr`) + 318 generated table | 197 / 205 for the reach variant (see below); 176 / 205 as written | refuses `dfs_first` and `dfs_first_memo` |

The generated table is `data-layer/lazy-architecture.tables.rr`. It is built by
`data-layer/gen-lazy-architecture.py`, and `--check` confirms it matches the linter at 2b1504adc. It holds every
module-level string and string-set constant of the script, as `la_set(NAME, v)`.

## How lazy_architecture was scored

rrlab gives `dfs_first` and `dfs_first_memo` no rows. This is a harness gap, and it is why the file as written loses 22
events (the other 7 misses are the dedupe ones below).

To check the edges, targets and roots, I scored a scratch variant. It replaces each
`dfs_first[_memo](g, _, i, edge, target[, roots])` with plain reachability: `i = min j : { edge(g, m, j), reach(m) }`.
I scored each entry on its own, to stay under the run cap.

| entry | exact / events | how |
|---|---|---|
| `scan_files` | 108 / 113 | reach variant |
| `scan_files.ast-component` | 40 / 43 | reach variant; the file as written now scores the same |
| `_scan_analyzed_files` | 6 / 6 | reach variant; the file as written now scores the same |
| `_scan_deleted_cold_bootstrap_surfaces` | 43 / 43 | LA-018-only variant |

The last entry reports only LA-018, which does not depend on the analysis. Its events hold real 5k–15k-line files,
and the full analysis of those takes about 38 s per event in rrlab.

The variant's 8 misses:

- **7 are the plugin's dedupe** (see below). The recorded `scan_files` results keep every site:
  - two LA-017s for one symbol: `test_reconciliation_direct_ddl_triggers_la017` and
    `test_fact_check_default_false_result_triggers_la017`, each recorded through both entries;
  - two LA-014s for one symbol: `test_la014_flags_dynamic_behavior_lookup_in_materializer`, also through both
    entries;
  - an `@overload` stack's two LA-010s: `lazy-overload-declaration-diagnostics`.

  The plugin keeps the first site of each, as the rule now does.
- **1 is a stand-in artefact:** `native-ordered-cycle`. Plain reachability answers through a node that an earlier
  root's memoised search already visited. `dfs_first_memo` handles that node as the script does, so the rule as
  written is right here.

## Deliberate differences

- **lazy_sql_surface reports one code, LSS-001, as the plugin does.** The message tells the patterns apart. The
  direct calls return messages only, and rrlab dl maps their findings to the one closed code.
- **lazy_adapter_ownership: the source-trigger gate is applied.** It is written `lao_doc`. The plugin applies this
  gate, but the direct `materialize_source_facts` path does not. No scored event depends on the difference.
- **lazy_architecture follows the suite plugin:**
  - **One finding per `LA-xxx::symbol`,** the first by (file, line), before the allowlist.
    - A function's symbol is module-qualified, so it lives in one file, and the rule takes `min` line within it.
    - The whole-file scans' symbols are constant: `defer_lazy_result`, and imported or called compatibility
      names. For those the plugin keeps the first site across files, ordered by path string, then line. The rule
      keeps the first site in each file, because `min` over symbols is not a path order.
  - **LA-018 (the text scan) is `.mode log`.** The plugin never runs it, and production events drop it. It is still
    reported for `scan_files` events.
  - **No source-trigger gate.** "def " is one of its triggers, so it gates almost nothing.
  - **Signature:** the allowlist is checked through signing.rr. A bad signature gives LA-000 at the script's line 1,
    as a gate.
  - **Budget:** `LA_budget` reports each code whose count of sites, taken before dedupe and allowlist, exceeds its
    ceiling in `.ratchet-baselines/lint_lazy_architecture.json` `/counts`. It reports at that file's line 1, as the
    plugin does.
    - A budget that does not load gives LA-000 there. This covers a missing code; LA-013 and LA-017 default to 0.
      As with the signature, this only applies where the script itself is present.
    - Two sites at one line with one symbol count once. The script counts them twice.

## Known approximations (lazy_architecture)

- `callable_aliases` follows plain names and tuple or list targets (each name is an alias when the value holds a
  callable; only a later plain assignment clears it). A cycle in a chain of name aliases gives no symbol, where the
  script returns the start name.
- The script's recursion and depth caps are ignored.
- "First" means first by `visit_index`. rrlab uses start order, while the engine uses CPython's ast order.
- `TryStar` (`except*`) blocks are scanned whole, an approximation of the script's walk.
- `_transitive_line_to_target_symbols` (the LA-011 permission search) counts a root that it reaches again through a
  cycle. `dfs_first_memo` never returns to its root, so that one case gives no finding.
- The scratch reachability stand-in differs from `dfs_first_memo` on ordered cycles, as above. It is used only for
  scoring.
- COVERAGE.md's no-L6 note stands. `scoped_rhs` and `callable_aliases` are lexical: the latest visible assignment,
  found by position.

## Engine reader refusals

- **lazy_architecture:** 9 lines use `dfs_first_memo` and 2 use `dfs_first`, and the reader answers "not lowered
  yet" for all of them. The form is the one COVERAGE.md names for these searches, so I kept it.
- **Rewritten so they lower:**
  - `code {c} in "regex"` on a `.violation` is a syntax error, so I dropped it.
  - `@place` outside a `.violation`'s `at` is not lowered, so `la_find` carries path and line.

## Library and core change requests

- **rrlab:** give `dfs_first` and `dfs_first_memo` rows.
- **rrlab:** lower `split_dot(s, _, x)`. It is written as `_ = @rr_dot_init(s)`, which Soufflé rejects. The library
  names the variable instead.
- **rrlab:** apply `.gate`. It is dropped, so a gating LA-000 is reported alongside the other findings.
- **Engine reader:** lower `dfs_first` and `dfs_first_memo`.

## Not expressible

Nothing.
