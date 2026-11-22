# rayon-core 1.13.0, patched

crates.io's rayon-core 1.13.0, with one addition: a job carries what its maker's thread holds to the thread that
runs it (`src/carrier.rs`, used in `src/job.rs`).

Why: decision 0442 keeps a run's state in the run, on the thread that runs it. A parallel loop splits by `join`,
and a half another pool thread steals ran there with nothing of its caller's: no run, no bindings. The engine
worked around it by keeping the runs inside a pool (`rerail_context::Inside`), which a stolen half could read only
when every job in the pool was one run's; with two runs' loops in one pool at once (a corpus run's phase 0 beside
its main program) a stolen half could not tell which it was of (work queue Q300).

What changed:
- `carrier::set_job_carrier(capture, enter)` installs, once for the process, a function that captures what a job
  made on this thread carries, and one that enters it on another thread until its guard drops.
- `StackJob` (a `join` half, an `install`ed closure) and `HeapJob` (a `spawn`ed or scoped job) capture when made
  and enter when executed. A `StackJob` its maker runs inline (a `join` half popped back) enters nothing. A
  `StackJob`'s guard is dropped before its latch is set, so the maker's next step never sees the job still in its
  state. `ArcJob` (broadcasts) carries nothing: the engine broadcasts nothing.
- Without a carrier installed, nothing is captured, and jobs are upstream's.

The engine installs its carrier in `rerail-context` (`job_run`, `enter`) when it makes a machine's pool.
