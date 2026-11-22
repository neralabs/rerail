# Roll-up 57: sync-integration-hub-tape (3c058416f0) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at b3396b5435 (roll-up 56, Q477). Theirs: sync-integration-hub-tape at 3c058416f0. The
merge base is 18805b0ab2. Theirs brings three commits:
- 0563 A for Python batches: a document may embed only where the store or its text says it may (`may_embed`), so a
  run that admits no guest drops the guest rules (`preparation::unfillable`);
- a removal join's `docof`, its node bound, computed from the node's id;
- the join screen checks its scan's floor before it measures the probe side.

## Items

1. **Theirs' hunks in code ours moved.** Ours' split moved `Report` to `corpus/base.rs`, and the code around
   `unmeasured_rows`.
   - Theirs' `Report::unfillable` goes on `base.rs`' `Report`.
   - `may_embed` goes in `corpus.rs`, beside its one caller's module.
   - Theirs' `unhex` is not added again: `may_embed` reads `content_store::observe::unhex`, the same function.
2. **The index's 0563 row is theirs'**, which names A's Python batches.

## Audit

`scripts/check/merge_audit.py` and the missing-hunk check name item 1's two hunks, placed as above.

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean, and `check_crate_dag.py` passes;
- rerail-tests-logic `made_batch_tests` 6, `session::tests` 20 and `prune` 7 pass, with theirs' three new tests and
  rerail-setfix's `a_small_scan_does_not_measure_its_probe_side`.
