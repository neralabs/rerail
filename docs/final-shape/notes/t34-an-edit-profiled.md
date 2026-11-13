# t34: A one-file edit, profiled top-down and bottom-up, and the work it leaves

Status: work plan (2026-11-12). Kept current as each item is done: an item names its commit when it is.

## Intro

An edit of one line in one file of a 20-file Python corpus (the pack's 278 programs, store `n48`) costs, after
the steps below: **122–131 G instructions, 5.0–5.5 s wall, about 9 s CPU**. A cold run of the same corpus costs
2,010 G. The goal is an edit five times cheaper than the start of this work (147 G, 6.8 s wall, 10.4 s CPU) by
making each step efficient, before any change to how a file's own rows are kept (doing the local part by delta
costs more to compute and store than it saves at this size, and is hard to make cheap later).

How it is measured:

```text
RERAIL_START_PAUSE_MS=2000   the engine waits before its first step, so `sample` (attached by pid) sees all of it
RERAIL_TIMELINE=1            each section's end with its CPU, and the main thread's spans as they open
prof.py sample.txt out 25    top-down tree per thread group, and bottom-up self time with the callers' paths
lines.py ... --self          a frame's own samples by source line (needs a dSYM: `dsymutil` of the build)
```

The first run after a rebuild is cold (the build is in the plans' keys); measure the second and later. Read the
component count of every run ("N components native"): after one rebuild, edits alternated between 16,990
components (cross 22 G) and 17,833 (cross 49 G) for eight runs, then settled. Not explained yet (item 9).

## The profile (2026-11-12, load about 6)

8.5 s busy across threads: main 3.9 s, workers 3.2 s, the store's writer 0.7 s, the cost model 0.3 s, the
store's collection 0.2 s.

| Where | Busy | What |
|---|---|---|
| Workers: evaluating rules | 2.0 s | the join loop (`setfix::run_join_in_each`): `candidates` and its binary searches 0.64 s, `flush` and sorting 0.3 s, `value`/`locate` 0.25 s |
| Workers: planning | 0.3 s | `plan_rule_reused` → `plan_search`: an edit still searches plans |
| Main: preparation read back | 0.6 s | the prepared programs 0.42 s, the combined program 0.12 s, the choices 0.08 s |
| Main: kept inputs and the cross part's setup | 0.4 s | `kept_inputs`, `cross_rows::decode`, `supports`, `retain::key`, sorting |
| Main: the rule texts read again | 0.45 s | `rule_files` 0.16 s, `texts_digest` twice 0.11 s, `program_texts` and phase 0 0.24 s |
| Main: whole-program passes | 0.5 s | `bridge::facts` 0.14 s, strata (Tarjan) 0.17 s, `text_kind_demand` 0.08 s, the constants and semijoins 0.12 s |
| Main: the engine's driver | 0.45 s | issuing jobs, `slice_contains` 0.05 s, `choices::lookup` 0.08 s |
| SQLite's I/O | 1.1 s | `pread` 0.53 s (a 1 MB page cache, no map), `pwrite` on checkpoints 0.26 s (most at the cost model's close), `fsync` 0.17 s, vacuum 0.11 s |
| The cost model | 0.3 s | a new model trained by every process |
| The store's collection | 0.2 s | an incremental vacuum by every process |

## The profile after items 1–5 (2026-11-12, load 14–25)

An edit: 118–124 G, 8.5–9.4 s CPU. 8.7 s busy: main 4.35 s, workers 3.6 s, the writer 0.59 s, the cost model 0.12 s
(the saved model put in place, no training), collection 0.07 s. **The main thread is the critical path**: busy 4.35
s of a 5–6 s edit, while the workers are idle most of it (3.6 s over 18 threads).

Sections of the main program's run (`RERAIL_TIMELINE`, CPU of all threads): local 4.0 s, cross 1.4 s, prepare
1.1 s, stats 0.8 s, waits 0.26 s, combine 0.44 s.

| Main thread | Busy | What |
|---|---|---|
| The engine's driver on main (`run_atoms_native_over`, `_hooked`) | 0.96 s | `plan::run`'s strata loop: `form`, `strand_forms`, `slice_contains` 48 ms, `choices::lookup` 98 ms, strata 42 ms |
| Prepared programs read back (`prepared::lookup`) | 0.50 s | `prepared_from` builds the program 0.27 s; `decode_view` 75 ms; `facts_memo::decode` 80 ms |
| The cross part's kept inputs and statistics (`{c273}`) | 0.46 s | `kept_inputs` 127 ms, `maintained_cross`/`supports` 73 ms, sorts 44 ms, `retain::key` 32 ms |
| `bridge::facts` | 0.16 s | |
| `combine_with` → `combined::lookup` | 0.14 s | |
| Drops of the run's structures | 0.12 s | |
| `from_frames` (identities, strata) | 0.12 s | |
| `text_kind_demand` | 0.11 s | |
| The constants and semijoins (`file_local`) | 0.17 s | |
| Per-item spans | | "native" 17,310 times, "key" 5,851, "hold the export"/"attribute to files"/"totals" 5,628 each |

SQLite's I/O is down to about 0.3 s (`pread` 63 ms, `pwrite` 90 ms, `fsync` 65 ms, the checkpoints). Planning on
the workers is 0.1 s (`plan_rule_reused`). The joins are 2.2 s on the workers, off the critical path.

The cost store gets about 6,000 samples a run: training is due again every 10 minutes (`RETRAIN_EVERY`).

## Done

- Prepared programs, the combined program and the stored choices read in place off their encodings
  (`program_facts::decode_view`): the main program's lookup 1.25 s to 0.5 s; the combined 0.47 s. (197b8abfc,
  2085e7ca1)
- The phases' programs were collected after every process: a plan was live only while a root's last two runs
  named it, and a process records a run per phase. (197b8abfc; its `NESTED_PLANS` static is item 3's to replace.)
- Six quadratic scans indexed (`projection_of`, inputs bound unchanged, `foldable`, `split_constants`,
  `semijoin_bases`, recursion in `supports`). (197b8abfc)
- Phase 1 no longer waits for the writer to reach its manifest (0.23 s). (197b8abfc)
- The engine's waits wake when a job ends, not after a nap of up to 1 ms (0.53 s on the critical path). (197b8abfc)
- The store's connections read through a map of the file (`RERAIL_STORE_MMAP_MB`, default 4096; 0 is off): about
  6 G and 0.6 s CPU an edit. (5a4f1a1cf)
- The program texts read once (item 1 below): `rr::read_seed` parses each once; `corpus::Texts` holds the run's
  reads and hands their seeds, graph digests, library files, names and phases on to the cutoff, its record,
  `phase_files` and `texts_digest`. An edit: 118–121 G (was 122–131 G). (59c8204e9)
- Caches refused by construction (item 2): `facts_only` green, in every targeted test run; a commit adding a
  static map or table it does not list, or a new entry in its lists of state, is refused before it is made
  (`scripts/check/check-no-new-caches.py`, run by a hook before each of my commits). (07f7ff7f7)
- My statics by flow (item 3): the frames' four tables are the store's schema (no lookups, no lazy creation);
  the confirmations and the admitted memo spaces live in the frames' snapshot; each run's report carries the plans
  it and its nested runs used (`Report::plans`, phase 0's on its graph), so the outermost run's record names them
  without `NESTED_PLANS`; the journal's volume per root is the observer's (`Observer::volumes`), not a process-wide
  table. (07f7ff7f7)
- Item 8, first part: the cross part's strata computed once and passed to each pass over it (the kept part's load,
  `cut`, the frames read and written, the derived relations, `maintain::rewrite`), not by each; `bridge::wanted`
  no longer scans every rule per relation read, and skips a program declaring no bridged relation. An edit:
  114.6–115.0 G, 7.7–8.0 s CPU (was 117–124 G). The prepared programs' decode (0.26 s) reads four programs, all used
  in an edit: left. (65accabdb)
- Item 8, second part: `text_kind_demand`'s `twinned` scanned every rule per text read; the pairs it asks about are
  found in one pass. An edit: 113.5 G. The supports rewrite (`maintained_cross`, 71 ms) already runs once, handed on
  (`maintained_early`). `kept_inputs` (0.13 s) decodes the kept inputs' old rows to patch them with the changed
  files' rows: the read of unchanged rows, left for the design of the local part's delta. (see git log)

## To do, in order

Items 1–3 (texts read once, caches refused by construction, my statics by flow) are done: see above.

4. **SQLite's I/O** (mostly done by the file map, 0.3 s left: the checkpoints' `fsync`s). The checkpoint at the cost model's close (0.14 s), the commit `fsync`s on the writer (0.12 s),
   the vacuum every process (0.11 s), the main thread's page reads (`frames::newest_by_path`, `facts_memo`).
5. **The cost model**: done. A process trains as a running one does: once 2,000 samples were written since the saved
   model and it is 10 minutes old; else the saved model is put in place (0.3 s CPU an edit). (see git log)
6. **Planning in an edit**: 0.1 s now (`plan_rule_reused`); left as it is.
7. **The joins** (2.0 s): `candidates`' binary searches per probe, `flush`'s sorts, `value`/`locate` per row.
8. **The main thread's passes** (next: the critical path, above): `bridge::facts`, strata computed more than once (`derived`, `cut`, `supports`),
   `text_kind_demand`, `kept_inputs`' decode, the engine driver's `slice_contains`.
9. **The alternating edits** after a rebuild (above): find what flips between runs.
10. **Memory**: an edit's peak is 2.2 GB, against a budget of 1 GB.
11. **A cold run's frames write waits on the writer** (found 2026-11-12, run n48s169, load 26): the cross part's
    frames are one job of 289.6 MB, above the writer queue's cap (192 MB) alone, so it waits while the memo's
    entries (158.6 MB) are still written; `RERAIL_SQL_STRICT` ends the run at that wait (`hazards::wait_in_wave`).
    Cold runs pass or abort by the writer's speed. With item 10: a job that size should not be held whole.
