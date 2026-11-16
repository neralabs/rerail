# t40: all of PostHog on the M3 Max: where the run is serial, and where it wastes work

Measured 2026-11-15 at 0af0aaaf9 (after decisions 0263 and 0264 §6–§8). `len-_py.rr` over all of PostHog (29,885
files), cold store, profiling build, K = 15, load average 6–11 from other work.

## Intro

A run of 40 s has about 12 s on one or two cores, and in its parallel part (the chunks) the pool's threads are
idle about 40% of the time. This note maps both, from three unsampled runs, the engine's busy cores every 0.2 s
beside the main thread's open span (`cpu.sh`, `saw.py`), and stack samples of each window (macOS `sample`).

## The run

| run | wall | user | sys | violations |
|---|---|---|---|---|
| m-a | 43.0 s | 231.6 s | 73.7 s | 29,747 (483a6f116876) |
| m-b | 43.1 s | 245.5 s | 77.8 s | the same set |
| m-c (CPU timeline) | 40.1 s | 231.9 s | 77.8 s | the same set |

The timeline of m-c:

| when | cores busy | what |
|---|---|---|
| 0–0.6 s | ~1 | start-up (not sampled) |
| 0.6–2.0 s | ~14 | phase 0 |
| 2.2–3.6 s | ~1 | phase 0's observations written; phase 1 prepared (timing varies run to run) |
| 3.6–6.1 s | ~12 | phase 1's sources |
| 6.3–8.5 s | 1–2 | phase 1's import fixpoint; the files consulted digested |
| 8.6–33.0 s | ~10.5 of 16 | the chunks |
| 33.2–37.2 s | 1–2 | the cross-file part, then its frames |
| 37.2–40.1 s | ~1 | the store drained, a collection pass, the process's exit |

## Serial

1. **Phase 1's import fixpoint (about 1.5–2 s, one thread).** "Requests answered inside the fixpoint": one pool
   thread answers 29,771 loads, runs `ask_py…` (50 relations) and the cone, in one native stratum whose joins walk
   whole. The main thread sleeps in a polling loop (`threads::Beside`, `std::thread::sleep`: 703 of 1,334 samples),
   and the pool is idle.
2. **The cross-file part (about 4 s, mostly one thread).** Its strata run one after another: `sym_of__sup0`
   0.81 s CPU (its walk 0.29 s), `sym_of` 0.40 s, `rr_diagnostic` 0.38 s, `export_query__sup0` 0.25 s,
   `rr_agg7__sup0` 0.23 s, `export_query`, `P__sup0`, `violation__sup0` 0.12–0.14 s each. Strata of one level
   ("a level of 10 strata placed") do not overlap much: one thread is busy 991 of 1,452 samples. Then the kept
   part's frames (`write_frames`, main thread) take 0.70 s.
3. **The tail (about 2.5 s).** The main thread waits for the store: pending atoms written row by row
   (`Atoms::write_pending` through `bounded_insert`, about 0.5 s), the frames (about 0.4 s), a collection pass over
   the store (`collect_corpus_pass`, about 0.6 s), checkpoints, then the process's exit (about 1.3 s after the last
   span).
4. **Path canonicalization on a cold store (about 0.5 s, main thread).** `World::allowed` asks the observer for
   each path's canonical form, and on a cold store the journal vouches for none: a `realpath` per path
   (`getattrlist` per component). Plus the files consulted digested one by one (`World::consulted`,
   `Observer::digest`, about 0.3 s).

## The chunks: idle and waste

Over a 3 s sample of the chunk phase, the main thread and the 15 pool threads are busy 59% of their samples.

- **The main thread evaluates chunks while it waits** (`help_once`: about 60% of its samples). Filing is in order,
  and while the main thread is inside a chunk's evaluation it begins and files none, so the pool runs dry. This is
  the likely cause of most of the idle 41% (not yet confirmed by a change).
- **Process-wide locks on every stratum of every chunk:**
  - `setfix::COMPONENTS`, a `Mutex<Vec<…>>` scanned linearly by (scope, name) under the lock (`component_in`), and
    the other process-wide maps of `vtab::Db`: `__psynch_mutexwait`, 830 samples (about 3% of thread time).
  - `plan::table::NAMES`, a global `RwLock` read on every lookup of a relation by name (`Module::live_sizes`,
    `plan_rule_reused`, `fork_with`, `Body`): `lock_shared_slow` and spinning `cthread_yield` under it, with the
    hash and compare of each name.
  - `plan::joins::MEMO`, the planner's process-wide memo, read on every rule planned (`joins::pending`).
- **rayon's idle spin** (`yield_now`, `swtch_pri`): 1,432 samples (about 6% of thread time), part of the run's
  75 s of system time.
- **The profile's own clock** (`clock_gettime` of thread CPU per span): about 2.5% of thread time. The walls here
  are profiled runs.
- **Statistics measured per chunk** (`corpus::sampled_statistics`, a sort per column): about 1.4%. Real work.

## Inefficiency beyond the chunks

- **`rr_agg7__sup0` is another split joined by text** (`$q = @cat($m, $x), @contains($x, ".") = 0, exported($m, $x)`),
  the shape decision 0264 §8 removed from `exports`. A planner rule for it (0264 Open) would cover both.
- **`sym_of` and `sym_of__sup0` are 6.4 M and 4.5 M rows**, one a name node, written whole to the frames
  (39 MB and 33 MB): most of the frames' 0.7 s and much of the cross part.

## Open

- The start-up's 0.6 s is not sampled.
- Whether the in-order filing with the main thread helping is what idles the pool: a run whose main thread helps
  only with small jobs would tell.
