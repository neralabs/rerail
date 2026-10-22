# Spike B report: base + delta dataflow for cross-file rules

Branch `Moti-Velz/facts-spike-b`, crate `spikes/facts-b`. Raw runs: `~/rerail-bench/facts/spike-b/runs/<run>/` (each holds `command.txt` with the commit and command line, `uptime.before`/`uptime.after`, `stdout.txt`, `stderr.txt` with `/usr/bin/time -l`, and `results.json`). Run names below are relative to that directory.

**Verdict.** The design is exact (B1: 0 differences over 1,576 worktree checks) and fast (B2: p95 0.39 ms per single-file edit, B5: 0.11 ms new worktree, 46 ms for 200 files). Memory misses the 200 MB target as specified: 255 MB resident (B3 fail, below the 500 MB kill line). Per-worktree state is proportional to divergence (B4). Two variants measured on the same corpus and replay close B3: two-level cluster bases (195 MB, exact) and a per-path plurality base (176 MB loaded, 197 MB after the replay).

## 1. Setup

| Item | Value |
|---|---|
| Code | `spikes/facts-b` at `3019c8adf` for every gate run (engine code identical to `485b5de40`; `3019c8adf` adds only the report draft). Variant `variant-plurality` at `772273183`. |
| Build | `cargo build --release -p facts-b` (workspace release profile: thin LTO, codegen-units 1), `CARGO_BUILD_JOBS=4`, `CARGO_TARGET_DIR=<worktree>/target` |
| Engine | differential-dataflow 0.25.1, timely 0.31.0, one worker (`timely::execute_directly`), timestamps `u32` epochs, diffs `i32` |
| Grammar | Rerail's vendored, patched tree-sitter-python (`native/grammars/tree-sitter-python/src/grammar.json`), generated and compiled by the crate's `build.rs` exactly as `native/build.rs` does. No production crate touched. |
| Machine | macOS (Darwin 25.6), arm64; every timing run under `~/rerail-bench/bench-lock`; 1-minute load before the runs 1.56 to 4.20 (all trusted, < 8) |
| Corpus | `~/tempo-qe/workspaces47` scan: 47 worktrees, 262,567 `.py` files, **7,091 distinct contents** (blake3). Worktrees `01`-`40` form the 40; they hold 7,057 distinct contents. |
| Base | strict majority: every (path, hash) held by ≥ 21 of the 40 worktrees: 5,458 files, 1,103,227 exported fact rows (`20261022-122050-divergence`) |
| Replay | seed `0x5EED_B` (388827), `~/rerail-bench/facts/spike-b/edits/seed-5eedb/{events.jsonl,summary.json}`: 1,000 single-file edits round-robin over 01-40, 5 bulk events, 5 new worktrees (41-45) × 10 edits, 51 real commits |
| Excluded | Worktrees 41-47 of the corpus (extracted only). R2 method scope and R3 policy disjuncts listed in § 5.4. No oracle parity against Jeremy's recordings (not B's gate). |

Edit mix (`summary.json`): delete a function 257, rename a function and its in-file references 200, add/remove an import 143, add/remove a call 204, revert the worktree's previous edit 92, whitespace-only 104 (10 of which are reverts with nothing to revert yet, recorded as `revert_fallback_to_whitespace`). The generator re-parses every edited content and only emits syntax-valid results; whitespace edits assert identical facts. Bulk events replace 200 files of worktree w with worktree v's content: (01←40), (10←30), (20←05), (35←15), (40←02). New-worktree events clone the base at edits 100, 300, 500, 700, 900, then 10 edits each. Real commits: `~/repos/nera-data-layer` first-parent `81228de89` (2b1504adc~50) to `2b1504adc`, replayed as bulk events on a fresh worktree cloned from base (first event = the whole tree at `81228de89`, 7,818 file changes).

## 2. Results

Timing rows are the median of three runs with the spread in brackets.

| Gate | Target | Measured | Result | Run directories |
|---|---|---|---|---|
| B1 incremental ≡ from-scratch | 0 differences | **0** differences: 1,112 checkpoints, 1,576 worktree checks (edited worktree after every event, all worktrees at load and every 100 edits), R1, R2 and R3 compared as sets, multiplicity 1 enforced | **PASS** | `20261022-121844-exactness-flat`, `20261022-120135-exactness` (earlier build), variant `20261022-121844-exactness-clusters` (0 of 1,576) |
| B2 single-file edit latency, 40 worktrees loaded | p50 ≤ 10, p95 ≤ 50, p99 ≤ 200 ms | n = 1,050: p50 **0.170** [0.169-0.171], p95 **0.388** [0.387-0.393], p99 **0.939** [0.930-0.956], max 4.0 ms | **PASS** | `20261022-121136-timing-flat-1`, `-121147-timing-flat-2`, `-121158-timing-flat-3` |
| B3 resident memory at steady state | ≤ 200 MB | RSS after the 1,000 edits (40 worktrees + 5 clones, R1-R3): **255.0** [254.8-256.4] MB; after load 251.5 MB; of which 87.8 MB is process + fact store + interner before any dataflow | **FAIL** (no kill: < 500 MB) | same three |
| B3, variant: two-level cluster bases | ≤ 200 MB | **195.1** [193.2-198.3] MB; exact | pass (variant) | `20261022-121143-timing-clusters-1`, `-121154-…-2`, `-121205-…-3` |
| B3, variant: per-path plurality base | ≤ 200 MB | 176.3 MB after load, 197.1 MB after the replay (one run) | pass (variant, one run) | `20261022-122500-variant-plurality` |
| B4 per-worktree state | ∝ divergence; clone ≈ findings only | 2.01-2.74 MB per worktree (91.4 MB for 01-40); Pearson r = **0.9987** against net delta fact rows over the 35 worktrees without bulk events, 11.5-15.6 bytes per net fact row. A clone holds no delta records until edited (zero input updates), 1.1-4.1 KB after its 10 edits | **PASS** | `20261022-121844-track-flat`, `20261022-122050-divergence` |
| B5 new worktree / 200-file bulk | ≤ 1 s / ≤ 2 s | **0.110** [0.106-0.110] ms / max **45.8** [45.1-46.0] ms (p50 34.6) | **PASS** | timing-flat-1..3 |

Kill criteria: B1 passes, so no stop. B3 is 255 MB, under the 500 MB threshold, so the partially-ordered-time variant is not triggered. B2 is far inside the target on all three rules together, so R3 needs no separate treatment.

Further measured rows (same timing runs unless named):

| Measure | Flat (gate design) | Cluster bases |
|---|---|---|
| Load: 40 deltas, one epoch per worktree | 3.65 s [3.64-3.67] (95-116 ms per worktree); base epoch 0.16 s | 1.16 s (8 cluster bases + 40 worktrees) |
| Arranged records (end, incl. commit worktree) | 6,087,019 | 3,133,202 |
| Single-file edit p50/p95/p99 by kind (ms) | call 0.161/0.351/0.824; delete 0.182/0.516/1.255; import 0.165/0.265/0.406; rename 0.179/0.384/1.191; revert 0.165/0.367/0.614; whitespace 0.146/0.353/0.515 | within 5% |
| Edits on new worktrees (n = 50) p50/p95/max | 0.181/0.427/0.623 ms | 0.177/0.475/0.594 ms |
| Real commits (n = 50, 0-1,418 files) p50/p95/max | 11.2/63.0/121.6 ms (max = the 1,418-file commit) | 11.4/63.0/120.5 ms |
| Real commit tree onto a base clone (7,818 files) | 424.7 ms [420.5-426.2] | 400.2 ms |
| RSS at end (with the 7,818-file commit worktree) | 353.0 MB | 301.2 MB |
| Peak RSS (`/usr/bin/time -l`) | 355.4 MB | 303.6 MB |

Step 7 (naive worktree-keyed R1, same replay up to the commits, R1 only; three runs each):

| R1 only | base + delta | naive (every worktree holds its full membership) |
|---|---|---|
| Arranged records | 2,333,300 | 13,341,222 (5.7×) |
| RSS after 1,000 edits | 160.5 MB [160.5-161.8] | 402.6 MB [402.3-403.7] |
| Load 40 worktrees | 1.33 s | 7.92 s |
| New worktree (clone of base) | 0.012 ms | 313.6 ms p50, 367.6 max |
| Single-file edit p50/p95/p99 | 0.025/0.057/0.110 ms | 0.029/0.074/0.131 ms |
| 200-file bulk max | 9.4 ms | 13.0 ms |
| Runs | `20261022-121215-timing-r1-basedelta-1`, `-121227-…-2`, `-121240-…-3` | `20261022-121217-timing-r1-naive-1`, `-121229-…-2`, `-121242-…-3` |

Extraction (step 1, separate from steady state; 7,091 contents, 4 workers, three runs `20261022-121252-extract-1`, `-121329-extract-2`, `-121407-extract-3`): per-file parse + extract p50 **0.98 ms** [0.97-1.05], p95 12.0, p99 36.3, max 75.1; 20.6 s CPU, 6.1 s wall. The corpus scan (read + blake3 of 262,567 files) took 34.5 s and is not included. An earlier run at load 19.7 (`20261022-112444-extract`) is untrusted and not used.

## 3. How measured

From the worktree root, after `cargo build --release -p facts-b` with `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target`. `spikes/facts-b/scripts/run.sh <label> [--lock] -- <args>` creates the run directory, records commit, command and uptime, wraps the command in `/usr/bin/time -l` (and `bench-lock` with `--lock`), and passes `--run-dir`. `E=~/rerail-bench/facts/spike-b/edits/seed-5eedb/events.jsonl`.

| Row | Command |
|---|---|
| Extraction, memberships | `run.sh extract-N --lock -- extract --threads 4` (writes `~/rerail-bench/facts/spike-b/membership/NN.tsv` and the store) |
| Base, divergence | `run.sh divergence -- divergence` (writes `membership/base.tsv`); `… -- divergence --plurality` (writes `base-plurality.tsv`) |
| Edit script | `target/release/facts-b edits --out ~/rerail-bench/facts/spike-b/edits/seed-5eedb --commits-repo ~/repos/nera-data-layer` (deterministic; regenerating rewrites the same events) |
| B1 | `run.sh exactness-flat -- replay --events $E` (`--clusters` for the variant). `results.json` → `exactness` |
| B2, B3, B5 | `run.sh timing-flat-N --lock -- replay --no-oracle --events $E` (×3; `--clusters` for the variant); summarize with `spikes/facts-b/scripts/summarize.py <run dirs>` |
| B4, hot keys | `run.sh track-flat -- replay --no-oracle --track --events $E` (`state`, `hot_keys` in `results.json`); divergence from the `divergence` run |
| Step 7 | `run.sh timing-r1-basedelta-N --lock -- replay --no-oracle --only-r1 --events $E --limit 1060`; same with `--naive` |
| Plurality variant | `run.sh variant-plurality --lock -- replay --no-oracle --plurality --events $E` |

Latency is wall time from offering the event's updates to the input session to the probe passing the event's epoch; every output (R1-R3 findings, inspected into the driver's maps) is upstream of the probe. Facts of contents an event introduces are loaded from disk before the clock starts (`fact_load_seconds_total` reports it). RSS is `proc_pid_rusage` resident size sampled every 100 edits; "steady state" is the sample after edit 1,000, before the real-commit worktree is created. The oracle is off in timing runs; it is on in exactness runs, which are not timed.

## 4. Exactness artifacts

- `20261022-121844-exactness-flat/results.json` → `exactness`: `checks` 1,112, `worktree_checks` 1,576, `differences` 0, `first_differences` []. Same for `…-exactness-clusters`. `20261022-120135-exactness` (commit `5576e4819`) also 0 of 1,576.
- The comparator (`replay.rs`, `full` and `diff_sets`) rebuilds each worktree's findings as base output + the worktree's tagged deltas (tags 0, its cluster, itself), fails on any multiplicity other than 1, and compares as sets against `oracle::evaluate` over the worktree's current membership. The oracle is naive Rust (hash maps, BFS) with no base, no deltas and no dataflow.
- Shared between oracle and dataflow, by design: the per-file derivations (`derive.rs`, a pure function of path and content) and the per-group functions of R2 (`rules::owner_families`, `rules::family_issues`, applied to a complete group). The gate therefore checks the cross-file machinery (joins, counts, presence flips, group recomputation, recursion, activation), which is what B owns.
- Initial findings per worktree, e.g. worktree 01: R1 2,147, R2 2,589, R3 113; base: R1 2,030, R2 2,507, R3 27.

## 5. Design notes

### 5.1 Exported fact schema (v0, B's part)

All relations are per content (blake3), path-independent, `sym` = per-file string-table index re-interned per process.

| Relation | Columns | Rows | Notes |
|---|---|---|---|
| `def` | `name:sym, kind:sym, qualname:sym, line:u32` | 246,411 | `function_definition` not inside a function (Jeremy's `raw_functions`) and `class_definition` not inside a function. `kind`: `function`, `method` (nearest enclosing class), `class`. `qualname`: `Class.name` for methods, `name` otherwise (Jeremy's id without the module). `line`: 1-based start of the definition node. |
| `ref` | `name:sym` | 1,228,411 | distinct `identifier` texts, excluding the `name` field of function/class definitions |
| `import` | `module:sym, name:opt_sym, local:sym, top:u8, module_scope:u8` | 268,944 | source order. `module` as written, leading dots kept (`..pkg`, `.`); `name` None for `import M`; `local` = alias, or first component for `import a.b`; `top` = direct child of `module`; `module_scope` = not inside a function or class |
| `call_edge` | `caller:sym, callee:sym` | 711,687 | distinct; `caller` = qualname of the enclosing declaration (calls in nested functions belong to it); `callee` = dotted text of an identifier/attribute chain; dropped when the head is bound locally in the caller (parameters, assignment/for/with/except/walrus targets, nested defs, local imports; `global` names and `self`/`cls` kept) |
| `def_decorator` | `qualname:sym, name:sym` | 47,217 | Jeremy's `^@\s*([^\s(]+)` |
| `dunder_all` | `literal:u8` | ≤ 1 | first module-level `__all__ = …` outside def/class/if/try; literal = list or tuple |
| `dunder_all_name` | `name:sym` | 15,734 | string elements of that literal (`string_content` concatenated) |
| `seed` | `qualname:sym, family:sym` | 95 | R3 targets, § 5.4 |
| `syntax_error` | (none) | 0 or 1 | the content does not parse cleanly |

File format (`~/rerail-bench/facts/store/<h[0..2]>/<h>.xfacts`, agreed with Spike A, whose `.facts` uses the same layout): little-endian; magic `RRFACTS\0`, `u32` schema version (0), 32-byte content hash, `u32` string count, strings as (`u32` length, UTF-8), `u32` relation count; per relation: name (string index), `u32` rows, `u32` columns, per column (`u32` name index, `u8` type: 0 u32, 1 u16, 2 u8, 3 sym, 4 opt_u32, 5 opt_sym, `u32::MAX` = null); then each column at native width, zero-padded to 4 bytes. Readers skip unknown relations. Encoder and decoder: `facts.rs` (`encode`, `decode`; `facts-b dump FILE` round-trips).

Volume: on disk 58.3 MB for 7,091 contents; per content mean 8,227 B, p50 4,107, p99 83,695, max 152,163 (`extract-*` `results.json`). Resident for the 40 worktrees' 7,057 member contents: 20.5 MB of integer columns (`20261022-113914-initial-check` → `resident_facts`; 261,048 interned strings), plus the interner; process + store + interner = 87.8 MB RSS before the dataflow is built.

### 5.2 Dataflow

One timely/differential dataflow, one worker. Every relation is a single collection of tagged records `(w, record)`: tag 0 is the base, tag w ≥ 1 is worktree w's signed delta X_w − X_B (in the cluster variant a worktree's tag is relative to its cluster base, whose own tag 100+c is relative to the base). The only input is `files: (w, (path, content))`: the base membership under tag 0 and each worktree's membership delta (+ its content, − the base's) under its tag. An edit is two updates, a new worktree cloned from base is zero updates. Per-file derivations (`derive.rs`) run in `flat_map` over that input, so a relation's records exist once per (tag, path, content), and a keyed relation is arranged once (`key → (w, value)`), base and every delta in the same arrangement, shared by every operator that reads it.

Operators (the expansion a compiler emits; `flow.rs`):

- **map/filter/flat_map**: per record, tag kept.
- **join**: one `join_core` over the two shared arrangements; a pair (w1, w2) produces tag 0 if both are 0 (A_B⋈B_B), w if one is w and the other 0 (ΔA⋈B_B, A_B⋈ΔB), w if both are w (ΔA⋈ΔB), nothing across worktrees. In the cluster variant, any pair on one ancestor chain yields the deeper tag.
- **presence (distinct)**: one `reduce` keyed by the record over tagged counts: the base count c and each worktree's count delta d_w; emits tag 0 once if c > 0, and tag w with diff `[c + d_w > 0] − [c > 0]` (counts for negation). Only keys whose count changes in w carry a w value.
- **antijoin**: `A ▷ K = A − A ⋈ presence(K)`; join is bilinear, so the ±1 presence deltas are exact.
- **group reduce**: one `reduce` keyed by the group over tagged members; emits `logic(base)` under 0 and, for each worktree with members at that key, `logic(base + Δ_w) − logic(base)` under w.
- **recursion (R3)**: `iterate` over reach records `(node, (w, family))` for the base and, in full, for each *active* worktree: one whose file delta contains a content with `call_edge` or `seed` facts (cluster deltas activate the cluster's worktrees). A reach record of worktree w joins facts tagged 0 or w (or w's cluster). No edge relation is materialized: the step walks reached node → `r3_target_by_node` → key → `r3_callsite` → caller, plus `r3_local_by_callee`. Worktrees that are not active read the base result.

Findings are keyed by symbol, not position: R1 by (path, qualname), R2 by (issue, family, exemplar path and name). Line numbers are presentation, attached from the current content's `def` facts at delivery; carried in records, every edit above a definition churns the dataflow. Arranged records after loading the 40 worktrees, as the design evolved (dev builds, oracle on): two arrangements per relation with lines 9.73 M (`20261022-114351-dev-initial`), one tagged arrangement per relation without lines 6.24 M (`20261022-114754-dev-initial`), no materialized R3 edges 5.31 M (`20261022-115141-dev-initial`); all three exact.

### 5.3 Delta expansion per rule

Notation: Δ = the worktree's tagged records; `c` counts; `pres(K)` the presence relation of K (0/1 under tag 0, ±1 under tag w).

**R1 dead code** (`dead = cand ▷ pres(ref)`):

- `ref_w(n)` multiset over member files; `cand(n → (path, qualname))`.
- `pres(ref)`: per name, `[c_B(n) > 0]` under 0; under w, `[c_B(n) + Δc_w(n) > 0] − [c_B(n) > 0]` for names with Δc_w(n) ≠ 0.
- `dead_B = cand_B − cand_B ⋈ pres_B(ref)`.
- `Δdead_w = Δcand_w − (Δcand_w ⋈ pres_B(ref) + cand_B ⋈ Δpres_w(ref) + Δcand_w ⋈ Δpres_w(ref))`.

**R2 public API families**:

- `decl((module, name) → (path, flags))`, `bind((module, local) → (module', name'))`, `export((module, name) → (export module, export name))`.
- Resolution, hop h = 0..4: `roots_h = cur_h ⋈ decl`; `cur_{h+1} = (cur_h ▷ pres(keys(decl))) ⋈ bind`, each join in the four-term form above. `roots = pres(⋃ roots_h)`.
- `surfaces = flat_map(roots)` (per record; mixed names go straight to `invalid_mixed_lazy_execution_name`).
- `families = reduce_owner(surfaces)`: groups (export modules) with a member delta in w are recomputed as `owner_families(base members + Δ_w)`; Δ = new − base.
- `issues = reduce_(base_name, implementations)(families)`, same form, `family_issues` keeps the first owner by name and emits `missing_sync`/`missing_async`/`missing_lazy`.

**R3 reachability**:

- `R_B = μR. seeds_B ∪ callers_B(R)`; for active w, `R_w = μR. seeds_w ∪ callers_w(R)` with `seeds_w = seeds_B ∪ Δseeds_w` and `callers_w` reading facts tagged 0 or w (`target_B + Δtarget_w`, `callsite_B + Δcallsite_w`, `local_B + Δlocal_w`, so each edge's multiplicity is its base plus delta multiplicity, ≥ 0, thresholded to presence per iteration).
- `found_x = callers_x(R_x)` restricted to in-scope callers; output under w: `found_w − found_B`.
- Demand-driven: the iteration starts at the seeds and walks callers backwards; nothing that cannot reach a seed is materialized. Per active worktree the state is its reach set (about 138 nodes) and its findings, not its tree.

### 5.4 Rule semantics and scope simplifications

The extractor is a stand-in for Phase 2's; B's gate is incremental ≡ from-scratch, so each rule below is a precisely defined rule over the exported facts. Simplifications narrow scope; none approximates within the scope.

- Every rule reads only contents without syntax errors (Jeremy's `syntax: valid`). A declaration's duplicates by qualname keep the last (max line), as Jeremy's `declarations` keeps the last by start byte per id.
- **R1** (from `data-layer-internal-dead-code`): a candidate is a `function`/`method`/`class` declaration in a file of the rule's include/exclude scope, module not `^(api|data_layer)\.`, path not `impl/browser/`, name not `^__.*__$` and not `main`/`pytest_configure`/`pytest_collection_modifyitems`. It is dead when no member file of the worktree (any path, tests included) has a `ref` of its name. Narrowed: Jeremy's reachability over reference edges, hierarchy, dispatch, annotations and dynamic imports, and his protocol/abstract/property exemptions, are replaced by name presence: the rule here is "no identifier anywhere in the worktree spells the name".
- **R2** (from `data-layer-public-api-triples`, option `executionOnly = false`): export files `^data_layer/[^/]+\.py$`; export names = top-level symbol imports (after Jeremy's last-binding-per-local dedupe over all symbol imports) and top-level declarations, without a leading `_`, filtered by the first module-level `__all__` (literal: listed names only; non-literal: none). Targets resolve through `(module, name)` declarations or, where none exists, the module's symbol binding, up to 4 re-export hops (Jeremy's `resolved_names` is unbounded; roots needing more hops are out of scope). Census over the base and the 40 initial memberships (`20261022-122757-r2-hops/stdout.txt`, `facts-b hops`): 57,111 roots resolve at 0 hops, 113,230 at 1, 7,878 at 2, 287 at 3, 82 at 4, none deeper (26,375 resolve to nothing in the workspace, e.g. third-party imports), so on this corpus the bound changes no result. Relative imports are resolved Jeremy's way (package prefix minus one component per extra dot). Function scope only: surfaces, slots (`_lazy`, `_async` or a `legacy_async` decorator, else sync; mixed names reported), `first_ordinal`/exemplar, the `cases/false` eligibility exceptions, the cross-owner dedupe, and the issues `missing_sync`, `missing_async`, `missing_lazy`, `invalid_mixed_lazy_execution_name`. Out of scope: method scope (needs the class hierarchy), `missing_legacy_*`, `missing_lazy_decorator`, `direct_wrapper_bypasses_lazy_owner` (need parameters and execution calls). Ordering ties Jeremy's SQL leaves unspecified (one export name resolving to two symbols) break by (path, name).
- **R3** (from `serverplan_state_tripwires` `calls` terms): seeds are declarations whose subtree has `.materialized_pipeline` or `getattr(ctx, "materialized_pipeline")` with a string literal (family `mp`, SPT-012's `mp-anchor-edge` target; the module-constant variant of the second argument is out of scope), or a call to `execute_runtime_nested_steps_sync|async` by name or attribute (family `nested`, SPT-005's first disjunct; the `PlanExecutor` chain disjuncts are out of scope). A finding is an in-scope (policy scope: `impl`, `data_layer`, `_internal`, `l####_*`, no test dirs) function with a call edge to a function that is a seed or reaches one; the graph spans every member file. Call resolution, per file: a bare name bound by a top-level function (or class → its `__init__` in the same file) and not by a module-scope import; a bare name bound only by a module-scope `from M import n` → (absolute M, n); `self.m`/`cls.m` in a method → the same class's method; `C.m` for a local class; `a.b.f` with `a` bound by `import` → module `a.b`, name `f`. A name bound by both a definition and an import is refused (no edge). Out of scope: MRO, `from M import x; x.f()`, `mod.Class.method()`, callbacks, re-export chains.

### 5.5 Divergence, hot keys

Divergence across the 40 worktrees (`20261022-122050-divergence`): delta paths 104-818 (median 285, total 16,415); raw delta fact rows 140k-604k; **net** delta fact rows (per path, the multiset symmetric difference of exported rows between the base content and the worktree's) 140k-232k, total 7.05 M against 1.10 M in the base. The distribution is two-level: worktrees 01-16 (commit dates up to 2026-09) differ in 504-818 paths; 17-40 in 104-339. About 106 paths have no content held by 21 worktrees, so the strict (path, hash) majority leaves them out of the base and every worktree carries the whole file (worktree 20: 106 paths, all additions, 140k net rows). Per-path plurality keeps them (5,562 base files) and cuts total net divergence to 2.28 M (`20261022-122146-divergence-plurality`); two-level cluster bases cut the summed worktree deltas to 1,559 paths plus 3,643 in the 8 cluster bases.

Hot keys (`20261022-121844-track-flat` → `hot_keys`): the widest reduce keys are R1 presence keys (a name) holding the base count plus one value per worktree whose count differs: 23,374 names carry values from ≥ 40 tags (`session_id`, `typing`, `exception_translation`, … 43). A touch re-evaluates the key over ≤ 44 tagged counts. Edit latency grouped by the widest key the edit touches shows no cost at this width: fan-in 0 p50/p95 0.246/0.638 ms (n = 503), >40 0.282/0.678 ms (n = 95) (tracking on, so absolute numbers are above the timing runs). Group reduces in R2 are keyed by export module and family; R3's iteration is keyed by node with worktree in the value.

## 6. Risks found

1. **The base choice drives memory.** The strict (path, hash) majority of the spec leaves churned files out of the base and makes every worktree carry them; with it, B3 is 255 MB. Per-path plurality (176 MB loaded) or cluster bases (195 MB after the replay, exact) both pass. Phase 2/3 should pick the base per path, and consider hierarchical bases: neighboring worktrees repeat each other's deltas (01-03 share an 818-path delta). The tag algebra generalizes to a tree; the cluster variant is that generalization at depth two.
2. **A fixed memory floor**: 88 MB of the RSS is the process, the fact store (20.5 MB of columns for 7,057 contents) and the interner, before any dataflow. Facts for edited and committed contents accumulate (15,717 contents by the end of the replay); nothing evicts them. Phase 3 needs eviction by membership or memory-mapped fact files.
3. **R3 materializes per worktree.** Every worktree here is active (all deltas carry call facts), so the reach set is held 40 times (5.5k records); cheap with 95 seeds, linear in seeds × worktrees. The activation test is per worktree, not per affected node.
4. **Rules are stand-ins.** R1 is name-presence based; R2 has no method scope and bounded hops; R3 resolves a subset of calls and two seed families. Exactness holds for these definitions; Phase 2 must re-derive the delta form for Jeremy's full stages (the operator set above covers joins, antijoins, counts, group aggregates and monotone recursion; window functions map to group reduces).
5. **Hot keys grow with worktree count.** A presence key carries one value per diverging worktree; 44 here with no measurable cost, but every touch is O(worktrees at the key).
6. **Measurement notes.** Per-worktree state is logical (live delta records × in-memory update size, `--track`), not allocator bytes; RSS is the ground truth for B3. Differential's logging is registered in every run (for arrangement sizes). Allocator retention makes RSS exceed the live footprint after transient peaks (end-of-run footprint 198 MB vs RSS 300 MB in `20261022-115141-dev-initial`).

## 7. Resume notes

- Branch state: all code committed (`spike-b:` commits up to the report commit). Nothing half-done for the gates.
- Reproduce: § 3. Store and memberships live under `~/rerail-bench/facts/`; `extract` and `divergence` recreate them, `edits` recreates the replay.
- Not done: a three-run timing set for the plurality variant; trace-level (allocator) bytes per worktree; fact eviction. The R2 hop census covers initial memberships only, not every replay step (the oracle uses the same 4-hop bound, so B1 is unaffected).
- Code map: `facts.rs` extractor and format; `store.rs` interner and content store; `base.rs` base and deltas; `derive.rs` per-file derivations; `rules.rs` R2 group logic; `flow.rs` dataflow and tag algebra; `oracle.rs` from-scratch evaluation; `edits.rs` replay generator; `replay.rs` driver and measurements.
