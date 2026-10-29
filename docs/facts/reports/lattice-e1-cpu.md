# Lattice E1: the semiring-round fixpoint on the CPU (H4, H5)

Branch `Moti-Velz/facts-engine-lattice-cpu` from integration `c6c33c491`. Code: `spikes/facts-b/src/lattice/` (`round`, `cols`, `build`, `bench`), Y2's `dc::scratch` split into reachability and findings with identical results (`spikes/facts-b/src/y2/dc.rs`, `y2/mod.rs`), drivers `spikes/lattice-e1/` (`e1.sh`, `e1_summary.py`, `e1_units.py`). Runs: `~/rerail-bench/facts/explore/lattice-e1/runs/` (below, `runs/`). Binaries are copied next to the runs (`BINARIES.txt`). Machine: Apple silicon, 18 cores (6 P + 12 E), shared with other jobs. Every sweep waited for the 1-minute load average to fall below a threshold (`QUIET`), and every run records its load. Counts are deterministic. Each wall-clock figure is the median of 15 timed runs (3 processes × 5 runs).

## Verdicts

| Hypothesis | Verdict | Deciding numbers | Runs |
|---|---|---|---|
| **H4**: one core much faster than the current evaluator on the same fixpoint (kill: < 2×) | **Holds, for the fixpoint alone.** | Oracle tree, same process, interleaved: Y2 reachability 321.2 ms against the sequential fixpoint's 7.19 ms (**44.7×**). Against Y2's traversal alone (its reachability minus `succ`, its edge evaluation) it is **16.5×** (118.8 ms). Fleet, 41 worktrees: 5,274 ms against 115 ms (**45.8×**; 42.9–54.6× per worktree). **Caveat:** the fixpoint reads a CSR that must already exist. Building that CSR cold from Y2's tables costs 346.7 ms, **1.08× Y2's whole reachability**. | `20261029-142010-build-oracle`, `-141943-build-oracle` (replicate: 44.9×, 17.1×), `20261029-142036-build-fleet` |
| **H5**: the regular shape scales across cores (kill: < 4× at 6 threads, or memory growing with threads beyond output slices) | **Killed within a round. Holds across worktrees.** | Within a round, oracle tree, best kernel at 6 threads: **2.12×** (min_level, `par`). The best configuration at 6 threads, against the st-order sequential kernel of the same lattice, is 3.14× (set, `par`; 2.37× in the replicate). The fleet within a round tops out at **1.79–1.89×** (4 threads). Across worktrees (each worker runs whole fixpoints): **5.13–5.29× at 6 threads, 8.7–9.1× at 12, 9.4–11.2× at 18**. Memory: `par` peak private footprint 5.9 → 6.6 MiB from 1 to 18 threads (thread stacks; 0 B heap per worker). Across worktrees it grows by one output slice per worker (1.97 MB). | `20261029-144742-sweep-oracle`, `-144755-sweep-fleet`, `-144749-sweep-oracle-hash`; replicates `-142218-sweep-oracle`, `-142224-sweep-fleet` |

Exactness was checked in every run. Every reached state and every level equals Y2's:
- on the oracle tree: every kernel, both lattices, both id orders, at 1, 2, 4, 6, 12 and 18 threads;
- on all 41 replay worktrees: every fleet mode (`within` with `seq`/`par`/`edge`, `across`) at the same thread counts.

The value digests are identical across thread counts. The unit tests compare every kernel with a naive reference fixpoint on graphs with cycles. Copy counter: 0 in every run.

## 1. What was built

**Where.** A module of facts-b, `src/lattice/`, not a new crate, for two reasons:
- Its tests then run in T0 unchanged (`facts-b-unit`).
- The CSR build needs Y2's `Facts` and `succ`, which live in facts-b.

The operator (`lattice/round.rs`) imports nothing from the rest of the crate: it reads `&[u32]`/`&[u64]` only. That is the "plan audit" of L5, by construction.

**The operator (L5).**
- **Edges.** A CSR with `u32` offsets and `u32` targets. `Csr::new` checks the layout once: offsets ascending, every row strictly ascending (sorted, no repeats), every target below n. A worktree's graph is `Layered` (L10): the base CSR, plus a bit per id, a rank per 64-id word and a delta CSR of replacement rows. A row read is one bit test, then the base row or the delta row at the bit's rank.
- **Lattices.** `min_level` uses (min, +1) over `u32` with `INF` = ∞; with every seed at 0 it is exactly Y2's BFS level. `set` uses (∨, ∧) over a bitset. Both have integer carriers.
- **Frontier.** A bitset. The sequential kernel also keeps a sorted `u32` list while the frontier is under n/64 ids, so sparse rounds skip the word scan (14 of 25 rounds on the oracle tree).
- **Kernels.** One sequential (`seq`) and four parallel within a round. All use scoped threads spawned once per fixpoint and a spin-then-yield barrier:

  | Kernel | Work unit | Join | Barriers per round |
  |---|---|---|---|
  | `par` | equal ranges of frontier words (16 per worker), claimed through one counter | atomic `fetch_min` / `fetch_or` into the shared value array; the worker whose join changed the value sets the next-frontier bit | 1 |
  | `edge` | equal ranges of the round's edges (the frontier is turned into a sorted list with prefix sums of row lengths; binary search to the first row) | as `par` | 2 |
  | `own` | phase 1: frontier word ranges, buffering candidate targets per partition; phase 2: each id partition is joined by the one worker that claims it | plain stores by the partition's owner; no atomic read-modify-write on values | 2 |
  | `dir` | as `par`, or pull rounds over target ranges with the transposed CSR (Beamer's α = 14, β = 24 switch, decided from counts) | as `par`, or owner stores (pull) | 1 |

- **The inner loop.** Read a frontier word. For each set bit, read two offsets and a slice of targets. For each target, one load and, rarely, one atomic. There is no hash probe, no allocation per row, no pointer chasing, and no dynamic dispatch (`Rows` is monomorphised).
- **Determinism (L4).** Values are joins, so they are commutative and idempotent. A changed value is counted once, by the worker whose join changed it. The direction switch reads counts only. The tests check every count of every kernel equal across 1, 2, 3, 4 and 8 threads (`dir` against itself). Every run checks it across all runs.

**Columns (L6).** `lattice build` writes raw `u32`/`u64` columns. `lattice run` maps them read-only (`mmap`, `MAP_SHARED`), and every worker reads the same mapping. `cols::COPIED` counts bytes copied out of a shared column; the only path that copies adds to it. It read **0** in every run. The mapped bytes break down as follows:

| | Oracle | Fleet |
|---|---|---|
| Inputs (CSR + seeds) | 16.0 MB | base 6.9 MB + 41 layers 47.7 MB |
| Y2's reference levels (the checks) | 7.2 MB | 122 MB |
| Transpose (`dir` only) | 16.0 MB | — |

Per-worker private bytes, measured:

| Kernel | Heap per worker |
|---|---|
| `par`, `dir` | 0 B (two counters) |
| `edge` | 1,552 B at 6 threads (a copy of the 97 range offsets) |
| `own` | its candidate buffers: 0.30–1.35 MB at 6 threads, 0.08–1.25 MB at 18 |
| `across` | one fixpoint's state: 1,973,904 B (values 1.80 MB, two frontier bitsets, two sparse lists) |

**Input: Y2's product graph as a CSR** (`lattice/build.rs`).
- **States.** Y2's `(node, layer)`: flow, annotated, and dispatch `flow(node, d)`.
- **Edges.** Y2's own `succ`, evaluated once per state of the closure of every state that could be reached: the roots, every definition's flow state and its module's. That closure is a superset of the reached set whatever the roots, so the fixpoint from Y2's roots is Y2's reachability.
- **Ids.** By default, sorted `(node, layer)` order (`--order st`). `--order hash` sorts by a fixed mix of the state. Rows are sorted with repeats dropped.
- **Fleet layering.** Worktree 1 (the replay's reference) is the base. Every other worktree stores only the rows that differ from the base; new states get appended ids.
- **Oracle-tree edges** are the dump's `views/reference_edges`, which equal native and Y1's base graph (145,431, Y2 report § 2); no Y1 export exists for the oracle tree.
- **Fleet edges** are Y1's export (`y2-local/runs/20261023-223115-y1-edges-300/edges.tsv`), as `y2 --edges` reads them.
- **Determinism of the build.** The columns are byte-identical across six oracle builds and three binaries (sha256 prefix `982903288f639fc5`), and across two fleet builds (`ab38286a966d45e8`).

| | Oracle tree | Fleet (41 worktrees) |
|---|---|---|
| ids | 1,038,978 (flow 77,852, annotated 77,852, dispatch 883,274) | 451,178 (union); 123,709–439,592 per worktree |
| CSR edges | 2,951,163 (2,964,884 with repeats) | base 1,284,830; 49.1M summed over worktrees |
| roots / Y2 reached | 13,655 / 895,971 (67,286 / 67,288 / 761,397, = Y2 report) | 94,739–399,826 per worktree |
| replaced rows per worktree (L10) | — | min / median / max 0 / 12,638 / 21,725 (median 2.9% of the rows); layer columns 47.7 MB against ≈ 263 MB for 41 flat CSRs |

## 2. Exactness

| Check | Result | Run |
|---|---|---|
| Unit tests (`lattice::round::tests`, 6): random graphs (n 1–5,000, cycles, self-loops, repeats, unreached parts) and a 3,000-round cycle; every kernel at 1, 2, 3, 4, 8 threads = naive Bellman–Ford fixpoint, both lattices; counts equal across thread counts; layered rows = the flat graph they denote; dense graphs take pull rounds; unsorted rows refused | pass (T0: `facts-b-unit` 32 tests) | `~/rerail-bench/facts/loop/runs/20261029T140553-t0` and later T0 runs |
| Build: the sequential fixpoint on each worktree's flat CSR = Y2's `scratch_reach`, state for state and level for level (asserted in every interleaved run) | oracle 7 runs, fleet 41 worktrees × 5 runs: 0 differences | `20261029-142010-build-oracle`, `20261029-142036-build-fleet` |
| Run: mapped columns, every reached id and level against Y2's | oracle: `exact` in all 50 configurations (2 lattices × (`seq` + 4 kernels × 6 thread counts)), each 3 processes × 5 runs; digests `abfae7dc07425bd4` (min_level) and `8933481e1ab29aa8` (set) in every kernel and thread count | `20261029-144742-sweep-oracle` |
| Fleet: every worktree in one further checked pass per configuration | 41 worktrees × 19 configurations: 0 differences; per-worktree digests = the sequential kernel's | `20261029-144755-sweep-fleet` |
| Hash id order | exact, digest-equal across thread counts (the digest differs from st order: other ids) | `20261029-144749-sweep-oracle-hash` |

## 3. H4: one core

Y2's from-scratch reachability, its findings, `succ` alone at the states it reached, and the lattice's sequential fixpoint on the same worktree's flat CSR were timed **in the same process, interleaved**, so each run of the four shares one load. Y2's findings are timed separately: 4.1 ms on the oracle tree, 91.4 ms summed over the fleet.

| Oracle tree (7 runs, medians) | Y2 reach | of which `succ` | Y2 traversal (reach − `succ`) | fixpoint | reach / fixpoint | traversal / fixpoint | load 1m |
|---|---|---|---|---|---|---|---|
| `20261029-142010-build-oracle` | 321.2 ms | 202.5 ms | 118.8 ms | 7.19 ms | **44.7×** | **16.5×** | 6.5 |
| `20261029-141943-build-oracle` | 347.7 | 215.2 | 132.5 | 7.75 | 44.9× | 17.1× | 7.2–7.4 |
| `20261029-143314-build-oracle-hash` (hash ids) | 311.4 | 188.7 | 122.7 | 12.32 | 25.3× | 10.0× | 5.9 |
| Fleet `20261029-142036-build-fleet`, Σ of 41 per-worktree medians | 5,274 | 3,300 | 1,974 | 115.2 | **45.8×** (per worktree 42.9 / 45.2 / 54.6) | 17.1× (14.6 / 17.0 / 20.7) | 3.8–5.8 |

In its own process over the mapped columns the sequential fixpoint is 5.98 ms (`seq`, `20261029-144742-sweep-oracle`; 5.54 ms in `-142218`).

Work, as algebraic counts:

| | edges examined | rounds | per-state cost |
|---|---|---|---|
| Lattice | 2,582,229 (deduplicated rows of the reached states) | 25 | one sequential pass |
| Y2 | 2,594,236 (one `succ` per reached state: 895,971 calls) | — | a binary-heap push/pop, a layered hash-map insert, and the dispatch-index pushes per state |

The gap is the evaluator, not the work.

**What H4 does not show.**
- The fixpoint needs the edge relation as columns.
- Built cold from Y2's tables through `succ` (`enumerate` 246.6 ms over 1,038,978 states, then `assemble` 100.1 ms to sort and map rows), it costs 1.08× Y2's whole reachability on the oracle tree, and 1.02× on the fleet (4,406 + 960 ms against 5,274).
- So the 45× is real only when the CSR is a maintained fact: L1's "edges are a non-recursive plan over facts", kept current per edit.
- E3 has to cost that maintenance (CSR rows per edit), not only the values.
- [INFERENCE] A bulk CSR build (joins over the tables instead of `succ` per state) would be cheaper than `enumerate`. It was not built.
- **Id order is a single-core lever.** Hash ids (the order that balances work units, § 4) double the sequential fixpoint (11.34 against 5.98 ms, `-144749` / `-144742`) through lost locality.

## 4. H5: across cores

### Workload A, oracle tree, within a round

`20261029-144742-sweep-oracle`, load 5.4–5.7. Speedup is against the sequential kernel (5.98 ms). Replicate `-142218` (load 3.5–3.8): same shape, best `par` 2.33× at 4 and 2.09× at 6. Memory in the sweep tables is in MiB: peak `ri_lifetime_max_phys_footprint`, which counts private pages and not the mapped columns.

| min_level, ms (speedup) | 1 | 2 | 4 | 6 | 12 | 18 |
|---|---|---|---|---|---|---|
| `par` | 6.02 (0.99) | 3.61 (1.65) | **2.43 (2.45)** | 2.81 (**2.12**) | 3.11 (1.92) | 3.16 (1.89) |
| `edge` | 7.82 (0.76) | 6.95 (0.86) | 4.62 (1.29) | 5.55 (1.08) | 5.74 (1.04) | 6.52 (0.92) |
| `own` | 8.60 (0.69) | 5.04 (1.19) | 3.22 (1.86) | 2.98 (2.00) | 3.02 (1.98) | 3.94 (1.52) |
| `dir` | 16.34 (0.37) | 8.75 (0.68) | 4.66 (1.28) | 3.74 (1.60) | 2.62 (2.28) | 2.96 (2.02) |
| CPU ms, `par` | 6.0 | 7.1 | 9.4 | 15.9 | 34.4 | 44.8 |
| peak private MB, `par` | 5.9 | 6.0 | 6.0 | 6.1 | 6.4 | 6.6 |

| set, ms (speedup against `seq` 6.62) | 1 | 2 | 4 | 6 | 12 | 18 |
|---|---|---|---|---|---|---|
| `par` | 5.70 (1.16) | 3.88 (1.71) | 2.57 (2.57) | **2.11 (3.14)** | 2.67 (2.48) | 2.90 (2.28) |
| `own` | 8.61 | 5.12 | 3.06 | 2.77 (2.39) | 2.63 (2.51) | 3.79 |
| `dir` | 10.74 | 5.77 | 3.18 | 2.57 (2.58) | 2.02 (3.27) | 2.50 |

**Hash id order** (`20261029-144749-sweep-oracle-hash`; speedups against its own `seq`, 11.34 ms):
- `own` scales furthest: 2.90× at 6 and 3.89× at 12 (2.92 ms).
- `par` collapses to 1.08–1.39×.
- In absolute time nothing beats st-order `par`.

### Workload B, 41 replay worktrees, cold

`20261029-144755-sweep-fleet`, load 5.4–5.7. Speedup is against `within-seq`: 104.8 ms for all 41 fixpoints. Replicate `-142224` (load 3.8–4.3): across 5.29× at 6, 9.07× at 12, 11.16× at 18.

| ms (speedup) | 1 | 2 | 4 | 6 | 12 | 18 |
|---|---|---|---|---|---|---|
| **across** (worker = whole worktrees, sequential kernel) | 106.5 (0.98) | 55.4 (1.89) | 29.5 (3.55) | **20.5 (5.13)** | 12.0 (8.71) | 11.2 (9.35) |
| within, `par` | 110.7 (0.95) | 70.2 (1.49) | 58.5 (1.79) | 70.6 (1.48) | 72.7 (1.44) | 85.6 (1.23) |
| within, `edge` | 137.9 | 118.5 | 89.4 | 103.1 | 107.2 | 148.5 |
| CPU ms, across | 106.5 | 109.6 | 114.2 | 118.7 | 134.3 | 143.7 |
| CPU ms, within `par` | 110.3 | 138.0 | 225.1 | 407.8 | 833.6 | 1,237.8 |
| peak private MB, across | 4.5 | 8.3 | 24.2 | 22.9 | 40.5 | 54.5 |
| peak private MB, within `par` | 8.5 | 8.2 | 8.4 | 8.5 | 8.4 | 8.7 |

Per worktree (min / median / max): rounds 17 / 21 / 24, maximum frontier width 17,444 / 94,652 / 106,383, edges scanned 265K / 1.10M / 1.18M, sequential fixpoint 1.25 / 2.56 / 2.71 ms.

**Memory against H5's second kill line.**
- Within a round, memory does not grow with threads: `par` +0.7 MB from 1 to 18 threads (thread stacks).
- Across worktrees it grows by one output slice per worker: 1.97 MB of values and frontier each. 18 × 1.97 = 35.5 MB of the +50 MB; [INFERENCE] the rest is the allocator keeping freed slices.
- `own` grows with threads beyond output slices: its candidate buffers, 7.9 → 11.5 MB.

### Why the within-round curve flattens: the counts

Per round, from `20261029-144742-sweep-oracle`. Times are medians over the 15 runs of each kernel, at 6 threads.

| rounds | frontier width | edges | `seq` ms | `par`@6 ms | speedup | what limits it |
|---|---|---|---|---|---|---|
| 0–1 | 13,655; 49,188 | 111K; 446K | 0.967 | 0.652 | 1.48× | **imbalance**: one work unit holds 25% / 46% of the round's edges, 1.50× / 2.76× a worker's ideal share. All module nodes sort together at the end of the id space (`MOD` bit) and are the hubs, up to 3,391 out-edges (`runs/20261029-142010-build-oracle/units.md`). |
| 2–3 | 254,994; 196,779 | 735K; 357K | 1.974 | 0.570 | **3.46×** | work dominates; the only rounds near linear |
| 4–11 | 17K–60K | 38K–147K | 2.389 | 1.092 | 2.19× | per-edge cost inflation (below) |
| 12–24 | 19–11,433 | 21–25K | 0.444 | 0.232 | 1.91× | **synchronisation**: rounds 18–24 cost 6–15 µs at 6 threads against 0–15 µs of work; at 12 threads, 7–17 µs |

Reading the table:
- **Where the time goes.** At 6 threads, `par` spends Σ 2.55 ms in rounds plus 0.11 ms to spawn. Each worker waits 0.46 ms in barriers (`barrier_wait_ms`).
- **Per-edge inflation.** CPU time at 6 threads is 15.9 ms against 6.0 ms sequential. Net of spinning, every edge costs about 2.2× more.
- **The inflation is write sharing.**
  - Across worktrees, workers share nothing written: CPU only 1.13× at 6 threads (118.7 against 104.8 ms) on the same cores and the same kind of rows.
  - So the inflation comes from the atomic joins and next-frontier bits written into cache lines that other cores read in the same round (rounds discover 33K–255K ids).
  - Scattering the ids makes it worse: hash order spreads every round's writes over every line, and `par` drops to 1.08× at 6.
  - Making the writes owner-only (`own`) removes it but pays buffers and a second barrier: 1.44× slower on one thread.
- **The ceiling** [INFERENCE from the table]. Suppose every round reached the best ratio observed (3.46×) and the 13 small rounds each cost the ~7 µs floor. Then 6 threads would take ≈ 1.75 ms, which is 3.4×, still under the kill line.
- **The shape caps per-round parallelism here.** There are 25 rounds. Only 2 have a frontier above 100K ids, and they carry 42% of the edges. 10 rounds carry 1.2% of the edges and are pure synchronisation.

**Why `edge` and `dir` lose.**
- `edge` splits the hub rounds evenly, but building the list and prefix sums costs more than the imbalance it removes: 1.29× at best.
- `dir`'s pull rounds write only owned values and scale furthest relative to themselves: 16.3 → 2.6 ms, 6.2× at 12 threads.
- But `dir` reads 8.80M transposed edges against push's 2.58M. 14% of the states are never reached, and every pull round scans their in-edges in full.

## 5. What this implies for E2 (GPU)

Measured frontier widths:

| Workload | rounds | widths | median width | where the edges are |
|---|---|---|---|---|
| A | 25 | 19 to 254,994 | 11,433 | 9 rounds carry 89% of the 2.58M edges (≥ 100K each); 10 rounds have < 10K edges each |
| B, per worktree | 17–24 | maximum width per worktree 17K–106K (median 94.6K) | — | 1.1M edges per worktree |

E0, by message, not re-verified: M2's `reachable` on worktree 1 has 24 levels, median frontier 1,663 and maximum 96,601 (`~/rerail-bench/facts/explore/lattice-e0/runs/20261029T134023-m2-3of8-w1`).

The baselines E2 must beat:

| | CPU |
|---|---|
| One fixpoint, one core | 6.0 ms |
| One fixpoint, best within a round | 2.4–2.8 ms |
| The whole fleet, across worktrees on 18 threads | 9.4–11.2 ms |

Implications, all [INFERENCE] (no GPU measured):
- **A single fixpoint is the wrong unit for the GPU.**
  - A GPU round needs a dispatch and a completion (or a frontier count read back) to decide whether to continue: 25 per fixpoint.
  - The work above the CPU's sequential floor is small: ~2.3M edges in 9 rounds.
  - Unless a round's dispatch plus completion is well under ~50 µs, one fixpoint on the GPU cannot beat 2.4 ms by a margin worth a backend (unverified on this machine).
- **The fleet is the right unit.** 41 worktrees in lockstep rounds is a frontier *matrix*: SpMM over the layered CSR. That is 24 dispatches for ~45M edges against the CPU's 9.4–11.2 ms. H6 should be decided there: E2's kill line against the CPU *across* worktrees, not within a round.
- **The tail rounds** (< 10K edges) should stay on the CPU or be fused: they are latency, not work.
- **Zero copy is ready.** The columns are already page-aligned mapped files, read in place by every worker (copy counter 0). Wrapping them as no-copy shared buffers needs no conversion.
- **GPU atomics.** `fetch_min` on a shared value array is cheap on a GPU, and the write-sharing penalty measured here is a CPU cache-coherence cost. The atomic `par` shape, not `own`, is therefore the one to port.

## 6. Contradictions with LATTICE.md, and proposed amendments

1. **L7, parallel within a round.** On this fixpoint it does not pay: ≤ 2.12× at 6 threads (min_level), ≤ 1.89× on the fleet. Only rounds of ≥ ~350K edges reach ~3.5×. Across contents (here, across worktrees) gives 5.1–5.3× at 6 and 9–11× at 18 with nothing shared written. *Amendment:* the scheduler parallelises across contents and independent facts first. It splits a round only when the round's edges exceed a threshold set from counts: ~300K here, deterministic, so L4 holds. The default unit is a whole fixpoint per worker.
2. **L6, "workers own only their output slices, merged by the lattice join."** The measurements give it two readings:
   - (a) An atomic join into one shared value array (`par`) is the fastest within a round, but its cost is cache-line sharing, which grows with scattered ids (hash order: 1.08×).
   - (b) Partitioned owner writes (`own`) remove the sharing but cost buffers that grow with threads (7.9 → 11.5 MB) and a second barrier. That conflicts with H5's "memory beyond output slices" line.

   *Amendment:* L6 should say which reading is meant. "Output slice" is literally true only across contents, where each worker's slice is a whole fixpoint's values (1.97 MB here).
3. **L5, work units of equal ranges of frontier or of edges.**
   - Edge ranges (`edge`) are slower here than frontier word ranges.
   - The imbalance comes from the id order: hubs adjacent at the end of the id space.
   - A hashed order fixes the balance (heaviest unit ≤ 0.14 of a worker's share, `20261029-143314-build-oracle-hash/units.md`) but doubles single-core time.

   *Amendment:* L5 should state that id assignment is part of the execution layout. Keep key-local ids. Give high-degree rows (module nodes here) their own edge-range units at build time [INFERENCE: not built].
4. **H4 as stated measures the wrong boundary.** "One core faster on the same fixpoint" holds at 45×. But the current evaluator's reachability includes evaluating its edges (63% of Y2's time is `succ`), and the lattice moves that into building the CSR (1.02–1.08× Y2's reachability, cold). *Amendment:* add to H4/H7 that the edge relation is a maintained fact, and that E3 measures CSR row maintenance per edit alongside L9's value maintenance.
5. **H3's threshold.** A median frontier width of 10K is too low as a predictor of per-round CPU parallelism. A passes it (11,433), yet rounds under ~100K edges get ≤ 2.2× at 6 threads. Per-round synchronisation is ~7 µs at 6 threads and the work per edge is ~2.3 ns.
6. **L10, layered state.** A base plus replaced rows works, and the read cost is not visible: the layered sequential pass over 41 worktrees took 104.8 ms against Σ 115 ms of flat passes in the build. But replaced rows are ~12.6K per worktree even when few files diverge, because dispatch states fan out per method (Y2's Jeremy item 2). Keying dispatch by class shrinks both the ids (883K of the oracle tree's 1.04M are dispatch states) and the layers.

## 7. What was not done

- **The current evaluators.** No `Op::Fixpoint` (facts-a) run on the same input: H4 compares with Y2 only. No run of the heaviest recursive stage E0 found (LATTICE.md's second E1 input).
- **Other experiments.** No GPU (E2) and no incremental maintenance (E3).
- **Layering depth.** Two levels only (base = the reference worktree, then per-worktree rows). Y1's cluster overlays were not reproduced.
- **CSR construction.** Built through Y2's `succ` per state, not a bulk build. No hub splitting. No persistent thread pool: spawning is included in every parallel wall time, 0.11 ms at 6 threads.
- **Machine control.** No core pinning (macOS). P/E-core placement is the scheduler's.
  - The final sweeps ran at load 5.4–5.7, the replicates at 3.5–4.3.
  - One E-core probe (`taskpolicy -b`, `/tmp` output, not kept as a run) put the sequential fixpoint at 13.6–15.0 ms, 2.5× a P-core.
  - A noisy build (`20261029-141905-build-oracle`, 3 of 7 Y2 runs doubled by a load spike) is listed but not used.
- **Process slip.** Commit `eed66c928` was committed before its T0. T0 passed on that tree immediately after (`facts-gate: passed`, 3/3). Every other commit had T0 green first.

## 8. Reproduce

```
cargo build --release -p facts-b    # copy target/release/facts-b to BIN
cd ~/rerail-bench/facts/explore/lattice-e1
E=<worktree>/spikes/lattice-e1/e1.sh
QUIET=6 Y2_RUNS=7 BIN=… $E build-oracle runs/<ts>-build-oracle          # ORDER=hash for hashed ids
QUIET=6 BIN=… $E build-fleet runs/<ts>-build-fleet
QUIET=6 BIN=… $E sweep-oracle runs/<ts>-sweep-oracle runs/<build>/graph 3   # KERNELS="par own" to restrict
QUIET=6 BIN=… $E sweep-fleet runs/<ts>-sweep-fleet runs/<build>/graph 3
python3 <worktree>/spikes/lattice-e1/e1_summary.py SWEEP GRAPH --json SWEEP/summary.json
python3 <worktree>/spikes/lattice-e1/e1_units.py GRAPH 6 12
```

`facts-b lattice run --graph G --threads T --kernel seq|par|edge|own|dir [--lattice level|set] [--fleet within|across] --out F` is one configuration: 5 timed runs, then the checks.
