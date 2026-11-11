# Data-layer third batch

The next ten checks now have native Rerail candidate policies, bringing the total to 30 of 142 checks.
The original test corpus and supplemental probes pass with zero detection mismatches.
Both earlier batches still pass.
This batch uses existing engine capabilities; no additional native engine feature was required.
The pack remains inactive while integration certification remains open.

The source pin is `2b1504adc4291daaf080b2e932962b134da1a69a` in `neralabs/nera-data-layer`.
The [evidence file](../fixtures/host/data-layer-port/batch-03-evidence.json) records provenance, coverage, policy digests, and the native binary hash.

## Policies and original tests

The policies live in [candidates/rules](../packs/data-layer/candidates/rules/).
They contain 198 reporting clauses, including path partitions and separate analyzer/reporter clauses.
Shared definitions do not count as reporting clauses.
The batch unit remains one source `LinterConfig` check.

| Check | Reporting clauses | Positive calls | Clean calls | Expected findings |
|---|---:|---:|---:|---:|
| `plan_scope_side_effects` | 6 | 6 | 1 | 6 |
| `plan_scope_ownership` | 26 | 13 | 1 | 14 |
| `transaction_closures` | 6 | 29 | 47 | 32 |
| `transaction_callback_wrappers` | 2 | 2 | 2 | 2 |
| `transaction_patterns` | 58 | 21 | 5 | 25 |
| `lazy_result_usage` | 6 | 2 | 3 | 2 |
| `swallowed_exceptions` | 4 | 11 | 5 | 13 |
| `error_handling` | 2 | 2 | 1 | 2 |
| `module_level_caches` | 40 | 3 | 2 | 7 |
| `connection_caches` | 48 | 27 | 11 | 29 |
| Total | 198 | 116 | 78 | 132 |

All 568 unchanged upstream pytest cases passed with the original root configuration.
The recorder checked 1,513 loaded Python source files against pinned Git blobs without a mismatch.
The [recording](../fixtures/host/data-layer-port/batch-03-recording.json) supplies 194 native replay calls.
Three calls retain original framework extraction-error contracts.
Another 402 tests have no recorded supported boundary.
They include detector helpers, constants, formatting, discovery, signatures, and baseline operations; they do not count as native passes.

This batch observes shared fact materialization and direct AST compatibility visitors.
The [adapter](../scripts/dsl/data_layer_batch_three.py) records the original parsed source, rather than reconstructing it with `ast.unparse`.
Original fixtures, assertions, and caller configuration remain active.
Extraction errors remain explicit and cannot enter the corpus as clean detections.

## Supplemental probes and scope

The [supplemental corpus](../fixtures/host/data-layer-port/batch-03-probes.json) contains 88 additional source cases and 176 analyzer/reporter observations.
Native replay compares 167 calls: 76 positive and 91 clean, with 92 expected findings.
Nine signed aggregate reporter observations remain separate retained contracts.
These probes are separate from the original 568 pytest cases.
Their run checked 125 loaded oracle source files against the pin.

The probe recorder uses temporary project roots and copies existing signed baseline artifacts unchanged.
It neither signs nor changes those artifacts.
Project-root globals point to the temporary project during these probes.
This preserves project-local path exceptions, including paths resolved through filesystem symlinks.

Across all three original corpora and both supplemental corpora, native replay covers 1,069 calls and 796 expected findings.
There are 671 positive calls and 398 clean calls.
Every comparable corpus has zero false positives, false negatives, engine failures, or snapshot/workspace differences.
The [CI gate](../test/conformance/data-layer/ports.test.ts) rejects empty, always-reporting, wrong-category, and wrong-severity detectors.
The empty third-batch detector produces 132 false negatives.

The native policies perform detection without calling Python.
Python remains the behavior oracle and retains contracts outside the detection gate.
The swallowed-exception policy emits source detections with severity codes in `steer` mode.
The original plugin verifies a signed aggregate baseline and reports its aggregate result.
Those are separate output contracts; the native gate does not claim aggregate enforcement parity.

## What the ports established

Plan-scope policies preserve nearest-scope ownership, eager-call checks, receiver-token checks, and source-order read tracking.
Audit findings remain available to analyzer replay; production evaluation excludes them as the original reporter does.
Ownership checks preserve nominal signatures, duplicate diagnostics, workflow classes, literal source checks, and actual attribute stores.

Closure policies preserve callback argument precedence, named callback definitions, repeated callback findings, and outer-variable mutation checks.
Local parameters and assignments follow the original collector's traversal behavior, including registration before an assignment's RHS.
The two compatibility visitors remain independently replayable through their respective clause subsets.

Callback wrappers preserve class boundaries, reviewed paths, annotations, and nested reentrant callbacks.
Transaction patterns preserve first-parameter selection, category configuration, inference order, and internal-transaction overrides.
The harness applies configuration before expanding option references and preserves category precedence.
Lazy-result policies preserve class exclusions, sibling authority methods, decorators, placeholder bodies, and return annotations.

Exception policies preserve handler ownership, protocol exceptions, severity, and legacy/current suppress-report multiplicity.
Error handling preserves adjacent direct statements and admin commit scope.
Cache policies preserve source triggers, chained assignments, managed constructors, decorators, and path/name-specific exceptions.

Several defects appeared during actual replay:

- A broad assignment-parent query treated attribute reads as writes. Parent-field matching and target ancestry now distinguish them.
- Recursive descendant searches repeated subtree work and exhausted the budget. Direct-child recursion now visits the relevant structure without that repetition.
- Whole-assignment ordering missed local registration before RHS evaluation. The policies now compare the target's position.
- Raw annotation regexes matched comments removed by the original AST formatter. Structural identifier and decoded-string queries now express that predicate.
- Chained assignments exposed multiple targets behind one terminal value. Shared recursive utilities now preserve each finding.
- An audit query searched nested calls where the oracle checks the condition's root. Root-call checks and referenced-name checks are now separate.

These were candidate or harness defects, rather than missing engine representation features.
The existing AST relations, node identity comparisons, decoded values, typed options, utilities, and per-binding reporting represent the demonstrated shapes.
No new Linear issue was created.

## Repository measurement

The cumulative run uses thirty policies across 9,010 pinned Python files and 137,896,473 UTF-8 bytes.
It compares 29 production plugin reporters and the swallowed-exception source fact extractor.
The signed swallowed-exception reporter result remains separately recorded; it reports no aggregate violation on this pin.
All 807 expected source detections match, with zero native failures or other findings.

| Measurement | Observed time |
|---|---:|
| Python shared fact engine and reporters, cold | 20.38 s |
| Native workspace, cold | 17.14 s |
| Native policy compilation | 1,215.7 ms |
| Native unchanged-file resend | 8.7 ms |
| Native one-file whitespace edit | 32.3 ms |

Both engines use four workers; the oracle fact cache is disabled.
Rerail starts without resident files or an attached store and uses bounded binary input batches and paged results.
These are individual observations on a shared machine.
The native build enables `code-rule-profile` and uses a system-allocator shim.
They do not establish a stable speed ratio or production jemalloc throughput.

Shared utilities reduced repeated policy structure and made the expensive subtree query complete within the existing work budget.
The cold run spent 10.98 aggregate worker seconds parsing, 6.91 scanning regexes, and 3.48 encoding tapes.
AST matching consumed 3.25 aggregate seconds; manifest candidate lookup consumed 4.26 seconds.
These counters overlap because workers run concurrently.
Parsing, text scans, dispatch, and repeated subtree work remain measured optimization areas.

## Verification and limits

All 249 focused native, schema, and port tests passed, as did six scoped pack checks.
All 25 structure checks passed, along with boundary, license, Windows-path, and DSL grammar checks.
The policy generator reproduced all ten documents byte for byte after verifying pinned source and configuration artifacts.
Both supplemental probe corpora regenerate exactly.
Analyzer-only configuration does not enter production reporter events.
The native binary is unchanged from the validated second-batch build.

The full TypeScript check still reports three errors in unchanged activation and PI integration files with the borrowed dependency installation.
The detection gate does not certify message wording, CLI rendering, signed policy integration, or ratchet activation.
The original helper and extraction-error contracts remain visible for separate review.
The first batch's known decoded-string location defects also remain outside its passing corpus.
These results do not certify the complete migration or a passing full repository test suite.

## Reproduce

Use a complete archive or checkout at the pin and its Python environment.
Run these commands from the Rerail root after building the native addon.

```sh
DL_SOURCE=/path/to/pinned/data-layer
DL_REPO=/path/to/data-layer/git/checkout
DL_PYTHON=/path/to/data-layer/.venv/bin/python

"$DL_PYTHON" scripts/dsl/data_layer_record.py --batch 3 \
  --source "$DL_SOURCE" --repo "$DL_REPO" \
  --output /tmp/data-layer-batch-03.json

node --import tsx conformance/data-layer/replay.ts \
  --recording /tmp/data-layer-batch-03.json \
  --output /tmp/data-layer-batch-03-parity.json

"$DL_PYTHON" scripts/dsl/data_layer_probe_record.py \
  --source "$DL_SOURCE" --repo "$DL_REPO" \
  --inputs fixtures/host/data-layer-port/batch-03-probes.json \
  --output /tmp/data-layer-batch-03-probes.json

"$DL_PYTHON" scripts/dsl/data_layer_generate_batch_three.py \
  --source "$DL_SOURCE" --repo "$DL_REPO" --output /tmp/data-layer-policies

"$DL_PYTHON" scripts/dsl/data_layer_suite_record.py --batches 1,2,3 \
  --source "$DL_SOURCE" --repo "$DL_REPO" --workers 4 \
  --output /tmp/data-layer-suite.json

node --import tsx conformance/data-layer/suite.ts \
  --recording /tmp/data-layer-suite.json \
  --output /tmp/data-layer-suite-parity.json --allocator production
```

The allocator argument records a label; it does not change the allocator.
The next batch contains SQL grammar checks and retains all current parity gates.

## Approved Linear amendments

[NER-1969](https://linear.app/nera/issue/NER-1969) now includes same-file content gates and selectable line terminators.
[NER-1971](https://linear.app/nera/issue/NER-1971) now includes bounded witness-relative source context.
Both descriptions preserve the earlier requirements and add pinned linter links, implementation requirements, and acceptance evidence.
Their workflow states remain unchanged.
The separate proposed stateful-filter ticket remains uncreated pending explicit approval.
