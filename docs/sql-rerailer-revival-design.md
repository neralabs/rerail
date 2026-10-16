# SQL rerailer revival design

## Status and authority

This document defines the new NER-1562 implementation direction.
It supersedes the earlier plan to put the complete rule language inside the packed executor.

The packed executor remains important.
It becomes a terminal and bulk-fact producer for the SQL rule engine.
SQLite owns higher-level rule composition.

The current native matcher remains the semantic oracle until the SQL path supports the complete public rule contract.
There is no production route for a partial SQL rule language.

## Decision

Rerail will compile each complete `rerail.policy/v1` clause into a relational plan.
SQLite will run that plan over typed, batch-scoped relations.

Native and accelerated code will perform work that SQLite cannot perform directly:

- Parse source and restore tapes.
- Run ast-grep structural patterns.
- Run bounded regex, Lua, and text programs.
- Produce scope, binding, value, call, and parsed-literal facts.
- Decode packed arrays when a relational scan needs them.

These producers must operate on batches.
A producer must not run one matcher for each SQLite row.

SQLite will perform these operations:

- Ordered boolean composition.
- Capture-environment joins.
- Relations and field constraints over staged node data.
- Counts, grouping, thresholds, and absence checks.
- Utility composition and bounded recursion.
- `violation.each` reduction.
- Cross-file joins and consumer queries.
- Stable output ordering.
- Completion and unknown-state propagation.

## What the historical result proved

Landing previously compiled complete rules into SQLite statements.
That implementation reached broad semantic coverage.
It then reported a large speed increase after native evaluation replaced SQL evaluation.

That result does not compare ordinary indexed SQL with the same algorithm in Rust.
The removed engine had these costly properties:

- It ran one check statement for each file.
- Virtual tables called Rust for tree walks and semantic facts.
- Scalar functions called Rust for node-level operations.
- Capture environments were Rust objects behind SQLite integer handles.
- Recursive utilities re-entered separately prepared statements.
- SQLite could not inspect or optimize the work behind those callbacks.
- Later native code added specialized memoization and linear relation algorithms.

The measured result rejects fine-grained callback orchestration.
It does not reject set-based SQL composition over typed relations.

## Historical source points

The final Landing commit before SQL evaluation was removed is:

```text
30a03c642cdd993bec39357c27eb432b28043136
```

The deletion and native replacement is:

```text
697bd2bd785b6fedebb39d73eff8b6eb271ee7f4
```

The historical compiler and tests at `30a03c642` are design inputs.
They are not a branch base and are not copied as one patch.

The current packed executor landing pad is:

```text
2f22bcb134ac60ddc468936a89ec2fe8b633cbda
```

The coherent pre-ABI-v4 packed reference is:

```text
4dff797b4
```

The completed ABI-v4 cutover is:

```text
ecb014feb
```

## Why this work does not use a revert

Rerail has no commit that contains the SQL rule engine.
Its initial import took Landing after commit `697bd2bd7` removed that engine.
The Rerail import is also one squash commit with Tempo-to-Rerail renames.

Therefore, no valid Rerail revert exists.
Reverting `697bd2bd7` in Landing and merging the result is also incorrect.
That operation would restore old matcher code and remove later native fixes.

The implementation must start from current Rerail `main`.
It must port selected compiler concepts and tests from `30a03c642` into new Rerail modules.

## Target module boundary

The implementation adds these modules:

```text
native/src/code_rule_dsl/sql/
  catalog.rs
  compile.rs
  connection.rs
  schema.rs
  execute.rs
  completion.rs
  tests/

native/src/code_rule_dsl/packed/
  abi.rs
  verify.rs
  batch.rs
  interpreter.rs
  metal/
  cpu_jit/
  relation.rs
```

The names can change during implementation.
The dependency direction cannot change:

```text
validated Rerail rule
  -> relational plan and verified terminal programs
  -> batch-scoped typed relations
  -> SQLite composition
  -> canonical clause outcomes
  -> existing violation publication
```

SQLite must not parse policy JSON, ast-grep syntax, SCM, regex source, or Lua source.
Executors must not call the current matcher.

## Batch model

The old engine ran a prepared statement for each `(file, clause)` pair.
The new engine runs one prepared clause plan over a file batch.

A batch contains:

- Stable file ordinals.
- Stable clause ordinals.
- Restored tapes.
- Candidate roots.
- Verified terminal programs.
- Immutable semantic side tables.
- Completion and failure records.

The normal unit is one clause over many files.
The implementation can fuse clauses after measurements support that change.

Statement count must scale with clauses and batch partitions.
It must not scale with candidate roots or file-clause pairs.

## Typed staging relations

The first prototype will compare two staging forms.

### Typed temporary tables

Rust inserts sorted typed rows into connection-local `TEMP` tables.
SQLite builds only the indexes required by the compiled plans.
Tables are cleared by batch generation, not by individual row deletion.

### Read-only batch virtual tables

A virtual table reads immutable packed arrays through one registered batch token.
`xBestIndex` must accept useful batch, file, program, node, and range constraints.
The cursor must not execute a matcher.
It only enumerates prepared typed records.

The spike must measure both forms.
The selected form must minimize total load and query time.
The design must not assume that fewer SQLite calls are faster.

## Initial relational schema

The exact physical layout is a measured decision.
The logical relations are fixed.

```text
batch_file(
  batch, file, document, grammar, source_length, complete, error_code
)

candidate_root(
  batch, clause, file, root, root_order
)

node(
  batch, file, node, parent, subtree_end,
  kind, field, start_byte, end_byte, flags
)

terminal_status(
  batch, terminal, file, complete, verdict, error_code, work_used
)

terminal_match(
  batch, terminal, file, match_id, root, alternative, local_order, env_id
)

capture(
  batch, terminal, file, match_id, capture_order,
  slot, binding_kind, node, sequence_start, sequence_count
)

fact_status(
  batch, producer, file, complete, error_code, work_used
)

fact(
  batch, producer, file, source_node, fact_order,
  fact_kind, key_id, value_id, target_document, target_node, verdict
)

outcome(
  batch, clause, file, root, outcome_order,
  env_id, verdict, complete, failure_code
)
```

Large text and variable-size capture sequences stay in immutable packed side arrays.
SQLite rows carry checked offsets and lengths.

All ordinals are logical identities.
SQLite row identifiers are not program identities.

## Capture environments

The historical environment-handle model preserved semantics but caused many scalar transitions.
The new engine keeps immutable environments but changes their production boundary.

A bulk terminal or fact producer can return an `env_id`.
SQLite can join and select these identifiers without calling Rust.
A plan that extends an environment submits a complete input relation to one bulk environment operator.
It does not invoke a scalar function for each row.

The implementation must measure a fully relational binding table as an alternative:

```text
env_binding(batch, env_id, slot, binding_kind, value_offset, value_count)
```

The selected representation must preserve:

- Single and multi-captures.
- Transformed text.
- Repeated-capture equality.
- Withheld bindings in three-valued `all`.
- Binding-follow cycle guards.
- Utility argument frames.
- Rollback after failed alternatives and negations.

## Rule lowering

The compiler consumes the validated `GrammarMatcher` and compiled clause metadata.
It does not consume source JSON.

Every public operation must have one lowering classification:

1. Native SQL expression or relational operator.
2. Verified terminal program.
3. Bulk semantic fact producer plus SQL sub-plan.
4. Separate producer or consumer phase for cross-file execution.

There is no `unsupported -> native matcher` production route.
An unsupported operation is a compiler defect and blocks production cutover.

The operation inventory must include tests and backend plans for:

- `Pattern`, `Kind`, `Regex`, and text comparison.
- `All`, `Any`, `Not`, unknown values, and term order.
- Parent, child, ancestor, descendant, sibling, and field relations.
- Every `stopBy` mode.
- `nthChild`, counts, and offsets.
- Utilities, recursion, and parameterized calls.
- Capture constraints, equality, and transforms.
- Scope, binding, usage, value, resolution, and calls.
- Arguments, format arguments, layout, and parsed literals.
- `violation.each`, fixes, suppression, and syntax-error rules.
- Evidence, `require_each`, `budget`, and `select_when`.

## Completion and absence

Every producer emits a status row for each requested unit.
A missing result row never proves a negative result.

SQL can infer absence only when all required producers report completion.
Cancellation, work exhaustion, parser failure, backend failure, and capacity exhaustion remain distinct failures.

A failed clause publishes no partial violations.
The engine must not retry through another semantic implementation.

## Ordering and first-match behavior

Every relation carries explicit logical order columns.
SQL must not use physical row order.

The compiler preserves:

- Rule term order.
- First matching `any` branch.
- First accepted relation candidate.
- Ast-grep alternative order.
- Capture order and multiplicity.
- `violation.each` exclusion order.
- Stable file, clause, root, and output order.

Window functions and ordered subqueries can implement priority.
Each use must have a parity fixture that changes output when the order reverses.

## Semantic fact production

Semantic producers operate once for a requested batch or relation.
They return typed rows and completion records.
They do not evaluate nested rule expressions.

For example, a binding producer can return ordered binding sites, values, annotations, and occurrences.
The SQL plan evaluates user sub-rules over those rows.

This separation retains the strongest part of the historical semantic lowering.
It also avoids opaque leaf callbacks that run a recursive matcher.

## Parsed and generated documents

Each parsed literal, embedded region, or generated source is a separate analysis document.
Each document has one source, grammar, and tree.

The workspace owns document mappings.
The generic tape does not contain source maps or foreign subtrees.

SQL uses document identifiers and explicit map relations.
It never treats overlapping trees as one node relation.

## Cross-file phases

Cross-file execution uses explicit phases:

```text
file-local clause and evidence plans
  -> immutable evidence relations
  -> deterministic merge
  -> cross-file consumer plans
  -> canonical outcomes
```

SQL is the natural implementation for anti-joins, grouped budgets, and threshold queries.
Incomplete evidence must propagate an unknown or failed outcome.
It must never prove absence.

## Packed executor role

The packed executor is not the complete rule engine.
It supplies high-throughput terminal and fact relations.

The first packed boundary includes:

- Structural `PatternNode` execution.
- Kind, field, flag, and text tests.
- Capture rows.
- Value programs.
- Typed completion and capacity records.

Later packed operators can accelerate measured SQL bottlenecks.
They must emit the same typed relation as the SQL implementation.
The relational plan does not change when Metal, CPU JIT, or Rust produces a terminal relation.

## Performance constraints

The revived engine must correct the historical execution shape.

Required constraints:

- No statement for each file-clause pair.
- No scalar callback that runs a matcher or semantic producer.
- No nested SQLite query from a producer.
- No repeated tape decompression in one batch.
- No statement preparation inside the file loop.
- No unbounded SQL expression tree.
- No dependence on SQLite's chosen physical row order.
- No materialization of the complete corpus node table.

The compiler can split large plans at explicit materialization boundaries.
A split is a plan operation, not a recursive statement callback.

## Required instrumentation

Every benchmark run records:

- Files, clauses, roots, nodes, matches, captures, and facts.
- Prepared statements and statement executions.
- Virtual-table `xFilter`, `xNext`, and `xColumn` calls.
- Scalar-function calls by function.
- Rows at each materialization boundary.
- SQLite VM steps and wall time.
- Producer calls and producer wall time.
- Tape decode count and bytes.
- Environment creations and joins.
- Temporary-table bytes and high-water memory.
- Complete, unknown, work-limit, and capacity outcomes.

A callback count that scales with nodes is acceptable only for row enumeration.
A callback count that scales with matcher terms per node fails the design gate.

## Differential oracle

The current matcher and SQL path receive the same:

- Compiled clause.
- File batch and analysis documents.
- Candidate roots.
- Work and witness limits.
- Syntax-error regions.
- Cross-file evidence snapshot.

They compare canonical outcomes, not only match spans.
The comparison includes messages, fixes, suppression, ordering, failures, work use, and multiplicity.

The historical SQL parity tests from `30a03c642` will be recovered.
They must be adapted to Rerail names and current semantics.
They are not accepted unchanged when the current matcher has newer behavior.

## Controlled performance experiment

The first performance decision requires four implementations over one frozen workload:

1. Current native matcher.
2. Historical callback-shaped SQL, where it can still run.
3. New batched SQL over typed staged relations.
4. New batched SQL with packed terminal production.

The experiment holds constant:

- Corpus and source revisions.
- Rules and compiled matcher digests.
- Candidate roots.
- Thread count.
- Limits.
- Result rendering and publication.
- Cold and warm cache state.

The report must attribute at least 95 percent of wall time to named phases.
It must include raw counters, not only ratios.

## Implementation sequence

### Phase 0: Recover evidence

1. Freeze a production Rerail corpus and rule catalog.
2. Recover the historical SQL compiler tests from `30a03c642`.
3. Add the complete public-operation inventory.
4. Add canonical outcome comparison.
5. Reproduce a small historical callback-shaped plan only as a benchmark control.

### Phase 1: Build relational staging

1. Add one in-memory SQLite connection per execution worker.
2. Add generation-scoped typed staging relations.
3. Stage nodes and candidate roots for a multi-file batch.
4. Compare temporary tables with read-only packed virtual tables.
5. Add completion rows and failure propagation.

### Phase 2: Lower the structural rule core

1. Lower ordered boolean composition.
2. Lower relations, fields, counts, and utilities.
3. Lower Smart ast-grep patterns through a terminal relation.
4. Run one clause plan over many files.
5. Prove parity with the current matcher.

### Phase 3: Lower all semantic families

1. Add bulk semantic fact producers.
2. Lower nested sub-rules over fact rows.
3. Add parsed-document and host-map relations.
4. Add suppression, fixes, and output rendering.
5. Reach zero unsupported public operations.

### Phase 4: Add cross-file execution

1. Materialize evidence relations.
2. Lower `require_each`, `budget`, and `select_when`.
3. Add deterministic producer and consumer invalidation.
4. Prove fresh, incremental, restored, and evicted parity.

### Phase 5: Attach packed backends

1. Port the packed Rust interpreter as the terminal oracle.
2. Port Metal and the reusable workspace pool.
3. Port the callback-free CPU JIT.
4. Feed their typed rows into the unchanged SQL plans.
5. Measure which SQL operators need further bulk acceleration.

### Phase 6: Production cutover

1. Run complete-catalog shadow evaluation.
2. Require zero semantic mismatches and zero unsupported operations.
3. Meet latency, memory, and storage gates.
4. Switch production as one semantic cutover.
5. Keep the current matcher as a test oracle until a later removal decision.

## Acceptance gates

The SQL path can publish production results only when all conditions pass:

1. Every valid public AST clause compiles.
2. Every shipped rule compiles for each applicable dialect.
3. Canonical outcomes equal the current matcher.
4. Cross-file fresh and incremental outcomes are equal.
5. Incomplete execution never proves absence.
6. No scalar callback runs a matcher or semantic producer.
7. Statement execution does not scale with file-clause pairs.
8. The packed and non-packed terminal relations are equal.
9. Incremental p50 is at most 100 ms.
10. Incremental p95 is at most 500 ms.
11. Steady-state RSS is less than 1 GiB for the target workload.
12. The benchmark report attributes at least 95 percent of wall time.

## Rejected approaches

### Revert the native replacement

Rerail has no such commit, and the revert would remove later correctness fixes.
Reject it.

### Fork Rerail from Landing commit `30a03c642`

That commit predates the Rerail repository and all current packs, adapters, Blade work, and product contracts.
Reject it.

### Restore the old SQL files and route production immediately

The old files preserve the callback shape that caused the disputed performance result.
Use them as a semantic and benchmark reference only.

### Put all high-level semantics in packed bytecode

This duplicates joins, grouping, absence, recursive composition, and cross-file relational operations in every backend.
Use packed code for measured terminal and fact production.

### Keep a permanent native fallback

A dual semantic product is not viable.
Partial coverage is allowed only in tests and shadow mode.
