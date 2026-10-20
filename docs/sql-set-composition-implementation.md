# SQL composition fixes and compiler spike

The validated SQL access paths and Python candidate guards are now production defaults.
The next compiler experiments remain opt-in in the `unbounded-work` build.
They reduce demand cardinality, but leave a substantial gap from native execution.

## Implemented defaults

Recursive demand and answer joins now start from the input frame with `CROSS JOIN`.
Dependency edges use integer columns and a composite primary key in a `WITHOUT ROWID` table.
`INSERT OR IGNORE` records recursive dependencies without a separate `DISTINCT` temporary.
The destination index supports changed-key-first dirty propagation.
Source deletion uses both `(su, s)` key columns.
The root dependency frame retains its existing semantics.

The shipped B020 rule checks loop-target ancestry before binding `$NAME`.
The shipped B023 rule checks loop and function ancestry before binding `$N`.
Both guards are necessary conditions of their original rules.
Policy identity, revision, messages, and suppression rules remain the same.
A regression test compares the original native findings with both guarded engines on positive Ruff and noqa fixtures.

The answer-difference rewrite remains disabled.
Worker count, automatic indexes, cache sizes, SQLite memory management, and default batch size retain their existing defaults.
The earlier [tuning report](sql-set-composition-tuning.md) records the measurements that selected these changes.

## Prior art from data-layer PR 2539

[PR 2539](https://github.com/neralabs/nera-data-layer/pull/2539) contains a large historical integration stack.
The inspected head is `281f500d7c8d413f17c6a27de2f3c518bb45258a`.
Its relevant components are the declarative fact pipeline, rather than its ServerPlan migration.

The [demand resolver](https://github.com/neralabs/nera-data-layer/blob/281f500d7c8d413f17c6a27de2f3c518bb45258a/scripts/lint_lib/fact_query/demand.py)
propagates relation, field, and physical-dependency demand before extraction.
Missing metadata requests broader extraction.
This provides a useful model for conservative narrowing: retain inputs when the compiler cannot prove a restriction.

The [fact store](https://github.com/neralabs/nera-data-layer/blob/281f500d7c8d413f17c6a27de2f3c518bb45258a/scripts/lint_lib/fact_store.py)
supports direct row-intent batches and batch atom lookup.
Text values become integer atom IDs, and only demanded slots need materialization.
The Rerail prototype applies that principle to existing typed frame inputs.
It does not import the persistent fact schema or the integration branch.
Rerail already has per-worker sessions, prepared-statement reuse, and capture liveness.

## Compiler experiments

Each experiment is independent and disabled by default.

| Environment setting | Behavior |
| --- | --- |
| `RERAIL_SQL_PATTERN_KEYS=integer` | Pass integer input-row handles to bound pattern terminals |
| `RERAIL_SQL_ROOT_GUARDS=1` | Apply proven structural restrictions before root frame construction |
| `RERAIL_SQL_ELIDE_KIND_COPIES=1` | Reuse frames whose known kinds already satisfy a kind test |

### Integer pattern inputs

The original bound pattern path constructs a materialized `(k, CSV environment)` relation.
The terminal parses each environment and joins its answer back through the text key.
The prototype passes input-row integers and a constant descriptor of the source frame and capture columns.
The terminal reads typed capture values directly from the frame store in one batch.
Its materialized result joins directly on the input row key.
This removes SQL environment formatting, CSV parsing, and the extra environment relation.

Single captures, empty lists, nullable captures, sparse row keys, and scan order retain their existing representation.
The source frame stays live through the terminal statement and its register allocation.
The frame borrow ends before each native callback.
Pattern matching remains native and batched, with its existing unification behavior.

These are frame-row handles, not globally interned environment tuples.
Equivalent environments at distinct row keys can still require repeated native matching.
The fact terminal retains its text-key protocol.
The prototype does not establish a speed benefit for integer keys alone.

### Smaller demand sets

The root pass derives necessary ancestor and descendant kind restrictions from the frame IR.
It follows conjunctions, disjunctions, calls, and recursive utilities with a bounded traversal.
An unknown branch in a disjunction prevents narrowing that disjunction.
Cycles and unsupported operators therefore retain broader inputs.
Ignoring a relation's field and stop restrictions also widens the precondition conservatively.
The original operator pipeline still evaluates those restrictions in its original order.

The pass runs only for two-valued statements.
Three-valued statements retain their existing capture-withholding behavior.
It reduces the number of capture-bearing keys demanded, rather than dropping observable capture columns from memo keys.

### Fewer frame copies

The emitter tracks known node kinds through operators that preserve the input node.
It also carries those facts into distinct-key frames.
When the known kinds imply a subsequent kind test, the emitter reuses the input frame.
This avoids a frame allocation and its `INSERT SELECT` copy.
The experiment does not reorder captures or fuse terminal calls into correlated SQL.
It currently runs only for two-valued statements.

## Measurements

The workload contains the same 135 supported Python policies, 146 clauses, and 32 pinned data-layer files.
It includes the two newly shipped rule guards.
All variants use batch size 2, four workers, the same binary, and fresh processes.
Three repetitions alternate variant order.
CPU time is process CPU, summed across threads.
Wall time remains sensitive to contention on the shared host.

The initial complete sweep gives these median apply CPU times:

| Variant | CPU seconds | Change from control |
| --- | ---: | ---: |
| Promoted defaults | 8.116 | — |
| Integer pattern keys | 8.266 | 1.8% slower |
| Root guards | 7.342 | 9.5% less CPU |
| Kind-copy elimination | 8.037 | 1.0% less CPU |
| All three | 7.246 | 10.7% less CPU |

All fifteen runs return the same 33 full finding objects, zero failures, and zero blind files.
The combined positive-fixture run preserves all 31 finding objects, including captures and suppression state.

A separate 138-file scan contains 106 files outside the tuning subset:

| Variant | CPU seconds | Wall seconds |
| --- | ---: | ---: |
| Native | 7.772 | 4.268 |
| Promoted SQL defaults | 46.596 | 29.548 |
| Combined prototype | 41.389 | 22.702 |

All three runs preserve the same 580 full finding objects with zero failures and zero blind files.
The prototype saves 11.2% SQL CPU in this single-run validation.
Its remaining CPU ratio to native is 5.33×.
These measurements use the same diagnostic allocator shim and release settings as the earlier investigation.
They do not reproduce Tim's missing external 818-clause catalog.

The demand trace explains the smaller gain:

| Counter | Promoted defaults | Combined prototype | Reduction |
| --- | ---: | ---: | ---: |
| Inserted demand keys | 708,699 | 465,150 | 34.4% |
| Inserted keys carrying captures | 253,104 | 156,872 | 38.0% |
| VM steps in statements referencing demand tables | 83,171,409 | 56,461,181 | 32.1% |

These are demand-related counters, not whole-workspace VM totals.
The complete screen reduces affected-row counts from 10,139,995 to 7,828,375.
Frame input work and root checks offset part of that reduction.
Integer-key and kind-copy results alone do not support a throughput claim.

The final three-round repeat after the frame-borrow cleanup gives 8.153 seconds for the control.
Root guards take 7.591 seconds, and the combined prototype takes 7.277 seconds.
The combined CPU reduction remains 10.7%.
All nine runs preserve the same complete output.

## Validation

The benchmark build passes 108 SQL tests with the default SQL and each spike configuration.
Nine tests remain ignored.
The suite includes 1,239 generated cases, replay cases, query-plan checks, and positive fixture comparisons.
The statement-count test enforces its original work ceiling while permitting fewer statements after copy elimination.
The production build also passes `cargo check --release --lib` without `unbounded-work`.

## Reproduction and remaining work

The [tuning harness](../benchmark/sql-set-tuning.mjs) records source hashes, addon hashes, policy hashes, timing, memory, and complete engine reports.
The [shape aggregator](../benchmark/sql-set-shapes.mjs) retains exact SQL and query plans.
The artifacts are under `/tmp/sql-set-spike-20261020` on the investigation host.

`confirmation.json` contains the complete three-round sweep.
`heldout.json` contains the 138-file validation.
`shapes-control.json` and `shapes-combined.json` contain the demand traces.
`final-confirmation.json` repeats the control, root-guard, and combined variants after the final frame-borrow cleanup.

The remaining work has a clearer order.
First, expand conservative demand restrictions with broader positive and three-valued coverage.
Then measure typed tuple interning that preserves sharing between equivalent environments at different row keys.
Extend integer keys to fact terminals only after isolating their materialization and lookup costs.
Finally, consider broader frame fusion where it removes memory traffic without repeating terminal evaluation.
The current results support further work on demand cardinality more strongly than the other two experiments.
