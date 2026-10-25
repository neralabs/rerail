# Reusable matcher batches and prefix sharing

The SQL engine now calls one native batch per resident partition for the shipped Python catalog.
The batch visits each file once and retains its matcher memo across rules.
Compatible rules share closed predicate prefixes before they execute their private residual matchers.
SQL supplies selective input domains and publishes typed results.

This implements the measured changes from the [joint optimization audit](sql-matcher-batch-optimization.md).
The [transition journal](sql-matcher-fragment-transition.md) records experiments, corrections, and qualification results.

The final 5,725-file qualification preserves all 97,798 finding identities and three edit results.
It uses 23 matching statements, 25.2 million VM instructions, and 64.2 million native root evaluations.
Whole-job CPU remains close to native: 110.41 seconds versus 112.56 seconds in the final paired run.
Wall time worsens in that run. The [evidence](../benchmark/sql-matcher-batch-integration.json) records both, with unchanged acceptance ceilings.

## Reuse and compatibility

The implementation follows the sibling `Pulling-it-all-together` worktree's current architecture and producer contracts.
The sibling worktree remains unchanged.

| Layer | Current owner | Reuse boundary |
| --- | --- | --- |
| Prefix transformation | `producers/prefix.rs` | Generic ordered semantic keys; no matcher or SQLite dependency |
| Matcher safety and identity | `matcher/factor.rs` | Closed predicate proof, complete keys, residual construction |
| Contract catalog | `producers/mod.rs` | Existing registry; separate from execution |
| Native batch transport | `sql/fragments/batch.rs` | Typed requests, file scheduling, producer execution, outcomes |
| SQL planning and publication | `sql/engine/planning.rs` | Candidate access paths, demand aggregation, final projection |

The sibling architecture maps these SQL modules to `engine/vtab`, `engine/plan`, and `engine/compose`.
The generic transformation can remain under `producers` when the transport moves.
Other backends can supply their own keys and execute the returned stages without importing SQLite.

`Plan<K>::new` accepts `(consumer_id, ordered_terms)` pairs.
Each stage names its parent, shared terms, and consumers.
Each tail names its consumer, parent stage, and consumed prefix length.
The transformation recurses after divergence. It does not stop after the first common prefix.
Singleton paths remain private residuals.

The caller must prove equal semantic domains and certify that each term can be shared.
The planner compares complete keys. A hash or a matching display name does not establish identity.
The matcher adapter retains complete transitive utility definitions for this comparison.
The versioned `matcher.closed_predicate.v1` contract declares typed outputs and domain completion.
The registry describes the operation; it does not execute the prefix plan.

## Sharing rules and lifetime

The native batch groups consumers by file, dialect, resource options, and the complete ordered root domain.
Different domains remain separate. The batch does not create a larger union domain to increase apparent reuse.
Capture-bearing requests, enumeration, semantic operations, and parameterized calls remain private.
They still benefit from the existing file memo where its native keys permit reuse.

Only closed syntactic predicates enter shared prefixes.
These include suitable kind, regex, relation, boolean, and unbound count predicates, including closed utilities.
Unknown-producing terms remain residuals. Resource failures remain explicit outcomes, never negative cache entries.
An empty completed prefix domain prevents all dependent residual work.
Each consumer receives the remaining allowance after its shared stages.
This does not establish equivalence of every historical cache-fill or instruction-charge observation.
That semantic question remains separate from finding, capture, and incomplete-coverage parity.

Compiled stage matchers retain stable addresses for the file memo's lifetime.
Some existing native caches use matcher addresses as keys.
Constructing and discarding temporary matchers inside that lifetime can produce incorrect cache reuse.
The batch caches prepared stage plans by consumer set and releases file caches after the file's consumers finish.
No producer result survives an edit. Existing workspace dependency invalidation remains authoritative.

Direct kind tests already guaranteed by admission are removed before prefix extraction.
Materializing their successful rows would copy the entire domain without filtering any work.
The first integrated experiment exposed this problem; the journal records its correction.

## SQL shape and virtual-table affordances

The generated statement has one native batch arm and direct SQL arms for pure structural clauses.
Mixed clauses supply collected, filtered domains through one demand aggregate:

```sql
SELECT clause, file, node, truth, ordinal, c0, c1
FROM matcher_batch_0(
  (SELECT batch_demands(clause, handle)
   FROM (
     SELECT 7 AS clause,
            (SELECT demand_7(request, file, node) FROM ...) AS handle
     UNION ALL
     SELECT 12,
            (SELECT demand_12(request, file, node) FROM ...)
   ))
)
UNION ALL
SELECT ...; -- pure structural result arms
```

The numbers and capture width above illustrate the shape.
The qualification artifact retains the actual generated SQL and its EXPLAIN plan.
Native-only clauses use their registered root domains directly, without SQL enumeration of every request.
An empty mixed demand remains empty; it cannot become an unrestricted scan.
Large catalogs split at 256 clauses per statement and retain the whole-job execution budget.

Candidate tables now support file enumeration and `(file, node)` membership lookup.
Membership uses sorted domain roots and a file index retained for the active fragment.
This permits a selective seed to drive the join and intersect the actual requested domain:

```sql
WITH qualifying_parent AS MATERIALIZED (
  SELECT p.file, p.id
  FROM candidate_files_i f
  CROSS JOIN node p ON p.file = f.file
  WHERE <selective parent predicate>
)
SELECT c.request, c.file, c.node
FROM qualifying_parent p
CROSS JOIN node n ON n.file = p.file AND n.parent = p.id
CROSS JOIN candidates_i c ON c.file = n.file AND c.node = n.id;
```

F707 can use the handler-first shape above.
Materializing qualifying parents prevents SQLite from moving their sibling checks after child expansion.
A parent predicate containing only a kind test needs no such materialization.
Rare descendant plans likewise start from indexed kind postings and intersect candidate membership.
Root-driven plans remain available when the stored counts favor them.
Anchor-implied structural predicates disappear, and unbound counts stop at their decisive threshold.

The planner reads counts from the content store's SQL kind vocabulary.
It does not recount a batch in Rust.
Each batch profile reports the counts actually used, their source, selected shapes, and lookup CPU.
Prepared SQL changes only when those counts change a planning decision.
Compilation and preparation remain separately reported and cached.
These counts cover indexed tapes in the store, across dialects and retained versions.
They are cost estimates, not candidate-domain sizes or admission proofs.
Search indexes are populated by maintenance. Attaching an empty store alone does not supply useful statistics.

## Measurement and remaining scope

Batch profiles include candidate roots, shared-prefix roots and outputs, producer consumers, memo hits and misses, and prefix CPU.
The compiler first identifies prefixes with possible reuse, so unique prefixes do not incur runtime domain-grouping costs.
The `grouping_roots` counter records the inputs that still need a domain-equality comparison.
Residual CPU remains attributed to its clause. Shared prefix CPU appears separately and is included in the qualification total.
The whole-job report records CPU, wall time, and peak memory.
Its gate enforces CPU and memory ceilings, finding and edit parity, complete coverage, and the 100-statement limit.
VM instructions remain a diagnostic.

The existing four-worker pool executes bounded resident partitions. This change adds no nested worker pool.
Partition sizing still uses source bytes. A new cost-balancing policy needs measured per-file costs before adoption.
The batch does not yet expose generic call-site or lexical-scope relations for SQL-side async admission.
It shares the corresponding closed matcher prefixes instead.
Capture-dependent factoring and cross-edit producer caching require additional contracts and are not inferred by this transformation.
