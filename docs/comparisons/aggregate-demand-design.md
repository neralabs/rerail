# Aggregate demand and selective semantic facts

## Scope

This change repairs three execution boundaries in the integration compiler.
Aggregate helpers must receive consumer keys. Zero-count tests must use anti-existence operations.
The async rules must request semantic facts only after structural and target-API filters.
The work starts from comparison commit `5666ea905`; the separate integration worktree remains untouched.

## Aggregate contract

A helper can retain all witness columns required by count and sum semantics.
Its input must include the union of grouping keys requested by its consumers.
The key includes every correlated variable, including document or world identity.
Empty groups still produce count and sum zero. Multiple consumers contribute a union of demands.

A demand rule contains safe consumer guards before the aggregate.
A guard cannot depend transitively on the helper or the consumer's recursive result.
A final dependency check rejects transformations that introduce aggregate or negative recursion.
If safe guards cannot bind every key, the compiler retains the original calculation and records the limitation.
The compiler must not silently remove a group or create a new recursive aggregate.

For count-equals-zero, replace the count and comparison with a keyed anti-join.
The helper remains demand-restricted; no global ancestor-by-name intermediate is allowed.
For other aggregates, preserve witness columns and the existing empty-group semantics.

## Semantic producer contract

Call structure starts from existing kind postings. Arguments are a separate demand batch.
Async context and a conservative target screen precede name resolution.
Unknown import cases reach the semantic producer. A screen cannot reject an unknown case.
Native producers return typed facts. SQL retains rule composition and final findings.
Source and annotation worlds remain distinct unless their identities and candidate scopes are identical.

## TDD and measurements

First record failing tests for zero-count shape and general aggregate demand.
Then test empty groups, duplicate witnesses, multiple consumers, recursion, and world separation.
Check intermediate helper rows on a workload with many irrelevant names and few demands.
Run the existing async positives and edge fixtures before the full corpus measurement.
Report any existing semantic differences separately from new regressions.

## Deferred statement reuse

The previous profiled job executed 141,771 SQL statements:

| Type | Executions |
|---|---:|
| DROP | 45,004 |
| CREATE | 43,869 |
| INSERT | 41,914 |
| Other | 10,984 |

Chunk and guest rounds repeatedly create, publish, and remove temporary relations.
Prepared statements do not remove these operations.
A later change must reuse execution structures and reduce materialization boundaries.
This change records those counts but does not redefine the 100-execution budget or claim that aggregate repair satisfies it.

## Implementation boundaries

`logic/rr/aggregate_demand.rs` runs after RR relation lowering.
It computes transitive dependencies from rule bodies and native producer inputs.
For each private aggregate helper, it collects every consumer's correlated columns.
Those columns include explicit world and document keys. Node IDs also retain document identity.

The pass copies independent positive guards and comparisons that those guards can bind.
It excludes guards that depend on the helper or the consumer's recursive result.
It unions consumer demands and adds that relation to the helper's body.
Count and sum retain their witness columns, so deduplication and empty-group behavior remain unchanged.
The compiler checks declarations and strata before accepting each change.

A scalar `count = 0` becomes a negative atom when nothing else reads its scalar result.
This removes scalar aggregation from the consumer. The helper still supplies witnesses for demanded keys.
It does not promise one native lookup per key or early termination on the first witness.
The native work-growth gate checks intermediate join outputs, not only final relation sizes.

The pass leaves uncorrelated aggregates unchanged.
It also refuses incompatible consumer key layouts and demands that require a recursive result.
These are explicit limits of this pass. They do not authorize a global intermediate for the async family.
That family uses the semantic producer path below.

## Native fact operations

| Operator | Demand | Result |
|---|---|---|
| `python_qualified` | Reference nodes and target names | Confirmed qualified names |
| `python_origins` | Expression nodes | Expression and binding value origins, including supported `with` bindings |
| `python_assigned` | Expression nodes | Expression and assigned values, with stop-at-global binding semantics |
| `python_arguments` | Call nodes | Positional index, keyword, spread kind, and value |

These operations return typed facts. They do not select violations or execute rule control flow.
The compiler preserves their input relations in dependency analysis and prepared-program serialization.
The evaluator reads each demand relation once and calls the producer with the full batch.

The batch groups demanded nodes by immutable document identity.
Qualification first checks import syntax from kind postings against the requested target names.
An uncertain import screen admits the request. Semantic resolution then confirms or rejects it.
Each admitted document has one cached scope table and a cache of projected node facts.
Other operations reuse those facts. Requests from different document identities cannot share semantic results.

The current adapter restores a semantic tape only for documents that need binding or qualification facts.
It checks node count, demanded-node spans, and kind IDs before using semantic node identities.
This restoration is a measured cost, not a free operation.
The adapter uses the engine's default Python version, as the existing comparison reference does.
A future configurable caller must include its Python options in the cache boundary.

## Async demand sequence

1. Read call and execution-context facts from the existing posting-based producer.
2. Keep calls in async function bodies and extract their reference nodes.
3. Screen those references against each rule's target APIs, then confirm qualification in native batches.
4. For spawn calls, request arguments only after qualification identifies a relevant `os.spawn*` API.
5. For `.open` calls, request receiver origins only after the attribute-name check.
6. Qualify constructor or mode-value references only when the preceding joins demand them.
7. Compose the returned facts with ordinary relational joins and anti-joins to produce findings.

The call structure no longer fills the global argument relation.
Existing readers of that relation can still request it; the async family uses selected argument batches.
Producer sharing is valid only when candidate scope and document identity agree.
A shared semantic cache avoids duplicate resolution without broadening a policy's candidates.

## Deferred execution-structure work

The next statement-reuse change needs separate correctness and cost gates.
Retain schemas and indexes across compatible chunks; clear or replace their contents at the batch boundary.
Keep source and guest worlds distinct. Test empty batches and changing relation cardinalities.
Reuse a published native relation where consumers can read it directly.
Materialize only when a consumer needs an access path or a durable boundary.

Measure each operation class before and after this change, including empty chunks and guest rounds.
Count executions, not only unique prepared SQL strings.
The acceptance gate remains at most 100 executions for the complete comparison job.
Preparation and execution costs must be reported separately.

## Physical planning and execution locality

A demand guard alone does not enforce a demand-first execution plan.
The native trace showed a witness scan before the guard, despite only three requested keys.
The planner now anchors generated aggregate helpers on their demand relation.
An explicit recursive delta still takes priority. Remaining joins use the existing cost planner.

Python fact operators preserve document identity and can run in document batches.
The file-local splitter now verifies their proof rules instead of rejecting every native operator.
Qualification targets must be constant across document batches.
A target set derived from another file must not receive a document-local classification.
The split retains native metadata only in the part that executes the operator.

## Reproduction

Use the optimized engine test binary and the preserved corpus from the comparison report.
`run.py --stage demand-tests` runs the targeted correctness and work-growth gates.
`--stage rr` runs the complete authored family without detailed profiling.
`--stage rr-profile` records its decisions, native join steps, SQL statements, and phase costs.
`--rules <directory>` selects the same four filenames from an alternate directory.
This permits a legacy-rule run through the repaired aggregate compiler, separate from the new semantic producer path.
Use a new output directory for each variant. Never overwrite the baseline profiles.

The repaired async profile still executes 10,272 statements: 2,022 DROP, 2,031 CREATE, 1,382 INSERT, and 4,837 other operations.
Removing setup and teardown alone would therefore leave 6,219 executions.
The follow-up must also reduce publication and repeated reads across 72 chunks.
Known-empty inputs should skip producer reads and per-column count queries.
These changes need their own execution-count gate; query preparation alone cannot meet it.

## Producer identity

A native helper is identified by its complete operation, request relation, and target relation.
Concatenated relation names alone are not a unique key: `(a_b, c)` and `(a, b_c)` collide.
The allocator compares complete contracts before reusing a helper and disambiguates collisions.
The regression covers both distinct contracts and repeated use of an identical contract.
