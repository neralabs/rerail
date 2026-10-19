# Data-layer first-batch execution report

The first ten checks now have native Rerail candidate policies and a working recording and replay harness.
Porting these checks identified three reusable engine capabilities, which are now implemented.
The recorded detection corpus passes with zero false positives, false negatives, engine failures, or execution-path differences.
The policies remain candidates because detection parity does not certify the complete integration contract.

Batch two is now implemented in the [second-batch report](data-layer-batch-two.md).
The [third batch](data-layer-batch-three.md) now extends the gate to thirty checks.
The measurements below remain the original first-batch observations.

Source: `2b1504adc4291daaf080b2e932962b134da1a69a` in `neralabs/nera-data-layer`.
The [evidence file](../fixtures/host/data-layer-port/batch-01-evidence.json) records counts, provenance, and the measured native binary hash.
The [batch manifest](../fixtures/host/data-layer-port/batches.json) retains all 142 checks in batches of ten, then two.
The [engine feature ledger](data-layer-engine-gaps.md) tracks three proven branch features under Linear umbrella NER-1967.
Jeremy must approve every new Linear issue before creation.

## Implemented checks

Policies live in [packs/data-layer/candidates/rules](../packs/data-layer/candidates/rules/).
There are ten check documents containing 41 diagnostic clauses.

| Check | Clauses | Positive calls | Clean calls |
|---|---:|---:|---:|
| `removed_compat_surfaces` | 3 | 3 | 1 |
| `versioned_lifecycle_legacy_surfaces` | 1 | 1 | 2 |
| `no_api_root_imports` | 2 | 2 | 2 |
| `backend_type_strings` | 13 | 43 | 17 |
| `hasattr_async` | 2 | 19 | 14 |
| `no_source_inspection` | 5 | 54 | 18 |
| `serverplan_name_inference` | 4 | 7 | 1 |
| `no_blanket_grammar_bypass` | 3 | 97 | 24 |
| `serverplan_grammar_surface` | 7 | 6 | 1 |
| `permission_error` | 1 | 2 | 3 |

The original ten test modules collected 567 cases: 564 passed and three skipped.
This run used the complete pinned archive, including its original root `conftest.py` and pytest configuration.
The recorder verified 1,493 loaded Python source files against Git blob identities without a mismatch.

The native replay contains 317 calls: 234 positive calls and 83 clean calls.
Those calls contain 297 expected findings.
The [recording](../fixtures/host/data-layer-port/batch-01-recording.json) is now a native CI fixture.
Its original Python test modules are not bundled or published.
The [native gate](../test/conformance/data-layer/ports.test.ts) replays this corpus without a data-layer checkout.
The scorer preserves duplicates and compares check, code, path, and line.
It also compares the column for backend string checks.
Each check must have positive and clean cases.

The gate rejects a deliberately empty detector with 297 false negatives.
It also rejects an always-reporting detector with 1284 false positives.
The native evaluator produces all candidate findings; it never calls the Python oracle.

## Harness behavior and limits

[data_layer_record.py](../scripts/dsl/data_layer_record.py) observes analyzer calls while the unchanged original pytest tests execute.
It records inputs before analysis and results afterwards.
Original fixtures, parameter expansion, helper calls, and assertions remain active.
A call enters replay only when its original test passes.

[replay.ts](../conformance/data-layer/replay.ts) compiles the actual candidate documents.
It evaluates each recorded case through both native snapshots and the production workspace, then reads paged workspace results.
Explicit-file tests use their original entry point's scope.
They do not inherit production default roots that the original test bypassed.
The backend `no_skip` fixture is recorded explicitly.

Twenty-two calls test unreadable files, source-reader mocks, or missing-file CLI diagnostics.
They retain their original assertions and remain separately reported.
Another 301 collected tests have no recorded analyzer-boundary call.
They include helper APIs, constants, discovery, CLI behavior, and plugin reporters.
They remain visible for adapter or contract review; they do not count as native parity cases.

The candidates use block mode, matching the ten plugins' hard-fail enforcement.
Message wording, CLI output, severity, and enforcement integration are outside the current detection gate.
Raw removed-surface scanning also deduplicates findings that its suite reporter preserves.
That facade distinction needs an explicit adapter when the corpus includes duplicate occurrences.
Python AST and tree-sitter acceptance can differ on syntax edge cases.
Decoded strings and concatenated literals also need additional location tests before claiming general equivalence.
These limitations prevent full migration certification, despite the passing recorded corpus.

Two later probes confirm decoded-string false negatives outside that corpus.
An import split across adjacent literals produces no native finding, while the oracle produces one.
Two imports separated by an escaped newline produce one native finding, while the oracle produces two.
Existing decoded-value queries can detect the adjacent-literal constant without an engine change.
These candidate defects do not establish a missing decoder or the broad extension proposed in the withdrawn NER-1968.
The [engine feature ledger](data-layer-engine-gaps.md) retains the evidence without asserting a new engine requirement.

## Engine capabilities added during porting

- Regex queries now support independent Python `str.splitlines()` lines, line filters, match filters, and first-match reporting.
  They preserve Unicode separators, byte offsets, and named capture locations.
  Query path restrictions express basename-specific checks and allowed-file exceptions.
- AST queries can gate on whole-file valid or invalid syntax.
  Valid-only checks skip the complete file; invalid-only checks can emit a normal syntax diagnostic.
  The gate also works in cross-file selection and incremental repair.
- Layout predicates can inspect the complete current or previous line.
  Permission-error suppression uses these predicates for inline and preceding markers.

These are general DSL features.
No native branch recognizes a data-layer check name or forbidden symbol.
The [DSL reference](code-rule-dsl-reference.html) documents them.
Five native integration tests cover these capabilities, including offsets, rejection, incremental repair, and cross-file evidence.

## Measured speed

The combined repository run uses all ten candidates together.
The input contains 9,010 pinned Python files and 137,896,473 UTF-8 bytes.
The original shared fact engine and Rerail both use four workers.
The oracle fact cache is disabled; Rerail starts with an empty resident workspace and no attached store.
Rerail uses the production binary input protocol, bounded file batches, and paged results.
Both runs report zero repository findings and zero native failures.
The positive corpus supplies the separate recall evidence.

| Measurement | Observed time |
|---|---:|
| Python shared fact engine, cold | 9.36 s |
| Native workspace, cold | 11.04 s |
| Native compilation | 53.2 ms |
| Native unchanged-file resend | 3.2 ms |
| Native one-file whitespace edit | 15.9 ms |

These are individual observations on a shared machine, not a stable throughput ratio.
The local jemalloc build invokes a blocked cleanup command.
The tested addon instead links an existing system-allocator shim.
The native source was built for this work; an older cached addon was not used for these results.
Production allocator, storage, repeated cold samples, and latency distributions remain unmeasured.

Worker phase counters overlap wall time because workers run concurrently.
The full run spent 9.95 aggregate seconds parsing, 4.99 scanning regexes, and 3.22 encoding tapes.
AST matching consumed 0.20 aggregate seconds.
The first optimization targets should therefore be shared text scans, parsing and tape costs, and dispatch overhead.
A shared split-line index could avoid rebuilding Unicode line boundaries for every regex clause.
Adding more specialized AST matcher shortcuts is not supported by this batch's profile.

## Verification

The focused native, schema, and port tests passed: 233 tests.
The data-layer pack checks passed, as did all 25 repository structure checks and the DSL grammar check.
Rust formatting and Clippy passed, including all native targets with warnings denied.
The original Python suite passed with 564 tests and three skips.

Five additional cases cover annotation references, deleted helpers, fallback codes, removed names, decorated methods, and production cache exclusions.
Their expected findings were checked directly against the pinned oracle.
They cover the two diagnostic codes without positives in the recorded upstream corpus.

The full TypeScript check still reports three errors in unchanged activation and PI integration files with the borrowed dependency installation.
An attempted full pack-catalog test aborted with SIGABRT; the scoped data-layer pack checks passed separately.
These checks do not establish a passing full repository test suite.

## Reproduce

Use a complete source archive or clean checkout at the pinned revision.
Use data-layer's Python environment so its dependencies and syntax are available.
Generated inputs remain outside the published pack.
Run these commands from the Rerail root after building its native addon.

```sh
DL_SOURCE=/path/to/pinned/data-layer
DL_REPO=/path/to/data-layer/git/checkout
DL_PYTHON=/path/to/data-layer/.venv/bin/python

"$DL_PYTHON" scripts/dsl/data_layer_record.py \
  --source "$DL_SOURCE" --repo "$DL_REPO" \
  --output /tmp/data-layer-batch-01.json

node --import tsx conformance/data-layer/replay.ts \
  --recording /tmp/data-layer-batch-01.json \
  --output /tmp/data-layer-batch-01-parity.json

"$DL_PYTHON" scripts/dsl/data_layer_suite_record.py \
  --source "$DL_SOURCE" --repo "$DL_REPO" --workers 4 \
  --output /tmp/data-layer-suite.json

node --import tsx conformance/data-layer/suite.ts \
  --recording /tmp/data-layer-suite.json \
  --output /tmp/data-layer-suite-parity.json --allocator production
```

The allocator label is supplied by the operator; it does not change the allocator.
The report records the loaded native binary's SHA-256.

## How subsequent batches should proceed

Keep the original pytest suite as the corpus generator.
Add adapters when a new batch exposes another analyzer boundary or repository artifact.
Keep helper and CLI contracts in Python unless the Rerail integration exposes the same contract.
Do not count missing captures, unknown results, or unexplained divergences as passes.

Before activating this batch, close the identified integration and edge-case gaps.
Batch three retains both earlier gates. Continue with batch four's ten checks.
Each batch should run its own positive corpus and the cumulative repository catalog.
Add engine semantics only when a concrete port and oracle case require them.
Before proposing a missing capability, demonstrate it through an actual port on this branch.
Prepare a draft with pinned linter lines, implementation requirements, and positive and clean acceptance cases.
Present each proposed Linear issue to Jeremy and wait for explicit approval before creation.
This approval requirement supersedes the earlier automatic ticket-creation instruction.
The [engine feature ledger](data-layer-engine-gaps.md) records the retained issues and the approval process.

Batch two exercised annotation and receiver semantics; batch three covered ownership and effects. The next batch tests SQL grammar.
Repository symbol and export checks come later.
Those ports should determine whether existing resolution and call summaries suffice before introducing another project graph.
Measure cold audits and edits separately with unchanged scope and diagnostic expectations.
A faster incomplete scan is not a performance improvement.
