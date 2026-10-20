# SQL compiler spikes: results and next steps

Commit `c034acd3` enables the previously measured optimization combination whenever the caller selects the SQL engine.
Typed pattern inputs, root guards, kind-copy elimination, direct candidates, and batch transactions need no environment settings.
The first-candidate probe regresses the real catalog and is now removed.

Three additional compiler prototypes implement the [compiler design](sql-set-composition-compiler-design.md).
These passes now run automatically in both ordinary and diagnostic builds.
Their earlier opt-in switches and the older compiler alternatives are removed.
The tables below retain measurements from the comparison build before that cleanup.
The selected implementation is the row labeled "All three new prototypes".
They improve the real scan modestly, while eliminating the quadratic search work on supported synthetic cases.
The remaining real-corpus gap is still about 4.5 times native.

## Implemented prototypes

| Compiler pass | Change |
| --- | --- |
| Filter fusion | Fuse deterministic filters and initial metadata projections in three-valued conjunctions |
| Shared search | Evaluate independent descendant predicates once per eligible node, then seek each root's first match |
| Bounded frame fusion | Inline adjacent, single-consumer frames within explicit materialization and query-size limits |

Shared search accepts kinds, plain regex, fresh-environment patterns, and conjunctions of those leaves.
Those predicates cannot read the input captures or return unknown.
They can bind fresh captures, which the selected node returns to the original root.
All other search bodies retain their existing lowering.
The successful-node relation uses a real `(file, node)` B-tree index.
Frame virtual tables do not support the required range seek.

Frame inlining substitutes each definition once and preserves the declared column names.
It preserves non-key column affinity with unary plus, which leaves the values unchanged.
It retains row-ID allocation, native frame handles, grouped results, sorted results, compound queries, and indexed join inputs.
Only complete metadata projections can absorb a potentially failing native terminal.
Other mutations and recursive checks remain barriers.
Each query can absorb at most four frames and contain at most 64 KiB of SQL text.

## Real-corpus measurements

The tuning input contains 135 supported Python policies, 141 AST matchers, and 32 files with 1,318,099 source bytes.
The native oracle returns 33 complete findings, zero failures, and zero blind files.
Each variant uses a fresh process and four workers, with two files per SQL batch.
Three interleaved rounds give these median apply CPU times:

| Variant | CPU seconds | SQL/native ratio |
| --- | ---: | ---: |
| Native | 1.355 | 1.00× |
| Enabled defaults | 6.337 | 4.68× |
| Frame inlining | 6.265 | 4.62× |
| All three new prototypes | 6.072 | 4.48× |

The combined gain is 4.2% against the enabled defaults.
Frame inlining alone gains 1.1 percent.
The selected configuration includes it with the other two passes, using the measured bounds.
An earlier interleaved comparison of the first two prototypes also showed small gains.
Their combined CPU time was 6.057 seconds against 6.223 for that comparison's control.

A separate single-round check scans all 138 manifest files, including 106 files outside the tuning subset:

| Variant | CPU seconds | Full findings |
| --- | ---: | ---: |
| Native | 7.624 | 580 |
| Enabled defaults | 35.495 | 580 |
| Frame inlining | 34.578 | 580 |
| All three new prototypes | 34.202 | 580 |

The combined gain is 3.6% on this larger input.
Every timed and traced variant preserves the complete native output, including captures and suppression state.
All variants report zero failures and zero blind files.
These are diagnostic measurements using the allocator and build settings from the earlier reports.
The shared host makes wall time unsuitable for throughput claims.
Tim's external 818-clause catalog remains unavailable.

## Work removed and work retained

Separate complete traces show:

| Counter | Enabled defaults | All new prototypes | Reduction |
| --- | ---: | ---: | ---: |
| Executed statements | 31,936 | 29,982 | 6.1% |
| Affected rows | 7,820,805 | 7,107,970 | 9.1% |
| VM steps | 284,939,748 | 273,521,130 | 4.0% |
| Frame selection/copy rows | 2,983,583 | 2,270,366 | 23.9% |
| Candidate rows | 678,834 | 678,834 | 0.0% |
| Recursive demand rows | 465,150 | 465,150 | 0.0% |
| Capture-bearing demand rows | 156,872 | 156,872 | 0.0% |

Affected rows count cumulative insert and update work, rather than distinct nodes or peak memory.
These traces exclude the final finding selects.
The timed statement counters include those selects, so their totals differ slightly.

The new passes remove metadata copies without narrowing the expensive search and demand domains in this catalog.
The trace records only seven executions of a shared-search lookup, yielding zero rows.
Its query plan uses primary-key range seeks as intended.
Most active policy searches have capture-dependent, nested, recursive, or unknown-producing bodies outside this prototype's scope.

The fact path remains substantial: about 0.93 CPU seconds in the combined trace.
Its VM work increases from 13.9 million to 15.0 million steps after fusion.
Fewer frames do not guarantee fewer instructions because expressions and lookups can execute again after inlining.
The overall 11.4-million-step reduction agrees with the modest measured CPU gain.

## Shared search removes quadratic work

The standalone reproduction generates nested TypeScript `if` roots and compares full findings and captures.
The early case matches each root's own condition.
The late case matches a `target` identifier at the deepest point.
The negative case contains `f(a, b)` at each level and tests the fresh pattern `f($X, $X)`.
It fails repeated-variable unification at every candidate.

A missing literal or regex alone is an invalid negative search benchmark here.
The existing anchor planner can reject it before SQL runs, even when its token appears outside the searched subtrees.
The final negative fixture passes necessary pattern anchors and verifies that SQL executes statements.

At depth 4,096, single-run apply CPU measurements are:

| Case | Native ms | Enabled SQL ms | Shared search ms | SQL row operations before → after |
| --- | ---: | ---: | ---: | ---: |
| Early | 52.21 | 11,542.06 | 50.45 | 25,184,256 → 28,672 |
| Late | 35.59 | 7,408.31 | 45.68 | 16,805,888 → 20,484 |
| No match | 1,912.62 | 2,191.65 | 52.82 | 8,394,752 → 8,192 |

The early and late cases each return the same 4,096 complete findings across all engines.
The negative case returns none.
Measurements at depths 128, 512, and 2,048 confirm linear row growth for shared search.
The original candidate-pair expansion grows quadratically.
Unlike the first-candidate probe, shared search does not restart a complete search after a failed probe.
No recursive CTE or per-row native matcher callback is introduced.

## Rejected widening of frame fusion

Single-consumer status alone does not make a frame expendable.
The plan gate caught grouped CTE scans after the first inliner removed their materialized indexes.
Grouping, sorting, and compound queries now retain their barriers.

Broader join inlining then passed that gate but made the generated-rule run much slower.
A process sample showed repeated frame filtering and SQLite execution.
The final pass preserves arbitrary join inputs and permits only the known metadata lookup by the original input key.
The full generated suite returns to normal runtime with that restriction.
The earlier, broader implementations are excluded from the final binary and measurements.

## Next compiler work

The trace makes capture-aware candidate selection the next priority.
The 156,872 capture-bearing demands and 678,834 candidate rows do not change under these prototypes.
Capture-dependent predicates need safe restrictions before those rows are materialized.
A single-variable bound pattern offers a possible restriction, but its lowering must preserve native structural unification.
A raw text hash is insufficient without proving the permitted node shapes.

For bodies that cannot become a shared eligible-node relation, the design specifies progressive search frontiers.
A frontier must advance unresolved roots, preserve strict and non-strict unknown behavior, and retain dependencies from earlier candidate batches.
Recursive answer changes must reopen affected stopping decisions.
That state machine is not implemented by these prototypes.

The design also describes a possible key specialization for captures used only through identifier unification.
An interned token atom can distinguish lookup identity from the original occurrence that must remain in the returned capture.
Nullable inputs and identity-sensitive uses require separate handling.
This experiment is not implemented and has no speed claim.

Further copy reduction needs row-selection views or shared immutable capture columns in the frame store.
This can preserve the evaluated values and indexes without substituting their expressions into later SQL.
It requires ownership and lifetime changes: clearing or reusing an input register must not invalidate a surviving view.
It also requires memory accounting for retained backing buffers.
The CTE experiment shows why widening fusion without those guarantees is a poor next step.

## Validation and reproduction

After switch removal, the ordinary release build passes 114 SQL tests with zero failures.
The diagnostic build passes 115 tests, including 3,000 generated native/SQL comparisons with zero differences.
Its query-plan audit covers 722 clauses and 33,686 statements.
Nine measurement tests remain ignored in each build.
The diagnostic test build uses 16 code-generation units for the engine crate.
Both builds use the allocator override described in the earlier reports.
Clippy passes with warnings denied.

The following historical checks describe the comparison build at `b4a9b406`, before the compiler switches were removed.

The comparison SQL suite passes 116 tests with nine measurement tests ignored in each configuration:
new defaults, filter fusion, shared search, frame inlining, and all three prototypes together.
Each configuration targets 3,000 compared generated cases and includes saved replays and the query-plan gate.
Regression tests cover first-match captures, outer captures, unknown propagation, domain gaps, linear work, column affinity, and error ordering.
The promoted production build passes its 109-test SQL suite.
All 21 seeded replay mutants are rejected through output differences or an unexpected refusal.
Production compilation and Clippy with warnings denied also pass.
A compatibility run also passes with the older broad probe and all new prototypes enabled.
Workspace reuse preserves a recursive-read error, then returns the correct repaired findings with the same session.

Raw configurations, full output oracles, traces, process samples, and results are under:

```text
/tmp/sql-set-compiler-20261020
```

The standalone search reproduction uses no external corpus:

```sh
node benchmark/sql-set-search-scaling.mjs \
  --addon /absolute/path/unbounded-work.node \
  --output /tmp/rerail-search-scaling \
  --depths 128,512,2048,4096
```

The current build needs no compiler settings:

```sh
node benchmark/sql-set-investigation.mjs /absolute/path/config.json
```

Historical variants require builds from their recorded commits.
Environment settings no longer select those compiler variants.
