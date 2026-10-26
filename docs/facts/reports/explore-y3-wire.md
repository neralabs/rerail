# Exploration Y3: Y1 + Y2 wired into one measured cross-file path

Branch `Moti-Velz/facts-engine-y3-wire` (worktree `.worktrees/y3-wire`), from integration `b501baba8`, with P3a's `Moti-Velz/facts-engine-p3a-memory` merged (`040948202`: M2, D1). Code: `spikes/facts-b/src/y3/` (`input`, `sym`, `wire`, `m2`, `world`, `replay`), `spikes/facts-b/src/lmap.rs`; Y2's `Facts`/`Dc`/`Ef` made layered. Runs: `~/rerail-bench/facts/explore/y3-wire/runs/`. Binaries: `…/y3-wire/facts-b-y3a` (checked run), `facts-b-y3b` (perf run). Every figure below is one run on this machine; counts are deterministic, milliseconds are not.

## Status

- **Exact**: base graph = native (Y1, unchanged); the maintained symbol table = the dataflow's stages on the oracle tree (1,098,978 rows, 12 tables) and on the 300-event prefix (381 checks: per-event presence changes and whole states); the wired path on the **full replay: 1,151 checks (40 loads + 1,111 events), 0 differences** — after every event, in a forked child, Y1's graph, the symbol table, Y2's edge index, both Y2 rules (findings and reached sets) and the three M2 plans from scratch against what was maintained.
- **No differential dataflow in the path.** The nine `proj_*` inputs and the per-definition traits now come from per-content exports + keyed tables + memoised queries (`y3/sym.rs`), not from the dataflow.
- **`CrossFile`**: `y3::world::World` implements facts-b's mirror of the daemon's trait (`apply(worktree, members, delta) -> FindingsDelta`, `forget`, `held_findings`); the daemon is not wired.
- **Compaction (item 5): not done** (budget). The breakdown below says where it pays.

## 1. The path per edit

per-content exports (one file: the X6 `symbols-merged` xexport feeds both Y1's `cgn_*` rows and the symbol table) → **Y1 `apply`** (returns the step's DefId pair delta, the re-read paths' old records and per path its pairs before/after, new in `Step.old_recs` / `Step.out_pairs`) → **edge delta as Y2 `Refs` presence changes** (`wire::refs_delta`: pairs out of recomputed paths, plus pairs into re-read paths through a maintained reverse index `target def → source defs`, because their target's position moves) → **symbol table step** (`SymState::apply`: block rows out/in, changed keys, readers re-run lowest level first with early cutoff, registration of new keys, output queries' old/new values diffed into presence changes) → **Y2 scoped** (`Dc`/`Ef` `prepare` → `Facts::apply` → `finish`, unchanged logic) → findings delta. The three small plans: **M2** (`y3/m2.rs`, P3a's `hybrid` as a struct): D1 per plan, else `batch::eval` in process.

Every component is layered as Y1: base (strict-majority membership) → 8 cluster overlays (plurality) → worktree delta. Overlays and worktrees are built by the *same step as an edit* (the diff from the layer's membership); the base by the step from nothing plus Dc/Ef from scratch. Generic layering is `lmap::LMap` (frozen `Rc` layers + a top holding only differing keys) and `lmap::Rev` (append-only layered reverse index).

**Deviation from the brief, agreed nowhere yet:** Y2 does not read Y1's materialised `GraphView` (23 ms per build, not incremental). It reads a position-keyed edge index in its own layered `Facts`, maintained from Y1's step delta (no `reference_edges` recompute). Y2's `prepare` must read the *old* graph after Y1 has applied, which a live view over Y1's mutated delta cannot give.

## 2. The symbol table (the nine inputs)

| query | key | SQL stage | reads |
|---|---|---|---|
| `decl` | id | `proj_declarations` (window by id) → `dc__dead_traits`, `ef__traits`, `proj_annotation_names` rows of the selected candidate | `T_CAND(id)` |
| `imp` | (module, name, base IS NULL) | `proj_imports` window | `T_TOK` |
| `res` | fqname | `proj_resolved_names` (closure as a search per name) | `T_FQ`, `T_IMPFQ` per name reached, `imp` |
| `bt` | (module, short) | `proj_base_edges`' three-way `coalesce` | `imp`, `T_FQ`, `T_MN`, `T_CLSN`, `decl` |
| `pb`, `be` | path, class | `proj_base_edges` rows | `T_PATH`, `decl`, `bt`, `pb` |
| `hier` | class | `proj_hierarchy` (BFS depth, least route) | `decl`, `be` |
| `pub` | class | `proj_public_methods` window | `hier`, `decl`, `T_MCLS` |
| `roots` | path | `proj_export_names` ⋈ `res`, `__all__` gate | `T_PATH`, `imp`, `decl`, `res` |

Per content (no table, no query; `y3/input.rs`): candidate traits, annotation names and `params` JSON, import tokens resolved against the path's module (SQL `LIKE`/`trim`/`rx_*` reused from `rerail_facts::sql::value` for identical semantics), base rows, `__all__` rows, module names, the two probes. Costs: from scratch over the oracle tree 0.43 s (X5 measured 2.3 s for `imports`+`resolved_names`+`hierarchy` alone on M2's executor); per single-file edit 0.11 / 0.67 / 1.45 ms (table below).

## 3. Performance characteristics (the deliverable)

Runs: perf `runs/20261025-210541-full-perf` (no checks forked; numbers below), checked `runs/20261025-205805-full-checked` (same binary family, 10 check workers: latencies ≈ 2–4× higher from fork/COW contention), symbol table vs dataflow `runs/20261025-symcheck-{oracle,300}`, per-index breakdown in the perf run's `results.json` (`memory[*].detail`).

### (a) Memory, 40 worktrees (MB; accounted bytes = capacities; heap = malloc in use)

| component | base | 8 overlays | 40 deltas at load | 40 deltas at edit 1,000 | at end (46 wt, 50 commits) | content store can serve? |
|---|---|---|---|---|---|---|
| graph (Y1 tables, memos, paths, edges) | 71.8 | 193.3 | 32.1 | 53.5 | 200.7 | no (maintained state) |
| graph ids (Y1 query ids 28.2, def ids 1.8) | 30.0 (shared) | | | | 47.3 | no |
| edge reverse index (DefId) | 1.4 | 6.8 | 1.3 | 2.6 | 4.4 | no |
| symbol tables (keyed tables, memos, readers, parents) + query ids 14.1 | 60.3 | 139.3 | 32.3 | 54.2 | 139.3 | no |
| Y2 inputs (`Facts`: 30 layered indexes incl. edge positions) | 46.9 | 166.5 | 92.1 | 142.8 | 224.2 | no (a copy of the two above, re-indexed) |
| reached sets (`Dc`: reached+levels, dispatch indexes, imported, findings) | 3.0 | 92.3 | 15.5 | 26.7 | 54.6 | no |
| `exported-families` state (`Ef`) | 3.3 | 5.3 | 0.2 | 0.6 | 3.4 | no |
| per-file inputs: Y1 summaries 165.2, symbol blocks 87.7, M2 exports 0.14 | 253.0 (all 19,801 contents) | | | | 253.1 | **yes** |
| strings: Y1 interner 212.6, Y2 interner ≈ 72.2 (est.), M2 interner 1.8 | 286.6 | | | | 306.8 | ≈ 123 MB of Y1's are summary strings (Y1 report): yes |
| plans/results: M2 members ≈ 9.0, results 0.13; findings held 108,614 → 137,799 | 9.1 | | | 10.2 | 10.6 | no |
| **heap in use** | | | **1,940** | **2,087** | 2,589 | |

Reading: graph state 297 MB (Y1 alone, as before) plus symbol tables 232, Y2 inputs 306, reached sets 111 at load; ≈ 950 MB maintained, ≈ 540 MB of per-file inputs and strings the content store should serve, ≈ 390 MB unattributed (hash-map slack, Y1 shared state not in the component sums). After 1,000 edits deltas grow +129 MB (Y1 +21, sym +22, Y2 inputs +51, Dc +11); the last 111 events (6 new worktrees over the base, 50 commits) add +500 MB, mostly as Y1 saw.

Where it is (per-index breakdown, `memory[*].detail`): Dc overlays hold **2.29M reached entries (62 MB)** against **94,739 in the base**: the strict-majority base reaches far fewer states (dispatch) than any cluster, so every overlay re-adds them — the base for `Dc` should be a cluster's reached set, or dispatch keyed by class (Y2's Jeremy item 2). Y2 inputs' overlays/deltas are dominated by `refs_to`/`refs_from` (21+21 MB overlays, 41+24 MB deltas at edit 1,000: positions move on every re-read path) and `defs`/`ef` (position payloads): keying Y2 by Y1's DefId (Y2 Jeremy item 3) removes most of both. Symbol-table overlays: `cand`/`tok` tables 30 MB and `decl`/`imp`/`pub` memos 19 MB, readers 19.5 MB.

### (b) Per-edit latency, ms (p50 / p95 / p99), perf run

| event | n | total | Y1 apply | edge delta | symbol table | Y2 update (dc+ef+inputs) | M2 recompute | max |
|---|---|---|---|---|---|---|---|---|
| single-file, all | 1,050 | **0.53 / 3.76 / 8.62** | 0.18 / 1.07 / 2.00 | 0.01 / 0.16 / 0.43 | 0.11 / 0.67 / 1.45 | 0.10 / 2.19 / 4.12 | 0.02 / 1.08 / 1.37 | 202 |
| — < 4 KB | 359 | 0.27 / 2.61 / 4.51 | 0.09 / 0.62 / 1.59 | 0.00 / 0.19 / 0.54 | 0.05 / 0.32 / 0.91 | 0.05 / 1.27 / 2.93 | 0.02 / 1.13 / 1.38 | 16 |
| — 4–16 KB | 385 | 0.44 / 1.89 / 3.42 | 0.17 / 0.37 / 0.64 | 0.01 / 0.04 / 0.23 | 0.11 / 0.34 / 0.55 | 0.09 / 0.90 / 2.71 | 0.02 / 0.03 / 1.12 | 202 |
| — 16–64 KB | 236 | 1.04 / 4.27 / 25.6 | 0.39 / 0.91 / 1.19 | 0.02 / 0.08 / 0.16 | 0.24 / 0.67 / 0.97 | 0.20 / 2.74 / 24.6 | 0.02 / 0.04 / 1.19 | 36 |
| — ≥ 64 KB | 70 | 2.79 / 9.61 / 23.5 | 1.15 / 2.84 / 3.55 | 0.07 / 0.39 / 0.55 | 0.67 / 2.22 / 3.26 | 0.54 / 4.12 / 11.5 | 0.02 / 1.26 / 1.39 | 29 |
| new worktree (clone of base, 6) | 6 | 10.6–13.3 | ≈ 0 | 0 | ≈ 0 | ≈ 0 | 10.5–13.3 (3 plans from scratch) | 13 |
| 200-file bulk | 5 | 387 / 550 / 550 | 67 / 109 | 7 / 12 | 40 / 79 | 231 / 371 | 1.5 / 2.1 | 550 |
| real commit | 50 | 63 / 470 / 760 | 15 / 120 / 157 | 1.4 / 10 | 12 / 111 / 180 | 32 / 299 / 391 | 2.6 / 5.6 / 19 | 760 |
| commit creating a worktree | 1 | 2,610 | 977 | 26 | 481 | 1,042 | 74 | 2,610 |

M2 ran 105 of 1,111 events (only `required-contracts`; D1 skipped `state-flow-project-1` and `quality-project-taxonomy` on every event). One 4–16 KB edit took 202 ms (Y2 dirty set on a hub; not investigated).

### (c) Scaling

- **New worktree vs divergence** (the 40 loads, step over its cluster layer, M2 excluded): divergence 0–5 paths: 4 ms, 0.46 MB; 6–20: 15 ms, 1.6 MB; 21–50: 87 ms, 4.8 MB; 51–200: 234 ms, 15.3 MB (medians). ≈ **2.9 ms and 156 KB per diverging path** (all components; Y1 alone was 29 KB). M2 adds ≈ 12 ms per worktree (3 plans from scratch). Overlay build: 106–818 paths, 0.48–0.96 s each.
- **Delta bytes vs diverging files**: as above at load; at edit 1,000 Σ deltas 280 MB over 40 worktrees.
- **Per-edit cost vs edges touched** (Y1 pairs added+removed + `Refs` rows changed): 0 → 0.26 / 1.54 ms (n 216); 1–10 → 0.31 / 2.49 (208); 11–100 → 0.57 / 2.94 (457); 101–1,000 → 1.87 / 7.1 (160); > 1,000 → 6.4 / 28.9 (9).

### (d) Counts per single-file edit (p50 / p95 / max)

Y1 table rows touched 273 / 2,154 / 13,562, queries re-run 0 / 7 / 96, pairs ± 1 / 11 / 366; `Refs` rows changed 15 / 333 / 2,894; symbol table rows touched 191 / 1,302 / 8,838, queries re-run 54 / 337 / 2,078, presence changes out 26 / 313 / 3,926; dead code dirty states 24 / 768 / 29,975, `succ` calls 46 / 1,538 / 362,130, states toggled 0 / 6 / 128; `exported-families` sources 10 / 116 / 1,672 (checked run's counts; identical by construction).

### (e) The floor

- Same contents re-applied (every change a no-op, sampled on 105 edits): **0.008 / 0.014 / 0.017 ms** — the dispatch only (no-op changes are dropped before Y1).
- Edits after which nothing changes anywhere (Y1 no key changed, no symbol-table or `Refs` change, D1 skipped all three M2 plans; 124 whitespace-like edits): 0.26 / 1.94 ms total: Y1 re-reads the path (0.12 / 1.11), the symbol table touches the block's keys (0.09 / 0.84), Y2 ≈ 0, D1 0.02 / 0.04. The floor of a real content change is the re-read of the changed block, not the rules.

## 4. Exactness evidence

| check | result | run |
|---|---|---|
| symbol table vs dataflow, oracle tree `2b1504adc`, from scratch | 1,098,978 = 1,098,978 rows over Ann, Bases, DeadTraits, Dyn (by target), EfTraits, Hier, Imports, ModuleNames, Probe, Public, Resolved, Roots | `runs/20261025-symcheck-oracle` |
| symbol table vs dataflow, 300-event prefix, 41 worktrees | 381 checks (loads, each event's presence changes, end states), 0 differences | `runs/20261025-symcheck-300` |
| wired path, full replay, from scratch per event | 1,151 checks, 0 unequal (Y1 edges + counters, symbol table rows, Y2 edge positions, dead code findings + reached sets per layer, `exported-families` findings, M2 stored = batch) | `runs/20261025-205805-full-checked/checks.jsonl` |
| dead code 4,931 / `exported-families` 0 on the oracle tree | layered Y2 over the oracle dump: 4,931, 0, reached 67,286 / 67,288 / 761,397 (as Y2); the symbol table equals the dump there, so the wired inputs equal Y2's | `runs/20261025-y2-layered-oracle` (`y2 --dump …dump-oracle`); not re-run through `y3 replay` on the oracle tree |
| base graph = native | unchanged Y1 code path (`ygraph parity`, 145,431) | Y1 report |

**Runtime refusals**: at the end, worktree 50 holds a live refusal: Y1 `refused_paths: 2` and 4 members without exports (`no exports for …`; the replay's contents without retained source). P3a reported none live at the end for its original dead-code plan; this disagreement is open (§ 6). Y3 reports them as `FindingsDelta.failures`; findings are not withheld yet.

## 5. `CrossFile`

`spikes/facts-b/src/y3/world.rs`: `trait CrossFile { name; apply(worktree, members, delta: &[PathChange{path, old, new}]) -> FindingsDelta{introduced, removed, failures, work}; forget; held_findings }`, implemented by `World`. Contents are blake3 hex (the daemon's store maps `SourceId` to it). An unknown worktree is created over the base from `members`. Findings are `(plan, row text)`; the daemon adapter (facts-a `Finding`) and a facts-b lib target are not written (P3a open item 1 still holds for the lib target).

## 6. Open items for a verification pass

1. **I8**: every symbol-table query body is hand-written per SQL stage (`sym::exec`), as Y1's link queries and Y2's rules are; read sets are recorded generically. Evidence is differential (the dataflow on oracle + prefix, scratch on the full replay), not derivation.
2. **Y1's two linker deviations** (link at the caller's hops instead of `link_reach` with `d < 19`; hop sets instead of `[lo, hi]` with `lo = 0`) are unchanged; their counters `anomalies.{negative_lo, deep_nonempty, ord_lengths}` are in every `results.json` and were 0.
3. **Runtime refusal disagreement** with P3a (w50 live at end; § 4). Decide whether dead code/`exported-families` withhold findings while one is live, as the generator does.
4. **What the gate must gain**: the `y3 symcheck` (oracle + prefix) and `y3 replay --check-every 1 --check-load` as checks; `lmap` unit tests are in `facts-b-unit`; mutants for `sym` (e.g. dropping absent-key reads, the `decl` tie-break) do not exist yet.
5. **Not done**: compaction (item 5); the dead-code/`exported-families` anchor against M2's *original* plans per event (only the dataflow on the prefix and Y2's scratch on the full replay compare them); `GraphView` as Y2's reader (§ 1).
6. **Memory levers, by size**: Dc's base (−≈ 90 MB: build the reached set's base from a cluster, or dispatch by class); Y2 keyed by DefId (−most of `refs_*`/`defs`/`ef` deltas); one interner (Y1 213 MB + Y2 72 MB hold the same paths/names); frozen-layer arenas for Y1, `sym` and `Facts` (Y1's estimate 1.5–2×).

## 7. Reproduce

```
cargo build --release -p facts-b
facts-b y3 spec ~/rerail-bench/facts/explore/y3-wire/specs/spec-replay-m2-three.json
facts-b y3 symcheck --oracle --dump …/y2-local/runs/20261023-210008-dump-oracle/dump.tsv --out D
facts-b y3 symcheck --dump …/y2-local/runs/20261023-211927-dump-replay-300/dump.tsv --limit 300 --out D
facts-b y3 replay --out D [--check-every 1 --check-load --workers 10] [--limit N]
```
`cargo test -p facts-b`: 26 passed (the two `lmap` tests among them).
