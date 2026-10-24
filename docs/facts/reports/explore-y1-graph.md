# Exploration Y1: the reference graph as a maintained, compact, shared structure

Branch `Moti-Velz/facts-engine-y1-graph`, worktree `.worktrees/y1-graph`, based on `685db3f76`. Code: `spikes/facts-b/src/ygraph/` (`facts-b ygraph parity|replay`). Runs: `~/rerail-bench/facts/explore-y1/runs/`. Status: **built and measured. Neither kill condition holds.**

## Decision-table row

| Design | Memory: base / per worktree | Per-edit p50 / p95 / p99 | Exact | Fallbacks | Verdict |
|---|---|---|---|---|---|
| **Y1**: the linker as keyed tables + memoised keyed queries with recorded read sets; level-ordered re-run with early cutoff; layers base → cluster overlay → worktree delta | **Base graph 72 MB** (5,458 members; tables 19 MB, memos with reverse indexes 49 MB). Shared: interner +89 MB of graph strings, query ids 14 MB; inputs (per-content summaries) 165 MB and their strings 123 MB. **8 cluster overlays 193 MB** (22–26 MB each). **40 worktree deltas 32 MB at load** (0.02–3.7 MB each), 50 MB at edit 1,000, 201 MB after the last 111 events (6 new worktrees, 50 commits). Without clusters: 1,443 MB of deltas (≈ 88 KB per divergent path). Heap: 869 MB loaded, 956 MB at edit 1,000 | **0.73 / 2.40 / 3.94 ms** wall over 1,050 single-file edits. Counts: queries re-run 0 / 7 / 17; rows touched 276 / 2,163 / 4,443 (table rows + re-runs + new queries); probes 1 / 14 / 28. Commit and bulk events (61): 45 / 205 ms, max 1.5 s | **yes**: base = native 145,431 edges (0 missing, 0 extra); maintained = from-scratch on all 1,151 checks of the full replay (40 loads + 1,111 events), and the per-step edge delta equals a recomputation on all 1,111 | `__init__.py` add/remove (scoped re-block of the subtree): **1 / 1,111** events. Open-summary count flip: **0**, and it needs no recompute here (the edge table is maintained regardless; the flag gates it) | **neither kill holds**; footprint, not latency, is now the open number (≈ 470 MB of graph state for 40 worktrees + base, before the shared inputs) |

## 1. Structure

- **Per path (X5 classes P, C)** — `path.rs`: module path from the in-scope package markers, `\x01<r>` substitution, `s_*` rows, `last_class_def`, `s_super`, `path_alt`, spelling. A path's block is a function of (path, content, module context) and is recomputed from the content summary when needed (not stored); the path record keeps defs and edge rows only.
- **Keyed tables (class G base)** — `engine.rs`: `by_name`, `aliases`, aliases by target, `opaque`, `modules`, `supers` (by class, by target, by `<base>` short), class defs by short name, `marker_class`, defs by name, edge sources. Rows are `[u32; 4]` + count. Aliases-by-target is maintained as a *filtered* index (only keys a `supers` target resolves through, the rows `base_targets` reads): unfiltered it was 85% of a worktree's table rows (hub targets copied per worktree).
- **Queries (G derived, R recursive, the per-name link core)** — `link.rs`: `base_class`, `bases`, `base_targets` (reverse of base_class), `class_root`, `subclass`, `reference_order`, `fq_class`, `type_key`, `class_of` (the walk), `method_first`, `field_class`, `compose_first`, `link_body` (`link_form` … `link_kid` for one name) and `link_result(name, hops)`. Each query evaluates its SQL stage(s) for one key; recursive stages are evaluated whole per key (BFS / walk), so no query depends on itself. Refusals are queries registered per `supers` class, per multi-name `by_name` key, per type alias key and per edge name.
- **Ids**: one interner (`Sym`); `DefId` = (path, def name, occurrence of the name in the path) — not the def's idx, which every inserted def shifts (switching cut single-edit re-runs from mean 11 to 1.6); `ContentId` = content hash `Sym`; query ids interned (`Q { kind, a, b }`).
- **Memo**: one `Box<[u32]>`: header, probed keys (64-bit hashes, so absent keys are recorded without interning), read queries, value words.
- **Layers**: `Base` frozen; a cluster overlay (plurality membership of Spike B's 8 commit-date ranges) is a frozen delta over the base; a worktree is a `Delta` over [cluster, base]. Lookups go delta → overlay → base; overrides include tombstones. New worktrees (clones of the script base) are deltas over the base.
- **Edges and API** — `view.rs`: `Graph` (base, CSR by (module, def name) and by target def) and `GraphView` (base + overlay + delta, materialised in ≤ 23 ms per worktree), both behind `GraphRead { edges_from(ModuleId, Sym), edges_to(DefId), definitions(ModuleId), module_of(&str), def(DefId), defs_by_name(Sym), epoch(), incomplete(), refused(), sym, str }`. Deviations from the agreed contract, agreed with Y2Local: one `EdgeKind` (`Ref`), target class-ness is `def(dst).kind`; `sym` is lookup-only. `apply()` returns the step's `(src, dst)` pairs added/removed and the paths whose defs were re-read (what Y2's local rules consume).

## 2. Maintenance (X5's rule, with recorded read sets)

Per event (`world::apply`): old block out, new block in for the changed paths (and, on an in-scope `__init__.py` add/remove, every member under that directory); the filtered alias index updated from the changed alias/supers keys; changed keys = touched keys whose rows differ; their readers re-run in static level order (every query reads only lower levels; `link_result(n, h)` reads `h' > h`), a re-run that reproduces the entry below is dropped from the delta and one whose value is unchanged stops propagation; roots (edge names) and refusal queries registered for the changed keys; edges recomputed for paths whose block or whose edge names' `link_result(·, 0)` changed.

Exactness argument: X5's (a deterministic evaluation whose reads are unchanged returns the same value; the first differing read is a probe of a changed key) — here the read sets are what the queries recorded, absent keys included, not a per-stage derivation. Counterexample found: none. X5's (c) (edges whose target path is the edited one) is covered by def identity: an edge holds a `DefId`, spans are read through the view.

## 3. Exactness evidence

| Check | Result | Run |
|---|---|---|
| Base vs native, corpus 2b1504adc (9,010 members, 3,110 in scope) on (path, start, target path, start, end, class) | 145,431 = 145,431, 0 missing, 0 extra; build 1.39 s | `parity-6` |
| 300-event prefix: from-scratch build of the worktree in a forked child after every event, plus the 40 loads | 340 / 340 equal (edges as `linked_edges` rows, refusal and incomplete counters); 300 edge-delta checks equal | `prefix-300-plur` |
| Full replay (1,111 events) | 1,151 / 1,151 equal; 1,111 / 1,111 edge-delta checks | `full-plur`; also `full-flat`, `full-clusters` (majority overlays) |
| Anomalies where the SQL and native could differ | 0 ranges with lo < 0 ≤ hi; 0 non-empty links at ≥ 19 hops; 0 classes with unequal base-ordinal lengths | all runs |

Two semantic choices differ from `reference_edges.sql`, both checked by the anomaly counters above: a name is linked where a caller demands it at the caller's hops (native's recursion) instead of over `link_reach` with its `d < 19` cut, and hops are sets rather than `[lo, hi]` tuples filtered by `lo = 0`. The SQL's membership restrictions that are keyed (`class_root`, `base_class` only for `supers` targets) are replicated. Not run: the SQLite linker on the replay's modified states (the raw cgn export exists only for the corpus), so "maintained = from-scratch" is against this builder, which equals native on the corpus.

## 4. Numbers

- **Footprint** (accounted bytes, capacities; heap measured alongside). Base layer 72 MB: 406,920 memos, 375,334 table rows, 61,908 edges. The materialised base `Graph` adds 8.7 MB. Cluster overlays: divergence from the base 106–818 paths, Σ 3,689; 193 MB (≈ 52 KB per divergent path). Worktree deltas over their cluster: Σ divergence 1,110 paths, 32 MB (≈ 29 KB per divergent path). Flat (no overlays): Σ divergence 16,415, 1,443 MB. Growth over 1,000 edits: +18 MB of deltas; the last 111 events (+150 MB) are new worktrees over the base and commits.
- **Latency**: above; counts are load-independent. Flat is slower (1.21 / 3.77 / 5.83 ms) only because delta maps are larger.
- **Fallbacks**: 1 `__init__.py` re-block of 1,111 events; 0 open-summary flips.

## 5. Kill conditions

- Dependency rule not exact: not observed (0 differences on 1,151 checks; no counterexample).
- Base footprint > 1 GB with ids: no (72 MB of graph state; ≈ 300 MB with the shared interner, query ids and the input summaries' strings).

## 6. What is open

- The 250 MB target is not met for 40 worktrees: base + overlays + deltas ≈ 300 MB of graph state at load, plus ≈ 100 MB shared strings/ids, before the per-content inputs. Frozen layers are still hash maps of boxed slices; a CSR arena for frozen tables, memos and reverse indexes (started, not landed) and narrower per-table rows are the next constant factors [INFERENCE: 1.5–2× on base and overlays].
- Demand is only grown (memos of names no longer referenced stay), as X5 states; no GC.
- I8: the query bodies are hand-written per stage; the read sets are recorded generically.

## 7. Commits and runs

- `7bce31858` y1: the graph, base parity, exactness harness, API.
- `2aecfee81` y1: occurrence def ids, layered overlays (plurality), filtered alias index, one-allocation memos, per-step edge delta.
- This report.
- Commands: `facts-b ygraph parity --out DIR`; `facts-b ygraph replay [--limit N] [--flat] [--check-every N] [--workers N] --out DIR` (`results.json`, `checks.jsonl`). `cargo test -p facts-b`: 15/15.
