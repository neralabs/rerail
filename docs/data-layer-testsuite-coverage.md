# Data-layer testsuite coverage

Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
The [coverage receipt](../fixtures/host/data-layer-port/testsuite-positive-coverage-20261103.json) records the complete 70-rule audit.

The unchanged upstream tests collect 4,227 cases: **4,224 pass and three retain their upstream skips**.
These Python results establish the oracle. They are separate from native parity results.

Rerail matches **3,427 native observations: 1,777 positive and 1,650 clean**.
All 70 rules have both native positive and native clean coverage.
There are zero extra findings, missing findings, engine failures, or snapshot/workspace differences.
Three additional signature observations execute through the existing host guard and do not count as native coverage.

## Stronger acceptance gate

Replay now rejects a corpus unless each rule has both positive and clean observations executed by the native engine.
Host signature failures cannot satisfy native positive coverage.
A regression proves this: one host-reported signature failure and one native clean result no longer certify a rule.

Each batch-seven rule also has an always-clean control.
The harness rejects all ten controls against original upstream positive observations and counts every missing finding.
The batch-seven gate requires native execution and zero host-guard observations.

The focused conformance suite passes 54 tests. Typechecking passes.
The sixth batch also passes all 1,259 observations in 24 bounded shards.
Each shard preserves every observation's complete file domain.
Positive and clean coverage are aggregated across shards before acceptance.

## Additional original tests and positive fixtures

The recorder now captures the original broad-authority and exact-symbol coverage helper tests.
It materializes their exact registry arguments as Python assignments and retains the original AST source.
Expected findings still come from the unchanged original helper calls.
The port selects that helper's diagnostic contract through the existing `entryPoint` option.

This adds three direct unit observations and two helper observations inside a signing test.
Capturing those helper calls does not certify the surrounding signing workflow.
The first native replay failed because it reported unrelated production diagnostics at these helper boundaries.
The corrected helper modes match the original findings.

Two additional oracle probes cover `HSP-003` and `POSTOP001`.
Batch seven now has **217 native observations: 146 positive and 71 clean**, covering 35 diagnostic codes.
Its unchanged Python suite still passes all 208 tests.

## Test setup and remaining boundaries

Run tests from a real Git worktree at the source pin.
Two signing tests read their committed baseline through `git show HEAD`.
An archive without Git metadata cannot satisfy that contract.
The inherited `GIT_PAGER` variable also causes the upstream scan guard to reject this read.
Remove that variable for the test process; keep the guard and test assertions intact.

```sh
env -u GIT_PAGER /path/to/data-layer/python scripts/dsl/data_layer_record.py \
  --source /path/to/pinned-data-layer-worktree \
  --repo /path/to/data-layer-git-repository \
  --batch 6 --output /path/to/batch-06-recording.json
```

Seven captured calls exercise an unused legacy visitor and remain outside production-rule replay.
Batch seven has 49 tests with no recorded detector call, including constants, helpers, and integration assertions.
Other batches also retain helper and integration boundaries in their recordings.
Do not describe the native observation count as a count of fully translated pytest cases.

The `POSTOP000` signing fallback has no positive fixture.
The other nine production post-op diagnostic codes have positive coverage.
The broader native suite's inherited snapshot-hash failure remains documented in the batch-seven report.

The production post-op policy resolves to exactly the same default program as before this audit.
The receipt records matching resolved-program hashes.
The earlier full-repository measurement remains historical; it was not rerun or relabeled for these test changes.
