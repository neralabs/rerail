# Data-layer rule compatibility

Base engine: `9bc955cc5`.
This branch supplies production compatibility for the recovered data-layer candidate policies.
The candidates and recorded oracle corpus remain on `jeremybarnes/Data-layer-rule-expressivity`.
No sibling worktree changes enter this branch.

## Supported boundary

The public TypeScript schema and native compiler accept the established relational clauses.
The schema also retains their source gates, line scans, capture columns, helper closures, and report thresholds.
Modern embedded-language schema behavior remains available.

The compiled-rule API evaluates snapshots. The workspace API evaluates resident files and edits.
Legacy clients that own execution separately can use the replay bridge on the candidate branch.
Relational execution uses the compatibility evaluator; it does not lower into the wave/GPU execution model.

Two generic capabilities support real seventh-batch rules:

- Configured SQL text uses `escape: "sql"`, which produces UTF-8 hexadecimal SQL text expressions.
- `verify_ed25519` verifies detached artifact signatures. Malformed input returns false.

Relational evidence retains its declared diagnostic anchor.
Repository execution admits a complete nonempty gate before loading source.
Syntax-only consumers load their gated file domain. Explicit source-text queries retain aggregate byte limits.
Per-file failures retain the partition path for diagnosis.
Production compatibility does not recreate the retired anchor corpus beside the current matcher.

## Evidence

All 70 candidates pass this branch's public schema and native compiler.
All seven recorded source-detection batches pass on the compiled production addon.
Batch seven includes 210 positive/clean observations and 50 edit/removal transitions.
The complete 9,021-file repository run matches all 24,993 expected findings with zero engine failures.
Cold CPU is 465.234 seconds; cold wall time is 382.467 seconds; OS peak RSS is 15.323 GB.
Memory remains an open concern.

The focused frontend tests pass 32 tests. Both candidate and engine worktrees pass typechecking.
The broad native suite passes 2,515 tests, ignores 67, and fails one inherited snapshot-hash guard.
That guard is `admission_spike::interpreter_snapshot_matches_the_current_independent_oracle`.
Both source files it checks are byte-identical to `9bc955cc5`.
The base's current region hashes to `25dd37caf0584afab85c4ed54d00c5d6e34e3f456ec3de2cec916eee29cfae0f`.
Its retained experiment expects `f9b8077c59090eefd5d84c92a2323114889733610a5780d65c452ff63de27b50`.
This branch does not update that independent experiment's snapshot to hide the failure.

Restore the pinned pack fixtures before broad native tests.
Missing fixture directories can also make the widened-walk sentinel report no differences.
The restored Python, SQL, and Vue fixtures use matching `test-data.lock` files.

Build with the normal allocator:

```sh
cargo build --profile profiling --locked -p rerail-engine
```

Set `RERAIL_ENGINE_PATH` to the resulting platform addon when running the candidate branch's conformance tools.
The candidate branch records the engine commit, addon checksum, policy checksums, source pin, and complete workload identity.
