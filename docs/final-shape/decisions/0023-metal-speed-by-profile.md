# 0023: Metal's speed by profile: GPU occupancy, and the host work around the GPU

Status: accepted (2026-10-31)

## Context

After 0022, S7 on Metal took ~4.8 s wall / ~10.3 s CPU, against the CPU backend's ~20.5 s /
~21 s (load 25–45). The profile split the Metal run into the work only Metal does and the
work both backends share. It used process CPU per top stage (`cpu_ms.*`) and GPU timestamps per
command buffer (`gpu.device_us`, `GPUStartTime`–`GPUEndTime`).

**Labels, Metal's own stage:** 1.32 s wall / 5.45 s CPU on Metal, against 16 s on the CPU
backend. Almost none of that CPU was the GPU run (`labels.gpu` 134 ms):

| Cost | Size | Where it went |
|---|---|---|
| Text rows | 2.5 s CPU | Each text test was evaluated over every node before each run, one slot at a time (54 slots, ~6 ms each). The byte NFA kept scanning after its states died, clearing an NFA-sized array per byte. So an anchored test read every ancestor's whole text, quadratic in depth. |
| Host sweeps of recursive groups | 1.9 s CPU | 17.5 M (node, pass) entries were sorted (278 of 388 ms to build them), each pass's walk was looked up in a map per entry, and confirming sweeps reran passes whose inputs had not changed. |
| GPU time | ~350 ms | ~280 ms was `label_micro`: the loop tier's 3% of starts, run on a pool of 1024 threads, too few to occupy the GPU. |

**Shared stages:** the candidate SQL took 1.34 s. Its final `ORDER BY qid,fid,pre` made SQLite
sort each of ~90 UNION ALL arms in a temp B-tree, even though each arm already comes in
(fid, pre) index order. Roots (JIT execution), ingest and layering are the same on both
backends.

## Decision

1. **One parallel pass for text rows.** A labelling's text rows are evaluated before its first
   run, one task per (slot, file), into `TextRows`.
2. **The byte NFA stops once no state is alive.** It also reuses its buffers and marks states
   seen by generation instead of clearing an array per byte.
3. **The loop tier's pool is 2048 threads of 64 frames.** It uses the same stacks as before, now
   holding twice the threads.
   - A run whose walk outgrows them reruns with 1024 threads of 128 frames
     (`label_micro_deep`), before the interpreter.
   - Before this, the interpreter was the only retry, and on S7 it failed the audit
     (`BackendFailure`).
   - 4096 threads of 32 frames overflowed twice on S7, and the reruns cost more than they saved.
4. **Host sweeps:**
   - entries are built node by node, so they come out in order without a sort;
   - each pass's walk is resolved by its index;
   - a pass whose read labels did not change since it last ran is skipped. A pass reads its
     walk's label class tests, its label domain, or its complement's source, and is
     deterministic in them.
5. **Candidate SQL streams its arms unsorted.** `select` sorts each plan's rows in Rust, in
   0.5 ms, since they arrive nearly sorted.
6. **On Metal, labels run beside the candidate SQL.** Every (clause, file) is active. The GPU
   pass is dense, so this costs the same: ~0.9 s wall either way, +0.1 s CPU, 8,446 → 21,750
   active pairs. Labels run on a scoped thread while the SQL runs on the store's connection. The
   CPU backend keeps its order, because there its labels would grow with the mask.
7. **Ingest parses a chunk's sources in parallel** (`parse_ahead`), before storing their tapes
   in order: `slice.push` ~470–580 → ~370–400 ms.

8. **A text test under its NFA's work bound runs as a DFA search.** The byte NFA charges at
   most `work_per_byte` per byte read; on a text shorter than `MAX_STEPS_PER_PATH /
   work_per_byte` it cannot reach the work limit, so a `regex_automata` meta search of the same
   pattern gives the same bit. On S7 the two agreed on all 12,061,817 such texts (checked once
   against the NFA). Text rows: ~0.9 → ~0.2 s CPU, on both backends.

9. **Candidate SQL skips releases past its last query.** They were a row-by-row `DELETE FROM
   candidate_keys` (~185 ms) of a table the cleanup drops whole.
10. **The CPU JIT runs evaluations in parallel, each one's tasks in order, with no locks.**
    Tasks locked their evaluation's mutex, so workers parked on a few hot evaluations: a range's
    run was 2–4× its longest evaluation. `jit.run` ~367 → ~268 ms.
11. **`to_words` fills each label's row in parallel:** 83 → 18 ms.
12. **Prefilter publication sums its store-wide counts over the batch** and writes one upsert a
    key, postings sorted across the batch. A tape already published is deleted only after the
    sums so far are written, as one tape at a time would have. `slice.push` ~420 → ~330 ms.
13. **Footprint during the overlap.** The text rows' searches use a cache per call rather than
    the regex's per-thread pool, and the candidates' rows are (tape index, preorder) `u32`s.
    Overlapped with the GPU runs, those had pushed the peak to 501–503 MB; it is 461–495 MB.

These were tried and dropped:

| Tried | Why dropped |
|---|---|
| Evaluating a text test that reads the walk's start only at the pass's roots | S7's roots cover ~274 k of its nodes, so it saved nothing. |
| Recursive groups as GPU rounds instead of host sweeps | It exceeds the launch budget (`BackendLaunchLimit`), because chains are deep. |
| Roots on the GPU (`MetalJit` instead of 0017's `CpuJit`), re-measured after these steps | Metal splits S7 into 260 ranges. Compiling them took 59 s and executing 4.76 s, against 0.8 s for the CPU JIT's whole roots stage. 0017 stands. |
| A 32 MB page cache (main and temp) for the candidate statement | Candidates went ~1.1–1.2 → 0.97–1.09 s, but the peak rose to 536–538 MB, over budget. |
| Building a range's rows per file in parallel | Rows went ~300 → ~220 ms, but CPU rose ~0.4 s and wall did not change measurably. |
| A 256 KB temp page cache for the candidate statement | Peak ~10–20 MB lower, candidates ~120 ms slower. |
| A 4× range budget for the roots | Still 45 ranges: the budget does not bound them. |
| Roots on the GPU, with the JIT cache keyed by program digests | The 59 s was the key: each lookup generated the kernel source; keyed by digests, `roots.jit` is 51 ms. Execution stays 4.4 s for 260 ranges, because a range dispatches 64 lanes (`EXECUTION_LANES`) for ~9 k tasks, and the peak is 1.56 GB. 1024 lanes fails with `BackendCapacity`: each lane's arenas are sized by its widest task. Wide GPU roots need that layout changed first. |
| GPU roots with budgeted arenas and wider dispatch | Read back, the evaluations' arenas use at most 4 results (24–28 a range) of 4 M reserved. Capping each lane's transient arenas at 1,024 records and the durable ones at 65,536, rerunning unbudgeted on any `BackendCapacity` (none did; 710 keys): 64 lanes 4.1 s, 128 2.4 s, 256 1.6 s, 512 1.36 s, 1024 11.3 s. The peak fell 1.56 GB → 632–690 MB, still over budget, and 1.36 s loses to the CPU JIT's ~0.6 s: 260 ranges each pay a queue, buffer uploads and a batch. Sized for the GPU (the CPU's partition, 45 ranges): 256 lanes 1.37 s, 512 lanes 1.03 s, peak 654–688 MB. Fewer ranges saved ~0.3 s, so the cost is the GPU's execution of 1.15 M scattered, divergent root tasks, not the per-range overhead. 0017 stands. |
| Two roots ranges at a time (0020 measured all at once) | `demand.execute` ~100–200 ms less, but the peak rose to 518–523 MB, over budget. 0020 stands. |
| Absorbing a roots range's rows on a thread while the next range runs | In an interleaved A/B the wall time did not change and CPU rose 0.7–1.3 s: the rows thread competes with the JIT's pool. |

## Consequences

**S7, one process, `cpu,metal`, after steps 1–5, load 28–40:**

| | Before | After |
|---|---|---|
| Metal, wall | ~4.8 s | 3.76 s |
| Metal, CPU | ~10.3 s | 7.09 s |
| CPU backend, wall | ~20.5 s | 17.2 s |
| CPU backend, CPU | ~21 s | 18.3 s |

The CPU backend also gained from steps 2 and 5.

**After steps 6 and 7:** Metal alone took 2.8–2.96 s wall / ~7.5 s CPU at load ~30. One
`cpu,metal` process at load 40–63 gave:

| | Wall | CPU |
|---|---|---|
| CPU backend | 17.6–18.2 s | 18.8–19.1 s |
| Metal | 3.3–3.9 s | 7.8–8.7 s |

**After step 8,** one `cpu,metal` process at load 14–32: CPU backend 16.0–16.3 s wall /
17.0–17.5 s CPU; Metal 3.1–3.2 s wall / 6.9–7.1 s CPU (~5× wall, ~2.5× CPU).

**After steps 9–13,** one `cpu,metal` process: at load 18–27, the CPU backend took 14.9–15.7 s
wall / 15.7–16.4 s CPU and Metal 2.43–2.54 s / 6.4–6.6 s; at load 30–44, 16.5–17.4 s / 17.4–18.1
s against 3.0–3.1 s / 6.9–7.0 s. That is ~5.5–6.2× in wall time and ~2.5× in CPU.

**Labels alone:** Metal ~0.85 s wall / ~3.1 s CPU, against the CPU backend's ~14 s / 13.4 s.
That is about 16× in wall time and 4.4× in CPU.

**End to end** the ratio is ~5–6× in wall time and ~2.3× in CPU (it was 4.6× and 2.6× after
steps 1–5, at lower load). It is bounded by the shared
stages: candidates ~1.0 s, execute ~1.05 s (2.2 s CPU), push ~0.55 s, layer ~0.22 s. Together
they are ~2.8 s wall and ~4.1 s CPU. These stages are CPU work in both backends.

**The bound.** On Metal the labels run beside the candidate SQL and finish first, so Metal's wall
time is already the shared stages' (candidates ~1.0 s, roots ~0.9 s, push ~0.3 s, layer ~0.2 s).
With labels free, the wall ratio could not pass ~15 s / ~2.4 s ≈ 6×; with labels at zero CPU,
the CPU ratio could not pass ~16 s / ~3.5 s ≈ 4.5×. A higher ratio needs the shared stages
themselves cheaper: the candidate SQL's lowering, or the roots on another engine (0017). A
timestamp-precise sweep skip (rerun an entry only where a label it read changed since) did not
help: the second sweeps already skip, and the cost is each entry's first visit.

**Correctness and footprint:** 710 keys equal W's on both backends, both mutations fail, and the
peak footprint is 461–495 MB (`phys_footprint_peak`). The peak is set during a labels GPU run
that overlaps the candidate SQL, so it varies with their timing.

**What remains:**
- The candidate SQL still sorts 37 UNION CTEs for merge-dedup, and materializes complements
  (`all_nodes EXCEPT …`, ~300 k rows). This is the prefilter lowering's own work, iterated in
  its lab.
- The loop tier could keep walks that overflow apart and rerun only them, not the whole run.

**Spike sources:** `prof` adds `cpu_ms.*` (process CPU for `slice.`, `demand.` and `labels.`
stages), `gpu.device_us`, `threads.<kernel>` and `size.text_slots` to the stage indexes.

## Re-profile, 2026-11-01: GPU counters, per-kernel times, and the CPU's shape

**Tools:** Metal System Trace (`xctrace`, 40-core M5 Max) gives per-encoder GPU intervals. Apple's
GPU exposes only `GPUTimestamp` to applications (`MTLCounterSet`), so there are no occupancy or
ALU counters. `RERAIL_GPU_KERNEL_TIMES` commits each kernel as its own command buffer and adds its
GPU time to `kernel_us.<label>`. The CPU side is from the xctrace Time Profiler.

**GPU:** the whole S7 run is ~280 ms of GPU time over a ~2.4 s run, so the GPU is idle about 90% of
the time. Of that GPU time:

| Kernel | Items | GPU ms (pool 2048 → 4096) |
|---|---|---|
| `label_micro` (loop tier) | 1.73 M | 193 → 121 |
| `label_micro_8` | 81 M | 78 |
| settle, patterns | — | < 1 |

The loop tier is a pool of threads that stride over the items, so its pool size is its occupancy.
At 8192 threads it runs in 85 ms, but the peak footprint reaches 511–515 MB, over budget. The pool
stays at 4096 (commit 19052e067).

**CPU:** the CPU backend ran non-recursive labels on the interpreter, a task per start, one file
after another (~6.5 s of 12.5 s of labels in the Time Profiler). They now run the same micro walk as
Metal and the sweeps, with files in parallel (b40b897be):

| CPU backend | Before | After |
|---|---|---|
| Labels | 12.5 s | 1.4–1.8 s |
| S7 wall | 15.7 s | 4.1–4.7 s |
| S7 CPU | 16.4 s | 7.3–7.6 s |
| Peak footprint | — | 358 MB |

The "Metal ~6× the CPU" figure was mostly the CPU running a worse shape, not Metal being fast.

**The common floor:** both backends now spend most of their time in the same CPU stages, about 2.4 s
together:
- candidates SQL, ~1.1 s on one connection;
- execute (roots on the CPU JIT, then rows), ~1.0 s;
- push, ~0.35 s;
- layer, ~0.25 s.

The labels are the only work Metal takes off this floor. For Metal to be much faster end to end, the
floor has to move.

**Where the candidate SQL goes:** S7's stages total ~430 ms, plus ~570 ms for two queries:
- `r161` is `all_nodes EXCEPT r12`, 239 K + 130 K rows, 146 + 72 ms.
- `r2` is 300 K rows, 72 ms.
- `r44` and `r210` follow, under ~75 ms each.

Many sets are "every node of the files in X" or a complement, stored as one `(fid, pre)` row per
node. The shape to change is the row format: a candidate set as `(file, all | bitmap)`. Complements
and whole-file sets then cost one row per file, and an intersection becomes a bitmap `AND`, which
the GPU can also do. That is a prefilter-lowering change, iterated in its lab.

### Candidate SQL, measured in the lab (`candidate_replay` over S7's captures)

The candidates return 1.15 M + 0.76 M rows. The largest plans have no root kinds; they are unions
with sets of every node in some tapes, e.g. plan 38 = `r58 ∪ all(tapes r39) ∪ all(tapes r76)`.

- **Kept, 5479c6563:** `(L \ R)` within tapes is built as `(L within tapes) \ R`. A full
  complement is no longer materialized; `sql.prepare` went from ~480 ms to ~270 ms.
- **Kept, f0a1cba36:** in a plan's output, a whole-tape set is emitted as one row per tape. `pre` is
  minus the tape's node count, and these rows come before the plan's other branches, which are one
  union. `execute` expands them and drops the other arm's rows in those tapes.
  - `sql.query` went from ~550 ms to ~345 ms; `demand.candidates` from ~0.85 s to ~0.62 s.
  - Every plan's rows are identical (count and checksum per plan and chunk).
  - Metal peak is 489–506 MB. A per-plan dedupe set had pushed it to 495–503 MB; it was replaced by
    the single union arm.
- **Dropped:** flattening nested tape tests, so that the tapes of `within(Y, T)` are tested as
  `Y`'s and `T`'s. Rows were the same, but VM steps rose from 74.5 M to 84.6 M with no CPU gain:
  each `IN (SELECT …)` builds its own ephemeral index.

### End to end, 2026-11-01 (load 23–27)

| Backend | Wall | CPU |
|---|---|---|
| CPU | 3.6–3.7 s | 6.9 s |
| Metal | 2.3–3.1 s | 6.4–6.7 s |

Both give 710 keys, equal to W.

Metal's whole gain is the labels, which overlap the candidates there. Everything else is the same CPU
work on both backends: candidates ~0.6 s, roots and rows ~1.0 s, push ~0.35 s, layer ~0.25 s. The
GPU's own time is ~200 ms.

For Metal to be much faster, roots, rows or candidates have to run on the GPU. Roots on the GPU
were measured four ways in this decision; the best took 1.03 s, against 0.6 s on the CPU JIT. The
next shape to try is candidate sets as per-tape bitmaps with GPU set algebra, which would also
remove the row-per-node SQL.

### Parallelism per stage, and the roots' shape (2026-11-01, load 23)

Process CPU over wall per stage:

| Stage | Metal | CPU backend |
|---|---|---|
| labels | ×4.8 (3.7 s CPU) | ×2.3 (3.8 s CPU) |
| roots | ×2.4 | ×2.3 |
| rows | ×0.9 | ×0.9 |
| layer | ×1.0 | ×1.0 |
| push | ×2.3 | ×1.3 |
| candidates | — | ×1.0 (one connection) |

- **Metal's labels still spend as much CPU as the CPU backend's.** The GPU does ~0.2 s of them; the
  recursive sweeps and text rows stay on the CPU (O26).
- **Candidates cannot close the gap on their own.** With free candidates, Metal would be push +
  labels + execute + layer ≈ 2.3 s against the CPU's ≈ 3.3 s, which is 1.4×.
- **Every S7 root program is a micro walk** (173 of 173, 1.9 M starts). The GPU-roots attempts above
  ran the interpreter's general machinery; the regular shape for them is the label kernel's walk.
  - At the labels' rate (82 M starts in ~1 s of CPU), the roots' 1.9 M starts would be tens of ms.
  - What it needs: the walk emits each path's start, output label and labelled captures, and the
    rows stage reads those in place of `PackedExecution::program_results`, with the same ordinals,
    result limits and statuses.
  - That change speeds up the CPU backend by as much as Metal. Metal's lead comes only from what
    the GPU runs faster than the CPU's cores.
