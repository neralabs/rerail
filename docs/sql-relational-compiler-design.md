# SQL relational compiler design

## Status and authority

This document defines the compiler boundary for the SQL-first Rerail engine.
It supersedes earlier text that lowers a complete `GrammarMatcher` into one packed `Program`.

Production compilation has this shape:

```text
compiled clause
  -> callback-free relational IR
  -> one SQL statement + one catalog of verified terminal programs
  -> one immutable PackedDemand
  -> one rerail_matches(:batch) xFilter execution
  -> one materialized typed relation
  -> SQL clause evaluation and violation extraction
```

The semantic matcher remains a test-only oracle.
Production has no matcher fallback and no second execution shape.

The current implementation does not complete this design.
`PackedProgramPlan` and `Program::lower_matcher` still attempt whole-matcher lowering.
The workspace tape-preparation and result-publication stages also remain incomplete.

## Decision

The compiler lowers complete clauses into SQL relational plans.
The compiler does not lower complete clauses into packed programs.

The SQL lowerer creates terminal programs when a SQL relation needs native work.
These programs become immutable inputs to `rerail_matches(:batch)`.
The virtual table executes all demanded programs when SQLite first calls `xFilter`.

SQLite owns these operations:

- Ordered boolean composition.
- Capture-environment composition.
- Navigation and field-relation joins.
- Ordered alternatives.
- Counts, grouping, thresholds, and absence checks.
- Utility composition and bounded recursion.
- Three-valued truth propagation.
- Cross-file relations.
- Clause-type semantics.
- Violation extraction and stable output order.

Packed programs own bounded terminal work:

- Structural `PatternNode` matching.
- Kind, field, flag, text, regex, and value tests.
- Ordered pattern captures.
- Navigation candidate production when SQL needs an axis relation.
- File-local semantic fact production.
- Parsed-document and generated-document terminal work.
- Explicit completion, work-limit, and failure records.

A packed program can optimize a complete capture-free terminal expression.
This optimization does not move clause composition out of SQL.

## Non-goals

This design does not restore the deleted callback SQL engine.
It does not add SCM to the production compiler.
It does not add a permanent native matcher route.
It does not execute one SQL statement for each file, clause, root, or program.
It does not let SQL call a packed program for each input row.

## Why the deleted SQL modules stay deleted

The deleted compiler encoded SQL text and host execution as one mechanism.
Its SQL called these host operations:

- `tq_pattern` for matcher execution.
- `tq_pred` for semantic predicates.
- `tq_util` and `tq_call` for utility execution.
- `rerail_nodes` for host tree walks.
- `rerail_facts` for lazy host fact production.
- Scalar functions for mutable environment and walk state.

Integer values in that engine referred to mutable host environments.
SQL evaluation order controlled incremental host work.
The virtual tables also performed semantic work during row access.

The callback model conflicts with the required production boundary.
Therefore, the deleted modules must not return as compiler dependencies.

Their parity fixtures remain useful.
They describe ordering, `stopBy`, counts, unknown values, and capture behavior.
The new compiler can adapt those fixtures without restoring callback code.

## Compiler input and output

The compiler consumes validated Rerail values:

- `CompiledCodeRule`.
- `CompiledClause`.
- `Query`.
- `GrammarMatcher`.
- Utility components.
- Violation and fix metadata.
- Capabilities and limits.

The compiler does not parse policy JSON.
It also does not parse SCM or reconstruct ast-grep patterns from source text.

The logical compiler result is:

```text
SqlCatalogPlan {
  statement
  relation_schema
  terminal_programs
  terminal_schemas
  fact_schemas
  task_requirements
  completion_requirements
  result_schema
  stable_order
}
```

The SQL statement and the terminal catalog share stable compiler-assigned identifiers.
A SQL terminal reference always identifies one verified program and one result schema.

## Compilation phases

### 1. Normalize the clause

The compiler converts a clause into a callback-free relational IR.
The conversion preserves source order, capture dependencies, and clause limits.

Normalization does not create SQL text.
It also does not run a matcher or inspect a source tape.

### 2. Extract terminal work

The compiler finds operations that SQLite cannot perform from existing rows.
It creates or reuses a terminal specification for each such operation.

Examples include:

- One ast-grep `PatternNode`.
- One node-text regex.
- One value predicate.
- One requested navigation axis.
- One binding-fact family.
- One parsed-literal operation.

Equal terminal specifications share one program within the catalog.
Their identity includes every semantic input that changes the result.

### 3. Lower terminals to verified programs

Each terminal specification lowers into a packed `Program` and a typed result schema.
The verifier rejects an unsupported operation before workspace execution.

The compiler never stores a `GrammarMatcher` in a packed demand.
The executor never calls the semantic matcher.

### 4. Render one SQL statement

The SQL renderer converts the relational IR into named CTEs.
The statement reads `rerail_matches(:batch)` once through a materialized CTE.

The renderer uses explicit logical order columns.
It does not depend on SQLite row order or backend completion order.

### 5. Build the immutable demand

The workspace supplies tapes, source metadata, selected roots, and limits.
The plan supplies programs, schemas, producer identities, and task requirements.

Preparation creates a complete sparse file/program task grid.
Preparation does not run terminals or create result rows.
Registration stores the immutable demand and returns its token.
Registration performs no semantic work.

## Relational IR

The IR records rule meaning without SQLite callbacks or backend details.
A representative operation set is:

```text
ScanTerminal(terminal_id)
ScanFact(producer_id, fact_kind)
Complete(producer_id, file_id)
Truth(true | false | unknown)
Bind(input, capture_slot, value)
Join(left, right, capture_compatibility)
UnionOrdered(branches)
AntiJoin(input, excluded, completion)
Navigate(axis, input, candidates)
StopAt(candidates, stop_relation)
FirstBy(input, semantic_order)
Count(input, minimum, maximum, distinct_key)
Group(input, keys)
Project(input, columns)
RecursiveUtility(component, arguments, limit)
MapDocument(input, source_map)
ProjectViolation(input, violation_plan)
Order(input, logical_keys)
```

The final implementation can use different Rust names.
The semantic boundaries in this list must remain explicit.

The IR cannot contain these values:

- Host callbacks.
- Closures.
- Mutable environment handles.
- Matcher objects.
- Prepared SQLite statements.
- Backend-specific pointers.
- Lazy fact producers.

## Terminal registry

The SQL lowerer owns one terminal registry for a compiled catalog.
A relational operation requests a terminal through this registry.

For example:

```text
AstMatcher::Pattern(pattern)
  -> terminal_registry.pattern(pattern, language)
  -> terminal_id 17
  -> SQL ScanTerminal(17)
```

A relation can request more than one program:

```text
AstMatcher::Has(inner, stop)
  -> navigation producer for descendant candidates
  -> terminals required by inner
  -> terminals required by stop
  -> SQL joins, orders, stops, and selects the first accepted candidate
```

The relation does not become one packed program by default.
A later optimization can fuse proven-equivalent terminals without changing the relational IR.

## Packed relation boundary

`rerail_matches(batch)` exposes one materialized relation for the complete demand.
The relation contains tagged, typed rows from all requested producers.

The logical row families are:

```text
batch_status
producer_status
terminal_match
capture
navigation
fact
document_map
```

Each row includes stable batch, producer, file, task, and local ordinals where applicable.
Each producer status identifies completion, verdict, work use, and failure.

A physical virtual-table schema can use a tagged union of nullable columns.
The SQL statement creates one typed CTE for each logical row family.

The current 13-column match schema is not sufficient for this compiler.
It lacks producer kinds, navigation rows, fact rows, and document mappings.
The schema must expand before complete relational lowering.

## One-statement execution

The generated statement starts with one materialized scan:

```sql
WITH packed AS MATERIALIZED (
  SELECT *
  FROM rerail_matches
  WHERE batch = :batch
),
producer_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = :producer_status
),
terminal_match AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = :terminal_match
),
capture AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = :capture
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = :navigation
),
fact AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = :fact
)
-- Generated clause CTEs follow.
SELECT ...
```

The example uses symbolic row-kind parameters for clarity.
The compiled statement can use fixed numeric constants.

The statement contains no custom scalar callback.
It contains no second scan that can start another terminal execution.
It does not enter SQLite recursively.

## Virtual-table execution

The host registers one immutable `PackedDemand` before statement execution.
The registration operation does not execute any producer.

The first `xFilter` performs these actions:

1. It claims the demand through its one-execution cell.
2. It runs the complete sparse task grid once.
3. It records typed rows and explicit producer statuses.
4. It stores the result relation or the batch failure.
5. It gives a read-only cursor to SQLite.

`xNext` and `xColumn` read stored rows only.
A later cursor reads the same stored relation.
No cursor operation invokes a matcher, relation, or fact producer.

## SQL lowering rules

### Atomic tests

`Pattern`, `Kind`, `Regex`, and value tests create terminal references.
SQL reads their verdict, match, and capture rows.

A trivial kind test can use an existing typed node value when one exists.
This case does not require a new program.

### Ordered conjunction

`All` joins terms in source order.
Each term reads the environment relation produced by its predecessor.
A failed term removes that environment.

A three-valued conjunction retains unknown state and withheld bindings.
A later false term still decides the result.

### Ordered alternatives

`Any` unions branch results with an explicit branch ordinal.
SQL selects the first accepted branch for each input environment.

An unknown branch that can bind captures blocks an incompatible later branch.
The IR records this dependency before SQL rendering.

### Negation

`Not` evaluates its input in an isolated environment relation.
SQL discards bindings from the nested relation.

SQL can prove a negative result only after the nested producer reports completion.
An unknown or failed nested result does not become false.

### Navigation relations

Relations use typed navigation rows and typed terminal rows.
Navigation rows include axis order and source and target node identities.

SQL applies `stopBy`, field constraints, and candidate order.
It selects the first accepted candidate when the language contract requires one.

A navigation producer can emit only the requested axes and candidate ranges.
The engine does not need one complete corpus-wide node table.

### Constraints and capture equality

Captures are relational rows, not host environment objects.
A terminal match has a local capture sequence with explicit slot and occurrence order.

SQL joins compatible capture rows across terms.
A packed equality terminal supplies structural equality when byte equality is insufficient.

### Counts

SQL counts rows only after every required producer reports completion.
Unknown walks retain lower and upper bounds when the rule contract requires them.

`distinct` uses explicit value or capture keys.
It never uses physical row identity.

### Utilities

The compiler lowers utilities into named relational components.
Stable component identity does not depend on table position.

Recursive strongly connected components use bounded recursive CTEs or an explicit bulk operator.
The selected form must preserve source-ordered errors and utility limits.
It must not prepare or enter another statement.

### Semantic predicates

A semantic producer emits typed facts without evaluating nested user rules.
SQL evaluates nested rules over those fact rows.

For example, a binding producer can emit these values:

- The binding site.
- The binding value.
- The annotation.
- Ordered occurrences.
- Shadowed or related bindings.
- A producer-completion status.

The SQL plan applies user sub-rules to the emitted node identities.
The producer does not call those sub-rules.

### Cross-file clauses

File-local programs emit evidence and completion rows in the same demand.
SQL performs deterministic cross-file joins, anti-joins, budgets, and thresholds.

Incomplete evidence cannot prove absence.
A failed dependency prevents publication of partial violations.

### Output operations

SQL owns `violation.each`, alternatives, projection, and final ordering.
Typed output terminals can supply transforms that require bounded native code.

The final projection creates complete violation rows.
The host publishes only these returned rows.
The host does not reconstruct a missing relation or call the matcher.

## Capture environments

The new engine does not use integer handles for mutable Rust environments.
It represents each environment as relational identity and binding rows.

The logical model is:

```text
environment(
  environment_id,
  parent_environment_id,
  branch_order,
  verdict,
  complete
)

environment_binding(
  environment_id,
  capture_slot,
  occurrence_order,
  binding_kind,
  node,
  sequence_start,
  sequence_count,
  text_value
)
```

The physical SQL can avoid materializing every parent row.
Any optimized representation must preserve the same logical data.

The representation must support:

- Single captures.
- Multi-captures.
- Capture occurrence order.
- Repeated-capture equality.
- Alternative rollback.
- Negation isolation.
- Withheld bindings after unknown terms.
- Utility argument frames.
- Transformed text values.

## Completion and failure

Every demanded `(file, producer)` pair emits a status row.
An empty root list is a demanded task with a complete negative result.
A missing sparse-grid pair means that the pair was not demanded.

These states are distinct:

- Complete positive.
- Complete negative.
- Semantic unknown.
- Work limit.
- Witness limit.
- Backend capacity.
- Parser failure.
- Cancellation.
- Invalid packed input.

SQL cannot infer a negative result from missing match or fact rows.
It must join the required completion rows before an anti-join or count proves absence.

A failed clause publishes no partial violations.
The engine does not retry through the semantic matcher or another backend.

## Deterministic order

Each relation carries explicit logical order columns.
The order keys include these values where applicable:

- Catalog and clause ordinal.
- File ordinal.
- Root preorder ordinal.
- Rule-term ordinal.
- Alternative ordinal.
- Navigation position.
- Terminal match ordinal.
- Capture occurrence ordinal.
- Violation ordinal.

Backend lane order and physical SQLite row order are not semantic inputs.
The final SQL projection includes an explicit `ORDER BY` over logical keys.

## Landing source use

The NER-1562 Landing-2 history supplies terminal-execution components.
Useful components include:

- Structural wordcode construction.
- Value and capture programs.
- CPU and Metal backends.
- Sparse task grids.
- Typed result pages.
- Explicit evaluation status.
- Bounded workspaces.
- Stable result ordering.

The SCM lowerer is a reference for terminal-program construction only.
SCM does not become a Rerail production input.

The Landing workspace executor is not a direct donor for SQL orchestration.
It executes `AstMatcher::Program` values before result publication.
Rerail requires SQLite to demand execution through `xFilter`.

The current dirty Landing worktree is research input only.
Ports must use named clean commits and must include their parity tests.

## Implementation sequence

### Phase 1: Replace the top-level compiler boundary

1. Define the callback-free relational IR.
2. Define the terminal registry and stable terminal identities.
3. Replace `PackedProgramPlan` with `SqlCatalogPlan`.
4. Remove whole-matcher `Program::lower_matcher` from production compilation.
5. Add a complete unsupported-operation inventory.

### Phase 2: Expand the packed relation schema

1. Add row kinds and producer identities.
2. Add explicit producer-status rows.
3. Add navigation and fact rows.
4. Add typed capture-sequence rows.
5. Preserve one `xFilter` and one terminal execution.

### Phase 3: Lower the structural core

1. Lower atomic tests into terminal references.
2. Lower ordered `All`, `Any`, and `Not` into relational IR.
3. Lower navigation, fields, `stopBy`, and `nthChild` into SQL.
4. Lower constraints, equality, and counts.
5. Prove complete outcome parity with the semantic matcher.

### Phase 4: Lower utilities and semantic facts

1. Lower utility components and parameterized calls.
2. Add typed fact producers.
3. Lower nested semantic sub-rules over fact rows.
4. Add parsed and generated document mappings.
5. Add cross-file calls and evidence relations.

### Phase 5: Lower clause and output semantics

1. Lower `forbid`, `requireEach`, `budget`, and `selectWhen`.
2. Lower anchors, templates, fixes, suppression, and transforms.
3. Lower `violation.each` and projections.
4. Add deterministic final extraction.
5. Reach zero unsupported public operations.

### Phase 6: Complete workspace integration

1. Implement inert tape and task-grid preparation.
2. Register one immutable demand.
3. Execute one generated SQL statement.
4. Publish only the returned violation rows.
5. Remove temporary stubs and dead-code allowances.

## Compiler contract suite

`native/src/code_rule_dsl/query/lowering_contract.rs` contains 108 hand-written lowering forms.
Each form declares its required terminal kinds and normalized SQL constructs.

The suite covers atomic tests, boolean composition, navigation, captures, utilities, semantic facts, clauses, and outputs.
It also rejects callback-shaped SQL fragments.

Fixture-validation tests make sure that the contract is complete and internally consistent.
They do not claim that lowering exists.

An active TDD test requires exact terminal and SQL-shape equality for all 108 forms.
It fails with an explicit implementation error until the relational compiler exists.
Connect this test to `SqlCatalogPlan::explain` when that API exists.

Do not rewrite a hand-written form into an easier equivalent to make the implementation pass.
Add separate TDD cases from real compiled policies as each compiler API becomes available.
Matcher-oracle tests remain necessary because an SQL shape does not prove semantic parity.

## Architecture gates

The production cutover must satisfy these gates:

- Every valid `rerail.policy/v1` clause compiles into relational IR.
- Every native terminal lowers into a verified packed program.
- The generated statement reads one materialized packed relation.
- One apply phase executes one statement.
- The first `xFilter` executes the complete demand once.
- Registration performs no terminal, relation, fact, or matcher work.
- Production executes zero custom scalar callbacks.
- Production exposes no `rerail_nodes`, `rerail_facts`, or `tq_*` path.
- Production stores no semantic matcher in a demand.
- Every demanded producer emits an explicit completion or failure status.
- Complete SQL outcomes equal matcher-oracle outcomes.
- Output order and failure selection remain deterministic.

## Current benchmark status

There is no valid post-cutover full-scan result.
The exact data-layer scan stops during whole-matcher packed lowering before SQLite execution.

The last 17.189-second full scan measured the deleted matcher-backed path.
The later rejection run measured only early compiler failure.
Neither result measures this design.
