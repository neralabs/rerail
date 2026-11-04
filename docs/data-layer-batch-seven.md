# Data-layer batch seven

Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
Status: ten candidates implemented; the complete 70-rule repository parity gate passes.

## Rules and source evidence

| Check | Behavior |
| --- | --- |
| `executor_publication_repairs` | Executor-owned publication repair tokens and source-line multiplicity |
| `publication_registry_writes` | Registry writes, deletion targets, and owner exemptions |
| `unknown_contract_defaults` | Empty defaults and truth tests on three-state contracts |
| `declared_fact_interrogation` | Fact-shape interrogation and overridden identity comparisons |
| `history_serverplan_projection` | History imports, projection calls, and production file selection |
| `revision_ancestry_invariants` | Removed descriptors, text casts, and diagnostics without source lines |
| `grammar_mutation_invalidation` | Mutation, invalidation, and constructor/runtime helper reachability |
| `versioned_merged_identity` | Constructor owners, ordered aliases, projections, and absent-owner findings |
| `post_op_no_internal_dispatch` | Internal dispatch, materializer ownership, and ordered payload aliases |
| `post_op_side_effects` | Production inventories, runtime references, signed surfaces, and verification bypasses |

The unchanged upstream suite passes 208 tests.
The [source recording](../fixtures/host/data-layer-port/batch-07-recording.json) contains 153 eligible observations: 106 positive and 47 clean.
The [supplements](../fixtures/host/data-layer-port/batch-07-probes.json) contain 64 observations: 40 positive and 24 clean.
All 217 observations pass snapshot and resident-workspace replay with zero finding differences and zero failures.

Seven captured observations retain findings from an unused legacy helper visitor.
Production code does not instantiate that visitor. These observations remain visible but do not count as production detector passes.
There are also 49 tests without a recorded detector call. Captured helper calls do not certify surrounding signing assertions.
Detection evidence does not certify every reporter, signing workflow, or integration contract.

The edit harness checks four oracle transitions for each rule.
It also removes every file and compares the answer with a fresh empty workspace.
All 50 transitions pass. A missing-owner rule can report an absence after removal.
The ten removal comparisons establish cache consistency, rather than additional oracle coverage.

## Source-driven details

Grammar reachability uses existing partitioned evidence closures.
Partitions retain file and class ownership. Probes cover cycles, nested functions, overlapping constructor/runtime paths, and deep helpers.
The failed correlated-call formulation did not prove a missing engine feature.

Merged identity uses the current function's last assignment before each constructor.
The dispatch checker preserves the source's monotone payload aliases, including its reassignment behavior.
Neither rule substitutes fixture-specific names for configured ownership data.

An absent merged-identity owner has an upstream line-zero sentinel.
The recording preserves `original_line: 0`; replay normalizes it to line one at the declared project location.
Signature probes retain Python's permissive base64 decoding and case-sensitive symbol prefixes.
Production post-op probes rebind file constants to their fixture repository; implementations and public keys remain unchanged.

## Integrated engine compatibility

The selected engine starts at `9bc955cc5`.
Changes are isolated on `jeremy/data-layer-integrated-compat-20261103`; sibling worktree changes were not copied.
The port branch preserves its original executor for comparison.

Compatibility restores relational clauses in the production native API and public TypeScript schema.
It also restores schema fields still implemented by the native compiler: source gates, line scans, capture columns, and helper closures.
All 70 candidates pass the selected frontend and native compiler.
The replay bridge supports both the original execution-owner API and the integrated compiled-rule/workspace API.

Two generic capabilities support actual batch-seven rules:

- `escape: "sql"` converts configured text into UTF-8 hexadecimal SQL text expressions.
- `verify_ed25519` verifies detached signatures on artifact rows and rejects malformed input without a runtime failure.

The production tests cover arbitrary configured text, the RFC 8032 signature vector, tampering, malformed input, and file-level relational anchors.
No new Linear issue was created.

Legacy relational clauses execute through the compatibility evaluator.
This work does not lower them into the newer wave/GPU execution model.
That migration remains a separate execution-contract task.

## Cumulative regressions and measurement

The first 70-rule run matched all 24,993 findings but reported 22 engine failures.
It used 646.248 seconds of cold CPU, 567.704 seconds of cold wall time, and 14.284 GB peak RSS.
Those errors reject the run as parity evidence.

Reduced regressions prove and address two causes:

1. Compatibility loaded repository source before empty-gate admission and treated syntax demand as whole-domain source access.
   It now loads source after admission and loads syntax only for gated files.
   Aggregate byte limits still apply to explicit source-text queries.
2. The `LEG-083` query correlated every syntax node with a function scan.
   It now starts from existing eager anchors.
   The reduced result statement completes with about 658,000 VM instructions, versus 73 million before interruption.

The [regression receipt](../fixtures/host/data-layer-port/batch-07-engine-regressions.json) records source identity and statement-level measurements.
The [journal](data-layer-performance-journal.md) retains complete measurement boundaries and rejected experiments.
The corrected run matches all 24,993 findings with zero engine failures across all 9,021 files.
The final sealed build uses 1.357 seconds of compile CPU, 395.519 seconds of cold CPU, and 321.603 seconds of cold wall time.
OS peak RSS through close is 15,398,617,088 bytes. The memory gate remains open.

The [testsuite audit](data-layer-testsuite-coverage.md) adds native-only coverage enforcement and ten always-clean controls.

## Reproduce the gates

Set `RERAIL_ENGINE_PATH` to the normal-allocator addon built from the selected compatibility commit.
Keep each recording's complete file domain. Diagnostic shards must split independent observations, not their files.

```sh
node --import tsx conformance/data-layer/compile-integrated.ts \
  --source .rerail/data-layer-batch-seven/engine-source \
  --output .rerail/data-layer-batch-seven/integrated-frontend.json

npx vitest run test/conformance/data-layer/batch-seven.test.ts \
  test/conformance/data-layer/integrated-compatibility.test.ts

node --import tsx conformance/data-layer/incremental.ts --batch 7 \
  --recording fixtures/host/data-layer-port/batch-07-recording.json \
  --recording fixtures/host/data-layer-port/batch-07-probes.json \
  --output .rerail/data-layer-batch-seven/incremental-removal.json
```

The complete repository gate uses `suite.ts --cold-only` with all 70 oracle events and all 9,021 pinned files.
Record compile CPU separately from cold CPU, wall time, and OS peak RSS through workspace close.
The pack remains private and inactive.

The [batch receipt](../fixtures/host/data-layer-port/batch-07-evidence.json) binds all gates to the sealed addon.
The [engine pin](../fixtures/host/data-layer-port/batch-07-engine.json) identifies the companion compatibility commit.
The [full measurement receipt](performance-decisions/data-layer-batch-seven-20261103.json) records compilation, cold execution, and close.
The broad native suite has one inherited snapshot-hash failure; 2,515 tests pass and 67 remain ignored.
Both checked source files are unchanged from the selected base. Typechecking and the focused frontend tests pass.
