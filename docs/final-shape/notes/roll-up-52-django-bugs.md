# Roll-up 52: django-bugs (87dbbe9135) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 15a575d133 (roll-up 51, Q472, and the switches fix). Theirs: django-bugs at
87dbbe9135, pinned in `tmp/rollup/pins4.txt`. The merge base is e7697f6e8e, the hub theirs merged in b08dd573f3.
Theirs brings 27 commits: decision 0540's steps 3-7 (a level's formulas one launch, a work queue with continuations;
an idle worker ends at once; block programs' look-back removed, the forward carry the device's only executor; a
rerun charged to a budget of its own), 0442 for a stratum's jobs on the pool, and 0601 (every launch counts in one
ledger, `rerail_accel::gpu::launch`, and past it is a hard failure).

## Items

1. **`logic::device::Device`: both sides' removals.** Ours' crate split 5i moved `column_stats` and `StatColumn` out
   of the trait. Theirs removed `install_launch_guard` (0601: no process-wide guard). Neither is kept here.
2. **`device_metal.rs`.** Ours moved the `Device` impl up the file. Its `install_launch_guard` goes, by item 1. The
   test loader had two `run_state::CARRIERS.register` calls, both sides' same line in two places; one stays, and the
   doc comment is theirs'.
3. **`lanes.rs`: theirs.** The executor's GPU lane charges each launch where it is made (`GpuBackend::run`).
4. **`metal_join.rs`: both.** Theirs' `commit().map_err(Error::from)`, with ours' `InFlight` records of the indexes
   the batch builds.
5. **`accel::gpu::join`'s `watch`: both.** Ours' `pub`, theirs' `key: None`.
6. **`rerail-metal`'s kernels ahead: theirs.** The block interpreter is gone (f0f7e4cca7), so ours' `ready()` gate on
   its warm-up goes with it. Ours' `offers_simd_width` in `block_program::ready` is kept.
7. **`block_common.uglykl`'s header: both.** Theirs' forward-carry text (no poll, no look-back), with ours'
   `UKL_SIMD_WIDTH_NEEDED` and the Arc's sub-groups.
8. **`tape_f16z.h`: one code, ours' comments.** Both sides ported G15's slot mask and 32-bit match places the same
   way. Ours' comments name G15.

## The ledger's invariant on ours' side

0601 requires every submitter to charge each launch to the one ledger. Ours' side since the base, read for
submissions:

- per file, the dispatch and command-buffer sites are theirs' tip's;
- ours' one new commit call, `join::Pass::commit_wait`, goes through `Pass::commit`, which checks the evaluation;
- no handler of `BackendLaunchLimit`, or user of `set_guard`, `install_guard` or `launch_budget`, is left.

The generated `relation.h` was merged as text from both sides' regenerations. `rerail-clif`'s `*_reads_are_these`
tests confirm it is the merged emitter's output.

## Audit

`scripts/check/merge_audit.py`'s lines are items 2, 6, 7 and 8's comment and code choices above.

## Checks

- spark2, CPU: `cargo check --workspace --tests` (with and without `metal-tests`) is clean, `check_crate_dag.py`
  passes, and `rerail-clif` `reads_are_these` 3 pass.
- m3: the Metal crates and the transducer tests compile (`--features metal-tests`). Nothing was run on a GPU.
- Theirs' 0601 lists six setfix device tests that fail on the M3, each a stratum past 200 launches. That is 0601's
  "work this sets" (a stratum's launches independent of its rules), which this merge does not change.
