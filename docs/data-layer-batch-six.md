# Data-layer sixth batch

This batch ports ten linter configurations to native candidate policies.
The private pack remains inactive.
Its manifest still has an empty `rules` list.
No Linear issues were created or amended.

The source pin is `2b1504adc4291daaf080b2e932962b134da1a69a` in `neralabs/nera-data-layer`.
The [evidence file](../fixtures/host/data-layer-port/batch-06-evidence.json) records the final policy identities and validation results.

| Check | Ported behavior |
|---|---|
| `serverplan_bypass` | Direct bypass methods, scope, and findings grouped by method and line |
| `serverplan_steps` | Step structure, callbacks, materializers, builders, return contracts, legacy forms, and exceptions |
| `serverplan_state_tripwires` | State signatures, value propagation, authority and provider sinks, return kinds, and project helpers |
| `serverplan_dispatch_architecture` | Dispatch boundaries, backend dependencies, forbidden literals, and permitted entry points |
| `serverplan_quality` | Quality clauses, runtime annotations, payload taxonomy, and shared conditional architecture |
| `serverplan_target_architecture` | Required files and contracts, forbidden surfaces, and missing-owner diagnostics |
| `execution_authority_bypass` | Execution authority boundaries and permitted contexts |
| `impl_direct_execution` | Direct execution calls, scopes, exemptions, and the FTS-specific analyzer |
| `raw_execute_regression` | Removed calls and definitions, compatibility stubs, and whole-file text markers |
| `legacy_execution_surfaces` | Removed Python and text surfaces, path restrictions, and diagnostic identities |

## Corpus and comparison

The unchanged upstream suite passes 933 tests, including 14 tests from the shared conditional analyzer.
The recorder verifies 1,521 loaded source files against the pinned Git blobs.
There are no source mismatches.

The original tests produce 1,234 source observations: 405 positive and 829 clean.
Twenty-five supplemental observations add twelve positive and thirteen clean cases.
The combined source gate compares 1,259 observations: 417 positive and 842 clean.
The native gate compares path, code, line, and finding multiplicity on both snapshot and workspace execution.
Every linter has positive and clean coverage.

The supplemental cases use the original analyzer to distinguish these shapes:

- Bare `Callable` and qualified `typing.Callable` annotations.
- Repeated callback fields and constructor callback parameters.
- Augmented assignments and annotation-only fields.
- Strategy files with 1,500 and 1,501 lines.
- Executor annotations in tuples, collections, unions, and supported wrappers.
- Metaclass keywords compared with actual step bases.
- Production plugin directories, test-directory exclusions, and source triggers.

The oracle results are recorded without modifying their expected findings.
The corpus preserves seven observations that require I/O or behavioral reporter adapters.
The recorder also lists 324 original tests without a captured analyzer boundary.
These observations and tests do not count as native passes.
Signed allowlists, generated-artifact freshness, CLI behavior, and aggregate reporters remain integration contracts.

The [incremental harness](../conformance/data-layer/incremental.ts) selects compatible positive and clean observations mechanically.
For each linter, it applies clean, positive, clean, and positive inputs to one resident workspace.
It checks complete paged results after each transition, including removed files.
All 40 transitions must match the source oracle.

## Engine capabilities driven by the ports

The [target analyzer](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_serverplan_target_architecture.py#L599) reports required contracts when their files are absent.
The [conditional analyzer](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/conditional_architecture_facts.py#L526) also reports missing canonical owners.
Relational plans can now declare normalized diagnostic paths in `locations`.
Only declared paths can produce diagnostics without an input anchor, and their byte offsets must both be zero.
Undeclared paths and invalid offsets fail evaluation.

The state and quality ports require typed syntax relationships and exact source ranges.
Projected captures now support `.kind` and `.endByte`.
Regex capture positions also work when several captures belong to one AST node.
Relational input coordinates use the selected evidence anchor.
The native tests cover parent joins, regex positions, selected anchors, and invalid output locations.

Most clauses analyze each file independently.
Their relational plans now declare `partitionBy: "file"`.
The workspace caches final findings by source and context version, then reevaluates changed files.
File partitions cannot declare project references or virtual diagnostic locations.
The [project state API](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_serverplan_state_tripwires.py#L3679) and conditional contracts keep repository partitions.

The [runtime annotation analyzer](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_serverplan_quality.py#L3590) needs lexical imports, shadowing, quoted annotations, and metadata exclusions.
Authored SQL stages represent those relationships.
The [taxonomy analyzer](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_serverplan_quality.py#L3035) combines recursive shapes and feature counts.
Bounded AST predicates supply those counts before project grouping.
These rule semantics remain in policies and authoring scripts.
The native engine contains no data-layer detector names or semantic allowlists.

## TDD ladders and root causes

Each ladder starts with an observed source mismatch or resource failure.
Reduced cases precede source-suite replay, edit transitions, and repository evaluation.

| Failure | Root cause | Correction |
|---|---|---|
| Deep stage plans failed compilation | Validation expanded earlier stages as nested views | Validate empty materialized tables; a 20-stage self-join regression passes |
| Missing target files could not report | Output anchors required an existing source row | Declare bounded diagnostic locations and test incremental repairs |
| Regex projections lost offsets | Same-node capture groups lacked exported span entries | Resolve capture ranges from their node groups |
| Callback cases lost or added findings | Qualified names, repeated fields, and augmented stores have distinct source behavior | Preserve AST shape, multiplicity, and assignment kinds |
| Conditional backend controls were missed | The sparse graph omitted condition roots | Include the control expressions and rerun all shared analyzer cases |
| Repository analysis retained excessive syntax | Local clauses stored project-sized evidence; immutable projections were cloned | Cache local results, share projections, and discard SQL-only capture spans |
| A large step file exceeded its witness bound | One syntax input combined too many node kinds | Split independent typed inputs within the existing input and witness limits |
| Name and scope stages exhausted SQL work | Import and function interval joins scanned unrelated nodes repeatedly | Materialize import ranges and derive owners through parent edges |
| Branch analysis exhausted SQL work | Each branch rescanned unrelated execution calls | Filter execution facts and index the bounded branch lookups |
| Executor provenance exhausted SQL work | Every pair of name uses repeated binding queries | Materialize binding facts once per name, then join them |
| A tuple return type tainted unrelated values | Annotation matching accepted any nested executor type | Follow only the wrappers accepted by the source analyzer |
| Legacy diagnostics appeared outside production scope | Standalone analyzer scope exceeded the production plugin scope | Use the plugin directories and source triggers; test both paths against the oracle |
| Step signature checks included the abstract base | A metaclass keyword matched the base-name pattern | Inspect positional bases and preserve the metaclass control |
| Large generated modules exceeded string and syntax bounds | Broad inputs retained unrelated literals and combined node families | Split typed inputs and literals, materialize bounded unions, and filter unused string values |
| State SQL exceeded work after input collection succeeded | Ancestor closure included wrappers unused by consumers | Keep self rows and omit wrapper-root expansion; preserve expression and diagnostic depth behavior |
| State owners used quadratic containment joins | Each function compared its span with every node | Carry function and class ownership through the syntax graph |

The engine also reuses parsed tapes within one evaluation.
An edited source cannot reuse the old tape.
The regressions verify both reuse and invalidation.

No existing AST, witness, SQL stage, input, or work limit was increased.
Generation must produce identical policies across Python hash seeds.

## Validation and performance

The source gate passes all 1,259 observations on snapshot and workspace execution.
All 40 resident-workspace edit transitions also pass.
The 71-file repository regression covers every path with an observed extra finding or incomplete input, plus the required conditional owners.
Its four expected findings match, with no extra findings, missing findings, engine failures, or edit differences.

All 1,349 native DSL tests pass; 22 remain ignored.
The selected TypeScript regressions pass 265 tests across seven files, including all six data-layer source gates.
Typechecking, repository checks, native formatting, Clippy, reference grammar, and license checks pass.

All ten checks match the full repository's 173 findings across 9,010 files and 137,896,473 UTF-8 bytes.
There are no extra findings, missing findings, engine failures, or edit differences.

| Measurement | Result |
|---|---:|
| Shared Python oracle, one worker, no fact cache | 70.35 s |
| Empty native workspace, four workers | 951.00 s (15m 51s) |
| Unchanged-file resend with complete result comparison | 929.2 ms |
| Trailing-newline edit with complete result comparison | 14.73 s |
| Peak process RSS | 11.97 GB (11.14 GiB) |

The native cold scan remains about 13.5 times slower in this sample.
The edit reruns project contracts, although file partitions reuse unchanged local results.

The final measurements and policy hashes are in the evidence file.
The Python oracle uses one worker and disables its fact cache.
The native release build enables profiling, thin LTO, and 256 code-generation units.
Its allocator is system malloc through the existing jemalloc symbol shim.
The harness records this allocator explicitly.
Source replay and other validation overlapped the beginning of the final repository scan.
These samples do not establish a stable production speed ratio.

A one-second CPU sample during the final scan found 733 of the active worker's 760 samples in relational evaluation.
Of these, 716 included SQL stage materialization.
Three other libuv workers waited during that interval.
This supports the observed cost of serial SQL materialization and index construction for that interval.
It does not attribute the whole scan to one stage.

The workspace shares parsed source, but these policies still extract overlapping syntax and rebuild SQL tables for separate plans.
File partitions cache final findings for subsequent edits.
They do not share all intermediate relational tables between clauses.
Further speed work should measure shared fact extraction and parallel execution of independent file plans against the passing corpus.
These are performance improvements to working rule representations, not unsupported engine features.

## Reproduction

Use the pinned source tree and its Python dependencies:

```sh
python scripts/dsl/data_layer_record.py --batch 6 --source SOURCE --repo REPO --output ORIGINAL.json
python scripts/dsl/data_layer_generate_batch_six.py --source SOURCE --repo REPO
python scripts/dsl/data_layer_probe_six.py --source SOURCE --repo REPO \
  --inputs fixtures/host/data-layer-port/batch-06-probe-inputs.json --output PROBES.json
node --import tsx conformance/data-layer/replay.ts \
  --recording ORIGINAL.json --recording PROBES.json --output SOURCE-RESULT.json
node --import tsx conformance/data-layer/incremental.ts \
  --recording ORIGINAL.json --recording PROBES.json --output EDIT-RESULT.json
python scripts/dsl/data_layer_suite_record.py --batches 6 --source SOURCE --repo REPO \
  --workers 1 --output REPOSITORY.json
node --import tsx conformance/data-layer/suite.ts \
  --recording REPOSITORY.json --output REPOSITORY-RESULT.json --allocator ALLOCATOR --progress
```

Profiling builds emit large matcher descriptions.
The source gate captures this output in a file instead of the test runner's nonblocking stderr pipe.

The committed [source gate](../test/conformance/data-layer/batch-six.test.ts) replays the original observations and supplemental cases.
The complete repository snapshot remains outside the repository because of its size.
