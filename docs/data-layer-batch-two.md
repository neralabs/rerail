# Data-layer second batch

The next ten checks now have native Rerail candidate policies.
The original test corpus and additional oracle probes pass with zero detection mismatches.
The first batch still passes.
The twenty policies remain inactive candidates while integration work remains open.

The oracle revision is `2b1504adc4291daaf080b2e932962b134da1a69a` in `neralabs/nera-data-layer`.
The [evidence file](../fixtures/host/data-layer-port/batch-02-evidence.json) records provenance, coverage, policy digests, and the native binary hash.
No new Linear issues were created.
Jeremy must approve every proposed issue before creation.

## Ports and corpus

The ten policies are in [candidates/rules](../packs/data-layer/candidates/rules/).
They contain 120 reporting clauses, including separate clauses for analyzer scope and production reporter scope.
Shared definitions do not count as reporting clauses.
The batch unit remains a source `LinterConfig` check, rather than an individual diagnostic clause.

| Check | Reporting clauses | Positive calls | Clean calls | Expected findings |
|---|---:|---:|---:|---:|
| `dynamic_module_loading` | 5 | 30 | 24 | 35 |
| `forwardref` | 3 | 2 | 3 | 2 |
| `unresolved_forward_refs` | 1 | 36 | 25 | 39 |
| `classvar_check` | 1 | 5 | 3 | 5 |
| `type_safety_patterns` | 9 | 31 | 15 | 32 |
| `conn_or_txn_union` | 2 | 2 | 3 | 2 |
| `async_executor` | 1 | 28 | 11 | 34 |
| `async_patterns` | 32 | 8 | 4 | 8 |
| `async_sync_boundary` | 63 | 3 | 2 | 3 |
| `admin_async_purity` | 3 | 26 | 14 | 31 |
| Total | 120 | 171 | 104 | 191 |

All 700 original pytest cases passed with the original root configuration and fixtures.
The recorder checked 1,504 loaded Python source files against pinned Git blobs, with no mismatch.
The [original corpus](../fixtures/host/data-layer-port/batch-02-recording.json) contains 275 replayable analyzer calls.
Another 29 calls retain Python-only contracts, such as mocked readers or unreadable files.
Another 399 tests have no recorded analyzer call.
Those tests remain visible; they do not count as native parity cases.

The [supplemental corpus](../fixtures/host/data-layer-port/batch-02-probes.json) contains 58 additional source cases.
Each case runs through both the pinned analyzer and its actual FactEngine plugin reporter.
This produces 116 calls: 74 positive and 42 clean, with 84 expected findings.
These are additional oracle probes, separate from the original 700 pytest cases.
The probe run checked 125 loaded source files against the pin.
The [probe recorder](../scripts/dsl/data_layer_probe_record.py) reproduced the committed corpus without a difference.

Across both batches and these probes, native replay covers 708 calls and 572 expected findings.
There are 479 positive calls and 229 clean calls.
All replay paths have zero false positives, false negatives, engine failures, or snapshot/workspace differences.
The [CI gate](../test/conformance/data-layer/ports.test.ts) rejects empty and always-reporting detectors.
It also rejects the correct location and code with the wrong type-safety category.

## Behavior preserved during porting

The policies preserve the original scanners' behavior, including deliberate source-text heuristics.
Dynamic loading uses literal file triggers, ordered quote transitions, comment filters, and the original `TYPE_CHECKING` indentation heuristic.
Admin purity counts both triple-quote delimiters and toggles on odd counts, even inside comments.
The executor scanner splits only on LF and applies its line suppression marker.
These checks can process malformed Python without an AST.

Annotation checks use existing AST queries, capture order, enclosing scopes, decoded values, and reusable definitions.
Forward-reference guards preserve their order relative to assignments and checks.
Resolved-hint fallbacks accept the original analyzer's later assignments.
ClassVar comparisons preserve operator position and repeated findings in comparison chains.
Connection/transaction checks preserve the first union diagnostic and separate-parameter fallback.

Async checks use existing bindings and annotation queries, plus source-order queries for aliases.
The original collector keeps some names after reassignment; the policies preserve that behavior.
The supplemental corpus includes reassigned transactions, sessions, query aliases, multiple targets, and aliases created before their source.
These ports did not require a new receiver-resolution engine feature.

Analyzer results can differ from production findings.
Async-pattern reporters exclude fallback findings; boundary reporters apply file and function exceptions.
The shared `oracleAnalyzer` option selects the recorded analyzer behavior during replay.
Its default is false, so ordinary policy execution uses production reporter behavior.
Supplemental probes test both settings against their corresponding original entry points.

The scorer preserves multiplicity, path, line, and diagnostic code.
It also compares diagnostic category for dynamic loading, admin purity, and type safety.
This prevents different diagnostics with the same code and line from canceling each other.

## Engine additions

These features are general DSL operations with native evaluation.
The native engine contains no data-layer check names or Python callbacks.

| Feature | Proven source need | Implementation requirements |
|---|---|---|
| Complete-file content filters | [Dynamic loading's fast trigger](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_dynamic_module_loading.py#L253) | Test the current file before scanning; support bounded include/exclude regexes; update after edits. |
| Per-file line state | [Dynamic loading's delimiter state](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_dynamic_module_loading.py#L258), [admin quote parity](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_admin_async_purity.py#L210) | Start fresh per file; apply ordered regex transitions; support bounded counts and parity; decide emission from starting state and transition suppression. |
| Line indentation filters | [Dynamic loading's type-checking heuristic](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_dynamic_module_loading.py#L209) | Preserve marker indentation until reset; ignore configured blank/comment lines; judge current line before updating state. |
| LF-only line splitting | [Executor source scanner](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_async_executor.py#L123) | Keep Python splitlines behavior as the default; allow LF-only scanning with correct original offsets. |
| Bounded context windows | [Signature scanner](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_type_safety_patterns.py#L483), [async fallback comments](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_async_patterns.py#L578) | Match complete lines before/after a witness; clip at file bounds; optionally stop at a matching line, inclusive. |

The compiler limits state names, state counts, transitions, context ranges, and regex sizes.
Counted transition scans and context searches observe the work budget.
Malformed definitions fail compilation; exhausted evaluation reports an incomplete result.

The [DSL reference](code-rule-dsl-reference.html) documents these operations.
Six [native capability tests](../test/engine/native/data-layer-line-state.test.ts) cover transitions, parity, malformed source, offsets, content gates, context bounds, and invalid definitions.
They also cover edits, deletion, and isolation between files.

## Repository measurement and limits

The cumulative scan uses all twenty policies together across 9,010 pinned Python files and 137,896,473 UTF-8 bytes.
Both the original shared fact engine and the native workspace use four workers.
The oracle fact cache is disabled; Rerail starts without resident files or an attached store.
Both scans produce zero findings, with zero native failures.
The positive corpus supplies the separate recall evidence.

| Measurement | Observed time |
|---|---:|
| Python shared fact engine, cold | 23.09 s |
| Native workspace, cold | 14.36 s |
| Native policy compilation | 249.8 ms |
| Native unchanged-file resend | 3.9 ms |
| Native one-file whitespace edit | 19.4 ms |

The evidence file records the input size, worker count, allocator label, policy digests, and binary hash.
These are individual local measurements, with concurrent machine activity.
The native build uses a system-allocator shim because the local jemalloc build invokes a blocked cleanup command.
These timings do not certify production jemalloc throughput or a stable speed ratio.
The cold run spent 11.00 aggregate worker seconds parsing, 6.41 scanning regexes, and 3.44 encoding tapes.
AST matching consumed 0.93 aggregate worker seconds; manifest candidate lookup consumed 2.98 seconds.
These concurrent worker counters do not add up to wall time.
Shared text scans, parsing, tape costs, and dispatch overhead remain the main measured optimization areas.

The original Python suite does not prove native coverage for all 700 tests.
The 29 retained calls and 399 uncaptured tests need separate contract review where relevant.
Message wording, severity, CLI rendering, signed allowlists, and enforcement integration remain outside the detection gate.
Existing decoded-string location defects in the first batch remain open, as described in its report.
Python AST and tree-sitter syntax acceptance also need further edge-case coverage before complete migration certification.

## Verification

All 244 focused tests passed across the port gate, native DSL runtime, capability tests, and TypeScript schema tests.
All six scoped pack checks and all 25 structure checks passed.
Rust formatting and Clippy passed, with all targets and warnings denied.
The DSL grammar check found zero differences across 38 productions.
Boundary, license, Windows-path, and Python syntax checks passed.
The probe recorder reproduced the committed oracle fixture exactly.

The full TypeScript check still reports three errors in unchanged activation and PI integration files with the borrowed dependency installation.
These checks do not establish a passing full repository test suite.

## Reproduce

Use a complete checkout or archive at the pinned revision and its Python environment.
Build the native addon before replay.
Run these commands from the Rerail root.

```sh
DL_SOURCE=/path/to/pinned/data-layer
DL_REPO=/path/to/data-layer/git/checkout
DL_PYTHON=/path/to/data-layer/.venv/bin/python

"$DL_PYTHON" scripts/dsl/data_layer_record.py --batch 2 \
  --source "$DL_SOURCE" --repo "$DL_REPO" \
  --output /tmp/data-layer-batch-02.json

node --import tsx conformance/data-layer/replay.ts \
  --recording /tmp/data-layer-batch-02.json \
  --output /tmp/data-layer-batch-02-parity.json

"$DL_PYTHON" scripts/dsl/data_layer_probe_record.py \
  --source "$DL_SOURCE" --repo "$DL_REPO" \
  --output /tmp/data-layer-batch-02-probes.json

node --import tsx conformance/data-layer/replay.ts \
  --recording /tmp/data-layer-batch-02-probes.json \
  --output /tmp/data-layer-batch-02-probes-parity.json

"$DL_PYTHON" scripts/dsl/data_layer_suite_record.py --batches 1,2 \
  --source "$DL_SOURCE" --repo "$DL_REPO" --workers 4 \
  --output /tmp/data-layer-suite.json

node --import tsx conformance/data-layer/suite.ts \
  --recording /tmp/data-layer-suite.json \
  --output /tmp/data-layer-suite-parity.json --allocator production
```

The allocator argument records the operator's label; it does not change the allocator.
The report hashes the loaded native binary and records compiled policy digests.
The [third batch](data-layer-batch-three.md) now contains ownership, callback, side-effect, and cache ports.
It retains both earlier corpus gates and the cumulative repository measurement.
