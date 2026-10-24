# Transition to direct relational SQL

Status: Test compiler qualified for 135 of 146 catalog grammars; production and complete-job gates remain closed, 2026-10-24.
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

The next kernel adds explicit true, false, and unknown outcomes for total structural operations.
It preserves withheld captures, ordered search, alternative commits, and count intervals.
Whole-node captures use native structural equality, including composite nodes, without native rule composition.
The [outcome evidence](../benchmark/sql-relational-outcomes-evidence.json) records exact roots, captures, unknown outcomes, EXPLAIN, and scaling.

Non-following value leaves and node-local rule-based stops now have an ordered SQL kernel.
It preserves reached atomic failures through sequencing barriers and early traversal termination.
Capture-bearing ordered walks, nested fallible traversal bodies, full-file failure publication, enumeration, and general recursive components remain pending.
Native work accounting now has a bounded outcome proof. Memo charge replay and stack/depth budgets remain unresolved.
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

The explicit-outcome kernel now represents false, true, and unknown with integer tags.
Missing captures remain separate NULL binding columns.
Conjunctions propagate withheld node captures and allow independent false terms to decide the result.
Alternatives commit the first decisive branch; capture-bearing unknowns block later alternatives.
Search preserves candidate order and its native unknown barrier.
Counts use lower and upper bounds; exported counts require an exact value.

Node-local terms use guarded scalar projections, without demand/result self-joins.
Eligible capture-free minimum-one counts use indexed existence probes.
A second probe checks for unknown only when no true candidate exists.
Other counts retain narrow candidate relations and interval aggregation.

The nine scaling kernels use one traced matching execution each and zero frame publications.
At widths 16, 64, and 256, a known-first count takes 309 VM instructions and one regex request.
All-unknown counts take 378, 618, and 1,578 instructions.
Independent conjunctions take 37,092, 146,676, and 585,012 instructions at sizes 16, 64, and 256.
The conjunction shape still creates automatic indexes across sequencing relations; the profile flags that remaining cost.

The kernel admits non-following `value` predicates as atomic semantic leaves.
Total structural walks can still evaluate candidates eagerly.
Rule-based stops and fallible node-local traversal bodies instead use a narrow recursive relation.
Its local phases preserve stop-before-candidate order for ancestor and sibling walks, and candidate-before-stop order for descendant walks.
Counts stop at decisive bounds; exact exports continue until their value is known.
Unknown stops leave an open frontier, which makes a count's upper bound unbounded.

Fault injection occurs at the same native value leaf in both engines.
The tests compare calls through the first failure and internal failure codes.
Choices, stop boundaries, and withheld captures suppress unreached failures.
Native attempts after its first recorded failure appear separately in the trace.
Native work budgets and full-file failure publication remain separate gates.
The first ordered traversal kernel required capture-free, node-local candidate and stop rules.
The subsequent kernels admit local capture masks and nested typed child results, as described below.

The [reached-kernel evidence](../benchmark/sql-relational-reached-evidence.json) contains 14 failure traces and six scaling profiles.
The [generated SQL](../benchmark/sql-relational-reached.sql) shows the local traversal phases and guarded scalar operands.
Known-first searches take 856 VM instructions and one value call at widths 16, 64, and 256.
All-unknown searches take 9,706, 37,114, and 146,746 instructions, with one value call per identifier.
Each profile uses one matching execution, zero frame publications, and zero automatic-index entries.
The recursive relation emits three logical states per visited candidate; those diagnostic counts are not physical writes.
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

## Nested reached traversal status

Nested bodies and stops now return typed integer/NULL columns through a correlated SQL row-value subquery.
SQL completes each reached child before the parent advances.
The row constructor only copies columns; it cannot evaluate rules, inspect ASTs, or execute SQL.
The outer recursive relation carries ten control columns plus observable exports.
The compact node-local kernel retains ten columns and reconstructs its local capture witnesses.
Admission currently stops at four traversal levels.

Correlated materialization is not a promise of one evaluation across multiple consumers.
An isolated regression doubled four atomic calls to eight when two consumers referenced the same correlated prefix.
The corrected shape gives that prefix one consumer and passes its computed state into a child scope.
Both nested Boolean composition and compact candidate composition use this boundary where required.

Twelve scaling profiles cover early matches, late inner matches, late outer matches, and independent roots.
Each uses one matching execution and zero frame publications.
At inner widths 16, 64, and 256, early matches require 2,363 VM instructions and one value call.
Other dimensions grow linearly; the journal records exact counts and typed return copies.
These results establish this bounded kernel, not whole-job performance or the 100-execution gate.

## Failure publication status

The bounded workspace kernel now publishes typed failures by clause and file.
Reached atomic limits propagate through negation, conjunctions, alternatives, searches, counts, and nested stops.
False and unknown remain separate from failure.
Unexpected host or SQLite errors still abort the statement.

A fallible clause sequences roots within each file and stops that pair after its first failure.
Total clauses retain set composition.
The final failure filter suppresses all matches from a failed pair, including earlier provisional matches.
Healthy files and sibling clauses continue in the same matching statement.
Captures remain typed until the final JSON projection.
The public diagnostic decoder uses the existing limit messages and `budget-exceeded` code.

Native differential cases cover reversed file and clause order, multiple failures, empty files, withheld failures, and exact captures.
A native workspace test also confirms suppression through public rendering after a real value-budget failure.
Stateless snapshot evaluation has a different contract: a file limit discards the entire clause result.
That aggregation remains the caller's responsibility; this kernel implements workspace-local publication.

The first publication shape repeated a shared failure query under each final clause arm.
At 32 clauses, that caused 116 ms of preparation for about 2.5 ms of execution.
The accepted shape materializes one typed publication relation and applies one failure filter.
The first comparison reduced preparation to about 11 ms, with slightly lower VM work.
The journal records final measurements and the rejected shape.

This kernel admits up to 32 clauses and 256 KB of generated SQL.
These are internal admission limits for the semantic proof, not runtime switches or complete-job batching policy.
The fixtures use TypeScript and admit every supplied file to each clause.
Mixed-dialect batches and per-clause file selection remain integration work.
The replacement remains test-only until the remaining transition gates pass.

## Native work allowance: bounded outcome milestone

The outcome compiler now carries remaining work through roots, Boolean operands, and nested traversals.
The atomic value cursor accepts an allowance and returns truth plus remaining work.
SQL owns the allowance for each clause/file pair.
There is no host budget accumulator and no unlimited value-leaf execution path in this compiler.
The test harness supplies either a finite allowance or `i64::MAX` to the same generated shape.

Counts charge each visited candidate before testing its body.
This includes punctuation and candidates that fail a kind test.
Their prior uncharged existence-probe reduction has been removed.
Utility entry charges survive inlining.
Pure structural operators retain set-based evaluation and return their unchanged allowance.

The publication kernel now uses native anchor kinds for its initial root selection.
The native oracle uses `run_clause`, file value caches, and utility memoization.
It compares reached value calls separately for each clause/file pair.
It also compares captures, provisional-match suppression, remaining allowance on published rows, and public diagnostics.

Nine new tests cover zero allowance, successful thresholds, decisive alternatives, unknowns, counts, nested walks, and utility entry.
The accepted fixtures contain 217 allowance-boundary jobs and 12 scaling jobs.
Each job uses one matching SQL execution and zero frame publications.
EXPLAIN and VM counters are retained in `benchmark/sql-relational-work-budget-evidence.json`.
These tests prove the bounded outcome contract. They do not prove the complete cold-job execution budget.
The work-allowance phase ended with a B023 CPU-gate failure at 2.02653× against a 2× limit. SQL and VM counts were unchanged.
An earlier run passed at 1.68182×. Both runs are retained, and timing qualification remains open.

Value-bearing utility reuse across Boolean scopes, traversals, or nested utilities still needs a shared demand contract.
Local reuse now replays cost correctly.
Two-valued charged search pruning is now admitted for the supported outcome operators, as described below.

Native stack/depth limits, deadlines, decisions, enumeration, general recursive outcomes, catalog coverage, and production integration remain separate gates.
The complete cold job must still prove the 100-execution limit.

## Local utility memo: bounded outcome milestone

The compiler now admits repeated value utilities within one Boolean demand.
It retains utility identity and stores typed truth, semantic cost, input captures, and output captures.
File and node identity belong to the enclosing demand.
The key includes every incoming capture, including captures the utility body does not mention.
A hit replays the original cost before it returns the answer.
Failed and unreached invocations do not create reusable answers.

Static capture versions select the SQL shape.
When versions match, later calls contain only cost and answer replay.
When a capture may change, SQL compares typed keys and guards the miss body.
Single-use utilities do not allocate answer storage.
Answer columns stop propagating after their last use.
There is one compiler path, with no runtime option or native composition fallback.

The core fixture now matches native execution: allowance three fails with `Work`; four succeeds with zero remaining.
The value leaf runs once per file.
False, unknown, count exports, unrelated captures, warm value caches, and reached failures have native comparisons.
The local storage limit is eight allocated key versions and 512 typed columns.
These are admission limits for the test-only kernel, not production workload limits.

At 32 identical calls, direct replay reduced VM work from 8,411 to 5,059.
SQL size fell from 44 KB to 21 KB; typed return cells fell from 136 to 12 across two files.
The eight memo tests pass 198 native-comparison jobs, including 12 profiles.
The frozen B023 run passes at 1.94833×, with unchanged query text and VM work.
The preceding 2.02653× failure remains in the evidence; this pass does not establish a B023 speedup.
The journal records final profiles, source hashes, and qualification results.
The complete cold-job 100-execution gate and production admission remain open.

## Native candidate pruning: bounded outcome milestone

The compiler now admits two-valued charged searches for the supported outcome operators.
It uses the native kind and required-text analysis through a test-only interface.
All two-valued searches apply kind bounds before the candidate body and its charges.
Descendant searches to the end also apply the native required-text bounds.
Their SQL seeks the next candidate through ordered, bounded kind postings.
Irrelevant nodes no longer allocate recursive traversal rows or typed child returns.

Text-filtered first candidates have one narrow materialized relation because the seed needs their result twice.
That relation contains demand ID, file, node, subtree bound, and remaining work.
Caller captures stay in the existing demand relation.
Neighbor and stop-bearing searches retain visitation order and guard only the candidate body.
Stops still run at skipped body candidates.
Three-valued searches and count visitation retain their prior contracts.

The allowance-one fixture passes; its three-valued counterpart needs two charges.
Finite-allowance comparisons cover text bounds, kind unions and intersections, captures, nested searches, counts, and stop order.
Negated searches expose remaining work when the body fails.
At 256 irrelevant siblings, sparse-kind VM work falls from 286,901 to 1,707 across two files.
It remains 1,707 at widths four, 32, and 256.
The sparse-text fixture falls from 288,981 to 4,302 VM instructions after sharing its first-candidate scan.
All 103 selected tests and both Clippy configurations pass.
The 66 preceding-phase controls retain identical SQL and VM counts.
The frozen B023 run passes at 1.83296×, with unchanged query text and VM work.
The journal records final profiles and both rejected shapes.
This is matching-query evidence; node-index construction and the complete cold job remain separate measurements.

## Nested Boolean utility replay: bounded outcome milestone

Utility answers now survive nested Boolean scopes within one root demand.
The compiler selects the smallest Boolean scope shared by repeated calls.
It emits one dependency chain through that scope and retires temporary columns after their last use.
Capture snapshots preserve commit and rollback behavior. Answers retain their original input keys and measured costs.
There is no runtime option or native composition fallback.

Separate answer CTEs failed reached-call parity because SQLite reevaluated correlated dependencies at multiple references.
The dependency chain removes that duplicated work.
An unconditional call can provide a replay proof for later siblings in a region without captures.
Calls in optional branches do not export that proof.
Other calls compare every input capture and preserve the bounded memo storage contract.

The initial fixture now passes: allowance three fails, while four succeeds with zero remaining and one value read per file.
The six new tests cover 804 native-comparison jobs and 15 profiles.
Coverage includes unknowns, capture rollback, changed keys, counts, independent utilities, reached failures, and skipped nested branches.
At 32 nested calls, VM work falls from 8,943 in the first correct shape to 5,771.
Typed return cells remain at 12 across two files. Materialization sites fall from 178 to 82.
The 81 preceding-phase controls retain identical SQL, output, and VM counts.
The journal records the qualification gates and measured source hashes.

## Nested value-utility replay: bounded outcome milestone

Utility answers now survive calls through nested value-bearing utility bodies at one root.
The outer miss runs in the existing dependency chain and records its entry charge plus all reached body work.
An outer replay consumes the complete measured cost and omits the body when a valid replay proof exists.
Inner typed answers remain available after the outer call, including false and unknown answers.
Optional body calls do not export replay proofs.

Native execution confirmed the initial boundary before implementation.
Allowance nine fails. Ten succeeds with zero remaining and one value read per file.
A false outer call followed by its inner utility needs five charges, even with a warm value cache.
A three-level shared-call fixture needs 33 charges and one value read.
The seven new tests cover 704 comparison jobs, including 15 profiles.
Changed keys, rolled-back captures, count exports, utility identity, reached failures, and bounded storage have native comparisons.

The physical pass reduces the seven-level fixture from 4,987 to 3,825 VM instructions across two files.
EXPLAIN materialization sites fall from 57 to 36. Typed return cells stay at 12.
All 96 preceding-phase controls retain identical SQL, output, and VM counts.
The frozen B023 VM gate passes, but the CPU gate fails at 2.02845× with unchanged SQL and VM counts.
The journal retains this failed gate, full qualification results, and measured source hashes.
Performance qualification remains open. The bounded kernel remains test-only.

## Traversal value-utility replay: bounded outcome milestone

Reached value-utility answers now survive traversal returns and overlapping roots within each clause/file pair.
The compiler selects an indexed scratch relation when traversal extends answer lifetime beyond one root.
The cursor stores integer keys and completed answers. SQL controls composition, measured costs, replay, and publication.
Keys contain clause, utility, file, candidate, and every incoming capture.
A materialized initializer clears clause storage once per statement execution. Prepared-statement reuse has a storage regression test.
Failed calls never publish answers. Unknown and false answers can be reused.

Native execution confirms both initial boundaries: four charges for repeated search, and eight for overlapping roots.
Both fixtures read one value per file. Changed capture keys require separate answers and can require fresh value reads.
The new tests cover nested utilities, captures, counts, all traversal directions, stops, reached failures, and independent clauses and files.
The connection executes one matching job at a time; interleaved jobs on a shared connection remain outside this test-only contract.

The first correct implementation still walks every irrelevant candidate.
At 32 overlapping roots across two files, it uses 3,624,895 VM instructions and 33,536 typed return cells.
A pure Boolean prefix now permits bounded kind seeks before any effect is reached.
This reduces those counts to 111,423 instructions and 1,280 cells, with unchanged results and charges.
Counts and stop rules retain their required visits. The memo store adds no recursive state columns.
The journal records qualification results and retained profiling failures.

## Compiled repeated traversal groups: bounded outcome milestone

Adjacent identical descendant searches in `all` now compile to one traversal program with guarded semantic-cost replay.
The proof requires no capture effects or stop rule, and a pure predicate prefix followed by one value-bearing utility call.
Incoming captures remain unchanged throughout the group. Different utility identities and prefixes form separate groups.
The first traversal populates clause/file utility answers and measures its cost.
False and failed first outcomes stop the group. True and unknown outcomes replay that cost for later calls.
Division checks the remaining allowance before multiplication, preserving integer arithmetic at signed limits.

Native preflight confirms three-call boundaries of six charges for true, four for false, and 12 for unknown.
Two overlapping roots need 12 charges and one value read per file.
Seven new comparison tests cover 662 jobs, including 15 profiles. A separate SQL test covers arithmetic near signed limits.

At 32 repetitions across two files, SQL text falls from 184,607 to 8,671 bytes.
VM work falls from 44,321 to 2,505 instructions, and typed return cells fall from 520 to 24.
EXPLAIN materialization sites fall from 623 to 35.
SQL size and return cells stay effectively constant from two through 32 repetitions.
SQLite adds one constant-initialization instruction when the replay multiplier exceeds one.
The journal records prior controls, full qualification, and frozen workload results.

## Repeat sharing in alternatives: bounded outcome milestone

Adjacent identical descendant searches in `any` now use the same repeat proof and child-program emission as `all`.
A compile-time Boolean selector changes the replay stop predicate.
True and failed first outcomes stop. False and noncapturing unknown outcomes replay the measured cost.
The alternative barrier retains its unknown flag, fallback reachability, and capture publication rules.
The grouping proof still excludes capture effects, stops, counts, unwrapped value leaves, and nonidentical operands.

Native preflight confirms three-call boundaries of two charges for true and 12 for false or unknown.
Two overlapping roots with true searches need four charges and one value read per file.
Seven comparison tests cover 973 jobs, including 18 profiles. A separate SQL test covers signed-limit arithmetic.
A reached-only test probe now suppresses fault injection during native cleanup after a prior failure.
The regression case retains the original Work failure and verifies a later fallback's Value failure when that fallback is reached.

Across two files, 32 false alternatives now need 8,766 SQL bytes, 3,629 VM instructions, and 36 typed return cells.
The previous shape needed 185,057 bytes, 81,215 instructions, and 904 cells.
EXPLAIN materialization sites fall from 623 to 35.
True, false, and unknown profiles retain constant return cells across 2/8/32 alternatives, including profiles with a later fallback.
VM counts differ by at most one constant-initialization instruction.
The journal records qualification and the frozen workload gates.

## Shared immutable utility plans: bounded outcome milestone

Each acyclic utility body now lowers once per clause, keyed by utility table identity and index.
Each call retains a charge operation and its capture metadata.
Traversal filters stay at call sites. Completed immutable bodies share storage, while active bodies remain outside the cache.
Effect and walk-depth summaries combine immediate child summaries once during lowering.
The single-clause compiler reuses its lowered plan for publication.

Profiles report distinct lowered operators, utility call sites, completed body lowerings, atomic terminals, and composition operators.
Compiler wall time includes lowering and SQL generation, with separate SQL preparation and execution counters.
A separate composition preflight applies the existing repeat proof and conservatively counts every other utility body at each call.
The limit remains 256 composition operators. Distinct lowered operators, capture width, traversal depth, and SQL size also retain their limits.

Native preflight proves that all six wide 32-search fixtures succeed while the preceding compiler rejects them at 256 expanded operators.
Those fixtures now lower 142 distinct operators and retain 18 composition operators after repeat grouping.
They share two utility bodies and nine atomic terminals.
The narrow 32-call fixture drops from 64 body lowerings to two, and from 32 value terminals to one.
The 30 previously admitted profiles retain identical SQL and VM counts.
The journal records semantic qualification, prior controls, and frozen workload results.

## Composition admission follows emitted sharing: bounded outcome milestone

The composition limit now follows the existing emitter's sharing decisions.
Each clause receives one generation counter, shared by every nested child program.
A proved replay consumes one operator. A generated miss body consumes its own operators.
Scalar and ordered programs consume their emitted trees. The limit remains 256.
The `compositionOperators` profile now reports generation work instead of the preceding conservative estimate.

Metadata analysis visits each distinct plan once per analysis and combines exact logical call counts with checked arithmetic.
A compact shared graph cannot force exponential metadata traversal before generation starts.
Graphs that still expand excessively fail at the shared generation limit.
Capture versions and local memo proofs retain their existing requirements.

All six previously rejected direct 32-call fixtures now compile with 46 composition operators and nine terminals.
The 18 profiles retain one outer miss body and one value-read site, with 12 typed return cells across two files.
Native boundaries, unknown behavior, capture keys, failure publication, and nested sharing have comparison coverage.
The journal records qualification and retained failures.

## Adjacent direct utility groups: bounded outcome milestone

Adjacent identical node-local utility calls now share one first-call result and one guarded cost calculation.
The existing local key-version or shared dominance proof admits each group.
The proof requires no input bindings or capture effects. Optional branches and different utility IDs retain their existing emission.
Traversal and direct groups share the same overflow-safe arithmetic.
Grouped calls consume their logical remaining uses, including nested utility calls, while the generation budget counts one replay group.

All 36 simple and nested utility profiles retain constant materialization counts and typed return cells across 2/8/32 calls.
SQL text grows by three bytes. VM work differs by at most one constant-register instruction.
At 32 true conjunction calls, SQL falls from 38,320 to 14,555 bytes.
Materialization sites fall from 103 to 42, and VM work falls from 7,773 to 3,219 instructions.
Simple value utilities use 19 materialization sites and 1,375 instructions for the same true fixture.
The journal records comparisons, semantic qualification, and frozen workload results.

## Consecutive total predicates: bounded outcome milestone

The shared Boolean emitter combines consecutive `Op::Predicate` operands into one guarded, ordered `CASE` expression.
Each run uses one materialized result and one sequencing barrier.
Existing scalar lowering supplies the predicate expressions. Each predicate appears once, in source order.
The guard preserves SQL NULL for unreached runs. The Boolean barrier retains unknown, failure, and capture publication rules.

Values, utility charges, unknown tests, captures, negation, and traversals end a run.
Generation counts the predicate leaves plus one Boolean operator. All existing limits remain in force.
For the preceding eight-predicate, 32-call true fixture, SQL falls from 14,555 to 8,366 bytes.
VM work falls from 3,219 to 2,237 instructions, and materialization sites fall from 42 to 28.
All 162 unique tests pass. The 231 preceding profiles contain 18 improvements and 213 unchanged SQL/VM results.
The final frozen B023 CPU gate fails at 2.021×, with unchanged SQL and VM counts.
The journal retains that failure and the initial 1.976× pass. CPU qualification remains open.

## Direct atomic predicates: bounded outcome milestone

Scalar outcomes and ordered Boolean tests now share direct expressions for kinds, regexes, empty sets, and all-node sets.
The surrounding guards, truth conversion, semantic charges, terminal registration, and generation limits remain unchanged.
Patterns, references, compound sets, and traversals retain their relational lowering.

All 54 width/repetition profiles preserve native results, charges, reached calls, and typed return cells.
Their correlated scalar subquery count is four at every width, down from 7, 13, and 37 at widths 2, 8, and 32.
Constant-row scans fall to two. Materializations remain at 28 and typed return cells remain at 12.
For 32 true utility calls, VM work falls from 2,237 to 2,111 instructions with eight predicates.
With 32 predicates, VM work falls from 2,933 to 2,471 instructions.
All 231 preceding profiles also reduce VM work, with unchanged native results, charges, reached calls, and typed returns.
All 165 unique tests pass. B023 passes both gates in this run at 1.941× reference CPU time, with unchanged SQL and VM counts.
The preceding CPU failure remains recorded. The journal does not claim a structural B023 speedup or stable CPU qualification.

## Bound set predicate emission: bounded outcome milestone

The emitter now represents each arm's sources and conditions before SQL formatting.
It emits a direct condition when the bound node supplies every source.
Witness joins retain an existence query. Union membership uses Boolean alternatives instead of a temporary union result.
Atomic outcome predicates use this same representation.
Reached-call guards and failure/capture publication barriers remain unchanged.

B023 removes all eight redundant outer existence layers and preserves results, captures, relation rows, and typed returns.
VM work falls from 2,812,255 to 2,804,023 instructions: a 0.29% reduction.
Both frozen gates pass at 1.830× reference CPU time in this run. Earlier CPU failures remain recorded.
All 166 unique tests pass. The 54-profile matrix and 231 prior controls retain identical SQL and VM counts.

This phase leaves 33 materialization sites, 54,156 automatic-index rows, and 35,379 full-scan steps unchanged.
Correlated subquery sites rise from 34 to 60 because Boolean alternatives now expose separate witness queries.
Constant-row scans fall from 58 to 46. Fewer wrappers do not establish that the remaining SQL work is efficient.

## Prior audit — preparation cost (superseded by the user on 2026-10-23)

The validation audit found a priority error in the preceding plan.
For frozen B023, median preparation CPU is 47.613 ms for generated SQL and 3.245 ms for the reference.
Execution medians are 73.578 ms and 64.110 ms.
Across paired samples, excess preparation accounts for 76–82% of the cold-CPU gap.
The earlier plan to begin with producer execution counters did not target that measured gap.

The new preparation gate requires generated median preparation CPU at most twice the reference median.
The saved baseline fails at 14.67×. `benchmark/relational-phase-audit.py` records this failure separately from the original passing gates.
This is a new engineering target. No retrospective gate change alters the recorded qualification.

Profile `sqlite3_prepare_v2` on the exact frozen generated and reference queries with registered virtual tables.
Keep parsing, terminal installation, preparation, and execution in separate measured spans.
Record preparation CPU, virtual-table index-selection calls, bytecode size, and query-plan size.
Use stack samples or equivalent attribution to distinguish expansion, planning, index selection, and bytecode generation.
Do not infer the cause from SQL length or static subquery counts.

Run controlled query-shape experiments against that baseline, changing one mechanism at a time.
Test shared dependency boundaries and materialization choices where the preparation profile identifies repeated expansion.
If a small number of statements is required, count setup and execution against the complete-job budget.
A cached warm run cannot substitute for the cold preparation gate.
Record failed candidates and retain the exact source, executable, SQL, and corpus hashes.

Promote a change only after it passes the preparation target, improves the measured cold kernel, and preserves semantic results.
Keep full-query VM work at or below 2,804,023 and repeat the 54-profile matrix and 231 preceding controls.
Do not use an arbitrary one-instruction improvement to claim that the bottleneck was addressed.
After preparation is addressed, use producer diagnostics to locate the remaining execution gap.

Production integration, deadlines, native depth limits, general recursive outcomes, and the complete cold-job 100-execution gate remain open.

## Current action — reference-shaped SQL and prepared execution

The user directed us to prepare queries once and exclude that cost from repeated execution.
This supersedes the preceding preparation-first priority and its cold preparation gate.
The audit evidence remains valid; its preparation gate is no longer the acceptance criterion.

The compiler must produce plans comparable to the reference in structure and size.
Before qualification, the new driver fixed these limits against frozen B023:

| Measure | Limit |
| --- | --- |
| SQL bytes | 1.25 times reference |
| Static bytecode instructions | 1.5 times reference |
| Materialization sites | Reference plus 4 |
| Correlated scalar subqueries | Reference plus 4 |
| Temporary B-tree sites | Reference plus 3 |
| Median prepared execution CPU | 1.25 times reference |
| Executed VM instructions | At most 2,804,023 |

Each query is prepared once and executes three times on that statement handle.
Three paired corpus samples supply nine executions for each query.
Preparation CPU is reported separately. Exact native parity, one matching statement, and zero frame publications remain required.
The structural and runtime gates must all pass. A small VM reduction alone cannot qualify the change.

The compiler changes share functional AST joins, combine demanded seed alternatives, and remove redundant region tests.
They combine equivalent child branches and eliminate ranking where a unique parent determines the witness.
Negative descendant probes use shared leaf sets and value/range anti-joins.
Recursive probes combine equivalent transitions instead of repeating full join arms.
The frozen B023 qualification passes all gates on the final source:

| Measure | Previous compiler | Current compiler | Reference |
| --- | ---: | ---: | ---: |
| SQL bytes | 21,882 | 10,231 | 8,196 |
| Static bytecode instructions | 4,760 | 2,203 | 1,947 |
| Materialization sites | 33 | 23 | 20 |
| Correlated scalar subqueries | 60 | 9 | 18 |
| Temporary B-tree sites | 27 | 8 | 9 |
| Executed VM instructions | 2,804,023 | 2,294,480 | 2,324,796 |

Final median prepared CPU is 29.422 ms generated versus 32.633 ms reference.
An earlier passing qualification measured 32.514 ms versus 31.997 ms with the same B023 SQL.
These runs establish comparable speed. They do not establish a stable generated-query speed advantage.
The final run preserves exact native results across 138 files and reports zero automatic re-preparations.

Review also tightened the inclusive-descent proof to preserve branch order when two branches publish different witnesses.
The regression test checks both orders and their distinct results.
Five rejected size candidates and both passing full qualifications remain recorded.

The remaining three materialization sites do not justify another cycle of removing isolated SQL wrappers.
Use these structural and prepared-execution gates for subsequent compiler changes.
The implementation remains test-only; production integration, general recursive outcomes, native limits, and the complete-job budget remain open.


## Current action — selective demand, then compiler replacement

Matching the reference shape left excessive candidate work.
The next change restricts origin seeds before ancestor traversal and removes unused values before capture choice.
Assignment witnesses use subtree ranges with an indexed test for intervening stop nodes.
Materialized binding keys prevent repeated AST lookups after SQLite flattens joins.
The compiler removes unused relation definitions through typed dependencies.

The new frozen-workload VM limit is 250,000. All preceding structural and prepared CPU limits remain unchanged.
The final query passes at 246,364 VM instructions and 10,232 SQL bytes, preserving exact native results.
Median prepared CPU is 4.264 ms generated versus 30.218 ms reference.
This is a 9.3-fold VM reduction against the preceding test compiler, with one matching statement and zero frame publications.
Two scaling checks cover sizes 32, 128, and 512:

- Unrelated functions must add no SQL VM work.
- Unused binding names must leave one capture-demand row for the one used value.

These checks do not exclude parsing or native index construction from total job cost.
Loop instrumentation is separate from prepared execution timing and ordinary-build VM qualification.
The experiment artifact retains the baseline failures, three rejected candidates, source patches, executable hashes, SQL, and loop measurements.

The catalog audit exposes the next transition gap.
Across all 138 Python policies, only 19 of 146 AST grammars compile through the replacement compiler.
The audit has no exclusions. Compilation admission does not certify runtime semantics or resource limits.
The remaining 127 grammars fail for these reasons:

| Missing contract | Grammars |
| --- | ---: |
| Outcome leaf or operator contract | 33 |
| Operators outside the total neighbor-set subset | 31 |
| Input capture effects | 23 |
| Ordered capture frontier | 14 |
| Recursive outward captures | 9 |
| Recursive outcomes | 7 |
| Pattern capture witnesses | 6 |
| Negative value probes without bound input captures | 3 |
| Recursive negation or region dependencies | 1 |

The next phase targets this ledger, rather than another isolated B023 optimization.
For each rejected family, add typed lowering and native differential execution on its real policies.
A grammar passes only after preparation, exact outputs, captures, failures, and resource limits agree.
Admitted grammars need the same execution checks; compilation success alone cannot qualify them.
Keep the full catalog visible alongside the existing frozen job and its unchanged exclusions.

Then replace the production compiler and scheduler in one cutover, using prepared relation plans and a shared job graph.
Remove the old frame compiler from production reachability. Do not add per-clause fallback or runtime selection between SQL compilers.
The complete-job gate still requires at most 100 actual executions and measured VM reduction against a fresh main-engine baseline.
The current admission audit fails that replacement readiness gate; this phase does not replace the main compiler.

## Catalog transition checkpoint: typed captures and atomic costs

The replacement compiler now admits 38 of the 146 catalog grammars.
All 38 pass exact native comparison across every checked-in upstream Python fixture: 595 files with no exclusions.
The complete-coverage gate remains a failing test, with 108 refusals.
This checkpoint does not replace the production compiler.

The new contracts cover capture-free stopped ancestor and sibling walks, node captures in composite patterns, ordered capture constraints, and neighbor-field traversal.
Atomic resolution, layout without capture inputs, absent arguments, and followed values retain native leaf evaluation.
SQL owns traversal, Boolean composition, capture transactions, and utility replay.
Typed applicability chooses the frontier or ordered representation before emission; compilation errors never trigger another compiler.

Finite-budget tests found and corrected two cost contracts.
Utility replay must exclude one-time native cache construction.
Scope-sensitive leaves must also retain the identity of their owning clause.
Constraints that share utilities with their caller require keyed memo storage, including calls in the constraint core.
A prepared job starts a fresh scope lifetime; independently compiled groups still need job-global clause tokens.

The next contract priorities are explicit:

1. Add typed list-capture columns, including input unification and publication. They are the first refusal for 34 grammars.
2. Add ordered recursive utility components. They are the first refusal for 49 grammars, with further recursive dependencies behind other refusals.
3. Add argument bindings, comparisons, binding-site predicates, and parsed-tree composition. Preserve each native unknown and failure boundary.
4. Qualify finite work, depth, deadlines, mixed-dialect routing, publication, and incremental context before production replacement.
5. Build one shared job graph, split only at measured limits, and enforce the complete-job 100-execution gate.

Neighbor fields do not imply support for longer field walks.
Native field traversal has distinct stop behavior, which requires its own SQL contract.
The catalog admission count does not certify full policy execution, and matching fixtures do not cover every semantic branch.

The standalone pattern contract now has a physical scaling gate.
With two call nodes and 32, 128, or 512 unrelated declarations, matching must use two pattern probes and two root node advances.
The gate omits an explicit DSL kind, so the compiler must derive demand from the pattern itself.
Capture constraints retain that root proof while evaluating each constraint at its captured node.

Checkpoint evidence is recorded in:

- `benchmark/sql-relational-cutover-coverage.json`: complete catalog ledger, fixture inventory, native/SQL output hashes, and remaining refusals.
- `benchmark/sql-relational-cutover-evidence.json`: source and binary pins, earlier failed checks, final qualification, and preceding shape controls.
- `benchmark/sql-relational-pattern-kind-experiment.json`: failing and corrected scaling results, generated SQL, and explained plans.

To rerun the catalog checks, use the engine test binary from `cargo test -p rerail-engine --profile loop --features unbounded-work --no-run`.
Set `RERAIL_RELATIONAL_CUTOVER_LEDGER` to a separate output path for each check.
Run `relational_cutover_catalog_fixture_parity --ignored --nocapture --test-threads=1` for admitted-grammar parity.
Run `relational_cutover_catalog_complete_coverage --ignored --nocapture --test-threads=1` for replacement coverage.
The latter must fail until every catalog grammar has an admitted contract.

## Catalog closure checkpoint: lists and typed semantic inputs

The next checkpoint admits 70 of 146 grammars, up from 38.
Every admitted grammar passes exact native matching on all 595 fixtures and has a positive witness.
The complete-coverage gate still fails: 55 grammars first need recursive outcomes, 20 need another operator contract, and one needs recursive negation.
These are first refusals; fixing one can expose another missing contract.

The added contracts cover ordered list captures, capture-independent binding predicates, argument bindings, capture-dependent layout/comparison, and End-field traversal.
Lists retain absent versus empty values and the exact native sequence, including punctuation.
Atomic leaves receive explicit typed inputs; SQL owns composition and capture commit/rollback.
Rule-stopped field walks, parsed-tree composition, parameterized utility calls, and binding-site child programs remain separate contracts.

Catalog execution also exposed two structural query defects.
Ordered pattern alternatives duplicated predecessor references during preparation; atomic witness steps now reference that predecessor once.
Capture-free pattern alternatives lost their necessary-kind proof; typed predicates now retain and propagate that proof.
The latter reduces B413's matching VM count from 417,040,778 to 3,987,126 on the same fixture batch.
The matching SQL still uses one statement for that grammar, with zero frame publications.
This is not a fused complete-job result.

A standalone recursive-chain experiment demonstrates shared demanded keys and suffix summaries with linear scaling.
It deliberately does not admit recursive outcomes: incoming utility depth and native stack-limit behavior remain unresolved.
The next recursive contract must carry reached truth, cost, captures, and failure precedence through a component, without origin-by-descendant expansion.
After catalog contracts close, the shared-job, publication, resource, context, and 100-execution gates still precede production replacement.

`benchmark/sql-relational-catalog-expansion.json` records the alternative-scaling samples, B413 SQL before and after pruning, and the same-70-grammar VM comparison.
The recursive-chain measurements in that artifact are experiments, not admitted catalog coverage.
List transport currently caps each cell at 1 MiB; the production resource contract must account for that explicit bound.

Memory qualification must measure list payload bytes, not only cells or row counts.
The legacy `integerCellsStored` counter describes sparse fields, and memo storage currently lacks an aggregate byte bound.
Add retained/copied byte telemetry and a defined aggregate failure contract before production cutover; silent memo eviction can change native work semantics.

The final named first-refusal ledger is:

| Missing contract | Grammars |
| --- | ---: |
| Recursive outcomes | 55 |
| Parsed-tree composition | 9 |
| Format-argument composition | 6 |
| Binding-site composition | 3 |
| Parameterized utility calls | 2 |
| Recursive negation or region dependency | 1 |

Qualification passes 254 relational tests and preserves all 285 preceding SQL/VM control profiles.
Frozen B023 also preserves every existing shape and VM gate.

## Catalog continuation: functional recursive chains

The test compiler now admits 76 of 146 grammars, up from 70.
The six additions are ASYNC210, ASYNC222, ASYNC250, ASYNC251, F622, and F633.
The component contract requires a pure base, a capture-free guard, and one monotone parent or sibling continuation.
A finite guard-kind proof selects the shared domain.
SQL stores suffix answers once per component and statement, keyed by file and node.
Every reached invocation retains its full logical utility cost.

This admission does not imply native stack-byte equivalence.
The chain contract checks logical depth before each recursive entry charge and includes the caller's utility depth.
Value-reading utility ancestors that contain a chain remain refused because their memo answers lack peak depth.
The complete-coverage gate and production cutover remain closed.

The first-refusal ledger now contains:

| Missing contract | Grammars |
| --- | ---: |
| General recursive outcomes | 49 |
| Parsed-tree composition | 9 |
| Format-argument composition | 6 |
| Binding-site composition | 3 |
| Parameterized utility calls | 2 |
| Recursive negation or region dependency | 1 |

The next recursive contract must handle ordered downward candidates, including anonymous tokens and unsuccessful child calls.
A single named-child shortcut cannot preserve native work or depth.
Parsed-tree and format-argument contracts must expose typed documents and candidates before SQL executes their child plans.
Binding-site contracts likewise require target facts, followed by ordered child transactions.

The new performance gates cover shared suffixes, disconnected guard regions, indexed lookups, and skipped or exhausted invocations.
A zero-work regression caught eager materialization through an outer join.
A guarded correlated lookup reduces the largest failing sample from 31,342 to 588 VM instructions.
The shared domain still includes disconnected guard nodes within candidate files, so its preparation cost remains linear in that domain.

The added catalog coverage also exposes expensive surrounding outcome programs.
Shared recursion does not resolve the ordered root-program overhead in F633 and the added ASYNC rules.
Further work must preserve finite allowances while batching those total structural prefixes and their costs.
`benchmark/sql-relational-chain-integration.json` records the SQL, explained plans, growth samples, and grammar measurements.
The complete-job 100-execution gate remains a separate requirement before production replacement.

Final qualification passes 263 relational tests and all 285 preceding SQL/VM controls.
All 76 admitted grammars match native results on all 595 fixtures, with a positive witness for each grammar.
Frozen B023 retains 246,364 VM instructions and passes its existing CPU and query-shape gates.

## Catalog continuation: ordered downward recursion

The test compiler now admits pure ordered child recursion with a finite guard-kind proof.
The component accepts a total capture-free base followed by a guard and a neighbor-child call to the same utility.
SQL builds a shared forest and calculates each invocation's visited preorder interval.
The first successful base truncates that interval, so failed siblings retain their exact utility costs.

Child eligibility follows native semantics.
Two-valued matching uses the native kind filter before each child invocation.
Three-valued matching disables that filter.
An unbounded base also requires all immediate children, including anonymous tokens.
Tests cover both filtered and unfiltered entry costs.

Logical-depth failures use a window sweep over entry markers and invocation queries.
Each invocation receives the first forbidden entry before its successful or exhausted subtree boundary.
This preserves the precedence between work and depth failures.
The compiler caps distinct caller-depth domains at 16 per component.
Native stack-byte equivalence remains unqualified.

The diagnostic catalog admits 78 grammars, up from 76, and passes all 78 on 595 fixtures.
B110 and B112 are the new admissions.
Three format policies advance to parsed-tree refusals, and SIM910 advances to a binding-site refusal.
The remaining first-refusal counts are:

| Missing contract | Grammars |
| --- | ---: |
| General recursive outcomes | 43 |
| Parsed-tree composition | 12 |
| Format-argument composition | 6 |
| Binding-site composition | 4 |
| Parameterized utility calls | 2 |
| Recursive negation or region dependency | 1 |

The next recursive boundary needs semantic or capture-bearing base outcomes with exact reached work and memo behavior.
Parsed-tree and binding-site operators also need explicit child transactions before their exposed grammars can pass.
Parameterized call frames need private capture namespaces and call-site memo identities.
PIE796 additionally depends on recursive semantic bases, so call-frame support alone cannot close its two clauses.
The full-coverage, complete-job execution, resource, and production-cutover gates remain closed.

`benchmark/sql-relational-downward-integration.json` records the new SQL, explained plans, depth and width samples, and first-refusal advances.
The final frozen build passes 272 relational tests and preserves all 285 preceding SQL/VM controls.
The shallow-forest guard reduces the depth-128 sample from 201,886 to 175,967 VM instructions.
Exhausted calls remain at 600 instructions as the unused guard domain grows.

Final catalog qualification passes all 78 admitted grammars on all 595 fixtures, with a positive witness for each grammar.
The preceding 76 retain exact SQL hashes, VM counts, and output hashes.
B110 and B112 use one matching statement each, with zero frame publications.
Frozen B023 also retains its exact SQL/VM shape and passes all existing prepared-execution gates.

## Larger catalog batch: semantic bases and child transactions

This batch advances admission from 78 to 101 of 146 grammars. The catalog checkpoint passes exact native parity on 595 files. Every admitted grammar has a positive fixture witness.

The implementation adds these contracts together:

- Binding resolution returns target facts and an ordered occurrence list. SQL evaluates site, value, annotation, and occurrence children.
- Parsed trees use registered subfiles. SQL evaluates supported children, including annotation patterns and literal values.
- Parameter calls isolate private captures. Memo keys use native utility identities, logical capture names, and active binding context.
- Format analysis returns ordered candidates. SQL handles candidate tests, incomplete inputs, unknown, and capture rollback.
- Recursive wrappers accept nonmemo semantic and capture-bearing base programs. A shared structural forest supplies ordered entries without copying ancestor/descendant pairs.
- Nested pure components receive the runtime utility depth of their semantic caller.

Parsed-document utility calls deliberately bypass memo storage. Native does not memoize those calls. They still spend entry work and contribute to utility depth.

The previous four-walk limit no longer controls admission. The compiler instead limits active typed returns to 128 cells. Existing emitted-operator and SQL-byte limits still apply. Tests admit 16-level examples and refuse a 32-level example that exceeds this resource bound. This is a compiler resource contract, not proof of native stack-byte equivalence.

The semantic recursion width sample uses constant SQL size as input grows. Removing a redundant one-arm alternative reduces its largest sample from 105,788 to 92,659 VM instructions. SQL size falls from 8,018 to 7,308 bytes. Deeper traversal samples scale nearly linearly with independent root count. Their preparation cost still grows with nesting depth and is reported separately.

The remaining 45 first refusals are:

| Missing contract | Grammars |
| --- | ---: |
| General recursive outcomes | 36 |
| Recursive parameter calls | 2 |
| Parsed exported spans or host hooks | 4 |
| Cyclic base dependencies | 2 |
| Recursive negation or region dependency | 1 |

The next implementation boundary is memoized recursive utility return. Begin with a failing native/SQL test that revisits a completed false sibling before a later successful sibling. Add work exhaustion and a cache hit that does not fit the remaining logical depth.

SQL must publish each completed sibling before evaluating the next sibling. A successful descendant must then publish its open ancestors in return order. Each answer needs exact replay cost, cache-fill exclusion, and peak depth. A flat preorder scan cannot reconstruct that publication order after execution. The next scheduler therefore needs explicit entry, base, and return phases with narrow control metadata.

Coverage gains do not establish native-speed parity. Newly admitted E721 still exceeds 100 million VM instructions in the diagnostic catalog run. ASYNC221 and ASYNC230 exceed 38 million. Their surrounding ordered root programs need further batching and shape work. Keep these measurements visible while completing contracts.

The full-coverage, complete-job 100-execution, resource, and production-cutover gates remain closed. This batch does not add a production opt-in path.

Final qualification passes 308 relational tests and all 285 preceding SQL/VM controls. All 101 admitted grammars pass the complete fixture batch. The preceding 78 retain identical SQL, VM counts, and outputs. Frozen B023 retains 246,364 VM instructions and 10,232 SQL bytes. Its prepared median CPU time is 4.085 ms against the 32.029 ms reference. All existing performance and shape gates pass.

`benchmark/sql-relational-broad-catalog-integration.json` contains the batch evidence and new grammar SQL. The coverage ledger records 45 remaining refusals. These results qualify this larger test-compiler batch; they do not open the production-cutover gates.

## Broad recursion and capture batch, 2026-10-24

The qualified test compiler admits 135 of 146 grammars, up from 101. All 135 pass native parity across 596 files with positive witnesses. The frozen suite passes 385 relational tests, both native lint configurations, and all 285 prior SQL/VM controls.

This batch implements the recursive return boundary described above. Completed utility answers carry their replay cost and peak depth. Recursive alternatives retain their original order. Numeric continuations hold control fields; capture inputs stay in the enclosing demand. Index seeks resume after the completed child's subtree endpoint.

It also adds pure structural recursion, mixed finite semantic regions, parsed span exports, host hooks, captured binding regions, file conditions, and named sibling classification. Span capability follows parameter aliases transitively. Sibling cache lookups and fills complete before the next demand starts. Adjacent pure predicates share one emitted expression where their effect contract permits it.

The remaining contracts now form two larger implementation tracks:

| Track | Current first refusals | Required change |
| --- | ---: | --- |
| Recursive execution contexts | 6 | Explicit recursive parameter inputs and cross-document capture returns. File identity and export maps must cross the continuation boundary. |
| Shared finite functions | 5 | Emit reusable utility bodies with typed arguments and separate native memo identities. Keep the 256-operator resource bound. |

The finite-function track must start from actual emitted duplication. SIM905 repeats pure bodies in separate keyword and positional branches. SIM101 repeats equivalent helper structures with different capture slots and native utility identities. SIM113 can reach a fallback helper after the first branch fails before that helper executes. Neither unconditional hoisting nor a source-plan cache removes these copies safely.

The next shared-function contract therefore needs lazy calls with typed inputs, a return continuation, and original memo identity. Its gate must compare full outputs, finite work, reached probes, unknown propagation, and VM counts against the current compiler. Preparation time stays outside execution timing. Captures and parser exports must not become serialized frame copies.

The six recursion refusals remain explicit. Do not substitute native child matching, relax progress checks, or raise resource limits to admit them. Complete-job batching and production replacement remain separate gates.


Final evidence is in [the recursive catalog integration report](../benchmark/sql-relational-recursive-catalog-integration.json). The source and executable hashes are in [the qualification record](../benchmark/sql-relational-cutover-evidence.json). The coverage ledger retains all 11 refusals, so the full-coverage gate still fails as intended.

Of the preceding 101 grammars, 96 retain exact SQL, VM counts, and outputs on the same expanded fixture inventory. Five change physical shape while retaining exact outputs. B507 rises from 22,062,220 to 25,073,720 VM instructions. ASYNC221 rises from 41,775,854 to 50,072,645. Exact depth and memo bookkeeping contributes to these costs. Do not describe this coverage batch as a catalog-wide performance improvement.

Frozen B023 retains 246,364 VM instructions, 10,232 SQL bytes, one matching statement, and zero frame publications. Prepared median CPU is 4.479 ms against 32.460 ms for the reference. Preparation remains a separate one-off cost. These results keep the existing performance and shape gates open for this workload; they do not qualify the complete-job execution budget.

## Catalog completion and enforced regression budgets, 2026-10-24

This checkpoint supersedes the 135-grammar admission and regression acceptance above. The compiler now admits all 146 catalog grammars. The final qualification uses all 597 fixture files, including a new positive semantic-annotation fixture.

The eleven former refusals required three changes. Captured recursion carries persistent capture handles through typed continuations. Parsed recursion carries document identity and explicit exports. Finite utility functions share their bodies while preserving each call's native memo identity. SQL still composes child rules; atomic native facts do not evaluate whole child programs.

Physical selection has one automatic entry point. It retains an inline plan when that plan fits. After a resource refusal, it tries shared finite functions, then shared recursive functions. Only the selected SQL executes. For the largest plan, immutable instruction metadata replaces repeated Search SQL templates. Existing plans retain their cheaper specialized traversal shapes where they satisfy the limits.

The complete catalog gate is:

```sh
python3 benchmark/relational-catalog-qualification.py /path/to/frozen/test-binary /path/to/new/output-directory
```

It requires 146 admissions, native fixture parity, a positive witness for each grammar, and every pinned VM ceiling. Each grammar must execute one matching statement and publish no capture frames. Every query must fit 256,000 SQL bytes. Ordered plans must also fit 256 source operators, 256 finite instructions, 256 emitted operators, and 128 active typed return cells. The existing set and single-leaf answer compilers retain their own physical contracts. They do not report fictitious ordered counters.

The driver pins the executable, source files, fixtures, and budget file. It records preparation separately from execution and saves every SQL query and EXPLAIN plan. A missing budget, changed output, unknown compiler scope, or changed input rejects the run. Existing ceilings cannot rise to accommodate a coverage change.

Regression closure does not qualify the remaining execution shape. SIM905, B615, and F632 still account for roughly one billion VM instructions together. Their continuation traffic is the next measured target. A change there must reduce repeated control transitions while preserving native work, order, cache replay, captures, and reached failures. Preparation improvements cannot satisfy that execution gate.

Production replacement, native physical stack-byte equivalence, aggregate storage bounds, deadlines, and complete-job batching within 100 executions remain separate failed or unqualified gates. One statement per grammar does not satisfy the whole-job budget.

Final qualification passes all 146 admissions and native fixture comparisons, all VM ceilings, all applicable resource limits, and all 285 prior SQL/VM controls. The full suite reports 459 passing tests and no failures. Both native lint configurations pass. The original 135 grammars use 2.5% fewer VM instructions. The complete catalog still uses 2.390 billion instructions, so the performance transition is not complete.

See [the complete catalog integration report](../benchmark/sql-relational-complete-catalog-integration.json), [the qualification record](../benchmark/sql-relational-cutover-evidence.json), and [the coverage ledger](../benchmark/sql-relational-cutover-coverage.json). Full EXPLAIN plans and native file traces are preserved in `benchmark/sql-relational-catalog-trace.json.gz`.
