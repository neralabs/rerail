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
