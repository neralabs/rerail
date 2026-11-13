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

## Smoking guns (2026-11-13, 300 files, profiling build)

A search for the large, structural wastes, not a benchmark. Fixed (see git log):

1. **A cover's cross product.** A cover of two asked variables was a product of everything that binds either one. It
   is now narrowed by the group that binds them. On 50 files: 716 → 463 G instructions, 5.67 → 2.34 GB.
2. **The planner's zero rows.** A relation that was empty when it was planned (`rel_base` before its guest round,
   `impmod`, `ruff_qual`, `frag_str`) priced every later step at 0. That made cross products look free
   (10,728 → 25.8 M → 10,728 rows). Estimates are now floored at one row; a memo's exact count is not.
   A cold run's local part: 1,079 → 921 G.
3. **Kept inputs refused.** `group_col` grouped `p197_arg_at` by column 2, the memo by column 0. Every edit then
   failed with "memo entry is gone". It now groups by the split's locations.
4. **A fallback that read the store inside the waves.** The kept route is decided before the first wave
   (decision 0104).
5. **The memory budget capped the store.** A relation past `RERAIL_CROSS_RETAIN_MB` was not written to the frames,
   so every next process derived it again: an edit ran 3,758 of 4,087 strata, 1,546 of them derived again. It is now
   written, then let go: 0 derived again.
6. **`@place` in a head had no native form.** The 0117 walk's maintained form has no SQL fallback.
   `violations_at_places…` had been red in every worktree since 11-12.
7. **A flaky test.** In `closures_of_one_level…`, the three closures' wave ran on a stratum job's thread and was
   counted there. The tally now goes with the run's ambient.

Open, largest first (the edit after items 1–5: 12.85 s of span CPU):

8. **A maintained stratum copies and re-indexes its whole kept relation** (4.47 s, 43.5% of the edit, against 0.76 s
   of joins). `Rel::based` clones the kept rows and chains, and `Rel::compacted` pushes every row again with a new
   dedup index (open: survivors, 1.41 s in 210 calls). `set_delta` and `restamp` walk every index, and `want(cols)`
   builds a whole-relation index per multi-column key (open: indexes, 1.37 s). A derived input read back from the
   frames has no chains, so each probed key set is indexed over all its rows (1.17 s in 2,447 calls). Each of these
   costs the relation's size, not the delta's. The shape is the one `Layered` gives inputs:
   - the stratum's relation is the kept `InRel`, shared through an `Arc`, with its dead rows marked;
   - its new rows go in a vector of its own (ids from `old.n` on);
   - the kept chains and indexes serve the old rows, and only the added rows are indexed.

   *The fix, located (2026-11-13, after the hub merge):* `Rel::based` (`native/setfix.rs`) clones `old.rows`,
   `old.dead`, `old.chains`, `old.serves`, `old.sketches` and `old.level`; `Rel::compacted` pushes every survivor
   through `Rel::push` (a dedup probe and an index insert each). Sharing needs the rows segmented in both forms:
   - `Rel.data` becomes a kept segment (`Arc<InRel>`, read in place, its dead rows marked in a bitmap of its own)
     and a tail of the rows this run pushed (ids from `old.n`); `close` makes an `InRel` of the same two segments,
     so the next run shares them again, never concatenating (a copy at close is the same cost moved).
   - `Index::find` and the probes take a row reader, not one `&[i64]`; the kept chains serve ids `..old.n`, and
     the run's runs only its tail, so `restamp` and `set_delta` touch the tail's runs alone.
   - `want(cols)` on a kept segment builds its index once per kept relation (kept with it, as the chains are),
     not per run; a relation read back from the frames gets its chains when admitted.
   - Compaction, when dead rows pass the eighth, is a background rewrite of the kept segment, not a start's cost.
   About 36 row reads in `setfix.rs` and 28 elsewhere in `native/`, and the device uploads of a relation's rows,
   which take each segment as a buffer.
9. **Statistics of `__old` relations are counted from their rows** ("count run_scope__old from its rows", 0.05–0.07 s
   each, dozens per edit), although decision 0120 keeps statistics in the frames under the rows' digest.
   *Fixed:* `Retained::admit`, which reads a reached relation's rows from the frames mid-run, read its rows and
   levels but not its statistics part. It now reads that part in the same batch (when stored for the same digest)
   and keeps it under the rows' address in this process (`late_stats`). `measured`, `patched`, `measured_all` and
   the frames' write-back read it as they read the statistics admitted before the run.
10. **`frag_str`'s `doc_role(d, 17505)` is planned from statistics made before its rows exist:** estimated at 1 row,
    2,455 found, 7.1 M rows, none kept. The floor (item 2) bounds this but does not fix it. The plan should be made
    when the relation is known.
    *Fixed:* the planning budget (`budget_takes`, decision 0122) took a kept order while the relations it reads
    grew by less than a search's work, by their rows alone. A literal with a constant reads that constant's rows:
    their counts (already in `live_sizes`) are now part of the growth, unknown counts left out. One row for 7 when
    planned and 2,455 now is 2,455 times growth: the line is searched again.
    *Still open in a cold run* (2026-11-13, after the hub merge, `cold-merge`): with no kept plan, the line is planned
    before the guest round makes `doc_role`'s rows, and runs `doc_role(d, 17505)` (2,455) → `kind($s, 1255)`
    unbound (7.1 M) → `$d = @doc($s)` (0): 0.80 s of the cold run. `$s` reaches `doc_role` only through `@doc`,
    so the plan's first step binds nothing the second reads. The fix: a step that binds none of the next one's
    columns is priced at the next one's whole size times its own rows (a cross product), not at its estimate,
    or the stratum is planned when its relation's rows are known (decision 0123's round, not the seed's).
11. **Smaller:** `p149_LOG001` scans `import_alias` with nothing bound (3.9 M rows → 0). `rr_agg48` goes from 135 k
    to 3.1 M rows. There are 1.5 M scalar node visits.
    *Located* (2026-11-13, a profiled cold run after the hub merge, 33.2 s wall, 77.6 s CPU, 615 findings):
    - `p149_LOG001`'s scan is gone: its largest `import_alias` scan now reads 16 k rows.
    - `rr_agg48` (1.15 s) is `body_stmt`'s index in `lib/python/ast.rr`,
      `i = count : { child(b, s2, _), !kind(s2, "comment"), s2 < s }`: for each of 125,910 statements, every
      earlier sibling (2.8 M pairs, two `node_order` lookups each). The count of a group's members before one
      is its rank: sorting the group once gives every statement's in one pass. Wanted: the lowering of a count
      whose only test against the outer row is `s2 < s` over the same group as a rank (a decision), not a rule
      rewrite.
    - The scalar visits are `BROAD_SCALAR_PROJECTION` (2.5 s CPU): 1,588,373 nodes visited and 984,956 strings
      interned to project `text`, `str`, `op`, `value` and `comment` for every node, where their consumers
      (`q5_noqa`, `case_value_name`, `py_import`, …) read a few kinds. The consumers' kind restrictions are to
      reach the projection (`Projection::of`) as demands, as `text_kind_demand` does for texts.

Tests, one a gun (each fixed gun's test was checked red with its fix undone; the open ones are red, `#[ignore]`d
with the gun's reason until fixed):

| Gun | Test | State |
|---|---|---|
| 1 | `rr::tests::a_cover_of_two_variables_is_narrowed_by_what_binds_them` | green |
| 2 | `plan::tests::a_relation_empty_when_planned_does_not_make_a_cross_product_free` | green |
| 3 | `retain::frame_tests::kept_rows_are_grouped_by_the_column_the_memo_attributes_them_by` | green |
| 4 | `retain::frame_tests::the_kept_inputs_are_known_before_the_waves_with_why_not` | green |
| 5 | `retain::frame_tests::a_relation_past_the_retain_budget_is_written_to_the_frames_and_let_go` | green |
| 6 | `strings::functions::tests::a_place_is_native_and_encoded_as_sql_encodes_it`, `plan::tests::violations_at_places_are_found_at_their_path_and_line` | green |
| 7 | `lanes::wave_tests::a_jobs_waves_are_credited_to_the_run_that_takes_it_back`, `plan::tests::closures_of_one_level_launch_in_one_wave` (the hub's fix, 4dde583e2, kept at the merge) | green |
| 8 | `setfix::survivor_tests::a_maintained_relation_starts_from_its_kept_rows_without_copying_them` | red, ignored |
| 9 | `retain::frame_tests::rows_read_where_reached_come_with_their_kept_statistics` | green (fixed at the hub merge) |
| 10 | `plan::tests::a_constant_counted_none_when_planned_is_growth_when_it_has_rows` | green (fixed at the hub merge) |
| 11 | none yet: located above (one gone; a rank lowering and the projection's demands wanted) | — |

After the hub merge (2026-11-13, profiling build, corpus0, B-now rules, load about 6): cold 615 findings, an
append 616, the rule edit 616, a second append 618. The second append, the steady state: **6.00 s of span CPU**
(12.85 s before guns 1–7, 9 and 10), 6.1 s wall, 9.7 s process CPU; six `__old` relations still counted from
their rows (those maintained for the first time in that run, by design). Gun 8 is now the largest: `open:
indexes` 0.92 s, `open: survivors` 0.83 s, `open: index of a derived input` 0.54 s, 38% of the edit.

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
10. **Memory**: an edit's peak is 2.1 GB (footprint), against a budget of 1 GB. Measured (2026-11-12, `RERAIL_PROFILE`,
    whose peak matches an unprofiled edit's): the peak is set by the **programs**, not the rows. Live at the peak, of
    1.72 GB of jemalloc heap: the prepared program's forms 435 MB (front and lowered 198 MB, the split's local and
    cross programs 120 MB, the rest), the join specs kept across chunks (`spec`, 8,808 of them) 366 MB, the kept
    inputs' rows 263 MB, the joins' indexes (`open: joins`) 159 MB. Transient rises: reading the prepared program
    off the store +1.0 GB (the encoded parts, their views and the programs at once), the combined program +348 MB.
    Done: the combined program is dropped once the prepared one is found (it was held through the run): peak 2081–
    2103 MB (was 2178–2180 MB). (f9f5cc76c) The join specs' memo (`logic::derived`, a cache not yet facts) is not
    what holds the specs: an edit with it off costs 117.1 G and peaks at 2074 MB, with it on 115.5–119.1 G and
    2043–2089 MB; the 366 MB are the specs the run holds. Next: one form of the program where four are held (front,
    lowered, the split's local and cross); the specs' size.
11. **A cold run's frames write waits on the writer** (found 2026-11-12, run n48s169, load 26): the cross part's
    frames are one job of 289.6 MB, above the writer queue's cap (192 MB) alone, so it waits while the memo's
    entries (158.6 MB) are still written; `RERAIL_SQL_STRICT` ends the run at that wait (`hazards::wait_in_wave`).
    Cold runs pass or abort by the writer's speed: 2 of the last 3 cold runs aborted (loads 14 and 26). The writer
    writes the memo's entries at about 35 MB/s of its CPU (1.7 s for the first 2 of 7 jobs). Splitting the frames job
    alone would not help (all of it is queued at once, when the cross part ends): the direction is each stratum's
    frame queued as the stratum finishes, so the writer works while the cross part runs, and a cheaper memo entry.
    Done (decision 0138): the writer's two misplaced costs moved. Marking a memo entry used rewrote its rows (a
    `WITHOUT ROWID` record with its rows in it): 2.9 s of a cold run's writer, now a row of `frame_part_used`. The
    WAL's checkpoints ran in the writer's commits (3.0 s): now beside it, `PASSIVE`, on a thread of its own. A cold
    run after a rebuild passed under the default cap with the queue never full (load 13). (see git log)
