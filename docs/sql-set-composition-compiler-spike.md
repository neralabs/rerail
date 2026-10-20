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

This section records the open work at `89f71979`.
The later experiments in this document implement bounded versions of several proposals.
General frame-view integration and progressive SQL execution remain open.

The trace makes capture-aware candidate selection the next priority.
The 156,872 capture-bearing demands and 678,834 candidate rows do not change under these prototypes.
Capture-dependent predicates need safe restrictions before those rows are materialized.
A single-variable bound pattern offers a possible restriction, but its lowering must preserve native structural unification.
A raw text hash is insufficient without proving the permitted node shapes.

For bodies that cannot become a shared eligible-node relation, the design specifies progressive search frontiers.
A frontier must advance unresolved roots, preserve strict and non-strict unknown behavior, and retain dependencies from earlier candidate batches.
Recursive answer changes must reopen affected stopping decisions.
Those compiler passes do not implement that state machine.
The later representation experiment exercises its cursor contract outside the SQL driver.

The design also describes a possible key specialization for captures used only through identifier unification.
An interned token atom can distinguish lookup identity from the original occurrence that must remain in the returned capture.
Nullable inputs and identity-sensitive uses require separate handling.
The later producer experiment uses atoms only for a proved terminal subset.
General utility memo keys still retain capture occurrence identity.

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

## Capture and representation experiments, 20 October 2026

These experiments extend the review of NER-1562, the landing worktrees, Data-layer expressivity, and `ast-index-spike`.
The implementation uses the current Rerail tape format.
It does not import the landing tape format or GPU execution system.

Commit `7b43e4b4` records the first complete set of prototypes, including the first subtree-demand pass.
Commit `e1a46a28` selects capture-aware relation candidates and direct single-capture terminals.
The final configuration also includes the corrected demand-insertion guard described below.
No environment setting selects a new compiler path.
The independent representation models compile only into the test executable.

### Capture-aware candidates and value postings

The compiler identifies a mandatory single-variable pattern before candidate allocation.
It crosses only kind and regex prefixes that cannot bind captures or return unknown.
For a disjunction, every branch must require the same input capture.
Parameterized utility references translate the capture slot into the caller's scope.

A bound named leaf gives a necessary text-equality condition for native unification.
The node table shares lazy `(kind, text hash)` postings across all requesting clauses in a batch.
The cursor intersects those postings with the walk's range and structural conditions.
A single-kind lookup shares its immutable posting buffer.
A missing value returns an empty scan without allocating an empty posting buffer.

Unbound and composite inputs retain their original candidate domain.
Candidate composites remain in the posting index because native leaf unification can compare their complete text.
Hash equality is only a necessary condition.
Native unification still checks the match and returns the correct capture occurrence.

Three-valued searches use the restriction only when the required pattern precedes every unknown-producing or binding prefix.
The compiler retains unknown and withheld-capture behavior elsewhere.
Count uses the same candidate restriction when the proof holds.

On the 32-file catalog, this pass removes 16,521 row operations from 7,107,970.
VM work decreases from 273,521,130 to 273,315,435 steps, about 0.08 percent.
The 465,150 demand rows and 156,872 capture-bearing demand rows remain unchanged.
A work-volume test confirms a larger reduction on a suitable bound-capture search in both verdict modes.
This proves applicability to that shape, rather than broad applicability to the catalog.

### Direct single-capture terminals

The common Smart root pattern `$X` no longer needs an environment map for its supported input shapes.
It accepts unbound inputs and exact leaf unification directly.
It also handles the identical-node case.
The general native matcher handles composite structural unification.
These are automatic specializations within the same terminal.

The specialized terminal preserves contextual root-kind restrictions and named extra nodes.
It returns the candidate occurrence after successful unification, as native capture insertion does.
It does not return the earlier input occurrence merely because their texts match.
All node pairs in malformed Python and TypeScript fixtures agree with general unification.
Those checks include nullable inputs, composites, comments, contextual patterns, and non-Smart patterns.

### Real-corpus comparison

The comparison retains the earlier 135 supported policies, 141 AST matchers, and 32 files.
Each run uses four workers and two files per SQL batch.
Three rounds alternate variant order and start a fresh process for each variant.
All variants preserve the complete 33-finding oracle, with zero failures and zero blind files.

| Variant | Median apply CPU seconds |
| --- | ---: |
| Native | 1.309 |
| Previous SQL configuration | 5.780 |
| Capture-aware relation candidates | 5.790 |
| Relation candidates and direct capture terminals | 5.618 |
| First broader subtree-demand pass | 5.771 |

The terminal combination improves the median by 2.8 percent in that initial comparison.
A five-round confirmation does not reproduce this gain: the control takes 5.877 CPU seconds and the terminal combination takes 5.956 seconds.
The terminal timing improvement is therefore unconfirmed.
The relation restriction alone does not improve this catalog's timing.
The first subtree-demand pass adds no executed support query and removes no further VM work.

The IR inspection explains the missed proof.
B020's `name-ref` utility tests its capture before its identifier kind.
The corrected analysis includes mandatory kind tests later in the same conjunction.
It does not move bindings or unknown-producing operations.
It recognizes five B020 utilities whose success needs an equal-text identifier in the current subtree.

The compiler places this guard directly in demand insertion.
It retains the original input frame and recursive origin columns, with no extra filter frame.
Nullable and composite inputs pass the guard unchanged.
Same-node cycles, upward walks, optional witnesses, and unknown-producing prefixes block the proof.
Three-valued unknown stops also block it.
The analysis has fixed limits of 4,096 IR visits and 64 utility levels.
Exhausting either limit retains the original demand domain.

A fresh three-round comparison gives:

| Variant | Median apply CPU seconds |
| --- | ---: |
| Native | 1.247 |
| Previous SQL configuration | 6.020 |
| Relation candidates and direct capture terminals | 5.935 |
| Corrected demand guard with those specializations | 5.744 |

The corrected combination gains 4.6 percent against the previous SQL configuration.
The remaining gap in this comparison is 4.6 times native.
A separate single-round check covers all 138 files, including 106 files outside the tuning subset:

| Variant | Apply CPU seconds | Full findings |
| --- | ---: | ---: |
| Native | 7.583 | 580 |
| Previous SQL configuration | 33.644 | 580 |
| Corrected combination | 32.199 | 580 |

That larger check gains 4.3 percent and preserves the complete native output.
Every run reports zero failures and zero blind files.
Separate complete traces show:

| Counter | Previous SQL | Corrected combination |
| --- | ---: | ---: |
| Executed statements | 29,982 | 29,568 |
| Affected rows | 7,107,970 | 7,017,058 |
| VM steps | 273,521,130 | 271,112,090 |
| Demand rows | 465,150 | 447,253 |
| Capture-bearing demand rows | 156,872 | 138,975 |

The capture-bearing demand reduction is 11.4 percent.
Overall VM work decreases by only 0.9 percent, which limits the available gain.
Guarded demand statements consume about 13 milliseconds across 262 executions in the trace.
Further tuning of those guard statements cannot explain or close a multi-second gap.

### Shared immutable capture storage

The executable model stores nullable integer columns in immutable reference-counted pages.
Columns use narrow or wide integers with null bitmaps.
A view contains shared pages, a projection, and a row-selection vector.
A further selection composes row indices instead of copying capture values.
Metadata remains separate.

The source register can disappear or be reused while a view retains its pages.
A byte budget counts each retained payload once and releases it after the last view disappears.
An allocation failure leaves the existing views intact.
Tests cover nullable values, wide list hashes, repeated selections, projected columns, register reuse, and release accounting.
The byte counter excludes allocation headers and collection metadata.

The copy benchmark uses the real frame virtual table as its control.
It copies 20,000 rows with 16 capture columns through eight frames.
The view model removes 2,560,000 capture-cell copies and retains 2,040,064 payload bytes across the full chain.
Every final nullable cell agrees with the frame output.

This model does not implement general SQLite values, frame indexes, or `xUpdate`.
Its construction benchmark excludes the final consumer scan in both variants.
It therefore measures the representation opportunity, rather than complete SQL execution.
The next integration needs explicit view instructions in the plan and ownership through register reuse.

### Progressive batched search and count

The cursor model keeps each root's next candidate, verdict, count interval, and dependency set.
Each phase collects requests across unresolved roots, evaluates one set, and commits each root's ordered prefix.
It never restarts the complete search after a failed probe.

Strict search stops at its first unknown candidate.
Non-strict search can continue to a later true candidate and retains unknown on exhaustion.
Count stops at its minimum when no maximum or exact count is required.
It stops after exceeding the maximum.
An exact count consumes the whole reachable candidate set.
An unknown stop leaves the upper count bound open.

Dependency sets retain reads from earlier failed and unknown candidates.
A changed recursive answer reopens affected roots, including an earlier candidate that can replace the previous winner.
An interrupted cursor returns an error without a completed answer.
The integrated driver still needs atomic phase publication.
Dependency tests exercise the cursor separately from the batched request fixture.
Exhaustive six-event sequences compare cursor results with an independent full-sequence oracle.

The scheduler benchmark uses 512 roots with 4,096 candidate positions each.
An eager search produces 2,097,152 root/candidate pairs.
At quantum 32, a first-position match produces 16,384 pairs in one phase.
A match at position 127 produces 65,536 pairs in four phases.
A final-position or missing match still produces all pairs, in 128 phases.
Minimum-only and maximum-bound count cases produce 16,384 pairs.
The exact-count case still produces all pairs.

Quantum 1 minimizes overfetch but requires 4,096 phases for a missing match.
Quantum 128 reduces that case to 32 phases but overfetches 65,536 pairs for an immediate match.
Quantum 32 is a useful integration starting point.
Actual SQL dispatch cost and corpus stopping distributions must determine the final schedule.
The model does not include SQLite phase dispatch, unknown stop-rule traversal, or the complete recursive driver.
It is not an application speed measurement.

### Shared producers, packed execution, and owner postings

The producer model keys a supported leaf-input terminal by term, file, candidate, and interned input text.
It uses this equivalence only for the proved single-capture subset.
General utility keys retain capture identity.
The fixture reduces 100,000 requests to 1,101 producer evaluations and preserves every returned capture occurrence.
This fixture deliberately contains high reuse and does not estimate catalog reuse.

The packed terminal model reads the current tape directly for Smart `$X` with unbound or leaf input.
It compares complete match and capture outputs with ast-grep over 110,001 candidate nodes.
Composite inputs require the general matcher.
This prototype does not implement ellipses, structural patterns, semantic facts, JIT compilation, or GPU execution.
Its local terminal gains do not imply comparable application gains.
The much smaller end-to-end gain confirms that terminal execution alone cannot close the gap.

The owner experiment builds nearest-owner columns and reverse postings in one preorder pass.
It includes the owner node itself, matching the stated oracle.
Nested-owner tests compare every node with ancestor traversal.
For 2,000 owners and 28,001 nodes, repeated scans visit 56,002,000 owner entries.
The reverse index builds 28,000 posting entries and answers 2,000 lookups.
This addresses the full-owner-array scan observed in the expressivity worktree.
The current SQL branch does not automatically gain from an index used by that separate implementation.

### Immutable compiler artifacts

The cache model uses a policy-scoped singleflight cell for each immutable pattern.
Its key includes dialect, source context, and selector.
It retains successful artifacts and source errors.
Concurrent requests build one artifact, and different dialects or selectors cannot share it.
The benchmark reduces 3,200 repeated builds to 16 and compares leaf outputs and captures.

This model does not cache mutable matcher graphs or statement-specific component numbers.
It applies to compilation cost, rather than the multi-second apply cost measured here.
The current lowerer already shares capture analysis and utility analysis across its program.
Reusing landing component IDs would mix different graph and capture scopes.

### What closes the remaining gap

The selected compiler still executes about 271 million VM steps and seven million row operations for 1.3 MB of input.
A small input can therefore have a large execution cost without a slow parser or one missing index.
The selected specializations remove only a small part of that relational work.
The earlier trace also attributes about 0.93 CPU seconds to the fact path.
That path still uses serialized environment keys.

The next integrated experiment needs shared capture pages and progressive requests in the actual plan driver.
The compiler needs explicit ownership, batch requests, continuation cursors, and dependency invalidation.
Producer sharing needs a proved capture equivalence and a map back to each original occurrence.
The B023 closure/loop-variable clause remains a concrete target.
Its `c68_d1` producer still inserts 86,498 capture-bearing demands in this trace.
The subtree witness proof cannot reject a general absence predicate, such as an unbound-name check.
Value-only unit keys need a separate proof that covers every read and every returned binding.
A successful call must distinguish a preserved caller occurrence from an occurrence that the body binds.
A broad text-atom substitution in general memo keys remains unsafe.
The executable models establish these contracts but do not implement that complete driver.

### Final validation

The final release SQL suite passes 132 tests, with 16 diagnostic or measurement tests ignored by default.
It includes 3,000 generated native/SQL comparisons with zero differences.
Those comparisons cover 1,699 three-valued cases, including 402 unknown verdicts.
Nine unsupported cases and 11 native utility-limit cases were excluded from the compared set.
The plan audit covers 722 clauses and 33,686 statements.

Clippy passes with warnings denied, and the ordinary production release compilation passes.
The final addon preserves the complete 32-file output oracle and reproduces the corrected trace counters above.
Session reuse after an unsupported recursive rule also agrees with a fresh native session after source repair.
Three separate representation measurement rounds pass all oracle assertions and produce 25 measurements each.
These model measurements do not replace the application comparisons above.

Raw configurations, binary hashes, full output oracles, traces, IR inspection, and measurements are under:

```text
/tmp/rerail-next-spikes-20261020
```

The representation runner requires a release test executable:

```sh
node benchmark/sql-set-representation-spikes.mjs \
  --binary /absolute/path/to/rerail-test-executable \
  --rounds 3 \
  --output /tmp/rerail-representation-results.json
```

The full application comparison uses `benchmark/sql-set-tuning.mjs` with a pinned corpus configuration.
The historical control binary needs its former three compiler settings enabled.
Current builds require none of those settings.
The benchmark uses the same allocator override and optimized build configuration as the earlier reports.
Tim's external 818-clause catalog remains unavailable.
