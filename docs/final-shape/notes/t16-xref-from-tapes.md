# T16 notes: the cross-file inputs from this crate's tapes (O39, O40)

Branch `jeremybarnes/final-rerail-shape-facts-crossfile-xref` from the port branch (`3c7e13e0e`)
merged with `35c010235`. Runs are under `~/rerail-bench/final-shape/xref/runs/` (`runs/` below),
each with `COMMIT.txt`, `load-start.txt`, `load-end.txt`, `stdout.txt`, `stderr.txt` and
`results.json`. Driver: `~/rerail-bench/final-shape/xref/run-test.sh`. **[unverified]** = not run
here.

## 1. What Y1 and the symbol table read

Y1 (`ygraph::summary`) reads exports 3 (`in_scope`) and 7–13: native's call summary
(`calls::summarize`) of the content under three symbolic module variants, after
`as_references` with the pack's links (`cgn_def`, `cgn_edge`, `cgn_alias`, `cgn_opaque`,
`cgn_module`, `cgn_open.open`, and the `cgn_reference_alias` rows whose key holds
`.<base-slot>.`). The symbol table (`y3::input`) reads exports 14–26: the X6 pack's thirteen
pattern inputs and four per-file stages over them (`proj_inventory`, the seed of
`proj_imports`' tokens, `proj_explicit_exports`, `dc__module_names`). facts-a produced the
summaries with this crate's `calls::summarize` (`cgnative`) and the inputs with its own leaf
executor (`xexport`).

## 2. The producer (`facts::xinputs`)

- `cgnative`: facts-a's extraction, copied, trimmed to the per-content part, on this crate's paths.
- `pack`: the pack's inputs, each compiled as a `budget` clause by the production compiler with
  the pack's `shared` define, run by `eval::stateless_rows_to` over the tape installed with
  `TapeCacheSlot::seed`. The file is read as `x.py`; the `file` path predicates (`py_all`,
  `py_all_names`) are taken out and applied by the reader, so every row is a function of the
  content alone.
- `exports`: the rows at a path, shaped as the exports (scope, path predicates and module applied
  there). The annotation, decorator, `__all__`-name and dynamic-import exports are distinct.
- `store`: the rows as narrow tables keyed by `tape_objects.tape_id` (`content_store::xinputs`,
  20 tables, texts as atoms, cascading from the tape), a derivation `xinputs:{tape hash}` per
  content (reads the tape hash, outputs the rows' digest), and `StoreSource`: (path, blake3)
  → source alias → tape (stored, else encoded as the tape family does) → set (stored, else
  produced and published) → exports.
- `y3::wire::Source`: the xref and symbols steps read the inputs of their changes on first use
  (`Shared::ensure`), through `summary::insert` and `input::insert`, so the readers are unchanged.
  `xinputs::store::Reading` is that source over `StoreSource`.
- `deadcode::replay`: `RERAIL_DEADCODE_INPUTS=store` (default) ingests the in-scope jobs'
  contents into the run's store (8,125 contents: sources, tapes, sets; 82.6 s), then reads Y1's
  and the table's inputs through `Reading`; `jsonl` loads X6's exports up front (the reference).

## 3. Recovery (O39)

`#xref:` and `#symbols:` record the membership digest their state holds (`Wt::xref_tree`,
`sym_tree`) as their read of `{scope}#tree:`. A run takes the push's edits only as a cache: it
uses them when they turn its membership's digest into the one bound now; otherwise (after a
crash, when no push handed them) it diffs its own membership (`Inputs::members`: Y1's records,
the table's members) against the path names bound under the scope
(`ContentStore::bindings_with_prefix`, SQLite: recovery runs on open, before any push, with
the overlay empty) and counts the run in `Step::rebuilt`. `DeadCode::observe` is the observer's
half of `push`. `bindings_differ` also checks that both states hold the bound membership.

## 4. Exactness

| Check | Result | Run |
|---|---|---|
| Producer = X6 exports, every (path, content) of the replay corpus (19,801 lines, 19,738 contents), exports 3 and 7–26, as multisets and in order | **18,005,997 rows; 0 lines differ (multiset), 0 (order); 0 contents failed**; 65 s at 6 threads, load 70 → 40 | `runs/20261102-154541-diff-full` |
| Job sources = the exports' contents (`facts-b b3sum` of every source) | 19,801 / 19,801 | (shell check, not recorded as a run) |
| Before the distinct exports: duplicates only (no row on one side only) in 16, 17, 20, then 22, 25 | fixed in `exports` | `runs/20261102-153753-diff-every50`, `-154404-diff-full` |
| Full replay on the store's inputs, maintained, graph on the store, check every load and event (forked) | **1,151 checks, 0 differences; 1,151 binding checks, 0** | `runs/20261102-155728-exactness-store-inputs` |
| Oracle tree: lattice findings = dataflow `dc__findings` = Y2 from scratch, both modes | **4,931 = 4,931 = 4,931**, same digest, `check` clean | `runs/20261102-160855-oracle` |
| Crash between observe and recompute (`deadcode::tests::a_crash_between_observe_and_recompute_recovers_xref_and_symbols_from_the_bindings`, both modes): `recover` rebuilds both deltas from the bindings (`rebuilt` = 2), `check` and `bindings_differ` clean, the next push takes the cache, outputs = a run that never crashed | pass | `cargo test --lib --features metal-tests -- facts::deadcode::tests` (2 passed) |
| Unit: `content_store::xinputs` (round trip, replace, cascade; tables = spec), producer smoke test | pass | `runs/20261102-153600-unit` |

## 5. Footprint and latency

| | T12 (end / peak: §4.1, `-102505-quiet-workload-maintained-store`; the samples below: §5, `-094712-workload-maintained-store`) | here, store inputs (`runs/20261102-160640-workload-store-inputs`) |
|---|---|---|
| footprint end / peak (MB) | 4,043 / 5,264 | **3,341 / 3,361** (−702 / −1,903) |
| after the inputs (MB) | 1,706 (JSONL loaded) | 1,343 (ingest done, nothing loaded) |
| Y3 world / 40 worktrees loaded (MB) | 2,661 / 3,109 | 1,821 / 2,307 |

The exactness run agrees (3,346 / 3,379 MB). The peak falls because nothing parses the 3.5 GB
export file; inputs are read per (path, content) as steps first touch them (8,149 sets read).

**Per-edit latency, quiet machine (decider, same binary `e5ac5e2ea`, maintained mode, store graph, n = 1,050, ms p50 / p95 / p99 / max):**

| | store inputs (`runs/20261102-173644-quiet-workload-store-inputs-maintained`, load 1.7 → 2.3) | JSONL inputs (`runs/20261102-173908-quiet-workload-jsonl-inputs-maintained`, load 2.7 → 2.4) |
|---|---|---|
| dead-code steps (Y1, symbols, CSR rows, values, findings, digests) | 0.389 / 2.66 / — / 19.7 | 0.381 / 2.63 / 5.76 / 19.5 |
| push outside the steps (observer, propagate, SQLite, the edited content's inputs) | **0.868 / 5.15 / 11.9 / 26.0** | 0.161 / 0.253 / 8.22 / 9.9 |
| **push total** | **1.29 / 7.81 / 16.2 / 44.5** | 0.553 / 3.46 / 9.66 / 19.7 |
| footprint end / peak (MB) | 3,324 / 3,344 | 3,939 / 5,227 |

- The dead-code steps cost the same on both inputs (2.6 ms p95); maintained values 0.003 ms p95.
- Store inputs add about 4.9 ms p95 per edit outside the steps. The JSONL carries every content's inputs precomputed, the edited versions included; with store inputs an edit encodes the new content's tape and runs the 13-input pack over it before the steps read it, and writes the rows. That is the honest per-edit cost of producing cross-file inputs; which part dominates (tape, pack, store writes) is not profiled [INFERENCE].
- They save 615 MB at the end and 1.88 GB at the peak.

## 6. Not done

- A profile of the 4.9 ms p95 the store inputs add per edit (tape encode, pack, store writes).
- The overlay graph with store inputs, and `recompute` mode, on the full replay (only maintained /
  store graph ran); the fast test covers both modes on the store.
- The fallback delta reads SQLite only: an overlay that still holds bindings must be flushed
  before it is used (true at recovery, which runs on open).
