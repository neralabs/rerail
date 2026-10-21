# SQL work amplification spikes

These experiments extend [the SQL generation spikes](sql-generation-shape-spikes.md).
They remain test-only and use one statement, or four bounded phases, across each admitted batch.
The production addon keeps its existing compiler path.

## Scope and decisions

Two workloads answer different questions:

- The real policy subset contains 55 admitted Python matchers from the 141-matcher catalog, across 32 or 138 files.
- A generated positive recursive rule searches the same corpus for the first `self` or `data` identifier inside each function.

The generated rule tests a compiler transformation with a known semantic contract.
Its results do not establish performance for the complete recursive policy catalog or Tim's original 818 clauses.
Every completed candidate compares matching roots and capture occurrences against the existing SQL engine.
Differential fixtures also compare the relevant rule semantics with native evaluation.

The useful results are constant capture-column removal and specialized witness relations.
Filter reordering, broad leaf batching, and generic JSON state changes do not provide a general solution.
The decisive improvement comes from changing the intermediate relation's grain before joining.

## Remove capture columns before their first binding

The frame emitter keeps every capture that a later operator can read or bind.
It does not distinguish a future capture's known `NULL` from a materialized input column.
The prototype propagates constants through recognized SELECT projections and omits those columns from intermediate schemas.
A later binding introduces the column when it gets a value.

The pass preserves output schemas and native terminal input schemas.
It preserves implicit SELECT column names and outer-join null extension.
It refuses ambiguous aliases, unsupported unions, stars, and unrecognized shapes.
Physical buffer names include a schema fingerprint, so different experimental layouts cannot reuse an incompatible registered table.

The real subset removes 439 capture columns across 91 relation schemas in the initial measurement.
These are schema positions, not 439 AST nodes or result captures.
The pass does not remove candidate rows or change witness order.
Its main effect is narrower copies and narrower SQLite intermediates.

The table reports medians from three ordinary-build runs.
Execution CPU excludes SQL generation, preparation, and diagnostic work.
Kernel CPU includes connection setup, preparation, execution, and result decoding.
Generation CPU remains separate.

| Files | Plan | Statements | VM steps | Published cells | Execution CPU | Kernel CPU | Generation CPU |
|---|---|---:|---:|---:|---:|---:|---:|
| 32 | Baseline, budget 16 | 1 | 44,302,447 | 1,103,356 | 920 ms | 1,042 ms | 61 ms |
| 32 | Narrow, budget 16 | 1 | 41,957,124 | 528,881 | 832 ms | 943 ms | 83 ms |
| 32 | Narrow, four phases | 4 | 43,630,066 | 251,474 | 823 ms | 944 ms | 98 ms |
| 138 | Baseline, budget 16 | 1 | 238,331,054 | 5,817,776 | 5,602 ms | 5,821 ms | 60 ms |
| 138 | Narrow, budget 16 | 1 | 226,107,222 | 2,808,255 | 5,316 ms | 5,505 ms | 81 ms |
| 138 | Narrow, four phases | 4 | 233,900,856 | 1,341,079 | 5,250 ms | 5,453 ms | 100 ms |

The budget-16 projection reduces VM work by 5.3 percent on 32 files and 5.1 percent on 138 files.
It reduces native publication cells by about 52 percent on both sizes.
Execution CPU decreases by about 10 percent and 5 percent respectively in this sweep.
All plans preserve 26 matching roots on 32 files and 449 on 138 files, including every capture.

Additional single-case processes reverse baseline and narrowed-case order.
They retain complete parity and the same VM counts, while showing timing variation.
These are modest improvements within the matching subset.

Budget 16 with one statement is the useful initial choice after narrowing.
Four phases give a small execution difference on the larger sample and add generation and preparation cost.
The physical planner can reserve those boundaries for preparation limits or measured larger gains.

This lexical pass establishes the value of capture liveness that tracks constants.
Production integration should represent constants and binding state in the compiler IR.
Repeated SQL parsing is unnecessary when the generator already knows each expression.
Compilation cost and prepared-program reuse remain separate from execution cost.

## Predicate movement needs physical cost evidence

The experimental logical pass moves total kind and regex guards before total pattern matches.
It preserves alternative order and stops at facts, recursive calls, and unknown-producing operations.
It also preserves capture withholding metadata.
This makes the semantic transformation conservative, but does not guarantee a better physical plan.

These guard comparisons use the scanstatus build.
On the 32-file budget-16 plan, guard movement increases VM work from 45.23 million to 47.14 million steps.
Automatic-index construction increases from 372,608 to 420,581 rows.
The hottest changed stage binds the pattern through an `sf_src1` wrapper, then rejoins the input to restore its unknown flag.
That indexed join offsets the smaller predicate input.
The prototype therefore rejects unconditional guard movement.

A typed totality analysis could avoid the redundant unknown-state join.
A generator with cardinality estimates could also choose a different materialization boundary.
Those are different changes from unconditional filter reordering.

## Narrow recursive leaf demands

The recursive machine initially batches every leaf across every node in the corpus.
A finite kind analysis now follows instruction edges, conjunction prefixes, relation candidates, and recursive unit references.
Kind domains only widen at joins, so recursion reaches a conservative fixed point.
A missing kind constraint allows all kinds.

A second candidate lowers a Smart root capture directly into SQL node eligibility and capture assignment.
It retains file dialect checks and the existing structural unification SQL for bound inputs.
A third candidate uses per-predicate lookups instead of one combined leaf relation and its large automatic index.
These candidates preserve the tested counts, unknown verdicts, captures, and reached-cycle refusals.

These changes reduce leaf work but leave the generic instruction machine in place.
The real 32-clause recursive batch still exceeds its execution guard.
The report records an incomplete candidate instead of claiming parity or treating it as no matches.

## Share recursive answers across roots

The original machine gives each root a separate memo object.
A candidate executes several roots in one rule/file lane and retains completed unit answers between them.
The full unit key still contains the node and capture inputs.
A root with a different capture environment cannot reuse an incompatible answer.

Separate candidates use binary JSON for environments, stacks, and memo values.
Externally visible captures and memo property names remain canonical JSON text.
This distinction preserves cycle comparisons and avoids concatenating binary JSON into a property path.
Tests include unknown counts, withholding, composite captures, comments, multiple files, and different file dialects.

A fixture drops from 8,594 operator transitions to 3,172 with shared roots.
The real recursive catalog does not get the same reduction and still reaches the execution guard.
Grouping unrelated capture-bearing demands adds ranking and state-management work.
JSONB also does not remove instruction dispatch, memo lookup, or state transfer.
These candidates are not selected as a general performance implementation.

These ordinary-build runs execute the complete 32-clause recursive batch until the VM guard.
They do not complete, so their CPU values are time to interruption, not workload runtimes.

| Plan | CPU to interruption | VM steps | Status |
|---|---:|---:|---|
| Original leaf sets | 8.44 s | 500,020,000 | VM budget, parity unassessed |
| Narrowed, direct captures, per-predicate sets | 7.75 s | 500,020,020 | VM budget, parity unassessed |
| Per-predicate sets, 64-root sharing, JSONB | 8.37 s | 500,020,000 | VM budget, parity unassessed |

Group sizes 8, 64, and 256 also fail to complete the profiled catalog batch.
The one-file 32-clause smoke workload and the differential fixtures complete with parity.
These results reject a blanket replacement of generic state representation as the way to close the recursive catalog gap.

The next memo analysis must distinguish demanded input values from returned capture occurrences.
It must also preserve which captures the selected branch actually overwrites.
Dropping capture keys without that proof would change unification and returned occurrences.

## Compile positive recursion into witness segments

The specialized compiler recognizes one positive recursive unit with this shape:

```json
{
  "rule": {"kind": "function_definition", "matches": "find"},
  "utils": {
    "find": {
      "any": [
        {"kind": "identifier", "regex": "^(self|data)$", "pattern": "$X"},
        {"has": {"matches": "find"}}
      ]
    }
  }
}
```

The base test is total, the input capture is fresh, and recursion walks unrestricted children.
For this contract, the first successful witness is the smallest matching preorder node ID in the root's subtree.
The compiler rejects other recursive shapes explicitly.

Four generated SQL shapes implement that contract:

| Shape | Intermediate work |
|---|---|
| Recursive closure | Propagate each witness to its ancestors, then group by root. |
| Interval join | Join roots to every matching witness in their subtree, then select `MIN`. |
| Correlated probe | For each root, order matching witnesses and select the first. |
| Witness segments | Partition the ID space by consecutive witnesses, then join each root to at most one witness. |

For sorted witnesses `w`, the previous witness partitions possible root starts:

```sql
segments(file, witness, previous) AS MATERIALIZED (
  SELECT file, node,
         LAG(node, 1, -1) OVER (PARTITION BY file ORDER BY node)
  FROM seed
),
accepted(file, node, witness) AS MATERIALIZED (
  SELECT n.file, n.id, s.witness
  FROM segments s
  CROSS JOIN node n
    ON n.file = s.file
   AND n.id > s.previous
   AND n.id <= s.witness
  WHERE s.witness <= n."last"
    AND n.kind = :function_kind
)
```

A root belongs to at most one segment.
The subtree-end test excludes a witness beyond that root's subtree.
The query therefore avoids generating later witnesses and does not need a final root/witness product followed by `MIN`.
The node table receives exact file, kind, and ID-range constraints.
The regex batch also receives the base predicate's required kind domain.

The following values are median statement execution CPU from three ordinary-build runs.
Each query uses one SQL execution and preserves all matching roots and capture occurrences.

| Files | Shape | VM steps | Execution CPU |
|---|---|---:|---:|
| 32 | Recursive closure | 3,846,676 | 46.0 ms |
| 32 | Interval join | 17,259,379 | 88.3 ms |
| 32 | Correlated probe | 17,184,118 | 86.2 ms |
| 32 | Witness segments | 1,634,901 | 17.4 ms |
| 138 | Recursive closure | 17,608,985 | 234.2 ms |
| 138 | Interval join | 47,783,391 | 263.9 ms |
| 138 | Correlated probe | 47,450,517 | 262.5 ms |
| 138 | Witness segments | 8,121,348 | 93.1 ms |

The segment plan uses 10.6 times fewer VM steps than the interval join on 32 files.
It uses 5.9 times fewer on 138 files.
All shapes return the same 1,478 and 5,631 matches respectively.
Each shape prepares in less than one millisecond in the ordinary build.

Fresh single-case process controls put segments first.
They measure 20.6 ms versus 95.6 ms for the interval join on 32 files.
The corresponding 138-file values are 113.5 ms and 294.1 ms.
This checks that the advantage survives the sweep's fixed case order.

These measurements exclude connection setup, batch construction, SQL generation, and rendering.
They are not complete-call latency or a native-engine speed comparison.
The segment plan is selected for this recognized workload, subject to its semantic contract.

The compiler derives these queries from logical operators.
It currently supports one node capture, one unit, a total base, and unrestricted child recursion.
It does not cover arbitrary stop rules, negative recursive dependencies, parameterized calls, or capture-dependent facts.
Native dependency-depth refusals still need integration before production use.

## Compiler direction

The experiments support a compiler that carries four properties into SQL generation:

1. Known constants and capture binding state, so capture columns appear only when needed.
2. Relation grain and uniqueness, so the generator collapses witnesses before joining upward.
3. Totality and unknown-state provenance, so pure operations avoid unnecessary restoration joins.
4. Unit input/output contracts, so recursive results can share safely across compatible demands.

A CTE materialization budget controls preparation expansion.
It does not replace those semantic properties.
A generic recursive operator interpreter is useful for feasibility tests, but is not the desired shape for common rule components.

## Reproduction and artifacts

The existing ignored workload harness selects these acyclic comparisons:

```sh
RERAIL_WHOLE_CALL_CONFIG=/absolute/path/config.json \
RERAIL_WHOLE_CALL_OUTPUT=/absolute/path/output \
RERAIL_WHOLE_CALL_CASES=shaped-16-1-All,narrow-16-1-All,narrow-64-4-All \
cargo test --profile loop --features unbounded-work \
  --manifest-path native/Cargo.toml whole_call_corpus -- --ignored --nocapture
```

`RERAIL_WHOLE_CALL_GUARDS=1` selects the guard comparison in tests.
For recursive catalog tests, the case is `recursive-machine`.
`RERAIL_WHOLE_CALL_LEAVES` selects `baseline`, `kinds`, `capture`, or `partitioned`.
`RERAIL_WHOLE_CALL_MEMO` selects `shared`, `binary`, or `shared-binary`.
`RERAIL_WHOLE_CALL_ROOT_GROUP` sets the experimental group size.
These controls compile only into tests and do not add production execution modes.

The generated positive rule uses the ignored `recursive_amplification_corpus` test.
`RERAIL_RECURSIVE_CASES` can restrict its comparisons to named machine or relational shapes.
`RERAIL_WHOLE_CALL_PROFILE=1` adds per-statement EXPLAIN, stage diagnostics, and available loop counters.
Native loop counters require `LIBSQLITE3_FLAGS=-DSQLITE_ENABLE_STMT_SCANSTATUS` at build time.
Runtime statement counts exclude diagnostic EXPLAIN executions and include nested callback executions.

The execution guard distinguishes VM exhaustion from elapsed-time exhaustion.
Per-root transition exhaustion is also an explicit incomplete status.
Failed candidates retain available partial counters and plans.

Local artifacts are under `/tmp/rerail-amplification-20261021`.
They contain frozen binaries, source hashes, SQL, coverage, complete reports, rejected candidates, and repeated timings.
The local macOS build also uses its existing `JEMALLOC_OVERRIDE` shim.

## Validation

The final ordinary test binary passes all 189 SQL tests.
The suite includes constant projections, nullable outer joins, implicit column names, and changed schemas on reused connections.
It also checks recursive counts, unknowns, withholding, composite captures, comments, cycle refusals, and mixed file dialects.
Release Clippy passes for both the production library and the profiling test build.
The existing profiling and SQL-shape script suites pass all 12 tests.

Formatting, boundary checks, Windows path checks, and whitespace checks pass.
The structure checker retains its two existing failure categories for fixture placement and cross-root fixture includes.
This work adds no new structure findings.
