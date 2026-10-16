# SQL engine shape analysis

## Purpose

This document identifies the costly shape of the restored SQLite rule engine.
It also defines the changes that can remove each cost.

The historical run establishes the performance baseline.
See `benchmark/history/ner-1562-historical-sql-reproduction-2026-10-16.md`.

## Implementation status

Rerail now prepares every SQLite AST file in parallel before batch registration.
Files share a batch only when their language and candidate-clause set are equal.
This restriction prevents a rejected terminal from spending another file's work budget.

Shared AST and text batches run each required terminal once across their files.
Each file retains its own deadline, completion state, scope cache, and witness limits.
Clause statements still run once for each file-clause pair.
Clause-major statement execution remains the next execution-shape change.

Profile output now reports SQLite batches, registered files, statement executions, result rows,
terminal executions, and virtual-table filters. These counters measure the remaining file-clause
statement shape without retaining raw benchmark output.

## Evidence

The complete data-layer run took 1,404.7 seconds inside `applyAsync`.
The run processed 5,725 files, 12 policies, and 829 clauses.
It used 15.1 GiB of memory at its peak.

The profile assigned 2,240.1 CPU-seconds to AST matching.
Parsing used 7.7 seconds, and regular expressions used 1.5 seconds.
Thus, parsing and text search do not explain the result.

The source identifies the execution topology.
It does not yet quantify every callback family.
New counters must quantify each family before an optimization changes it.

## Current execution topology

The workspace prepares files in parallel worker-local chunks.
It registers compatible prepared AST files through `SqlPlans::with_files()`.

The batch then evaluates one file at a time.
`evaluate_statements()` loops over each candidate clause for that file.
It calls `QueryBatch::run_while()` once for each clause.
Therefore, statement execution still scales with file-clause pairs.

Each clause is one large SQL expression.
The expression calls Rust scalar functions for matcher terms and state changes.
Nested relations scan `rerail_nodes` virtual-table cursors.
Semantic terms scan `rerail_facts` cursors.
Utilities and deep terms run nested prepared statements.

The effective loop is:

```text
worker
  -> file
    -> clause statement
      -> candidate root
        -> matcher term callback
          -> relation candidate or semantic fact
            -> more callbacks or a nested statement
```

SQLite controls this loop, but SQLite cannot inspect most of its work.
The expensive operations remain behind callbacks and integer handles.

## Problems and corrections

### 1. Batching does not include clause statements

Production now registers every compatible file in one immutable batch.
Language and candidate-clause differences can produce separate batches.
The `OnceLock` terminal cache processes all files in each batch.

The clause statements still run separately for each file.
Thus, the first batching change removes terminal duplication but not statement duplication.

**Correction:** Run a clause plan over all eligible files in one batch.
Partition a batch only for a measured memory or backend limit.

**Gate:** Statement executions must scale with clauses and partitions.
They must not scale with file-clause pairs.

### 2. The loop order is file-major

`evaluate_statements()` loops over all candidate clauses inside one file evaluation.
This order repeats SQLite setup and plan traversal for every file.

The statement cache prevents repeated preparation in the common case.
It does not remove repeated execution or virtual-table setup.

**Correction:** Schedule work by clause and batch partition.
Pass candidate roots as a relation keyed by clause and file.
Keep explicit file, clause, root, and output order columns.

**Gate:** Reversing physical batch order must not change canonical results.

### 3. Matcher terms are scalar callbacks

The compiler emits calls such as `tq_pattern`, `tq_regex`, `tq_pred`, and `tq_struct`.
It also emits callbacks for boolean state, captures, walks, counts, and utility calls.

A callback obtains the batch, reads an environment handle, and runs Rust logic.
SQLite sees only a scalar function with an integer result.
It cannot reorder, combine, or index that hidden work.

**Correction:** Produce terminal matches and semantic facts in bulk.
Expose the results as typed rows with explicit completion records.
Use SQL only for relational composition over those rows.

A scalar function remains valid for a small, pure operation on one value.
It must not run a matcher, a tree walk, or a semantic producer.

**Gate:** Count calls by function.
A matcher or producer callback count greater than zero fails the target design.

### 4. Tree relations repeat virtual-table walks

A relation opens `rerail_nodes` with an axis and an origin node.
Correlated subqueries repeat that operation for candidate roots and nested terms.

The engine has local mitigations for same-key siblings and capture-free structures.
Those mitigations do not change the correlated execution shape.

**Correction:** Give SQL prepared node topology or prepared relation rows.
Compare two physical forms before selection:

1. Sorted rows in generation-scoped temporary tables.
2. Read-only virtual tables over immutable packed arrays.

Both forms must support file, node, parent, subtree, kind, field, and order constraints.
A cursor must enumerate prepared records only.
It must not run a matcher.

**Gate:** Record `xFilter`, `xNext`, and `xColumn` calls by table.
Record the number of nodes in each logical walk.

### 5. Capture environments are opaque handles

SQL carries each capture environment as an integer.
`EnvArena` stores immutable Rust objects behind those handles.

Many terms lock the arena, clone an `Arc`, convert bindings, and store a new environment.
The arena retains intermediate environments until the file batch ends.
SQLite cannot join or filter individual bindings.

This design can produce high allocation and retention costs.
The 15.1 GiB peak makes this a priority measurement, not a proven sole cause.

**Correction:** Compare these representations:

1. A typed `env_binding` relation that SQL can join directly.
2. An immutable packed arena that bulk operators extend outside scalar callbacks.

Both representations must preserve repeated captures, multi-captures, transforms, rollback, and three-valued state.

**Gate:** Record environment reads, creations, retained bytes, and peak live environments.
Select a representation only after this measurement.

### 6. Bound patterns can bypass terminal rows

`tq_pattern` uses cached terminal output only when its captures are not already bound.
Otherwise, it runs ast-grep directly at the current node.

This fallback returns to one matcher call for each SQL row.
Repeated-capture equality and nested patterns can trigger this path.

**Correction:** Compile complete structural terminals that accept capture constraints.
The terminal relation must return ordered matches and captures in one bulk operation.
Direct lowering from ast-grep `PatternNode` must preserve Smart strictness and ellipses.

**Gate:** Record direct pattern attempts separately from terminal executions.
The production count for direct pattern attempts must become zero.

### 7. Semantic facts are lazy per node and environment

`rerail_facts` computes facts during `xFilter`.
Its cache key includes the term, document, node, and environment.

This design mixes fact production with SQL plan execution.
Environment-dependent keys can multiply the same semantic analysis.
SQLite also cannot schedule the producer as one batch operation.

**Correction:** Split semantic evaluation into two parts.
A bulk producer emits typed facts and one status row for each requested unit.
The SQL plan evaluates user sub-rules over those facts.

Environment-independent facts must be shared across clauses.
Environment-dependent requests must be grouped into one input relation for a bulk operator.

**Gate:** Record producer calls, distinct producer keys, output rows, cache hits, and producer time.

### 8. Utilities re-enter SQLite

`tq_util` and `tq_call` invoke `recursion::run()` from inside a running statement.
Each nesting depth uses another prepared statement for the same connection.

This design crosses the SQLite boundary for each utility evaluation.
It also retains prepared copies by SQL text and recursion depth.

**Correction:** Compile the utility graph before execution.
Evaluate acyclic utility groups as relational plan stages.
Evaluate recursive groups through an explicit bounded fixpoint stage.
Use a recursive CTE only when it preserves rule order and three-valued semantics.

**Gate:** Record nested statement calls, maximum depth, preparations, and VM steps.
The target production path must not step a statement from a scalar callback.

### 9. `violation.each` reruns the root statement

`QueryBatch::enumerate()` reruns `plan.root_sql` after each new excluded binding.
The number of statement executions can scale with the number of reported captures.

**Correction:** Make the terminal producer emit all ordered capture alternatives.
Then apply exclusion, suppression, and witness limits over that finite relation.

If enumeration needs rounds, run one bulk round for all active roots.
Do not run one statement for each root and captured value.

**Gate:** Record enumeration rounds, root-statement executions, and emitted alternatives.

### 10. Every worker caches the full statement catalog

Each worker connection sizes its prepared-statement cache for all distinct plan statements.
A catalog can have three main statements per clause and nested utility statements per depth.

Large SQL text and prepared programs multiply by the number of worker connections.
This multiplication is a possible contributor to the measured memory peak.

**Correction:** Measure prepared SQL bytes and SQLite memory by connection.
Use bounded plan stages and shared SQL templates where semantics permit them.
Prepare only the plans assigned to a worker partition.
Release all plan state when the catalog generation becomes inactive.

**Gate:** Record statement count, SQL text bytes, prepared bytes, cache hits, and high-water memory.

### 11. Completion is not a uniform relation

`rerail_matches` emits explicit completion rows.
Other callback families report errors through statement failure or hidden matcher state.

A missing row cannot safely prove that a rule did not match.
Mixed completion mechanisms make absence queries difficult to audit.

**Correction:** Every terminal and fact producer emits a status row.
The row identifies completion, failure code, and work use for each requested unit.

A clause can infer absence only after all required status rows report completion.
A failed clause publishes no partial violations.

**Gate:** Inject cancellation, capacity, parser, and backend failures at each phase.
None of these failures can produce a negative match result.

## Proposed implementation order

Each performance commit must change one measured cause.
The matcher remains the semantic oracle after every commit.

1. Add counters without changing execution.
2. Freeze the data-layer workload and a smaller diagnostic workload.
3. Measure callback, statement, walk, environment, fact, and memory counts.
4. Change to real multi-file batches and clause-major scheduling.
5. Move structural pattern work to bulk terminal relations.
6. Select a measured physical form for node and candidate relations.
7. Move semantic facts to bulk producers.
8. Select a measured capture-environment representation.
9. Remove nested utility statements through explicit plan stages.
10. Move `violation.each` to bulk ordered alternatives.
11. Attach packed, Metal, and CPU-JIT producers to the same terminal relation.
12. Cut over only after complete semantic and failure parity.

Steps 4 through 10 change the execution model or physical schema.
They require approval before implementation.
The counter work does not require a schema change.

## First measurement change

Add one batch metrics object and report these counters:

- Main and nested statement preparations and executions.
- SQLite VM steps.
- Scalar calls by function name.
- Virtual-table calls by table and method.
- Logical axis-walk nodes.
- Direct and cached pattern attempts.
- Environment reads, creations, and retained bytes.
- Fact requests, unique keys, rows, and producer time.
- Terminal executions, rows, and completion states.
- `violation.each` rounds and root-statement executions.
- Prepared SQL bytes and SQLite high-water memory.

Run the counters on the frozen workload before the first optimization.
The first fix must target the largest measured cost that the counters can isolate.
