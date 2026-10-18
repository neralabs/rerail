# SQL-driven transducer execution

Status: production has one engine shape. Compiled clauses lower into relational SQL plus verified terminal programs. The workspace prepares one immutable sparse batch. SQLite demands it through the terminal virtual table. The first `xFilter` executes it once. A materialized SQL relation evaluates clauses and extracts ordered violations. The workspace publishes only those rows. The semantic matcher is a test oracle only. Unsupported lowering, workspace tape preparation, and result publication currently fail at their typed stage. Production never falls back.

[`sql-relational-compiler-design.md`](sql-relational-compiler-design.md) defines the compiler boundary and the packed terminal registry.

## Design decision

SQLite drives all production rule execution.
The host registers an inert batch descriptor and executes one SQL statement.
The host must not execute terminals, relations, or semantic producers before that statement.

The SQL statement reads one eponymous virtual table.
The first `xFilter` call executes the complete sparse batch on demand.
That call materializes ordered match, capture, and completion rows in the batch.
The cursor then exposes those rows to SQLite.
SQLite extracts violations from the relation in the same statement.

The required boundary is:

```text
relational SQL plan + workspace files + terminal programs + sparse task grid
  -> register inert batch descriptor
  -> execute one SQL statement
    -> rerail_matches(:batch) xFilter
      -> execute every demanded file/program task as one batch
      -> materialize typed terminal, capture, fact, and completion rows once
    -> SQLite evaluates relations, joins, counts, and clause outcomes
    -> SQLite extracts ordered violations
  -> publish the final SQL rows
```

This design is not a pre-execution pipeline.
Rust does no rule work before SQLite asks for the virtual table.
SQLite owns demand, execution lifetime, relational composition, and result extraction.

## Production engine

Rerail always uses SQLite in production. There is no runtime engine selector. The workspace memory profile reports `"executionEngine":"sqlite"`. An unsupported packed clause reports an `adapter-error`; Rerail does not retry it with the matcher.

## Current file filter

Rerail uses persisted, conservative source facts for file filtering.
These facts are four-lane bit sets, not exact value postings.

`source_anchor_facts` stores these facts for each source and grammar:

- Kinds that occur in the source.
- Small exact terminal values that occur in the source.
- Required text trigrams that occur in the source.

`candidate_anchor_facts` stores the required facts for each check and grammar.
It also stores possible root kinds.
`AnchorPlan::candidate_facts()` creates these values from the compiled matcher.

The publication SQL compares the source and check lanes with bit operations.
The SQL inserts each possible pair into `workspace_file_checks`.
Clean snapshots use `snapshot_file_checks`.
These tables contain exact materialized file/check eligibility after conservative filtering.

The physical indexes are:

- The primary key `(workspace_id, check_id, path_id)` on `workspace_file_checks`.
- `workspace_file_checks_by_file(workspace_id, path_id, check_id)`.
- The equivalent primary and secondary indexes on `snapshot_file_checks`.
- The primary key `(source_id, grammar_id)` on `source_anchor_facts`.
- The primary key `(check_id, grammar_id)` on `candidate_anchor_facts`.
- Catalog and completion-marker indexes for bulk manifest reads.

The completion rows in `workspace_candidate_files` and `snapshot_candidate_files` gate visibility.
An incomplete publication cannot prove that a file/check pair is absent.

## Path from SQL to the matcher

The runtime reads the complete candidate manifest before worker evaluation.
The read uses one joined SQL statement for the requested catalog.
It returns stable `CandidateClause` identities, not SQLite identifiers, to the runtime.

`restore_persisted_file()` maps each identity to a compiled `ClauseId`.
It replaces `file.candidates.ast` with the stored subset before tape restoration and index construction.
`candidate_dispatch()` then builds these in-memory dispatch lists:

- `Candidates.by_kind` for clauses with root kinds.
- `Candidates.unrestricted` for clauses without a safe root kind.

`evaluate_ast_file()` builds `FileAnchorIndex` only for the remaining clauses.
It creates one `Selection` for each clause.
The preorder matcher loop reads `by_kind`, `unrestricted`, and each selection.
Thus, rejected file/clause pairs never enter the matcher loop.

## Search index

Schema 12 replaced the file-by-predicate posting tables with contentless FTS5 documents.
The index contents do not depend on the code-rule catalog.
Only the kind family remains; the exact-value and source-word families were deleted with their readers.

`search_kind_fts` stores one kind token for each preorder node.
It answers file-level kind presence for a stored tape (`get_search_kind_presence_bulk`).

`search_index_sets` records the tape, index version, node count, completion state, and syntax errors.
A complete, error-free index without any requested kind proves that the kinds are absent from the tape.
A missing, stale, or syntax-error index set means unknown presence.
Unknown presence decodes the tape and evaluates it normally.

The store maintenance thread publishes kind documents in batches of at most 64 tapes.
The transaction deletes and inserts by immutable tape identity.
The tape delete trigger removes the FTS document with the tape.

## Terminal virtual table

`rerail_matches(batch)` is an eponymous-only SQLite virtual table.
The hidden `batch` column is required by `xBestIndex`.
A batch stays in Rust and contains verified programs, file tapes, limits, and a sparse task grid.
JavaScript does not receive the batch token, tape identifiers, or SQL connection.

Batch registration stores data only.
It does not run anchor selection, a matcher, a relation, or a semantic producer.
The sparse task grid identifies the valid file/program pairs and their selected roots.

The target `xBestIndex` contract requires one equality constraint on `batch`.
A `program_index` constraint must not divide execution into one call per program.
A file constraint must not divide execution into one call per file.
The batch executor can use both constraints as filters only when one execution still owns the full demanded task set.

SQLite calls `xFilter` when the SQL plan first reads `rerail_matches(:batch)`.
The cursor calls the selected packed backend once for the sparse batch.
The backend executes terminal and fact-producing programs for the SQL plan.
A `OnceLock` stores the normalized output relation and its failure state.
A later scan of the same batch reads the stored relation and does not execute it again.

The SQL query must use one materialized CTE for this relation.
This rule prevents SQLite aliases or nested loops from opening independent executions:

```sql
WITH terminal AS MATERIALIZED (
  SELECT *
  FROM rerail_matches(:batch)
)
SELECT ...
FROM terminal;
```

The cursor performs result extraction from the stored typed rows.
It does not call a matcher from `xNext` or `xColumn`.

The relation contains these columns:

```text
batch
file_index
program_index
match_index
start_ordinal
output_label
capture_index
capture_label
capture_node
start_byte
end_byte
complete
error_code
```

A complete negative pair has one status row with a null `match_index`.
A failed pair has `complete=0` and an error code.
SQL must inspect completion before it uses absence.

## SQL as the outer rule engine

A query can join terminal rows with rule metadata and application facts:

```sql
WITH terminal AS MATERIALIZED (
  SELECT * FROM rerail_matches(:batch)
), complete_pairs AS (
  SELECT file_index, program_index,
         min(complete) AS complete
  FROM terminal
  GROUP BY file_index, program_index
), matches AS (
  SELECT *
  FROM terminal
  WHERE complete = 1 AND match_index IS NOT NULL
)
SELECT r.rule_id, m.file_index, count(DISTINCT m.match_index) AS match_count
FROM rule_programs r
JOIN complete_pairs c ON c.program_index = r.program_index AND c.complete = 1
JOIN matches m ON m.program_index = r.program_index
GROUP BY r.rule_id, m.file_index
HAVING count(DISTINCT m.match_index) >= r.minimum;
```

SQL can then implement these higher-level operations:

- Joins between captures from different terminal programs.
- Counts, ranges, grouping, and threshold expressions.
- Cross-file relations.
- Rule alternatives with explicit priority columns.
- Violation templates from relational columns.
- Absence checks after a completion join.
- Incremental invalidation joins by tape and catalog identity.
- Stable output order with explicit logical keys.

SQLite built-in expressions can perform pure operations on one value.
Examples include comparison, collation, and text formatting.
The production statement must not call a custom scalar function.

`rerail_nodes`, `rerail_facts`, and the `tq_*` callback family are not production execution boundaries.
They remain deleted historical machinery.
Relational SQL replaces their composition logic, and packed programs replace their terminal and fact callbacks.

Use the virtual table for batched relation execution and typed output extraction.
The virtual table selects the CPU, CPU-JIT, Metal, or Metal-JIT backend inside `xFilter`.
The SQL schema and outer query do not change when the backend changes.

## Execution phases

The host performs only setup and publication:

1. The host registers files, verified programs, roots, limits, and the sparse task grid.
2. The host starts one SQL statement with the batch token.
3. SQLite opens `rerail_matches(:batch)` when the query needs terminal rows.
4. `xFilter` executes and materializes the sparse batch once.
5. SQLite composes rules and extracts ordered violations from the virtual-table rows.
6. The host publishes the rows that the SQL statement returns.

The virtual table can partition an internal dispatch only for a hard backend limit.
The partition does not create another SQL statement or another virtual-table execution.
The batch records the limit and the partition count.

## Execution ownership

The SQL statement owns these actions:

- Demand for the terminal relation.
- The first and only batch execution.
- Completion and absence checks.
- Cross-program and cross-file joins.
- Counts, thresholds, alternatives, and stable result order.
- Final violation extraction.

The packed backend owns these actions inside `xFilter`:

- Terminal-program execution.
- Ordered pattern control flow.
- Pattern capture state and rollback.
- Typed fact production declared by verified programs.
- Per-task limits and completion status.
- Materialization of typed output rows.

The host owns registration, cancellation, publication, and batch release.
It does not schedule clauses or file groups.

## Prohibited shapes

The production path must not contain these shapes:

- A host loop that calls one SQL statement for each clause.
- A host loop that calls one SQL statement for each compatibility group.
- A virtual-table scan for each file, root, program, or outer SQL row.
- A scalar callback that executes a matcher or relation.
- A scalar callback that enters SQLite recursively.
- A pre-execution stage that materializes match or fact relations before SQL asks for them.
- A fallback from an unsupported packed operation to the callback engine.

## Cutover sequence

1. Restore the all-program production access path in `rerail_matches(batch)`.
2. Make one `OnceLock` own all rows and status for the sparse batch.
3. Route one SQL statement through one materialized terminal CTE.
4. Lower complete Rerail clauses into callback-free relational SQL plans.
5. Generate verified terminal and fact programs that those plans reference.
6. Move `violation.each` alternatives into one ordered batch output.
7. Remove clause scheduling from `workspace/evaluate.rs`.
8. Reject any production plan that references `tq_*`, `rerail_nodes`, or `rerail_facts`.

Each step must preserve matcher parity.
The cutover is complete only when a full workspace apply causes one virtual-table execution per backend batch.

## Batch descriptor

The batch descriptor is immutable after registration.
It contains these values:

- The catalog generation and grammar identities.
- Verified packed programs.
- Source and tape handles for all files.
- A sparse file/program task grid.
- Ordered candidate roots for each task.
- Per-file and per-task budgets and deadlines.
- The selected backend and its hard dispatch limits.
- One lazy output cell for rows or a batch failure.

The sparse grid replaces compatibility-group scheduling.
A missing pair means that SQL eligibility rejected that task before registration.
A present pair retains its own limits, selected roots, and deterministic order key.

Workspace preparation can parse, restore, and verify immutable inputs.
Registration stores the completed immutable descriptor only.
Registration must not evaluate a rule or produce a match, capture, fact, or violation.

## Virtual-table state

A registered batch has these states:

```text
registered -> executing -> materialized
                       \-> failed
registered -----------------> released
materialized ---------------> released
failed ---------------------> released
```

Only `xFilter` can change `registered` to `executing`.
One lazy cell serializes concurrent first access.
The executor does not hold the registry mutex while it runs.

Cancellation produces incomplete status rows for affected tasks.
A panic or backend error produces a stored batch failure.
A later scan observes the same rows or failure.
It never starts a second execution.

The cursor owns an `Arc` to the materialized rows.
Batch release removes the registry entry after the SQL statement closes its cursors.

## Row and order contract

The virtual table exposes one 29-column tagged relation.
Its row kinds are batch status, producer status, terminal match, capture, navigation, fact, and document map.
[`sql-relational-compiler-design.md`](sql-relational-compiler-design.md) defines these logical row families.
The current executor emits status, terminal-match, capture, and navigation rows.
One navigation producer emits typed parent-child edges with grammar field and named-node data.
SQL derives navigation semantics from those edges.
Later producers will emit fact and document-map rows.
This change does not require a new persistent storage schema.

`(producer_id, file_index, task_index)` identifies one demanded terminal task.
`match_index` orders matches within that task.
Separate capture rows carry capture slot, occurrence, and sequence ordinals.
`output_label` identifies the program output path.

Every demanded task emits one producer-status row.
A complete negative task has no terminal-match row and has `complete=1` in its status row.
An incomplete task has `complete=0` and an `error_code`.
SQL cannot infer absence without a complete status row.

The final SQL order uses explicit catalog, file, program, match, output, and capture keys.
Backend completion order cannot affect the final order.

## Validation gates

The cutover must meet these execution gates:

- One outer SQL statement drives one workspace evaluation phase.
- Each backend batch records exactly one virtual-table execution.
- A second scan of the materialized CTE records no execution.
- Production records zero `tq_*` matcher callbacks.
- Production records zero `rerail_nodes` relation walks.
- Production records zero lazy `rerail_facts` builds.
- Registration records zero match, capture, fact, and violation rows.
- Every sparse task records complete output or an explicit incomplete status.
- The matcher and SQLite paths return identical ordered results and failures.

The benchmark must report virtual-table filters, batch executions, tasks, outputs, completion rows, partitions, time, and peak memory.

## Correctness boundaries

The fixed-word compiler still supports only exact Smart root terminals.
The virtual table can expose future terminal outputs, but it cannot add missing compiler semantics.
Structural patterns, metavariable equality, and ellipses need terminal-compiler support.
Alternatives and relations need relational SQL lowering plus their required terminal programs.

SQLite must not infer absence from missing index rows, cancellation, resource exhaustion, or backend failure.
Those states keep `complete=0`.
The final query must preserve capture order and explicit output order.
