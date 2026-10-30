# Lattice E3: maintenance per edit in the regular shape (H7)

Branch `Moti-Velz/facts-engine-lattice-maint` from integration `949428223`. Code:
- `spikes/facts-b/src/lattice/maint.rs`: the maintained layered CSR, `min_level` and `set` maintenance, unit tests. It depends only on `round`.
- `spikes/facts-b/src/lattice/e3.rs`: the replay driver and its checks.
- `lattice/cols.rs`: `CowCol`, value columns a worktree maps privately.
- Y2/Y3 instrumentation, behaviour unchanged (§ 7): `y2/dc.rs` `Scope`/`CAPTURE`, `scope_ms`/`findings_ms`; `y3/wire.rs` `StepOut.scope`.
- Drivers: `spikes/lattice-e3/` (`e3.sh`, `e3_summary.py`).

Runs are under `~/rerail-bench/facts/explore/lattice-e3/runs/` (below, `runs/`). `BINARIES.txt` maps each binary to its commit; every final run used `facts-b-e3c` (sha256 prefix `94d19866837696b7`, commit `005e6f036`). The machine (Apple silicon, 18 cores) was shared with other jobs. The 1-minute load average is given with every wall-clock figure. Counts are deterministic: they are identical across the checked and perf runs (`summary.txt` diffed). Every millisecond figure comes from one run.

## Verdict

| Hypothesis | Verdict | Deciding numbers | Runs |
|---|---|---|---|
| **H7**: maintenance stays in the regular shape and within Y2's latency. Kill: summed p95 (CSR rows + values) > 1.5 ms, or any difference. | **Holds** for latency and exactness on dead code (`min_level`). For `set` it **holds with B/F** and is **killed with DRed**. **"In the regular shape" holds for the values only.** Maintaining the edge relation is keyed work (`succ` at the dirty ids, plus a hash probe per target) and is ~99.6% of the summed time. | Single-file edits, n = 1,050, p50 / p95 / p99 / max: **CSR rows + `min_level` = 0.042 / 0.804 / 1.788 / 20.5 ms** (replicate 0.039 / 0.754 / 1.677 / 18.1). Y2's dead code on the same events in the same process: 0.082 / 2.006 / 4.637 / 228 ms (replicate 0.078 / 1.974 / 4.059 / 236). Of the summed p95, values take 0.003 ms. `set` by B/F: p95 0.025 ms; by DRed: p95 9.3 ms (385K ids over-deleted at p95). Exactness: **1,151 checks (40 loads + 1,111 events), 0 differences**, twice (two bases). | `20261030-093829-perf-w1-a` (load 6.9), `-093922-perf-w1-b` (load 8.7–9.1), `-094140-checked-w1`, `-094340-checked-majority` |

On the 300-event prefix (the 298 single-file edits Y2 reported), the summed time is 0.043 / 0.878 / 1.606 / 20.5 ms (replicate 0.038 / 0.779 / 1.527 / 16.1). Y2's report gives 0.07 / 1.5 / 3.1 / 8.8 for these edits, measured with dataflow inputs at load 4–6. The same edits on Y2 in this process cost 0.081 / 1.898 / 5.558 / 27.5 (replicate 0.071 / 1.678 / 3.486 / 26.4).

The max exceeds Y2's reported 8.8 ms on two edits: 20.5 and 16.9 ms. Each is one changed method whose dispatch layer holds 63,391 states, every one re-evaluated (§ 5).

## 1. What was built

### Input deltas

Y3's wired path runs unchanged (`y3::world::World`: Y1 → edge delta → symbol table → Y2). The CSR's rows are dead code's product graph (`y2::dc::succ`), as in E1. A row depends on Y1's edges and also on symbol-table rows and Y2's derived keys (`class_target`, `unique_class`, …). So no single input delta names the rows that change.

The exact reader set is the one Y2's `prepare` already computes. That set is exact by Y2's read-set proof (Y2 report § 4). With `dc::CAPTURE` set, `prepare` also returns its scope:
- every dirty state, reached or not;
- the dispatch layers `d + 2` of the changed definitions;
- the root candidates.

`e3` turns the scope into ids:
- every base or local id of a dirty node;
- every id of a dirty layer;
- `succ` re-evaluated at them under the new facts. Ids that are new to the worktree are allocated and their rows evaluated in the same pass.

**Deviation from the brief:** the rows are not patched from "edge deltas from Y1 / the symbol table / Y2's edge index". They are re-evaluated at Y2's reader set, because the product graph's edges are a join over all three.

### The edge relation: E1's layered CSR, maintained in place

- **Base.** Worktree 1's product graph (E1's choice, `--base w1`): 439,592 ids (dispatch 327,506), 1,284,830 edges. It is written as columns and mapped read-only (L6): forward CSR, transpose, `St` per id, a (layer → ids) index, base values. Total 20.5 MB.
- **Alternative base.** `--base majority` uses Y3's strict-majority base layer: 123,709 ids, 332,382 edges.
- **The worktree's forward layer (`MutLayer`).** E1's read path is unchanged: one bit test, then the base row or a replacement row.
  - E1's dense rank per word became a per-word block offset (`wbase[w] + popcount`). A word's slots are contiguous, with slack up to a power of two. A bit that flips shifts at most 63 slots, or moves its word's block to the end of the slot table.
  - E1's dense rank had to be rebuilt on every flip. In the first smoke run that cost a 6,886-word pass and an O(replaced rows) slot copy per edit (`20261029-155501-smoke20`, rows p95 1.32 ms).
  - Rows live in an arena. They are overwritten in place when they do not grow, and the arena is compacted when half dead (0 compactions on the replay).
- **The worktree's transposed layer.** The same structure. It is maintained from the forward edge delta, and both deletion algorithms read it.
- **Local ids.** States the base lacks get ids past `nb`, kept in sorted vectors. The ids added during a step are merged once per step.

### Values (L9)

- **`min_level`** (`level_update`), with exact BFS levels after every edit:
  1. **Invalidation, level by level.** A checked id at level ℓ keeps its value while a predecessor at ℓ − 1 that is not invalidated remains: a scan of its transposed row until the first witness; at level 0, while it is a seed. An invalidated id queues its successors at ℓ + 1.
  2. **Re-derivation.** Each invalidated id takes 1 + the least level among its predecessors, in one pull over the transposed rows.
  3. **Propagation.** Decreases (re-derived ids, inserted edges, new seeds) propagate in rounds of uniform level, a bucket per level, which are E1's rounds.
- **`set` by DRed** (`set_update`). Over-delete forward from deleted edges and seeds over reached ids, re-derive from the transposed rows, then forward rounds. Support counts were not used: the graph has cycles, where counts never fall to zero.
- **`set` by B/F** (`set_update_bf`; Motik et al.'s backward/forward, specialised to reachability).
  - Each candidate is a deleted edge's target, a deleted seed, or a successor of a dead id. It is checked by a backward search over the new transposed rows, in frontier rounds, which stops at the first seed or at an id already proven this edit.
  - A search that finds neither has visited the candidate's whole backward cone, all of it unreachable. The cone dies at once, and its successors become candidates. Then forward rounds run from the inserted edges and new seeds.
  - No witness is stored: one bit per id.

Every phase is a frontier list or bitset scanned against CSR rows. Scratch is per worker and cleared sparsely. Inside the value algorithms there is no hash map, no allocation per row, and no per-key memo.

### Value columns per worktree

The base's value columns (levels, set bits, seed bits) are mapped privately per worktree (`CowCol`, `MAP_PRIVATE`). The kernel copies a 16 KB page on the worktree's first write to it. Private bytes are measured from `mincore` (resident and anonymous pages), not estimated. The copy counter of shared columns (`cols::COPIED`) reads 0 in every run.

### Exactness checks

Each check runs in a forked child, after every load and every event (`--check-every 1 --check-load`):
- every row of the worktree's id space equals `succ`;
- every target and root has an id;
- no replaced row equals the base's;
- the transposed layer equals the transpose;
- the seed bits equal Y2's roots;
- `min_level`, `set` (DRed) and `set` (B/F) equal E1's sequential fixpoint on the maintained CSR;
- the reached states equal Y2's maintained reached set, state for state;
- every level equals Y2's from-scratch level.

## 2. Exactness

| Check | Result | Run |
|---|---|---|
| Unit tests (`lattice::maint::tests`, 6). Covered: deleting a cycle's entry unreaches the cycle; a cycle does not support itself; a second witness keeps a level and re-parents it deeper; removing and adding seeds; a 3,000-chain cut in the middle and restored; 40 random graphs × 30 edits (n ≤ 201, cycles, self-loops, new ids, seed changes). Asserted after every edit: both lattices and B/F = the naive fixpoint; every layer row = the flat graph; scratch bitsets cleared; no replaced row equal to the base's | pass | T0 `~/rerail-bench/facts/loop/runs/20261030T094903-t0` (`facts-b-unit` 39 tests); T1 `…/20261030T095150-t1` at `005e6f036` (5/5 checks pass, load 23.6) |
| Mutants: 75 (`min_level`'s re-check counts an invalidated predecessor as a witness), 77 (DRed's over-delete takes no forward round), 79 (B/F accepts an id reached before the edit as support) | all killed by `facts-b-unit` | same T0 |
| Full replay, base w1: 40 loads + 1,111 events, every check above | **1,151 checks, 0 differences** | `runs/20261030-094140-checked-w1` (load 6.7 → 12.6) |
| Same, base majority | **1,151 checks, 0 differences** | `runs/20261030-094340-checked-majority` |
| Earlier binaries (before the time breakdown; the first before B/F) | 1,151 / 0 and 1,151 / 0 | `runs/20261030-093232-full-checked-w1`, `-092217-full-checked-w1` |
| Y3 unchanged by the instrumentation: `y3 replay --limit 300 --check-every 1 --check-load` with the E3 binary | 340 checks, 0 unequal | `runs/20261030-094639-y3-unchanged-300` |

**Y2's levels are not `min_level`.** Y2 keeps a level as an upper witness: it does not lower levels after insertions. At a median check, 13,284 of Y2's maintained levels differ from the BFS depth (max 51,957; checks file, `y2_levels_drifted`). The lattice keeps exact values. So the check compares the lattice's levels with Y2's levels from scratch, and its reached states with Y2's maintained reached set.

## 3. Per-edit latency (H7)

Single-file edits (n = 1,050), ms, p50 / p95 / p99 / max. Run a: `runs/20261030-093829-perf-w1-a` (load 6.9). Run b: `-093922-perf-w1-b` (load 8.7–9.1).

| | run a | run b |
|---|---|---|
| **CSR rows** (Y2's scoping + rows) | 0.042 / 0.803 / 1.788 / 20.5 | 0.038 / 0.753 / 1.676 / 18.1 |
| — Y2's scoping (`prepare` up to the dirty set) | 0.012 / 0.184 / 0.331 / 0.97 | 0.011 / 0.177 / 0.341 / 1.00 |
| — `succ` at the recomputed ids | 0.008 / 0.154 / 0.482 / 11.7 | 0.008 / 0.142 / 0.405 / 10.3 |
| — targets → ids (hash probe per target) | 0.005 / 0.384 / 0.878 / 6.50 | 0.005 / 0.340 / 0.778 / 5.39 |
| — layer writes, both directions, seeds | 0.001 / 0.018 / 0.030 / 0.072 | 0.001 / 0.017 / 0.027 / 0.069 |
| **values, `min_level`** | 0.000 / 0.003 / 0.012 / 0.510 | 0.000 / 0.003 / 0.012 / 0.483 |
| **sum (H7)** | **0.042 / 0.804 / 1.788 / 20.5** | **0.039 / 0.754 / 1.677 / 18.1** |
| `set` by B/F | 0.000 / 0.025 / 0.066 / 0.122 | 0.000 / 0.019 / 0.045 / 0.167 |
| `set` by DRed | 0.000 / 9.275 / 10.32 / 11.9 | 0.000 / 8.435 / 9.358 / 13.1 |
| Y2 dead code, same events, same process | 0.082 / 2.006 / 4.637 / 228 | 0.078 / 1.974 / 4.059 / 236 |
| Y2 without its findings | 0.079 / 1.998 / 4.607 / 228 | 0.076 / 1.970 / 4.042 / 236 |

- **Edits above 1.5 ms** (run a): 13 of 1,050 for the lattice, 75 for Y2.
- **Bulk and commits** (run a; p50 / p95 / max):

  | | lattice | Y2 |
  |---|---|---|
  | 5 bulk events | 68.9 / 136.8 / 136.8 | 224.9 / 397.0 / 397.0 |
  | 51 commits | 35.7 / 249.0 / 600.3 | 38.6 / 525.8 / 941.3 |

- **Majority base** (`runs/20261030-094010-perf-majority`, load 7.9): sum 0.056 / 1.381 / 3.234 / 20.5. Its layer writes are slower (p95 0.461 ms: blocks of 300K+ slots) and so are its id lookups (p95 0.757 ms: 295K local ids per worktree).

**By rows recomputed** (run a). Latency in ms, p50 / p95:

| recomputed ids | edits | lattice sum | Y2 |
|---|---|---|---|
| 0–10 | 312 | 0.008 / 0.042 | 0.009 / 0.114 |
| 11–100 | 505 | 0.045 / 0.202 | 0.089 / 0.797 |
| 101–1,000 | 188 | 0.208 / 0.881 | 0.384 / 2.848 |
| > 1,000 | 45 | 1.109 / 4.612 | 1.878 / 4.637 |

**Loading a worktree** (the base plus one edit: every base id's row re-evaluated). Median 165 ms, of which rows 153 ms and `min_level` 1.3 ms. Y3's own load step: median 51 ms (run a). A worktree created during the replay costs 144–156 ms.

## 4. Counts per single-file edit (L8)

p50 / p95 / p99 / max. Identical in every w1 run.

**CSR rows**

| | Count |
|---|---|
| dirty states (Y2's scope) | 24 / 768 / 3,080 / 29,975 |
| ids recomputed | 24 / 808 / 3,386 / 63,733 |
| — of which dispatch | 0 / 625 / 3,210 / 63,391 |
| `succ` targets | 93 / 33,145 / 102,841 / 196,885 |
| rows changed | 1 / 12 / 41 / 312 (dispatch 0 / 3 / 15 / 128) |
| ids added | 0 / 2 / 14 / 64 |
| edges deleted | 1 / 18 / 46 / 403 |
| edges inserted | 0 / 10 / 42 / 339 |
| seeds changed | 0 / 1 / 2 / 22 |
| transposed rows changed | 1 / 14 / 35 / 169 |
| layer bits flipped | 1 / 8 / 28 / 214 |
| slots shifted | 0 / 15 / 48 / 1,195 |
| blocks moved | 1 / 6 / 16 / 64 |

**`min_level`**

| | Count |
|---|---|
| checked | 0 / 10 / 52 / 16,336 |
| invalidated | 0 / 9 / 48 / 16,268 |
| invalidation rounds | 0 / 3 / 6 / 11 |
| transposed edges to the first witness | 0 / 13 / 77 / 33,861 |
| re-derive edges | 0 / 7 / 54 / 32,933 |
| propagation rounds | 0 / 3 / 7 / 9 |
| propagation edges | 0 / 16 / 156 / 37,810 |
| values changed | 0 / 10 / 50 / 16,268 |
| reachability toggled | 0 / 6 / 34 / 128 |

**`set` by B/F**

| | Count |
|---|---|
| candidates | 0 / 17 / 46 / 403 |
| searches | 0 / 11 / 31 / 133 |
| — found a seed | 0 / 7 / 20 / 69 |
| search rounds | 0 / 18 / 53 / 264 |
| ids visited | 0 / 2,873 / 8,677 / 23,644 |
| transposed edges | 0 / 2,895 / 9,066 / 24,360 |
| ids dead | 0 / 4 / 18 / 68 |

**`set` by DRed**

| | Count |
|---|---|
| over-deleted | 0 / 385,147 / 388,034 / 388,140 |
| over-delete edges | 0 / 1,127,198 / … / 1,136,538 |
| re-derived | 0 / 9,131 / 9,151 / 9,168 |

This is the module hub that made Y2's DRed slow (Y2 report § 1.1). Restricting DRed to reached ids does not change it.

**Y2, same events**

| | Count |
|---|---|
| dirty reached | 20 / 692 / 3,071 / 29,975 |
| `succ` calls | 46 / 1,538 / 7,387 / 362,130 |
| unsupported | 0 / 8 / 48 / 16,268 |
| toggled | 0 / 6 / 34 / 128 |

These equal Y3's report § 3(d), and toggled equals the lattice's toggled.

## 5. The dispatch fan-out (L10)

- **Rows.** 84% of the replaced forward rows are dispatch states: 1,177,806 of 1,403,193 summed over 40 worktrees at load (`perf-w1-a`). Over the majority base: 10,980,090 of 12,599,649 (87%), plus 11.75M local ids (`perf-majority`). Dispatch is also 74.5% of the base's ids: 327,506 of 439,592.
- **Latency.** It is the tail, not only memory.
  - At p95, 625 of the 808 recomputed ids are dispatch states.
  - The two slowest edits (20.5 and 16.9 ms; Y2: 2.7 and 4.6 ms) each recomputed 63,391 dispatch states and changed 40 rows. A changed method `d` dirties every `flow(n, d)` state of the closure.
  - The CSR holds the closure, a roots-independent superset of the reached set, so that the edge relation is a non-recursive plan. Y2 expands the layer over its reached nodes only.
  - Keying dispatch by class removes the layer [INFERENCE: not built].
- **Per worktree.** Median replaced rows at load: 44,661 (`perf-w1-a`), of which dispatch about 39K.
  - E1 reported a median of 12,638 on Y2's dump (first touch per worktree). Here the input is Y3's 40 memberships, and 31 of them differ from worktree 1 in more than 200 paths.
  - The two figures measure different inputs, not the same quantity twice.

## 6. Memory

The lattice state of 40 worktrees at load (`perf-w1-a`), MB. Shared base columns: 20.5, mapped once. Private values are page-granular, from `mincore`.

| | at load (40) | end (46) |
|---|---|---|
| forward layers (tight: without slack) | 99.0 (60.3) | 189.8 (96.2) |
| transposed layers (tight) | 103.4 (59.4) | 208.0 (91.7) |
| local ids | 6.5 | 31.0 |
| `min_level` values, private pages | 64.0 | 80.0 |
| — the same as a sparse delta (8 B per differing id) | 9.6 | 24.8 |
| `set` bits (B/F), private pages | 2.4 | 3.1 |
| seed bits | 2.2 | 2.8 |
| **total** (`set` once) | **277.5** | **514.7** |

The end column includes six worktrees created during the replay as clones of Y3's majority base (§ 7).

**Against Y2's 24 MB** (reached sets, base + delta, 41 worktrees; Y2 report § 5):
- **Values.** The lattice's values cost 68.6 MB as copy-on-write pages, or 9.6 MB of levels as a sparse delta.
  - Copy-on-write at 16 KB pages degenerates to a near-full copy per worktree: 3,849 of the 40 × 108 level pages differ.
  - Exact levels differ from the base's in 37,667 ids (median worktree), scattered over the column. Set bits differ in 33,790 (median), in half of the column's words. The set column is 4 pages, all of them copied.
  - Y2 inside Y3 holds 15.5 MB in its worktree tops at load (measured here). Y3 reports 3.0 MB base and 92.3 MB of overlays besides (Y3 report § 3(a)).
- **Edge relation.** The edge layers (202 MB, tight 120 MB) have no counterpart in Y2. Y2 holds no edge relation: it re-evaluates `succ` against its inputs (306 MB in Y3).
- **Not counted:** the base's `St → id` hash map, heap. [INFERENCE: about 9 MB at 439,592 entries.]

**By divergence from worktree 1** (paths; medians per bin, at load):

| paths | worktrees | replaced rows (dispatch) | forward KB | transposed KB | values KB | levels differing |
|---|---|---|---|---|---|---|
| 0–5 | 3 | 3 (0) | 89 | 113 | 66 | 0 |
| 51–200 | 6 | 4,120 (2,605) | 977 | 423 | 1,425 | 2,791 |
| > 200 | 31 | 46,671 (39,005) | 2,710 | 3,586 | 1,933 | 40,149 |

- **Floor per worktree.** A worktree with no divergence costs about 270 KB. That is the bit and block-offset arrays (13 B per 64 ids, both directions) plus touched value pages.
- **Base choice.**
  - A worktree created during the replay as a clone of Y3's majority base diverges from worktree 1 in 826 paths. It costs 308K replaced rows (21.7 MB).
  - Worktree 50 (`commit_init`, 8,361 paths) costs 833K rows and 662K local ids (111.5 MB).
  - Over the majority base, every loaded worktree holds about 312K replaced rows and 295K local ids (41 MB each).
  - A single base therefore serves only worktrees close to it: per-worktree state is proportional to divergence from *the base*, and the CSR needs Y1's cluster overlays too.

## 7. Contradictions with LATTICE.md, and proposed amendments

1. **H7 verdict.** *Proposed:* "**holds** for latency and exactness:
   - dead code `min_level`: summed p95 0.80 / 0.75 ms against Y2's 2.0 ms on the same events; 1,151 checks, 0 differences;
   - `set` holds by B/F (p95 0.025 ms) and is killed by DRed (p95 9.3 ms);
   - 'regular shape' holds for values only: the edge relation's maintenance is keyed and is the whole cost."
2. **L9, `set`.** "Support counts or over-delete-and-rederive restricted to the affected ids" is wrong on this graph.
   - Support counts never fall on cycles.
   - Restricted DRed still over-deletes 385K ids at p95.
   - *Amendment:* `set` deletion is backward/forward. A backward search per candidate stops at the first seed or proven id; a failed search kills its whole cone. DRed is allowed only where the closure is declared small.
   - `min_level`'s levelled re-check is confirmed as written, with exact levels.
3. **L9/L10, the transpose.** Both deletion algorithms read predecessors, so the edge fact needs its transposed CSR maintained per worktree as well: 103 MB at load, the same size as the forward layers. *Amendment:* L5/L10 list the transposed layer as part of every lattice fact with deletions.
4. **L5, "the edge relation is a maintained fact" in the regular shape.** The regular shape covers the rounds and the value maintenance: values cost 0.003 ms at p95. The rows do not fit it:
   - re-evaluating `succ` at the dirty ids: keyed lookups, Y2's code;
   - mapping each target `St` to its id by a hash probe: the largest single part, p95 0.38 ms;
   - Y2's keyed scoping.

   *Amendment:* L5 should say the edge relation is maintained by its membership plan (I8, Y3), outside the round operator. Its rows should arrive keyed by dense u32 ids issued where the facts are keyed. [INFERENCE: that removes the mapping step, about half of the rows' p95.]
5. **L6/L10, values do not move.** Mapping the base's value columns copy-on-write keeps the base shared and the copy counter at 0. But each worktree's first writes copy almost every page: level pages 89%, set pages 100%.
   - Exact `min_level` values shift wherever a worktree's graph differs, at about 35K ids per worktree, scattered.
   - This is in effect the "full copy of the values per worktree" that L10 forbids.
   - *Amendment:* layer the values as a sorted sparse delta (8 B per differing id: 9.6 MB against 64 MB here), or accept dense per-worktree values for fixpoints of this size (1.9 MB each) and say so.
6. **L10, base and layering depth.** Two levels (base and worktree) are not enough:
   - the majority base costs 9× the replaced rows of a worktree-1 base, and 59× the local ids;
   - a clone of another base costs 308K rows.

   *Amendment:* L10's "base, cluster overlay, worktree delta" applies to the CSR too. The base must be a layer that reaches what its worktrees reach.
7. **L10, dispatch.** "Dispatch keyed by class is the rule-side fix" (stated for memory) also governs the latency tail: 63K-row edits, the 20.5 ms max. Keeping the closure as the edge relation makes a changed method dirty its whole layer. *Amendment:* add the tail to L10's text.
8. **H4 caveat in E1** ("the 45× only counts if the CSR is maintained"): settled.
   - The CSR is maintained at 0.80 ms p95 per edit, below Y2's whole update.
   - But building a worktree's layer from the base costs a full `succ` pass: 165 ms per load against Y3's 51 ms.

## 8. What was not done

- **Not built:** the GPU, tree closures, the compiler surface, `best_witness`.
- **Maintenance across worktrees.** Each event touches one worktree and runs on one thread; no parallel maintenance was measured.
- **Cluster overlays for the CSR or the values.** Only two levels were built (base + worktree), with two bases compared.
- **Layers as mapped files.** The layers are heap vectors. Values are private mappings of the base's files.
- **Not attempted:**
  - dispatch keyed by class;
  - edge rows keyed by dense ids at the source;
  - a sparse value delta;
  - dead-code findings from the lattice's values.

  The findings stay Y2's: Y2's `finish` still runs, and the lattice path does not compute findings.
- **Instrumentation in Y2/Y3.** Behaviour is unchanged: `CAPTURE` is off by default, the timers only read the clock, and `is_root` became `pub(crate)`. Evidence: the 300-event Y3 check above, and Y2's counts equal Y3's report. I did not run the full Y3 check (1,151) again.
- **Count baseline.** The work counts of the gate subset are untouched: no facts-a or generator change, so no count-change row. T2 was not run, per the brief.
- **Machine.** The machine was shared, at load 6.7–12.6. Wall times are single runs plus one replicate. No core pinning.

## 9. Reproduce

```
cargo build --release -p facts-b         # copy target/release/facts-b to BIN
cd ~/rerail-bench/facts/explore/lattice-e3
BIN/facts-b y3 spec spec-replay-m2-three.json       # the M2 spec naming this checkout's packs
E=<worktree>/spikes/lattice-e3/e3.sh
QUIET=10 BIN=… $E checked runs/<ts>-checked-w1 w1    # or majority
QUIET=8  BIN=… $E perf    runs/<ts>-perf-w1-a w1
python3 <worktree>/spikes/lattice-e3/e3_summary.py RUN [--json OUT]
cargo test -p facts-b --bin facts-b lattice::maint   # unit tests; FACTS_MUTANT=75|77|79 with --features facts-b/mutants
```

`facts-b lattice maint --out DIR [--base w1|majority] [--limit N] [--check-every N] [--check-load] [--workers W] [--spec F]`
