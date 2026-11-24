# 0303. The process's end waits for an upkeep step under way, and a mostly free store is vacuumed whole

Status: built (audited 2026-11-23). Decided (2026-11-16). Builds on 0190 (upkeep behind the runs), 0227 (collection by
reachability), 0053 (corpus runs' bounded history), 0302 (a policy set's newer lane supersedes its older). Q19 in the
work queue.

## Intro

Upkeep is the work done behind the runs: sweeping frames nothing holds, giving free pages back to the disk,
analyzing tables. A scheduler thread steps the jobs when the machine allows. At the process's end, `finish_soon` runs
what is left of the utility jobs, such as the frames sweep, so a CLI process does not exit with its own garbage
behind it.

Two things went wrong on ep2 (a 1,208-file corpus, one rule body edit per run):

- The frames sweep never finished. For 31 runs in a row its work was rolled back, and swept frames stayed between
  289 and 322.
- Once the sweep did run, each edit cost 101–119 G instructions, against 53–58 G before. The extra time was spent
  behind the run, in the collector's vacuum.

## Decision

### 1. `finish_soon` waits for a step the scheduler thread is in

While a job steps, its entry is out of the job list (`step_job` takes it out, so the lock is not held during the
step). `finish_soon` walked the list, did not see the job, and returned. The process then exited under the step, and
the sweep's transaction was rolled back. A small sweep finished in the gap. ep2's backlog took 10–34 s and never did.

Now every step runs under one lock (`STEPPING`). `finish_soon` takes that lock before its passes, so it waits for a
step under way, and it holds the lock through its passes, so the scheduler cannot take out another job behind it.

A progress handler stops a sweep when a run begins, and that was the other suspect. It does stop the sweep, and the
sweep is still due afterwards (`a_run_that_begins_stops_a_sweep_of_many_small_deletes`), so it was not the cause.

### 2. A mostly free store is vacuumed whole, by the upkeep job

After a pass, the collector gave back 4,096 free pages with `PRAGMA incremental_vacuum`. That cost grows with the
free list, because each page it moves walks the list's trunk pages. ep2's store was 91% free after the backlog
(1,818,329 of 1,993,536 pages), and one such step took 8.1 s. A whole `VACUUM` of the same file took 10 s and left
704 MB of an 8.2 GB file.

One policy, `give_back`, now decides for both places. A file past `VACUUM_SHARE` (20%) free is vacuumed whole, in
either vacuum mode, when its live pages are within `VACUUM_BYTES` and the disk has room for their copy. The cap is
on the live pages, not the file: a `VACUUM`'s cost is the live pages'. Below that share, an incremental file gives
pages back incrementally as before. The collector's own vacuum leaves such a file to the upkeep job
(`leaves_to_whole_vacuum`). The whole vacuum stays idle-machine work, since it holds the file's writers for its
length.

## Consequences

- Tests:
  - `upkeep::tests::the_process_end_waits_for_a_step_under_way` (accel) fails without §1.
  - `upkeep::tests::a_file_mostly_free_is_vacuumed_whole_in_either_mode` and
    `corpus_runs::tests::a_store_mostly_free_is_left_to_the_whole_vacuum` fail without §2.
  - `free_pages_go_back_and_own_tables_are_analyzed` now expects a whole vacuum for its three-quarters-free file in
    both modes.
- On a busy machine the whole vacuum may wait, and the file then stays large. The runs no longer pay for it.
- ep2: see the work queue's Q19 entry for the measured runs.
