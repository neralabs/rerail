# Structural SQL compiler contracts

This specification records the executable structural spikes authorized after commit `2dc203cf`. It does not qualify a production cutover.

The transition has three separate obligations: compute reusable answers, preserve native observations, and publish clause results. Statement packing solves a fourth problem: bounded SQL preparation and execution counts. Packing does not reduce repeated computation inside each statement.

## Relation contracts

A producer has an explicit input relation and output schema. Its identity includes the normalized operation, language, document grammar, and semantic context. A caller has a separate native utility identity and ordered invocation position.

| Field | Required meaning |
| --- | --- |
| Demand key | Document, node, host context, scope, and each live capture input |
| Input type | Engine capture tag and payload, including absent values |
| Output key | One completed answer per demand, or an explicitly ordered witness relation |
| Truth | False, true, unknown, or an explicit reached failure |
| Captures | Typed output values plus a write mask for rollback and export |
| Cost | Native work for the proved region, with cache-fill work separate from replay work |
| Depth | Relative entered peak and the first rejected-entry prefix |
| Effects | Ordered callbacks, cache transactions, parsing, and capture writes |
| Cardinality | Uniqueness and the maximum witnesses retained per demand |
| Lifetime | Statement, clause/file, parsed document, or recursive component |

Ambient three-valued mode belongs to producer and memo identity. It can disable native candidate pruning and make recursive searches visit punctuation. Extracting a helper into a new grammar can change that mode. Transfer tests must retain the original full-clause context as well as isolated helper controls.

Result totality and execution totality are separate properties. B002 can have a total result relation while native utility calls still consume work. A work-aware job must not use a set-only answer as its complete execution contract.

SQLite storage classes alone cannot identify Engine capture types. A node ID and a count can share an integer representation but differ semantically. A span uses an encoded blob and can also carry a node shadow. Memo canonicalization must preserve the native rule for that shadow. Producer keys must retain their Engine tags. NULL, empty text, empty blob, and a zero byte remain distinct.

A caller can share physical computation without sharing its native memo identity. Physical input projection and native memo-key projection need separate proofs. An unread held capture can still change native memo hit behavior. That difference can change the reported peak when work runs out. Preserve the complete native memo context unless its own scope analysis permits projection. A completed memo answer needs truth, replay cost, relative peak, and capture behavior. A cache fill can make execution cost exceed replay cost. Insufficient depth prevents reuse even when the answer exists.

A native memo key includes the utility origin, three-valued mode, exclusion and following-origin context, and clause owner when the computation reads clause state. These fields can differ from the physical producer key. Capture dataflow needs read, mention, and write masks, plus withheld-capture state. Flattening an original `All` or `Any` boundary requires a proof that rollback and withholding are unchanged.

Missing rows also need an explicit contract. An accepted scalar demand produces exactly one answer or reached failure. An unsupported demand produces an admission refusal. An unreached demand produces no event. The emitter must not silently convert a refusal or an unreached event to false.

## Logical operations

| Operation | SQL shape | Required proof |
| --- | --- | --- |
| Restrict demand | Selective joins or indexed membership | The restriction cannot skip native work or effects |
| Project live inputs | Narrow keyed relation | Removed inputs cannot change answers, captures, work, or context |
| Distinct demand | `SELECT DISTINCT` over complete typed keys | Logical invocation identity remains available separately |
| Produce answer | Closed CTE over distinct demands | No free per-caller correlation remains |
| Join answer | Keyed join with declared uniqueness | Missing, duplicate, and unknown rows have explicit behavior |
| Exists | Semijoin or `EXISTS` | No selected capture or witness identity is needed |
| First witness | Ordered reduction before the caller join | Order matches native traversal, including comments and punctuation |
| Ordered Boolean composition | Relational truth and reached-prefix expressions | Short-circuit order, unknown propagation, and rollback agree |
| Pure cost summary | Truth, cost, and relative peak relation | No mutable input, throwing callback, or variable cache-fill cost |
| Effect boundary | Small ordered frontier | Only reached callbacks execute; failure prevents later effects |
| Recursive component | Narrow recursive CTE | Recursion state excludes surrounding finite composition |
| Parsed document | Reached parse plus document-local relations | Grammar, host chain, exports, and parse failure remain explicit |
| Publish | Per-file first-failure reduction | Failed clauses discard provisional results and preserve work |

Ordinary conjunction, alternatives, and negation must not allocate numeric continuations. A continuation can remain for an actual recursive call or an ordered effect boundary. Its payload must contain only live state.

## Work and depth

Incoming logical depth is a demand column. The emitter must not clone the entire pure program for each possible incoming level. Dynamic-depth summaries retain the same failure prefixes as constant-depth summaries.

A completed scalar cost does not describe every failure prefix. The emitter must distinguish complete execution, insufficient work, and rejected utility entry. The native order determines which failure happens first.

Utility failure is not always an immediate abort of every remaining observation. Native alternatives and child searches can still recall completed utility memos after a sticky limit. A fitting cached answer can raise the recorded peak before work spending is refused. The failure frontier must preserve those reached memo-only followers. It must not execute their bodies, spend more work, change captures, or publish incomplete answers.

Failure successors can differ from normal successors. For example, a failed first term skips the remaining terms of its conjunction, then reaches later alternatives. The compiler needs explicit utility-call failure edges and the original capture context at those edges. Ordinary Boolean operations still do not need individual continuation records.

For a region with a proved fixed rejected-root cost, prefix sums can replace one transition per rejected root. The boundary event still needs exact execution. An exhausted prefix must guard the event query itself. A final `CASE` cannot undo an already reached callback.

Pure summary production can speculate only over operations with no observable effects. Value evaluation and parsing remain reached operations. A correct result set alone does not establish this contract.

Logical-depth tests use the original matcher with an explicit incoming level. They check cold and warm answers near levels 254–256. These tests do not establish parity for native physical stack limits.

## Physical planning

A native utility cache can be warm inside a cold caller. Utility interval events must distinguish entry, cache-fit replay, and completion publication. A scalar cost alone cannot reproduce every failed-call peak.

Materialization belongs after demand restriction and witness reduction. A materialized CTE inside a correlated scalar query still executes for each correlation. It is not a shared producer.

SQLite can expand a shared CTE graph beyond its reference limit during preparation. A MATERIALIZED hint does not necessarily bound this expansion. The planner must insert a small number of real table boundaries when the closed graph exceeds preparation limits. Every table population and index build remains part of measured execution.

The planner must compare materialized joins, materialized scalar probes, semijoins, and inlining. Measurements include producer evaluations, VM instructions, intermediate cardinality, and SQL bytes. Preparation time remains separate from repeated execution.

The demand matrix varies file count, caller count, capture diversity, and recursion. A separate membership matrix compares actual `EXISTS` with joins. These experiments select physical alternatives; native-policy tests establish semantic transfer.

The pure-cost spike retains broad construction as a control. Its restricted variant constructs summaries only over demanded nodes and their descendant closure. This closure is sound for its admitted language, which excludes ancestor and sibling reads. On the unrelated-node control, VM instructions decrease from 2,840,462 to 6,231 with identical answers, costs, and peaks. Other traversal operators require their own closure proof.

## Recursion and parse boundaries

A structural recursive component carries document, node, component identity, and live outputs. It must not carry a generic program counter for every Boolean operation around it.

Ordered recursion needs first-success, accumulated unknown, failure, and caller state. Shared pure classifications can supply its transitions. Observable memo and callback operations remain in native order.

Parsed nodes belong to a document identity and host chain. Identical annotation text in separate hosts does not authorize sharing exported spans or scope-dependent results. Parsing itself is an ordered boundary. Successful documents can supply shared structural producers afterward.

The parsed spike preserves the original F632 first arm. Rejected-root compression requires a source proof for its initial pure guard and unique utility use. Original-file memo reads cannot be discarded without that proof.

## Job composition

The spike assigns terminal IDs before lowering. It does not reparse generated SQL to replace numeric IDs. Each closed branch receives separate clause, memo, component, and scope identities.

Packing obeys both limits: at most 32 branches and at most 256,000 SQL bytes per statement. Every packed statement receives the full compatible file batch. The complete 146-grammar job must execute at most 100 SQL statements, including schema, reset, population, index, and matching work. Preparation and EXPLAIN remain separate.

A production implementation should use typed terminal IDs with explicit allocation. The spike uses temporary terminal prefixes to test namespace correctness. Those placeholder terminals are not installed for execution.

The work-aware catalog packing spike passes all 2,477 native output rows and their remaining work in 24 matching statements. Its largest statement is 255,548 bytes. VM work is 2,777,339,105 instructions. The earlier output-only 23-statement result missed work charges. These measurements establish batching feasibility, not an acceptable computation shape.

Cross-rule sharing requires compatible producer keys and contexts. Closed branch packing alone retains duplicate work. A separate shared-producer control checks that compatible rules consume one physical producer.

Physical table boundaries must be grouped by compatible relation schema and execution lifetime. A producer ID distinguishes independent meanings inside a shared table family. Logical rule IDs remain in a separate consumer relation. One CTAS or index per utility would recreate the execution-count problem.

The staged-family control varies two node-fact families across 2, 32, and 146 logical rules and 1 or 16 files. It compares repeated rule-local producers with a shared candidate table and a rule-demand table. It counts both table populations, both indexes, and final matching. This control specifies how physical cuts can stay bounded as rule count grows; it does not establish arbitrary captured-schema compatibility.

## Common compiler mapping

The reference emitters establish interfaces for a common compiler. They do not authorize keeping separate policy selectors in production.

| Compiler step | Concrete representation | Executable reference |
| --- | --- | --- |
| Analyze source | Effect class, live capture masks, utility SCC, traversal closure, original evaluation order | Pure-cost source admission; SIM905 source manifest; F632 refusal controls |
| Allocate identities | Typed terminal, clause, utility, document, and component IDs before rendering SQL | Job namespace and collision controls |
| Collect demands | `Demand(producer, document, host, scope, node, typed_inputs)` plus separate invocation order | Capture demand matrix and B615 SQL producer |
| Build pure answers | `Answer(demand, truth, outputs, cost, peak)` over a closed demand relation | Constant and dynamic-depth cost emitters |
| Reduce witnesses | Ordered candidate relation, first decisive position, reached-prefix cost | SIM905 keyword, positional, and receiver profiles |
| Preserve memo behavior | Utility entry/completion intervals with separate fill and replay costs | B615 warm-inner/cold-outer counterexample and interval frontier |
| Schedule effects | Reached event relation plus narrow work/depth/memo state | Numeric Value frontier, binding frontier, parsed-document frontier |
| Place physical cuts | Closed producer CTAS and necessary indexes, each separately explained and measured | SIM905 stages and stage-accounting controls |
| Publish | First failure per clause/file, provisional-result discard, final remaining work | Publication oracle and rejected-suffix diagnostics |
| Pack job | Compatible file batches, private logical owners, bounded statement size and count | Complete catalog packing and cross-owner parse-cache controls |

The optimizer should start with a closed producer graph. For each edge, it records the required key, retained columns, estimated distinct demands, and expected reuse. An edge must not introduce a free reference to a caller row unless it represents an ordered effect.

Node-kind partial evaluation comes before row production. A constant truth/cost/peak stratum becomes a direct projection over its demanded nodes. Only unresolved strata enter the structural relation. Materializing each Boolean expression over the entire descendant closure is an intentionally rejected physical baseline.

Acyclic single-use expressions can remain inline. Reused producers become closed keyed relations. Recursive components should share node summaries across roots where their context keys agree. Flattening a complete recursive trace for every root can grow with roots times depth; interval correctness alone does not justify that physical shape.

The planner counts CTAS and index execution as matching work. Diagnostic EXPLAIN and output-cardinality queries are recorded separately. Query preparation is a reusable cost and stays separate from repeated execution.

## Required changes to the current emitter

The common compiler should make these changes in order:

1. Replace finite Boolean function states with same-demand projections. Materialize utility and traversal boundaries only when their consumers justify reuse. The literal projection spike supplies the reference shape.
2. Split budget-only failure from ordered effects. A pure region with a work cost does not need one recursive transition per Boolean operation.
3. Give every producer a closed demand input. The current dynamic cost producer supplies a correctness control, but its per-expression materialization is not the target emitter.
4. Collect shared recursive nodes and call edges once per semantic context. Keep native memo entry, completion, replay, and failure as separate events.
5. Allocate relation and terminal identities before SQL rendering. The emitter must own CTE definitions and dependencies explicitly. It must not recover dependencies by reparsing generated SQL.
6. Partition physical cuts by schema, lifetime, SQL limits, and the remaining statement budget. Use shared staging table families when multiple rules need the same boundary.
7. Preserve the current publication contract while replacing its inputs. Failed clause/file rows carry no node or captures. Published rows retain exact remaining work.

The first production candidate must generate the accepted SIM905 and F632 shapes through these common operations. Policy-name recognition and handwritten reference selectors are not production solutions. Keep one production compiler selector. Reference and control variants remain test-only.

Prepared execution needs its own contract. Empty schema creation and statement preparation are one-off costs. Every job still counts resets, population, index maintenance, and final matching. The prepared-family spike changes the rule batch between executions and checks that no stale rows survive.

## Parse cache ownership

Packing exposed a native-work bug that output-only comparisons missed. SQL parks file scopes across clause branches. A parse cached by an earlier owner previously suppressed the next owner's first-fill work.

The fix keys literal parses, value parses, and decoded values by clause owner. All scalar Value leaves also install their allocated owner. Parse cache reads also mark the utility answer as dependent on clause state. The dependency marker consumes no work. Within-clause reuse remains warm; a different clause has a cold logical cache. Cloned Reading IDs and PHP formats decoded from concatenated strings have explicit native-oracle controls.

This implementation can duplicate immutable parse trees across owners. A future shared tree store must separate physical tree reuse from each owner's logical first-fill charge. Sharing parsed content cannot merge host chains, exported spans, or scope identity.

## Qualification and implementation order

The executable runner is `benchmark/relational-structural-spikes.py`. It records each suite, preserves failed gates, and pins sources and the test executable. The catalog qualifier remains separate.

1. Establish cost, effect, typed-key, recursive, and parsed-document contracts with native oracles.
2. Measure the complete SIM905 reference against the original 597-file batch.
3. Require at most 44,908,386 VM instructions and 158,186 typed child-result transfers for that reference.
4. Transfer captured recursion to B615 and parsed recursion to F632.
5. Lower the accepted reference through the common relation IR.
6. Require generated SQL size and VM count within 25% of the accepted reference.
7. Retain all 146 native catalog comparisons, existing VM ceilings, and the whole-job execution budget.

The current production compiler remains the baseline until these gates pass. A rejected reference changes the design; it does not justify lowering the gate.

## Experimental dispositions

All eight experimental areas now have executable contracts. The final evidence report records the frozen run and source hashes. These dispositions apply to references, not a production compiler cutover.

| Area | Selected result | Remaining compiler obligation |
| --- | --- | --- |
| S0: attribution | Separate typed transfers, actual continuation traffic, producer evaluations, stage rows, and VM instructions | Retain operator provenance in common lowering. |
| S1: costs and effects | Dynamic-depth pure summaries and reached effect frontiers pass native controls | Prove closure, failure prefixes, and memo-only failure successors for each admitted operator. |
| S2: demands | Closed typed producers; inline for measured single-use cases, materialize for measured reuse | Keep physical key projection separate from native memo identity. |
| S3: SIM905 | 35,866,088 VM, 31,103 typed transfers, 12 statements | Generate this shape without policy-name recognition. |
| S4: B615 | Shared utility graph, indexed parent links, ordered binding and failure boundaries | Complete alias, bare-call, nth, and mixed-capture composition before claiming whole-policy replacement. |
| S5: F632 | 19,227,542 VM, 41,489 typed transfers, one statement | Generate the parsed-document frontier through the common compiler. |
| S6: job | All 146 grammars and 2,477 native rows with exact published work in 24 statements | Combine packing with shared computation; packing alone uses 2,777,339,105 VM. |
| S7: specification | Typed contracts, compiler-step mapping, selected plans, and explicit gates | Implement one common compiler and pass reference-relative gates before cutover. |

### Measured reference shapes

The SIM905 baseline uses 449,083,864 VM instructions and 1,581,869 typed transfers. The accepted reference uses 35,866,088 and 31,103 respectively. Continuation pushes decrease from 426,911 to 2,539. The reference contains 50,384 SQL bytes across 12 statements; the largest statement has 22,123 bytes. These counts include table population and index construction. Preparation, EXPLAIN, and diagnostic cardinality reads remain separate.

The complete fused SIM905 graph has 46,387 SQL bytes. It fails SQLite preparation with `too many references to "node": max 65535`. Keep real table boundaries at closed producer cuts. The earlier missing-CTE-name failure was a generator defect and supplies no evidence for this decision.

The F632 baseline uses 240,957,037 VM instructions and 1,440,173 typed transfers. The reference uses 19,227,542 and 41,489. Continuation pushes decrease from 131,436 to 8,544. The complete reference statement has 29,948 bytes at the catalog work allowance. The previous 7,831-byte figure came from a later control that overwrote its SQL artifact. The native and VM measurements remain valid. The corrected 125% SQL-size gate is 37,435 bytes; the original archive remains unchanged. Native outputs, remaining work, reached parsing, exported identities, and tested logical-depth boundaries agree.

The result-only SIM905 reference uses 5,198,507 VM instructions. It speculates over Value checks and lacks the full work/effect contract. It remains a result-only structural comparison, not an accepted replacement.

### Captured recursion decision

Flattening a trace for every root repeats shared recursive suffixes. At depth 32, 66 demands produce 1,320 interval rows for 70 unique utility answers. The selected reference represents the 70 answers and 69 call edges once. Memo entry, completion, replay, and failure remain ordered operations.

Precompute parent links and leading/trailing punctuation costs in one indexed relation. The recursive answer query then joins this relation directly. It must not perform virtual-node joins or correlated child counts at every recursive step.

The parent-link variant wins all eight measured depth, root-density, and ambient-mode cells. At depth 32, two-valued all-root execution decreases from 333,370 to 228,119 VM instructions. Three-valued all-root execution decreases from 1,501,132 to 1,174,378. These totals include table population, indexes, and failure-follower construction. Unique-key, forced-join, and ANALYZE variants all lose to the original baseline.

The recursive answer producer itself decreases from 118,434 to 4,691 VM instructions in two-valued mode. In three-valued mode, it decreases from 351,213 to 13,279. The remaining three-valued all-root cost concentrates in ordered frontier consumption: 760,550 instructions. Failure-follower construction uses another 105,760. This is the next measured optimization target; it is not evidence that arbitrary recursion has been solved.

The structural helper admits no real corpus callees in its narrow parenthesized-wrapper domain. A separate binding transfer passes 19 real imported AutoModel/AutoTokenizer paths, including original three-valued context. Those are positive helper transfers. Neither result is a complete B615 performance claim.

### Bounded staging and prepared reuse

Across 146 pure rules and 16 files, shared producer visits decrease from 7,008 to 96. Including two populations and two indexes, execution decreases from 391,113 to 114,484 VM instructions in five statements. Staging loses in the two-rule case. Use estimated reuse to choose a cut, rather than staging every producer.

The prepared-family control creates empty schema and prepares statements once. Changing batches from 2 to 146 to 32 rules preserves native output and leaves no stale rows. The 146-rule job uses five executions and 105,676 VM instructions. Resets and population count as job execution; preparation does not.

### Hard implementation gates

1. Generate SIM905 and F632 through shared logical operations, without handwritten policy selectors.
2. Keep total program SQL bytes and VM instructions within 125% of each accepted reference. Total bytes include all stages.
3. Preserve exact native output, work, reached callbacks, captures, memo behavior, unknowns, and tested logical depth.
4. Keep the complete job within 100 executions. Count resets, population, index maintenance, and matching.
5. Preserve all 146 catalog comparisons and existing per-grammar VM ceilings. Do not raise ceilings to admit regressions.
6. Retain the 256,000-byte statement limit and existing per-clause limits: 256 operators and 128 nested return cells.
7. Keep one production compiler. Reference alternatives remain test-only.

Physical stack-byte equivalence, arbitrary mutual recursion, complete captured-schema sharing, aggregate storage limits, deadlines, and production incremental invalidation remain unqualified. These require explicit gates before broader production admission.

## Reproduce the evidence

Build the engine test executable with `cargo test -p rerail-engine --lib --profile loop --features unbounded-work --no-run`. Use the executable path printed by Cargo:

```sh
python3 benchmark/relational-structural-spikes.py /path/to/test-executable /new/spike-output-directory
python3 benchmark/relational-catalog-qualification.py /path/to/test-executable /new/catalog-output-directory
```

Both runners require a new output directory. They pin their inputs and reject source changes during execution. The structural runner requires every registered suite to pass. The catalog runner separately checks all 146 grammars and existing performance ceilings.

`benchmark/sql-relational-structural-spikes.json` contains the compact results. `benchmark/sql-relational-structural-trace.tar.gz` contains SQL, EXPLAIN output, counters, native comparisons, rejected checkpoints, and validation logs. Its manifest records file hashes and the measurement environment. Raw timing is diagnostic; the acceptance gates use VM work, statement counts, native observations, and source size.

## Common compiler integration status

The common compiler now contains demand-scoped pure producers, metadata proofs, static Boolean routes, parsed frontiers, and bounded job packing. Physical cuts retain completed dependency summaries in keyed tables. Memo answer sets and ordered logical entries have separate result types. The scheduler uses one recursive arm. Proved silent false search candidates do not enter that scheduler.

Immutable batch statistics select rare anchors, direct-edge join order, and storage for pure relations. These decisions preserve logical work and effect order. Unknown statistics remain distinct from a known zero count.

The integration report is `benchmark/sql-relational-common-compiler-integration.json`. Its archive contains pinned sources and fixtures, emitted SQL, EXPLAIN output, loop traces, and native comparisons. The report identifies its measured checkpoint. The journal records subsequent experiments until their qualification completes.

The common compiler remains test-only. SIM905 still fails the reference-relative VM and SQL-size gates. Existing catalog ceilings and the whole-job execution limit do not substitute for those gates. Production cutover remains unqualified.
