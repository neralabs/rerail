# SQL-driven transducer execution

Status: the `rerail_matches` virtual table was deleted on 2026-10-06 (D36 in `docs/design/rerail-query-engine-proposal.md`), with `transducer/sql_backend.rs` and its terminals; policy evaluation runs no SQL. Regex checks are scanned natively (`query::regex_matches`) and AST rules are matched by the grammar matcher (D35). The terminal virtual table, SQL-as-outer-rule-engine and execution-phase sections below are history; the transducer `Program` interpreter is exercised by tests and the benchmark backends only.

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
It returns stable `CandidateCheck` identities, not SQLite identifiers, to the runtime.

`restore_persisted_file()` maps each identity to a compiled `CheckId`.
It replaces `file.candidates.ast` with the stored subset before tape restoration and index construction.
`candidate_dispatch()` then builds these in-memory dispatch lists:

- `Candidates.by_kind` for checks with root kinds.
- `Candidates.unrestricted` for checks without a safe root kind.

`evaluate_ast_file()` builds `FileAnchorIndex` only for the remaining checks.
It creates one `Selection` for each check.
The preorder matcher loop reads `by_kind`, `unrestricted`, and each selection.
Thus, rejected file/check pairs never enter the matcher loop.

## Search index

Schema 12 replaced the file-by-predicate posting tables with contentless FTS5 documents.
The index contents do not depend on the policy catalog.
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
A batch stays in Rust and contains its terminals and file versions.
JavaScript does not receive the batch token, tape identifiers, or SQL connection.

SQLite calls `xFilter` with one batch token and the terminal's `program_index`.
The cursor executes that terminal once over every file of the batch.
A `OnceLock` keeps the terminal rows for later scans of the same batch.
A query that references the relation twice does not execute the terminal twice.
Without a `program_index` constraint, the table fails in production.
Only a test batch of transducer programs reads every file/program pair through `execute_batch()`.

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
  SELECT m.*
  FROM (SELECT DISTINCT program_index FROM rule_programs) p
  JOIN rerail_matches(:batch) m ON m.program_index = p.program_index
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
- Finding templates from relational columns.
- Absence checks after a completion join.
- Incremental invalidation joins by tape and catalog identity.
- Stable output order with explicit logical keys.

SQL scalar functions are suitable for pure operations on one value.
Examples include hash comparison, small posting decoding, collation, and finding formatting.
A scalar function must not run one matcher for each file or rule row.
That design recreates the nested execution loop.

Use virtual tables for batched terminal work and posting expansion.
A future `rerail_postings(batch)` table can expose decoded preorder lists.
A future Metal-backed `rerail_matches(batch)` cursor can select the prepared GPU backend.
The SQL schema and query do not change when the backend changes.

## Execution phases

Each phase uses one bulk operation:

1. SQL kind presence decides which stored tapes need decoding.
2. One GPU dispatch finds exact roots for all file/program pairs.
3. `rerail_matches(batch)` exposes matches and completion rows to SQL.
4. SQL joins terminal rows and creates higher-level findings.

For exact literals, the GPU anchor kernel proves the complete terminal predicate.
The runtime emits terminal rows directly and omits a second terminal dispatch.
Structural programs still need terminal verification.

A dispatch loop is valid only when a hard device limit requires partitioning.
The runtime must record that limit and the number of partitions.
It must not split work to hide contention or setup costs.

## Correctness boundaries

The fixed-word compiler still supports only exact Smart root terminals.
The virtual table can expose all future transducer outputs, but it cannot add missing compiler semantics.
Structural patterns, metavariable equality, ellipses, alternatives, and relations need compiler support first.

SQLite must not infer absence from missing index rows, cancellation, resource exhaustion, or backend failure.
Those states keep `complete=0`.
The final query must preserve capture order and explicit output order.
