# Data-layer batch ten

Source pin: `2b1504adc4291daaf080b2e932962b134da1a69a`.
The inventory now has 100 authored ports out of 142 checks.
The private pack contains 30 shipped documents. Seventy earlier ports remain candidates.

| Rule | Complete API observations | Positive observations |
| --- | ---: | ---: |
| graph_lazy_sql_plan | 37 | 33 |
| bounded_composed_graph_paths | 12 | 11 |
| graphql_plan_ownership | 11 | 9 |
| plan_optimizer_v2 | 32 | 25 |
| conditional_architecture | 4 | 3 |
| schema_introspection_data_paths | 12 | 9 |
| matching_architecture | 48 | 44 |
| hnsw_architecture | 19 | 16 |
| embedding_vector_orchestration | 16 | 12 |
| generated_index_sources | 12 | 7 |

The [complete corpus](../fixtures/host/data-layer-port/batch-10-complete-api.json) retains source text, caller parameters, and complete original results.
Replay checks diagnostic multiplicity, path, code, message, and line through snapshot and workspace execution.
It also checks that the public diagnostic agrees with the SQL result row.

The corpus contains 326 pytest cases: 265 original upstream cases and 61 added cases evaluated by the original linters.
The optimizer has no dedicated upstream test module. Its 32 added cases cover every diagnostic family, including registry and documentation checks.
The other 29 added cases cover aliases, quoted annotations, generic bases, parser errors, file artifacts, and direct source APIs.

There are 203 recorded detector results, including 169 positive results.
These results occur in 202 tests. The other 124 tests check upstream utilities, constants, DTOs, discovery, or CLI behavior.
Those utility and CLI tests retain their original assertions. They do not count as native detector observations.
Original detector functions and collector methods are forbidden during native detector replay.

The graph source API completes independent files concurrently. Its cross-file list order varies between original runs.
Replay compares its complete results as a multiset. Per-file diagnostic fields and multiplicity remain exact.
Generated-index observations remove only pytest's changing temporary-directory prefix.

The [configured corpus](../fixtures/host/data-layer-port/batch-10-configured-defaults.json) records 61 observations from actual upstream reporters and discovery.
Both native APIs check these observations with shipped scopes and default options.
This gate covers ignored directories, unrelated files, parser messages, required documents, and reporter-specific filtering.
Direct source APIs have separate contracts where upstream behavior differs.
For example, the HNSW source API checks forbidden file existence before parsing; its configured plugin does not.
Embedding source observations include EVO107 and EVO108. The configured reporter preserves its narrower upstream failing-code set.

Rule decisions execute in SQLite over canonical syntax columns and text evidence.
CPython supplies syntax facts. It does not import or execute analyzed source.
The conditional rule reuses the existing shared conditional producer from batch six and selects the actual CONDA-005 findings.
The bounded-path rule preserves upstream text checks for Python, RST, and Markdown files, including duplicate missing-artifact findings.

Batch ten required these generic engine changes:

- `source_text(path)` reads a complete immutable file. Missing and empty files remain distinct.
- Evidence relations accept `file.content` regex queries with typed capture projections.
- `relational.line` can narrow a proven text span and retain line one for declared missing-file locations.
- Workspace text evidence retains its projected values and remains complete when an unrelated AST query encounters syntax errors.

Regression tests cover the engine changes before the rule replay gates.
The workspace test also checks edits and removal.
The existing source-work budgets, output limits, and SQL submission guard remain in place.

Matching initially repeated canonical preparation across detector clauses and exceeded the AST output budget on its repository test.
One shared producer now serves the detectors for each candidate file.
Its static-string relation also needed an indexed intermediate relation to avoid repeated scans of a recursive CTE.
The repository assertion retains its full original domain and its actual 140 selected source files.
No cumulative cold benchmark was run for this batch.

Generate the policies with:

```sh
python3.13 scripts/generate/packs/data_layer_batch_ten.py \
  --source /path/to/pinned-data-layer
```

Run unchanged assertions against the recorded oracle with:

```sh
python3.13 scripts/dsl/data_layer_ten/suite.py \
  --source /path/to/pinned-data-layer --repo /path/to/git-objects \
  --baseline fixtures/host/data-layer-port/batch-10-complete-api.json \
  --policies packs/data-layer/rules --mode snapshot --output /tmp/batch-ten.json
```

Repeat with `--mode workspace`. Add `--oracle` and omit `--baseline` to record original execution.
The fixture recorder never edits upstream test assertions or linter implementations.
Native replay needs CPython 3.13 on PATH.

Run the compact replay and configured-default gates separately:

```sh
npx vitest run test/conformance/data-layer/batch-ten.test.ts
npx vitest run test/conformance/data-layer/batch-ten-defaults.test.ts
npx vitest run test/engine/native/data-layer-ten/
```

The [assertion receipt](../fixtures/host/data-layer-port/batch-10-native-assertions.json) binds native runs, disabled-detector controls, and implementation hashes.
The [journal](data-layer-performance-journal.md) records the defects and verification results.
These batch gates do not certify all 100 rules together on the full repository.
The prior complete repository measurement still covers 80 rules.
