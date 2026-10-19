# Data layer rule migration assessment

Implementation update: the first thirty checks now have candidate policies and a native parity gate.
See the [first-batch report](data-layer-port-execution-plan.md) and [second-batch report](data-layer-batch-two.md) for the first two batches. The [third-batch report](data-layer-batch-three.md) covers the current implementation results and measurements.
The [engine feature ledger](data-layer-engine-gaps.md) links three demonstrated branch features under umbrella NER-1967.
Jeremy must approve every new Linear issue before creation.
The survey below describes the starting point before those ports.

The migration needs current behavior oracles, reusable repository facts, and measurements across the complete rule selection.
Rerail can express many structural checks today.
Its main gaps concern public-symbol queries, general relation queries, and policy artifacts.
The historical SQL implementation supplies useful lessons, but its timings do not measure the current Rerail engine.

## Compared revisions

| Source | Revision | Role |
|---|---|---|
| [Data-layer main](https://github.com/neralabs/nera-data-layer/commit/2b1504adc4291daaf080b2e932962b134da1a69a) | `2b1504adc4291daaf080b2e932962b134da1a69a` | Migration oracle, main as retrieved on October 19, 2026 |
| [PR 2539](https://github.com/neralabs/nera-data-layer/pull/2539) | `281f500d7c8d413f17c6a27de2f3c518bb45258a` | Historical declarative implementation |
| Rerail | `0ab3cedc` | Engine source in this checkout |

The PR remains open.
It combines ServerPlan changes with a declarative linter migration.
Its review describes extensive conflicts and distinct fact-store work that needs separate extraction.
The PR is unsuitable as the behavioral specification for this migration.

The static inventory contains these `LinterConfig` declarations:

| Inventory | Current main | PR 2539 |
|---|---:|---:|
| Declared checks | 142 | 104 |
| Enabled declarations | 141 | 104 |
| `hard-fail` | 115 | 84 |
| `ratchet` | 16 | 17 |
| `info` | 4 | 1 |
| `metric` | 7 | 2 |

There are 99 common names, 43 additions, and five removals.
All 99 common checks have different entry-script hashes.
A hash difference establishes source changes, not a behavioral difference.
These counts describe suite checks, not individual diagnostic codes or proven port coverage.
The disabled current declaration is `tstring_syntax`.

The removed names are `async_def_naming`, `declarative_direct_row_intent`, `fact_query_compliance`, `linter_fact_schema_shape`, and `test_bootstrap_strict`.
Their removal from the suite does not establish removal of their underlying constraints.
New checks include plan ownership, schema contracts, publication ownership, matching architecture, and read-through authority.
The inventory tool lists every declaration and its enforcement.

## What main retained

Current main retains the fact-first framework: versioned extractors, shared traversal for AST collectors, source triggers, caches, and project-derived facts.
It also retains the symbol graph and several storage providers.
Its default fact provider uses filesystem JSON.
The optional raw SQLite provider stores facts, but the suite still runs Python reporters.

Current main does not contain PR 2539's `lint_specs/` YAML catalog or its `fact_query/` compiler and executors.
Thus, a retained fact-storage provider does not imply retention of SQL-only rule evaluation.
Recent main also converts required-fact extraction errors into `FACT-EXTRACTION-FAILED` findings, with a syntax-error exception.
The migration must preserve failure visibility as well as findings.

Relevant current sources are [suite.py](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/suite.py),
[fact_engine.py](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/fact_engine.py),
and [symbol_graph.py](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_lib/symbol_graph.py).

## Capability assessment

These are implementation assessments from source review.
They are not a classification of every diagnostic inside all 142 checks.

| Requirement | Existing Rerail support | Remaining work |
|---|---|---|
| Local syntax and names | AST patterns, captures, ancestry, fields, boolean composition, regex, structural comparisons | Rule translation and differential fixtures. Usually no engine extension. |
| Lexical binding and imports | `binds`, `resolves`, binding sites, annotations, static values | Preserve shadowing, aliases, and context. Text matching alone loses this behavior. |
| Paired declarations and simple repository requirements | Projected AST `evidence`, `require_each`, `budget`, `select_when` | Use existing relations where sufficient. Local declaration pairs do not automatically require a new engine feature. |
| Calls through helper functions | Cross-file `calls`, inheritance-aware method linking, incremental reachability and consumer stamps | Prove the required symbol coverage. Ordinary typed parameters and dynamic receiver values can remain unknown. |
| Public API and inherited signatures | AST queries and call-linking internals supply some inputs | Expose reusable export identities, re-export closure, inherited ownership, normalized signatures, and annotation relationships to rules. |
| General relation programs | Two-relation requirements, grouped counts, and limited row predicates | No general DSL for named intermediate relations, arbitrary joins, recursive relations, derived columns, and conditional aggregation. |
| Signed exceptions and ratchets | Rule scopes, suppression comments, rule modes, recorded violations | Add explicit policy inputs, signature checks, baseline matching, expiry behavior, and their invalidation keys. |
| Repository governance | Source queries can inspect particular files | Checks over manifests, documentation coverage, runner registration, and artifact signatures need their actual repository inputs and reporting contracts. |
| Ordered effects and provenance | Binding/value queries, structural relations, and call reachability | Model the exact state tracked by the oracle. Reachability alone does not establish value flow or transaction ownership. |

### General relations

Rerail evidence projections currently expose captures and file identity fields.
Row predicates support `all`, `any`, `not`, `eq`, and `ne`.
The `define` clause shares AST utilities, rather than a catalog of materialized repository relations.

PR 2539's language supports joins, left joins, unions, recursion, derived expressions, classifiers, and aggregates.
A direct translation of that YAML therefore exceeds the current Rerail relation language.
This does not mean that each YAML operator needs a corresponding engine operator.
Many local queries can become cheaper AST patterns.

The engine extension with the broadest potential value is a typed relation interface over shared summaries.
It can expose definitions, imports, exports, signatures, and policy entries without exposing parser storage.
The port can then express repository constraints over those facts.
New relations need declared dependencies, bounded work, incomplete-result semantics, and precise invalidation.

Relevant Rerail sources are [code-rule.ts](../src/engine/code-rule/code-rule.ts),
[evidence.rs](../native/src/code_rule_dsl/eval/evidence.rs),
and [crossfile.rs](../native/src/code_rule_dsl/query/crossfile.rs).

### Symbols and value flow

The current data-layer `public_api_triples` check groups exported execution surfaces and inherited methods.
It considers decorators, ownership, and several architecture-specific exceptions.
A missing sibling declaration can use `require_each`.
Determining which inherited implementation is part of the public API needs additional reusable facts.

Rerail already models class bases and method receivers inside its call graph.
That graph does not expose a general public-symbol relation to policies.
Its Python call summary treats an ordinary parameter receiver as open, rather than inferring its class from an annotation.
Calls through returned objects, subscriptions, or other dynamic expressions also have limits.
These limits need explicit unknown outcomes rather than silent exclusions.

The plan-scope oracle also tracks assigned eager-read names before a branch in a write plan.
This behavior needs the same state and ordering rules as the oracle.
A broad dataflow system is not necessarily required for this particular check.

Relevant sources are [calls/summary.rs](../native/src/code_rule_dsl/calls/summary.rs),
[calls_state.rs](../native/src/code_rule_dsl/workspace/calls_state.rs),
and the current [public API check](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_public_api_triples.py).

## Existing examples are partial projections

The two `example-data-layer-*` documents are host fixtures.
They are not complete ports of the current checks.

The layer example searches for top-level `impl` imports with a text regex.
The current oracle also considers numeric layer order, API/implementation separation, conformance exceptions, groups, and type-checking context.
The regex cannot establish equivalent behavior.

The plan-scope example searches for resolution calls beneath two plan opener names.
Current main recognizes six opener names and several independent diagnostic families.
Eight synthetic probes exercise the current fact collector:

| Probe | Current oracle | Existing example regex projection |
|---|---|---|
| Resolution under `open_read_plan` | `PSE-020` | Match |
| Resolution under a provider write plan | `PSE-020` | No match |
| Raw database call inside a plan | `PSE-030` | No match |
| Legacy callback inside a plan | `PSE-040` | No match |
| Network side effect inside a plan | `PSE-050` | No match |
| Eager value access inside a plan | `PSE-010` | No match |
| Allowed execution through the plan | No finding | No match |
| Resolution outside a plan | No finding | No match |

These probes demonstrate missing coverage without an engine extension.
The comparison evaluates the example's regex conditions on these small inputs, not the native AST matcher.
It does not prove native parser or matching parity.
Full parity also needs branching, nesting, async scopes, suppressions, paths, and failure cases.

The inputs are in [data-layer-port-probes.json](../fixtures/host/code-rule-dsl/data-layer-port-probes.json).
The current oracle is [lint_plan_scope_side_effects.py](https://github.com/neralabs/nera-data-layer/blob/2b1504adc4291daaf080b2e932962b134da1a69a/scripts/lint_plan_scope_side_effects.py).

## Performance findings

### Historical SQL costs

PR 2539's [200-file performance plan](https://github.com/neralabs/nera-data-layer/blob/281f500d7c8d413f17c6a27de2f3c518bb45258a/docs/design/DECLARATIVE_LINTER_200_FILE_PERFORMANCE_PLAN.md)
records these development measurements:

| Phase | Query-fixes run | Eight-worker run |
|---|---:|---:|
| Extraction complete | 4.907 s | 5.300 s |
| Refresh | 8.730 s | 8.712 s |
| SQL/report | 47.757 s | 36.627 s |
| Large save batch | 130,663 writes, 3.112 s | 130,663 writes, 2.555 s |

These phase labels come from the historical plan.
Their boundaries can overlap, so their sum is not an established total.
The plan describes excessive fact volume and repeated broad SQL work as hypotheses for further measurement.
Its later commits narrow fields and physicalize relation rows.
The table is not a fresh measurement of the final PR tip.

The transferable lesson is demand-driven extraction with shared summaries and early candidate reduction.
Generic facts need not mean every AST field, text fragment, and child relation for every selected rule.
Moving Python decisions into SQL does not automatically reduce total work.

### Current Rerail costs

Rerail's production code-rule evaluator performs native regex and grammar matching.
The retired SQL outer engine and benchmark transducer are not its production evaluation path.
The historical SQL times therefore cannot establish a current Rerail bottleneck.

The following costs are visible in current source, but their contribution needs measurement:

1. **Matching across the selected clauses.** Rerail shares parsing, tapes, anchors, and scope tables, with candidate filters.
   Each eligible clause still performs its own matching work.
   Broad candidate kinds and repeated descendant queries can dominate after parsing becomes cheap.
2. **Global evidence reconstruction.** A dirty global check collects witnesses across its partitions.
   Relation evaluation projects those witnesses again.
   A small edit can therefore cause repository-wide row work for that check.
3. **Join shape.** `require_each` builds hash buckets from usable equality predicates.
   Without an extracted equality, it scans evidence candidates until a match or interruption.
   That fallback can approach subject-count times evidence-count.
   Even the hashed path rebuilds its buckets for each evaluation.
4. **Call summaries and propagation.** The call adapter already tracks graph fingerprints, affected consumers, and result stamps.
   It avoids treating every edit as a complete graph rebuild.
   High-fanout semantic changes can still affect many consumers.
5. **Scheduling.** Native asynchronous applies share a process-wide `cpu_slot`.
   A concurrent apply can return `WORKSPACE_BUSY`.
   File jobs use a four-thread worker pool, so additional workspaces do not multiply evaluation capacity.
6. **Results and storage.** Witness projection, result serialization, stored-result retrieval, and persistence add work beyond matching.
   Large finding sets need separate measurement from violation-free files.
   The engine already provides paged results and storage reuse.

The source evidence is in [workspace/global.rs](../native/src/code_rule_dsl/workspace/global.rs),
[query/crossfile.rs](../native/src/code_rule_dsl/query/crossfile.rs),
[workspace/evaluate.rs](../native/src/code_rule_dsl/workspace/evaluate.rs),
[workspace/api.rs](../native/src/code_rule_dsl/workspace/api.rs),
and [lib.rs](../native/src/lib.rs).

Existing per-clause and per-file work budgets can stop evaluation.
A faster run with more coverage failures is not a performance improvement.
Parity comparisons must include failures and unknown outcomes, not just violation counts.

### Required benchmark

The existing DSL microbenchmark uses one TypeScript pattern and a new workspace for each timed run.
It does not measure all data-layer rules or resident Python edit latency.
The existing [query parity harness](../benchmark/query-parity.ts) already supports explicit corpora, manifests, engine sides, and edit workloads.
It supplies a starting point for the migration benchmark.

The acceptance matrix needs these cases:

| Scenario | What it establishes |
|---|---|
| Cold process and empty store | Complete initial cost, including compilation and discovery |
| New workspace and populated store | Persistent reuse across sessions |
| Resident unchanged apply | Cost of no semantic change |
| One local function edit | Local matching and invalidation cost |
| Signature, import, export, or base-class edit | Correct downstream invalidation and propagation cost |
| File deletion or path change | Removal of stale facts and results |
| Complete selected catalog | Shared extraction demand and clause fanout |
| Many findings or incomplete files | Reporting cost and honest coverage |

Each result needs corpus and rule digests, file count, bytes, engine build, worker count, and cache state.
Measurements need wall time, worker CPU, parse/match time, graph work, relation rows, serialization, storage, memory, findings, and failures.
The corpus ramp can use 200 files, 1,000 files, and the complete applicable repository.
The original 200-file list is not available here, so a new manifest cannot reproduce the historical run exactly.

No fresh current-engine throughput claim accompanies this assessment.
The release build failed because the local safety hook blocked `rm` inside jemalloc configuration.
A cached addon in another worktree belongs to an older Rerail revision and cannot establish current-engine speed.

## Migration order

1. Freeze the current oracle revision and enumerate diagnostics inside each configured check.
   Preserve enforcement, path scope, exceptions, reporting, and extraction failures.
2. Port local checks with existing AST and binding operators.
   Compare positive cases, near misses, and repository findings against the current oracle.
3. Port simple cross-file requirements with `evidence`, `require_each`, and `budget`.
   Measure hashed joins and invalidation before adding relation operators.
4. Extract reusable symbol and signature facts for the repository checks that need them.
   Define unknown behavior for unresolved exports and receivers.
5. Add signed policy artifacts and ratchet semantics before replacing CI enforcement.
   Rerail's `block` mode currently delivers guidance like `steer`, so it does not replace a CI hard-fail contract.
6. Add relation operators only for demonstrated gaps in the current rules.
   Share demand, summaries, indexes, and dependency tracking across the selected catalog.
7. Run the complete catalog in shadow mode before replacing the data-layer runner.
   Admit a check as exact only after its diagnostics and coverage match the oracle.

This assessment adds an inventory and oracle probes.
It does not install a rule pack, change engine behavior, or replace any data-layer check.

## Reproduction

The inventory needs Git objects for both pinned revisions.
It imports no source-repository modules and writes JSON to standard output.

```sh
python3 scripts/dsl/data_layer_rule_audit.py \
  --repo /path/to/nera-data-layer > /tmp/data-layer-rule-audit.json
```

For the oracle probes, the Python environment needs data-layer dependencies.
The oracle source must contain all Python scripts from the pinned main revision and the production packages they import.
The tool compares every Python script against the Git revision before it imports the oracle.
It also rejects a checkout at another commit or with modified tracked scripts.

```sh
/path/to/data-layer/.venv/bin/python scripts/dsl/data_layer_rule_audit.py \
  --repo /path/to/nera-data-layer \
  --oracle-root /path/to/pinned-data-layer-source \
  > /tmp/data-layer-rule-audit-probes.json
```

`--current` and `--historical` accept explicit Git revisions for later comparisons.
Source signals in the JSON guide review and do not certify expressibility.
