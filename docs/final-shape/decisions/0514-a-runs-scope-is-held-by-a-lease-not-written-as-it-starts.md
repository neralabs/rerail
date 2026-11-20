# 0514. A run's analysis scope is held by a lease, not written as it starts

Status: accepted (2026-11-20, the user: "do this … Deletions that are concurrent with runs should be handled by
making the delete wait, or making it fail and need to be retried after"). Follows 0104 (one read before the first
wave, writes behind the run) and 0306 (a run's small writes are its record's items). Changes the scope protocol of
`docs/final-shape/notes/git-inputs/analysis-scopes.md` (73689678b and the commits before it). Numbered 0494 on
benchmarks-on-integration; renumbered at its merge into CUDA-support (2026-11-20), as four other worktrees have their own 0494.

## Intro

A scan that keeps its results in a content store belongs to an **analysis scope**: a named scope
(`--analysis-scope review`) or the directory it scanned. Other processes act on scopes while scans run. `rerail
scope delete` removes a named scope; a home that stops watching a directory sends a **stop**, a release of the
directory's scopes up to a control generation (its cutoff).

Until now a scan announced itself by writing, as it began, three things to the store: the scope's generation (a
number a deletion changes), an acquisition number from the store's counter, and a *pending* row that a deletion
could see. A deletion that came while the scan ran changed the generation, and the scan's publication then found
itself revoked ("released before publication").

Those writes came before the run's first wave, from the run's own thread. Decision 0104 allows only reads there,
and the strict mode (`RERAIL_SQL_STRICT=1`) ends any process that writes there. Since the CLI batch path started a
scope for every `rerail-engine logic` run (73689678b, through the sync branch), every strict run ended at its
start.

Marking those writes as the store writer's would have hidden the violation, not removed it. Queueing them on the
writer and waiting for them would have been the same write with the wait on another thread. So the protocol
changes: a scan's start only reads, and what other processes must see of it is a lease, a locked file beside the
store.

## Decision

1. **A run's start only reads.** `AnalysisRun::begin_at` reads the store's acquisition counter, whether the scope
   has a generation, and which generation the run will publish under. It writes nothing to the store.

2. **The lease is what other processes see.** Each run holds a file in `<store>.scopes/`, named by its
   acquisition and locked while the run lives (`content_store::scope_lease`). It records the scope, the control
   generation, and the generation the run's first write creates if the scope had none. The run removes it as it
   finishes; a lease whose lock is free (its process died) is removed by whoever finds it. A lease is written under
   a temporary name, locked, then linked to its number, so it is never seen unlocked while its run lives.

3. **The acquisition number comes from the leases.** It is above the store's counter and above every live lease
   (the link fails if another run took the number; the next one is tried). The store's own allocations
   (`next_acquisition`) are above the live leases too.

4. **A run's writes acquire first.** Its first write, the manifest the store writer applies behind the run, writes
   the acquisition: the scope's generation if it had none, the directory's highest control generation, and the
   counter raised to the acquisition (`AnalysisOwner::acquire`, idempotent; every later write repeats it).

5. **A new generation is known at start.** The store seeds a new generation from its counter by a trigger, which a
   read cannot predict. A run that creates one creates its own acquisition as the generation instead: a number of
   the same sequence, above every earlier generation (a deleted scope's raises the counter), so a writer of a
   deleted scope still cannot publish into its reuse. Two runs that start a new scope at once create the same
   generation, the first one's: the second sees the first's lease.

6. **The publication is the writer's.** `finish` waits for the writer to apply the run's writes, decides the
   outcome by a read (its lease holds off every deletion that would revoke it), and queues the publication as an
   item the writer applies after the manifest. It waits for that too, since the outcome is the caller's answer. A
   failed publication leaves the run and its lease open, to be finished again.

7. **A deletion that would revoke a running scan fails, to be retried after it.** `delete_analysis_scope`,
   `delete_directory_analysis_scopes`, a delivered stop (`DirectoryRelease::deliver`) and a named release
   (`release_named_scope`) fail with `StoreError::InUse` ("analysis scope in use: … retry once it has finished")
   while a live lease is one they would revoke. A stop revokes only the runs whose control generation is at most its
   cutoff; a later writer runs on. A replayed stop (its receipt delivered already) releases nothing and is not
   refused. The home's release journal keeps a failed delivery and delivers it again later
   (`src/engine/native/analysis-scope.ts`, `deliverRelease`).

8. **Starts and deletions take turns.** A start and a deletion each hold `<store>.scopes/.gate` while they run.
   So a start comes wholly before a deletion, which then sees its lease, or wholly after it, which then reads the
   generation the deletion left. Two starts see each other's leases. A deletion writes the acquisitions of the live
   runs it leaves before it releases, as their own first writes would, so a stop keeps a later writer's
   generation.

9. **The pending row is retired.** A run writes no `analysis_scope_pending` row; its lease is its pending state
   (`named_acquisition_state`, `legacy_named_route_state`). A finished run's staging key closes with its lease: an
   owner whose run has finished is no longer `current`. Collection keeps removing older writers' pending rows.

## What is not changed

- The workspace API's owner (`begin_analysis_scope_at`, `workspace/api.rs`) still acquires as it begins, with no
  lease. It is not on a strict path (the Node workspace attach). A deletion still revokes it through the
  generation, as before. Moving it onto a lease is an open item (work queue, Q203).
- The rows a publication writes, and what collection keeps, are as before.

## Tests

- `a_runs_start_commits_nothing_to_the_store`: no commit between before and after `begin` (`PRAGMA data_version`
  on another connection); the lease is live, then gone after `finish`.
- `a_deletion_of_a_running_scope_fails_until_the_run_has_finished`: `InUse` while the run holds the scope; the
  retried deletion removes its publication; a stop below a later writer's control generation leaves it, and a run
  that starts below a delivered stop's cutoff is released.
- `runs_that_start_a_new_scope_together_share_its_generation`: both publish; the generation is the first's
  acquisition.
- `a_dead_runs_lease_neither_blocks_a_deletion_nor_survives_it`.
- Ported to the new rule: `named_scope_release_requires_the_registered_store_and_fails_while_a_run_is_unfinished`
  (was `…_and_revokes_an_unfinished_run`), `completed_analysis_runs_survive_restart_…`,
  `a_numbered_parent_stop_compacts_only_fully_covered_directory_generations`,
  `a_numbered_home_stop_revokes_older_unqualified_directory_writers_once` (a stop fails until the writers it
  revokes have finished, and is then delivered), `a_publication_error_rolls_back_…` (no pending row; the lease
  survives the failure), and the TypeScript `refuses a release between evaluation and publication, and the retried
  release removes the published run` (was `reports a release between evaluation and publication …`).
- A strict `rerail-engine logic` run, twice on one store: both publish (acquisitions 1 and 2), nothing written
  before the waves.
