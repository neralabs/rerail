# Data-layer batch ten

Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
There are 100 authored ports out of 142 checks. Thirty documents ship in the private pack.
The cumulative repository result still covers 80 rules.

## Completion repair

The earlier completion claim was wrong. Nine audit inputs exposed failures in four ports.
The harness also omitted the shared conditional suite and used Python to select matching source files.
The previous 326-case receipt did not establish the claimed contract.

The repair fixes authority-token extraction, application-path classification, repeated assignment targets, and relative import levels.
It also fixes parenthesized conditional rejection, method-event ownership, and backend-control deduplication.
Matching now applies its source-selection predicate in the native rule.

The conditional analyzer now uses canonical Python syntax in SQL.
It no longer reuses the earlier CST approximation from batch six.
Its relations preserve alias timing, class and method replacement, control-flow rejection, branch provenance, and state-write events.
They also preserve observed upstream quirks, including the inert assignment callback.

Generic engine changes support these contracts:

- `ast_level` exposes the actual relative-import level.
- Canonical expansion retains structural parents without source locations, including `match_case`.
- Invalid module expansion yields an empty relation without losing other documents or explicit parse errors.
- SQL-selected source expansion preserves exact source spans and retains their proofs with cached SQL rows.

Earlier batch-ten changes provide complete file text, typed regex evidence, and diagnostic line narrowing.
All rule decisions execute in SQLite. CPython supplies syntax facts without executing analyzed source.

A further audit invalidated the 744-case completion claim. It found raw-newline normalization,
malformed-file isolation, textual line ordering, auxiliary-file admission, and Python Unicode regex defects.
The fixes preserve file APIs separately from source APIs and prevent project checks in tree-only calls.
The focused corpus contains 276 original-evaluated inputs, replayed through both APIs.
The receipt rejects original runs presented as native, missing call traces, changed policies, and incomplete configured coverage.

Matching now selects raw sources in SQL before canonical expansion.
A generic source-relation expansion supplies canonical module facts without a preliminary CST index.
It preserves malformed-module isolation and UTF-8 source spans.
The 100-statement engine limit and existing capacity bounds remain in place.

Commit `8b0c99311` and its completion message overstated these gates.
A subsequent audit found 43 counterexamples in matching and bounded-path checks.
Python whitespace, uppercase expansion, uppercase-name equality, and Unicode decimal paths had non-equivalent SQL translations.
The focused regressions now preserve those counterexamples and their clean controls.
The [journal](data-layer-performance-journal.md) records the retraction and repair evidence.

The [structural audit](data-layer-batch-ten-semantic-audit.md) records the subsequent predicate review and TDD repairs.
It found annotation, nullable-field, Unicode-import, message, result-order, and embedded-NUL line defects.
It also maps each detector's collector contract to its SQL implementation and test evidence.

## Evidence

The [compressed complete corpus](../fixtures/host/data-layer-port/batch-10-complete-api.json.gz) retains source text, parameters, and complete original results.
Compression preserves the full production-file inventory without adding a 59 MB JSON file.

| Rule | Complete API results | Positive results |
| --- | ---: | ---: |
| graph_lazy_sql_plan | 115 | 105 |
| bounded_composed_graph_paths | 66 | 61 |
| graphql_plan_ownership | 50 | 42 |
| plan_optimizer_v2 | 166 | 144 |
| conditional_architecture | 82 | 73 |
| schema_introspection_data_paths | 54 | 45 |
| matching_architecture | 287 | 275 |
| hnsw_architecture | 61 | 52 |
| embedding_vector_orchestration | 70 | 60 |
| generated_index_sources | 24 | 13 |

The corpus has 1,098 cases: 279 upstream cases and 819 supplementary cases evaluated by the original linters.
The upstream cases include all 14 shared conditional tests that the earlier harness omitted.
Supplementary cases include 708 syntax transformations, 50 repair and ownership cases, and 61 earlier boundary cases.
The 708 transformations cover every rule and use positive and clean source seeds.
Their expected results come from original execution, without an assumption that formatting preserves results.

There are 975 complete detector observations, including 870 positive observations, across 974 tests.
Of these observations, 781 have distinct normalized inputs and results; 683 are positive.
The other 124 cases still execute Python helpers or CLI assertions. They are not native detector comparisons.
The v4 receipt lists all their test IDs explicitly under `retainedPythonCaseIds`.
The structural audit identifies their assertion targets and explains their relationship to the native ports.
The conditional observations include 47 calls to the shared analyzer.
Replay compares diagnostic multiplicity, path, code, message, line, and rule-specific DTO fields through both native APIs.
It also checks that public diagnostics agree with SQL result rows.

The matching repository assertion supplies 2,870 production files and the auxiliary operator file, containing 57,328,575 source bytes.
The Python adapter no longer calls `_source_requires_matching_scan` to reduce that inventory.
The native selection gate has a separate file-set comparison against the original selector.
The full matching assertion runs separately from the small cases to keep each command below two minutes.
Compact replay omits that expensive repeated assertion; the native assertion receipt covers it through both APIs.

The [configured corpus](../fixtures/host/data-layer-port/batch-10-configured-defaults.json.gz) has 1,392 original reporter observations, including 978 positives.
It covers every distinct recorded input, owner-path relocation, and focused file-contract inputs.
It covers shipped scopes, clean files, ignored directories, parser behavior, missing artifacts, and matching selection.
Both APIs run it with default options: 2,784 complete-result comparisons.
A further 878 comparisons check the exact matching file selection, including its 139-file repository subset.
There are also 106 diagnostic-family removal controls, each checked through both APIs.
These controls remove one diagnostic code at a time and require all other original findings to remain.

The [receipt](../fixtures/host/data-layer-port/batch-10-native-assertions.json) binds assertion results, disabled-detector controls, and implementation hashes.
It also requires the complete union of focused assertions: 276 inputs, including 194 positives, through both APIs.
The [repair regressions](../fixtures/host/data-layer-port/batch-10-repair-regressions.json) preserve the nine disproving audit inputs.
The [journal](data-layer-performance-journal.md) records failed attempts and subsequent verification.

These finite tests provide regression evidence. They do not prove equivalence for every possible Python program.
No cumulative cold benchmark or new Linear ticket forms part of this repair.

## Run the gates

Generate all ten documents:

```sh
python3.13 scripts/generate/packs/data_layer_batch_ten.py \
  --source /path/to/pinned-data-layer
```

Run the small assertions with a recorded original baseline:

```sh
PYTEST_ADDOPTS="-k 'not test_current_matching_architecture_is_clean_tdd_red'" \
python3.13 scripts/dsl/data_layer_ten/suite.py \
  --source /path/to/pinned-data-layer --repo /path/to/git-objects \
  --baseline fixtures/host/data-layer-port/batch-10-complete-api.json.gz \
  --policies packs/data-layer/rules --part 0 --parts 4 \
  --mode snapshot --output /tmp/batch-ten-0.json
```

Run the full matching assertion as a separate command:

```sh
python3.13 scripts/dsl/data_layer_ten/suite.py \
  --source /path/to/pinned-data-layer --repo /path/to/git-objects \
  --baseline fixtures/host/data-layer-port/batch-10-complete-api.json.gz \
  --checks matching_architecture --policies packs/data-layer/rules \
  --mode snapshot --output /tmp/batch-ten-repository.json \
  /path/to/pinned-data-layer/scripts/tests/test_lint_matching_architecture.py::test_current_matching_architecture_is_clean_tdd_red
```

Run the small assertion command for parts 0 through 3, then repeat both commands with `--mode workspace`.
Add `--oracle` and omit `--baseline` to record original execution.
The recorder does not change upstream assertions or linter implementations.
Native replay needs CPython 3.13 on PATH.

Run compact replay and configured defaults separately:

```sh
npx vitest run --dir test test/conformance/data-layer/batch-ten.test.ts
npx vitest run --dir test test/conformance/data-layer/batch-ten-repair.test.ts test/engine/native/data-layer-ten/
npx vitest run --dir test test/conformance/data-layer/batch-ten-families.test.ts
```

Run the focused file-contract tests in bounded groups:

```sh
for pattern in '^(?!.*(?:collector|structural)).*' 'structural' 'collector'
do
  timeout 115s npx vitest run --dir test \
    test/conformance/data-layer/batch-ten-file-contracts.test.ts -t "$pattern"
done
```

Run configured observations in bounded groups, as CI does:

```sh
for pattern in \
  'configured (graph_lazy_sql_plan|bounded_composed_graph_paths|graphql_plan_ownership) ' \
  'configured (plan_optimizer_v2|conditional_architecture) ' \
  'matching_architecture' \
  'configured (schema_introspection_data_paths|hnsw_architecture|embedding_vector_orchestration|generated_index_sources) '
do
  timeout 115s npx vitest run --dir test \
    test/conformance/data-layer/batch-ten-defaults.test.ts -t "$pattern"
done
```

On macOS, use `gtimeout` in place of `timeout`.
Run the configured repository test separately for each mode with `-t 'configured repository snapshot'`
and `-t 'configured repository workspace'`.
These tests are in `test/conformance/data-layer/batch-ten-configured-repository.test.ts`.
