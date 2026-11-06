# Data-layer batch ten

Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
There are 100 authored ports out of 142 checks. Thirty documents ship in the private pack.
The [completion review](data-layer-batch-ten-dod-audit.md) exposed optimizer, production configuration, lifecycle, and cumulative-validation gaps.
The latest re-audit disproved the previous completion claim. Configured HNSW file discovery differed from the original reporter.
The repair preserves its separate file and directory behavior. Standalone scanning still accepts an explicit regular file.
The historical 100-rule run matched 25,262 findings over 9,021 code files and seven documents.
That joint measurement predates this scope repair. Only the changed HNSW rule was rerun over its entire input domain.
The [assertion receipt](../fixtures/host/data-layer-port/batch-10-native-assertions.json) separates current comparisons from historical measurements.
The [measurement](performance-decisions/data-layer-batch-ten-20261106.json) records CPU, wall time, compilation, and RSS.
Receipt metadata, disabled-evidence guards, and all 36 evidence-validator tests pass.
Type checking, boundary checks, and Windows path checks pass.
The structure check retains exactly three pre-existing flat-directory findings; this repair adds none.

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
The other 124 cases were previously left on Python implementations.
Of these, 67 rule/helper cases now execute native contracts.
The assertions remain unchanged in 66 cases; one traversal assertion uses actual native counters.
They produce 104 complete helper observations per API, separately from detector results.
The other 57 cases contain 75 transferable assertion sites across 47 cases.
Rerail counterparts now cover those sites, with 102 passing contract tests and controls.
Sixteen Python-specific assertion sites remain in the original tests.
The assertion receipt lists every assertion decision and keeps source execution separate from its Rerail counterpart.
The structural audit maps all 124 cases to their execution targets.
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

The [assertion receipt](../fixtures/host/data-layer-port/batch-10-native-assertions.json) binds assertion results, disabled-detector controls, helper execution, transferred assertions, and implementation hashes.
It also requires the complete union of focused assertions: 276 inputs, including 194 positives, through both APIs.
All 67 rule-relevant helper cases require native traces and complete original results.
Helper replay includes 15 original-evaluated controls, including positive path classification and clean matching modules.
The [repair regressions](../fixtures/host/data-layer-port/batch-10-repair-regressions.json) preserve the nine disproving audit inputs.
The [journal](data-layer-performance-journal.md) records failed attempts and subsequent verification.

These finite tests provide regression evidence. They do not prove equivalence for every possible Python program.

The [fresh re-audit corpus](../fixtures/host/data-layer-port/batch-10-reaudit.json) adds 324 original-evaluated inputs, including 251 positives.
It moves declarations across guard, class, and function boundaries for all ten rules.
The public audit API compares complete configured results through both execution APIs: 648 comparisons.
Ten disabled-detector controls check both APIs. Four persistent revisions check HNSW file-to-directory transitions.
These 660 tests supplement the original suite. They do not replace its assertions or establish universal equivalence.
The complete DoD repair adds production consumer roots, approval validation and signature verification, verified suppression, and `SIDP000`.
It also supplies matching phases, command summaries, and explicitly requested signing through the original signer.
Tests use isolated fixture keys. The runner never searches for private keys.
The original BOM, newline, and Base64 acceptance contracts have separate positive and negative controls.
The permanent lifecycle matrix covers all ten rules and 57 persistent revisions.
The focused matrix includes a 68,008-case original-Python Base64 contract.

The [work log](data-layer-batch-ten-dod-worklog.md) records generic capacity fixes and failed full-run attempts.
It also records SQL repairs for identity selection, lazy rollups, purity seed selection, and bootstrap diagnostic lookup.
No new Linear ticket was needed.
The accepted complete-process measurement is 2,856.494 CPU seconds, 3,035.380 wall seconds, and 10,234,003,456 bytes peak RSS.
Rule compilation is separate: 2.151 CPU seconds and 2.249 wall seconds.
The [validation summary](performance-decisions/data-layer-batch-ten-validation-20261106.json) retains the earlier sixty-rule gates, 4,317 native assertions, and 1,674 passing Rust tests.
Twenty-two existing external-corpus benchmarks remain ignored and individually named.
The source assertion suites do not skip linter cases.

Performance remains expensive. The original complete-process measurement uses 429.978 CPU seconds.
A one-second external sample identifies cache-eviction bookkeeping as a concrete performance lead.
That sample does not establish whole-run CPU attribution.
The journal retains failed runs and distinguishes this correctness baseline from speed comparisons.

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
  --policies packs/data-layer/rules --part 0 --parts 8 \
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

Run the small assertion command for parts 0 through 7, then repeat both commands with `--mode workspace`.
Add `--oracle` and omit `--baseline` to record original execution.
Original recording leaves upstream assertions and linter implementations unchanged.
Native execution translates one Python mock-count assertion into actual native instrumentation assertions.
The semantic audit describes that translation and its source hashes.
Native replay needs CPython 3.13 on PATH.

Run compact replay and configured defaults separately:

```sh
timeout 115s npx vitest run --dir test test/conformance/data-layer/batch-ten.test.ts -t '^(?!matches complete )'
for check in graph_lazy_sql_plan bounded_composed_graph_paths graphql_plan_ownership \
  plan_optimizer_v2 conditional_architecture schema_introspection_data_paths matching_architecture \
  hnsw_architecture embedding_vector_orchestration generated_index_sources
do
  for mode in snapshot workspace
  do
    for part in 0 1
    do
      timeout 115s npx vitest run --dir test test/conformance/data-layer/batch-ten.test.ts -t "^matches complete $check .* $mode API part $part$"
    done
  done
done
npx vitest run --dir test test/conformance/data-layer/batch-ten-repair.test.ts test/engine/native/data-layer-ten/
npx vitest run --dir test test/conformance/data-layer/batch-ten-families.test.ts
npx vitest run --dir test test/conformance/data-layer/batch-ten-helpers.test.ts
```

Run the focused file-contract tests in bounded groups:

```sh
for pattern in '^(?!.*(?:collector|structural)).*' 'structural' 'collector'
do
  timeout 115s npx vitest run --dir test \
    test/conformance/data-layer/batch-ten-file-contracts.test.ts -t "$pattern"
done
```

Run the transferred assertions in the same bounded groups as CI:

```sh
for pattern in \
  '^transfers upstream assertions' \
  '^executes transferred vocabulary .*positive ' \
  '^executes transferred vocabulary .*lookalike ' \
  '^executes transferred vocabulary .*other-family ' \
  '^rejects removal of transferred symbol ' \
  '^(preserves native command paths|rejects disabled native transport|transfers command help)'
do
  timeout 115s npx vitest run --dir test test/conformance/data-layer/batch-ten-transfer.test.ts -t "$pattern"
done
timeout 115s npx vitest run --dir test test/conformance/data-layer/batch-ten-transfer.test.ts -t '^requires native evidence'
```

The data-layer command now defaults to text output and report-only exit behavior.
Use `--check` for enforcement and `--json` for structured output.

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

## Reproduce the cumulative gate

Use the pinned checkout and its Python 3.13 environment, including the recorded dependency versions.
These complete repository measurements have an explicit exception to the ordinary two-minute command bound.
Run them without profiling or memory-census instrumentation.

```sh
python3 scripts/profiling/data_layer/measure.py --output /tmp/original-100-process.json -- \
  python3.13 scripts/dsl/data_layer_suite_record.py \
  --source /path/to/pinned-data-layer --repo /path/to/git-objects \
  --batches 1,2,3,4,5,6,7,8,9,10 --workers 1 --output /tmp/original-100.json

python3 scripts/profiling/data_layer/measure.py --output /tmp/native-100-process.json -- \
  node --import tsx conformance/data-layer/suite.ts \
  --recording /tmp/original-100.json --source /path/to/pinned-data-layer \
  --python /absolute/path/to/venv/bin/python --cold-only --progress \
  --output /tmp/native-100.json

PYTHONPATH=scripts/dsl python3.13 -m data_layer_ten.completion \
  --contracts /tmp/dod.json --oracle /tmp/original-100.json \
  --native /tmp/native-100.json --process /tmp/native-100-process.json \
  --output /tmp/completion.json
```

Generate `/tmp/dod.json` with Vitest's JSON reporter over `test/conformance/data-layer/dod/`.
The verifier requires all 57 named tests, both API paths, all 100 programs, exact finding multiplicity, and zero engine failures.
The cumulative comparison checks rule, path, line, and code.
The accepted repository measurement is cold-only; full-repository unchanged/edit phases were not run.
The separate lifecycle gate covers 57 persistent revisions for the latest ten rules.
Complete latest-ten diagnostic DTOs, configuration, and positive behavior have separate required gates described above.
The historical reporter cuts for earlier batches remain documented in the original recording.
