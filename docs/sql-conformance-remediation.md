# SQL conformance remediation

## Purpose

This plan corrects the 108 SQL contracts and completes the SQL-only execution path.
Each valid policy must compile, execute, and return the same ordered result as the test oracle.

The production path must remain:

```text
compiled clause
  -> callback-free relational IR
  -> one SQL statement and one terminal catalog
  -> one immutable PackedDemand
  -> one rerail_matches(:batch) xFilter execution
  -> one typed relation
  -> SQL clause evaluation and violation extraction
```

The semantic matcher is a test oracle only. Production must not use it as a terminal, callback, compatibility path, or fallback.

## Definition of complete

The 108-case gate is complete only when each case has these artifacts:

- One complete `rerail.policy/v1` policy.
- All source files and file metadata required by the behavior.
- Required document maps, capability limits, and failure injection.
- One authored exact normalized SQL statement.
- One authored exact ordered terminal catalog.
- Exact ordered violations and failures from the matcher oracle.
- Exact ordered violations and failures from SQL execution.
- Exact architecture counters for registration, statements, filters, and terminal execution.

A valid policy that reports unsupported lowering fails the gate.

## Contract fixture model

Replace the current three-field fixture with a typed structure.

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractCase {
    name: String,
    form: String,
    policy: Value,
    workspace: ContractWorkspace,
    expected: ContractExpected,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractWorkspace {
    files: Vec<ContractFile>,
    document_maps: Vec<ContractDocumentMap>,
    capabilities: Value,
    failure_injection: Option<FailureInjection>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractExpected {
    normalized_sql: String,
    terminal_catalog: Vec<ExpectedTerminal>,
    violations: Vec<Value>,
    failures: Vec<Value>,
    counters: ExpectedCounters,
}
```

`ContractFile` must contain the path, language, source, and optional module identity.
The fixture loader must reject missing fields, unknown fields, duplicate names, and duplicate policy IDs.

### TDD criteria

**Red:** Load the current fixtures with the strict structures. The test must list all missing contract fields.

**Green:** All 108 fixtures load without defaults for semantic data.

### Validation

- Parse each fixture twice and compare the values.
- Require exactly 108 unique names.
- Require exactly 108 unique policy IDs.
- Require one complete policy in each case.
- Reject null SQL, catalog, workspace, or expected-result fields.

## Exact SQL contracts

Store the complete normalized SQL for every case. Normalization can change whitespace only.
It must preserve CTE names, columns, predicates, joins, identifiers, parameters, and ordering.

Do not add a fixture-update mode that reads `SqlCatalogPlan` output.
Do not accept SQL fragments, operation lists, or hashes as exact SQL evidence.

### TDD criteria

**Red:** Compare all compiled statements with authored SQL. The test must fail for every missing or different statement.

**Green:** Every policy produces its authored statement.

### Validation

- Prepare every statement with SQLite.
- Bind all required parameters.
- Reject unused fixture parameters.
- Run repeated and parallel compilation.
- Require byte-identical normalized SQL for every run.

## Exact terminal-catalog contracts

Each expected terminal entry must contain:

- Terminal ordinal.
- Terminal kind.
- Language dialect.
- Producer kind.
- Canonical program digest.
- Result schema.
- Owning clause IDs.
- Result limit.
- Completion requirement.

### TDD criteria

**Red:** Compare the complete ordered catalog. The current test must fail because it has no expected catalogs.

**Green:** Every catalog equals the authored catalog.

### Validation

- Compile each case repeatedly.
- Compile the complete catalog in different thread schedules.
- Require stable terminal identity and order.
- Require each SQL terminal reference to resolve to one catalog entry.
- Reject unreferenced catalog entries.

## Contract-policy corrections

### `node-text-equality`

The current policy compares two captures. The compact form requires node text and a literal.
Use the public policy representation for exact literal node text.
Add one equal source and one unequal source.

**Red:** The unequal source currently produces the wrong result.

**Green:** Only the exact literal matches.

**Validation:** Include Unicode, nested nodes, and equal substrings with different complete node text.

### `any-first-branch`

The current policy uses metavariable-only patterns. The compact form requires literal branches `a` and `b`.
Restore the literal branches and preserve their authored order.

**Red:** Use a fixture that exposes branch substitution or reversed branch priority.

**Green:** SQL selects the first successful authored branch.

**Validation:** Change source order without changing branch order. The selected branch must not change.

### `utility-parameterized-call`

The current utility has one parameter. The compact form requires node and text arguments.
Declare both parameters and read both in the utility body.

**Red:** Change only the second argument. The current policy result remains unchanged.

**Green:** Either argument can change the result.

**Validation:** Execute all four combinations of matching and non-matching arguments.

### `utility-depth-limit`

The current fixture does not reach the utility depth limit.
Add a tree deeper than the case-specific limit.

**Red:** Require the exact depth-limit failure.

**Green:** SQL returns the same failure as the oracle and returns no partial violation.

**Validation:** Add a second tree one level below the limit. It must complete.

### `utility-work-limit`

The current fixture does not exhaust `maxVisitedNodes`.
Set a small case-specific limit and add a recursive source that exceeds it.

**Red:** Require the exact work-limit failure.

**Green:** SQL returns the failure with an incomplete producer status.

**Validation:** Increase the limit enough to complete the same source.

### `count-distinct-capture`

The current policy uses a clause budget. It does not represent a related-node count with a distinct capture key.
Create a fixture that counts related matches with duplicate capture values.

**Red:** The current substitution counts the wrong relation or grouping unit.

**Green:** SQL counts distinct capture values for the related-node count.

**Validation:** Cover duplicate text on different nodes, repeated occurrences, branch duplicates, and incomplete input.

### `capture-projection`

The current policy uses a budget wrapper. It does not isolate relation projection.
Use a genuine evidence relation with two projected capture fields.

**Red:** Require two projected fields from one evidence row.

**Green:** SQL preserves both values and their source row identity.

**Validation:** Use rows with one equal projected field and one different field.

### `semantic-generated-document-map`

A Blade `echo` policy alone does not test document mapping.
Add a Blade source, generated PHP, and an exact source map.
Match the generated PHP node and map the violation to Blade.

**Red:** Require document-map rows and a Blade source span. The current executor emits neither.

**Green:** SQL returns the Blade path and Blade span.

**Validation:** Cover direct, split, missing, and overlapping mappings.

### `semantic-fact-producer-failure`

The current policy contains a normal binding query. It does not cause a producer failure.
Add deterministic, test-only capacity failure injection for the fact producer.

**Red:** Require an incomplete producer status and the exact capacity error.

**Green:** The batch emits no partial violation after the failure.

**Validation:** Require zero fact rows after the failed boundary.

### `clause-file-condition`

The current policy tests only the path. The contract requires path, language, and content behavior.
Use all public file-condition operands and files that fail one operand at a time.

**Red:** A file that fails language or content currently reaches terminal execution.

**Green:** Only the file that meets all file conditions creates tasks.

**Validation:** Rejected files must have no producer status because no task was demanded.

### `clause-syntax-error-unknown`

The current fixture has no malformed source.
Add malformed source with an error region that overlaps the rule.

**Red:** Require unknown state for the malformed source.

**Green:** SQL does not turn the unknown state into a negative result.

**Validation:** Correct the source and require a complete result.

### `output-complete-negative`

The current case has no source that proves a complete negative result.
Add a source with no terminal match and require an explicit complete producer status.

**Red:** Remove the completion row. The current SQL can infer absence incorrectly.

**Green:** A complete negative produces no violation and no failure. An incomplete task produces a failure.

**Validation:** Run both row sets through the same SQL statement.

### `never`

The current public policy expresses an empty relation through a contradiction.
Do not claim that this policy compiles to the internal `AstMatcher::Never` variant.

**Red:** Require an executable empty relation against a source with many identifiers.

**Green:** SQL returns no rows and performs no unnecessary semantic work.

**Validation:** Compare empty, small, and large files.

### `disabled-when`

`SqlCatalogPlan` currently ignores `GrammarMatcher::applies`.
Lower `applies = false` to an empty relation with no demanded terminal task.

**Red:** Require zero tasks and zero violations for the disabled options.

**Green:** Change the language option so that `when` becomes true. The terminal must then execute.

**Validation:** Compare the task grid before SQL execution.

### Output-policy cases

The lowerer currently ignores key fields in `ViolationSpec`.
The affected contracts include:

- `capture-each-distinct`
- `capture-anchor`
- `capture-anchor-fallback`
- `capture-transform`
- `capture-message-template`
- `capture-fix-range`
- `output-fix-and-suppression`
- `output-stable-multiplicity`

**Red:** Require exact final violation rows for every output case.

**Green:** SQL owns anchors, templates, transforms, fixes, suppression, multiplicity, and ordering.

**Validation:** Compare every violation field with the oracle.

## Comparison IR

`AstMatcher::Compare` currently reports unsupported lowering.
Add a `RelationNode::Compare` variant with explicit operands, comparison mode, collation, and truth behavior.
Read capture values from the relational binding environment.

### TDD criteria

**Red:** Execute equality, inequality, ordering, missing-capture, and unknown fixtures.

**Green:** SQL evaluates comparisons without a matcher terminal.

### Validation

Cover every public text mode and collation used by policy packs.

## Three-valued value IR

Value predicates currently report unsupported lowering.
Add immutable value-fact producer descriptors and explicit relational truth state.
Use distinct values for true, false, and unknown.

### TDD criteria

**Red:** Require the value and three-valued Boolean contracts to match the oracle.

**Green:** SQL preserves unknown state through `All`, `Any`, and `Not`.

### Validation

Cover false after unknown, withheld captures, unknown alternatives, and negated unknown values.

## Utility IR

`AstMatcher::Util` and `AstMatcher::Call` currently report unsupported lowering.
Lower stable `UtilityNode` components into named relational expressions.
Use explicit argument frames for parameterized calls.
Use bounded recursive CTEs for recursive components.

### TDD criteria

**Red:** Execute all utility contracts against their oracle fixtures.

**Green:** All utility contracts produce executable SQL.

### Validation

Cover shared components, nested calls, self-recursion, mutual recursion, argument isolation, returned captures, and limits.

## Semantic fact producers

`AstMatcher::Semantic` currently reports unsupported lowering.
Compile each semantic operation into an immutable fact-producer descriptor.
A producer must emit facts only. It must not execute nested user rules.
SQL must apply nested rules to fact node identities.

Implement these fact families:

- Symbol resolution.
- Binding sites.
- Binding values.
- Binding annotations.
- Binding occurrences.
- Binding creation.
- Redefinition.
- Value facts.
- Argument facts.
- Parsed-literal facts.
- Format-argument facts.
- Layout facts.
- Call facts.

### TDD criteria

**Red:** Require exact fact rows and final outcomes for each semantic family.

**Green:** Each producer emits typed facts and one explicit completion status.

### Validation

Reject unknown fact kinds, invalid node identities, missing completion, and partial facts after failure.

## Parsed-document producers

Parsed literals currently report unsupported lowering.
Add immutable parsed-document producer descriptors.
Emit parsed-document identity, host identity, parsed node identity, captures, and completion.

### TDD criteria

**Red:** Execute parse, host, and capture-export contracts.

**Green:** All parse work occurs inside the first `xFilter`.

### Validation

Cover successful parse, empty parse, parser failure, host access, and exported captures.

## Document-map producers

The 35-column relation reserves document-map columns but emits no document-map rows.
Add `PackedRow::document_map` and its verified producer descriptor.

### TDD criteria

**Red:** Require exact map rows for a generated document.

**Green:** Emit map rows during the first `xFilter` execution.

### Validation

Reject absent documents, invalid node identities, invalid spans, and nondeterministic overlap order.

## Regex clause lowering

`forbid(regex)` clauses currently report unsupported lowering.
Add content-regex and path-regex terminal descriptors.
Add file source and path metadata to `PackedDemand`.

### TDD criteria

**Red:** Execute content and path regex contracts through SQL.

**Green:** SQL returns exact captures, spans, completion, and output rows.

### Validation

Cover named groups, fixes, result limits, and complete negatives.

## `select_when` lowering

`Query::SelectWhen` currently reports unsupported lowering.
Lower the `select` and `exists` operands as separate relations.
Use completion-guarded `EXISTS` and anti-join expressions.

### TDD criteria

**Red:** Execute present, absent, unknown, and failed existence fixtures.

**Green:** Incomplete existence input cannot prove absence.

### Validation

Require exact task dependencies for both operands.

## `require_each` lowering

`ClauseKind::RequireEach` currently reports unsupported lowering.
Lower subject and evidence relations with a completion-guarded anti-join.

### TDD criteria

**Red:** Require one violation for each unmatched subject.

**Green:** Evidence matches the authored projected keys.

### Validation

Cover duplicate subjects, duplicate evidence, cross-file evidence, and incomplete evidence.

## Budget lowering

Budget maximum, distinct, and grouping currently report unsupported lowering.
Lower query rows, grouping keys, distinct keys, thresholds, and output projection into SQL.

### TDD criteria

**Red:** Execute all budget contracts with boundary values.

**Green:** SQL owns grouping, counting, distinct values, and threshold comparison.

### Validation

Cover zero rows, exact limits, one row over the limit, duplicate rows, and multiple groups.

## File-condition lowering

The SQL plan currently ignores matcher file conditions.
Add immutable file metadata rows and apply file conditions before terminal demand.

### TDD criteria

**Red:** Require no terminal tasks for rejected files.

**Green:** Only eligible files appear in the sparse task grid.

### Validation

Count demanded tasks, producer statuses, and terminal rows by file.

## Fact rows in `rerail_matches`

The physical relation defines `RowKind::Fact`, but `PackedDemand::execute` emits no fact rows.
Add a typed constructor and strict column invariants.

### TDD criteria

**Red:** Require exact fact rows for one binding fixture.

**Green:** The fact producer emits rows and one status row.

### Validation

Reject fact rows with missing producer, file, node, kind, or provenance data.

## Document-map rows in `rerail_matches`

The relation defines `RowKind::DocumentMap`, but execution emits no document-map rows.
Add a typed constructor and strict mapping invariants.

### TDD criteria

**Red:** Require exact map rows for one Blade fixture.

**Green:** The map producer emits stable ordered rows and completion.

### Validation

Reject mappings with missing documents, invalid spans, or invalid node identities.

## Per-clause producer dependencies

The final SQL currently blocks all clauses when any producer is incomplete.
Track producer dependencies for each clause.
Guard each clause with only its required producers.

### TDD criteria

**Red:** Fail one producer that belongs to another clause. Require the independent clause to complete.

**Green:** Publish independent clause results in source order.

### Validation

Inject failure at each producer ordinal in a multi-clause catalog.

## Typed producer state

Use a closed state model for producer outcomes:

- Complete positive.
- Complete negative.
- Semantic unknown.
- Work limit.
- Witness limit.
- Capacity failure.
- Parser failure.
- Cancellation.

### TDD criteria

**Red:** Add one relation fixture for each state.

**Green:** Reject invalid state and column combinations.

### Validation

For example, reject `complete = 1` with a capacity-failure error.

## `prepare_packed_inputs`

`prepare_packed_inputs` currently returns an unimplemented error.
Implement immutable tape restoration and sparse-task preparation.

The output must include:

- Restored tapes.
- File dialects.
- Clause eligibility.
- Selected roots.
- Sparse file-and-producer tasks.
- File metadata.
- Syntax-error ranges.
- Document maps.
- Per-task limits.

Preparation must not execute a producer or construct a witness.

### TDD criteria

**Red:** Require exact sparse tasks for a multi-file, multi-clause workspace.

**Green:** Return valid `PackedFileInput` values without execution.

### Validation

Require zero match, capture, fact, map, and violation rows before SQL starts.

## Inert registration

Registration must store immutable demand data only.
Only the first `xFilter` can execute the demand.

### TDD criteria

**Red:** Register a demand and inspect counters before SQLite reads it.

**Green:** Registration records zero work and zero result rows.

### Validation

Prepare and close a statement without reading the virtual table. Execution must remain zero.

## `publish_sql_outcome`

`publish_sql_outcome` currently validates counters and returns an unimplemented error.
Change the final SQL projection to return typed violation rows.
Publish those rows without matcher calls or host-side reconstruction.

### TDD criteria

**Red:** Require a complete workspace result for a structural policy.

**Green:** Return violations, failures, coverage, and metrics from SQL rows only.

### Validation

Remove one required output column in a test statement. Publication must reject the row.

## SQL violation schema

`QueryOutcome.rows` currently contains producer `PackedRow` values.
Create a separate `SqlViolationRow` type for final SQL output.

Required non-null fields are:

- Rule ID.
- Clause ID.
- Code.
- Message.
- Path.
- Start and end positions.
- Rule mode.
- Stable order key.

Optional typed fields include fix data, variables, and suppression provenance.

### TDD criteria

**Red:** Require complete typed construction for one final violation row.

**Green:** `publish_sql_outcome` accepts only `SqlViolationRow` values.

### Validation

Reject null required fields and invalid spans.

## Architecture validation

Add executable checks for these invariants:

- One outer SQL statement per workspace batch.
- One `rerail_matches` reference in the statement.
- One virtual-table `xFilter` call.
- One terminal execution.
- A second cursor reads stored rows without execution.
- Registration performs no semantic work.
- Production records zero matcher calls.
- Production records zero scalar callbacks.
- Production records zero host relation walks.
- Unsupported lowering fails before registration.
- Production has no fallback engine.

### TDD criteria

**Red:** Run one complete workspace case and require all counters.

**Green:** Every counter has the required value.

### Validation

Run the architecture checks with structural, semantic, cross-file, negative, and failure cases.

## Current status and execution direction

Phases 1 through 4 are complete.
The aggregate Phase 2 through Phase 4 SQL gate is green for all 39 contracts.
The aggregate ordered terminal-catalog gate is green for all 39 contracts.
All Phase 2 through Phase 4 behavioral oracle checks are green.

The completed aggregate contracts include:

- Thirteen Phase 2 truth and value statements.
- Ten Phase 3 utility statements.
- Sixteen Phase 4 semantic fact statements.
- Thirty-nine ordered terminal catalogs with 95 terminal entries.

The catalog fixture contains only ordered terminal-catalog data.
The SQL fixture is the only owner of authored SQL statements.
This separation prevents two fixtures from assigning different meanings to one SQL contract.

Commit `cd4dbb1d` is the aggregate TDD-red commit for incomplete Phase 2 and Phase 3 SQL boundaries.
Commit `0601812d` is the completed focused red commit for nested Phase 2 truth composition.
Commit `17decf57` is the completed focused red commit for leaf utility SQL.
Commit `2d4cc9cc` is the completed focused red commit for capture utility SQL.
Commit `2fbafd0e` is the completed focused red commit for recursive utility SQL.
Commit `c90da557` is the completed focused red commit for isolated utility-frame SQL.
Commit `89ee5bb4` is the completed focused red commit for typed utility-limit SQL.
Commit `e5197b32` is the completed focused red commit for parsed-document SQL behavior.
Commit `dd3e38c1` is its TDD-green implementation.
Commit `e63f33be` is the completed focused red commit for generated-document mapping.
Commit `24e32bed` is its TDD-green implementation.
Commit `c2358ea9` is the active red commit for exact Phase 5 SQL and terminal catalogs.

Phase 5 is in progress.
Parsed literals, relational host predicates, and exported parsed captures now execute through bounded
parse-fact descriptors and SQL composition. Generated results cross typed document-map rows in SQL.
Missing maps, invalid spans, and overlaps fail closed before registration. Mapped spans equal the
committed oracle for the generated-document contract.

Authored Phase 5 SQL and catalog contracts remain incomplete. Source-path identity assertions and
atomic parse-capacity failure tests also remain incomplete.

The next required package is exact Phase 5 SQL and ordered terminal catalogs.
Do not change the completed Phase 2 through Phase 4 contracts to accommodate later work.
All 39 Phase 2 through Phase 4 catalogs pass exact, repeated, and concurrent compilation checks.

### Relation to the packed execution spike

The target execution shape matches the Landing packed-batch spike:

```text
one policy catalog
  -> one callback-free relational plan
  -> one SQL statement
  -> one immutable sparse task grid
  -> one rerail_matches(:batch) xFilter
  -> one packed backend invocation
  -> one materialized typed relation
  -> SQL violation extraction
```

A terminal catalog does not define the backend invocation count.
Many files, programs, roots, and clauses share one sparse task grid.
The Metal JIT selects the program from `task.program` inside one kernel dispatch.

SQLite runs relational composition on the CPU.
This work does not cause more GPU dispatches.
The packed backend runs bounded terminal and fact work.

A normal file-local batch has this target GPU shape:

1. An optional anchor-selection dispatch runs.
2. One packed terminal and fact dispatch runs.
3. A later consumer dispatch runs only when a required workspace phase exists.

A hard backend limit can partition an internal dispatch.
The partition must not create another SQL statement or another `xFilter` execution.
The demand records the partition count and its cause.

This shape is not yet an end-to-end production result.
`prepare_packed_inputs` and `publish_sql_outcome` remain incomplete.
Pattern execution also remains CPU-only in the active route.
There is no valid post-cutover launch trace or benchmark.

### Execution proof package

Phases 5 through 7 must complete the remaining semantic inputs before the execution proof starts.
Phase 8 then proves the immutable demand and one-execution boundary.
Phase 9 proves the complete workspace route.

The execution proof must use a multi-file and multi-program workspace.
The workspace must include positive, negative, semantic, and failed tasks.
The proof must record these counters:

- Demand registrations.
- SQL statements.
- `rerail_matches` references.
- `xFilter` calls.
- Backend invocations.
- Internal backend partitions.
- Terminal tasks.
- Metal command buffers.
- Metal kernel dispatches.
- Matcher calls.
- Host callbacks.

For one admitted file-local batch, the required values are:

```text
registrations        = 1
SQL statements       = 1
rerail_matches scans = 1
xFilter calls        = 1
backend invocations  = 1
terminal tasks       > 1
matcher calls        = 0
host callbacks       = 0
```

On Metal, one unpartitioned packed batch must use one packed command buffer and one packed kernel dispatch.
An optional anchor dispatch must have a separate counter.
Do not combine the anchor count with the packed-program count.

## Workplan

Complete each phase before work starts on the next phase.
A phase is incomplete while one required test fails or reports a warning.

### Phase 1: Correct all 108 contracts

**Scope**

- Replace the current fixture model with the strict typed model.
- Correct every policy substitution listed in this document.
- Add source files, metadata, maps, limits, and failure injection.
- Add source fixtures for exact matcher-oracle execution.
- Reserve exact SQL and catalog expectations for each owning implementation phase.

**TDD red**

Commit tests that reject the current incomplete fixtures.
The tests must identify missing data and incorrect policy behavior.

**Implementation**

Author all required contract data without using production compiler output to create expectations.

**Exit criteria**

- All 108 policies pass public policy compilation.
- All fixture invariants pass.
- Matcher-oracle tests pass for all cases.
- Production lowering failures remain visible in the separate implementation gate.

### Phase 2: Complete relational comparison and truth semantics

**Scope**

- Add comparison IR with explicit capture operands.
- Add typed true, false, and unknown states.
- Add value-fact descriptors for literal and flow-dependent values.
- Correct `All`, `Any`, and `Not` propagation.
- Lower disabled `when` queries to empty relations.

**TDD red**

Use the atomic and Boolean contracts to expose each incorrect truth result.
Include withheld captures and false-after-unknown cases.

**Implementation**

Render comparison and truth operations as callback-free SQL.
Emit only bounded value facts from packed producers.

**Exit criteria**

- Atomic, comparison, value, and Boolean contracts pass exact SQL checks.
- Their ordered SQL results equal the oracle results.
- Disabled queries demand no terminal tasks.

### Phase 3: Complete utilities and bounded recursion

**Scope**

- Lower stable utility components.
- Add parameter frames.
- Preserve returned captures.
- Add bounded self-recursion and mutual recursion.
- Enforce depth and work limits.

**TDD red**

Run every utility case with sources that distinguish shared state, isolated frames, and recursion limits.

**Implementation**

Use named relational components and bounded recursive CTEs.
Do not execute utilities through `GrammarMatcher`.

**Exit criteria**

- All utility SQL and catalog goldens pass.
- Utility results equal the oracle.
- Limit failures are typed and contain no partial violations.

### Phase 4: Implement semantic fact producers

**Scope**

- Implement resolution facts.
- Implement binding site, value, annotation, occurrence, creation, and redefinition facts.
- Implement value, argument, layout, format, and call facts.
- Add explicit fact completion and failure states.

**TDD red**

Require exact typed fact rows before final SQL composition.
Require final behavior for every semantic contract.

**Implementation**

Emit facts from immutable producer descriptors inside `xFilter`.
Apply nested user rules in SQL.

**Exit criteria**

- Every semantic fact family has exact row tests.
- Every semantic contract equals its oracle result.
- Fact producers never call nested user matchers.
- Capacity failure produces an incomplete status and no partial violation.

### Phase 5: Implement parsed and generated documents

**Scope**

- Add parsed-document descriptors.
- Add host-node facts and capture export.
- Add generated-document identities.
- Add document-map rows and span mapping.

**TDD red**

Require parse, host, exported-capture, and Blade generated-map behavior.

**Implementation**

Run bounded parse work inside the first `xFilter`.
Map generated results to source documents in SQL.

**Exit criteria**

- Parse and document-map SQL goldens pass.
- Parse and map rows have explicit completion.
- Final paths and spans equal the oracle.
- Invalid or missing maps fail closed.

### Phase 6: Complete clause-level relational semantics

**Scope**

- Lower content and path regex clauses.
- Lower `select_when`.
- Lower `require_each`.
- Lower budget maximum, distinct, and grouping.
- Apply file conditions before terminal demand.
- Preserve syntax-error unknown state.

**TDD red**

Use multi-file fixtures with positive, negative, unknown, and incomplete dependencies.

**Implementation**

Use completion-guarded joins, anti-joins, grouping, and thresholds.
Track producer dependencies per clause.

**Exit criteria**

- Every clause SQL and catalog golden passes.
- Every clause result equals the oracle.
- Rejected files create no terminal tasks.
- One failed clause dependency does not suppress an independent clause.

### Phase 7: Complete SQL violation extraction

**Scope**

- Add output IR for anchors, templates, transforms, fixes, and suppression.
- Add `violation.each` distinct behavior.
- Add stable alternative and capture ordering.
- Add the typed `SqlViolationRow` result schema.

**TDD red**

Require exact final violation rows for all output contracts.

**Implementation**

Project complete violations in the generated SQL statement.
Do not reconstruct output in Rust.

**Exit criteria**

- Every output field equals the oracle.
- SQL returns complete `SqlViolationRow` values.
- Missing captures use the specified anchor fallback.
- Suppressed violations do not reach publication.

### Phase 8: Complete the packed relation and immutable demand

**Scope**

- Emit fact rows.
- Emit document-map rows.
- Add file and path metadata required by terminals.
- Enforce the closed producer-state model.
- Enforce per-producer and per-task limits.
- Keep registration inert.
- Build one immutable sparse file, producer, and root task grid.
- Record backend invocation, partition, command-buffer, and dispatch counters.
- Keep backend selection inside the first `xFilter` call.

The demand contains verified programs, restored tapes, selected roots, metadata, maps, and limits.
The demand does not contain matcher objects, callbacks, prepared statements, or mutable environments.
A missing file and producer pair means that eligibility rejected the task before registration.

Registration stores the demand and returns its token.
Registration does not select more roots, run a producer, or construct a result row.
The first `xFilter` executes all admitted tasks as one backend batch.
A `OnceLock` stores the normalized rows and terminal failure state.
Later cursors read the stored value.

**TDD red**

Require exact physical rows, statuses, and counters for each producer family.
Use one workspace with multiple files, programs, roots, and negative tasks.
Require more than one terminal task and exactly one backend invocation.
Open a second cursor and require no additional execution.

**Implementation**

Extend `PackedDemand` with verified immutable descriptors and metadata.
Execute each demanded task once inside `xFilter`.
Use one Metal program table and dispatch by `task.program` inside the kernel.
Partition only when a recorded hard backend limit requires it.

**Exit criteria**

- Every demanded task has one complete or incomplete status.
- Invalid row-state combinations are rejected.
- Registration creates no result rows.
- Repeated cursors do not repeat execution.
- One unpartitioned batch records one backend invocation.
- One unpartitioned Metal batch records one packed kernel dispatch.
- A partition records its count, limit, and stable task ranges.
- SQL does not change when the selected packed backend changes.

### Phase 9: Complete workspace preparation and publication

**Scope**

- Implement `prepare_packed_inputs`.
- Build the complete sparse file-and-producer task grid.
- Execute one SQL statement.
- Implement `publish_sql_outcome` from `SqlViolationRow` values.
- Remove remaining workspace stubs and transitional matcher scheduling.
- Expose the Phase 8 execution counters in workspace results and profiles.

`prepare_packed_inputs` can restore and verify immutable data.
It must not run semantic work or construct witnesses.
`publish_sql_outcome` accepts only typed final SQL rows.
It must not inspect terminal rows to reconstruct a violation.

**TDD red**

Run complete workspace applies for structural, semantic, cross-file, negative, and failure cases.
For the file-local case, require one statement, one `xFilter`, and one backend invocation.
Remove one required SQL output column and require typed publication failure.

**Implementation**

Prepare immutable inputs without semantic evaluation.
Publish only rows returned by SQL.
Preserve the same SQL statement for CPU, CPU-JIT, Metal, and Metal-JIT backends.

**Exit criteria**

- A workspace apply completes through SQL only.
- One statement causes one `xFilter` and one terminal execution.
- One unpartitioned Metal batch records one packed kernel dispatch.
- Optional anchor work has a separate dispatch counter.
- Production records zero callbacks and zero matcher calls.
- Unsupported lowering fails before registration.
- Production contains no fallback route.
- SQL publication rejects incomplete or inconsistent final rows.

### Phase 10: Complete conformance and performance validation

**Scope**

- Run all exact SQL contracts.
- Run all exact catalog contracts.
- Run all behavioral oracle contracts.
- Run the complete repository validation.
- Run the canonical unlimited-work benchmark.

**TDD red**

No new red test is required in this phase.
Any failed conformance or architecture check reopens its owning phase.

**Implementation**

Correct root causes only.
Do not change expected behavior to match incorrect production output.

**Exit criteria**

- All 108 contracts pass representation and behavior checks.
- The complete relevant suite passes without warnings.
- The canonical benchmark uses the exact required policy suite and corpus.
- The benchmark report separates historical and post-cutover results.
- The report includes statement, `xFilter`, backend, partition, and kernel-dispatch counts.
- The report separates anchor dispatches from packed-program dispatches.

## Required code validation

After the complete gate is green, run:

- The focused 108-case SQL tests.
- The focused 108-case catalog tests.
- The focused 108-case behavioral tests.
- The complete query suite.
- The complete serial native suite.
- Default Clippy with `-D warnings`.
- `code-rule-profile` Clippy with `-D warnings`.
- `cargo check --tests`.
- `cargo fmt --check`.
- `git diff --check`.
- `npm run check`.
- Script tests.
- Documentation tests.
- A full warning and error scan.

The complete validation must contain no warnings.

## Benchmark gate

Run the canonical benchmark only after SQL-only workspace publication works.
Use the exact 12 policies, 829 clauses, 5,725 files, and 95,697,853 bytes.
Keep raw output under `/tmp`.

Record the task count and all execution counters with the timing and memory data.
Explain each internal partition from a recorded hard backend limit.
Do not report a matcher-backed run or a lowering rejection as a post-cutover result.

Current benchmark status: no valid post-cutover result exists.
