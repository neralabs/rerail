# SQL composition boundary audit

This audit examines committed source `eee4b8cd1` and its saved full-corpus profile.
It changes no execution code. The measurements below come from existing evidence, not a new benchmark run.

## Finding

The production corpus path executes relation boundaries instead of compiling a complete relational query.
The retained-demand change removes one expensive round trip. It does not implement SQL composition.

There are two distinct problems:

1. The measured async job computes most rules natively but repeatedly uses SQL to publish and read small relations.
2. The general SQL fallback executes individual rule inserts, then merges their outputs through a wave scheduler.

The first problem accounts for this profile. Fixing only the second problem does not fix this workload.

## Evidence

Source profile: `benchmark/async-family-integration/retained-demands/after/rr-profile.json.gz`.
The job processes 5,725 files in 72 chunks.

| Executed statement | Count |
|---|---:|
| CREATE | 915 |
| DROP | 906 |
| INSERT | 909 |
| SELECT | 897 |
| PRAGMA | 76 |
| BEGIN | 72 |
| COMMIT | 72 |
| ATTACH | 2 |
| Total | 3,849 |

The profile records 336,517 SQL VM instructions.
No executed SQL mentions `wave_`; no statement group contains `cold rule`.
The expensive general SQL wave path is therefore not active in this measurement.

The following disjoint profile groups locate the repeated work:

| Profile group | Executions |
|---|---:|
| `local/run/setup` | 1,033 |
| `local/run/views and tables` | 864 |
| `local/run/outputs` | 576 |
| `local/run/outputs/engine materialize` | 288 |
| `local/export to cross` | 144 |

Constant relations such as `q0_async_http_verb` get 73 CREATE executions and 73 INSERT executions each.
Those are 72 local runs plus the cross-file run.
The four local outputs account for 864 executions: create, publish, and read, repeated across 72 chunks.
The native engine already holds these outputs before their SQL publication.

## Where compilation becomes execution

Paths below are relative to `native/src/code_rule_dsl/`.
Line numbers refer to the audited commit.

| Location | Current behavior | Required change |
|---|---|---|
| `logic/plan/corpus.rs:2281`, `:2562`, `:2577` | The chunk callback enters the complete planner for each memory chunk. | Separate source paging from relational execution phases. |
| `logic/plan.rs:4274` | Scan the temporary schema and drop previous tables. | Define the job's sources once; remove intermediate table lifecycle from query evaluation. |
| `logic/plan.rs:4340`, `:4393` | Create views and constant tables before evaluating rules. | Represent constants as typed VALUES relations; compose derived relations as CTEs. |
| `logic/plan.rs:4463` | Iterate components, dispatch native work, or execute SQL immediately. | Traverse dependencies to build a phase; execute only after the phase is complete. |
| `logic/plan.rs:2473` | Publish and SELECT the target set for each semantic producer. | Pass constant target sets directly; compose derived target demands into the phase boundary. |
| `logic/plan/sql_kernel.rs:98`, `:178` | Build per-rule queries for one component, then call `execute_existing`. | Append checked queries to the complete phase instead of executing the component. |
| `waves/schedule.rs:296`, `:321`, `:347` | Query pending steps, execute each rule insert, then fold each written relation. | Remove this scheduler from acyclic composition; use relational CTE dependencies. |
| `logic/plan.rs:4669` | Count each completed SQL relation and each column's distinct values. | Plan from stored/source statistics; collect optional runtime statistics at real phase boundaries. |
| `logic/plan.rs:4682`, `:4795` | Publish native outputs and SELECT each output separately. | Return all finding outputs from one tagged result query. |
| `logic/plan.rs:4548` | Insert produced answers one row at a time. | Bind a batch relation or append its producer query. This path is not identified as active by this profile. |

Preparing these statements once does not remove their executions.
Combining their text with semicolons does not compose their relational work.

## Existing composition code

`logic/plan/emit.rs:261` already implements `single`.
It replaces derived reads with CTE references and combines rule arms by relation.
However, it rejects constants, nested CTEs, recursive dependencies, and lattices.
It materializes every non-output relation and still emits an INSERT for the wave executor.
The demand and findings wave paths use it; the corpus path does not.
This is useful checked-query infrastructure, not the completed production solution.

The merged executor contains a stronger phase interface in `sql_executor/phase.rs`.
Callers append relation definitions and outputs, then execute one WITH statement.
`sql_executor/temporal/compose.rs` and `reductions/compose.rs` use this interface.
Its output protocol publishes validated changes against frozen native versions.
Reuse this composition and publication design, but do not import incremental support bookkeeping into a cold query without need.

The async reference supplies the concrete two-query contract in `findings/tests/async_reference.rs`:

1. Join call sites, qualified references, target symbols, and receiver facts to select candidates for all four rules.
2. After demanded native arguments and value facts complete, join those facts and return tagged findings for all four rules.

The reference binds native relations directly. It does not create a SQL table for every logical relation.
Its two execution counter increments cover those two queries; they do not establish a whole-job SQL budget by themselves.

## Required production shape

```text
compile rules + stored statistics
    -> checked dependency graph
    -> typed native producer contracts + relational phase queries

batch source preparation across files
    -> narrow shared call/reference facts
    -> execute candidate query once across all four rules
    -> collect and execute only the demanded native semantic batches
    -> execute findings query once across all four rules
```

The compiler must preserve file/world keys and each consumer's candidate scope.
Within a relational phase, joins, anti-joins, unions, and aggregates stay in SQL.
Multiple derivations preserve set semantics. UNION ALL requires a proof that duplicates are harmless or removed at the correct boundary.
Native producers remain behind typed batch contracts. They must not become one scalar callback per candidate.

Use CTE materialization for reused, narrow results when measurement supports it.
Inline cheap single-use projections and filters.
Do not fence every intermediate relation: that blocks demand pushdown and recreates publication work inside SQLite.
Only actual producer dependencies, supported recursion boundaries, and measured engine limits justify separate execution phases.

Memory chunks are not automatically query boundaries.
For this job, accumulate narrow call/reference facts across source chunks, with handles for later demanded tape access.
Then run the two relational phases across the complete job.
Retained handles must keep their data alive or support reload after source chunks expire; test this explicitly.
This does not require all AST facts to remain resident.
Bounded SQL batching remains possible if necessary, but two queries across 72 batches already exceed the 100-execution budget.

Ordinary closure belongs in recursive CTEs where the backend can express it.
Ordered or sequential semantics belong behind a native batch relation.
General recursive aggregates and lattice updates need an explicit physical-plan decision; they must not silently restore per-rule SQL scheduling.
This decision does not block the acyclic four-rule conversion.

## Concrete next implementation

1. Compile the real combined four-rule RR program into a job plan before evaluation.
   Reuse the checked Query tree and the merged phase builder's composition approach.
   Add constants, scoped nested CTEs, and tagged final outputs to the composition contract.
   The graph traversal appends definitions; it cannot execute SQL.
   Statistics access belongs before this traversal through an explicit planning input.
2. Partition only at native producer dependencies.
   Batch semantic requests from the completed demand relation, preserving consumer target predicates.
   Keep compilation generic: no rule-name checks or replacement with the hand-authored reference.
3. Route the production corpus call through this plan.
   Source chunks populate the phase's narrow inputs without re-entering `plan::run`.
   Retain the existing native producers, statistics, and cache contracts.
   Remove per-output publication and decoding statements from this route.
4. Compare emitted SQL and execution against the reference on the same prepared facts.
   Then measure whole-job CPU, wall time, peak memory, semantic work, and all SQL executions.

## Gates before claiming success

The retained-candidate gate currently reports the 100-execution result but excludes it from its pass condition.
It proves candidate retention only. It is not an execution-shape acceptance gate.

The replacement must enforce these independent requirements:

- The current 3,849-execution report fails the hard whole-job budget test.
- The real four-rule program produces two relational evaluation statements across the complete corpus.
- All job SQL executions, including setup and publication, remain at or below 100.
- The acyclic path has no wave control SQL or per-intermediate CREATE, INSERT, DROP, and count queries.
- A chain and a fan-out/fan-in program increase query definitions without increasing execution count per relation.
- Constants, duplicates, anti-joins, aggregates, and overlapping rule candidates retain correct semantics.
- Multiple files and worlds cannot collide; demands survive release of their original source chunks.
- Positive and negative fixtures match native and preserved Ruff results. Zero full-corpus findings alone do not prove parity.
- EXPLAIN and runtime counters show demanded joins and batch producer calls, without repeated broad scans or automatic materialization of every CTE.
- Whole-job measurements accompany prepared-fact comparisons. Compilation and preparation costs remain separate.

No runtime improvement is claimed by this audit. The production path still fails the execution budget.
