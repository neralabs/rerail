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
  6 G and 0.6 s CPU an edit. (uncommitted)

## To do, in order

1. **The program texts, read once.** `settings::program_texts` loads each text; the whole-run cutoff
   (`texts_digest` → `graph_digest` → `text_seed`), phase 0's graphs, the rule files' record
   (`reuse::rule_files` → `phase_files` → `declared_names`, `included_files`) and `combine` (`read_text`) each
   parse it again. One read gives the statements, the include seed, the declared names and the included files;
   they are handed on, not looked up. No memo behind the callers.
2. **Caches, refused by construction.** `logic::facts_only` (decision 0104) lists every store table and every
   process-wide or thread-local map; it has been red since several merges and no run checked it. Make it green
   (each unlisted entry moved to facts, removed by a change of flow, or listed as state with its reason), run it
   with every targeted test, and check each commit's added lines for a new static map or table.
3. **My statics of 2026-11-12, by flow.** `frames::known_tables`/`lazy_table` (the frames' tables made with the
   store's schema, so nothing checks for them), `frames::CONFIRMS` (the confirmations held in the frames'
   snapshot that `preload_frames_for` builds, not a map beside it), `corpus::NESTED_PLANS` (the nested runs' plans
   carried by the run, not a static), the tables `frame_confirmed` and `frame_used` (rows of `frames` or listed as
   fact store, with their reason).
4. **SQLite's I/O.** The checkpoint at the cost model's close (0.14 s), the commit `fsync`s on the writer (0.12 s),
   the vacuum every process (0.11 s), the main thread's page reads (`frames::newest_by_path`, `facts_memo`).
5. **The cost model**: train when there are new samples worth a model, not every process (0.3 s CPU).
6. **Planning in an edit**: why `plan_search` runs (0.3 s on the workers) when the plans are kept (0116 step 2).
7. **The joins** (2.0 s): `candidates`' binary searches per probe, `flush`'s sorts, `value`/`locate` per row.
8. **The main thread's passes**: `bridge::facts`, strata computed more than once (`derived`, `cut`, `supports`),
   `text_kind_demand`, `kept_inputs`' decode, the engine driver's `slice_contains`.
9. **The alternating edits** after a rebuild (above): find what flips between runs.
10. **Memory**: an edit's peak is 2.2 GB, against a budget of 1 GB.
