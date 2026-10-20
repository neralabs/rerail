# Shared SQL execution for data-layer rules

## Implementation

Keep SQLite as the relational execution engine.
Compile compatible detectors into one authored relational clause.
Load its source inputs once, then create the shared temporary tables and indexes once.
Run bounded diagnostic SELECTs against those tables and combine their outputs with `UNION ALL`.

The removed Rust spike used a stage-name admission rule, content fingerprints, and a retained-table cache.
That implementation and its profiling switch are removed.
The replacement has no runtime artifact cache, content hashing, table renaming, or special treatment of a stage name.

The existing SQL executor already implements each declared stage with `CREATE TEMP TABLE ... AS SELECT ...`.
It creates the declared indexes and collects SQLite statistics.
The authoring change uses that existing execution scope for multiple consumers.
SQLite owns the intermediate tables, indexes, recursive queries, joins, and result queries.
The connection closes after the clause completes.

## SQL grouping

The original quality generator produced nine contract clauses because of stage-count and statement-size limits.
The new generator produces three compatible groups.
Each group shares source extraction, input insertion, temporary SQL tables, and indexes.

Primary and fallback detectors share one group.
Their production SELECTs already apply their respective `primary_file` and `fallback_file` conditions.
The shared AST input reads the union of those file domains.
Callback and inventory detectors retain separate source-trigger gates.
Project taxonomy and cross-file conditional rules retain their separate execution scopes.

Diagnostic SELECTs remain in packets of at most approximately 12 KB before option expansion.
Each packet becomes a SQL stage. The final query combines these stages.
Each statement still obeys the existing 16 KB authored SQL limit.
The native and TypeScript schemas now permit 64 stages instead of 32.
The largest quality clause uses 59 stages after typed-input partitioning.
Input relation count, witness limits, VM work limits, and cancellation remain enforced by the existing evaluator.

The generator uses declared detector dependencies and source groups.
It does not discover equivalent runtime tables or maintain an invalidation protocol.
The existing final-result cache still handles unchanged files and edits.
Regrouping changes clause IDs and finding IDs. Diagnostic parity compares path, line, code, and multiplicity.
Workspace-versus-stateless checks also compare the findings produced by the same generated policy.
A shared clause has the existing clause-level failure boundary: a failed shared prerequisite fails that clause.

## Materialized CTEs and indexed tables

SQLite can share a producer within one statement through `AS MATERIALIZED`.
A direct SQLite test checks that two SELECT consumers execute that producer once.
It also checks that `EXPLAIN QUERY PLAN` reports its materialization.

This implementation uses shared temporary tables because the existing stages declare explicit indexes.
The recursive descendant relation and its indexes now have multiple consumers in the same SQL scope.
There is no claim that temporary tables outperform materialized CTEs for every query.
The selected implementation preserves the existing indexed-stage representation and requires no new executor.

## Reproduction

Generate the shared SQL policies from the pinned data-layer source:

```sh
/path/to/data-layer/.venv/bin/python scripts/dsl/data_layer_generate_batch_six.py \
  --source /path/to/pinned/nera-data-layer --repo /path/to/data-layer/repo \
  --output /tmp/shared-sql-policies
```

Add `--independent-sql` to generate the previous independent SQL groups.
That mode reproduces the previous checked-in quality policy byte for byte.
Other sixth-batch policies remain identical.

Use the bounded profiling controller with either policy directory:

```sh
npm run profile:code-rule-dsl -- \
  --source /path/to/pinned/nera-data-layer \
  --selection fixtures/host/data-layer-port/profiling/quality-five.json \
  --policies /tmp/shared-sql-policies --query-plans \
  --output .rerail/profiles/shared-sql
```

`--query-plans` records SQLite plans in `sql-plan` events, including the SQL text and selected indexes.
The comparison tool rejects different policy digests by default.
For this SQL rewrite, pass `--allow-policy-changes`; its report lists every changed policy digest.
Workload identity, native binary, allocator, measured phases, profiling modes, and passing parity must still match.

## Validation and measurements

The [measurement evidence](../fixtures/host/data-layer-port/profiling/shared-sql-evidence.json) contains both matched comparisons.
Both comparisons use five files, 1,026,452 source bytes, and the same native binary.

| Measurement | Independent SQL | Shared SQL |
| --- | ---: | ---: |
| Cold wall, tracing off | 53.178 s | 18.308 s |
| Cold process CPU, tracing off | 46.707 s | 14.370 s |
| Edited-file wall, tracing off | 14.784 s | 4.086 s |
| Unchanged-file wall, tracing off | 9.636 ms | 10.143 ms |
| Peak RSS, tracing off | 1.346 GB | 1.334 GB |
| Cold wall, plans and sampling enabled | 59.311 s | 18.805 s |
| File-local descendant builds | 30 | 5 |
| SQL stage builds | 890 | 443 |
| SQL index builds | 1,362 | 602 |
| AST input extractions | 1,155 | 355 |

The tracing-disabled pair measured 2.90 times faster cold evaluation and 69.2% less cold process CPU.
Edited-file evaluation was 3.62 times faster. Unchanged-file latency did not improve.
The second pair measured 3.15 times faster cold evaluation with query plans and three external samples per run.
These are two matched pairs on a shared host, not a production guarantee.

The descendant query plans retain the same recursive SQL and indexed edge lookup.
The improvement comes from executing fewer copies of that work.
External baseline samples spend much of their time in SQLite record comparison and B-tree navigation.
Shared-SQL samples expose more input extraction and trace-writing work after duplicate SQL work disappears.
The tracing-disabled pair checks the result without detailed trace-writing overhead.

All 200 recorded quality cases pass: 135 positive cases and 65 clean cases.
There are no false positives, missed findings, engine failures, or workspace/stateless differences.
All 40 resident-workspace transitions across the ten sixth-batch rules pass.
The 226 runtime tests and eleven Python profiling/SQL tests also pass.
Raw profiles, query plans, external samples, and corpus reports remain in the local profiling artifact directory.
All commands use the profiling controller deadline, which cannot exceed 115 seconds.

Further work must follow SQL evidence: query plans, unnecessary input columns, recursive closure size, and actual input insertion cost.
These changes do not add parallel relational execution or claim complete batching.

The [follow-up profile](shared-sql-performance.md) measures the next costs without changing this implementation.
