# Fifth data-layer batch: work in progress

The source pin is `2b1504adc4291daaf080b2e932962b134da1a69a`.
This batch is incomplete. It contains eight candidate policies for the next ten checks.
`public_api_triples` and `internal_dead_code` have source probes but no candidate policy.
`imperative_private_functions` passes the small corpus but fails repository parity.
No Linear issues were created.

## Evidence

The unchanged upstream suite passes all 646 tests.
The recorder observes 239 calls and preserves synthetic graph inputs separately.
Those inputs do not count as native passes.
Many upstream tests exercise helpers rather than complete analyzers; the recording lists these gaps.

The eight candidate policies pass 180 native comparisons: 94 positive and 86 clean.
These comparisons cover diagnostic codes, paths, lines, multiplicity, and agreement between execution paths.
The corpus includes parameter order, annotations, imports, exceptions, decorators, aliases, and real Python source graphs.
The full batch has 76 supplemental source cases. Thirteen cases cover the two unported graph checks.
Twelve further cases test declared references. Seven cases preserve unresolved graph behavior and its controls.
The unresolved cases are outside the passing subset and currently produce three false negatives.

The repository measurement covers 5,914 files and 106,201,417 source bytes.
Seven detectors match the repository oracle.
The imperative candidate has 26 false positives, 1,904 false negatives, and no work-limit failures.
Every diagnostic mismatch concerns `calls_imperative`; direct execution and eager signature detection match.
The [evidence](../fixtures/host/data-layer-port/batch-05-evidence.json) records the current results and the earlier checkpoint.

The Python shared engine takes 34.6 seconds with one worker.
The native workspace takes 15.4 seconds with four workers, plus 516 milliseconds for compilation.
An unchanged file takes 31 milliseconds; an edited file takes 73 milliseconds.
These timings measure incomplete semantics. They do not establish a speedup with parity.
The native build uses the profiling feature and the existing system allocator shim.

## Engine changes

The rebuilt addon supports these reduced cases:

- Evidence projections can extract a regular-expression group from a string field.
  Ordered evidence predicates compare strings in UTF-16 ordinal order.
  The layer check uses four-digit layer numbers from paths and import modules.
- Source-text comparisons can read exact relation counts.
  The missing-pair check compares positional parameter counts and corresponding parameter names.
- AST rules can test `filePath`, including within a call-graph seed.
  The imperative check seeds functions from architectural paths.
- Python supports `calls: {graph: "references", at: "definition", ...seed}`.
  It follows callbacks, returned functions, imported declarations, and methods selected through declared parameter types.
  Method references resolve on the declaring class and its bases.
  Ordinary call queries retain their existing dispatch behavior.

Invalid extraction groups, invalid patterns, unknown keys, and excessive pattern lengths fail compilation.
The layer regression checks snapshot evaluation, workspace evaluation, and an edit that removes a finding.
Reference tests also change a callee while preserving the caller file, and check that its finding changes.

## Porting and speed findings

The recorder originally reduced some source paths to basenames.
It now preserves relative paths and paths within the pinned source tree.
The test suite was rerun after that correction.

The delegation port initially used 29 AST traversals.
A large upstream file charged approximately 3.7 million nodes and exceeded the snapshot evaluator's work limit.
Each clause passed in isolation.
Combining mutually exclusive checks reduced the policy to 13 traversals without changing the limit.
Factoring repeated expressions into shared utilities reduced source-corpus replay from approximately 30 seconds to three seconds.
Both execution paths then passed.

Repository replay exposed further policy defects that small probes missed:

- Production roots include top-level `l####_*` packages and `_internal`.
- A docstring-only async body is an allowed empty body.
- Method signature analysis excludes `self` and `cls` annotations.
- The source graph keeps the final function definition, including after overload declarations.
- Property setters and deleters do not seed eager signature findings.

These defects were corrected. The remaining imperative mismatches concern graph propagation.

The first declaration-selection optimization caused 131 work-limit failures.
It built the full Python usage index for a declaration-only question.
It also counted matching class definitions across the module for every method.
Declaration-only binding queries now use a cached name-site index and avoid unknown-value matching when their filters need no usage facts.
The policy checks the class name's binding chain instead of rescanning the module.
A 700-method class, including an overwritten definition, passes both execution paths at the existing limit.
The repository no longer has work-limit failures. The limit was not increased.

The full native suite exposed repeated capture analysis during compilation.
The compiler now shares utility summaries with capture ordering, restoring the linear utility-chain bound.
Stale count-value, parser-vocabulary, and budget-test expectations were updated.

Policy generation also sorts constant sets before lowering them.
The eight outputs are identical under two different Python hash seeds.

## Remaining graph work

The [imperative analyzer](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_imperative_private_functions.py#L174) propagates findings through `graph.callers`.
Its graph includes function references passed as arguments or returned as values.
The declared-reference mode now handles those shapes, including declared receiver dispatch.
Reduced oracle probes also distinguish assignment aliases, quoted annotations, and positional-only parameters.
All twelve probes pass in snapshot and workspace evaluation.

Three further source pairs prove unresolved propagation behavior:

| Required relation | Upstream implementation | Reduced case |
|---|---|---|
| Decorator-generated delegation to a named sibling, including inherited siblings | [Legacy decorator edges](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L2262) | `decorator-generated-edge` |
| Method selection through an annotated instance field and its declaring hierarchy | [Chained receiver resolution](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L2022) | `typed-field-edge` |
| Method selection through the receiver's base classes | [Super resolution](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L2011) | `super-method-edge` |

The [remaining-graph corpus](../fixtures/host/data-layer-port/batch-05-remaining-graph-probes.json) records each positive case and a control without that edge.
Native replay misses exactly the three propagated findings and adds no findings on the controls.
These cases do not explain every remaining repository difference.
They establish concrete requirements for further porting, without creating Linear tickets.

The [public-triple analyzer](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_public_api_triples.py#L499) groups exported functions and inherited public methods.
Its requirements include export aliases, inherited method selection, family slots, parameter agreement, and diagnostic deduplication.
The source probes exercise exported functions, exported classes, inherited methods, and missing decorators.

The [dead-code graph walk](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py#L2569) starts from public exports.
It follows references, annotations, class uses, inheritance, protocol methods, convention-based siblings, and module-level references.
Source probes cover unused symbols, exported roots, calls, callbacks, annotations, and implicit methods.
A call-only graph cannot establish parity for this shape.

The signed category/total baseline reporter for `async_sync_delegation` also remains a separate integration contract.
The suite recorder preserves its upstream output while comparing its raw source detections.

## Validation

All 1,347 native DSL tests pass; 22 tests are ignored.
The 51 TypeScript regressions pass, including seven fifth-batch capability tests.
Typechecking, repository checks, native formatting, Clippy, and reference grammar checks pass.
The unresolved corpus remains a failing parity check with three false negatives.

## Reproduction

Use the pinned source tree and a Python environment with its dependencies:

```sh
python scripts/dsl/data_layer_record.py --batch 5 --source SOURCE --repo REPO --output ORIGINAL.json
python scripts/dsl/data_layer_probe_five.py --source SOURCE --repo REPO \
  --inputs fixtures/host/data-layer-port/batch-05-probe-inputs.json --output PROBES.json
python scripts/dsl/data_layer_probe_five.py --source SOURCE --repo REPO \
  --inputs fixtures/host/data-layer-port/batch-05-graph-probe-inputs.json --output GRAPH.json
python scripts/dsl/data_layer_generate_batch_five.py --source SOURCE --repo REPO
npx vitest run test/conformance/data-layer/batch-five.test.ts \
  test/engine/native/data-layer-batch-five-capabilities.test.ts --maxWorkers=1
```

Replay the unresolved cases separately; this command currently exits with three false negatives:

```sh
node --import tsx conformance/data-layer/replay.ts \
  --recording fixtures/host/data-layer-port/batch-05-remaining-graph-probes.json --output REMAINING.json
```

`data_layer_suite_record.py` now accepts batch 5 and an explicit `--checks` subset.
It includes the symbol survey's project inputs, beyond its sentinel file.
The current repository snapshot and native result files remain under `/tmp`; they are too large for the fixture corpus.
