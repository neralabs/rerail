# 0022: the engine's footprint within 500 MB: return freed pages, and size GPU buffers by the run

Status: accepted (2026-10-31)

## Context

After 0021, S7's Metal audit peaked at 0.94–0.99 GB of physical footprint (`ri_phys_footprint`,
which jetsam reads, so it is what the 500 MB budget means). The CPU backend peaked at 575–607 MB.
Earlier figures included W's run in the same process. Since this decision, the gate audits
before W runs: W's run left ~260 MB behind.

Footprint marks (`RERAIL_FOOTPRINT`) located the memory:
- the high-water mark (`max_footprint_mb`), live jemalloc bytes, and live Metal reservations at
  points inside each run;
- pauses at a named mark (`RERAIL_HOLD`/`RERAIL_HOLD_AT`), where `footprint` and `vmmap` can read
  the categories.

What they found:

| Source | What held it |
|---|---|
| Freed heap pages | jemalloc keeps them for a second (`lib.rs`'s `dirty_decay_ms:1000`). After the audit 62 MB was live and ~600 MB was held. |
| The system allocator | SQLite's memory: 142 MB held for 30 MB live, fragmented, so pressure relief could not return it. |
| Bound Metal buffers | A run charges every bound buffer's whole length again as GPU memory. Separately, the driver owns a fixed ~170 MB for any execution. |
| Buffers sized by the plan, not the run | The label prefix had all 833 labels (34 MB); scratch had a row per label (33 MB); counts had a row per count (24 MB); the pool's stacks were 31 MB. |
| Host copies | Dense empty label sets (34 MB) were held through every run. Every run's root lists were built in planning (~118 MB). A second copy of the labels was made at each run's end. |
| The roots stage | It held every range's execution, each with its packed batch (~200 MB). Each capture row's structure key was its subtree's text (235 MB for 31,704 rows). |
| The sweeps | Three label programs needed 5–7 cursors, so 498k (node, program) calls fell back to the interpreter. |

## Decision

1. **Freed pages are returned at stage ends.** `prof::release` purges every jemalloc arena. It
   runs:
   - at the end of every audit stage;
   - before each GPU run, after its sizing, and after its encoding;
   - after the roots' partitions, and after each roots range.

   In all it costs 50–67 ms of S7's CPU time.
2. **SQLite allocates from jemalloc** (`SQLITE_CONFIG_MALLOC`). A load-time initializer installs
   it in the addon and the test binary alike, before anything can initialize SQLite. Freed SQLite
   pages are then purged with the heap's.
3. **GPU buffers are sized by the run.**
   - **Label prefix:** only the labels a run reads or writes, by row (`label_rows`). Walk class
     tests, settle ops, complements and pattern constraints use rows, and the host scatters the
     rows back into the full labels.
   - **Fallback:** where an interpreter task or a GPU sweep could read labels by index, the
     prefix keeps every label.
   - **Scratch and counts:** a row per pass and per count of the widest level. A level's settle
     clears them.
   - **Pool stacks:** 1024 threads.
4. **No duplicate host labels.**
   - Planning holds no label sets.
   - A run's roots are built when it starts and freed with its plan.
   - A run's result comes back in the plan's own allocation.
5. **The roots stream into rows.** Each execution's rows are taken as soon as it finishes, and
   then it is dropped. A structure key holds 128-bit digests of its syntax and semantic parts:
   the SQL only compares them for equality.
6. **Walks carry 8 cursors** (was 4), so no S7 label program falls back to the interpreter.

## Consequences

**Footprint, measured engine-only** (lifetime `phys_footprint_peak` at a pause after the audit):

| Backend | Before | After |
|---|---|---|
| Metal | 0.99 GB | 477 MB |
| CPU | 575–607 MB | 447 MB |

**Time.** On Metal, S7 takes 4.6–5.6 s wall / 10.2–11.6 s CPU, against the CPU backend's
20.5–24.4 s / 21–25 s (load 25–45). Both backends find exactly W's 710 keys, and both mutations
fail the gate.

**Fixed costs that remain.** The driver's ~170 MB for any GPU execution is outside the buffers'
control, and the live heap during a run is ~160 MB: tapes, candidates, and the host labels.
Sparse host labels are next if the corpus grows.

**Spike sources:** the indexes record peak footprint per stage. These are `max_mb` and
`live_mb`, in `prof::peak`, along with `sys_live_mb` and `sys_held_mb`.
