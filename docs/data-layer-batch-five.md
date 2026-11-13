# Data-layer fifth batch

All ten fifth-batch checks have native candidate policies.
This brings the authored candidates to 50 of 142 checks.
The private pack remains inactive; `packs/data-layer/pack.json` has an empty `rules` list.
No Linear issues were created or amended.

The source pin remains `2b1504adc4291daaf080b2e932962b134da1a69a` in `neralabs/nera-data-layer`.
The [evidence file](../fixtures/host/data-layer-port/batch-05-evidence.json) records policy hashes, coverage, measurements, and remaining integration contracts.

## Rules and parity

| Check | Behavior exercised |
|---|---|
| `missing_async_pairs` | Companion names, parameter counts, parameter order, and exemptions |
| `sync_async_signatures` | Transaction and connection modes in paired signatures |
| `async_sync_delegation` | Thin wrappers, helper families, decorators, and empty bodies |
| `public_api_triples` | Exports, inherited public methods, family slots, decorators, exemptions, and deduplication |
| `data_layer_facade_purity` | Facade imports and source structure |
| `data_layer_compat_aliases` | Compatibility aliases and permitted forms |
| `layer_violations` | Layer ordering, import modules, paths, and typing-only exemptions |
| `backend_coupling` | Backend boundaries and configured exceptions |
| `internal_dead_code` | Export roots, references, annotations, modules, inheritance, protocols, callbacks, and diagnostic categories |
| `imperative_private_functions` | Direct execution, eager signatures, and declared-reference propagation |

The unchanged upstream suite passes 646 tests.
Its recorder preserves 239 analyzer observations.
Of these, 124 observations have source inputs suitable for native evaluation.
Seventy public-triple observations use synthetic graph models; 44 imperative observations also use synthetic graphs.
One delegation observation exercises I/O behavior.
These 115 observations remain separate and do not count as native passes.
Other original tests exercise helpers without a supported analyzer boundary.

The combined native source gate compares 312 calls: 198 positive and 114 clean.
Both snapshot and workspace evaluation match the oracle, with no failures or execution-path differences.
The comparisons include code, relative path, line, multiplicity, and recorded category or backend column.
Every check has positive and clean coverage.
The supplemental probes retain unchanged oracle outputs and verified source identities.
The public-triple rule has positive probes even though the repository detector reports no violations.

The full repository gate compares 5,914 files and 106,201,417 UTF-8 bytes.
All ten detectors match 23,835 findings, with zero false positives, false negatives, or engine failures.
Unchanged-file resends and a trailing-newline edit also match the oracle.
The harness now checks their complete paged results, as well as measuring their time.

## Engine capabilities driven by the ports

### Declared references

The [imperative analyzer](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_imperative_private_functions.py#L174) propagates through the source graph's callers.
The source graph includes callbacks and returned functions, beyond direct calls.
Python `calls` queries now support `graph: "references"` with `at: "definition"`.
This mode preserves ordinary call-query behavior and uses declaration semantics for its own graph.

The [source resolver](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L1970) motivates the implemented relation shapes:

- Declared imports, reexports, final function definitions, and module method inventories.
- Callbacks, returned functions, nested function bodies, and declared parameter receivers.
- Declaring-class methods, breadth-first base traversal, `super`, and composed mixin lookup.
- Constructor parameter types and direct class-field annotations.
- Authored decorator-to-sibling links, including inherited siblings.

`calls.siblingLinks` declares decorator patterns and sibling-name rewrites.
`calls.constructorFields.fieldToParameter` declares a field-name rewrite for constructor parameter lookup.
These contracts are bounded, validated, hashed, and isolated in graph caches.
No native detector contains data-layer decorator names or naming conventions.
The policies supply those values from the pinned source constants.

The source graph deliberately ignores several runtime relations.
The port preserves its behavior for assignment aliases, quoted receiver annotations, positional-only annotations, conditional field annotations, and indirect module-valued reexports.
The regression corpus includes positive cases and controls for these distinctions.
Changes to callees, import inventories, and class-name uniqueness invalidate unchanged callers.

### Project relations and recursive queries

The [public-triple analyzer](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_public_api_triples.py#L499) groups exported functions and inherited public methods.
It compares family slots, parameter lists, decorator requirements, and inherited implementation identities.
A local AST predicate cannot represent those project-wide joins and groups.

The [dead-code walk](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L2569) adds recursive relations.
Its roots include public exports, imported module initialization, and production dynamic imports.
It follows references, annotation classes, public methods, dunders, subclasses, mixins, protocols, and convention-named siblings.
A second annotation expansion has different propagation rules.
A class can produce two diagnostic categories at one source span.

The engine now supports a general `relational` clause:

- AST evidence supplies named tables with scalar projected fields and exact source anchors.
- Ordered, materialized SELECT stages supply joins, groups, windows, indexes, and recursive CTEs.
- An optional native `reference_edges` table supplies declared-reference edges.
- Final rows select input anchors, diagnostic identity fields, and optional diagnostic codes.
- Suppression retains facts and removes only covered output findings.
- Incremental evidence changes and semantic graph changes invalidate dependent global clauses.

Queries cannot access files, metadata, writes, pragmas, attached databases, or nondeterministic functions.
Compilation checks every query against empty schemas before reading source.
Evaluation enforces input completeness, stable identities, source-anchor validity, work limits, and interruption checks.
The work allowance counts input rows, regex text chunks, and calibrated SQLite instruction groups.
The [DSL reference](code-rule-dsl-reference.html#relational-clause) documents the contract.

## TDD ladders and root causes

The port progressed through reduced source cases, native execution-path comparisons, incremental edits, and the complete repository oracle.
The [graph corpus](../fixtures/host/data-layer-port/batch-05-remaining-graph-probes.json) contains 51 declaration-resolution cases.
The [relational corpus](../fixtures/host/data-layer-port/batch-05-relational-probes.json) contains 68 family and reachability cases.

| Failure | Root cause | Correction and regression |
|---|---|---|
| Imperative propagation missed repository callers | Runtime call semantics differed from the source's declaration graph | Separate reference mode; callback, import, annotation, inheritance, and sibling-link ladders |
| Declaration matching exceeded work limits | Declaration-only queries built usage indexes and rescanned whole classes | Cached name-site lookup; a 700-method class passes at existing limits |
| Utility compilation repeated capture analysis | Shared utilities were summarized repeatedly | Reuse utility summaries; the linear-chain regression passes |
| Family queries exhausted SQL work | Recursive name resolution used an unfavorable join order; export lookups rescanned literals | Indexed stages, bounded statistics, explicit worklist joins, and filtered execution-call inputs |
| Module initialization exhausted SQL work | Every identifier used repeated declaration-span scans | Capture AST contexts once; use indexed declaration identities |
| Dead-code parity had 42 extra and one missing diagnostic | Plain imports vanished through SQL null arithmetic; multiline bases failed normalization; module aliases added a false edge | Import-root, multiline-base, conditional-class, and reexported-module regressions |
| Dynamic-import probes disagreed | Standalone analysis and the production plugin use different extra roots | Preserve the entry-point distinction; test both paths against the original functions |

No oracle source, expected findings, or existing AST work limit changed to obtain parity.
Generated policies remain identical across Python hash seeds.

## Speed findings

| Measurement | Result |
|---|---:|
| Shared Python oracle, one worker, no fact cache | 47.00 s |
| Empty native workspace, four workers | 168.78 s |
| Native policy compilation | 1027.9 ms |
| Unchanged-file resend, including complete result comparison | 1577.3 ms |
| Trailing-newline edit, including complete result comparison | 32.25 s |

The native workspace is slower than the Python oracle in this sample.
The earlier concurrent-validation run took 204.90 seconds cold and 38.52 seconds for the edit.
The measured dead-code edge stage takes 18.12 seconds; its recursive dispatch stage takes 6.20 seconds.
These stage timers exclude AST extraction, input insertion, graph linking, and result publication.
The remaining wall time cannot establish one narrower bottleneck.
Unchanged input does not rerun SQL stages.
The whitespace edit reruns both project plans.

The original Python run uses one worker; the native workspace uses four workers.
The native release build enables profiling, thin LTO, and 256 code-generation units.
It uses the existing system-allocator shim.
These shared-machine samples do not establish production throughput or a stable speed ratio.

The two project rules add substantial cost.
The public-triple candidate gathers declaration and annotation tables before grouping families.
The dead-code candidate also gathers module-initialization facts and computes recursive reachability.
Both rebuild materialized SQL stages when their input facts change.
A small source edit can therefore cause a project-wide query rerun.

Profiling identified concrete costs, rather than a missing detector feature:

1. Recursive joins require indexes and selectivity statistics to avoid repeated scans.
2. Raw execution-call collection originally retained 388,945 rows; filtering at extraction removes unrelated calls.
3. Module initialization originally used repeated interval scans; captured contexts reduce that stage to indexed lookups.
4. Generic relational inputs repeat AST extraction across rule documents.
5. SQL materialization and recursive dispatch remain visible costs after parity corrections.

Further speed work should measure shared source-fact extraction and incremental relational maintenance against this passing gate.
It must preserve positive source cases, repository findings, edit results, and existing resource failures.
Performance work does not justify a new Linear feature issue.

## Remaining integration contracts

These policies establish source-detection parity for the recorded corpus and pinned repository.
They do not certify every synthetic graph helper or every possible Python program.
The 114 retained graph-model observations still require a separate model adapter.
They remain visible in the original recording.

The signed category/total reporter for `async_sync_delegation` remains a separate integration contract.
The public-triple recorder now compares raw analyzer detections and retains the signed reporter output separately.
The private pack remains inactive pending migration integration.

## Validation

All 75 TypeScript regressions pass across eight files, including all data-layer conformance gates.
All 1,347 native DSL tests pass; 22 tests remain ignored.
Typechecking, repository checks, native formatting, Clippy, reference grammar, and licensing checks pass.

## Reproduction

Use the pinned source tree and its Python dependencies:

```sh
python scripts/dsl/data_layer_record.py --batch 5 --source SOURCE --repo REPO --output ORIGINAL.json
python scripts/dsl/data_layer_generate_batch_five.py --source SOURCE --repo REPO
npx vitest run test/conformance/data-layer/batch-five.test.ts \
  test/engine/native/data-layer-batch-five-capabilities.test.ts \
  test/engine/native/relational-evidence.test.ts --maxWorkers=1
```

Regenerate each supplemental recording with `data_layer_probe_five.py`, using its corresponding `*-inputs.json` file.

```sh
python scripts/dsl/data_layer_suite_record.py --batch 5 --source SOURCE --repo REPO \
  --workers 1 --output REPOSITORY.json
node --import tsx conformance/data-layer/suite.ts \
  --recording REPOSITORY.json --output RESULT.json --allocator ALLOCATOR
```

The complete source snapshots and repository result files remain under `/tmp` because of their size.
The committed evidence file contains their measurements and identities.
