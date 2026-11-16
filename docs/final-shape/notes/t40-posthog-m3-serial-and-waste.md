# t40: all of PostHog on the M3 Max: where the run is serial, and where it wastes work

Measured 2026-11-15 at 0af0aaaf9 (after decisions 0278 and 0279 §6–§8). `len-_py.rr` over all of PostHog (29,885
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
  the shape decision 0279 §8 removed from `exports`. A planner rule for it (0279 Open) would cover both.
- **`sym_of` and `sym_of__sup0` are 6.4 M and 4.5 M rows**, one a name node, written whole to the frames
  (39 MB and 33 MB): most of the frames' 0.7 s and much of the cross part.

## Open

- The start-up's 0.6 s is not sampled.
- Whether the in-order filing with the main thread helping is what idles the pool: a run whose main thread helps
  only with small jobs would tell.

## Follow-up (2026-11-15, after the bell, ByScope and paths resolved together)

The same run on the M3 Max, profiled, at b476bbf52 plus the observer's root test by bytes. Findings unchanged in
every run (29,747, the same set).

- **The main thread no longer schedules by running chunks.** Before the bell (decision 0254 §5) its waits ran
  others' jobs: "a chunk evaluated" 11.2 s waiting of which 9.6 s inside jobs, and made 305 waits late. With the
  bell it waits idle (10.5–11.5 s) and the pool takes the chunks. Wall 40.7/39.5 s → 36.9/35.3 s. Workers taking
  Begin and Finish themselves is not needed while the main thread's waits are idle.
- **The process-wide maps of `setfix` are sharded by connection** (`rerail_accel::sharded::ByScope`): within
  noise on the wall (37.4/36.5 → 35.8/36.7 s); `__psynch_mutexwait` is gone from the strata's samples.
- **Phase 1's fixpoint.** Path resolution runs on the pool (`Observer::canonical_all`), and the observer's root
  scans compare bytes (`under`) where `Path::starts_with` parsed both paths per root per call: in a 4 s sample of
  the fixpoint, 61 main-thread frames → 0. Phase 1: 3.85 s → 3.69/3.67 s; "requests answered inside the fixpoint"
  3.39 → 3.23/3.24 s.
- **Still serial in phase 1:** the cone's strata (`ask_loaded_bff`, `ask_py_…`: about 0.6 s each, one walk each,
  in one job), and "answer load" (about 1 s on the main thread). The cone's documents are built on the pool.
- **Then (decision 0292):** a CPU timeline showed the loads answered with 8–12 cores busy, and then 1.6–1.8 s on two:
  the one recursive import stratum (50 relations), whose joins make text on every row and so walked whole. They
  now walk in morsels that defer their new texts (one walk's atoms), a load's chunks evaluate at once, and a
  long-text read goes with the file-local join that binds its node (which also fixed two tests that failed on many
  threads). Phase 1 3.80/3.77 s → 3.60/3.53 s; the stratum's joins 0.70 s → 0.30 s on its thread (1.1 s CPU in
  650 morsels). "Answer load" was waiting for documents built on the pool, not serial work.
- **Then (decision 0294), the cross-file part:** its strata are a chain (each reads the one before), and a level's
  strata already overlap; the big walks were in morsels, but the work around them was not. On the M5 Max under load
  (the M3 unreachable): spans per step showed `sym_of__sup0`'s scan order sorted on one thread (0.19 s), its morsels'
  runs merged on one thread (0.13 s), and a false slow-join flag (a cut walk in morsels counted more work than its
  counts, stopped short of its check, and was judged by a second walk's 19 rows) taking exact statistics (0.1 s).
  Then each walk going on from a cut sorted all 2.2 M rows again. Sorts, merges, key reads and statistics now run
  on the pool, a cut walk counts as one, and its rows left are read in their order: the cross part's strata 1.74 s →
  1.07 s, `sym_of__sup0` 0.5–0.6 s → 0.18 s, the same findings. A whole plan's exact statistics read the wrong join
  and panicked about one run in six (fixed, afe32582a).
