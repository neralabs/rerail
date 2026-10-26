# Native operation batches and shared matcher work

The engine batches native operations across programs and documents.
A lane identifies a program, document, and preorder node. SQL transports demands and completed typed results.
The CPU executor groups lanes by operation, advances active lanes, and scatters results into the original request order.
Its structural and pattern kernels do not call `AstMatcher` or install thread-local matcher state.
Semantic fact projections install native file and clause state for each operation group.

The [journal](sql-matcher-fragment-transition.md) records measurements, rejected assumptions, review findings, and corrections.
The [preceding qualification](../benchmark/sql-matcher-batch-integration.json) measures the earlier file-first implementation at commit `92b1978e`.
That implementation still called the native matcher once per residual root.
Its SQL submission count did not measure native execution granularity.

## Ownership and reuse

| Layer | Owner | Responsibility |
| --- | --- | --- |
| Generic prefix planning | `producers/prefix.rs` | Ordered semantic keys and reusable producer stages |
| Predicate identity | `matcher/factor.rs` | Closed predicate proof, complete utility definitions, residual construction |
| Operation lowering and execution | `matcher/wave.rs` | Program interning, numeric lanes, operation cohorts, shared navigation, bounded caches |
| Capture-bearing execution | `matcher/cohort.rs`, `matcher/pattern_batch.rs` | Capture frames, utility calls, ordered continuations, pattern operations |
| Semantic projections | `matcher/cohort_host.rs` | Typed facts, child demands, parsed-document contexts |
| Typed transport | `sql/fragments/batch.rs` | Consumer domains, two-axis tiles, file allowances, residual execution, result publication |
| SQL generation | `sql/engine/planning.rs` | Candidate access paths, demand aggregation, final projection |
| Tape navigation | `tape.rs` | Native node links and sibling starting-position semantics |

The sibling `Pulling-it-all-together` specification separates local transducers, relational facts, and native fact producers.
This implementation preserves that separation. The sibling worktree remains unchanged.
Its document-table and `(program group, document, root)` task model guide the lane interface.

The generic prefix planner remains available to other producer backends.
The production matcher now interns closed operation graphs and shares completed keyed facts within its executor.
It does not execute the old generic prefix trie in its hot path.
The wave module has no dependency on SQLite or catalog rule names.
An index-provider function supplies existing kind postings without coupling the executor to SQL.

## Native execution

Each submission contains at most 2,048 lanes. The driver tiles both the file and program axes.
Operation cohorts can contain several programs and documents.
Active masks preserve ordered short-circuit behavior while removing completed lanes.
Programs with shared closure or sibling-table fills admit roots in file-clause order.
Other programs retain full-width root admission. Independent owners remain active together.
The result records truth, logical utility work, maximum utility depth, and incomplete execution.

The supported exact operations are kinds, regexes, booleans, tree relations, stop predicates, counts without exports, and closed recursive utilities.
Literal regex forms become byte predicates after syntax-tree analysis. Other regex forms use the existing regex engine.
Complete closed programs can publish results without scalar matcher calls.

Navigation has several physical forms:

- Neighbor and stop-rule walks advance one cursor per active lane.
- Ancestor and sibling end-walks share a demanded suffix graph. Each suffix is evaluated once.
- Filtered descendant end-walks seek preorder postings within each requested subtree interval.
- Repeated unfiltered descendant walks can materialize a zero-cost local predicate over the demanded subtree union.

The last form starts after seven failed probes.
It requires sixteen active requests and at least four times as much remaining traversal as unique nodes.
This condition retains short-circuit execution for early matches and avoids unconditional whole-file materialization.
Completion intervals distinguish a proven negative from an uncovered node.
Utilities, exported counts, navigation inside the child predicate, and semantic operations do not use this materialization.

Sibling walks use the native byte-positioned initial anchor, including document roots and zero-width nodes.
After that initial position, suffix continuation follows structural sibling links.
Internal suffix keys cannot collide with user-root keys.

## Mixed programs and captures

The compiler also builds negative admission proofs inside mixed alternatives and utility definitions.
A false result proves that native execution stops before reaching an opaque operation.
A possible match admits the original program; it does not publish a finding or suppress native evaluation.

Conjunction proofs stop at the first opaque term. Every disjunction branch must have a proof.
Partial proofs cannot pass through negation or move across capture and semantic operations.
Rejected paths charge their proven utility work. Admitted paths retain the original program's work charges.

Capture-dependent matching now runs through a second native operation stage.
It handles ordered alternatives, capture frames, utility calls, constraints, counts, and enumeration.
Pattern matching dispatches kind, text, capture, and navigation operations across ready lanes.
Binding and parsed-literal predicates project facts, then submit child demands to the same scheduler.
Atomic semantic predicates read existing native facts. A runtime audit detects scalar matcher calls inside these projections.

Closed subprograms inside capture graphs use the numeric executor and retain navigation facts across streaming arrivals.
Equivalent pattern plans share results for identical immutable input environments, documents, and nodes.
Constrained patterns execute pattern and constraint stages in batches without general child frames.
Conjunctions retain their capture transactions and unknown-capture dependencies in the general operation scheduler.

Enumeration carries explicit exclusion sets and rematches from the original input environment.
Parsed documents carry explicit parent bridges and coordinate maps.
No semantic callback runs an entire child matcher.

This remains a CPU executor. It does not implement a GPU backend.
A device backend still needs packed operation tables, device text predicates, capture storage, and queue execution.
Semantic child submissions currently use recursive cohort calls on the host.

## Scope, resources, and cache lifetime

Each consumer retains its selected roots, file context, and allowance.
Sharing a closed `(program, document, node)` fact does not expand a consumer's domain.
Program identity includes complete predicate definitions. Utility names or hashes alone do not establish equality.
Walk admission mode remains part of the lane identity.

Utility and short-text caches each retain at most 32,768 entries.
Posting caches retain at most 131,072 node IDs; larger postings remain shared within a dispatch.
Subtree facts record their completed intervals and have bounded retained entries and node IDs.
All caches belong to immutable document revisions held by one executor. Disposal invalidates those revisions.

Incomplete computation never becomes a negative fact.
An incomplete admission proof sends the original file-clause demand to the capture cohort stage.
A failure in that stage discards its provisional rows and publishes the existing failure outcome.
Cancellation is checked during navigation even when every candidate fails its kind filter.
Demand nodes are checked before tape access.

Residual matcher objects remain stable for the whole batch.
This preserves the native memo's address-based ownership keys.
File-local semantic caches remain available across residual rules and are released after the file completes.

## SQL and measurement

The SQL shape remains a small set of partition plans with typed native batch boundaries.
Selective parent relations materialize before child expansion where that avoids repeated qualification.
Production planning reads available SQL kind statistics. Compiled output and prepared statements retain their existing caches.
No program counter or matcher state machine is emitted into SQL.

The profile separates SQL statements, VM instructions, operation lanes, kernel dispatches, navigation work, and scalar residual roots.
It also reports shared closure nodes, posting seeks, subtree materializations, partial-proof rejections, cache reuse, and resource replay.
These counters prevent a small SQL count or wide dispatch from concealing repeated native work.

Qualification checks native verdicts, work charges, captures, catalog coverage, whole-job findings, and three edits.
A deep-tree test bounds navigation operations for suffix closure and both descendant shapes.
Whole-job gates retain 100 matching statements, a 1.30 CPU ratio, a 1.50 edit CPU ratio, and 1.5 GiB peak RSS.

## Published baseline before the complete conversion

The [qualification report](../benchmark/sql-native-wave-integration.json) passes all existing ceilings and the new native-batching checks.
The workload contains 5,725 Python files, 95.7 MB of source, 138 policies, and 146 AST clauses.
All 97,798 finding identities and three edit results match native.
The catalog also preserves raw matches and captures across 87,162 rule/file pairs.

| Measure | Native | SQL with native waves |
| --- | ---: | ---: |
| Whole-job CPU | 113.85 s | 111.72 s |
| Whole-job wall time | 66.80 s | 69.67 s |
| Median edit CPU | 202.82 ms | 213.17 ms |
| Peak RSS | 0.79 GiB | 1.10 GiB |

This paired run does not establish a material CPU improvement over the previous SQL implementation, which used 110.41 seconds.
The previous and current runs have different host load. CPU and wall measurements are both retained.

The job uses 23 matching SQL statements and 25,216,565 VM instructions.
Native waves process 30,044,209 input lanes in 14,759 submissions.
A submission includes up to nineteen programs and nineteen documents. Maximum kernel width is 2,048 lanes.
The executor performs 242,632,839 operation lanes in 5,726,033 kernel dispatches: 42.37 lanes per dispatch.
There are no resource-triggered native replays.

Scalar matcher roots fall from 62,364,147 to 53,109,000, a 14.8% reduction.
Residual CPU falls from 73.24 to 64.49 seconds. Wave preparation and execution rise from 2.64 to 13.10 seconds.
Total matching CPU remains approximately unchanged: 82.24 versus 82.64 seconds.
The batch execution model is real, but these numbers do not demonstrate a whole-job speedup.

At that baseline, scalar work remained substantial. Binding, scope, capture, and mixed semantic rules accounted for most residual CPU.
The largest residuals include B023, F405, B007, and B615.
The current conversion replaces those residuals with typed semantic facts and capture operations.
It remains unqualified: the latest full paired run uses 198.09 SQL CPU seconds versus 110.82 native CPU seconds.
Findings and edits match, but the 1.30 CPU ceiling fails. The journal records subsequent changes and measurements.
The [rejected qualification](../benchmark/sql-native-capture-cohort-rejected.json) records that failure and the passing gates.
Its raw [catalog](../benchmark/sql-native-capture-cohort-rejected-catalog.json.gz),
[native](../benchmark/sql-native-capture-cohort-rejected-native.json.gz), and
[SQL](../benchmark/sql-native-capture-cohort-rejected-sql.json.gz) reports remain separate from the accepted baseline.
The zero-cost subtree materialization does not activate on this full workload; its benefit is established by the amplification gate only.

Raw evidence is retained as compressed [catalog](../benchmark/sql-native-wave-catalog.json.gz),
[native](../benchmark/sql-native-wave-native.json.gz), and [SQL](../benchmark/sql-native-wave-sql.json.gz) reports.
The summary records source hashes, executable identity, corpus identity, query plans, and acceptance ceilings.

The checker requires wave evidence and both batching axes.
The new conversion gate requires zero scalar residual calls and zero scalar callbacks from semantic projections.
It also rejects a mean kernel width of four lanes or less.
These are regression ceilings for this workload, not claims that the remaining scalar work is acceptable as the final architecture.
