# 0170: A process whose atoms fork from the store's maps its ids at the store

Status: decided (2026-11-13), with provisional ids (0160, `RERAIL_ATOMS_PROVISIONAL=1`)

## Intro

Every string the engine handles is an atom: a dense id (`Sym`) shared by the processes that open one store, and
written into its rows. Decision 0160 gives ids out without a lease: a process takes the next id from its own end,
and checks it against the store's when its atoms are written. When two processes start at once, both give out the
same ids to different strings, and one of them stores first. The other has forked.

0160's trial stopped there: the forked process dropped its writes and refused its next run. Two to three of four
concurrent cold runs failed that way. This record keeps the forked process going: it keeps its own ids in memory,
and maps them to the store's wherever a row crosses the store's boundary.

## The decision

1. **The fork.** A process forks when it finds, inside the writer's transaction (which holds SQLite's write lock),
   that the store's atoms end past where this process's stored ones did, while some of its own are pending. It says
   so once:

   ```
   rerail: atoms: another process stored atoms 1..2387 while this one held 2362 (decision 0170): mapping this
   process's ids to the store's from here
   ```

   Ids below the fork point (`fork`) mean the same in both. Past it, the process's ids and the store's differ.

2. **The map** (`content_store/atoms/map.rs`). Two directions, past the fork point only:
   - local to store, when a row is written. Each local atom is looked up in the store by its key (decision 0111)
     and verified by its string. A string the store does not have is appended to the store, in segments of at most
     64 MB, at the store's end. The ids of segments appended but not yet committed are held as open, and dropped
     if their transaction rolls back.
   - store to local, when a row is read. A store id this process does not know is read (just that string, from the
     segment that holds it) and interned locally.
   The lookups happen in the writer's transaction or when rows are read, never in the waves.

3. **What is mapped.** A write job says what its rows hold (`write_back::Syms`):
   - `Mapped`: rows with symbol columns, mapped inside the job. Frames (`b's'` columns, typed from the program's
     declarations: the kept strata, the retained part, and the kept cross-file part's inputs, which were untyped
     before and are read back only when their types match), the memo's entries (their symbol columns, known from
     the program's declarations, `memo::register_syms`), and the kept statistics by constant (`stat_blob`: the
     exact maps of symbol columns, `Keeper::map_symbols`).
   - `None`: no atoms in the rows (tapes, plans, programs, automata, statistics of files, pins, the pending atoms
     themselves, the planning choices, whose facts keep their own string table).
   - `Unmapped` (the default): rows that may hold a `Sym` in an untyped column. A forked process drops them, as a
     cache's misses (`DIVERGED_TEXT`). No job is left there today; a new one holding atoms types its columns or is
     dropped when forked, never written with the wrong ids.
   - Written in their own transactions, and mapped there: the derivation tables (`name_bindings`, `derivations`,
     `derivation_inputs`), the cross-file memo's rows (`put_cross_rows`, by the program's `sym` columns), and the
     prefilter's term ids (`Counts::flush`, `put_prefilters_bulk`, `candidate_transaction`, inside `atoms::mapped`).
   - The `atoms` virtual table answers in the store's ids once forked (`Cursor::stored`): the tables it joins hold
     the store's. So the prefilter's gates and the vocabulary run in a forked process.
   - Guard: `every_stored_column_is_classified` lists every integer, untyped and blob column of the store's schema,
     each either an atom column (`ATOM_COLUMNS`, with how it is mapped) or not one (`NOT_ATOMS`, with why). A new
     column fails it until it is classified.

4. **Reads.** A row read from the store carries the store's atoms (`Data::Encoded { store }`), and is mapped to
   local ids when decoded. Snapshot rows (this process's own) are not mapped. The atoms a read row names are held
   as it is admitted, before the waves (`hold_symbols`: a frame's `s` columns, and a memo frame's entry rows through
   the reader the memo registers for its space, `register_frame_symbols`), so a wave decodes it without SQL. A
   process that gives ids out without the lease holds them too: a row another process stored past its end shows
   the fork as it is read, not in a wave.

5. **Past `READ_WHOLE`.** A run over a store whose atoms are too many to read whole, without the lease, maps from
   its end (`map_from_end`) as a fork does: its waves give out ids from its end, and the writer finds the store's
   by key. One string keeps one id within the run because every id its read rows name is held at admission.

6. **Batched.** Every id lookup and translation is one statement per batch: the segments a batch names read once
   (`map::fetch`, `WHERE first IN rarray`), strings found by key in one `rarray` lookup (`find`), the derivation
   tables' names in one lookup (`Atoms::lookup_all`) and one `json_each` read, key rows written by
   `bounded_insert`. A job maps all its rows in one call (`map_entries`, `Keeper::map_symbols`).

7. **The writer** retries a batch that finds the store busy, with backoff, until the job's deadline. Mapping holds
   the write lock longer than a plain write; a writer that failed `busy` lost the run's results.

## Measured

Four strict runs at once over four parts of posthog (hogql, models, api, temporal), one fresh store, profiling
build, load 80:

| | 0160's trial (twice) | this |
|---|---|---|
| runs that ended with their solo findings | 1 of 4, 3 of 4 | 4 of 4 (three forked) |
| wall time of the four | 17.2 s, 19.4 s (failed runs end early) | 27.1 s |

Findings: hogql 1,629, models 494, api 1,057, temporal 1,063, as alone. The store after: 1,461,017 atoms, each
string once, no gaps in the ids.

A second pass over a store two forked runs (models, temporal) wrote: both read the mapped frames and memo, and end
with the same findings, in 9.4 s against 13.4 s cold.

The first version read each candidate's whole 64 MB segment for every string it looked up, holding the write lock
for seconds while the other processes' writers waited at `BEGIN`. A lookup now reads each segment once per batch
and slices it.

## Tradeoffs

| | 0160 trial: drop and refuse | map at the store (this) | one id from the string (64-bit hash) |
|---|---|---|---|
| concurrent cold runs | most fail | all finish | all finish |
| cost alone | none | none (no fork, no map) | 8-byte ids, no dense arrays |
| cost when forked | a failed run | a lookup per symbol of rows crossing the store | none |
| invariant to keep | none | every stored `Sym` is in a typed column, or its job is `Unmapped` | none |

## Measured, the second step

Same four parts, profiling build, load 40 to 85, the shape above:

| | runs with their solo findings | wall of the four |
|---|---|---|
| cold, a fresh store each (26 trials, the final build but the export measure below) | 104 of 104 | 15 to 41 s |
| cold, the final build (6 trials) | 24 of 24 | 14 to 22 s |
| cold, then three warm passes over it (86 MB of atoms, past `READ_WHOLE`) | 16 of 16 | 26.0, 15.1, 4.2, 4.2 s |
| the final build: two warm passes over a cold trial's store | 7 of 8 | 10.5, 1.9 s |
| two parts at once (models, temporal), cold | 2 of 2 | 12.0 s |

The store after: 1,461,032 atoms, each string once, no gaps.

What the trials found on the way, each fixed:

- A wave mapped a memo entry's ids and read the store (strict abort): the admission hold saw a frame's `s` columns
  only, and a memo entry types its symbols in its own rows. The memo now registers a reader for its space.
- A warm run read another process's local ids from the kept cross-file part's inputs (`no symbol 1461127`): they
  were written untyped. Typed now, and an untyped one stored before is derived again.
- Before a process giving ids out without the lease held what its read rows name, a row another process stored
  past its end was read as this process's ids.

## The third step: the failures reproduced

The open failures of the second step were each seen once or a few times in about 150 trials. A hook makes a fork
happen where a test chooses: `RERAIL_TEST_FORK_AFTER=k1,k2,…` has the `sqlite3` shell (another process) append 20
strings at the store's end once this process has given out `k` new ids. A cold run with the hook, then warm runs over
its store (copied, so it replays), failed the same way every time for some `k`:

| fork after | what the warm runs did | cause |
|---|---|---|
| 100 ids | `phase 0: no statistics for p0_lib.c0` | the kept strata wrote symbols in `i` columns |
| 1000 ids | `// ensure_import(…)…: cannot read the file` (a path read as a comment's text) | a batch that forked part way |

Each was found by emptying one table, then one fact space, of the copied store and replaying, then decoding the
frames of the one space that mattered with their symbols as text: a frame of the retained part held the hook's own
strings (`rerail-test-fork-…`) where paths belonged.

1. **Symbols in integer columns.** Over atoms the engine holds a symbol column as an integer (`sym_char`, O13), and
   the kept strata, the cross-file part's relations and the retained part's derived relations were kept under the
   engine's column types. The store maps only `s` columns, so a forked process wrote its own ids. Each frame is now
   typed by its declaration (`stored_types`, `retain::frame_types`), and read back under the engine's
   (`retain::engine_types`).
2. **Rows keyed by symbols.** The retained part's documents and granules are keyed by an input's owner, which is an
   atom where the owner is a text (`p0_resolved`'s path). The store can map a value, not a part number or a document
   key, so after a fork that part's frames are not written (`queue_frames_keyed_by_symbols`, `Syms::Unmapped`); the
   next run derives them again.
3. **A batch that forked part way.** The writer's batch checks for a fork as it begins (`begin_write`), but only when
   ids are pending. Found with none pending, the store had moved past this process's end: the ids given out during the
   batch were the other process's, a later job in the batch forked, and the batch's commit (which runs mapped)
   committed the earlier jobs' rows of those ids. Now `begin_write` moves this process's end past the other's atoms
   when none is pending, under the write lock, so no id given out during the batch is theirs; and a batch that forks
   after it began rolls back and runs again, forked from its start (its jobs' rows mapped or the jobs dropped).

Then, under four concurrent real runs over one store (no hook), two more:

4. A forked process's memo read refused an entry whose relation had no symbol columns registered, and the run
   failed (`this process's atoms forked …`). That entry is a miss now.
5. The corpus collection pass (deletes and flags, no atom) was refused after a fork; it runs mapped now.

And at open: opening a thread's connection configures it with reads (`page_count`, `journal_mode`) that wait for
another process's writer for 250 ms only. Past that `lock()` failed, and every caller `expect`s its connection
(`content store mutex poisoned: "content store writer is busy"`). The open is tried again now, as a write is
(`retry_busy`).

Measured after, profiling build: the hook at 1, 30, 100, 300, 1000, 3000 and 30000 ids (models), and at 100 and
5000 (hogql, api), each a cold run and two warm runs: every run gives its solo findings. Four concurrent real runs
over one store, cold then warm: on the last build, 2 trials, 15 of 16 runs give their solo findings and one warm run
hung (below); every run that finished, on any build of this step, gave its solo findings.

## The fourth step: the open points closed

1. **The hung warm run.** Sampled at 45 s: the main thread waited in `ContentStore::written` for queued jobs while
   the writer sat idle in its queue's `recv`. A batch rolled back (a fork part way, item 3 above, or a busy commit)
   runs again; its first run may have taken the next unbatched job (a wait for the writer, `commit_now`) off the
   queue, and the second run took another and overwrote it. The lost job's count never left the queue, so `written`
   waited for ever. A run already holding the next job ends there now (`write_back.rs`; test
   `a_batch_run_again_after_a_fork_keeps_every_wait_queued_meanwhile`, which timed out before).
2. **The phase 0 test in the template's process.** `bootstrap.rr` names `core.rr` only for file seeds, and
   `p0_absent` has no includer: the template's run, which seeds every library module, gave KEPT graphs that differ
   from one text's own derive. A seed's graph is now its own, whatever run derived it (`phase0.rs` `Graph::of`;
   test `a_seeds_graph_is_its_own_whatever_run_derived_it`).
3. **A granule the store no longer holds.** The cone's rounds read the last run's inputs granule by granule
   (`read_input`), and a missing one failed the run (`phase 0: done_p0_file_t_shape_b: granule 9 of its stored rows
   is missing`). It is a miss now: the rounds give up and the gated fixpoint derives the part again. No read is added;
   K4's admission already took a missing granule as a miss (test `an_inputs_missing_granule_is_a_miss_not_a_failure`).

Measured after, profiling build: 10 trials of four concurrent strict runs over one store (hogql, models, api,
temporal), cold then warm: all 80 runs give their solo findings (1629, 494, 1057, 1063), none hangs (before: a hang in
about every other warm trial). The replay that failed on a deleted granule gives 494.

## Departures and open points

- The memo's foreign id (`the memo's entries: symbol 2767 is no atom of this process's (it holds 2760)`), seen once,
  in a concurrent trial before the third step: a forked process's entry held, in a symbol column, a value past its
  own atoms, so not one of its ids (likely a store id carried in from a read). Not seen since, in the hook sweeps
  and the 80 concurrent runs above. The mapping takes any value past the fork point as the process's own: a store id
  below the process's atom count would be mapped to the wrong string without an error. Not reproduced; open.
- A forked process's memo misses every entry whose relation has no registered symbol columns: a cost of the fork,
  not of correctness.
- The kept statistics' sketches (estimated distinct counts) and the retained part's statistics by constant
  (`kept_stats`, `Patched { counts, tested }`) are not mapped: a forked process's estimates for a symbol constant
  may be another string's. Estimates only; the rows are mapped. Digests taken over a forked process's rows are of
  its numbering: a later process sees them differ and derives again.
- `fact_value_stats` (the facts database's value statistics, `val ANY`) holds slice scope text and clause numbers,
  no atoms, and is written on its own connection.
- Still behind `RERAIL_ATOMS_PROVISIONAL=1` (0160).

## Prior work consulted

- 0049 (the lease), 0104 (no SQL inside the waves), 0111 (atoms found by key), 0146 (the memo), 0160 (provisional
  ids and the translation it called for).
- ast-index-spike indexes: interned ids were process-local, with no shared store to map against.
