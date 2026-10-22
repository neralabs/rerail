# Transition to direct relational SQL

Status: Milestone 1 complete; Milestone 2 structural components implemented and measured; remaining gates pending, 2026-10-22.
Baseline: `a0051ae8`, including the complete B023 reference query.
This document changes no production behavior.

Implementation progress: Milestone 1 now passes on the frozen corpus.
The compiler lowers the original full B023 `GrammarMatcher` to one statement, without a policy recognizer or native composition callback.
The [complete evidence](../benchmark/sql-relational-complete-evidence.json) records exact native/current-engine parity, generated SQL, EXPLAIN, source hashes, and paired CPU samples.
The [generated SQL](../benchmark/sql-relational-complete.sql) is available separately for review.
All 138 file hashes match the original reference corpus.

The complete query returns the exact two roots and capture witnesses.
It uses 2,812,255 VM instructions, one traced matching execution, and zero frame publications.
Median cold kernel CPU is 125.553 ms, versus 63.349 ms for the handwritten reference.
The 1.982 ratio passes the twofold CPU gate narrowly.
Preparation remains the largest gap: 51.560 ms versus 2.916 ms.
These measurements include counters and exclude parsing and diagnostic queries.

Outside-region assignments add no SQL work: sizes 32, 128, and 512 each use 2,999 VM instructions.
Combined assignment/load sizes use 44,248, 170,392, and 674,968 instructions.
Renamed utilities and captures, changed loop kinds, and changed pattern leaves retain exact native parity.
The compiler also passes the upstream fixtures and 22 adversarial snippets.

Earlier evidence remains available for the [target-slot](../benchmark/sql-relational-target-slot-evidence.json), [region-demand](../benchmark/sql-relational-region-demand-evidence.json), and [capture-demand](../benchmark/sql-relational-capture-demand-evidence.json) stages.
The complete lowering adds value-keyed exclusions, downward parameter recursion, body expansion, indexed load probes, and final ordered capture choices.
Equivalent probes and regional recursive producers share answers.
Pure root/value constraints can move through choices; witness selection retains its original boundary.

Milestone 2 now includes positive structural components, with native and current-engine parity.
The compiler emits one recursive CTE for mutually recursive utilities, keyed by `(utility, file, node)`.
Self-recursion uses the same termination proof and retains its narrower `(file, node)` CTE.
Parent, child, and sibling steps are supported when each component advances in one strict preorder direction.
The [component evidence](../benchmark/sql-relational-components-evidence.json) includes six scaling cases, EXPLAIN, exact outputs, and source hashes.
The [generated component SQL](../benchmark/sql-relational-components.sql) shows the three-state diamond query.
All six kernels use one traced matching statement, zero frame publications, and zero automatic-index rows.

Unknown outcomes, reached failures, enumeration, parameterized witnesses, and general recursive components remain pending.
Native work and stack/depth budgets also remain unresolved.
The compiler remains test-only until the semantic coverage and production cutover gates pass.
The complete-job 100-execution gate remains a separate requirement.

There will be one production compiler after the transition.
The replacement will develop through a test harness until it covers the current engine's supported semantics.
Then one production cutover will replace the existing compiler and scheduler.
There will be no per-clause fallback, user switch, or production shadow execution.
This choice delays production gains until the coverage gate passes, but prevents a permanent mixed architecture.
Intermediate commits will still provide executable measurements through the replacement compiler.

## Evidence and limits

The [B023 evidence](../benchmark/sql-b023-relational-evidence.json) compares identical inputs and exact root and capture outputs.
Both kernels use one matching SQL statement across 138 files.

| Measurement | Current compiler | Direct reference |
| --- | ---: | ---: |
| Median kernel CPU, three samples | 4,616.048 ms | 49.230 ms |
| VM instructions | 369,851,302 | 2,324,796 |
| Frame publications | 6,944,829 | 0 |
| Node advances | 3,745,416 | 81,315 |
| Matching SQL bytes | 278,847 | 8,196 |

The CPU improvement is approximately 94-fold for this kernel.
It does not predict a 94-fold whole-job improvement.
The kernel measurements exclude parsing, which costs a further median 717.679 ms for these files.
The direct query still creates 37,241 automatic-index entries.
Removing frame publication does not remove all intermediate storage.

The reference covers total structural matching, capture unification, negation, ordered choices, and structural recursion.
It does not prove unknown propagation, enumeration, or general recursive rule semantics.
Those features have explicit gates before production cutover.

## Replacement interfaces

The implementation now lives in `native/src/code_rule_dsl/sql/relational/`.
The table below describes the target interfaces.
Current ordered answers and negative-probe closures share `answers.rs`; the broader interfaces remain planned.

| Module | Responsibility | Required output |
| --- | --- | --- |
| `analysis.rs` | Summarize DSL semantics and utility dependencies | Capture reads/writes, outcome properties, ordering, effects, recursive components |
| `ir.rs` | Describe typed logical relations | Columns, keys, multiplicity, demand dependencies, choice boundaries, source provenance |
| `lower.rs` | Lower `GrammarMatcher` and utility definitions directly | A graph independent of frames and emitted SQL |
| `demand.rs` | Restrict producers and project columns | Required regions, capture keys, fields, and proven sharing |
| `choices.rs` | Preserve native witness and capture order | Ordered reductions at their original semantic boundaries |
| `recursion.rs` | Lower each recursive component | Structural closure, linear fixed point, or proven local continuation |
| `physical.rs` | Select access paths, storage, and cuts | A complete phase graph with costs and an execution bound |
| `emit.rs` | Produce SQL from physical operators | SQL, typed phase outputs, parameter bindings, and source mapping |

The public sequence is `add_clause(grammar, each)`, `finish()`, then `plan(job_inputs, budget)`.
The first two operations produce an immutable logical program.
The final operation uses file membership and index statistics to produce the physical job plan.
All clauses enter the graph before physical planning starts.
Leaf interning remains available during semantic lowering, but leaf execution follows reached demand.

A relation carries these properties:

```text
Relation {
  operator, inputs, typed_columns, semantic_key,
  multiplicity, capture_reads, capture_writes,
  outcome_contract, order_contract, effect_contract,
  demanded_by, source_locations
}

JobPlan {
  phases, immutable_stores, output_layouts,
  execution_reservations, relation_profile_map
}
```

Logical operators include `Scan`, `Filter`, `Join`, `SemiJoin`, `AntiJoin`, `Project`, `Group`, `OrderedChoice`, `Closure`, and `RegionRestrict`.
Typed expression trees represent predicates and keys.
SQL strings appear only after physical planning.
The existing `fused::graph::Relation` stores SQL bodies, so it cannot supply this semantic interface unchanged.

Capture values retain their native distinctions: node, node list, count, and text with span.
Integer atoms represent text equality where the existing semantics permit it.
An atom does not replace a capture's witness node or source span.
Unobserved capture columns disappear before expensive joins.
Unknowns use an explicit outcome contract and withheld-capture state, rather than ordinary SQL nullability.

## Milestone 1: compile the complete B023 shape

This milestone has three reviewable commits.
Each commit updates the [journal](sql-set-composition-compiler-spike.md) with its SQL and measurements.

1. Add the relation types, semantic summaries, and a direct compiler entry in the B023 harness.
2. Add structural demand, closures, capture projection, and ordered reductions until the entire B023 policy compiles.
3. Add physical access selection and compare generated SQL against the frozen reference.

The first commit compiles B023's complete capture-free `target-slot` utility through execution and native parity.
That utility exercises parent and sibling probes, Boolean composition, text leaves, and recursive destructuring.
Its policy source is [`ruff-b023-function-uses-loop-variable.json`](../packs/python/rules/ruff-b023-function-uses-loop-variable.json).
It establishes the DSL-to-relation interface without constructing an unused general framework.
The next commits extend the same operators to the complete policy.
The hand-written SQL remains an independent performance reference.

The complete generated graph must contain the following logical relationships.
Relation names can differ, but their keys and semantic boundaries must agree.

| Relation | Key or grain | Production rule |
| --- | --- | --- |
| Eligible functions | `(file, function)` | Apply the policy's safe-lambda exclusions |
| Function/loop pairs | `(file, function, loop)` | Climb from eligible functions through the correct stop boundary |
| Required regions | `(file, first, last)` | Collapse overlapping selected loop subtrees before utility scans |
| Assignment targets | `(file, target, atom)` | Expand only demanded target slots |
| Loop assignments | `(file, loop, atom)` | Retain the first successful assignment witness |
| Function demand | `(file, function, atom)` | Retain the nearest successful loop and its assignment capture |
| Parameters and stores | Keys required by the anti-joins | Evaluate only the required domains |
| Identifier loads | `(file, load, body)` before choice | Probe name postings inside demanded body intervals |
| Matches | `(clause, file, root)` plus captures | Retain the nearest successful body and exact capture identity |

This graph requires backward demand propagation through utility definitions.
The compiler first derives function/loop relationships without the capture-bearing assignment environment.
It then constrains assignment discovery to the resulting regions.
The assignment answers provide name atoms for subsequent identifier probes.
This avoids starting with every identifier and repeatedly rediscovering its structural context.

The transformation needs proof conditions in the semantic analysis:

- Reordering applies only across total, pure operations with compatible capture dependencies.
- A semijoin requires that the consumer observes existence alone.
- A projection retains every field that downstream choices, captures, errors, or output observe.
- A shared producer covers the union of consumer demands, including dialect and context identity.
- An ordered reduction stays at the DSL choice boundary, before any later predicate that cannot backtrack.

The reference's `MIN(target)` is valid only because target order matches this policy's witness order.
The generic operator uses an explicit order key and deterministic tie rules.
It does not assume that every first match equals the smallest node ID.
Likewise, B023 store exclusion includes nested scopes because the policy requires that behavior.
The compiler must not substitute a conventional lexical-binding interpretation.

**Milestone 1 acceptance:**

- Exact native and current-engine roots and captures on the corpus, upstream fixtures, and adversarial cases.
- Generated SQL from renamed utilities and changed kinds or names, without policy-ID recognition.
- One matching statement, zero frame publications, and no global instruction driver.
- At most twice the reference VM count: 4,649,592 instructions on the frozen corpus.
- Median kernel CPU at most twice the reference, measured together under identical controls.
- Constant SQL work for unrelated assignments outside relevant regions.
- Near-linear work for the existing combined assignment/load scaling cases.

The twofold allowance measures compiler overhead against a concrete attainable shape.
The complete replacement kernel now passes this gate; the CPU margin remains small.
The harness must also report preparation, native leaf work, automatic indexes, and memory.
Equal outputs with excessive work fail this milestone.

## Milestone 2: prove the difficult semantics early

This milestone follows B023 before broad policy migration.
It prevents an easy-subset compiler from becoming another architecture that cannot handle the remaining language.

| Case | Concrete proof artifact | Required behavior |
| --- | --- | --- |
| Ordered choices | Native differential cases and generated SQL | No later predicate can introduce backtracking |
| Unknowns and withheld captures | Three-valued conjunction, search, and count kernels | Preserve reached outcomes and unavailable bindings |
| Fallible leaves | Reached and unreached error cases | Preserve failure visibility without speculative failure or hidden work |
| Resource limits | Reached-budget differential cases | Preserve native work and stack/depth limits before production admission |
| Enumeration | `violation.each` with overlapping results and exclusions | Preserve round order, captures, and exclusion semantics |
| Parameterized utilities | Equal values with different witness nodes | Share equality work without merging observable witnesses |
| Positive linear recursion | Cycles, diamonds, and mutually recursive utilities | Deduplicate by semantic state, without path explosion |
| General recursion | Nonlinear dependencies, ordered captures, unknowns, reached cycles | Preserve current supported behavior and termination rules |

The existing semantic mutation kit supplies adversarial cases for these kernels.
Each kernel needs an executable reference shape, native parity, and a scaling experiment.
Unknown and fallible operations retain sequencing barriers until a rewrite proves equivalence.
Empty or unreached branches must not acquire errors through speculative evaluation.

The first implemented kernel covers positive structural recursion.
Dependency components use a tagged `UNION` closure with seed arms before recursive arms.
Utility views project each state from that shared closure.
Recursive arms contain one component reference, with indexed AST probes and no path or capture environment.
Seedless closures are empty; negative, nonlinear, and nonmonotone components fail compilation.
The compiler also rejects components with more than 128 expanded arms before SQL preparation.

The differential tests cover multiple files, diamonds, external consumers, separate components, empty closures, both directions, and multiple hops.
A capture-bearing integration case preserves all five native roots and exact witnesses through mutually recursive structural utilities.
Component members remain shared; demand propagation cannot restrict or inline one member independently.
Component-wide demand restriction remains pending.

| Scaling case | Sizes | VM instructions | Component rows |
| --- | --- | --- | --- |
| Diamond depth | 16 / 64 / 128 | 9,519 / 35,583 / 70,335 | 157 / 589 / 1,165 |
| Independent regions | 16 / 64 / 256 | 38,790 / 155,046 / 620,070 | 640 / 2,560 / 10,240 |

Component row counts come from separate diagnostic queries.
They do not represent physical write counts or add to the reported matching executions.
Statement memory is 12,864 bytes in all six kernels; this excludes the recursive queue and temporary storage.
The evidence does not establish whole-job memory usage or the 100-execution gate.

The next semantic kernel must represent true, false, and unknown outcomes explicitly.
Start with the existing conjunction tests for withheld captures, then extend the same contract to ordered search and count.
Later predicates must not turn an unknown binding into a usable capture or move across a reached failure.
Milestone 2 remains incomplete until every semantic gate above passes.

Structural recursion uses narrow closure relations like those in B023.
Eligible mutually recursive linear utilities use a tagged recursive relation.
Nonlinear or ordered components need a separate proof of their state representation.
A local SQL continuation is admissible only for an irreducible component with a bounded, measured state contract.
It cannot become a global `(pc, frame, stack)` interpreter for ordinary composition.
No host loop submits statements per recursive round.

General recursion remains the main design risk.
The earlier continuation spike proves expressibility but also shows severe amplification.
If a component lacks an acceptable SQL shape, this milestone remains incomplete.
The response is another bounded reference experiment, not fallback or a new exclusion.

## Milestone 3: cover the catalog and share selective work

The compiler harness will produce a coverage ledger from every currently supported clause and semantic test.
Each entry records its operators, unresolved features, generated relations, and measured work.
The ledger counts complete clauses, not successful fragments.
Unsupported fragments cannot disappear through lowering to an empty relation.

The migration order follows measured cost after Milestone 1:

1. Structural searches and capture-producing utilities with B023's operator set.
2. Counts, enumeration, parameterized captures, and three-valued constructs from Milestone 2.
3. Remaining recursive components and workspace integration cases.

Equivalent utilities share a producer only after demand projection and semantic identity checks.
Producer identity includes grammar, capture values read, context, ordering, and outcome behavior.
Per-consumer witness selection remains separate where the consumer observes different order or captures.
When their combined domain costs more, different selective domains can use separate producers.

The AST-index prior art supplies selective anchors, interval probes, and reduction before joins.
The data-layer prior art supplies typed relation references and dependency-driven projection.
Neither supplies a proof for Rerail's ordered captures or general recursion.
The [architecture review](sql-relational-architecture-review.md) records the source revisions and these limits.

## Milestone 4: plan the complete job within 100 executions

The physical planner consumes the complete relation graph and job membership.
The target is one matching statement across all applicable files and rules.
Cuts require a specific backend limit, memory bound, or measured reduction in total work.
File count, rule count, and recursion depth do not directly determine phase count.

The existing fixed file batches require changes in `exec.rs`, `workspace/sql.rs`, and the query caller in `eval/query.rs`.
The planner must account for worker partitions before it admits a job.
Parallel parsing can remain independent of SQL partitioning.
Extra SQL workers are acceptable only within the shared execution and memory budgets.

Every stored intermediate must have a closed producer, a narrow schema, a semantic key, and declared access paths.
The planner first restricts demand and columns, then chooses storage.
Cheap single-use relations remain inline.
Expensive reused relations can use statement materialization.
Compiler expansion or phase boundaries require real immutable storage, not another `MATERIALIZED` hint.
The earlier 65,535-reference preparation failure is a regression fixture for this distinction.

SQL produces intermediate rows in bulk.
Native buffers can store and index completed relations, but cannot evaluate rule composition or submit hidden SQL.
Temporary SQL tables remain another physical storage choice, with creation and index costs in the budget.
`ANALYZE` requires a measured benefit and its own reservation.
Virtual-table planning instead uses actual supported probes and useful row estimates.

The job ledger reserves all actual statements:

```text
job executions = setup + transactions + phase producers
               + matching + helper queries + cleanup
sum across every worker and session <= 100
```

The existing `benchmark/sql-profile/execution-budget.mjs` checks this limit after a complete run.
The transition adds pre-execution admission and a shared runtime counter.
Cleanup and failure handling need reserved capacity before matching starts.
The runtime refuses an unreserved statement before execution 101 and reports an incomplete failed job.
That failure cannot pass the complete-workload acceptance gate.
Profiler-generated EXPLAIN remains separate diagnostic work.

Each cut records its reason, estimated rows, width, indexes, bytes, and execution reservation.
SQL-size and expansion thresholds become internal planner constants with regression cases.
Join shape, materialization, and statistics decisions use measured access costs.
They do not become user tuning switches.

## Instrumentation starts with the first generated query

Every emitted relation retains its DSL and utility source locations.
The profile groups cost by logical relation and physical phase, rather than instruction address.
It includes SQL text, EXPLAIN, preparation time, VM work, leaf requests, rows, probes, and storage construction.
Cold measurements also include parsing, indexes, output decoding, and cleanup.

Ordinary CTE cardinalities are not automatically available from SQLite statement counters.
Separate diagnostic count queries can measure logical relations, but can alter their execution plan.
Those counts must carry a diagnostic label and cannot masquerade as physical write counts.
Physical buffer counters, virtual-table probes, and statement counters supply direct execution evidence.

The actionable summary will flag:

- Capture-bearing demands much larger than their distinct keys.
- Wide relations whose consumers read few columns.
- Repeated leaf requests for the same semantic demand.
- Scans where a supported name, kind, parent, node, or interval probe exists.
- Products of independent existence-only branches.
- Repeated recursive states without observable path identity.
- Expensive materializations with little reuse or late demand restriction.
- Preparation expansion and unexpected statement-budget consumption.

Each flag includes its source location, evidence, candidate rewrite, and semantic condition.
Flags distinguish a measured physical cost from an inferred logical inefficiency.

## Milestone 5: cut over and remove the scheduler

The final integration keeps `ProgramCompiler`, `Compiled`, `Session`, `SqlMatch`, and output environment reconstruction as caller-facing contracts where practical.
Internally, `ProgramCompiler::clause` records typed relations instead of emitting a frame statement immediately.
`finish` resolves the whole graph, and job planning selects physical phases.
`Session` runs those phases and decodes final capture payloads.
JSON capture payloads remain acceptable at the final result boundary, not inside intermediate joins.

The cutover removes production dependencies on:

- Frame-oriented `plan` and `emit` lowering.
- SQL reparsing in `fused::adapter` and policy-shape recognizers.
- The `large` global instruction driver and mutable frame publications.
- Scheduler-specific stage tables, register allocation, dirty-demand rounds, and frame-copy paths.

Compact AST indexes, leaf primitives, capture decoding, and profiling remain reusable components.
Immutable bulk storage can survive only through the new typed relation contract.
The native matcher remains the independent oracle.
The old SQL compiler is removed from production in the cutover change, without a runtime rollback switch.
A source-control revert remains the rollback mechanism.

The release gates are:

1. Complete supported-language coverage, including the difficult semantic kernels and mutation tests.
2. Exact native outputs on the complete frozen job: 135 policies, 141 AST matchers, and 138 files.
3. The existing three policy exclusions remain unchanged, with no additional refusals or blind files.
4. Complete result equality includes captures, ordering, diagnostics, failures, rendering, and fixes where applicable.
5. At most 100 actual executions across the cold job, with complete session traces.
6. The compiler-generated B023 shape still meets Milestone 1's work and timing gates.
7. The scaling suite excludes quadratic existence joins, repeated irrelevant scans, and recursive path amplification.

Full-job comparisons use fresh measurements of the unchanged compiler, replacement compiler, and native engine.
The historical 72-execution result and approximately 33.3 CPU seconds are context, not the new comparison baseline.
Timing uses interleaved fresh processes, frozen inputs, and separate profiling runs.
The report includes kernel CPU, cold wall time, preparation, peak memory, and every execution category.

The initial performance targets are at least fourfold fewer whole-job VM instructions and twofold lower SQL apply CPU.
Cold job time must also improve, without hiding additional native fact construction.
These are proposed engineering targets, not extrapolated B023 results.
Missed targets require a documented cost attribution and another focused shape experiment before declaring the transition complete.

## Next implementation action

The next change adds the relation types, semantic analysis, lowering, and minimal SQL emission for `target-slot`.
A test-only entry in the B023 harness compiles that utility from the DSL and compares its relation with the native result.
The following changes compile the entire policy and enforce the reference work bounds.
Broad catalog migration starts only after that complete generated query passes.
The journal records each accepted or rejected rewrite, SQL shape, semantic evidence, and measurement artifact.
