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

## Integrated execution experiments, 20 October 2026

This continuation connects the representation experiments to the SQL engine.
The compiler selects each specialization automatically.
No environment setting selects a compiler variant.

### Shared frame selections

A key-preserving projection can retain immutable source columns and a shared selection vector.
SQLite evaluates the predicate and returns source row IDs.
The frame store publishes the selected columns after that query completes.
Projection columns retain their SQLite types, nulls, wide integers, text, blobs, and real values.
Indexes and native input readers use the same column interface.

A later selection composes its row vector once per distinct source vector.
The source register can clear or receive new rows while a selection retains its pages.
A write to a shared column first copies its current values.
A clear releases that register's page references.
The last reference releases the page and its retained-byte charge.

The selected specialization requires an empty destination, an ascending integer source key, and direct column projections with identical affinity.
It supports filters over one source frame.
Joins, expression projections, generated keys, duplicates, and changed affinity use normal frame insertion.
A retained-page limit of 512 MiB bounds pages that survive register reuse.
The existing logical row budget also applies.
Exhausting the retained-page limit uses normal insertion without changing the result.
Trace counters include the actual selection query's VM work.

### Progressive packed pattern walks

The compiler recognizes a total pattern body with kind and regex tests.
It supports child, descendant, ancestor, previous-sibling, and next-sibling walks with fixed neighbor or end stops.
One virtual-table call receives all input roots through integer frame handles.
Persistent cursors advance unresolved roots in phases of 32 candidates.
A winner ends a first-match cursor immediately.
A count ends after its minimum when it has no maximum or returned count.
Exceeding a maximum ends that cursor as false.
A returned count consumes the entire candidate domain.

Descendant cursors share kind postings within the input set.
A supported bound root capture also reads the shared value postings.
The Smart root-capture kernel reads the current tape directly.
General structural patterns use native unification within the same terminal.
Every successful pattern returns its candidate capture occurrences.

Unknown-producing bodies, recursive reads, semantic effects, field walks, and rule-based stops remain in the relational driver.
These operations require its complete dependency and unknown-stop logic.
The compiler proof prevents the packed cursor from bypassing those contracts.
The existing three-valued and recursive comparisons exercise both sides of that boundary.
At that revision, the general recursive frontier remains a design experiment.
A full integration needs persistent lineage snapshots for each request phase.
Reusing temporary frames can otherwise erase earlier failed or unknown reads before the driver records their dependency edges.
The selected compiler keeps those operations in the existing driver.

### Shared producers and capture identities

A new proof identifies utility input captures that affect evaluation only through leaf-text unification.
A fixed return contract must also hold on every successful path.
The first contract preserves the original caller occurrence.
The second contract always returns a newly bound candidate.
The rebinding proof accepts mandatory named root captures and recursive paths that reach such a capture.
Mixed return paths keep their occurrence keys.
The proof follows recursive references and crosses negation or count only with their binding-discard rules.
Semantic reads, occurrence walks, exports, constraints, and uncertain operations block the proof.
The analysis has a fixed 4,096-visit limit.

For a proved slot, equal-text leaf inputs use one canonical node in the demand key.
The node index compares exact source strings and does not treat a hash collision as equality.
Composite inputs retain their original identity.
A preserving call keeps its own original capture occurrence.
A rebinding call returns the winning candidate occurrence.
An unknown answer keeps the caller occurrence under both contracts.
Dependency edges still identify the shared demand and its answer.
Utilities with uncertain binding behavior retain their original memo keys.

General pattern producers share results only for identical typed input tuples within the immutable batch.
Their cache retains at most 65,536 entries and approximately 32 MiB of payload.
The direct packed root-capture kernel avoids that cache because its own comparison is small.

### Integer fact keys and compiler artifacts

Capture-free facts now receive packed integer file/node keys.
Their SQL statements omit environment-string construction and its extra key frame.
Capture-bearing facts retain their full environment keys.
Both representations preserve distinct-key evaluation, file scopes, parsed trees, and unknown verdicts.

Policy compilation shares immutable patterns through a scoped singleflight cache.
The cache key includes language, context, and selector.
It also retains source errors during that compilation.
Matcher graphs, utility instances, and recursive component numbers remain local to each query.
The cache releases its map after compilation, while the compiled matchers retain shared patterns.
SQL lowering shares terminals by artifact, dialect, and input-read shape.

The current nearest-node table already builds reverse owner groups in linear passes.
The owner-posting experiment therefore needs no second index in this branch.
Its separate expressivity-worktree opportunity remains specific to that implementation.

### Validation scope and rejected candidate

The final integrated build passes 1,425 code-rule tests, including 3,000 generated native/SQL comparisons with no differences.
Its 138-file run preserves all 580 native findings.
The SQL plan audit covers 722 clauses and 33,005 statements.

A broader selection candidate initially retained an index built while its destination was empty.
It lost three findings on the 138-file catalog and failed recursive differential cases.
The corrected build restores all findings and passes the complete differential set.
Selection publication now invalidates destination lookup indexes and scan order, as ordinary insertion does.
A regression case populates a selection after an indexed empty read.
The incorrect candidate remains only in the external measurement artifacts.
Its timing is not an accepted performance result.

### Selection experiments and configuration choice

The broader projection experiment supports joined filters and constant metadata columns.
It shares 1,833,103 logical rows and reduces the complete trace to 252,243,463 VM steps.
The selected narrow projection shares 190,187 rows and uses 266,576,348 VM steps.
The published build uses 271,112,090 VM steps.
These counters include the selection queries that replace ordinary insertion.

A direct three-round comparison on 138 files gives 26.607 CPU seconds for narrow selections and 27.238 seconds for broader selections.
The broader selection reduces maximum RSS from 746 MB to 641 MB in that comparison, but increases median CPU time by 2.4 percent.
A separate variant copies keys and traversal metadata into contiguous columns.
In another three-round comparison, that variant takes 37.813 CPU seconds against 31.336 seconds for narrow selections.
The compiler therefore retains narrow selections as its single configuration.
The broader variants remain external experimental binaries.

The same confirmation compares narrow selections with the published branch: 31.336 against 32.002 CPU seconds, a 2.1 percent gain.
An earlier single-round 138-file comparison reports an 11.7 percent gain.
That larger timing gain is not stable across repetitions on this shared host.
The initial three-round 32-file comparison reports a 6.8 percent gain.

Before uniform rebinding, demand volume remains 447,253 rows, including 138,975 capture-bearing rows.
The preserving proof alone applies to 12,284 inserted rows but removes no further demand rows in this catalog.
That initial complete trace retains 93,896 inserts into B023's `c68_d1` producer.
This complete total includes trace fragments beyond the earlier single-fragment count of 86,498.
The scan terminal runs 12 times in the 32-file trace.
Its statement CPU time is about one millisecond.
The capture-free fact path contributes the clearest measured reduction: about 889 to 495 milliseconds in the first comparable traces.

The remaining multi-second cost still comes from millions of relational rows and hundreds of millions of VM steps.
The integrated spikes do not establish native-speed SQL execution.
Tim's unavailable external 818-clause catalog remains outside this comparison.

### Uniform rebinding and final results

The final compiler also proves a uniform rebinding contract for mandatory named root captures.
Each successful recursive path must return such a candidate capture.
The caller retains its own input occurrence on an unknown answer.
A mixed preserving/rebinding disjunction blocks the proof.
Structured patterns without this root-capture guarantee also block the rebinding proof.

This specialization applies to the B023 producer that the preserving proof alone cannot share.
The complete 32-file traces give:

| Counter | Published `d5321267` | Final compiler |
| --- | ---: | ---: |
| Executed statements | 29,568 | 29,417 |
| Affected rows | 7,017,058 | 6,790,242 |
| VM steps | 271,112,090 | 265,045,566 |
| Demand rows | 447,253 | 406,898 |
| Capture-bearing demand rows | 138,975 | 98,620 |
| B023 `c68_d1` demand rows | 93,896 | 58,607 |

Capture-bearing demands decrease by 29.0 percent, and the B023 producer decreases by 37.6 percent.
Total demand volume decreases by 9.0 percent.
Overall VM work decreases by only 2.2 percent.
This smaller reduction limits the application gain.

An initial three-round 32-file comparison reports 5.497 CPU seconds for the published branch and 4.559 seconds for the final compiler.
A five-round confirmation gives a smaller improvement:

| Comparison | Published CPU seconds | Final CPU seconds | Improvement |
| --- | ---: | ---: | ---: |
| 32 files, five-round medians | 4.447 | 4.316 | 2.9% |
| 138 files, one round | 25.461 | 24.961 | 2.0% |

The larger initial timing gain is not stable.
The final compiler preserves all 33 findings on 32 files and all 580 findings on 138 files.
Every accepted application run reports zero failures and zero blind files.
The host runs other workloads, so these CPU timings need their interleaved controls.

The final source passes 1,425 code-rule tests with zero failures and 40 ignored measurement or external-corpus tests.
Its 3,000 generated native/SQL comparisons report zero differences.
A separate skip-count check passes 141 SQL tests, including 500 generated comparisons.
The final source also passes Clippy with warnings denied and the ordinary production release compilation.
A reused session agrees with fresh native and SQL sessions after an unsupported recursive batch and a source repair.

The selected code includes narrow shared selections, progressive packed pattern walks, proved producer sharing, integer capture-free fact keys, and immutable compiler artifacts.
The existing nearest-node index already supplies reverse owner groups.
At that revision, general recursive scheduling remains a test-only spike because request phases need persistent lineage and atomic dependency publication.
The compiler retains the existing recursive driver for those operations.
No opt-in or opt-out setting selects these compiler paths.

Configurations, source hashes, binary hashes, full output oracles, and traces are under:

```text
/tmp/rerail-integrated-spikes-20261020
```

The final artifacts use the `uniform-` prefix.
The five-round timing confirmation is `uniform-confirmation.json`.
The final compiler still takes roughly four times the native CPU time on this catalog.
These spikes reduce specific costs and establish their semantic contracts, but they do not close the complete SQL performance gap.

## Shared projections and recursive scheduling, 20 October 2026

This section supersedes the integration limits in the preceding report.
The compiler now includes broader shared projections and a general candidate scheduler.
Both operate automatically within the existing SQL engine configuration.
The earlier B-tree frame-storage diagnostic override is removed because the scheduler requires frame ownership.

### Shared projections with computed captures

A projection can share direct columns through joined filters and constant metadata.
It can also compute capture columns while sharing the remaining columns.
SQLite evaluates the predicate and computed expressions in one query.
Rust installs the direct columns as views over immutable pages.
Computed columns retain SQLite's returned value types.
The projection parser respects nested expressions, quoted commas, escaped quotes, and clause names inside literals.

A complete identity projection needs no SQLite query or row mapping.
Dense integer keys map directly to source rows.
Other selected keys use the existing ordered lookup.
Composed views share one row mapping across their columns.
Integer comparisons follow the underlying page directly.

Shared pages contain typed buffers, which remove a second storage dispatch from each read.
When the final reader clears a shared `i32` page, the register reuses its buffer.
Other readers retain their own immutable page references.

Publication clears the destination's lookup indexes and scan order.
A source clear or register reuse does not invalidate a destination page.
Appending to a shared column first creates an owned copy.
The retained-page budget remains 512 MiB, and the logical row budget still applies.
Selections outside the supported shape retain ordinary insertion semantics.
These include duplicate output keys, generated keys, aggregates, scalar subqueries, and changed affinity.

### Candidate ownership and recursive dependencies

The compiler schedules relation searches and predicate counts when their bodies contain recursive or semantic demands.
The operation supports nested compiler operators, parameterized utilities, field walks, and rule stops.
Single-candidate walks need no scheduler.
Simple pattern walks retain the existing packed kernel.
Set-only bodies retain their direct SQL evaluation.
An exact returned count still consumes its complete domain.

Each invocation owns its candidate order, parent cursors, and immutable source pages.
The driver groups stable candidate keys by parent and walk order.
It publishes each candidate prefix directly into a shared frame.
This operation uses no SQL ranking table, cursor-state updates, or per-cell `xUpdate` calls.
Nested invocations own separate cursors.
An empty domain still runs one phase, which clears temporary results from an earlier invocation.

A phase evaluates the complete retained prefix.
The final phase therefore leaves coherent intermediate frames for deferred dependency tracking.
Earlier failed and unknown candidates remain in that evaluation.
The existing lineage logic cuts recursive reads after the deciding candidate.
No dependency refers to temporary keys from a discarded phase.

When a recursive answer changes, the existing dependency worklist marks its readers dirty.
A dirty member starts new candidate cursors and evaluates its earlier candidates again.
An earlier candidate can replace a later winner and return its own capture occurrence.
The scheduler does not preserve a stale winner between recursive passes.

Strict searches stop on true or unknown candidate events.
Non-strict searches stop on true events and preserve unknown evidence otherwise.
Unknown stop events can close a strict prefix only after every earlier candidate has been evaluated.
Count predicates stop only when their true lower bound proves a minimum success or an over-maximum failure.
Otherwise the cursor exhausts its domain.

### Prefix growth and remaining costs

The initial prefix contains up to eight candidates per parent.
An unresolved parent doubles its prefix.
If its domain fits within four times the current prefix, the next phase consumes the remainder.
This rule avoids replaying an almost complete domain before the final phase.
A growing parent evaluates fewer than twice its domain's candidates before exhaustion.

A decided parent's prefix stops growing.
Its retained prefix can still be evaluated while another parent advances.
This replay preserves coherent lineage without per-phase dependency snapshots.
Recursive demand keys and pattern results remain cached across phases.
The initial candidate relation still contains the complete narrowed domain.
These costs remain visible in the benchmark; this implementation does not claim a delta-only scheduler.

The first native scheduler prototype passes 3,000 generated comparisons with zero differences.
Its initial three-round 32-file comparison uses 5.522 CPU seconds against 5.263 seconds for the published branch.
Maximum RSS decreases from 425 MB to 393 MB in that comparison.
That prototype precedes computed-capture sharing and the final prefix-growth tuning.
Its artifacts use the `native-prefix-` prefix in `/tmp/rerail-general-sharing-20261020`.


### Configuration comparison

Both prefix configurations use the same typed pages and final-reader buffer reuse.
Their source manifests differ only in the initial prefix constant.
The three-round 32-file comparison gives:

| Configuration | Median application CPU seconds | Maximum RSS MB |
| --- | ---: | ---: |
| Published `1124b629` | 5.509 | 429 |
| Initial prefix 8 | 5.466 | 392 |
| Initial prefix 32 | 5.525 | 397 |

The timing difference between prefix sizes is small on this shared host.
The selected configuration uses eight because it reduces first-phase overfetch.
No environment setting selects a prefix configuration.

The selected implementation passes 1,434 code-rule tests, including 3,000 generated comparisons with zero differences.
The skip-count check passes 150 SQL tests with 500 generated comparisons.
Clippy passes with warnings denied, and the production library check passes without warnings.
The SQL plan audit covers 722 clauses and 33,039 statements.
New cases check phase boundaries, unknown events, counts, multiple files, and capture byte ranges.
They also check that recursive changes reopen earlier candidates and that closed parents retain their prefixes.

Application artifacts remain under `/tmp/rerail-general-sharing-20261020`.
The selected binary and its source manifest use the `selected` prefix.


### Application results and limits

The five-round 32-file confirmation preserves all 33 findings.
The three-round 138-file confirmation preserves all 580 findings.
Every run reports zero failures and zero blind files.
Both engines use the same allocator shim, four workers, and two-file SQL batches.

| Corpus | Published CPU seconds | Selected CPU seconds | Published RSS MB | Selected RSS MB |
| --- | ---: | ---: | ---: | ---: |
| 32 files, five rounds | 5.523 | 5.507 | 426 | 391 |
| 138 files, three rounds | 31.063 | 31.469 | 756 | 628 |

The 32-file CPU result is effectively unchanged.
The 138-file confirmation uses 1.3 percent more CPU, with 17 percent lower peak memory.
An earlier 138-file comparison gives 31.839 versus 30.557 CPU seconds, a 4.0 percent improvement.
That timing gain does not reproduce in the confirmation.
Large wall-time variation on the shared host prevents a reliable latency claim.

The complete 32-file trace explains the remaining gap:

| Counter | Published `1124b629` | Selected configuration |
| --- | ---: | ---: |
| Executed operations | 29,417 | 33,210 |
| Logical affected rows | 6,790,242 | 7,156,019 |
| SQLite VM steps | 265,045,566 | 246,714,372 |
| Recursive demand rows | 406,898 | 406,550 |
| Capture-bearing demand rows | 98,620 | 98,277 |

The selected trace shares 2,371,528 rows through projections and 297,832 rows through candidate prefixes.
The native scheduler uses about 42 milliseconds of thread CPU across 2,988 phases.
Logical affected rows include shared views; they do not measure copied cells.

VM work falls by 6.9 percent, but demand volume falls by less than 0.1 percent.
Most demands in this catalog still require a complete search or fit inside the first prefix.
The scheduler therefore removes little recursive work here.
Prefix replay and shared-page reads offset the saved SQL work.
These changes complete the ownership and scheduling implementation, but they do not close the catalog's native-engine performance gap.

A separate case evaluates 64 roots with 512 candidates each:

| Winner position | Native CPU ms | Published SQL CPU ms | Selected SQL CPU ms |
| --- | ---: | ---: | ---: |
| First | 49.9 | 146.8 | 65.0 |
| Last | 73.6 | 169.4 | 176.4 |
| Missing | 69.2 | 153.4 | 164.0 |

All synthetic runs preserve complete output parity.
Early matches benefit from smaller demand sets.
Late and missing matches pay the prefix replay cost.
The named production tests bound early demand volume and check complete exhaustion for late and missing matches.

The timing artifacts use the `selected-` prefix.
They precede final storage corrections for negative row keys and projection-source parsing.
Compiler-generated row keys in these benchmarks are nonnegative.
Separate storage tests check SQLite's next-row-key behavior and projection sources inside quoted text or subqueries.

The final source and binary use the `complete` prefix.
Their patch includes the new compiler, scheduler, and storage source files.
The earlier timing sources also retain hash-checked copies of their new source files.

The final binary also preserves all 33 and 580 findings after both storage corrections.
Its trace retains the same VM, demand, and logical-row counters as the timing build.


## Shared root-input slices, 21 October 2026

The selected-root loader now exposes immutable Rust slices through a read-only `roots` virtual table.
It replaces the per-root SQLite INSERT loop and its temporary B-tree.
The SQL interface remains `(run, file, node)`, so emitted rule SQL needs no changes.
This is the selected-root input path, with no optimizer switch.
The small `runs` table still represents files that use all roots.

`RootInputs::install` retains the existing ascending, unique `Arc<[u32]>` slices.
It builds and sorts small metadata records for each `(run, file)` selection.
Unsorted or duplicate inputs are normalized to preserve the old `INSERT OR IGNORE` behavior.
Repeated selected-file entries merge their roots.
An empty selection remains empty and does not become an all-root selection.

Each SQLite cursor retains an immutable input snapshot.
Integer equality probes restrict the scan by run, file, and node.
SQLite rechecks affinity and collation, while unsupported probes use a conservative scan.
The planner leaves the unique-scan flag unset.
The session clears its input snapshot after both successful and failed batches.

Implementation: [inputs.rs](../native/src/code_rule_dsl/sql/inputs.rs) and [exec.rs](../native/src/code_rule_dsl/sql/exec.rs).
The original B-tree remains only as a correctness oracle in tests.

### Measurements

The comparison uses the existing 135-policy Python workload and its native output digests.
The 32-file tuning corpus and the 138-file corpus retain the earlier file lists and exclusions.
Each variant uses four workers, two files per batch, and the same release build settings.
Runs alternate between the frozen baseline and the new implementation, reversing their order each round.
Profiling is disabled during these measurements.

The baseline contains the native source from `72d4ca09`.
Both addons use the same diagnostic system-malloc shim and `unbounded-work` feature.
The shim is `/tmp/librerail_jemalloc_check_shim.dylib`.
The host is an Apple M5 Max with other concurrent work.
These results describe this diagnostic build and do not establish production allocator performance.

| Corpus | Rounds per variant | Baseline apply CPU | Virtual-root apply CPU | CPU reduction | Input/clear CPU | Peak RSS |
|---|---:|---:|---:|---:|---:|---:|
| 32 files | 5 | 5.280 s | 4.746 s | 10.1% | 460.9 ms → 16.7 ms | 374.7 → 365.5 MiB |
| 138 files | 3 | 29.356 s | 26.120 s | 11.0% | 2760.0 ms → 88.5 ms | 574.8 → 588.8 MiB |

Times are medians across fresh-process runs.
The input timer includes input maintenance and frame clearing, rather than root publication alone.
Peak RSS is the maximum across those runs.
Memory results are mixed, and wall times vary with host load.
The measured benefit is lower CPU, while rule execution still dominates the workload.

Earlier runs of the initial spike showed smaller CPU reductions, approximately 6.5% and 7.1%.
The table records fresh measurements of that final implementation.
Both sets showed a CPU reduction on both corpora.

### Profile and correctness checks

The final 32-file profile publishes **919,765 candidate memberships in 16 batch operations**.
Publication takes **0.666 ms of recorded thread CPU** and creates **zero per-root input INSERT executions**.
These published memberships refer to existing slices and do not represent copied rows.
The trace contains **105,837 events**, compared with **1,025,596** in the earlier insertion-path capture.
The report recognizes virtual publication and removes the per-root insertion recommendation for this capture.

All final benchmark runs match the stored native output digests, including complete captures and findings.
The profiled run also matches a fresh native evaluation.
The SQL suite passes **156 tests**, including 500 generated differential comparisons.
Tests compare root queries against the old B-tree, including duplicates, ordering, joins, NULLs, numeric text, real values, and blobs.
Additional tests establish slice sharing, cached-query reuse, cleanup after a forced read failure, and capture preservation.
Clippy and the production-feature build check pass.

Raw artifacts are under `/tmp/rerail-root-input-spike-20261021`.
`final-comparison-32.json` and `final-comparison-138.json` contain all measured runs and parity results.
`baseline-build.json` and `final-build.json` record binary hashes and native source hashes.
`final-source/` preserves the changed native files, including the new input module.
`profile-32/summary.md` and `profile-32/profile.html` contain the updated actionable profile.

## Remaining SQL costs after root-input adoption

The root-input implementation is part of the production SQL path in `b16d698f`.
The main design document now describes this input contract.
There is no alternative root loader or runtime switch.

A fresh profile uses the same 32 files, 135 policies, four workers, and two files per batch.
It preserves complete output parity with the native engine and the stored digest of 33 findings.
The artifacts are in `/tmp/rerail-next-sql-bottleneck-20261021`.
The addon and native source hashes are in `build.json`.
These measurements use the diagnostic allocator described in the preceding section.

### Index construction is a small cost

The profiler now times frame lookup construction separately, including required scan-order preparation on cache misses.
All **2,530 builds took 5.6 ms**, approximately **0.1% of 5,100.7 ms recorded CPU**.
An earlier summary grouped expensive queries under an index-build warning without measuring construction time.
The warning described repeated builds, but did not establish that builds caused the query cost.

The summary now includes construction times and timed-build coverage.
Cheap measured indexes no longer classify entire query families as index-construction priorities.
Old captures retain an explicit indication that construction time is unavailable.
The timer runs only during a profiled cache miss, with no clocks in row callbacks.
Construction time is already part of SQL CPU and does not increase the reported budget.

### Join order causes avoidable frame expansion

The default capture records **251,656,437 SQLite VM instructions** and **12,623,013 frame advances**.
Root publication takes 0.702 ms and performs no individual root INSERT calls.
The remaining problem includes excessive intermediate work after input publication.

One sibling query, `Q3772`, belongs to `ruff-b039-mutable-contextvar-default`.
Its relevant SQL is:

```sql
FROM f145_0 f
JOIN node fn ON fn.file = f.file AND fn.id = f.node
JOIN node c ON c.file = f.file
  AND c.parent = fn.wpar AND c.id <= fn.aprev AND c.kind IN (215)
```

SQLite starts with the global kind postings for `c`.
It joins each candidate to the frames for that file, then probes `fn` and tests the sibling relationship.
This order advances frame cursors **1,178,918 times** for **1,875 output rows** across seven executions.
The query takes **146.1 ms** and **15,426,221 VM instructions**.
Its index construction takes only **0.012 ms**.

The compiler emits reorderable joins in `Emitter::tree_candidates`.
The frame planner advertises a fixed scan estimate of **1,048,576 rows**.
The node planner also uses fixed estimates, rather than the actual candidate cardinalities.
These estimates permit an expensive order for a dependent tree walk.

A diagnostic run sets the existing benchmark-only `RERAIL_SQL_FRAME_SCAN_COST=1000` override.
The production default stays unchanged.
This run starts the query from its input frame and then probes the anchor before candidates.
The same seven executions produce the same 1,875 rows:

| Query work | Default estimate | Diagnostic estimate |
|---|---:|---:|
| Frame advances | 1,178,918 | 1,055 |
| VM instructions | 15,426,221 | 4,791,721 |
| Profiled query CPU | 146.1 ms | 47.2 ms |

The whole workload also retains complete native output parity.
Its VM count decreases from **251.7 million to 239.1 million**.
Its frame advances decrease from **12.62 million to 11.24 million**.
Some other query shapes require more instructions with the lower estimate.
These single profiled captures establish a plan problem, not an accepted global tuning change or an unprofiled speedup.

The next targeted compiler change is anchor-driven scheduling for dependent relation walks.
It must keep parent, sibling, subtree, and kind restrictions available before candidate expansion.
An explicit join order or a native bounded candidate source can enforce this contract.
A global scan-cost decrease changes unrelated queries and does not solve the cardinality model.
Cached statements also need estimates that remain useful across batches.

### Terminal and demand work remain substantial

The default capture has these disjoint SQL families, classified by SQL text:

| Family | Whole-query CPU | Executions |
|---|---:|---:|
| Node joins and navigation | 1266.9 ms | 8,172 |
| Fact terminal materialization and rejoin | 628.9 ms | 1,158 |
| Pattern terminal materialization and rejoin | 485.1 ms | 1,022 |
| Demand insertion and deduplication | 330.0 ms | 1,566 |
| Demand expansion keys | 216.9 ms | 761 |

Terminal query CPU includes native matching and semantic evaluation.
It does not isolate SQLite materialization overhead.
The terminal SQL gathers keys, materializes a batched terminal result, and joins that result back to its input frame.
The next measurement must separate terminal evaluation from those scans, key sets, and rejoining costs.
Removing `MATERIALIZED` without another batching contract can restore per-row terminal calls, as the original design investigation established.

Capture-bearing demands also repeat canonical-key expressions in projection and anti-join predicates.
For example, `Q3611` repeats a `COALESCE((SELECT atom FROM node ...), f.s1)` expression.
Its plan contains correlated node lookups and a temporary B-tree for `DISTINCT`.
It takes **106.5 ms** and **6,698,391 VM instructions** across 20 executions.
Projecting the canonical integer key once is a separate compiler improvement after dependent-walk scheduling.

`profile-32/` contains the default capture and updated summary.
`profile-frame-cost-1000/` contains the diagnostic capture and its recorded environment.
`join-order-probe.json` compares identical SQL text with equal execution counts across both captures.
`sql-families.json` records the family classification and counters.

## Join, CTE, and statistics follow-up

The [SQL shape investigation](sql-query-shape-spikes.md) compares joins, correlated subqueries, CTEs, copied inputs, and `ANALYZE` commands.
It includes live-frame replay and 51 fresh-process workspace runs with complete native output parity.
Explicit dependency order gives the hot sibling walk a bounded parent/range plan.
The broader prototype reduces median apply CPU by approximately 3% on both tested corpora.
Additional materialization and statistics do not close the remaining performance gap.
The comparison alternatives remain test-only experiments.
The compiler now adopts the preferred join order, with no new addon switches.
The [addon results](sql-anchor-first-results.md) report its effect on total workload CPU and the remaining engine gap.


## Production integration of accepted relational spikes — 2026-10-21

The integration starts from commit `268a1fd3`.
The existing storage, progressive-search, integer-key, and anchor-order changes are already active.
The remaining accepted components are batch-wide acyclic fusion, constant capture removal, stage membership, ancestor anchors, and specialized recursive witnesses.

The engine will select supported relational components automatically, without an execution switch.
Stateful components retain their existing scheduler until a replacement preserves their complete contract and improves measured work.
The generic recursive machine, unconditional guard movement, and blanket JSONB replacement remain rejected experiments.

Production acceptance includes selected roots, file dialects, terminal failures, recursion refusals, and connection reuse after failures.
Complete addon measurements will include compilation, setup, application, and output parity.
Kernel-only measurements remain separate.
Results and any narrowed integration scope will follow in this journal.

### Initial integration observations

The first complete addon smoke run preserves all 33 findings, captures, failures, and blind-file results on the 32-file corpus.
It admits 55 of 141 clauses to one relational statement per batch.
The remaining clauses retain stateful scheduling.
This first run uses the historical two-file batch size and increases total CPU.
It is a rejected timing candidate, not an accepted speedup.

The new profile isolates 1,687 ms outside SQLite execution for the fused operator, versus 860 ms inside SQLite.
The statement contains 363 KB of SQL and executes across 16 batches on four sessions.
Preparation and result handling dominate the apparent matching gain.
The physical planner now uses a smaller expansion budget for small file batches.
The session also declares component schemas before preparing fused SQL, so later clause activation cannot invalidate that preparation.

These are physical planning choices within one engine configuration.
They add no optimizer switch.

Two existing tests rejected every `WITH RECURSIVE` statement.
The execution contract now permits recursive SQL, so those assertions no longer apply.
The plan audit still checks bounded tree access and indexed joins, including the generated relational statements.

### Plan audit and terminal rejoins

The expanded audit explains the actual relational queries, as well as the existing frame statements.
A global materialized `qN` relation is a closed SQL expression.
SQLite can display its primary scan below its first correlated reader.
The audit now distinguishes that one-time materialization from correlation inside the relation's body.
A regression test still rejects a repeated data probe and an unindexed inner join.
The dispatcher's final result scan also has a proved one-row outer input.

Four generated clauses exposed a real inner scan after a native publication boundary.
Their terminal rejoin compares `t.ek` with `f.file * 4294967296 + f.node`.
SQLite can start with terminal answers and scan the complete native frame for each answer.
That arithmetic equality cannot probe the frame's `(file,node)` index by inversion.
The physical generator now keeps that native frame outermost and probes materialized terminal answers by their packed key.
It changes this join only when its input crosses a native boundary.

### Current integration boundary

The real catalogue admits 55 of 141 clauses to relational SQL.
The other clauses still use mutable demand tables or enumeration state.
This includes shared nonrecursive utilities as well as general recursion.
Their statement count still dominates complete calls.
The generic recursive CTE interpreter remains a rejected performance candidate.

The specialized positive recursive workload has a different result.
Complete calls improve from 1,855 ms to 194 ms on 32 files in the first three-run comparison.
On 138 files, they improve from 13,706 ms to 1,183 ms.
All results preserve 1,478 and 5,631 findings respectively, including their captures.
The workspace uses four worker batches, each with one SQL statement.
Those figures include parsing, planning, matching, and rendering.

They do not establish catalogue-wide recursive coverage.

### Compiler ownership and practical limits

Fusion admission uses checkpoints over relation counts, use counts, handles, barriers, and the small publication caches.
A rejected component restores that checkpoint.
It does not copy the accumulated relation bodies before each attempted admission.
A regression executes later accepted components after a rejection and checks that abandoned terminal effects cannot appear.

The engine keeps one physical planning policy.
File batches of at most eight use an expansion budget of four.
Larger batches use sixteen.
Programs with more than 3,000 logical relations can use additional bounded statement phases.
Small programs retain one fused execution per worker batch.

Large terminal fences use balanced sums.
The regression executes 1,100 required effects exactly once in one SQLite statement.
A linear sum would exceed SQLite's expression-depth limit.
Required terminal effects remain observable even when later filters remove every finding.

The positive recursive compiler also counts the initial utility invocation toward the depth limit.
A boundary comparison found an off-by-one error in the first integration guard.
The final guard agrees with native acceptance at that boundary and still ignores deep branches after an earlier witness.
The old SQL scheduler refuses some shallower valid examples because its convergence-pass limit is stricter.
The typed plan accepts those valid examples.

### Measurement method for the integrated engine

The baseline is a clean detached checkout of `268a1fd3`.
Both addons use the same release flags, four workers, `unbounded-work`, and the diagnostic allocator shim.
The driver checks complete serialized outcomes against native, including findings, captures, failures, and blind files.
It reverses variant order between rounds.
The historical catalogue comparison uses two files per SQL batch.
A separate comparison uses the engine's default batch size of 64.

Each fresh process records compilation, setup, application, peak RSS, clause admission, and physical statement counts.
Application CPU includes parsing, planning, matching, and rendering.
Total CPU additionally includes cold compilation and setup.
Profiled timings are separate from these unprofiled comparisons.
The original 818-clause translated-policy workload is outside this measurement scope.

The artifact directory is `/tmp/rerail-engine-integration-20261021`.
It contains frozen addons, complete source manifests, policy and corpus hashes, configurations, outcomes, repeated measurements, and profile traces.
The `pre-checkpoint` directory preserves the preceding candidate's results.

### Adopted components

| Component | Engine integration |
|---|---|
| Shared immutable relations | Eligible clauses share one relational program and persistent prepared statements. |
| Capture constants | Recognized projections omit capture columns until a value must exist. |
| Stage membership | Proved input-only stage bridges use row-value membership. |
| Ancestor anchors | Necessary ancestor guards retain file and candidate-kind restrictions. |
| Native publication | Typed aggregates publish sets without interpreting rules or executing nested SQL. |
| Positive recursive witnesses | Supported fresh-capture subtree utilities use witness segments and a reached-depth check. |
| Profiling | Complete SQL plans and counters remain available, with separate internal stage timings. |
| Input publication | Both selected roots and whole-file memberships use immutable virtual input relations. |
| Earlier accepted work | Frame sharing, progressive search, integer capture-free keys, and anchor-first walks remain active. |

The logical guard-reordering candidate, generic recursive interpreter, and blanket JSONB replacement remain rejected.
The heterogeneous leaf and policy-composition examples remain semantic feasibility experiments.
They do not replace the existing complete policy evaluator.

Per-clause timers cover scheduled components.
Shared relational time belongs to its batch query and internal stages.
The profiler does not divide that shared time among clauses or add stage time to the enclosing SQL budget.

### Admission cost

The first integration copied the whole accumulated relational graph before every attempted clause admission.
That copies SQL bodies even when the next clause contains an unsupported scheduling marker.
Checkpoints now copy only use counts and small publication caches.
Rollback removes new relations, handles, required effects, and interning entries.
The generated query and admitted-clause set remain the same.
This change removes avoidable compiler work without introducing another execution path.

### Selected statement phases

A final paired comparison tests one phase against four on the integrated engine.
It uses two fresh processes per variant and reverses their order.
Both variants preserve complete output parity.

| 32-file configuration | One-phase peak RSS | Four-phase peak RSS | One-phase apply CPU | Four-phase apply CPU |
|---|---:|---:|---:|---:|
| Historical two-file batches | 594 MiB | 485 MiB | 5,427 ms | 5,320 ms |
| Default batch size 64 | 614 MiB | 475 MiB | 5,253 ms | 5,277 ms |

These samples support a memory reduction of about 18 to 23 percent.
Their CPU differences remain small relative to run variation.
They do not establish a general execution speedup from splitting statements.

The selected policy requests four phases above 512 logical relations.
Additional phases can bound larger graphs beyond 3,000 relations per phase.
The generator can still emit one statement where no publication boundary needs a split.
This includes the test with 140 simple rules across multiple files.
The 55-clause catalogue component uses four statements per worker batch.
The specialized recursive component retains one.

The comparison changes physical boundaries without changing matching semantics.
It supports shorter materialization lifetimes as a practical memory control.
It does not isolate which SQLite allocation accounts for every saved byte.
The configuration remains automatic and exposes no phase-selection switch.

### Final complete-call results

The selected engine uses the four-phase policy above.
The following figures supersede the earlier integration timings.
CPU figures are medians of fresh-process application CPU, in milliseconds.
Peak RSS is the maximum across those processes, in MiB.
The catalogue contains 135 policies and 141 Python AST matchers.

| Catalogue configuration | Before CPU | Integrated CPU | CPU change | Before peak RSS | Integrated peak RSS |
|---|---:|---:|---:|---:|---:|
| 32 files, batch size 2, three rounds | 4,722 | 5,322 | +12.7% | 364 | 483 |
| 138 files, batch size 2, three rounds | 27,854 | 28,415 | +2.0% | 558 | 696 |
| 32 files, default batch size 64, two rounds | 4,398 | 5,214 | +18.5% | 359 | 478 |

The general catalogue has no measured speedup.
The 138-file CPU difference is small relative to variation between runs.
Its median wall time increases from 11.72 to 14.05 seconds.
Lower memory than the one-phase candidate still leaves substantially more memory than the original engine.

Cold total CPU also increases.
The respective before/after figures are 4,935/5,634 ms, 28,072/28,770 ms, and 4,606/5,585 ms.
The catalogue preserves all 33 findings on 32 files and all 580 findings on 138 files.
The comparison also checks captures, failures, and blind files.

The generated positive recursive policy exercises the specialized witness compiler on the same source corpora.
It uses batch size 256 and three rounds per variant.

| Files | Native CPU | Before SQL CPU | Integrated SQL CPU | SQL speedup | Before peak RSS | Integrated peak RSS |
|---|---:|---:|---:|---:|---:|---:|
| 32 | 257 | 2,243 | 193 | 11.6× | 160 | 130 |
| 138 | 1,298 | 15,497 | 1,152 | 13.4× | 460 | 279 |

This policy preserves 1,478 and 5,631 findings, including their captures.
Each complete call executes four matching SQL statements, one per worker batch.
The result demonstrates a large improvement for the supported recursive shape.
It does not demonstrate general recursive scheduling or catalogue-wide native performance.

All 34 final timed runs preserve complete native output parity.
The measurements use the diagnostic allocator shim and disabled work limits.
They establish a comparison in this environment, not shipping allocator performance.

### Actual SQL counts and remaining amplification

The production trace compares the clean baseline with the selected engine on 32 files and batch size two.
These are actual SQL executions, rather than the engine's logical plan-step counter.
The latter includes some native frame operations and omits some setup statements.

| Traced SQL category | Before | Integrated |
|---|---:|---:|
| Matching and cleanup | 34,487 | 32,559 |
| Transactions | 32 | 32 |
| Schema and setup | 7,610 | 9,904 |
| Per-file input publication | 305 | 0 |
| Total | 42,434 | 42,495 |

Matching and cleanup executions fall by 5.6 percent.
Additional schema setup offsets that reduction in a cold call.
The total statement count remains effectively unchanged.
The file loader no longer inserts memberships individually into a SQL table.
It publishes an immutable virtual relation once per batch.

SQLite VM instructions increase from 233,296,897 to 245,655,418, or 5.3 percent.
Native frame cursor advances decrease from 11,225,553 to 10,263,036, or 8.6 percent.
Fewer frame visits do not imply less total SQL work.
The integrated profile has no unexpected EXPLAIN failures or incomplete operations.
Thirteen setup explanations fail because their schema objects already exist.

The admitted component covers 55 clauses.
The remaining 86 clauses have these compiler admission reasons:

| Admission reason | Clauses |
|---|---:|
| Column-specific insert or mutable demand | 47 |
| Stateful frontier instruction | 35 |
| Enumeration requires exclusion rounds | 4 |

These categories overlap conceptually with shared utilities and progressive searches.
They do not mean that all 86 clauses require recursion.
The scheduler still executes those components repeatedly across demands and batches.
Their work prevents the complete catalogue from approaching a small number of SQL executions.

The next compiler priority is direct relational lowering of demands, frontiers, and their callers.
That lowering must preserve binding state, relation grain, required effects, and exclusion semantics in the intermediate representation.
The current adapter still inherits frame restoration, row numbering, coercion, and intermediate relations from emitted frame SQL.
Removing statement boundaries alone does not remove that work.

Another hypothesis concerns empty inputs.
The scheduler can skip later operations after an empty result, while a fused query can retain required publication boundaries.
The next profile must isolate empty publications and their consumers before it attributes the regression to this difference.
The current counters establish the regression, but do not identify one exclusive cause.

### Final validation and reproduction

The selected source passes 1,481 code-rule tests, with 44 ignored tests and no failures.
The ordinary build also passes eight targeted integration regressions and library Clippy.
The selected build passes Clippy for the library and tests.
Thirteen profiling and SQL-shape script tests pass.
Boundary and Windows path checks pass.

A final repeat of the unchanged script tests cannot start.
The installed Homebrew Node binaries cannot load `libsimdjson.33.dylib`.
The earlier successful script log remains in `published-script-tests.log`.
Rust formatting and the native source-manifest comparison still pass after the journal update.

The structure check retains two existing failure categories.
They concern benchmark fixture locations and cross-root fixture includes in `policy_guards.rs`.
This integration adds no findings to either category.

The final addon SHA-256 is `37c08d195ebe4d796b5faf32dd3a60e7ce0945b0b9b545db30afd8587f034875`.
Its source manifest matches every native source file in this integration.
The manifest records a dirty checkout based on `268a1fd3`, because the build precedes the integration commit.

The artifact directory contains these final records:

| Record | Purpose |
|---|---|
| `validation.json` | All 34 outcomes, source identity, and final summaries. |
| `selected-comparison-{32,138}.json` | Historical batch-size catalogue comparisons. |
| `default-comparison-32.json` | Default batch-size catalogue comparison. |
| `positive-comparison-{32,138}.json` | Specialized recursive comparisons, including native. |
| `before-build.json`, `after-build.json` | Exact source hashes, build flags, and addon identities. |
| `profile-before-32/`, `profile-adopted-32/` | Complete baseline and selected production profiles. |
| `phase-comparison-{small,default}.json` | The one-phase versus four-phase selection. |

Earlier candidate profiles remain in the directory for the journal history.
`profile-adopted-32` is the final selected profile.
`after.node` is the selected four-phase addon.
The temporary artifact directory is local and is not part of the published branch.

The saved configurations reproduce each comparison through the existing driver:

```sh
node benchmark/sql-set-tuning.mjs /tmp/rerail-engine-integration-20261021/selected-sweep-32.json
node benchmark/sql-set-tuning.mjs /tmp/rerail-engine-integration-20261021/selected-sweep-138.json
node benchmark/sql-set-tuning.mjs /tmp/rerail-engine-integration-20261021/default-sweep-32.json
node benchmark/sql-set-tuning.mjs /tmp/rerail-engine-integration-20261021/positive-sweep-32.json
node benchmark/sql-set-tuning.mjs /tmp/rerail-engine-integration-20261021/positive-sweep-138.json
```

## Hard job budget: establish the failure before further compiler changes — 2026-10-21

The previous integration achieves component fusion, not whole-job fusion.
Its statement count does not meet the requested architecture.
The new acceptance limit is **100 actual SQL executions for one complete cold job**.
That limit covers every worker and includes setup, DDL, transactions, matching, and cleanup.
Changing batch size does not reset the limit.

`benchmark/sql-profile.mjs` now rejects a job above that fixed limit, even when its output matches native.
This is an acceptance test after complete execution.
It does not interrupt matching at execution 101.
Completing the run preserves the full output check and the trace needed to explain the failure.
The gate has no budget override.

The counter uses actual traced SQL bodies.
One `execute_batch` with 200 statements consumes 200 executions.
Repeated executions of one prepared statement each count.
Native publication spans do not duplicate their enclosing SQL execution.
The profiler's own EXPLAIN statements are diagnostics outside the workload budget.

### Coverage correction

The earlier 42,495 count covers AST sessions only.
An audit found separate connections for `select_when` and cross-file policy composition.
Those connections did not participate in the earlier trace.
The previous number is therefore a lower bound for the complete SQL job.

Both auxiliary connection families now install their observer before schema setup.
Their traces use the same process clock and enter the same job total.
Each catalogue records statement starts, completions, outstanding executions, and its connection family.
The acceptance gate checks these counts against the raw trace.
It also checks AST session coverage against the workspace report.

A missing trace, unfinished operation, inconsistent count, or output mismatch prevents acceptance.
Historical traces without the new coverage counters cannot establish a passing result.
Their observed counts can still establish that the job exceeds 100.
The catalogue benchmark uses an in-memory workspace, without content-store connections.

### Required SQL shape and allocation

The compiler must plan the entire job before it allocates worker batches or SQL statement phases.
The following allocation is a design target, not a measured passing implementation.
It assumes four workers and at most one file partition per worker.
Additional partitions must consume the same job allocation.

| Work | Job allocation | Required shape |
|---|---:|---|
| Session configuration | 4 | At most one configuration statement per worker. |
| Matching | 64 | At most 16 statement phases per worker, shared across files and clauses. |
| Policy composition | 16 | Shared relations and tagged outputs, rather than a SQL call per policy and file. |
| Transactions or remaining setup | 8 | Bounded setup independent of rule count. |
| Reserve | 8 | Explicit spare capacity, never a new per-batch allowance. |
| Total | 100 | One allocation for the complete job. |

Frame and terminal registration already use SQLite's C API without SQL schema writes.
The new compiler must extend that property to demand and frontier state.
Their current mutable B-trees require thousands of schema and cleanup statements.
Immutable input relations and typed publication handles can cross statement boundaries without per-row INSERT or SQL DELETE.
Buffer release remains a native storage operation, without rule evaluation or nested SQL.

One program must return tagged results for all clauses in a worker partition.
A phase ends only at a compiler-selected resource boundary.
It must not end because the next rule, utility demand, candidate, or file needs attention.
The physical planner must check its total phase allocation before execution.
A plan outside that allocation must fail acceptance instead of silently using the old scheduler.

The typed intermediate representation needs these contracts before physical SQL generation:

| Logical operation | SQL representation and proof obligation |
|---|---|
| Shared acyclic utility | A relation keyed by unit, file, node, and only the bindings it reads. Separate caller rows restore caller-specific output. |
| Ordered walk | Candidate ordinals and first-event selection inside SQL. Unreached candidates must not expose failures or utility reads. |
| Demand or frontier | Typed key and answer relations inside the program. No host loop that executes SQL for each round. |
| Recursive component | A component-level recursive CTE with proved key transitions, capture state, termination checks, and reached-read semantics. |
| Enumeration | All findings or exclusion state inside a bounded program, preserving ordering and duplicate suppression. |
| Policy composition | Leaf outcomes grouped by policy and file, followed by shared decision SQL. No auxiliary table loader per evaluation. |
| Capture storage | Typed live columns in intermediate relations. Serialize captures only at an output boundary. |

The existing specialized witness compiler proves one recursive shape.
It does not prove arbitrary demanded-key recursion, negative recursive reads, or capture-bearing recursive calls.
Those remaining contracts need executable SQL examples and differential tests before integration.
The previous JSON continuation machine is not a performance solution.
Moving its control loop inside one SQL execution only meets the statement-count dimension.

The compiler must preserve the logical operations instead of recovering them from mutable frame SQL.
Otherwise the adapter inherits the same demand rounds, frame restoration, and cleanup that caused this failure.
Statement count, complete output parity, SQL work, CPU, and memory remain separate checks.
A count below 100 alone does not establish an efficient engine.

### Measured red acceptance cases

The frozen addon contains the coverage changes and the same matching compiler as `507b5ebb`.
Its SHA-256 is `0c62cf7c13551ce52c80835ffaddfbdfed81f29e70f0e84672fc9f187f917e71`.
All three runs use a four-worker pool and compare complete outcomes against native.

| 32-file workload | Setup | Transactions | Matching and cleanup | Job total | Limit | Acceptance exit |
|---|---:|---:|---:|---:|---:|---:|
| Full catalogue, batch size 2 | 9,904 | 32 | 32,559 | 42,495 | 100 | 1 |
| Full catalogue, default batch size 64 | 9,904 | 8 | 13,649 | 23,561 | 100 | 1 |
| Specialized positive recursive policy | 2 | 8 | 4 | 14 | 100 | 0 |

Both catalogue runs preserve all 33 findings.
The control preserves all 1,478 findings.
All captures, failures, and blind files match native.
Each trace has equal start, completion, and observed counts, with no outstanding executions or coverage problems.
Both catalogue runs use four cold AST sessions.
The faster control reuses two sessions across its four worker batches.

The first catalogue run fails with this actual message:

```text
Error: SQL job execution budget FAILED: 42495/100; budget-exceeded.
```

Its execution 101 is already schema setup:

```sql
CREATE TEMP TABLE IF NOT EXISTS c138_d2(id INTEGER PRIMARY KEY, file, node, dirty INTEGER DEFAULT 1);
```

At default batching, execution 101 is an index declaration:

```sql
CREATE INDEX IF NOT EXISTS c138_d7_at ON c138_d7(file, node);
```

The setup count alone exceeds the entire job allocation by almost a factor of 100.
Larger file batches reduce repeated matching calls, but leave that setup count unchanged.
This proves that batch tuning cannot make the current catalogue plan meet the budget.

These catalogue and control workloads do not use the auxiliary connection families.
Their complete counts therefore equal the AST-session counts.
A separate profiled differential fixture exercises both auxiliary families.
It records 75 AST executions, 157 `select_when` executions, and 69 cross-file executions.
Their coverage counters agree with every raw SQL event.
This fixture tests observer coverage, rather than acceptance of the catalogue workload.

The artifact directory is `/tmp/rerail-sql-budget-20261021`.
Each run retains the full profile, native and SQL outcomes, source identities, and `execution-budget.json`.
The `.exit` files record the acceptance process status.
The journal preserves the measured totals because the temporary artifacts are local.

The red case reproduces with this command in the current workspace:

```sh
DYLD_LIBRARY_PATH=/tmp/ner-1562-node-libs RERAIL_BENCH_WORKERS=4 \
  node benchmark/sql-profile.mjs /tmp/rerail-sql-budget-20261021/catalogue-32.json \
  /tmp/rerail-sql-budget-20261021/catalogue-32-red-repeat
```

The library path uses the existing local shim for Homebrew Node's missing `libsimdjson.33.dylib`.
It does not change the SQL addon.
The profile runner requires a fresh output directory and returns exit code 1 for this workload.

The gate tests pass at exactly 100 executions and fail at 101 across multiple workers.
They also reject missing coverage, unfinished operations, mismatched outcomes, and a budget override in workload configuration.
The Rust observer regression proves that cached executions and individual statements within `execute_batch` count separately.
The profiling and SQL-shape scripts pass all 16 tests.
The complete code-rule suite passes 1,481 tests, with 44 ignored and no failures.
Both ordinary and profiling builds pass library Clippy.

The current compiler remains a failing implementation under this acceptance test.
No SQL generation change in this step claims to meet the 100-execution requirement.

## Compile the scheduler into SQL — 2026-10-21

The next implementation changes plan execution itself.
Eligible immutable components retain their shared relation graph.
The remaining plan instructions now compile into materialized set producers and a recursive SQL control relation.
This includes utility fixed points, candidate frontiers, recursion checks, and enumeration rounds.
The batch driver executes the resulting programs, rather than their individual instructions.

Native callbacks only publish, append, replace, or clear relation buffers.
They do not receive an AST operator, interpret a plan, or execute nested SQL.
The existing reached-read graph check remains a structural validator over an aggregate's rows.
SQL contains the loop conditions, candidate decisions, ordering, and enumeration transitions.

Mutable demand tables also become native relation registrations.
Their DML becomes SELECT producers and set publication boundaries.
The schema expansion distinguishes omitted defaults from explicit NULL values.
Candidate prefixes use SQL ranks and per-parent limits instead of a native frontier scheduler.

SQLite does not automatically refresh an independent materialized CTE inside a correlated recursive step.
A small experiment repeated a stale value through five iterations.
The compiler therefore gives every mutable input an explicit dependency on the current SQL iteration.
A regression must demonstrate fresh results across a recursive back-edge.

The physical generator uses balanced dispatch expressions instead of a linear CASE over thousands of instructions.
It bounds the number of matching programs per worker and reserves job capacity for setup and policy composition.
File partitioning uses the complete job and worker count, rather than multiplying programs through small file batches.
The budget test and full native output comparison remain acceptance requirements.

This construction initially preserves the former operator granularity inside SQL.
It is not evidence that all intermediate work disappears.
SQL instructions, frame visits, CPU, and memory still need complete measurements after statement-count acceptance.
Validation results and any rejected implementation will follow here.

### SQL shape corrections

The first recursive prototype did not meet the performance requirements.
Independent materialized CTEs kept stale values across recursive iterations.
Adding iteration dependencies fixed freshness but caused repeated membership scans and repeated CTE construction inside data-row loops.
These candidates timed out and remain rejected experiments.

The compiler now publishes shared producers before their consumers.
This includes nested CTEs and closed membership queries.
Consumers use indexed probes into those published sets.
The membership transformation preserves NULL values, empty sets, and negation.
Terminal demand lists retain their batched virtual-table contract.

Shared utility bodies compile once and use a SQL return stack.
Textual expansion of every call site caused excessive compilation and failed the utility-chain test.
SQL stage markers prevent repeated publication during enumeration.
Enumeration counters now live in the SQL control row.
Fresh-witness checks probe both root columns through EXISTS.

Stage membership lists remain stable after their guarded publication.
They can stay cached across control iterations within one SQL execution.
Mutable relations retain explicit iteration dependencies.
The compiler also emits SQL guards from the existing emptiness proofs.
Empty demand sets therefore avoid their producers and full-table update copies.

Temporary relations reuse compatible schemas after their last consumer.
This reduced the stateful compiler's registered schemas from 10,060 to 3,922 on the catalogue workload.
The publication aggregate owns producer values before replacement.
This permits streamed SQLite producers without a second materialized copy.
Schema inference also stopped copying the complete schema catalogue for each CTE.

### Profiling and interpretation

Every executed program retains its complete query plan and bytecode.
Internal set-step records expose producer SQL, control transitions, CPU, wall time, and incomplete steps.
These times belong to their enclosing SQL execution.
They must not enter the execution budget or the CPU total a second time.
The summary treats whole programs as programs instead of assigning their entire cost to an embedded operator keyword.

The first step logger used an unbuffered file writer.
Its many small writes inflated the instrumented candidate times.
Those times do not establish an unprofiled speedup from temporary relation reuse.
The final logger buffers its records and flushes at each completed program boundary.
Unprofiled comparisons remain the performance evidence.

Large EXPLAIN catalogues exceeded JavaScript's maximum string length during report generation.
The reporter now writes complete records in chunks.
It stores bytecode once per physical plan and groups variants that differ only in connection addresses.
Every grouped variant retains references to all original raw captures.
HTML keeps plan data in separate embedded records and parses a query's plans on demand.

The plan audit now examines actual compiled programs.
It distinguishes control-row correlation and batched terminal demand from correlation over data rows.
This exposed repeated witness membership scans and enumeration counter reads.
The complete audit passes after the SQL shape corrections.

Internal step records also identify their source clause or shared stage.
Clause records include the rule and clause names from the compiled catalogue.
The summary therefore links a costly producer to a policy, its SQL body, and its next control transition.
Shared utility bodies retain their owning clause or stage attribution.

### Complete-job acceptance results

The final engine passes the unchanged 100-execution acceptance gate on all three catalogue runs.
Each run compiles all 141 matchers from the same 135 policies as the failing baseline.
The three pre-existing policy exclusions remain identical between engines.
The complete native outcome digests match, with no failures or blind files.

| Files | Compiler batch hint | Previous executions | New executions | Findings |
|---:|---:|---:|---:|---:|
| 32 | 2 | 42,495 | 72 | 33 |
| 32 | 64 | 23,561 | 72 | 33 |
| 138 | 2 | Not traced | 72 | 580 |

Each new job contains four setup statements, eight transaction statements, and 60 matching statements.
Four immutable programs and eleven stateful programs run on each of four whole-job partitions.
The statement count stays constant across these file counts and compiler hints.
No enumeration round, utility iteration, or candidate frontier launches another SQL statement.

The caller executes the physical programs produced by compilation.
Every clause belongs to the immutable relation graph or the stateful programs.
A coverage assertion rejects an uncovered clause during compilation.
Native storage callbacks execute no nested SQL and interpret no rule operators.

The compact evidence is `benchmark/sql-large-plans-evidence.json`.
It records workload hashes, outcome digests, execution coverage, build settings, and measured CPU and memory.
Complete query plans, bytecode, traces, and interactive reports remain under `/tmp/rerail-sql-large-20261021/`.
The final captures use the `catalogue-attributed-*-profile` directories.
The build manifest is `build-attributed.json` in the same directory.

### Performance remains open

The execution budget now passes, but cold CPU and memory regress against commit `c109a915`.
These are single unprofiled diagnostic samples under shared machine load.
Both builds use the same diagnostic allocator override and four workers.

| Files | Hint | Previous apply CPU, seconds | New apply CPU, seconds | Previous peak RSS, GB | New peak RSS, GB |
|---:|---:|---:|---:|---:|---:|
| 32 | 64 | 5.09 | 14.86 | 0.49 | 1.39 |
| 138 | 2 | 29.02 | 51.74 | 0.73 | 2.76 |

The stateful SQL totals about 11.5 MB before SQLite prepares its programs.
Preparation and handling are the largest recorded bucket on the 32-file workload.
SQL execution dominates on the 138-file workload.
The larger programs still publish many intermediate frames and retain substantial prepared-statement memory.
The next performance work must compact straight-line regions and reduce intermediate frame copies.
The execution-budget result does not close the performance gap.

The complete code-rule suite passes 1,488 tests, including generated differential cases and the actual-program plan audit.
The suite retains its 44 existing ignored tests.
Seven compiler regressions cover iteration freshness, defaults, mutation predicates, NULL membership, indexed probes, and temporary schema reuse.
The profiling and query-shape scripts pass 18 tests.
Library Clippy passes with and without `unbounded-work`.
Formatting and whitespace checks pass.

## Profile-guided SQL generation fixes — 2026-10-21

The baseline for this investigation is commit `5f1960f2`.
Its complete profiles identify preparation and intermediate relation publication as separate costs.
The 32-file capture records 441,117,364 VM instructions and 9,970,270 published frame rows.
The 138-file capture shifts more CPU into SQL execution.
The workloads, exclusions, worker count, allocator override, and native outcome oracle remain unchanged.

### Preserve access paths and remove unnecessary copies

The general membership rewrite destroys an existing batched access path.
It converts immutable file demands from `IN` into correlated `EXISTS` predicates.
The node virtual table then loses its all-at-once file constraint.
The compiler now preserves membership queries that read only immutable workspace demands.
Mutable demands still refresh on every SQL control iteration.

The rewrite also copies primary-key sets before it considers direct indexed probes.
The compiler now resolves proven non-null key probes before general nullable membership lowering.
A composite primary key alone does not prove that its columns are non-null.
The NULL, empty-set, and negation regression now includes nullable composite keys.
The proof also excludes keys that an outer join can extend with NULL.
A separate regression checks nullable keys on both sides of membership.

Single-use CTEs at their consumer's first row source now remain relational.
The compiler still publishes shared producers and producers beneath per-row predicates.
It also resolves nested `WITH` scopes before it considers this transformation.
These restrictions preserve iteration freshness and prevent repeated materialization inside data-row loops.

The first inlining candidate incorrectly extracts membership from a nested CTE scope.
Its profile fails closed at 28 executions because the job is incomplete.
Thirteen existing tests also reject that candidate with `no such table: u`.
The corrected transformation passes those tests and a new nested-scope regression.
The corrected 32-file job passes the complete 72-execution gate with native parity.

Publication aggregates now use bounded row-aligned chunks instead of one allocation per row.
Duplicate suppression reads only key columns from existing rows.
It no longer clones complete capture rows to encode a primary key.
The SQL producer still determines every row; storage callbacks execute no SQL or rule predicates.

Together, these changes reduce stateful SQL from 11,466,112 to 10,404,379 bytes.
They reduce registered stateful relations from 3,922 to 3,746.
The corrected intermediate capture records 359,884,105 VM instructions and 8,841,123 published frame rows.
Its unprofiled 32-file apply uses 12.64 CPU seconds, compared with the baseline's 14.86 seconds.

### Identify preparation directly

Operator-exclusive CPU previously combines preparation, result handling, and profiling overhead.
The profiler now measures preparation and cache lookup separately within that span.
It also records prepared-statement memory before execution.
Set-step traces include published rows, frame advances, column reads, insertions, and index costs.
These subcosts do not create SQL executions or add CPU twice to the summary.

The corrected intermediate capture measures 7.24 CPU seconds in preparation across 60 submissions.
Its largest prepared statement occupies 17.84 MiB before execution.
A native sample identifies SQLite expression comparison and constant handling during preparation.
The generated instruction dispatcher contributes thousands of distinct comparison constants per program.
SQLite's constant factoring searches a linear expression list, which amplifies preparation work.

The dispatcher now tests PC bits and uses eight reusable leaf labels.
Its masks are powers of two, so dispatch no longer creates a distinct comparison constant for every instruction.
Sparse branches retain the full program address range and cannot alias higher PCs onto lower leaf labels.
A regression checks every PC in a 16,384-address sparse program.
The first unprofiled 32-file sample uses 9.84 CPU seconds with complete native parity.

The isolated dispatch capture measures 4.37 CPU seconds in preparation, compared with 7.24 seconds before that change.
SQL execution remains approximately 5.5 CPU seconds in both captures.
The bit dispatcher executes slightly more VM instructions, but avoids substantial work during preparation.
This comparison isolates preparation savings from relational execution savings.

### Final verification and remaining work

The final build passes 1,494 code-rule tests, including generated differential cases and the actual-program plan audit.
It retains the 44 existing ignored tests.
The profiling and query-shape scripts pass 19 tests.
Library Clippy passes with and without `unbounded-work`.
The frozen build and its source hashes are recorded in `/tmp/rerail-sql-tuning-20261021/build-chunks.json`.

Both final complete-job captures pass at 72 actual executions with complete native outcome parity.
Each capture includes all four sessions, setup, transactions, matching, plans, and bytecode.
The captures are `chunks-32-profile` and `chunks-138-profile` under the same artifact directory.
The native outcomes remain 33 and 580 violations, with no failures or blind files.

The larger job still publishes about 58.8 million frame rows and executes about 2.38 billion VM instructions.
Its baseline publishes about 67.0 million rows and executes about 2.86 billion instructions.
B023's recursive demand insertion and expansion remain the hottest individual set steps.
Their SQL repeats canonical node-to-atom lookups inside demand joins and anti-joins.
The total cost also includes many smaller publications across other clauses.

The remaining compiler work is broader than changing join syntax.
Straight-line operators must remain relational producers across operator boundaries, with publication reserved for recursion and shared reuse.
Capture columns should survive through selection views or late joins instead of repeated value-buffer copies.
Canonical demand keys should be projected once before joins and duplicate suppression.
These are proposed next changes; the current fixes do not claim those reductions.
Peak process memory remains high, so lower CPU does not close the memory regression against `c109a915`.


### Reject unbounded publication buffers

The first publication-buffer spike uses one growing value vector.
Three cold runs confirm CPU improvements but expose a memory regression on the larger workload.
Its median peak RSS rises from 2.78 GB to 3.27 GB.
This candidate is rejected despite its CPU improvement.

The selected buffer uses row-aligned chunks with at most 4,096 values per ordinary chunk.
A row wider than that limit receives its own chunk.
Unused capacity is bounded independently of the relation's total size.
A regression crosses several chunk boundaries and checks generated rowids, NULL values, and ignored duplicate keys.

Both chunk-buffer acceptance captures pass, and all repeated measurements preserve complete native outcomes.
Three cold runs per build and cohort give these medians:

| Files | Previous apply CPU, seconds | New apply CPU, seconds | CPU reduction | Previous peak RSS, GB | New peak RSS, GB |
|---:|---:|---:|---:|---:|---:|
| 32 | 14.75 | 9.78 | 33.7% | 1.43 | 1.28 |
| 138 | 52.08 | 41.43 | 20.5% | 2.78 | 2.52 |

Each sample uses a new Node process and four workers.
The baseline samples alternate with the rejected flat-buffer candidate; the final chunk-buffer samples run subsequently.
All builds use the same diagnostic allocator override.
Host load varies, so these comparisons use CPU rather than wall time.
The evidence retains every wall-time, CPU, memory, and outcome sample.

The final stateful SQL totals 10,954,255 bytes across eleven programs, with 3,746 registered relations.
The final 32-file capture executes 364,668,440 VM instructions and publishes 8,841,123 frame rows.
The larger capture executes 2,384,203,863 instructions and publishes 58,799,718 rows.
Frame index construction costs 0.42 CPU seconds within 36.07 SQL CPU seconds on the larger capture.
This measured subcost does not account for the remaining runtime by itself.

The compact evidence is `benchmark/sql-generation-tuning-evidence.json`.
It records the source fingerprint, frozen addon hash, complete acceptance gates, intermediate failures, isolated dispatcher comparison, and repeated samples.
The final addon hash is `2a06576473d6517d6fc56cfca2cf49f948483b8023a0b20ee49577215f0d6df5`.
The evidence identifies proposed further changes separately from implemented changes.

## Frame publication and VM instruction spikes — 2026-10-21

Commit `42ab496a` contains the previous preparation and publication fixes.
This investigation uses that published commit as its baseline.
The same 135-policy workload contains 141 matchers across 32 or 138 files.
The existing three policy exclusions remain unchanged.
All candidates use four workers and the same diagnostic allocator override.

### Replace whole-frame updates with column deltas

The 138-file baseline publishes 58,799,718 frame rows.
Whole-frame `UPDATE` lowering accounts for approximately 10.6 million rows inside stateful programs.
That lowering selects every column and replaces the complete table to change a few flags.
It also republishes rows that do not satisfy the update predicate.

The new lowering selects the integer primary key and assigned values for matching rows.
SQL evaluates the predicate, subqueries, and assignments against the original table snapshot.
A publication aggregate buffers bounded chunks until its producer reaches EOF.
The storage callback then locates keys and changes only assigned columns.
The callback executes no SQL and evaluates no rule predicates.

When an update preserves its integer primary key, the compiler selects this shape automatically.
Other updates retain the general replacement shape.
This is a physical planning decision, with no new execution option.
Column updates preserve NULLs, type changes, shared-page isolation, and unaffected lookup indexes.
Affected indexes are invalidated before the next consumer.

Profiling now records updated cells separately from inserted frame rows.
The text summary includes both totals and shows updated cells for each hot set step.
Updates do not count as complete row publications.
Older captures remain readable without the new counter.

The isolated update capture passes native parity and the complete 72-execution gate.
It records 49,307,027 published rows and 6,066,384 updated cells.
That removes 9,492,691 complete row publications, or 16.1% of the baseline.
Column reads decrease from 443,532,144 to 411,358,263.
VM instructions decrease from 2,384,203,863 to 2,339,149,830.
This change removes copying, but leaves most relational work intact.

### Reject wider pipeline substitution

A separate candidate raises the fusion bounds from four frames to 32 frames.
It also admits anti-join consumers and publication into shared stage tables.
The candidate preserves native outcomes and passes its semantic tests.
It preserves empty-probe semantics by leaving anti-join emptiness decisions in SQL.

The 138-file capture executes 2,335,871,590 VM instructions.
That saves less than 0.2% against the update-only candidate.
Published rows increase to 49,319,113, and preparation increases from 3.94 to 4.21 CPU seconds.
The wider substitution changes materialization and register allocation without removing the dominant recursive publications.
The final compiler retains its original fusion bounds.
The rejected patch remains in `/tmp/rerail-frame-publications-20261021/pipeline-experiment.patch`.

### Replace repeated scalar atom lookups with joins

The hottest B023 demand statements repeat canonical node-to-atom subqueries.
They use those values in demand insertion, duplicate checks, dependency reads, and answer joins.
The new emission joins each canonical capture to `node` once per input row.
The expression uses the joined atom and retains the occurrence value as its fallback.
A left join preserves NULLs and missing node rows.
This shape adds no frame publications.

The isolated comparison uses the wider-fusion candidate on both sides.
It decreases VM instructions by 40,492,741, from 2,335,871,590 to 2,295,378,849.
It also removes 5,352,154 frame column reads.
EXPLAIN reports `canonical_s1 VIRTUAL TABLE INDEX 0:f=i= LEFT-JOIN`.
The lookup has both file and node constraints.
The final candidate combines these joins with column deltas and the original fusion bounds.

The update shape changes from a complete replacement to a selected column patch:

```sql
-- Previous shape: copy every column of every row.
SELECT p_replace_d(driver.tick,p.id,p.file,p.node,p.c0,p.dirty)
FROM (
  SELECT id,file,node,c0,
         CASE WHEN predicate THEN 0 ELSE dirty END AS dirty
  FROM d
) p;

-- New shape: select keys and changed values, then apply the completed batch.
SELECT p_patch_d(driver.tick,p.id,4,0)
FROM d p
WHERE predicate;
```

The canonical demand shape uses a shared point lookup within each SQL producer:

```sql
SELECT DISTINCT f.file,f.node,COALESCE(a.atom,f.s1)
FROM input f
LEFT JOIN node a ON a.file=f.file AND a.id=f.s1
WHERE NOT EXISTS (
  SELECT 1 FROM demand d
  WHERE d.file=f.file AND d.node=f.node
    AND d.c0 IS COALESCE(a.atom,f.s1)
);
```

The join supplies the canonical value to both projection and duplicate suppression.
A separate SQL producer still repeats that lookup for its own input.
These changes do not share the value across every recursive operation.

### Final validation

Both complete final captures pass native parity at 72 actual SQL executions.
The execution count includes setup, transactions, matching, and cleanup across all four workers.
The captures contain no unexpected EXPLAIN failures or incomplete operations.
The native outcomes remain 33 and 580 violations, with no failures or blind files.

| Metric | 32-file baseline | 32-file final | 138-file baseline | 138-file final |
|---|---:|---:|---:|---:|
| Published frame rows | 8,841,123 | 7,625,537 | 58,799,718 | 49,307,027 |
| Updated cells | 0 | 976,921 | 0 | 6,066,384 |
| VM instructions | 364,668,440 | 355,316,964 | 2,384,203,863 | 2,298,657,053 |
| Frame column reads | 64,474,036 | 60,137,439 | 443,532,144 | 406,006,109 |
| Frame advances | 15,061,704 | 15,061,704 | 104,565,549 | 104,565,549 |
| Actual SQL executions | 72 | 72 | 72 | 72 |

The new tests compare updates against ordinary SQLite tables.
They include simultaneous assignments, aggregate subqueries, NULLs, type changes, missing matches, and cached indexes.
A separate test checks column updates through shared selection pages.
It confirms that updates do not change another view or alter live row accounting.

The final suite passes 1,496 code-rule tests, including generated differential cases and the actual-program plan audit.
It retains the 44 existing ignored tests.
The profiling and query-shape scripts pass 20 tests.
Library Clippy passes with and without `unbounded-work`, with warnings denied.
Formatting and whitespace checks pass.

The frozen final addon is `/tmp/rerail-frame-publications-20261021/engine-verified.node`.
Its source hashes are in `build-verified.json` in the same directory.
The complete captures are `verified-32-profile` and `verified-138-profile`.
The addon SHA256 is `36074c14527a8739e6f78f5601ecaa9c5456ece1ae67dbacbb2d413ef8ead61e`.

### Cold performance and remaining work

Three fresh-process samples cover each variant and workload.
The order reverses on the second round.
Every sample matches the complete frozen native outcomes and expected digest.
Input and addon hashes remain stable during each sample.
The stored profile fingerprints also match the current inputs.

| Files | Baseline CPU seconds | Update-only CPU seconds | Final CPU seconds | Final reduction |
|---|---:|---:|---:|---:|
| 32 | 9.84 | 9.60 | 9.53 | 3.2% |
| 138 | 38.75 | 37.45 | 38.02 | 1.9% |

The CPU improvement is modest despite the stable publication and instruction reductions.
The shared host also introduces variation between rounds.
The isolated joins reduce VM instructions, but their CPU effect is unresolved against the update-only candidate.
Two paired rounds favor the joins.
The median across all three rounds favors update-only emission.
The final compiler retains the joins because they remove repeated scalar evaluation without additional publications.

Median peak process RSS decreases from 1,328,070,656 to 1,244,413,952 bytes for 32 files.
For 138 files, it changes from 2,463,137,792 to 2,469,363,712 bytes.
The larger workload has no demonstrated memory improvement.

Frame advances remain unchanged at 15.1 million and 104.6 million.
The engine still traverses the same intermediate rows and performs the same recursive scheduling.
The accepted changes remove copies and repeated scalar evaluation within that traversal.
They do not remove the traversal itself.

B023 demand insertion and dependency publication remain the hottest individual set steps.
Ranked frontier tables still carry complete capture rows.
The next compiler change must reduce intermediate row traversal and carry keys through frontier operations.
Capture values can remain in their original relation until a consumer needs them.
Raising substitution limits alone does not produce that shape.

The checked-in evidence is `benchmark/sql-frame-publication-evidence.json`.
It records paired samples, build hashes, complete-job acceptance, rejected fusion, and the remaining limitations.

## Eliminate predicate and frontier traversal — 2026-10-21

The baseline for this investigation is commit `eb9b0d4e`.
The earlier update changes leave 104,565,549 frame advances on the 138-file workload.
This investigation targets unnecessary intermediate rows and repeated traversal inside SQL.
The complete-job limit remains 100 actual SQL executions.

### Re-read ast-index SQL generation

The review uses `neralabs/ast-index-spike` at commit `9449260fea46fa81358fc638bf95a1566d475e27`.
The local checkout is `/tmp/rerail-ast-index-frontier-20261021`.
The relevant sources are `bench/sql_rec.py`, `bench/plan_stmt.py`, and `DESIGN.md`.

`_subtree_ctes` collapses each branch to its parent keys and required captures before the upward join.
`_branch_semijoin_ok` replaces capture-free child products with existence probes.
`_probe_semi` selects SQLite probe shapes from actual measurements.
`_mat` distinguishes closed shared relations from correlated readers.
The planner also uses corpus cardinalities to choose rare anchors.

The current Rerail driver violates the intended shape at many operator boundaries.
It preserves broad capture frames through total Boolean predicates and structural existence tests.
Those predicates need only a truth value per input key.
Its frontier also scans ranked candidates outside the selected prefix.
A low execution count does not bound that work.

### Reject correlated predicate CTEs

The first candidate lowers capture-free Boolean regions into CTEs at `(key,file,node)` grain.
Only its final join restores capture columns.
However, mutable inputs depend on the recursive driver's iteration token.
SQLite treats the resulting CTEs as correlated and repeats their materialization.

One completed predicate step records 3,003 frame scans and 12,255,243 frame advances.
That single step spends 4.76 CPU seconds and publishes no rows.
Its source is clause 36 in the 32-file catalog.
The full run exceeds its 180-second diagnostic timeout.
It has no complete native parity or execution-budget acceptance.
This candidate fails and remains excluded from the engine.

The failed source and patch remain under `/tmp/rerail-key-regions-20261021`.
Their names are `correlated-predicates.rs` and `correlated-regions.patch`.
This failure demonstrates why a procedural scheduler inside SQL cannot rely on ordinary closed-CTE caching.

### Compile total predicates directly

The next candidate lowers capture-free kinds, regexes, negations, conjunctions, and alternatives into Boolean SQL.
Explicit `CASE` expressions preserve short-circuit domains.
The regex intrinsic evaluates one node's text with the existing compiled Rust regex.
It implements no relation, capture binding, recursive utility, or rule operator.
This follows ast-index's leaf-regex UDF boundary.

The compiler also admits total `has` and `inside` predicates with fixed stops.
Those predicates become bounded existence probes over the node table.
A count without an output binding becomes bounded existence probes at its lower and upper thresholds.
Capture bindings, facts, recursive utilities, and unknown-producing stop rules remain barriers.

A complete total predicate can select roots directly from the node table.
The compiler then omits the broad root frame and the subsequent predicate frames.
A predicate inside a larger program filters its input and carries captures only once.
This is one automatic physical choice, with no execution option.

Regex requests, node advances, node column reads, and node filters receive separate profiler counters.
Those counters expose leaf work and structural probes that frame counters do not measure.
A lower frame count alone cannot establish a lower total work count.

### Seek frontier prefixes by rank

Ranked frontier storage now contains only `(position,key,parent,order)`.
A global rank gives each parent's candidates a contiguous interval.
Parent state stores the interval start and its current prefix length.
The generated SQL drives from parent state into a bounded rank range.
A point join restores captures from the original candidate relation.

The storage virtual table now exposes primary-key range constraints to SQLite.
It seeks to the lower and upper bounds instead of scanning a whole parent domain.
The lookup implements buffer indexing and executes no SQL or AST operator.
A regression requires an eight-row interval to advance exactly eight rows from a 10,000-row frame.
Additional cases compare range affinity and boundary behavior with an ordinary SQLite table.

### Compose nonbinding facts without a verdict frame

A later conjunction term previously creates a full verdict frame for its fact answers.
Another full frame then combines those answers with the conjunction's unknown flag and capture mask.
The new emitter combines that metadata in the fact's final join.
It retains the batched fact-answer relation and its original demanded inputs.
It omits the intermediate full-width verdict frame.

The transformation admits facts that bind no captures.
The existing withholding analysis still determines whether a term can run.
The SQL join carries incoming captures unchanged and combines unknown flags directly.
This preserves fallible terminal ordering and avoids broader eager fact evaluation.

### Start recursive answer joins from their witnesses

The next profile exposes a separate join-direction error.
One B023 answer-publication step scans 391,497 caller rows and publishes six results per run.
Its `CROSS JOIN` order starts from callers, then probes demand and answer tables.
A canonical capture lookup also runs for each caller.

The candidate starts from existing answers and recovers each demand row by its integer primary key.
It then probes callers by `(file,node)` and checks capture keys.
Canonical capture lookups occur only for those candidate callers.
Demand collection and recursive scheduling retain their original order.
Unknown answers and capture restoration retain their original expressions.

A storage regression uses 10,000 callers and one answer.
The original join advances at least 10,000 rows.
The reverse join advances three rows: one answer, one demand, and one caller.
Its first caller-index construction still reads 10,000 rows and has a separate counter.
This result does not establish a whole-job improvement.

The complete 138-file profile confirms this sparse B023 hotspot.
Frame advances decrease from 783,006 to 194 across two runs.
Node advances decrease from 391,497 to 180.
Frame column reads decrease from 3,915,054 to 1,355.

Caller-index construction increases from 288,470 to 391,497 indexed rows.
Step CPU decreases from 314.2 ms to 28.0 ms under instrumentation.
Those timings include index construction.

This is an elevenfold local CPU improvement.
The cursor ratio exceeds 4,000, but index construction prevents that ratio from describing total physical work.
The complete engine still allocates broad upstream demand and capture relations.
The answer join fixes a costly consumer of those relations.
It does not eliminate their production.

Standalone root regexes retain the existing batched text scanner.
Boolean regions use the direct regex intrinsic to preserve their short-circuit probe domains.
Regex-request counters count requested node tests, not physical regex-engine invocations.
The batched scanner can amortize many requests through one file scan.

### Remaining compiler frontier

The current engine still encodes many operator boundaries as mutable frame publications inside one recursive SQL driver.
The execution limit controls submissions, not this internal work.
A large statement can still behave like a procedural interpreter.
The observed improvement does not establish a hundredfold reduction for the complete job.

The next compiler representation needs explicit keys, demanded inputs, required captures, and observable terminal effects for each relation.
A capture-free branch needs a key set or an existence probe.
A capture-bearing branch needs only its output captures before the parent join.
Recursive units need separate demand, witness, and answer relations.
A consumer must not force full caller traversal merely to find a few answers.

The desired shape resembles this dependency graph:

```sql
WITH RECURSIVE
anchors AS MATERIALIZED (
  SELECT file, id FROM node WHERE kind = :selective_kind
),
demand(file, node) AS (
  SELECT file, id FROM anchors
  UNION
  SELECT child.file, child.id
  FROM demand parent JOIN node child
    ON child.file = parent.file AND child.parent = parent.node
  WHERE child.kind IN (:required_kinds)
),
witness AS MATERIALIZED (
  SELECT demand.file, demand.node
  FROM demand
  WHERE EXISTS (SELECT 1 FROM eligible_leaf WHERE eligible_leaf.node = demand.node)
)
SELECT caller.key, needed_capture
FROM witness
JOIN caller ON caller.file = witness.file AND caller.node = witness.node;
```

This graph illustrates relation grain and join direction.
It is not a general replacement for capture-sensitive recursion, unknown propagation, or fallible leaves.
Each recursive dependency needs an explicit proof that its demand expansion preserves terminal reachability and capture semantics.
Mutually recursive branches also need SQLite-compatible recursive terms or separate closed phases.

The highest-value restriction belongs before demand insertion and candidate-frame publication.
Applying a selective predicate after publication still pays for the broad relation.
Existing structural root guards run only on two-valued clauses.
Extending those guards to three-valued clauses requires proofs about unknown results, stop rules, and observable leaf failures.
A kind restriction alone does not supply those proofs.

Closed relational regions also need a boundary independent of the driver's iteration token.
Otherwise, SQLite can rematerialize them for every recursive-driver row, as the rejected candidate demonstrates.
Increasing ordinary CTE substitution limits does not establish that boundary.
These representation changes remain necessary after the measured fixes in this entry.

### Whole-job counters and acceptance

The final frozen addon is `engine-final.node` under `/tmp/rerail-key-regions-20261021`.
Its SHA-256 is `9d5dbc1d4e773bc71422a01a90ae15f3c6d85a706e9fb5eaea6335689d52b571`.
The source manifest is `build-final.json` in that directory.
Every Rust source hash matches the manifest after compilation and profiling.

Both workloads use 135 Python policies, 141 grammar matchers, four workers, and the existing three policy exclusions.
The workloads contain 32 files with 1,318,099 bytes and 138 files with 7,336,001 bytes.
Compiler partition hints remain 64 and 2, respectively.
No acceptance scope changes or additional exclusions apply.

Both jobs execute 72 SQL statements across all workers.
That total includes four setup statements, eight transaction statements, and 60 matching or cleanup statements.
Both profiles have complete native outcome parity, zero failures, and zero blind files.
The smaller workload retains 33 findings.
The larger workload retains 580 findings.

| Counter | 32-file baseline | 32-file final | 138-file baseline | 138-file final |
|---|---:|---:|---:|---:|
| VM instructions | 355,316,964 | 321,074,810 | 2,298,657,053 | 2,060,142,394 |
| Frame advances | 15,061,704 | 12,640,543 | 104,565,549 | 86,522,912 |
| Frame column reads | 60,137,439 | 53,415,976 | 406,006,109 | 358,804,728 |
| Published rows | 7,625,537 | 6,679,446 | 49,307,027 | 43,784,220 |
| Rows read for frame indexes | 3,487,365 | 3,000,202 | 26,269,866 | 23,333,904 |
| Node advances | 3,813,038 | 4,127,907 | 25,121,199 | 26,315,247 |
| Node filters | 5,862,614 | 5,858,529 | 37,974,972 | 37,278,636 |
| Regex node requests | 311,603 | 396,656 | 1,809,348 | 2,207,076 |

Frame advances decrease by 17.3 percent on 138 files.
Frame and node advances together decrease by 13.0 percent.
Regex requests increase by 22.0 percent.
The counters measure different operations and do not form a complete count of physical instructions.
These results establish an incremental reduction, not a hundredfold whole-job improvement.

The baseline counter addon adds only instrumentation to `eb9b0d4e`.
Its VM and original frame counters exactly match the previous frozen baseline.
Its patch is `baseline-instrumentation.patch` in the evidence directory.
This preserves a comparable baseline for the new node and regex counters.

The final DSL suite passes 1,499 tests, with 44 existing ignored tests and 112 filtered tests.
The script suite passes 21 tests.
Clippy passes with and without `unbounded-work`, with warnings denied.
The generated-plan audit remains unchanged.

### Paired cold measurements

Each variant runs in a fresh Node process without SQL tracing.
Three repetitions alternate baseline and final builds, with reversed order in the middle repetition.
Native tests, native compilation, and profiled workloads finish before these samples.
The host remains busy with unrelated work.
These are diagnostic CPU measurements with the same allocator override, not production latency measurements.

| Workload | Baseline median apply CPU | Final median apply CPU | CPU reduction |
|---|---:|---:|---:|
| 32 files | 8,819.075 ms | 8,440.403 ms | 4.3% |
| 138 files | 37,631.671 ms | 33,299.889 ms | 11.5% |

Median peak process RSS decreases from 1,299,546,112 to 1,202,257,920 bytes on 32 files.
It decreases from 2,550,841,344 to 2,357,133,312 bytes on 138 files.
Every repeated output matches the complete native oracle.
Input, policy, capabilities, and addon fingerprints remain stable during each sample.

The checked-in evidence is `benchmark/sql-frontier-shape-evidence.json`.
It includes exact counters, repeated CPU samples, source hashes, complete-job acceptance, the sparse B023 SQL, and the rejected correlated candidate.
Raw profiles and frozen binaries remain under `/tmp/rerail-key-regions-20261021`.
The workload metadata records a corrected informational manifest pointer.
The runner ignores that pointer, and independent source and binary hashes establish the build identity.

The accepted changes reduce the measured job cost.
They do not yet give the compiler ast-index's compact relation shape throughout the program.
The remaining 43.8 million publications justify further work on broad demand production and mutable operator boundaries.

## Relational architecture review — 2026-10-22

The user requested an architectural review before further engine optimization.
The review is [SQL relational architecture](sql-relational-architecture-review.md).
This entry records documentation work only.
It introduces no engine changes or new benchmark results.

The review compares these exact sources:

- Rerail `9207ff453e312f762c8347c67a60ee7665bf8070` and its checked-in frontier evidence.
- ast-index-spike `9449260fea46fa81358fc638bf95a1566d475e27`, especially subtree identities, anchor selection, semijoins, and materialization.
- Data-layer PR 2539 at `281f500d7c8d413f17c6a27de2f3c518bb45258a`, including both the fact-query compiler and ServerPlan composition.

The PR review uses its exact head rather than the later `fce4dc7208` checkout cited in earlier notes.
Its dependent-read strategy composes eligible DAGs through typed relation references.
Its linter compiler emits one query per declarative specification, while the batch executor still loops over plugins.
Its bounded recursive traversal does not establish general recursive Rerail semantics.

The current large-plan driver meets the statement budget by scheduling frame operations inside recursive SQL.
Its program counter, mutable publications, and tick-dependent reads retain substantial interpreter work.
The immutable fusion subset already uses relation composition, but it still derives its graph from frame-oriented emission.
The review recommends a typed relational IR before storage decisions, with explicit identities, demand, cardinality, order, unknowns, and effects.

Materialization boundaries must have stable inputs and narrow outputs.
Selective anchors and subtree reductions precede those boundaries.
Small physical phase splits remain valid for preparation limits and measured access paths.
The whole cold job must remain within 100 actual SQL executions across all workers.

Recursion remains in SQL, with specialized state for each recursive component.
Efficient nonlinear and ordered recursion remains an explicit design proof obligation.
The review does not claim that a generic continuation CTE solves that performance problem.
It proposes hand-designed reference kernels before the next compiler implementation pass.

Documentation checks cover local source links, pinned source claims, measurement scope, and prose consistency.
No engine tests or benchmarks ran because this change only adds the review and this journal entry.

## Direct B023 relational spike — 2026-10-22

The user authorized an executable B023 spike and requested ongoing journal entries.
The starting engine revision is `e5783859`.
The experiment replaces the complete matching query for B023 in a test harness.
It does not add a production engine mode.

The existing 138-file profile attributes 4.911 CPU seconds to B023 instruction steps.
Those steps publish 6,883,089 rows and perform 14,965,449 frame advances.
These attributed costs exclude shared stages and are not a standalone B023 baseline.
The spike will measure a standalone baseline on the same inputs.

The proposed query uses loop assignments, eligible functions, parameter/store exclusions, and identifier loads.
All composition and recursive slot propagation remain in SQL.
Native primitives supply AST access and leaf text/pattern operations only.
The acceptance checks require exact roots and captures, one matching execution, and zero frame publications.
The experimental target is at least tenfold fewer VM instructions for this rule.

Initial source inspection identifies three semantic requirements beyond ordinary scope joins.
Safe lambdas affect function boundaries.
Loop assignment searches stop at functions and lambdas.
The policy's function-store exclusion searches the complete function subtree, including nested scopes.
The query must reproduce the policy, rather than replace it with a different interpretation of Ruff B023.

### First query and capture correction

The first direct query matches all identifier roots in the upstream fixtures.
Its capture comparison fails: native matching replaces `$N` with the selected assignment identifier after unification.
Returning the reported load as `$N` therefore changes capture identity, despite equal text and finding roots.
The rejected query and failure log remain under `/tmp/rerail-b023-relational-20261022/v1`.

The correction retains assignment witnesses through ancestor traversal.
Each loop/name pair selects its first assignment.
Each function/name pair selects its nearest successful loop.
Each load selects its nearest successful function body, including the load itself for a lambda body.
These reductions occur at their semantic boundaries, before final output.

The test harness compares native matcher roots and captures, current-engine results, and direct-query results.
The fixtures include the upstream B023 corpus and suppression fixture.
Twenty-two additional cases cover shadowing, nested functions, safe lambdas, destructuring, comprehensions, stores, repeated names, and Unicode.
An allocator build encountered the environment's deletion restriction.
Subsequent builds use the existing diagnostic allocator override for both variants, as earlier measurements did.

### First complete corpus measurement

The capture-correct query passes both fixtures and all 22 adversarial cases.
It also matches both oracles across all 138 frozen corpus files, with two raw B023 matches.
Three fresh-process samples give median kernel CPU of 4.768 seconds for the current engine and 0.522 seconds for the direct query.
The direct query executes once and publishes zero frames.
VM instructions decrease from 369,851,302 to 43,235,816, an 8.55-fold reduction.
This candidate misses the tenfold VM target despite a 9.14-fold CPU improvement.

The profile explains the remaining work.
SQLite scans all nodes for one named-expression child search and scans all identifiers for two small relation joins.
The parameter relation contains only 166 walk rows, yet its final join starts from the complete identifier domain.
The next candidate fixes these access directions with explicit driving relations.
This is physical planning over the direct relation graph, not another frame-scheduler optimization.

Artifacts and exact input hashes are in `/tmp/rerail-b023-relational-20261022/v2`.
The comparison driver freezes the corpus and checks policy, query, and input hashes across samples.
It keeps diagnostic relation counts and EXPLAIN outside timed matching execution.

### Access direction and selective regions

The third candidate fixes three reversed access paths with `CROSS JOIN`.
It drives child lookup from named expressions and drives identifier lookup from parameter/store rows.
It passes fixture and corpus parity and reduces VM instructions to 14,400,337.
Its single trace-disabled sample improves kernel CPU by 15-fold.
Artifacts remain in `/tmp/rerail-b023-relational-20261022/v3`.

The fourth candidate restricts assignment and store discovery to loop regions that contain eligible functions.
It combines overlapping regions before subtree scans.
This restriction preserves all stores in candidate functions, including nested scopes.
It reduces target-slot rows from 30,335 to 457 and assignment ancestor states from 142,371 to 2,785.
The direct query now uses 2,324,796 VM instructions, 159-fold fewer than the standalone current-engine baseline.
Its first trace-disabled sample improves kernel CPU by 91-fold.
These are preliminary timings pending the final repeated measurements and scaling checks.
Artifacts remain in `/tmp/rerail-b023-relational-20261022/v4`.

### Scaling checks and measurement controls

All 138 source hashes match the previous complete-workload evidence.
The comparison driver freezes those sources before its first execution.
It also checks the executable, query, harness, driver, and policy hashes after the run.

The scaling tests pass at sizes 32, 128, and 512.
Unrelated assignments outside relevant loops leave direct SQL work constant at 1,822 VM instructions.
Combined repeated assignments and repeated loads use 24,909, 94,797, and 374,349 VM instructions.
The final case has sixteen times more assignments and loads, but approximately fifteen times more SQL instructions.
All scaling cases preserve native roots and captures.
AST parsing and index construction still process input data and remain outside this VM-only growth claim.

The preliminary timing samples disable SQL tracing but retain row counters.
The final harness disables both tracing and frame counters during timing samples.
It includes SQL generation and connection setup, and excludes artifact writes and diagnostic queries.
The separate profile retains exact row counters, VM instructions, relation sizes, and EXPLAIN output.

### Final result and compiler target

The final query passes exact root and capture equality against the native matcher and the current SQL engine.
The corpus contains 138 files, 7,336,001 source bytes, and two raw B023 matches.
The two upstream fixtures contain 29 matches, and the 22 adversarial cases contain 24 matches.
All six scaling cases also pass both oracles.

| Measurement | Current SQL kernel | Direct query |
|---|---:|---:|
| Median kernel CPU, three fresh processes | 4,616.048 ms | 49.230 ms |
| VM instructions, separate profiled execution | 369,851,302 | 2,324,796 |
| Frame publications | 6,944,829 | 0 |
| Frame advances | 15,597,627 | 0 |
| Node advances | 3,745,416 | 81,315 |
| Matching SQL executions | 1 | 1 |
| Actual executions including kernel setup and transactions | 4 | 1 |
| Generated matching SQL bytes | 278,847 | 8,196 |

The kernel CPU improvement is 93.77-fold, and the VM reduction is 159.09-fold.
These timings include SQL generation, preparation, connection setup, index construction, and execution over already parsed ASTs.
They exclude artifact writes and diagnostic queries.
Shared parsing costs a median 717.679 ms separately.
The native oracle's direct identifier scan is a correctness reference, not a production native-engine benchmark.
The current kernel precedes the direct query in each sample on a shared host with the diagnostic allocator override.

Zero frame publications does not mean zero SQLite materialization.
The direct statement creates 37,241 automatic-index entries.
The diagnostic query reports logical relation cardinalities, rather than physical allocation or write counts.
The largest named relation is the function ancestor traversal, with 23,208 rows.
The useful function/loop relation contains 48 rows, which select 16 loops and ten disjoint subtree regions.
Those regions constrain assignment and store discovery before capture-dependent joins.

The result provides a concrete compiler target:

1. Derive structural demand before utility evaluation: `function_loops` selects `relevant_loops`, which collapse into disjoint `regions`.
2. Restrict utility seed scans to those regions before recursive slot propagation.
3. Compute assignment answers at `(file, loop, name_atom)` identity and retain the first assignment witness.
4. Join those answers to function demand, select the nearest successful loop, and apply parameter/store anti-joins.
5. Probe identifier postings by name and body interval, then select the nearest successful body.

The required operators are constrained structural closure, keyed joins, anti-joins, region restriction, and ordered witness reduction.
The query contains no program counter, mutable frame publications, dirty-demand loop, or native rule-composition callback.
Native callbacks only read source text and match the fixed `functools.reduce` leaf pattern.
The production compiler must generate these relations from the DSL, including the capture-order proofs.
Hard-coding B023 in production is not the proposed integration.

This spike does not change the production engine or claim a whole-job speedup.
The complete 135-policy, 100-execution acceptance gate must run after compiler integration.
The existing 72-execution full-job result remains historical evidence for the unchanged engine.

The checked-in artifacts are:

- `native/src/code_rule_dsl/sql/spikes/whole_call/b023.sql`: the direct query template.
- `native/src/code_rule_dsl/sql/spikes/whole_call/b023.rs`: both output oracles, fixtures, scaling checks, and corpus harness.
- `benchmark/b023-relational-spike.py`: frozen-input profiling and repeated-timing driver.
- `benchmark/sql-b023-relational-evidence.json`: compact results, iterations, hashes, scaling evidence, and expanded SQL.

Raw profiles, EXPLAIN output, relation counts, and sample reports remain under `/tmp/rerail-b023-relational-20261022/final`.
The driver accepts the existing SQL workload configuration format and creates a new output directory.
The commands used for the final run were:

```sh
JEMALLOC_OVERRIDE=/tmp/librerail_jemalloc_check_shim.dylib \
  python3 benchmark/b023-relational-spike.py \
  /tmp/rerail-b023-relational-20261022/v2/config.json \
  /tmp/rerail-b023-relational-20261022/final
```

The allocator path is specific to this development environment.
Without that environment restriction, the driver accepts an ordinary Cargo build.
It builds the optimized test executable, checks fixtures and scaling, freezes inputs, profiles one corpus execution, and takes three fresh-process timing samples.

Strict Clippy reports an existing regex-construction warning at `native/src/code_rule_dsl/sql/tests/mod.rs:197`.
That code predates this spike and remains unchanged.
The follow-up Clippy check passes with only `clippy::regex_creation_in_loops` permitted and all other warnings as errors.
The targeted Rust suite passes three tests, and the corpus test passes the profile and all three timing samples.
Python syntax, Rust formatting, evidence hashes, and whitespace checks also pass.

## Direct relational compiler transition plan — 2026-10-22

The user requested a transition plan after the complete B023 reference spike.
The [transition plan](sql-relational-transition-plan.md) turns that result into five implementation milestones.
This entry records design work only, with no production compiler change or new performance measurement.

Source inspection identified three integration constraints.
`ProgramCompiler::clause` currently emits frame SQL during clause registration.
The fused graph stores SQL bodies, and its adapter reconstructs relations from emitted SQL.
The replacement must introduce typed relations before either operation, rather than extend SQL reparsing.

The execution path also partitions files before it runs the compiled program.
Whole-job planning therefore needs caller changes in `workspace/sql.rs`, `eval/query.rs`, and `sql/exec.rs`.
The existing execution-budget script explicitly enforces acceptance after the complete run.
The plan adds admission and runtime accounting across workers, including reserved cleanup capacity.

The chosen migration develops one replacement compiler through a test harness.
Production cutover waits for complete supported semantics and removes the old compiler and scheduler.
There is no proposed per-clause fallback or new user switch.
This delays production gains until coverage passes, while intermediate compiler measurements remain executable.

The first milestone generates the complete B023 shape directly from the DSL.
It starts with the complete capture-free `target-slot` utility, then adds demand propagation, capture projection, and ordered choices.
It uses the reference query as an independent performance target, not a production policy template.
Renamed utility and changed-constant cases guard against policy recognition.

The initial B023 gates permit at most twice the reference VM work and median kernel CPU.
They also require exact roots and capture identities, one matching statement, zero frame publications, and the existing scaling bounds.
These are proposed compiler acceptance thresholds, not new measurements.

The second milestone proves unknowns, failures, enumeration, and general recursion before broad catalog migration.
The continuation design remains unresolved for irreducible recursive components.
The plan requires executable semantic and scaling evidence before it accepts that operator.
It prohibits a global instruction interpreter or statements per recursive round.

Later milestones cover selective sharing, physical planning, complete-job instrumentation, and production cutover.
The final gate retains the 100-execution budget and all current workload coverage.
Proposed whole-job targets require fourfold fewer VM instructions and twofold lower SQL apply CPU, plus improved cold time.
Fresh comparative measurements must establish those results.
The 94-fold B023 kernel improvement is not a whole-job prediction.

The plan names implementation modules, relation contracts, cutover removal points, and the first three reviewable commits.
Documentation checks cover local links, source references, evidence numbers, sentence length, and whitespace.
No engine tests rerun because this change contains documentation only.

## Transition phase 1: direct utility compilation — 2026-10-22

The user requested the first transition phase.
The first implementation slice compiles B023's complete `target-slot` utility from `GrammarMatcher`.
The replacement remains test-only, as the transition plan requires.
This slice does not yet compile the complete capture-bearing B023 rule.

The new `sql/relational` modules contain set relations, semantic admission, direct DSL lowering, physical join arms, and SQL emission.
Each relation has `(file, node)` identity.
Kinds, regex leaves, Boolean composition, neighbor relations, and positive parent recursion form the initial admitted subset.
The compiler rejects captures, unknown outcomes, nonlinear recursion, recursive negation, and unsupported traversal forms.
No frame lowering or SQL reparsing participates in compilation.

The compiler derives recursive seeds and arms from utility definitions.
A recursive arm must reference its own relation once through strictly ascending parent dependencies.
That restriction proves structural progress without a program counter or mutable frames.
Join emission starts with the recursive frontier or selective kind evidence, then uses structural probes.
The existing `node_regex` primitive supplies only regex leaf evaluation.
Native code does not evaluate relation composition.

The initial harness compares every node with native execution across multiple files.
Additional cases rename the utility and change its kinds and regex constants.
These cases require compilation from the actual DSL instead of policy recognition.
The first build and semantic checks are in progress.

### First utility slice: results

The direct compiler passes native and current-SQL equality for all 30,335 target slots across the frozen 138 files.
All source hashes match the complete B023 reference corpus.
The compiler emits one matching statement and publishes no frames.
The current SQL engine publishes 6,499,631 frames and advances through 14,306,235 frame rows for the same utility.
Both kernels use one matching statement, so this result isolates the internal work difference.

The first generated query uses 4,243,601 VM instructions.
Inspection found an unnecessary AST lookup for every row in a relation alias.
It also found redundant per-arm DISTINCT operations beneath a set UNION.
The emitter now preserves relation aliases directly and relies on the UNION's duplicate elimination.
Single-arm relations retain DISTINCT where their joins can repeat a root.
These generic changes reduce VM instructions to 3,704,554 and node advances from 167,626 to 137,291.

The final EXPLAIN shows kind postings for both seed branches, parent probes for children, and node-ID probes for siblings.
The recursive arm reads its frontier, filters destructuring kinds, and probes children.
The query uses no automatic index entries and reports zero fullscan steps.
SQLite still uses temporary B-trees for set ordering and final output ordering.
Zero fullscan steps does not imply zero scans or zero storage inside SQLite.

The utility still returns every target slot in the corpus.
The complete hand-written B023 reference returns only 457 demanded target slots after region restriction.
This difference identifies the next major compiler task: derive relevant function/loop regions before utility evaluation.
The current 3.70-million instruction count does not pass the complete B023 milestone by itself.
The capture-bearing rule and its full-query comparison remain outstanding.

The upstream fixtures and adversarial file contain 100 target slots.
Renaming the utility preserves all 100 matches.
Changing an assignment kind produces 149 matches, and changing the loop keyword regex produces 91 matches.
Every variant agrees with both oracles.
The compiler contains no policy name or B023 SQL template.

| Scaling case | Size | Matches | Generated VM instructions |
| --- | ---: | ---: | ---: |
| Unrelated calls | 32 | 11 | 663 |
| Unrelated calls | 128 | 11 | 663 |
| Unrelated calls | 512 | 11 | 663 |
| Repeated assignment targets | 32 | 352 | 18,550 |
| Repeated assignment targets | 128 | 1,408 | 73,942 |
| Repeated assignment targets | 512 | 5,632 | 295,510 |

These are SQL work measurements, not constant-time parsing or index construction claims.
The four compiler tests cover neighbor semantics, positive parent recursion, unsupported constructs, and preparation limits.
The fixture/mutation test, scaling test, and explicit corpus test also pass.
The three existing B023 reference tests pass, with the corpus reference test left ignored in that regression run.
Clippy passes with only the existing `regex_creation_in_loops` warning permitted.
Rust formatting, Python syntax, evidence hashes, and whitespace checks pass.

The current scope has two important limits.
The compiler does not reproduce native utility work or stack/depth budgets.
The harness checks logical outputs with generous work allowance and reports any native oracle failure.
This restriction appears in the relation profile and blocks production integration.
The complete job's 100-execution gate also remains pending because this slice changes no production path.

The durable evidence and reproduction command are:

- `benchmark/sql-relational-target-slot-evidence.json`: generated SQL, EXPLAIN, counters, scaling, and source/executable hashes.
- `benchmark/relational-compiler-spike.py`: frozen-input driver with semantic, fixture, scaling, and corpus checks.
- `/tmp/rerail-relational-phase1-20261022/final`: raw results, frozen inputs, query, and check logs.

```sh
JEMALLOC_OVERRIDE=/tmp/librerail_jemalloc_check_shim.dylib \
  python3 benchmark/relational-compiler-spike.py \
  /tmp/rerail-b023-relational-20261022/final/config.json /absolute/new/output
```

The allocator override is specific to this development environment.
The driver builds the optimized test executable unless the caller supplies a matching executable with `--binary`.
All reported timings retain instrumentation, and their scopes differ between the two kernels.
This slice makes no repeated-timing speedup claim.

## Transition phase 1: structural region demand — 2026-10-22

The user requested the next phase after direct target-slot compilation.
This slice adds ancestor closure, capture-free pattern leaves, and subtree demand propagation to the same compiler.
The test harness expresses a capture-free demand view with B023's unchanged loop, function, and target-slot utilities.
The compiler derives the SQL regions from that ordinary DSL query.
Automatic extraction of this view from the complete capture-bearing rule remains a later integration step.

The inverse descendant closure starts with matching functions and climbs through parent links until the policy's inclusive stop condition.
Its output selects loop roots, and interval normalization removes nested roots from the scan domain.
The compiler specializes the target producer for that domain without changing other consumers.
It removes unreachable unrestricted producers from the emitted query.

Seed restriction requires a boundary proof.
Every recursive dependency must use one parent hop with a kind guard that excludes region-root kinds.
Without that proof, the compiler retains global utility evaluation and filters its output.
This preserves utilities whose matching seed lies outside a requested subtree.
Seed restriction applies to output keys, while parent and sibling metadata probes can cross the boundary.

The implementation and corpus checks are complete for this demand view.
Full B023 generation remains outside this slice.

### First region profile and the remaining amplification

The initial corpus run agrees with native and current SQL on all 457 demanded target slots.
It derives 16 selected loop roots and ten disjoint regions.
Its generated query still uses 13,419,168 VM instructions.
The logical cardinality profile exposes 71,729 answers from the unrestricted safe-slot producer.
Only lambda candidates consume those answers.
The matching statement makes 54,258 pattern requests while constructing that global producer.

The next rewrite specializes a negative utility probe to its consumer's candidate kinds.
A separate key closure adds the recursive parent dependencies needed to answer those candidates.
Seed and recursive outputs then stay inside that required key set.
Other consumers retain independent producer variants.
The compiler also removes a redundant region filter after a proved restricted producer.

The first profile's node and regex counters include its subsequent diagnostic cardinality query.
Its VM counter and pattern-request counter describe the matching statement alone.
The harness now snapshots statement counters before diagnostic queries.
Final evidence will keep logical diagnostic cardinalities separate from physical matching counters.
Initial evidence remains in `/tmp/rerail-region-demand-20261022/initial.json`.

### Final shape and measured work

Consumer-key restriction reduces safe-slot evaluation from 71,729 answers to 24 answers over 143 required keys.
The key closure includes parenthesized dependencies, so a safe lambda can still inherit its answer from an enclosing expression.
It does not publish intermediate frames or invoke native rule composition.

The final physical rewrite changes the target seed's join direction.
Previously, each seed arm scanned every node in a region and inspected its parent.
It now scans parents of the required kinds inside the inclusive region, then probes their children.
Every such child is a strict descendant of that region.
This proof permits removal of the repeated output-domain membership check.

The generated graph has these stages:

```text
candidate lambda keys -> required parent keys -> safe-slot answers
eligible functions -> stopped ancestor closure -> selected loops
selected loops -> disjoint subtree regions
regions -> kind-filtered target parents -> seed children -> recursive target slots
```

Every arrow stays inside one matching SQL statement across all 138 files.
All inputs match the frozen reference by SHA-256.
Each measured query agrees with both native evaluation and the current SQL engine on all 457 target slots.

| Measurement | Region restriction only | Add recursive key demand | Add parent-first seeds |
| --- | ---: | ---: | ---: |
| VM instructions | 13,419,168 | 1,184,676 | 704,585 |
| Pattern requests | 54,258 | 9 | 9 |
| Automatic-index entries | 84,433 | 12,701 | 12,701 |
| Node advances, matching only | Not comparable | 59,453 | 31,689 |
| Regex requests, matching only | Not comparable | 10,089 | 1,148 |

The final shape uses approximately 19 times fewer VM instructions than the first region implementation.
This comparison covers the same demand view, rather than the previous unrestricted utility or the complete B023 query.
The final query publishes zero frames; the current engine publishes 2,476,234 frames for this view.
Both use one matching statement.
The final instrumented prepare-and-execute sample costs 33.46 ms, including 1.35 ms for preparation.
These single samples do not establish a CPU speedup ratio.

The logical cardinality profile contains 8,489 eligible functions, 4,176 ancestor keys, 16 selected loops, and ten disjoint regions.
The current capture-free existence query can merge identical ancestor keys from different functions.
The complete rule must retain function identity where downstream capture matching observes it.

EXPLAIN confirms kind-indexed candidate scans, point probes for required parents, and region-plus-kind scans for target seeds.
It still shows materialized relations, automatic covering indexes, and temporary B-trees for set union and ordering.
These structures are immutable SQLite intermediates; they are not frame publications.
The query's 13,338 fullscan steps include intermediate scans and do not describe total AST work.

### Scaling and semantic checks

| Scaling case | Size | Matches | VM instructions |
| --- | ---: | ---: | ---: |
| Assignments outside regions | 32 / 128 / 512 | 2 | 1,044 at every size |
| Calls outside regions | 32 / 128 / 512 | 2 | 1,044 at every size |
| Assignments inside regions | 32 | 354 | 19,860 |
| Assignments inside regions | 128 | 1,410 | 76,308 |
| Assignments inside regions | 512 | 5,634 | 302,100 |

Unrelated growth adds no SQL VM work in these cases.
Sixteen times more demanded assignments costs about fifteen times more VM work.
These measurements exclude parsing and index construction.

Seven compiler semantic tests pass.
They cover neighbor joins, positive recursion, shared consumers, boundary-crossing seeds, inclusive stops, recursive required keys, and unsupported constructs.
The region fixtures return 58 matches; utility renaming preserves those matches.
Changing the loop kind returns ten matches, and changing the pattern constant returns 59 matches.
Every variant agrees with both oracles.
The earlier target-slot fixture and scaling checks also pass.
The three handwritten B023 reference regression tests pass.
Its full-corpus test remains ignored in that regression run; the new region corpus test runs explicitly.
Clippy passes with only the existing `regex_creation_in_loops` warning permitted.
Rust formatting, Python syntax, evidence hashes, and whitespace checks pass.

The durable evidence is `benchmark/sql-relational-region-demand-evidence.json`.
It includes final SQL, EXPLAIN, counters, cardinalities, scaling results, source hashes, and both intermediate SQL shapes.
The raw final artifacts are in `/tmp/rerail-region-demand-20261022/final`.
The driver now runs the region corpus plus the earlier target-slot fixture and scaling checks:

```sh
JEMALLOC_OVERRIDE=/tmp/librerail_jemalloc_check_shim.dylib \
  python3 benchmark/relational-compiler-spike.py \
  /tmp/rerail-b023-relational-20261022/final/config.json /absolute/new/output
```

The profile's matching counters exclude the later EXPLAIN and relation-cardinality queries.
Its `diagnosticCpuNs` includes both diagnostics and connection setup.
The single matching statement is not a count of every execution in a complete cold job.

### Remaining transition work

This compiler still runs only in tests, through the same entry point as the previous slice.
There is no additional production switch or native rule fallback.
The harness supplies the capture-free demand view; the compiler does not yet derive that view from complete B023.
The next step must preserve function identity, join target atoms to reads, and implement ordered capture selection.
Native utility work and stack/depth budget semantics also remain outstanding.
The full-rule performance gate and complete-job limit of 100 executions remain required before production cutover.

## Transition phase 1: capture demand from the original rule — 2026-10-22

The user requested the next phase after structural region demand.
This phase removes the harness-supplied demand view and preserves origin and witness identity through the generated closures.
The compiler reads the original `GrammarMatcher` and its utility graph and capture order.
Its output is an intermediate capture-demand relation, not complete clause findings.

The admitted operator sequence is an ordered ancestor search whose producer contains one descendant binding search and pure conjuncts.
The caller must already bind the capture key.
The discovery pass carries binding availability through conjunctions and utility calls.
It does not move ordered alternatives or capture-bearing negative predicates across their boundaries.
Necessary pure predicates define an origin domain that can include candidates rejected by later capture-dependent filters.
No policy name, utility name, kind constant, or SQL template identifies this sequence.

The origin closure retains `(file, origin, ancestor, distance)`.
Its selected ancestor keys define the subtree regions that restrict the existing target producer.
The binding closure retains `(file, atom, witness, ancestor)` and obeys the descendant search's separate inclusive stop condition.
The first successful descendant wins per ancestor and value.
The nearest successful ancestor then wins per origin and value.
These reductions happen before later negative predicates or consumers can introduce backtracking.

The value key and witness node remain separate columns.
The current frontier admits leaf capture values, whose text atoms preserve native equality.
A composite binding candidate raises an execution error instead of receiving an invalid equality key.
Complete clause lowering must prove its caller's key contract before it can consume this intermediate relation.

Implementation and validation are in progress.

### Admission and oracle checks

An initial semantic check exposed missing propagation through nested conjunctions.
The analysis now computes definite bindings through conjunctions, alternatives, traversals, and utilities.
Alternatives contribute only names bound on every successful branch.
An unbound ordered search fails compilation.

The native oracle enumerates origin nodes and candidate values from source trees independently of SQL output.
It evaluates the original search with each bound value and compares the selected witness, ancestor, and distance.
The corpus check also compares projected demand rows against the handwritten reference's demand relation.
The reference still runs its complete native/current-engine/full-query parity check separately.

### First profile: correctness passes, structural work gate fails

The first corpus run returns 1,029 demand rows with exact native and reference parity.
It retains 48 origin/ancestor pairs.
These pairs select 16 loop roots and ten disjoint regions.
The binding stage contains 361 leaf candidates and 312 ancestor/value answers.
However, the matching statement uses 5,730,787 VM instructions and 65,428 regex requests.

The unrelated-assignment scaling check fails: 32 assignments cost 5,989 instructions, while 512 assignments cost 65,509 instructions.
The demand decision explains the cause: the new search-root relation has no inferred kind bound.
The boundary proof therefore retains the unrestricted target-slot producer and filters its output afterward.

The correction propagates the ancestor predicate's kind bound through the search-root projection.
This permits the existing closed-boundary rewrite to restrict recursive target seeds to the selected regions.
The scaling assertion remains unchanged.
The initial query, EXPLAIN, counters, and oracle results are in `/tmp/rerail-capture-demand-20261022/initial-corpus.json`.

The admitted producer reads only the input capture's leaf value.
Its successful binding replaces the witness node, and its remaining predicates are capture-free.
Thus `(file, origin, atom)` identifies its answer without retaining the caller's previous witness in intermediate joins.
The output retains the chosen binding witness separately.

### Final results and SQL shape

The corrected compiler keeps exact parity on all 1,029 demand rows across 138 files.
The native comparison includes origin, value, witness, selected ancestor, and ancestor distance.
The independent reference comparison includes origin, value, and witness.
The reference's complete rule still returns two findings with native/current-engine parity.
Those two findings are not outputs of the new intermediate compiler stage.

| Measurement | Initial origin-preserving query | With propagated kind bound |
| --- | ---: | ---: |
| VM instructions | 5,730,787 | 2,238,220 |
| Regex requests | 65,428 | 1,148 |
| Node advances | 205,618 | 67,386 |
| Node filters | 186,044 | 39,045 |
| Automatic-index entries | 70,664 | 40,330 |
| Pattern requests | 9 | 9 |
| Matching statements | 1 | 1 |
| Frame publications | 0 | 0 |

The final query contains 7,175 SQL bytes.
Its instrumented prepare-and-execute sample costs 66.77 ms.
Preparation accounts for 1.77 ms.
These single samples do not establish a CPU speedup ratio or complete-job improvement.
The earlier 704,585-instruction region query drops function identity and has no capture choices, so it measures a different stage.

The generated CTE graph is:

```text
origin candidates -> origin/ancestor/distance closure -> selected origin/ancestor pairs
selected ancestors -> disjoint regions -> restricted target recursion -> leaf binding candidates
leaf candidates -> stopped binding closure -> first witness per ancestor/value
origin/ancestor pairs + selected witnesses -> nearest successful ancestor per origin/value
```

The first closure contains 23,208 rows over 8,489 candidate functions.
It retains 48 selected function/loop pairs, 16 distinct loops, and ten disjoint regions.
The second closure contains 2,785 rows from 361 binding candidates.
Its first-witness reduction produces 312 ancestor/value answers before the final 1,029 origin/value demand rows.
The larger final relation reflects distinct functions that share a value, rather than duplicate copies of their environments.

The compiler lowers these operators to typed relation references.
The shared emitter constructs the SQL after physical planning.
The search-root projection exposes its kind bound to the existing subtree safety proof.
SQL contains immutable CTEs, atom keys, explicit witness columns, and ordered reductions.
No SQL callback evaluates utility composition, and no intermediate frame is published.

EXPLAIN still contains materialized origin and stop predicates, automatic covering indexes, and temporary B-trees for reductions and ordering.
These costs remain visible in the profile.
The next full-query profile must assess their combined cost with exclusions and read probes.

### Final checks and remaining work

| Scaling case | Size | Demand rows | VM instructions |
| --- | ---: | ---: | ---: |
| Assignments outside selected regions | 32 / 128 / 512 | 2 | 1,963 at every size |
| Assignments inside selected regions | 32 | 34 | 19,371 |
| Assignments inside selected regions | 128 | 130 | 71,595 |
| Assignments inside selected regions | 512 | 514 | 280,491 |

Unrelated growth adds no SQL VM work in this experiment.
Sixteen times more demanded assignments costs about 14.5 times more VM work.
These work counts exclude parsing and index construction.

All ten compiler semantic tests pass.
They include origin sharing, nested stops, repeated values with different witnesses, cross-file identity, unbound inputs, optional bindings, and composite-key refusal.
The six compiler fixture and scaling tests pass.
These tests include the previous target-slot and region stages.
The complete-rule fixtures produce 64 demand rows, and utility/capture renaming preserves every row.
Changing the loop kind produces 22 rows, and changing the pattern constant produces 71 rows.
Every variant agrees with the native demand oracle.
The three handwritten reference regression tests also pass.
Clippy passes with only the existing `regex_creation_in_loops` warning permitted.
Rust formatting, Python syntax, source and input hashes, and whitespace checks pass.

The durable evidence is `benchmark/sql-relational-capture-demand-evidence.json`.
It includes generated SQL, EXPLAIN, exact output rows, counters, cardinalities, scaling results, source hashes, and the initial failing physical shape.
All 138 input hashes match the frozen reference.
The raw final artifacts are in `/tmp/rerail-capture-demand-20261022/final`.

```sh
JEMALLOC_OVERRIDE=/tmp/librerail_jemalloc_check_shim.dylib \
  python3 benchmark/relational-demand-spike.py \
  /tmp/rerail-b023-relational-20261022/final/config.json /absolute/new/output
```

The driver builds the optimized test executable unless the caller supplies a matching binary with `--binary`.
The matching counters exclude the subsequent EXPLAIN and cardinality queries.
One matching statement here is not the complete cold-job execution count.

The automatic demand-extraction and ordered binding stage is complete for the admitted operator sequence.
This remains an intermediate test-only compiler stage, with no new production switch.
The next stage must compile parameter/store exclusions, body discovery, and load matching from the original rule.
It must preserve the selected assignment witness through final ordered capture selection.
Complete B023 performance gates, native resource-budget semantics, and the complete-job limit of 100 executions remain pending.

### Complete ordered answer lowering — work in progress, 2026-10-22

This phase completes the original B023 grammar through final findings.
It extends the existing compiler entry; it adds no production switch or rule-name recognizer.
The previous phase stopped at 1,029 intermediate function/value demands.
This phase must preserve exact final root and capture identities.

The new answer operators represent ordered alternatives, ancestor expansion, binding constraints, and negative value probes.
Negative probes start from distinct requested function nodes.
Their recursive rows contain `(file, origin, node, state)`, without capture environments or witnesses.
Only binding leaves produce atom keys for the anti-join.
The recursive parameter search must move strictly down the tree; zero-distance cycles fail compilation.
Pure predicates use bound node probes and proven subtree restrictions for recursive utilities.

Validation will compare native evaluation, the current engine, and generated SQL on identical sources.
The full reference query remains independent.
The acceptance limits remain 4,649,592 VM instructions and twice the reference median kernel CPU.
One matching statement and zero frame publications are required.
These checks do not establish the complete-job 100-execution gate or permit production cutover.

The first fixture run failed during diagnostic cardinality preparation.
SQLite reported `too many references to "node": max 65535`.
The matching query had executed, but final parity assertions had not yet run.
Repeated utility expansion duplicated complete answer subplans.
The compiler now shares identical typed answers and caches lowered capture-bearing utilities.
Diagnostic cardinalities use separate queries instead of a compound query that repeats all dependencies.
These diagnostic statements remain outside the matching-statement count.

The planner also pushes root-kind and atom constraints into descendant expansion through ordered alternatives.
These constraints observe the root and value, so they do not depend on the selected witness.
This prevents a function/value-by-descendant intermediate before load filtering.
The compiler rejects negative value probes whose input capture is not definitely bound.

The first complete corpus measurement passed exact parity: two findings across all 138 files.
It used 3,130,168 VM instructions, below the 4,649,592 limit.
However, it failed the CPU gate: median cold kernel CPU was 170.417 ms, versus 63.804 ms for the reference.
Preparation consumed a median 94.335 ms; the reference needed 2.791 ms.
Execution consumed about 74 ms, versus about 60 ms for the reference.
This is a compiler-output problem, despite acceptable execution work.

The next physical changes remove pass-through answer materializations and merge same-node filters.
Disjoint necessary kind sets remove impossible branches before SQL emission.
Equivalent regional recursive producers now share one CTE.
Existing key-demand producers remain shared with the frontier instead of acquiring a redundant regional copy.
A producer already restricted to the required region can drive a join directly.
This replaces a region-wide node scan followed by an existence check against that same producer.

Initial scaling checks passed before these changes.
Outside assignments at sizes 32, 128, and 512 used exactly 3,580 VM instructions.
Combined repeated assignments and loads used 54,839, 211,223, and 836,759 instructions.
The complete upstream fixtures produced 29 exact findings.
The 22 adversarial snippets produced 24 exact findings.

The second corpus profile reduced work to 2,897,697 VM instructions.
Preparation fell to a median 54.876 ms.
The CPU gate still failed: 132.061 ms versus 62.200 ms, approximately 2.12 times the reference.
The diagnostic relation counts exposed two identical negative-probe closures.
Each contained 195 traversal rows and produced 89 exclusion keys.
Merging same-node filters had duplicated those probes across function-kind consumers.
The emitter now shares probes by their typed plan and exact input relation.
The lowerer also removes predicates that the frontier's origin scan has already proved.
These changes target repeated dependency expansion during preparation, as well as repeated execution work.

The new zero-distance recursion test initially expected rejection in relational lowering.
The policy parser rejects that cycle earlier.
The corrected test requires compilation to reject the cycle at either boundary.
The check does not execute a cycle or reinterpret it as an empty result.

### Complete B023 acceptance result — 2026-10-22

Milestone 1 now passes.
The [evidence](../benchmark/sql-relational-complete-evidence.json) contains the generated query, EXPLAIN, row counts, source hashes, and paired samples.
The [SQL artifact](../benchmark/sql-relational-complete.sql) makes the complete generated shape directly reviewable.
The compiler does not read that artifact.
It lowers the unchanged full grammar through the normal relational compiler entry.

The frozen corpus contains 138 files and 7,336,001 source bytes.
Every file hash matches the original reference corpus.
Native evaluation, the current engine, and both relational queries agree on the exact two roots and capture witnesses.

| Measurement | Generated full query | Handwritten reference |
| --- | ---: | ---: |
| Actual traced matching executions | 1 | 1 |
| VM instructions | 2,812,255 | 2,324,796 |
| Frame publications | 0 | 0 |
| Median cold kernel CPU, three samples | 125.553 ms | 63.349 ms |
| Median preparation CPU | 51.560 ms | 2.916 ms |
| Median execution CPU | 72.052 ms | 59.500 ms |
| Node advances | 87,702 | 81,315 |

The VM ratio is 1.210; the acceptance limit is 2.
The CPU ratio is 1.982, so this gate passes narrowly.
Preparation remains the main performance gap; this result does not establish comfortable CPU headroom.
The current engine needed roughly 5.5 seconds of kernel CPU in the paired runs.
The generated query uses about 132 times fewer VM instructions than the recorded 369,851,302-instruction current-engine baseline.
These are full B023 kernel measurements, not whole-policy-catalog or whole-job speedups.

The final graph retains 1,029 function/value demands.
One shared exclusion closure contains 195 traversal rows and produces 89 function/value keys.
Its anti-join leaves 1,020 function/value answers.
Function bodies retain those keys until indexed name/interval probes produce the two final loads.
The final query contains 22,020 SQL bytes, creates 54,156 automatic-index entries, and uses 1,327,504 bytes of statement memory.
There are 3,092 regex requests and nine capture-free pattern requests.
Intermediate JSON environments and frame publications are absent.

The final outside-assignment scaling cases use 2,999 VM instructions at all three sizes.
Combined assignment/load cases use 44,248, 170,392, and 674,968 instructions at sizes 32, 128, and 512.
Thus 16 times the combined input requires about 15.3 times the SQL work.
The upstream fixtures produce 29 exact findings.
The 22 adversarial snippets produce 24 exact findings.
Renamed utilities and captures preserve results; changed loop kinds and leaf patterns change them with native parity.
Late same-node binding preserves capture overwrite order.
Unbound negative inputs and zero-distance recursive cycles fail compilation.

Validation passed 24 targeted Rust tests, rustfmt, and both Clippy configurations from `npm run check:native`.
Clippy exposed an existing regex construction inside a test loop; that construction now occurs once before the loop.
No production selector or fallback was added.
The replacement remains test-only while unknowns, reached failures, enumeration, general recursive components, and native resource budgets await Milestone 2.
The complete-job 100-execution gate remains unproved by this kernel test.

Reproduction uses the same frozen config and the new driver:

```sh
JEMALLOC_OVERRIDE=/tmp/librerail_jemalloc_check_shim.dylib \
python3 benchmark/relational-complete-spike.py \
  /tmp/rerail-b023-relational-20261022/final/config.json \
  /tmp/rerail-complete-reproduction
```

The driver builds the test binary unless `--binary` supplies one.
It freezes the inputs, checks their hashes against the reference manifest, and records source and executable hashes.
It alternates generated/reference order on fresh connections with identical instrumentation.
Parsing, EXPLAIN, and diagnostic cardinality queries stay outside the kernel timing and matching-execution count.
It preserves evidence before failing either performance gate.
Raw accepted artifacts are in `/tmp/rerail-complete-20261022/accepted`.

### Milestone 2: structural recursive components — started 2026-10-22

The next kernel compiles mutually recursive, capture-free utilities into one tagged recursive CTE per component.
Its semantic key is `(utility, file, node)`.
Each recursive arm must reference the component once and advance strictly in one common preorder direction.
This proof excludes node revisits within an evaluation path; utility dependency cycles remain valid.
`UNION` removes duplicate answers at diamonds before further expansion.
The compiler will reject negative, nonlinear, and nonmonotone components until their outcome contracts exist.

Demand propagation must not specialize one component member independently or inline a cycle.
Tests will compare native, current-engine, and generated results across multiple files, cycles, diamonds, empty closures, and external consumers.
Scaling evidence will include actual traced executions, VM work, EXPLAIN, and component cardinalities.
This work does not complete the unknown, failure, enumeration, witness, or resource-limit gates.

The first five component checks passed before the depth-scaling run stalled in the native oracle.
The harness omitted the native evaluator's normal per-file memo scope.
I stopped that run and added the same memo scope used during file evaluation.
Memoization preserves native utility-budget charges, including repeated logical paths.
The scaling policy therefore uses adjacent seed levels to avoid exponential native budget charges while retaining duplicate SQL paths.
The smaller differential cases retain alternating success and failure levels.
No performance result came from the stopped run.

Self-recursion now uses the same termination proof as mutual recursion.
Single-state closures retain their narrower `(file, node)` key.
Parent and previous-sibling steps decrease preorder; child and next-sibling steps increase it.
Each component requires one common direction, including every hop within an arm.
Negative and nonlinear dependencies still fail compilation.
Seedless positive closures produce no rows.
Components exceeding 128 expanded arms fail before SQL preparation.
This bound prevents a new unbounded compound query; later physical planning must handle larger programs.

### Structural component results — 2026-10-22

The component kernel passes native and current-engine differential tests.
The [evidence](../benchmark/sql-relational-components-evidence.json) records exact outputs, EXPLAIN, source hashes, executable hashes, and the B023 regression check.
The [generated SQL](../benchmark/sql-relational-components.sql) contains one three-state recursive relation.
Each state view projects from that shared relation.
No host loop schedules recursive rounds.

The test harness now traces actual matching executions, including SQL issued inside callbacks.
It removes the trace before EXPLAIN and diagnostic cardinality queries.
All six scaling kernels execute one matching statement and publish zero frames.

| Case | Size | Matches | Component rows | VM instructions |
| --- | ---: | ---: | ---: | ---: |
| Diamond depth | 16 | 53 | 157 | 9,519 |
| Diamond depth | 64 | 197 | 589 | 35,583 |
| Diamond depth | 128 | 389 | 1,165 | 70,335 |
| Independent regions | 16 | 224 | 640 | 38,790 |
| Independent regions | 64 | 896 | 2,560 | 155,046 |
| Independent regions | 256 | 3,584 | 10,240 | 620,070 |

Eight times the depth requires approximately 7.4 times the VM work.
Sixteen times the independent regions requires approximately 16 times the VM work.
All six kernels create zero automatic-index rows and use 12,864 bytes of statement memory.
Statement memory excludes the recursive queue and temporary storage; these results do not establish total memory use.
Component row counts come from separate diagnostic queries, not physical-write counters.

EXPLAIN shows a kind-indexed seed scan, followed by file/node and file/parent probes in recursive arms.
The recursive relation stores only state, file, and node.
Its `UNION` deduplicates answers before further expansion; paths do not enter its semantic key.
The final output uses a temporary ordering tree.
SQLite labels recursive compound substeps `UNION ALL` in EXPLAIN; the emitted recursive CTE uses `UNION` and deduplicates its queue.

Tests also cover independent components, external negative consumers, multiple files, seedless recursion, and single-state closures.
The capture-bearing integration case returns five exact roots and witnesses across two files.
Those captures stay outside the shared structural component.
The compiler rejects negative dependencies, nonlinear dependencies, mixed traversal directions, and excessive component expansion.
Demand propagation retains complete shared component answers and rejects independent member specialization.

The frozen B023 regression run passes all existing gates.
The generated query is byte-identical to the Milestone 1 artifact.
All three samples use 2,812,255 VM instructions, one traced matching statement, and zero frame publications.
The 138 files retain the exact two native roots and capture witnesses.
Paired median cold kernel CPU is 113.006 ms, versus 61.352 ms for the reference: a ratio of 1.842.
This confirms the gate on the current source; unchanged SQL prevents attributing timing variation to a new execution optimization.

Validation passed the relational tests, B023 fixtures, adversarial snippets, frozen corpus, rustfmt, and both Clippy configurations.
The replacement remains test-only.
Milestone 2 still needs explicit unknown outcomes, withheld captures, reached failures, enumeration, parameterized witnesses, and general recursion.
Native work and stack/depth limits remain unresolved, as does component-wide demand propagation.
The complete cold-job budget of 100 executions remains a later acceptance gate.

Reproduce the component measurements with:

```sh
JEMALLOC_OVERRIDE=/tmp/librerail_jemalloc_check_shim.dylib \
python3 benchmark/relational-components.py /tmp/rerail-components-reproduction
```

The driver builds the test binary unless `--binary` supplies one.
It runs the relational differential suite and preserves raw results, generated SQL, logs, and source hashes.
Raw component artifacts are in `/tmp/rerail-components-20261022-second`.
Raw B023 regression artifacts are in `/tmp/rerail-components-b023-20261022`.
The next kernel starts with three-valued conjunction and withheld captures, then extends that outcome contract to ordered search and count.

### Milestone 2: explicit outcomes and withheld captures — started 2026-10-22

This kernel lowers three-valued rules into typed demand and outcome relations.
Each outcome contains its demand key, an explicit false/true/unknown tag, and observable binding columns.
SQL null denotes an absent binding or an unreached test, never an unknown verdict.
Conjunctions retain local withheld-capture columns and evaluate later terms only where their inputs remain available.
Failed or unknown composites restore their input bindings.

Ordered alternatives and searches retain the first decisive candidate at their original boundary.
A binding candidate's unknown outcome is decisive; a capture-free candidate's unknown outcome permits a later true answer.
Counts aggregate true and unknown candidates into lower and upper bounds.
An exported count requires an exact value.

The initial admission contract permits total structural leaves, whole-node captures, and the existing synthetic unknown leaf.
It excludes fallible semantic leaves, recursive outcome components, enumeration, and rule-based traversal stops.
Candidate evaluation can precede ordered reduction only under this total, effect-free contract.
Native work and depth budgets remain separate gates.
The implementation must retain exact capture identity without intermediate JSON environments or native composition callbacks.

The first profile exposed repeated dependency expansion during preparation.
A conjunction over 32 call roots generated 57 CTEs and used 44,778 VM instructions.
Preparation took 22.194 ms; execution took less than one millisecond.
The same shape created 634 automatic-index entries.
Its SQL text contained only 9,792 bytes, so text length alone did not explain the preparation cost.

Node-local operations now use guarded scalar expressions along one input chain.
These operations no longer join each result back to its demand relation.
`CASE` prevents evaluation of decided or withheld terms.
Composite outputs export bindings only on true; their callers keep their original bindings on false or unknown.
This removes redundant joins used only to restore values that no consumer can observe.

The first suite passed 30 tests and rejected one invalid count fixture in the policy parser.
The fixture specified a zero minimum without a maximum, which the DSL rejects as vacuous.
The corrected fixture includes a maximum and still tests exact count exports, unknown counts, offsets, and empty inputs.
Raw first-draft profiles remain in `/tmp/rerail-outcomes-20261022-first`.

The scalar-chain rewrite passed all 31 tests present at that point.
On the same 32-root conjunction, preparation fell from 22.194 ms to 2.147 ms.
VM work fell from 44,778 to 37,092; automatic-index entries fell from 634 to 263.
The query retained 49 CTEs, so relation count alone also failed to explain preparation cost.
Repeated references and joins across those relations were the important distinction.

The next profile showed minimum-one counts evaluating every candidate after a known match.
A capture-free, node-local body now lowers to indexed `EXISTS` probes when the traversal supports a direct probe.
SQL first asks whether any candidate is true.
Only if that fails does it ask whether any candidate is unknown.
Unknown is neither true nor false; negation and nested Boolean bodies preserve that distinction.
This rewrite excludes exported counts and bodies with capture inputs or outputs.
Neighbor walks and descendant intervals qualify; other walks retain the generic count relation.

The test harness now compares true and unknown root sets with the native oracle.
It separately compares exact observable node captures, optional captures, and count exports with both engines.
A pure equality helper implements native structural capture equality for composite named nodes.
SQL still owns all rule composition and capture commits.
The profile records equality requests, reached leaf requests, preparation, VM work, EXPLAIN, and logical relation sizes.
Separate diagnostic queries provide those sizes; they do not measure physical writes.

### Explicit outcome kernel accepted — 2026-10-22

All 33 relational tests pass, including 11 outcome tests.
They cover withheld captures, unknown redefinitions, independent false terms, branch rollback, ordered search, exact counts, optional captures, and composite capture witnesses.
Nested Boolean count probes and multiple files also match the native oracle.
Unproved semantic leaves, recursive outcomes, and rule-based stops fail admission explicitly.
Formatting and both `check:native` Clippy configurations pass.

The final profile uses the same source and binary for nine scaling cases:

| Case | Sizes | VM instructions | Regex requests | Automatic-index entries |
| --- | --- | --- | --- | --- |
| Independent conjunctions | 16 / 64 / 256 | 37,092 / 146,676 / 585,012 | 48 / 192 / 768 | 263 / 1,079 / 4,343 |
| Minimum-one count, known first match | 16 / 64 / 256 | 309 / 309 / 309 | 1 / 1 / 1 | 0 / 0 / 0 |
| Minimum-one count, all unknown | 16 / 64 / 256 | 378 / 618 / 1,578 | 17 / 65 / 257 | 0 / 0 / 0 |

Each kernel uses one traced matching execution and zero frame publications.
The known-first count uses three node advances at every width.
Its query has eight CTEs and 1,739 bytes, with no stored candidate relation.
At width 256, the existence rewrite reduces VM instructions from 40,498 to 309, approximately 131-fold.
This compares two versions of this kernel, not whole-job runtime.
All-unknown probes inspect the width once for a true answer, then one candidate for an unknown answer.

The conjunction query has 49 CTEs and 8,854 bytes.
Final preparation is 2.325 ms at size 16, versus 22.194 ms in the initial draft.
Preparation and automatic-index entries remain flagged for this shape.
These costs come from joins across ordered candidate and sequencing relations.
The largest diagnostic relation equals the number of root demands; independent branches do not form a product.
Further reduction should combine adjacent local projections and reduce stored joins at proven choice boundaries.
It must preserve withheld inputs and exact witness selection.

The summary separates measured inefficiencies from admission limitations.
Preparation receives a flag only above one millisecond and twice execution CPU.
Logical cardinalities remain explicitly separate from physical writes.
Raw results, full SQL, EXPLAIN, exact outputs, and hashes accompany the checked-in evidence.

The frozen B023 regression also passes, for 44 unique passing tests in total.
All 138 file hashes match the reference corpus; the two roots and capture witnesses remain exact.
Generated SQL retains SHA-256 `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
All three samples use 2,812,255 VM instructions, one matching execution, and zero frame publications.
Median cold kernel CPU is 126.756 ms generated versus 63.504 ms reference.
The 1.996 ratio passes the twofold gate with almost no timing margin.
This phase claims unchanged B023 SQL and VM work, not a B023 speed improvement.

Artifacts:

- `benchmark/sql-relational-outcomes-evidence.json`: final profiles, exact outputs, source hashes, iteration comparisons, and B023 regression summary.
- `benchmark/sql-relational-outcomes.sql`: generated conjunction query.
- `benchmark/sql-relational-outcomes-count.sql`: generated indexed count query.
- `/tmp/rerail-outcomes-20261022-final-summary`: raw outcome artifacts and logs.
- `/tmp/rerail-outcomes-b023-20261022-final`: raw frozen regression artifacts and logs.

Reproduce the outcome evidence with:

```sh
JEMALLOC_OVERRIDE=/tmp/librerail_jemalloc_check_shim.dylib \
python3 benchmark/relational-outcomes.py /tmp/rerail-outcome-reproduction
```

The driver builds the binary unless `--binary` supplies one.
It hashes all nested relational source files and refuses changed artifacts during measurement.
The existing component and complete-query drivers now hash nested source files too.

Milestone 2 remains incomplete, and the replacement stays test-only.
The next phase must define reached semantic-leaf failures and rule-based traversal stops before lowering them.
Unknown-aware recursion, enumeration, native resource limits, full catalog coverage, and the complete-job 100-execution gate remain pending.
No production switch or fallback was added.

### Milestone 2: reached value leaves and traversal stops — started 2026-10-22

The native traversal order determines which failures are visible.
Ancestor and sibling walks test the stop before the candidate.
Descendant walks test the candidate first and omit the stop after a decisive result.
An unknown stop prevents traversal beyond that boundary.
For descendant walks, later sibling subtrees remain reachable.
For counts, an unknown stop makes the upper bound unbounded.

The next kernel uses a narrow recursive traversal relation for ordered, node-local tests.
It carries no capture environment or global instruction program.
The first semantic admission is a non-following `value` leaf, evaluated through the existing atomic leaf primitive.
The kernel must preserve reached failures and early count thresholds.
It does not claim native resource-budget parity or file-level failure publication across the full job.

The ordered traversal uses three local phases per visited node: first test, second test, then advance.
For descendants, the first test is the candidate; for other axes, it is the stop.
A materialized scalar subquery evaluates each Boolean operand once.
`CASE` guards every later operand and the descendant stop after a decisive candidate.
The recursive row contains ten columns: demand key, file, node, bound, phase, candidate verdict, stop verdict, lower count, uncertain count, and open frontier.

Whole-node captures and nested relational bodies do not yet enter this ordered traversal.
Those forms require a wider observable-state contract.
The existing total structural plans retain their set-based SQL.
Eligible total minimum-one counts retain indexed existence probes.
Only rule-based stops or fallible traversal bodies require the new ordered relation.

Root-kind pruning also needs an effect boundary.
A total prefix of a conjunction can restrict root demand before its first fallible term.
A later kind cannot suppress an earlier value failure.
The fault fixtures check this order, early search success, count thresholds, exact count exports, and withheld inputs.

The first differential run passed three tests and failed three oracle assertions.
The failures exposed two details in the existing error contract.
Native traversal can attempt more value leaves after it records the first failure.
The SQL leaf adapter returns the internal limit code, such as `Value`, rather than its rendered diagnostic message.
The corrected oracle compares calls through the first failure and the exact failure code.
It records later native attempts separately; SQL aborts at the first failing leaf.
Full-file diagnostic ordering and partial-result handling remain unproved.

The first scaling profile used one matching statement and zero automatic-index entries in every case.
Known-first searches used 1,067 VM instructions and one value call at widths 16, 64, and 256.
All-unknown searches used 13,351, 51,319, and 203,191 instructions, with one value call per identifier.
The SQL preserved short-circuiting but materialized constant Boolean identities and nested kind alternatives per candidate.
The next rewrite folds total Boolean kind tests into one scalar predicate and removes identity projections.
Fallible operands retain guarded materialization and one physical evaluation.
Raw first-draft results are in `/tmp/rerail-reached-first-results.json`; this draft did not pass the complete differential suite.

Review found a second root-pruning hazard in the previous outcome kernel.
A later composite can contain a kind check and read a capture withheld by an earlier unknown.
Native evaluation skips that whole composite, including its kind check.
Its kind therefore cannot restrict the root demand.
The fix limits conjunction pruning to the first term and later terms without capture dependencies.
A regression fixture must preserve unknown outcomes on roots outside the skipped kind.

### Reached value and stop kernel accepted — 2026-10-22

All 40 relational tests pass, including seven new reached-evaluation tests.
The evidence records 14 fault fixtures with exact calls through the first failure and matching internal error codes.
Each fixture traces one SQL matching execution, including failed executions.
The extra root-pruning fixture preserves three unknown outcomes outside a withheld number-kind test.
Formatting and both `check:native` Clippy configurations pass.

| Case | Widths | VM instructions | Value calls | Traversal states |
| --- | --- | --- | --- | --- |
| Known first match | 16 / 64 / 256 | 856 / 856 / 856 | 1 / 1 / 1 | 6 / 6 / 6 |
| All identifiers unknown | 16 / 64 / 256 | 9,706 / 37,114 / 146,746 | 16 / 64 / 256 | 99 / 387 / 1,539 |

Each profile uses one traced matching execution, zero frame publications, and zero automatic-index entries.
The recursive relation has three states per visited candidate.
Known-first search visits the opening parenthesis and the first value, then stops.
The unknown case visits every identifier and punctuation node in the arguments.
The profile reports traversal-state cardinality and derives visited candidates separately from physical storage counters.

The scalar rewrite reduces known-first VM work by 19.8 percent and the largest unknown case by 27.8 percent.
Both comparisons use this phase's initial draft, not the production engine or whole-job runtime.
The final query has ten top-level relations and 3,290 SQL bytes.
Preparation takes 0.242–0.329 ms across the six samples.
The width-256 unknown sample uses 1.531 ms of cold kernel CPU with counters enabled.
This excludes parsing, diagnostic cardinality queries, and complete-job setup.

The remaining work is bounded but visible: every visited candidate emits three recursive states and uses point probes to advance.
Total bodies should continue to use direct set predicates or indexed existence probes where their contracts permit that rewrite.
Ordered effects require a sequencing proof before any phase or probe disappears.

The frozen B023 regression passes, for 51 unique passing tests in total.
All 138 file hashes match the reference corpus, with exact roots and capture witnesses.
The query retains SHA-256 `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
Every generated sample uses 2,812,255 VM instructions, one matching execution, and zero frame publications.

The timing gate passes at a median CPU ratio of 1.128, but the samples vary substantially:

| Pair | Generated cold kernel CPU | Reference cold kernel CPU |
| --- | --- | --- |
| 1 | 120.578 ms | 65.064 ms |
| 2 | 307.392 ms | 130.212 ms |
| 3 | 146.943 ms | 150.016 ms |

The medians are 146.943 ms and 130.212 ms.
These timings do not support a B023 speed improvement claim.
The stable regression evidence is unchanged SQL, unchanged VM work, and exact results.

Artifacts:

- `benchmark/sql-relational-reached-evidence.json`: six profiles, 14 fault traces, source hashes, draft comparisons, and frozen regression results.
- `benchmark/sql-relational-reached.sql`: generated ordered traversal with scalar sequencing barriers.
- `/tmp/rerail-reached-20261022-final`: full profiles, generated SQL, and differential logs.
- `/tmp/rerail-reached-b023-20261022-final`: full frozen regression evidence and logs.

Reproduce this phase with:

```sh
JEMALLOC_OVERRIDE=/tmp/librerail_jemalloc_check_shim.dylib \
python3 benchmark/relational-reached.py /tmp/rerail-reached-reproduction
```

The driver builds the test binary unless `--binary` supplies one.
It pins the relational sources, atomic value code, fault hook, terminal scope adapter, and executable hash.
The complete-query regression driver now pins those external atomic-leaf files too.

Milestone 2 remains incomplete, and this compiler remains test-only.
The next phase adds capture-bearing ordered traversal, including the pending-unknown-stop boundary described in the transition plan.
Nested fallible traversal bodies and full-file failure publication follow that contract.
Native resource budgets, enumeration, general recursion, catalog coverage, and the complete-job 100-execution gate remain pending.
No production switch or fallback was added.

### Milestone 2: captures in ordered traversal — started 2026-10-22

Node-local binding operations can retain an input witness or replace it with the current candidate.
The new candidate compiler uses that invariant to track bindings with integer masks.
Its Boolean states contain truth, bound-slot, and withheld-slot masks, rather than copied capture columns.
An input equality check runs only before that slot binds to the candidate.

A true candidate result carries its bound-slot mask beside the truth tag in one signed integer.
False and unknown retain their existing tags and export no captures.
The completed traversal supplies the exact winning node; output columns reconstruct each capture once.
The recursive walk keeps its existing ten columns, regardless of capture width.
The local mask admits at most 61 distinct slots; wider local contracts fail compilation.
Two truth bits and 61 capture bits fit in a nonnegative signed SQLite integer.

A strict descendant search tests the next candidate before acting on a pending unknown stop.
A true candidate wins at that boundary.
A false candidate ends the search unknown before its own stop runs.
An unknown capture-bearing candidate ends the search immediately.
Counts discard probe captures and retain their non-strict interval semantics.
Stop rules start with an empty environment, regardless of caller capture names.

The local candidate program admits predicates, atomic values, whole-node captures, and nested `all`, `any`, and `not`.
It rejects nested traversal before SQL execution; those bodies need a separate reached-demand contract.
Each reached Boolean operand has one materialized test result before the next state reads it.
These scalar barriers preserve failure order and prevent repeated atomic value calls.
They do not publish mutable frames or serialize capture environments.

For one exported capture, the result tag has this form:

```sql
-- truth: 0=false, 1=true, 2=unknown; bit 0 represents the local capture slot.
SELECT CASE WHEN truth=1 THEN 1+((bound&1)<<2) ELSE truth END
FROM candidate_result
```

A winning result of 5 exports the candidate node.
A result of 1 retains the caller's original capture, including an absent optional capture.
Only the final traversal output expands the mask into capture columns.
A correlated lookup by demand key supplies incoming witnesses when equality needs them.
Private stop captures use no caller lookup; count probes discard their result masks.

The first draft passes nine new differential tests, including four fault-injection cases.
The raw log is `/tmp/rerail-capture-walks-first-tests.log`.
The final test set adds incoming-witness scaling, all 61 mask bits, and parent/sibling stop boundaries.
The final generator also omits witness columns from traversals that export no captures.

### Capture-bearing ordered traversal — measured results, 2026-10-22

All 51 relational tests pass, including 11 new tests for this kernel.
They compare native roots, exact capture witnesses, absent captures, count exports, and unknown roots.
The cases cover every structural axis, strict unknowns, no backtracking, Boolean capture rollback, withholding, and private stop environments.
A 61-capture case exercises the highest mask bit and confirms ten recursive columns.
Wider local masks and nested traversal bodies fail admission before execution.

All 18 fault traces pass, including four new capture-bearing cases.
Each trace records one matching execution, including reached failures.
The new traces establish these boundaries:

- A value failure in the next candidate is reached after an unknown descendant stop.
- A false next candidate ends a strict search before its own failing stop runs.
- A successful capture-bearing candidate skips its own failing descendant stop.
- A withheld capture dependency prevents a nested Boolean term from reading its value.

The scaling driver emits SQL, EXPLAIN, source hashes, exact outputs, and separate diagnostic relation counts.
It also reports local candidate CTEs, which the top-level relation count alone would omit.

| Dimension | Size 16 VM | Size 64 VM | Size 256 VM | Value requests |
| --- | ---: | ---: | ---: | --- |
| First candidate matches | 1,258 | 1,258 | 1,258 | 1 at every size |
| Last candidate matches | 16,283 | 64,043 | 255,083 | 1 at every size |
| Independent roots | 17,728 | 70,432 | 281,248 | 16 / 64 / 256 |
| Incoming capture witnesses | 16,948 | 68,452 | 274,468 | 0 at every size |

Every profile uses one traced matching execution and zero frame publications.
Incoming-witness searches issue 15, 63, and 255 atomic equality checks.
EXPLAIN shows `SEARCH d USING AUTOMATIC COVERING INDEX (k=?)` for the correlated input lookup.
The two root-scaling series build 60, 252, and 1,020 automatic-index entries across their query joins.
This work grows with root count; it does not scan the entire input relation for every candidate.

Known-first search emits six recursive rows and visits two nodes at every width.
Late-match search emits 96, 384, and 1,536 recursive rows and visits 32, 128, and 512 nodes.
Independent roots emit the same three row counts.
Incoming-witness search emits 93, 381, and 1,533 rows, with 31, 127, and 511 visited nodes.
These are logical CTE cardinalities from diagnostic queries, not physical write or allocation counts.

The queries contain seven to eleven local candidate CTEs and 5,138–5,626 SQL bytes.
Preparation takes 0.496–0.686 ms in these samples.
The largest cold kernel sample takes 3.961 ms with counters enabled.
Cold kernel timing excludes parsing and diagnostic queries; it does not establish a whole-job speedup.

The remaining local cost is visible in the late-match case: about 498 VM instructions per visited node.
Its Boolean state barriers still execute after a total prefix has already rejected that candidate.
Atomic values remain guarded, but the later scalar projections still consume VM work.
A future rewrite can collapse those false-prefix states only if it preserves withholding and reached failures.
The current measurements establish linear growth and exact semantics, not a minimal instruction count.

The six capture-free ordered profiles were rerun as a regression control.
Their generated SQL is byte-identical to the previous phase, with unchanged VM counts.
Known-first remains at 856 VM instructions; unknown-width remains at 9,706, 37,114, and 146,746.
The raw results are `/tmp/rerail-capture-walks-capture-free-20261022.json`.

The first frozen B023 run passes all correctness tests and the VM gate, but narrowly fails the timing gate.
The generated query retains SHA-256 `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
All 138 file hashes match, and both exact roots retain their expected capture witness.
Every generated sample uses 2,812,255 VM instructions, one matching execution, and zero frame publications.

| First-run pair | Generated cold kernel CPU | Reference cold kernel CPU |
| --- | ---: | ---: |
| 1 | 159.657 ms | 81.792 ms |
| 2 | 161.847 ms | 79.125 ms |
| 3 | 164.140 ms | 77.460 ms |

The median ratio is 2.04546 against a maximum of 2.0.
This is a failed gate, despite unchanged SQL and VM work.
A separate confirmation run was selected before its results, with the first run retained.
The final report includes both runs and the combined median; it does not select the faster run.

The confirmation run passes the existing timing gate at a median ratio of 1.90483:

| Confirmation pair | Generated cold kernel CPU | Reference cold kernel CPU |
| --- | ---: | ---: |
| 1 | 151.082 ms | 75.742 ms |
| 2 | 150.389 ms | 81.690 ms |
| 3 | 166.206 ms | 79.315 ms |

Across all six pairs, the medians are 160.752 ms and 79.220 ms: a ratio of 2.02919.
The combined result remains above 2.0, so the timing gate stays open.
The unchanged query and VM count establish unchanged SQL work; they do not establish an acceptable timing ratio.
No further runs were selected to obtain a passing result.
This phase completes the node-local capture implementation, with this performance qualification retained for the transition.

Validation totals 62 unique passing correctness tests: 51 relational, nine B023 fixtures, one adversarial test, and the frozen corpus.
Formatting and both default and `code-rule-profile` Clippy checks pass.
The profiles and both frozen runs pin the same executable and the matching source hashes.

Artifacts:

- `benchmark/sql-relational-capture-walks-evidence.json`: twelve profiles, eighteen fault traces, capture-free controls, and both frozen runs.
- `benchmark/sql-relational-capture-walks.sql`: the generated known-first capture query.
- `/tmp/rerail-capture-walks-20261022-final`: full profiles, SQL, EXPLAIN, and differential logs.
- `/tmp/rerail-capture-walks-b023-20261022-final`: the retained first frozen run, including its failed timing gate.
- `/tmp/rerail-capture-walks-b023-20261022-confirmation`: the confirmation run.

Reproduce the capture profiles with:

```sh
JEMALLOC_OVERRIDE=/tmp/librerail_jemalloc_check_shim.dylib \
python3 benchmark/relational-capture-walks.py /tmp/rerail-capture-walks-reproduction
```

The driver builds the test executable unless `--binary` supplies one.
Use `benchmark/relational-complete-spike.py` with the frozen corpus configuration for the separate B023 gate.

The compiler remains test-only, with no new runtime option or fallback.
The next semantic kernel is nested fallible traversal with typed child results and explicit parent resumption.
The transition plan gives its first two-array fixture and required failure boundaries.
Full-file publication, native resource budgets, general recursion, enumeration, and complete-job execution accounting remain pending.
The B023 timing gate also remains open; the unchanged VM count does not waive it.

### Milestone 2: nested reached traversal — started 2026-10-22

The next kernel returns distinct outer and inner capture witnesses from one matching statement.
The first fixture searches two arrays in an arguments node.
The first array binds `A`, then its inner number binds `X` and supplies an atomic value test.
A decisive first match must prevent any value read in the second array.

The physical boundary uses a correlated SQL row-value subquery:

```sql
CROSS JOIN relation_row r ON (r.c0,r.c1,r.c2) IS (
  WITH RECURSIVE child_input AS MATERIALIZED (...), ...
  SELECT truth,cA,cX FROM child_result
)
```

`relation_row` supplies one row from its integer or NULL constraints.
It copies these columns without AST access, rule evaluation, SQL execution, or persistent result storage.
Its cursor admits only a contiguous constrained prefix and rejects unconstrained scans.
SQL owns every child rule, stop, Boolean operation, and recursive step.
The profiler counts supplied rows and cells separately from frame publications.

Each nested walk carries ten control columns plus its observable child exports.
Incoming captures remain in keyed demand rows and enter a child only when that child is reached.
A count or stop requests only the child truth column.
Stop programs use an empty capture environment.
Separate recursive arms preserve descendant body-before-stop order and ancestor/sibling stop-before-body order.
The first admission limit permits four traversal levels.

An isolated experiment exposed a materialization hazard before full-engine validation.
Two consumers of a correlated CTE executed its atomic prefix twice, despite `AS MATERIALIZED`.
Four expected value calls became eight.
The rewrite gives the prior state one consumer and passes its computed columns into a separate child scope.
A permanent row-constructor test requires four calls and exact NULL preservation.
The same correction also applies to compound Boolean operands in the compact node-local candidate compiler.

### Nested reached traversal — implementation and validation

The implementation is part of the test-only relational compiler.
It adds no production switch or whole-rule native fallback.
The native engine remains the independent semantic oracle.

The new cases cover distinct outer and inner witnesses, caller capture equality, Boolean restoration, withheld captures, private stops, and exact count exports.
They also cover pending unknown stops, skipped failing tails, parent/sibling order, and the four-level admission limit.
The row constructor tests cover NULL, all 129 columns, rejected unconstrained scans, and one evaluation per input.

The first full run passed 60 tests and failed an obsolete nested-stop refusal assertion.
Replacing that assertion exposed a legacy compiler failure: `no such table: u` for a synthetic unknown stop.
The replacement compiler was not the source of that failure.
A dedicated regression now compares exact true/unknown roots directly with native evaluation for that fixture.
Other profile cases still compare the native engine, existing SQL compiler, and replacement compiler.
All 62 relational tests then passed.
Formatting and both required Clippy configurations also passed.

The normal optimized test build spent over 50 minutes in local ThinLTO code generation before interruption.
The retained compiler sample identifies that build stage; it does not indicate a query performance problem.
Validation used a temporary Rust compiler wrapper that adds `-C lto=off` only to the `rerail_engine` test executable.
The resulting build completed in 59 seconds.
The evidence stores the wrapper text, binary hash, and source hashes for reproduction.
The repository's production build configuration is unchanged.
Absolute CPU comparisons against previous phases' binaries would be invalid.
The frozen B023 generated/reference comparison uses the same new executable for both shapes.

The first successful profiling run overlapped Clippy and supplies semantic and deterministic work checks only.
The retained performance run followed completion of Clippy and preceded the B023 run.
No matching workload or compiler source changed between those two runs.

| Workload | Sizes | VM instructions | Value calls | Typed rows / cells |
| --- | --- | --- | --- | --- |
| Known first inner match | 16 / 64 / 256 | 2,363 / 2,363 / 2,363 | 1 / 1 / 1 | 4/10 at every size |
| Late inner match | 16 / 64 / 256 | 15,983 / 59,567 / 233,903 | 16 / 64 / 256 | 4/10 at every size |
| Late outer match | 16 / 64 / 256 | 38,423 / 153,815 / 615,383 | 16 / 64 / 256 | 64/160, 256/640, 1,024/2,560 |
| Independent roots | 16 / 64 / 256 | 35,438 / 141,278 / 564,638 | 16 / 64 / 256 | 64/160, 256/640, 1,024/2,560 |

Every sample has exact native outputs, one traced matching execution, and zero frame publications.
The evidence includes 30 reached-call traces across the current semantic suite.
Typed row/cell counts measure actual cursor inputs before any diagnostic queries.
They are copies and must not be described as zero intermediate work.

Each sample generates the same 8,997-byte query with 23 profiled relations across lexical scopes.
Nine additional local candidate CTEs implement its compact Boolean program.
The outer recursive state has 12 columns: ten controls and the two distinct capture witnesses.
The inner local traversal has ten columns.
Preparation takes 0.640–0.843 ms; measured cold kernel CPU ranges from 0.953 to 6.105 ms.
These are diagnostic kernel measurements, not whole-job timing or native resource-limit parity.

EXPLAIN shows a kind-indexed root scan, node point probes, and automatic key indexes at outer demand/result joins.
Independent roots create 60, 252, and 1,020 automatic-index entries; the other dimensions create none.
The recursive child remains correlated and creates local single-demand materializations.
That preserves reached evaluation but still costs about 2,400 VM instructions per array in the late-outer workload.
Each array adds both an array candidate and an intervening sibling token to that walk.
The next physical optimization is to collapse proven single-consumer Boolean barriers without repeating atomic work.
Three traversal states per candidate and repeated node probes are another measured cost to address.
The current result establishes linear growth and correct early termination, not an optimal constant factor.

Full-file failure publication is the next semantic phase.
Use two files and two clauses to prove which results survive a reached failure, then encode that publication contract.
General recursion, enumeration, following values, native budgets, and the complete-job 100-execution gate remain open.

The frozen B023 regression passes on 138 files with two exact matches.
Generated SQL retains SHA-256 `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1` and 2,812,255 VM instructions in all three pairs.
Median cold kernel CPU is 118.520 ms generated versus 60.454 ms reference: a 1.96052 ratio, within the 2.0 gate.
The previous phase's 2.02919 combined ratio remains recorded; this run does not establish a cross-build timing improvement.
All 12 previous capture scaling profiles retain identical SQL, VM work, value calls, one matching execution, and zero frame publications.
The full validation selection passes 73 unique tests: 62 relational, nine B023 fixtures, one adversarial case, and one frozen-corpus case.

Artifacts:

- [Reproduction driver](../benchmark/relational-nested-walks.py).
- [Profile, EXPLAIN, reached traces, source hashes, and regression evidence](../benchmark/sql-relational-nested-walks-evidence.json).
- [Generated nested query](../benchmark/sql-relational-nested-walks.sql).
- Raw profiles: `/tmp/rerail-nested-20261022-profile`.
- Frozen regression: `/tmp/rerail-nested-b023-20261022-final`.
- Previous capture control: `/tmp/rerail-nested-capture-control-20261022`.

Reproduce with the evidence's test-build command, then pass that executable to these drivers:

```sh
python3 benchmark/relational-nested-walks.py /tmp/new-nested-profile --binary target/loop/deps/rerail_engine-11d3550fad116d17
python3 benchmark/relational-complete-spike.py /tmp/rerail-b023-relational-20261022/final/config.json /tmp/new-nested-b023 --binary target/loop/deps/rerail_engine-11d3550fad116d17
```

The output directories must be new.
Compiler artifact names can change; use the executable path reported by the build.
The evidence includes the temporary wrapper contents needed to reproduce this diagnostic build.

### Milestone 2: failure publication — started 2026-10-22

This phase tests two files and two clauses with successful roots before and after an injected value failure.
The native clause runner stops further roots in the failed clause/file pair.
Workspace reporting discards all witnesses from that pair, including earlier successful roots.
Other files and clauses remain independent.
The public diagnostic uses `budget-exceeded` and the native limit message.

The stateless snapshot evaluator has a different aggregation contract.
It discards the entire clause result after a file limit and stops that query's file scan.
The replacement matching kernel will expose typed failures by clause/file pair.
Workspace publication can then preserve healthy pairs without confusing this contract with stateless query aggregation.
The phase must record this distinction and test each claimed boundary.

The planned SQL shape sequences roots only within each clause/file pair.
Each root invokes its compiled SQL child through the existing typed row boundary.
Negative outcome tags carry reached limits through Boolean and traversal composition.
A final failure relation excludes every successful root from a failed pair.
No host retry, per-root SQL execution, or mutable failure side table supplies composition.

The first differential run passed the existing kernels but found two mistakes in the new fixtures.
The helper selected dialect variants from the first clause instead of selecting each clause's TypeScript matcher.
A count fixture also omitted the required maximum for a zero minimum.
The corrected harness checks the number of clauses and compares every clause/file pair.

A separate native workspace test uses a real value-budget failure, without fault injection.
Its failed file retains one provisional witness internally, but reporting publishes none from that clause/file pair.
The healthy file and the sibling clause still publish their witnesses.
A stateless API test confirms that its failure discards the whole clause result instead.

The first correct publication shape used a separate final output arm for each clause.
Each arm referenced the shared failure query.
At 2, 8, and 32 clauses, preparation took 0.869, 7.924, and 116.319 ms.
Execution took 0.105, 0.449, and 2.480 ms, with linear VM work.
This is unacceptable preparation amplification for a small workload.
The rejected shape and its source hashes remain in `/tmp/rerail-publication-20261022-final`.

The next rewrite unions typed provisional outputs before one failure filter.
Columns are clause, file, node, outcome, and the maximum observed capture width in this bounded batch.
Capture positions follow each clause's published name order.
Only the final projection converts count values to text and constructs JSON.
A fallible root stream has one consumer and no join back to its root demand for output identity.
Its recursive relation can use a coroutine instead of forced materialization.
The shared typed publication relation is materialized because the failure filter and final results both read it.
This materialization has an explicit consumer contract and a measurable width.

### Failure publication — accepted kernel and profiles

The final shape passes 69 relational tests and the native workspace publication test.
The B023 selection adds nine fixtures, one adversarial case, and one frozen-corpus case: 81 unique tests pass.
Formatting and both required Clippy configurations pass.
The evidence contains 27 publication traces, including file/clause reversals and multiple failures.

The compiler uses one failure representation and one publication contract for workspace-local matching.
Atomic value limits return negative integer tags instead of raising a SQLite statement error.
Composition propagates the tag and suppresses later reached work.
Unexpected terminal or SQL errors remain statement errors.
The decoder uses the existing public diagnostic code, native message, clause ID, and path.
Public diagnostic order uses the existing result-order function.

Reached failure tests exercise `Value` limits.
Encoding tests cover all four native limit tags, but do not establish their budget-trigger parity.
The normal single-clause fallible compiler now uses the same publication kernel as the batch compiler.
There is no runtime switch or host retry loop.
The replacement remains test-only.

| Dimension | Sizes | Final VM instructions | Atomic value calls | Typed return rows / cells |
| --- | --- | --- | --- | --- |
| Roots, two clauses | 4 / 16 / 64 | 3,597 / 13,449 / 52,857 | 8 / 32 / 128 | 8/16, 32/64, 128/256 |
| Roots after first failure, two clauses | 4 / 16 / 64 | 1,668 / 3,396 / 10,308 | 2 / 2 / 2 | 2/4 at every size |
| Files, two clauses | 4 / 16 / 64 | 3,981 / 15,369 / 60,921 | 8 / 32 / 128 | 8/16, 32/64, 128/256 |
| Clauses, one root/file | 2 / 8 / 32 | 1,134 / 4,448 / 18,068 | 2 / 8 / 32 | 2/4, 8/16, 32/64 |

All 12 profiles have exact native outputs, one traced matching execution, and zero frame publications.
The five-column typed publication relation carries one capture in these workloads.
Typed return counts measure physical cursor inputs, not all SQLite temporary writes.

At 32 clauses, final preparation takes 10.455 ms versus 116.319 ms for the rejected shape: about 11.1 times lower.
VM work falls from 19,233 to 18,068, a 6.1% reduction.
The final statement is 80,154 bytes.
These are diagnostic shape measurements, not a whole-job speedup claim.
At 64 roots, automatic-index entries fall from 253 to 126 after removal of the root-output join.

EXPLAIN still materializes the recursive root states, despite leaving that choice to SQLite.
Removing forced materialization did not establish streaming execution.
The accepted improvements are the shared failure filter, typed publication boundary, and removed root-output join.
Candidate-key construction still grows after an early failure, although value calls remain constant.
Preparation still dominates small jobs with many clauses; the profiler flags that cost.

The new correctness contract adds work to existing fallible kernels.
All 30 prior reached, capture, and nested profiles retain exact outputs, atomic call counts, one execution, and zero frame publications.
Their VM counts rise by approximately 5–32% for error propagation and root sequencing.
The known-first nested case rises from 2,363 to 2,759 instructions and remains constant across inner widths.
At 256 independent roots, the nested case rises from 564,638 to 654,029 instructions and remains linear.
The control run overlapped Clippy, so only its deterministic counters are used.
This overhead remains visible as a future physical-planning target.

The final B023 run retains its exact two matches across 138 files.
Its SQL SHA-256 remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
All three samples use 2,812,255 VM instructions.
Median cold kernel CPU is 115.236 ms generated versus 61.790 ms reference, a 1.86498 ratio.
Both B023 gates pass.
The earlier publication draft also passed at 1.69508; neither ratio implies a cross-build speedup.

The fixtures admit all supplied TypeScript files to each clause.
Mixed-dialect batches and per-clause file selection remain integration work.
The kernel admits at most 32 clauses and 256 KB of SQL before a measured partitioning policy is required.
Native work accounting is the next semantic phase, followed by the remaining depth, enumeration, recursion, and production gates.
The complete cold job still must prove the 100-execution budget.

Artifacts:

- [Reproduction driver](../benchmark/relational-publication.py).
- [Accepted profiles, rejected-shape comparison, traces, and source hashes](../benchmark/sql-relational-publication-evidence.json).
- [Accepted 32-clause SQL](../benchmark/sql-relational-publication.sql).
- [Rejected 32-clause SQL](../benchmark/sql-relational-publication-rejected.sql).
- Raw accepted profiles: `/tmp/rerail-publication-20261022-accepted`.
- Raw rejected profiles: `/tmp/rerail-publication-20261022-final`.
- Frozen regression: `/tmp/rerail-publication-b023-20261022-accepted`.

The evidence records the diagnostic allocator and test-only LTO wrapper used in the preceding phase.
Production build settings are unchanged.
Reproduce with a fresh output directory and the executable path reported by that build:

```sh
python3 benchmark/relational-publication.py /tmp/new-publication-profile --binary target/loop/deps/rerail_engine-11d3550fad116d17
```

### 2026-10-23 — Native work allowance through typed outcomes (in progress)

The next transition phase carries native work allowance through SQL composition.
The previous kernel lifted the allowance at every value leaf.
It also erased utility entry charges and reduced total counts without charging candidate visits.

The implementation adds a two-column atomic value cursor: truth and remaining allowance.
SQL passes the allowance to each reached leaf and carries the returned allowance through child programs.
The host has no shared budget accumulator.
Each clause/file root sequence owns its allowance.
Counts charge a step before each tested candidate, including punctuation and failed kind tests.
Utility entry remains explicit after lowering.

The first regression run found a test-oracle mismatch in root selection.
An older reached-call test invoked `match_root` on every node; production uses `run_clause` and native anchor kinds.
The bounded publication compiler now uses those native root kinds.
The reached-call oracle now also uses `run_clause`.
This changes the test with a value leaf followed by an incompatible kind: production never reaches that leaf.

Validation is in progress. Work-limit parity, profiling results, and admission limits are not yet claimed.

#### Work-budget milestone: implementation and admission

The bounded outcome compiler now has one work-accounting path.
`relation_value_budget(term, file, node, allowance)` returns two typed columns: truth and remaining work.
A NULL allowance denotes an unreached operand and performs no native leaf call.
Negative allowances are rejected.
The previous unlimited `relation_value` function has been removed from this compiler.

A root state carries `(key, file, node, ordinal, truth, work, captures)`.
A nested child returns `(truth, work, captures)` through the existing row-value bridge.
The bridge now admits 130 columns: truth, work, and 128 captures.
It still only copies SQL values; it cannot compose rules or execute SQL.

Each count candidate consumes one work unit before its body runs.
A zero allowance produces `Work` before any value leaf runs.
An exhausted value decoder produces `Value`.
Both failures suppress the entire clause/file pair, including earlier provisional matches.
Unrelated pairs retain their own allowance.
The final output carries remaining work as column six for the test-only contract.

The native reference now installs file value caches and runs actual clauses.
The comparison checks reached value-call order separately for each clause/file pair.
It excludes native probes attempted after a limit is already recorded.
It also checks exact captures, remaining work on published rows, failures, and public diagnostics.
Nine new tests cover 217 allowance-boundary jobs and 12 scaling jobs.

Two admission checks prevent incorrect resource claims:

- Value-bearing utility bodies require memo charge replay. Native charges a remembered utility's original cost even when its value is cached.
- Two-valued charged searches require native candidate pruning. Native can skip punctuation before invoking a utility; three-valued searches visit it.

The first rejection has a concrete regression fixture.
Two identical utility invocations over one number cost four units natively.
With three units, the second invocation fails with `Work`.
Simply inlining it would spend three units and incorrectly succeed after a value-cache hit.
The compiler refuses this case pending a keyed utility cost relation.
Pure utility entry charges are retained and tested, including repeated calls, searches, and counts.

The second rejection came from a differential failure.
`has(matches(number))` over `f(1,2,3)` succeeds natively with allowance one in a two-valued clause.
The unfiltered SQL walk instead spent its first unit on punctuation and then failed.
The three-valued version correctly charges that visit.
The compiler now rejects the unproved two-valued charged search instead of silently changing its budget behavior.

#### Physical shape: reject unnecessary child materialization

The first correct budget implementation moved every fallible local walk into the general typed child kernel.
This preserved results but increased a capture-free, 256-candidate control from 154,657 to 336,563 VM instructions.
That was a 2.18-fold regression against the preceding compiler, which lacked work accounting.

The accepted planner recognizes a node-local value leaf preceded by total, capture-free predicates.
It emits one guarded atomic cursor rather than a demand CTE plus a materialized result CTE.
Total node-local child tests also return their result directly.
Unknown or capture-bearing prefixes retain the general composition contract.
Aliases are compiler inputs; the rewrite does not edit or reparse generated SQL.
There is no runtime switch between shapes.

The same 256-candidate control now uses 172,664 VM instructions.
This removes 48.7% of the rejected implementation's work.
It remains 11.6% above the preceding compiler because allowance propagation has a cost.
Reached calls remain exactly 256 in all three cases.
The known-first control remains constant as its unused tail grows.

The finite-budget count profile also improves from 144,632 to 112,008 VM instructions at width 64.
Both versions execute exactly 128 value leaves across two files.
These are physical improvements to the new accounting contract, not evidence of complete-job performance parity.

#### Final evidence and qualification

Validation passed for 90 unique tests: 78 relational tests, one workspace publication test, nine B023 fixtures, one adversarial test, and one corpus test.
Formatting and both default and `code-rule-profile` Clippy configurations passed.
The 217 allowance-boundary jobs and 12 scaling jobs compare native results and reached calls.
The evidence contains those 229 traces plus 27 publication traces from the preceding phase's fixtures.

All 12 finite-budget profiles use one matching execution and zero frame publications.
At zero allowance, value calls remain two as roots per file grow from four to 64.
Candidate enumeration still grows; a sibling total clause also processes every root in that fixture.
The statement count does not include cold database setup and does not establish the complete-job 100-execution gate.

Thirty preceding-phase controls retain exact outputs, outcomes, and value-call counts.
Their additional VM work is 5.3–7.0% for nested traversal, 1.7–13.9% for capture walks, and 8.5–11.6% for capture-free reached walks.
The evidence retains the larger regression from the rejected child-materialization shape.
Allowance correctness has a measured cost; this phase does not claim a global performance improvement.

The frozen B023 query remains byte-for-byte unchanged:
`84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
Both runs report 138 files, two matches, and 2,812,255 generated-query VM instructions in all three pairs.
Both pass the VM gate.

The earlier run passed the CPU gate at 102.208 ms generated versus 60.772 ms reference: 1.68182×.
The final-source run failed narrowly at 126.279 ms versus 62.313 ms: 2.02653×, against the 2× limit.
Other workloads were active on the host. That observation does not establish the cause of the variation.
Both results are retained. The final timing gate remains unresolved; the earlier pass does not replace it.
Production admission and the complete-job budget remain pending.

Committed evidence:

- `benchmark/sql-relational-work-budget-evidence.json`: source hashes, 12 profiles, 256 traces, 30 work-count controls, publication controls, and both B023 timing gates.
- `benchmark/sql-relational-work-budget.sql`: finite-budget count and publication query.
- `benchmark/sql-relational-work-value-walk.sql`: accepted capture-free walk query.
- `benchmark/sql-relational-work-value-walk-rejected.sql`: rejected materialized child query.

Raw final profiles are in `/tmp/rerail-work-budget-20261023-accepted`.
The earlier correct but slower shape is in `/tmp/rerail-work-budget-20261023-final`.
Final B023 evidence is in `/tmp/rerail-work-budget-b023-20261023-accepted`; its driver exits with the retained CPU-gate failure.
The preceding run is in `/tmp/rerail-work-budget-b023-20261023-final`.
The evidence's revision names the parent commit; source hashes identify the measured working tree.

The build uses the same diagnostic allocator shim and test-only local-LTO wrapper as the previous phase.
Production build settings remain unchanged.
Reproduce the finite-budget profiles with a new output directory:

```sh
python3 benchmark/relational-work-budget.py /tmp/new-work-budget-profile --binary target/loop/deps/rerail_engine-11d3550fad116d17
```

To collect the 30 control samples, run the outcome tests with these output variables:
`RERAIL_RELATIONAL_NESTED_EVIDENCE`, `RERAIL_RELATIONAL_CAPTURE_WALK_EVIDENCE`, and `RERAIL_RELATIONAL_REACHED_EVIDENCE`.
`RERAIL_RELATIONAL_PUBLICATION_EVIDENCE` collects the preceding publication fixtures.
Each variable names a JSON output file.
The committed evidence supplements the driver's output with these work-count comparisons and both frozen B023 gate reports.

The next transition action is the local utility memo-cost fixture documented in the transition plan.
The broader work-budget contract remains open until memo replay and two-valued candidate pruning are admitted.

### 2026-10-23 — Local utility memo costs in typed SQL

This phase starts from `2612d778` and addresses the next transition fixture.
A repeated value utility must replay its original work cost, even after the value decoder caches its answer.
At allowance three, two calls to a one-value utility must fail with `Work`.
At allowance four, they must succeed with zero work left and one value-leaf call.

The first implementation preserves utility identity during lowering.
It stores typed truth, measured cost, input captures, and output captures inside one Boolean demand.
File and node identity belong to that demand.
A reached hit replays cost before it returns the stored answer.
Failures and unreached invocations do not create reusable answers.
The native key includes all incoming captures for value utilities, including unrelated captures.
The compiler therefore preserves those inputs through the demand boundary.

The initial native comparisons pass for true, false, unknown, count exports, changed captures, and previously cached values.
Cross-traversal, nested-utility, and cross-Boolean reuse remain explicit admission failures.
This phase does not establish global utility sharing or change production admission.

The initial SQL retains a guarded miss body for each repeated invocation.
The next measurement compares that shape with direct replay when static capture versions prove an identical key.
Profiles will include preparation, VM instructions, typed return cells, EXPLAIN, and reached value calls.

#### Measured shape change

The baseline passed 84 relational tests and retained 434 publication traces.
Its 32-call true fixture generated 44,263 SQL bytes, used 8,411 VM instructions, and copied 136 typed return cells across two files.
Only two value leaves ran. The remaining return copies came from guarded miss subqueries at later call sites.

The compiler now tracks static capture versions within a Boolean demand.
An identical version proves that a reached call can replay an existing answer directly.
Conjunction withholding only grows, and a decisive alternative stops later alternatives.
Therefore, an identical later call cannot become reached after its earlier call was skipped.
A possible capture write invalidates this proof and retains a typed key comparison with a guarded miss body.
This is a SQL generation decision, not a runtime option.

Before final column retirement, the direct-replay fixture used 20,557 SQL bytes, 5,063 VM instructions, and 12 typed return cells.
The 12-cell count stayed constant at two, eight, and 32 repetitions.
False and unknown fixtures retained exact native charges and calls, with similar VM reductions.
Preparation timings varied; individual timing samples do not establish a general speedup.

The compiler also skips memo allocation for single-use utilities and drops answer columns after their last use.
A regression fixture rejects more than eight allocated key versions before SQL execution.
The storage bound is deliberately conservative and counts allocated versions, including versions whose columns have retired.

#### Final evidence and qualification

Validation passed for 98 unique tests: 86 relational tests, one workspace publication test, nine B023 fixtures, one adversarial test, and one corpus test.
The eight memo tests include 198 native-comparison jobs, including 12 profiles.
The full relational suite produced 454 publication traces.
Formatting and both default and `code-rule-profile` Clippy configurations passed.

All 12 memo profiles use one matching SQL execution and zero frame publications.
The 32-call true fixture now uses 5,059 VM instructions and 20,539 SQL-file bytes, including the final newline.
The rejected shape used 8,411 VM instructions and 44,264 SQL-file bytes.
This is a 39.9% VM reduction and a 53.6% SQL-size reduction for this fixture.
Typed return cells fall from 136 to 12 and remain constant as repetitions grow.
EXPLAIN materialization sites fall from 111 to 80.
Preparation falls from 9.305 ms to 4.031 ms in these individual instrumented samples.
These timings are observations, not a general performance guarantee.

At 32 roots per file, the memo fixture uses 38,195 VM instructions versus 41,779 in the rejected shape.
Distinct roots still require distinct utility answers.
The fixture retains two files, exact native work, and one leaf invocation per root.
The compiler does not claim sharing across different root demands.

The 54 preceding-phase control samples have identical SQL, output, and VM counts.
These include 30 traversal controls, 12 finite-allowance profiles, and 12 publication profiles.
Their reached calls and outcome comparisons also pass.
The memo change adds no SQL work to those existing shapes.

The frozen B023 corpus still has 138 files, two matches, and 2,812,255 generated-query VM instructions in every pair.
The query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
The final run passes both gates: 124.478 ms generated versus 63.890 ms reference, or 1.94833× against the 2× CPU limit.
The preceding phase's 2.02653× failure remains in its evidence.
The query and VM counts did not change. This pass does not establish a B023 speedup from memo compilation.

Committed evidence:

- `benchmark/relational-utility-memo.py`: pinned-source profiling driver with native parity and constant-return-width assertions.
- `benchmark/sql-relational-utility-memo-evidence.json`: 198 memo traces, 12 profiles, rejected-shape comparisons, 54 unchanged controls, and the B023 gates.
- `benchmark/sql-relational-utility-memo.sql`: final 32-call query.
- `benchmark/sql-relational-utility-memo-rejected.sql`: the same fixture with a guarded miss body at every call.

Raw memo runs are in `/tmp/rerail-utility-memo-20261023-{baseline,replay,final}`.
The final B023 run is in `/tmp/rerail-utility-memo-b023-20261023-final`.
Source hashes identify the measured tree; the recorded revision is the parent commit.
The build retains the diagnostic allocator shim and test-only local-LTO wrapper from the preceding phase.

Reproduce the memo profiles with a new output directory:

```sh
python3 benchmark/relational-utility-memo.py /tmp/new-utility-memo-profile --binary target/loop/deps/rerail_engine-11d3550fad116d17
```

This completes local utility cost replay within the bounded outcome kernel.
Cross-Boolean, cross-traversal, and nested value-utility sharing remain explicit admission failures.
The next implementation action is native candidate pruning for two-valued charged searches.
Start with `has(matches(number))` over `f(1,2,3)` at allowance one, as specified in the transition plan.
Production integration, broader semantic gates, and the complete cold-job 100-execution gate remain open.

### 2026-10-23 — Native candidate pruning for charged searches

This phase starts from `9be0dc0c` and addresses the next transition fixture.
A two-valued `has(matches(number))` over `f(1,2,3)` must succeed with one utility charge.
The corresponding three-valued search visits punctuation before the number and needs two charges.

Native search uses two different filters.
All two-valued searches apply a kind filter before the candidate body.
Descendant searches to the end also apply required-text clauses through the native posting index.
Neighbor searches and searches with stop rules do not apply that text filter.
Stop rules still run at nodes the body skips.
Counts retain their separate per-candidate charge contract.

The compiler now reads the native filter analysis through a test-only interface.
Required-text disjunctions lower to atomic literal regex tests; SQL owns the conjunction and traversal.
The first measurement keeps the existing traversal and guards the body before utility charges.
The intended physical shape uses indexed, ordered candidate seeks for descendant searches without stop rules.
This should remove recursive rows for irrelevant nodes while preserving native work and witness order.

The new fixtures compare finite allowances, negated failed searches, captures, nested searches, count bodies, and stop order.
Profiles separate sparse kinds, sparse text, dense failures, known-first success, and stop-pruned subtrees.
The replacement remains test-only. Complete-job execution and production admission remain separate gates.

#### Guarded baseline

The guarded baseline passes 90 relational tests and records 325 pruning jobs, including 15 profiles.
At 256 irrelevant siblings, the sparse-kind fixture uses 286,901 VM instructions and 2,068 typed return cells across two files.
Its sparse-text counterpart uses 288,981 VM instructions and the same return-cell count.
Each file has one successful utility invocation.
The recursion still visits irrelevant AST nodes, despite correct utility charges.
Known-first success and stop-pruned subtrees already retain constant VM work as their unused tails grow.

The default `has` fixture uses immediate children, not descendant-to-end traversal.
Native visits its opening punctuation but applies the kind filter before the utility charge.
Its successful allowance-one visit count is therefore two, while its charge is one.
The first test expected one visit; that assertion was corrected without changing the compiler.
Native candidate counters include cleanup visits after a limit and are reported as diagnostic counts.
They are not asserted equal to SQL visits after a failure.

The candidate-seek implementation uses the node virtual table's existing ordered kind postings.
Each seek constrains file, lower and upper node IDs, and candidate kinds, then applies required-text clauses.
`ORDER BY id LIMIT 1` supplies the next eligible descendant.
Only eligible candidates enter the existing charged traversal.
The node index still has an input-dependent construction cost; these profiles do not establish constant complete-job cost.
Stop-bearing and neighbor searches retain the guard shape, so stop evaluations keep their native order.

#### Indexed seeks and the first-candidate boundary

The first seek implementation passes 91 relational tests and 345 pruning jobs.
The sparse-kind fixture uses 1,707 VM instructions at all three widths: four, 32, and 256 irrelevant siblings.
At width 256, this is 168× less VM work than the guarded baseline.
Typed return cells fall from 2,068 to 16 across two files.
The sparse-text fixture falls from 288,981 to 6,871 VM instructions.
Dense failures still visit genuine candidates; their width-256 work falls from 326,876 to 176,834 VM instructions.
Stop-rule profiles remain unchanged.

The first seek shape evaluates the initial text scan twice: once to project its candidate and once to exclude an empty seed.
Its width-256 sparse-text profile advances 1,036 node rows across two files.
The final shape materializes that first candidate once when a required-text filter exists.
The boundary stores only demand ID, file, candidate, subtree bound, and remaining work.
Caller captures stay in their existing demand relation.
Kind-only seeks retain the smaller inline seed because they already use direct bounded postings.
The profiling driver checks constant VM work for irrelevant kind tails and checks the text-scan advance bound.

#### Final evidence and qualification

Validation passed for 103 unique tests.
These include 91 relational tests, one workspace publication test, nine B023 fixtures, one adversarial test, and one corpus test.
The six pruning tests cover 355 native-comparison jobs, including 15 profiles.
The full relational suite records 809 publication traces.
Formatting and both default and `code-rule-profile` Clippy configurations passed.
The final boundary tests include empty first roots and nested subtrees with matching nodes outside their upper bounds.
They preserve captures, outcomes, and remaining native work.

All 15 profiles use one matching SQL execution and zero frame publications.
At width 256 across two files, the final measurements are:

| Fixture | Guarded VM instructions | Final VM instructions | Guarded return cells | Final return cells |
| --- | ---: | ---: | ---: | ---: |
| Sparse kinds | 286,901 | 1,707 | 2,068 | 16 |
| Sparse text | 288,981 | 4,302 | 2,068 | 16 |
| Dense false candidates | 326,876 | 176,834 | 3,084 | 2,056 |
| Known-first success | 2,229 | 1,707 | 20 | 16 |
| Stop-pruned subtree | 3,570 | 3,570 | 44 | 44 |

The sparse-kind query stays at 1,707 VM instructions as irrelevant siblings grow from four to 256.
The sparse-text query still examines eligible kind postings for its required text.
Sharing its first candidate reduces node advances from 1,036 to 524 and VM instructions from 6,871 to 4,302.
EXPLAIN shows candidate cursors with file, both ID bounds, and kind constraints: `f=i>i{k=`.
The node cursor supplies ascending candidate IDs for `ORDER BY id LIMIT 1`.

The sparse-kind SQL grows from 5,118 to 5,494 bytes, including the final newline.
Its individual preparation samples are 0.881 ms guarded and 0.809 ms final.
Preparation variation does not establish a timing improvement.
The work-count reduction comes from removing irrelevant traversal states, not from smaller SQL text.

All 66 preceding-phase controls retain identical SQL, output, and VM counts.
These include 30 traversal controls and 12 profiles each for allowances, publication, and utility memo replay.
The native outcome and reached-call assertions also pass.

The frozen B023 corpus retains 138 files, two matches, and 2,812,255 generated-query VM instructions in all three pairs.
Its query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
The final run passes both gates: 117.875 ms generated versus 64.309 ms reference, or 1.83296× against the 2× CPU limit.
The query and VM counts did not change. This pass does not establish a B023 speedup from candidate pruning.

Committed evidence:

- `benchmark/relational-candidate-pruning.py`: pinned-source profiling driver with constant-tail and text-scan assertions.
- `benchmark/sql-relational-candidate-pruning-evidence.json`: 355 traces, 15 profiles, both rejected-shape comparisons, 66 controls, and the B023 gates.
- `benchmark/sql-relational-candidate-pruning.sql`: final sparse-kind query.
- `benchmark/sql-relational-candidate-pruning-text.sql`: final sparse-text query with its narrow first-candidate materialization.
- `benchmark/sql-relational-candidate-pruning-guarded.sql`: correct charges with irrelevant recursive states still present.
- `benchmark/sql-relational-candidate-pruning-repeated-seed.sql`: indexed text seeks with the first scan repeated.

Raw runs are in `/tmp/rerail-candidate-pruning-20261023-{guard,seek,final,accepted}`.
The `accepted` run includes the final subtree-boundary fixtures; its work counts match the preceding `final` run.
The frozen B023 run is in `/tmp/rerail-candidate-pruning-b023-20261023-final`.
The recorded revision is the parent commit; source hashes identify the measured working tree.
The build retains the diagnostic allocator shim and test-only local-LTO wrapper.

Reproduce the pruning profiles with a new output directory:

```sh
python3 benchmark/relational-candidate-pruning.py /tmp/new-candidate-pruning-profile --binary target/loop/deps/rerail_engine-11d3550fad116d17
```

This completes candidate pruning for the admitted two-valued outcome searches.
Dense failures still pay for three traversal phases and a utility program per genuine candidate.
Those materializations remain a physical planning cost; the sparse-case gains do not remove it.
The next semantic transition is utility cost replay across nested Boolean scopes within one root demand.
The transition plan specifies the rejected fixture and its exact allowance thresholds.
Production integration, broader semantic gates, and the complete cold-job 100-execution gate remain open.


### 2026-10-23 — Utility answers across nested Boolean scopes

This phase extends measured utility cost replay within one root demand.
The starting fixture repeats a value utility across a double negation.
Allowance three must fail. Allowance four must succeed with one value call and zero remaining work.

The first proposed shape placed immutable typed answers at the smallest Boolean scope shared by both calls.
Nested Boolean operators used reached demand relations in that scope.
Later calls joined prior answers by demand, file, node, and all input captures.
Only successful misses could publish answers. A replay consumed the original measured cost.
This proposal failed the reached-call checks described below.

Validation covers branch order, unknowns, capture rollback, changed keys, failures, and bounded storage.
Profiles include EXPLAIN, VM instructions, typed return cells, and existing controls.
Existing direct sibling replay retains its cheaper row-local specialization.
Traversal and nested value-utility sharing remain separate admission gates.
The frozen B023 gates and the complete cold-job limitation remain unchanged.


The first physical shape failed reached-call parity before profiling.
It joined separate immutable answer CTEs inside a correlated root program.
SQLite reevaluated their correlated dependencies despite `MATERIALIZED`, duplicating value reads.
Allowance one produced two leaf calls where native execution produced one.
That shape was rejected.

The replacement uses one dependency chain through the shared Boolean scope.
Each barrier references its predecessor once and projects only live state.
Nested branches retain typed binding snapshots until commit or rollback.
Memo answers survive those boundaries and retire after their last use.
A guaranteed first invocation with no bindings allows direct replay across later branches.
Other invocations retain guarded lookup, including misses after an earlier skipped call.


The first dependency-chain build passes six tests and 628 native-comparison jobs.
All 15 profiles use one matching statement and zero frame publications.
At 32 repeated nested calls, it uses 8,943 VM instructions and 12 typed return cells across two files.
EXPLAIN contains 178 materialization sites. SQL size is 39,552 bytes.
The corresponding existing direct-sibling profile uses 5,059 VM instructions.
This comparison measures nesting overhead, not a speedup over the previously rejected fixture.

The next physical pass removes redundant double negations when the child exports no bindings.
It also propagates replay proofs through sequential siblings in regions without captures.
A later reached sibling proves that the preceding sibling was reached.
Only that sibling's unconditional first call exports a proof. Optional branch calls do not.
This permits direct replay after an earlier syntactic call was skipped.
Binding-bearing regions retain dynamic key comparisons and conservative storage limits.
Explicitly false guards prevent an unreached nested scope from replaying a cached charge.


The replay-proof pass adds unreachable nested-scope fixtures and passes 804 comparison jobs.
At 32 repeated calls, VM work falls to 6,791 and materialization sites fall to 114.
Typed return cells stay at 12 across two files.
The skipped-first fixture now supports 32 calls with 16 return cells, independent of repetition count.
The earlier eight-call shape copied 40 cells because it retained guarded miss programs.

EXPLAIN still shows one redundant materialization after every atomic result.
The atomic predecessor already stores the reached call or replay.
The next pass lets SQLite inline the following projection, which only renames results and commits bindings.
Boolean sequencing barriers remain materialized to bound expression expansion and preserve reached work.


#### Final evidence and qualification

The final shape passes 109 unique tests.
These include 97 relational tests, one workspace publication test, nine B023 fixtures, one adversarial test, and one corpus test.
The six new tests record 804 native-comparison jobs, including 15 profiles.
The full relational suite records 1,613 publication traces.
Formatting and both Clippy configurations pass.

The core fixture now matches native execution.
Allowance three fails with `Work`. Allowance four succeeds with zero remaining and one value-leaf call per file.
Negation rolls back bindings while retaining answers and spent work.
Changed input captures select distinct memo keys, including unrelated bindings.
Unknown answers and counts retain their measured costs and exported values.
Failed and unreached calls cannot become reusable answers.

Every profile uses one matching SQL statement and zero frame publications.
The following measurements cover two files:

| Fixture | First chain VM | Replay-proof VM | Final VM | First chain return cells | Final return cells |
| --- | ---: | ---: | ---: | ---: | ---: |
| 32 repeated nested calls | 8,943 | 6,791 | 5,771 | 12 | 12 |
| 32 false alternatives | 9,161 | 6,813 | 5,729 | 12 | 12 |
| 32 unknown alternatives | 9,043 | 6,759 | 5,675 | 12 | 12 |
| Eight calls, first skipped | 5,012 | 3,092 | 2,766 | 40 | 16 |
| 32 roots, two calls each | 47,283 | 43,637 | 41,717 | 384 | 384 |

The repeated-call fixture uses 35% fewer VM instructions than the first correct chain.
SQL size falls from 39,552 to 27,144 bytes, including the final newline.
EXPLAIN materialization sites fall from 178 to 82.
The final query retains sequencing barriers but inlines projections after stored atomic results.
Its individual preparation sample is 6.899 ms, versus 10.341 ms for the first chain.
These individual wall-time samples do not establish a timing improvement.

The skipped-first fixture now supports 32 calls with 6,390 VM instructions and 16 typed return cells.
It evaluates the value once per file. Repetition does not increase return-cell copies.
The unchanged direct-sibling 32-call control uses 5,059 VM instructions.
The final nested shape therefore retains about 14% more VM work than that simpler equivalent fixture.
This is a physical-shape comparison on newly admitted programs, not a speedup over an earlier correct cross-scope implementation.

All 81 preceding-phase controls retain identical SQL, output, and VM counts.
These comprise 30 traversal controls, 12 allowance profiles, 12 publication profiles, 12 local memo profiles, and 15 pruning profiles.
The frozen B023 run retains 138 files, two matches, and 2,812,255 generated-query VM instructions in every pair.
Its query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
Both gates pass: 128.493 ms generated versus 65.364 ms reference, or 1.96580× against the 2× CPU limit.
The unchanged SQL and VM counts do not establish a B023 speedup.
Earlier passing and failing CPU samples remain in the preceding-phase evidence.

Committed evidence:

- `benchmark/relational-shared-utility-memo.py`: pinned-source profiling driver with reached-call and return-cell assertions.
- `benchmark/sql-relational-shared-utility-memo-evidence.json`: 804 traces, 15 profiles, both shape comparisons, rejected-DAG diagnostics, 81 controls, and B023 gates.
- `benchmark/sql-relational-shared-utility-memo.sql`: final 32-call query.
- `benchmark/sql-relational-shared-utility-memo-skipped.sql`: final query with an earlier skipped call.
- `benchmark/sql-relational-shared-utility-memo-first-chain.sql`: first correct chain before physical simplification.

Raw profiles are in `/tmp/rerail-shared-memo-20261023-{chain,proven,final}`.
The frozen B023 run is in `/tmp/rerail-shared-memo-b023-20261023-final`.
The recorded revision is the parent commit. Source hashes identify the measured working tree.
The build retains the diagnostic allocator shim and test-only local-LTO wrapper.

Reproduce the nested memo profiles with a new output directory:

```sh
python3 benchmark/relational-shared-utility-memo.py /tmp/new-shared-memo-profile --binary target/loop/deps/rerail_engine-11d3550fad116d17
```

This completes the nested Boolean milestone for the bounded outcome kernel.
The scope carries its capture columns and live memo answers through one dependency chain.
Temporary binding snapshots and Boolean results retire at scope exit.
Replay proofs currently require a region without captures. Other regions use dynamic comparisons and conservative storage limits.
The memo limit remains eight allocated answer entries and 512 typed columns. Shared sequencing state also has a 512-column limit.
These remain test-only admission bounds, not production workload limits.
The next transition extends the same answer lifetime through nested value-bearing utility bodies at one root.
The transition plan specifies the initial fixture and its expected finite-allowance boundary.
Cross-traversal sharing, production integration, broader semantic gates, and the complete cold-job 100-execution gate remain open.


### 2026-10-23 — Nested value-utility answers

This phase starts from `4391fb57` and extends answer lifetime through utility bodies at one root.
The first fixture repeats an outer utility whose body repeats an inner value utility.
The expected native boundary is nine charges failing and ten succeeding, with one value read.
The existing compiler rejects nested value-bearing utilities.
The first check records native execution and that admission failure before changing the compiler.

The intended shape keeps the existing single dependency chain.
An outer miss charges its entry, runs its body in that scope, and stores the total measured cost.
An outer replay consumes that cost without repeating the body.
Inner answers remain available after the outer call, including false and unknown answers.
Traversal and cross-root sharing remain separate gates.


Native execution confirms the boundary: allowance nine fails with `Work`, and ten publishes one match with zero work remaining.
Both runs read the value once.
The compiler then fails with its nested-utility admission error, as expected.
The rejected run is retained in `/tmp/rerail-nested-memo-rejected.log`.


The first implementation passes five new tests and 669 native-comparison jobs.
All 15 profiles use one matching statement and zero frame publications.
Across two files, eight outer calls use 3,076 VM instructions and 12 typed return cells.
Seven nested utility levels use 4,987 VM instructions and the same 12 return cells.
The skipped-first profiles retain 16 return cells as the call count grows.

EXPLAIN shows redundant materializations around utility entry and return.
The next physical pass inlines first-miss lookup projections because they only copy constants and input work.
Dynamic key lookups remain materialized.
Outcome and answer projections inline into the materialized return boundary.
That boundary limits expression expansion across nested utilities.
Profiles also record the typed column count at each sequencing projection.


#### Final evidence and qualification

The final implementation passes 116 unique tests.
These include 104 relational tests, one workspace publication test, nine B023 fixtures, one adversarial test, and one corpus test.
The seven new tests record 704 native-comparison jobs, including 15 profiles.
The full relational suite records 2,317 publication traces.
Formatting and both default and `code-rule-profile` Clippy configurations pass.

The initial allowance boundary remains exact: nine fails, while ten succeeds with zero work remaining and one value read per file.
The false-outer fixture needs five charges before its direct inner replay can finish.
A warm value cache does not reduce the recorded utility cost.
The three-level shared-call fixture fails at 32 and succeeds at 33, with one value read per file.
Capture rollback, changed keys, unknowns, count exports, distinct utility identities, and reached failures retain native parity.
Oversized nested answer storage fails during compilation.

The compiler activates the shared scope at the smallest caller that spans the relevant utility calls.
An outer miss snapshots only bindings its body can write.
It records input work before the entry charge and subtracts remaining work after the reached body finishes.
The difference is the complete reusable cost.
Inner answers stay in the dependency chain after the outer return.
Proofs from optional body calls do not escape their utility boundary.
A proven outer replay omits the body and updates compile-time use counts for its skipped inner calls.
There is no runtime option or native composition fallback.

All 15 profiles use one matching SQL statement and zero frame publications.
Across two files, the final work counts are:

| Fixture | First-chain VM | Final VM | First-chain return cells | Final return cells |
| --- | ---: | ---: | ---: | ---: |
| Eight outer calls | 3,076 | 3,010 | 12 | 12 |
| Eight inner calls per outer body | 3,172 | 3,106 | 12 | 12 |
| Eight roots, two outer calls each | 15,720 | 15,192 | 96 | 96 |
| Seven nested utility levels | 4,987 | 3,825 | 12 | 12 |
| Eight calls, first skipped | 4,232 | 4,064 | 16 | 16 |

The seven-level fixture uses 23% fewer VM instructions than the first correct chain.
EXPLAIN materialization sites fall from 57 to 36.
SQL size falls from 24,971 to 24,698 bytes, including the final newline.
Its widest materialized sequencing projection has 40 typed columns. The widest projection has 43.
These widths describe generated columns, not runtime row counts.
Its preparation sample is 4.472 ms, versus 4.175 ms for the first chain.
The VM reduction does not establish a preparation-time improvement.

Repeated outer calls and repeated inner calls retain one value read per root.
Typed return cells stay constant per root in all five profile families.
The larger depth improvement comes from removing redundant entry and return materializations at each nested level.
Capture-sensitive dynamic lookups still have guarded miss bodies and conservative answer-storage limits.
Broader replay proofs across those boundaries remain a possible physical optimization.

All 96 preceding-phase controls retain identical SQL, output, and VM counts.
These include the prior 81 controls and 15 shared-Boolean memo profiles.
The frozen B023 corpus retains 138 files, two matches, and 2,812,255 generated-query VM instructions in every pair.
Its query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
The VM gate passes.
The CPU gate fails: 124.463 ms generated versus 61.358 ms reference, or 2.02845× against the 2× limit.
The preceding phase measured 1.96580× with the same SQL and VM counts.
The failed run is retained without repeating it to obtain a passing sample.
The unchanged query does not establish either a B023 speedup or a structural regression from this phase.
Performance qualification remains open.

Committed evidence:

- `benchmark/relational-nested-utility-memo.py`: pinned-source profiling driver with value-call, return-cell, and state-width summaries.
- `benchmark/sql-relational-nested-utility-memo-evidence.json`: 704 traces, 15 profiles, native admission failure, shape comparisons, 96 controls, and B023 gates.
- `benchmark/sql-relational-nested-utility-memo.sql`: final eight-call query.
- `benchmark/sql-relational-nested-utility-memo-depth.sql`: final seven-level query.
- `benchmark/sql-relational-nested-utility-memo-skipped.sql`: final query with an earlier skipped call.
- `benchmark/sql-relational-nested-utility-memo-first-chain.sql`: seven-level query before physical simplification.

Raw profiles are in `/tmp/rerail-nested-memo-20261023-{chain,final}`.
The frozen B023 run is in `/tmp/rerail-nested-memo-b023-20261023-final`.
The recorded revision is the parent commit. Source hashes identify the measured working tree.
The build retains the diagnostic allocator shim and test-only local-LTO wrapper.

Reproduce these profiles with a new output directory:

```sh
python3 benchmark/relational-nested-utility-memo.py /tmp/new-nested-memo-profile --binary target/loop/deps/rerail_engine-11d3550fad116d17
```

This completes nested value-utility support for the bounded outcome kernel.
The memo limit remains eight allocated answer entries and 512 typed columns.
Shared sequencing state also has a 512-column limit.
The kernel remains test-only, and the failed frozen CPU gate remains visible.
The next transition addresses repeated candidate visits through traversal and overlapping roots.
Its answer lifetime must span a clause/file pair rather than reset at each root.
The transition plan specifies two failing fixtures and their expected allowance boundaries for native confirmation.
Production integration, broader semantic gates, and the complete cold-job 100-execution gate remain open.

### 2026-10-23 — Traversal utility answer lifetime

The next fixtures repeat a value utility through descendant searches and overlapping roots.
Native boundaries and the existing admission rejection must be recorded before implementation.

The proposed physical shape keeps reached utility answers in an indexed, integer-only scratch relation.
Its key contains clause, utility, file, candidate, and incoming captures. Its payload contains truth, measured cost, and exports.
SQL performs lookup, guards the miss program, computes its cost, and publishes only completed nonfailure answers.
The scratch cursor stores supplied integers. It cannot inspect ASTs, evaluate rules, or execute SQL.
A materialized clause initializer clears that clause's storage once per statement execution, including prepared-statement reuse.
Root and traversal recursion retain their current widths; answer maps do not travel inside recursive rows.

This storage introduces observable writes during SELECT execution. Explicit dependency barriers must prove lookup, miss, and publication order.
Tests must include repeated execution, skipped calls, failed misses, capture changes, nested utilities, and independent files and clauses.
The compiler chooses this storage for clauses whose value utilities cross traversal boundaries.
Existing local typed-column specialization remains valid where answer lifetime stays inside one root.
No user option selects the storage shape. Performance and semantic results are pending.

Native preflight confirms the planned boundaries. The repeated search fails at three and succeeds at four.
Overlapping roots fail at seven and succeed at eight, with two matches and one value read.
The original compiler rejects both programs. The saved rejection log records this before admission changed.

The first implementation passes all 110 relational tests except the new scratch-storage unit test.
That test exposes a planner issue for literal `IS NULL` constraints, which SQLite reports separately from `IS`.
The cursor now accepts both constraint forms. The test includes prepared-statement reuse and null capture keys.

The first profiles expose a separate SQL amplification problem:
32 overlapping roots across two files need 3,624,895 VM instructions and 33,536 typed return cells.
Only two answers are stored, and only two value reads occur. Memo lifetime alone does not fix traversal work.
Each search still walks irrelevant candidates in its nested subtree.

The physical change seeks bounded kind postings when a pure prefix proves rejection before effects.
It applies to descendant searches without stop rules. Counts retain every charged visit.
The compiler records this proof in the profile. Recursive row width does not change.
The scratch cursor also switches to sparse typed arguments and payloads, avoiding a full-width allocation per lookup.
Final qualification and before/after measurements remain pending.

Final qualification:

- All 123 unique tests pass: 111 relational tests, one workspace test, nine B023 fixtures, one adversarial test, and one corpus test.
- Six new traversal tests cover 723 comparison jobs, including 15 profiles. The former admission test adds 26 native comparisons.
- The storage test covers sparse slot 127, null keys, skipped publication, rejected failure publication, and prepared-statement reuse.
- The retained suite contains 3,066 publication traces. All 111 preceding control profiles have identical SQL, output, and VM counts.
- Formatting and both Clippy configurations pass. The optimized test build uses the recorded jemalloc shim and disables local test-executable LTO.
- Each new profile executes one matching statement and publishes zero legacy frames. This does not establish the complete cold-job execution budget.

| Fixture, across two files | First-walk VM | Final VM | First-walk return cells | Final return cells |
| --- | ---: | ---: | ---: | ---: |
| Two overlapping roots | 17,575 | 7,263 | 176 | 80 |
| Eight overlapping roots | 236,767 | 28,095 | 2,240 | 320 |
| 32 overlapping roots | 3,624,895 | 111,423 | 33,536 | 1,280 |
| 32 repeated searches | 71,009 | 44,321 | 776 | 520 |
| 32 disjoint roots | 166,845 | 113,469 | 1,792 | 1,280 |
| Early match, 32 siblings | 5,459 | 3,791 | 56 | 40 |

The 32-root overlap uses 32.5× fewer VM instructions. Scaling from eight to 32 roots now increases work by approximately four times.
EXPLAIN reports bounded candidate access as `SCAN candidate VIRTUAL TABLE INDEX 0:f=i>i{k=`.
Both versions perform two value reads and store two utility answers for the overlap fixture.
The final representation stores 24 integer cells for those answers, independent of root count.
Traversal rows remain 11 columns. Memo storage adds zero recursive columns.

The capture-width profiles use two, four, and eight incoming bindings.
They store six, ten, and 18 answers across two files, occupying 84, 160, and 360 integer cells.
Native execution requires a fresh value read for each changed key.
The first profiling assertion incorrectly expected one read per file. The evidence retains that failure and its correction.
The Rust implementation already matched native execution; only the profiling assertion changed.

Repeated-search preparation remains expensive. At 32 repetitions, generated SQL is 184,607 bytes with 623 EXPLAIN materialization sites.
That sample spends 54.33 ms preparing and 3.12 ms executing. These are instrumented single samples, not a CPU qualification result.
The transition plan now specifies shared traversal compilation for a proved, adjacent repeat group, followed by guarded semantic-cost replay.

The frozen B023 query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
It reports 138 files, two matches, and 2,812,255 generated VM instructions in all three pairs.
The VM gate passes. Median generated CPU is 131,351,500 ns versus 63,057,625 ns for the reference: **2.083039×**.
The 2× CPU gate fails. This run is retained without a retry for a passing sample.
Unchanged SQL and VM counts do not support a structural B023 regression or speedup claim.

Artifacts:

- `benchmark/relational-traversal-utility-memo.py`: reproducible native comparisons, EXPLAIN, storage counters, and profile assertions.
- `benchmark/sql-relational-traversal-utility-memo-evidence.json`: source/binary hashes, 723 traces, 15 profiles, 12 physical comparisons, controls, and failed gates.
- `benchmark/sql-relational-traversal-utility-memo.sql`: final overlap query.
- `benchmark/sql-relational-traversal-utility-memo-first-walk.sql`: overlap query before bounded kind seeks.
- `benchmark/sql-relational-traversal-utility-memo-repeats.sql`: remaining repeated-program amplification.
- `benchmark/sql-relational-traversal-utility-memo-captures.sql`: widest capture-key fixture.
- Raw logs: `/tmp/rerail-traversal-memo-20261023-{first,final,accepted}` and `/tmp/rerail-traversal-memo-b023-20261023-final`.

The scratch relation uses writes during SELECT evaluation. Materialized dependencies and reached-call comparisons establish the tested ordering contract.
The test-only connection runs one matching job at a time. Interleaved jobs on a shared connection are outside this contract.
Production integration, general recursive outcomes, native depth limits, deadlines, and the whole-job 100-execution gate remain open.

### 2026-10-23 — Compiled repeated traversal groups

The next phase removes duplicated traversal programs from adjacent identical searches in a conjunction.
The first supported group has no capture effects or stop rule. Each candidate has a pure predicate prefix and one value-bearing utility call.
Utility identity, prefix identity, traversal direction, and unchanged incoming captures establish equality between calls.

The first traversal must run normally. It fills every reached utility answer in clause/file storage.
Later calls visit the same candidates and replay those answers, so their semantic costs equal the first traversal's measured cost.
False and failed first outcomes stop the conjunction. True and unknown outcomes permit repeated-cost replay.
An insufficient replay allowance produces Work failure without entering another child program.
The SQL must guard multiplication against signed integer overflow before subtracting the repeated cost.

The proposed shape compiles one child program, materializes its two-column result once, and applies guarded cost replay.
It must preserve exact charges, failure tags, reached leaves, provisional publication, and incoming captures.
Unwrapped value leaves, captures, stops, counts, different utilities, and nonidentical prefixes remain outside the grouping proof.
Native boundary checks and baseline profiles precede the compiler change. Results are pending.

Native preflight confirms the boundaries for three calls before compiler changes:

- A true search needs six charges and one value read per file.
- A false search needs four charges, reads both candidates, and stops the conjunction after the first search.
- An unknown search needs 12 charges and reads both candidates once.
- Two overlapping roots need 12 charges and one value read. Exhaustion on the second root discards provisional matches.

The first grouped implementation passes all seven new semantic tests.
The repeat count becomes a SQL integer constant; it does not duplicate the traversal program.
For 32 true searches across two files, SQL size falls from 184,607 to 8,671 bytes and VM instructions fall from 44,321 to 2,505.
Typed return cells fall from 520 to 24.
Two repetitions use one fewer VM instruction than eight or 32: SQLite reuses an existing constant register for the value one.
The profiling assertion permits this single initialization instruction while requiring constant return cells and bounded SQL growth.

The proof excludes unwrapped value leaves because warm-cache charges can differ.
It also excludes changed prefixes, different utility identities, capture effects, counts, stop rules, and nonadjacent searches.
Stable incoming captures are retained, and a binding between groups starts a separate group.
Nested utility costs remain measured by the first traversal and replayed in full.
Signed-limit SQL tests verify division before multiplication, including false and failed first outcomes.
Final qualification is pending.

Final qualification:

- All 131 unique tests pass: 119 relational tests, one workspace test, nine B023 fixtures, one adversarial test, and one corpus test.
- Seven new comparison tests cover 662 jobs, including 15 profiles. One additional SQL test covers signed-limit replay arithmetic.
- The suite retains 3,728 publication traces. Formatting and both Clippy configurations pass.
- All 111 preceding controls retain identical SQL, output, and VM counts.
- All 15 traversal profiles from the preceding phase improve in VM work, with identical native outcomes, captures, charges, and reached value calls.
- Every new profile executes one matching statement and publishes zero legacy frames. The whole-job execution budget remains unproved.

| 32-call fixture, across two files | Before VM | After VM | Before return cells | After return cells | SQL bytes after |
| --- | ---: | ---: | ---: | ---: | ---: |
| True search | 44,321 | 2,505 | 520 | 24 | 8,671 |
| Unknown search | 81,409 | 3,633 | 904 | 36 | 8,659 |
| False search | 7,475 | 3,571 | 160 | 36 | 8,671 |
| Two overlapping roots | 88,293 | 4,691 | 1,040 | 48 | 8,671 |
| Stable incoming capture | 45,554 | 2,746 | 522 | 26 | 9,621 |

True-search SQL falls from 184,607 to 8,671 bytes, a 95.3% reduction.
VM work falls by 17.7×. EXPLAIN materialization sites fall from 623 to 35.
The instrumented preparation sample falls from 56.21 ms to 2.32 ms; this is not a production CPU qualification result.
The two-call group uses 2,504 VM instructions, while eight and 32 calls use 2,505. All three use 24 return cells.
The three-byte SQL difference comes from the replay multiplier's extra digit at 32 calls.

The preceding phase's 32-root overlap falls further from 111,423 to 70,270 instructions, with return cells falling from 1,280 to 768.
Its 32 disjoint roots fall from 113,469 to 72,316 instructions.
Eight capture changes fall from 32,021 to 19,421 instructions. Each changed key still gets its own answer and native value read.
No group crosses a capture-changing operand.

The frozen B023 query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
It reports 138 files, two matches, and 2,812,255 generated VM instructions in all three pairs.
Both gates pass in this run. Median generated CPU is 131,297,750 ns versus 69,982,958 ns for the reference: **1.876139×**.
The preceding 2.083039× failure remains recorded. Generated CPU is almost unchanged; the reference sample is slower.
Unchanged SQL and VM counts do not support a structural B023 speedup or stable CPU qualification claim.
The optimized test build uses the recorded jemalloc shim and disables local test-executable LTO.

Artifacts:

- `benchmark/relational-repeat-walk.py`: native comparisons, group-shape assertions, EXPLAIN, counters, and pinned source/binary hashes.
- `benchmark/sql-relational-repeat-walk-evidence.json`: 662 traces, 15 before/after profiles, unchanged controls, improved traversal controls, and frozen gates.
- `benchmark/sql-relational-repeat-walk.sql`: the 32-call true group.
- `benchmark/sql-relational-repeat-walk-unknown.sql`: the 32-call unknown group.
- `benchmark/sql-relational-repeat-walk-capture.sql`: stable incoming captures.
- The previous full query remains in `benchmark/sql-relational-traversal-utility-memo-repeats.sql`.
- Raw logs: `/tmp/rerail-repeat-walk-20261023-{before,final}` and `/tmp/rerail-repeat-walk-b023-20261023-final`.

The next phase applies the same proof to alternatives, with a true outcome as the group's stop condition.
The frontend still expands utility bodies and retains its 256-operator bound. SQL generation now avoids repeated child programs within the proved group.
Production integration, native depth limits, deadlines, general recursive outcomes, and the complete-job 100-execution gate remain open.

### 2026-10-23 — Repeat sharing in alternatives

This phase extends the existing repeat-group proof to adjacent identical searches inside `any`.
The compiler must preserve the first true outcome as the stop condition.
False and noncapturing unknown outcomes continue, with the original charge for each reached alternative.
The alternative retains its accumulated unknown flag and can reach a later fallback.

The proposed change reuses the signature proof, first traversal, and guarded cost projection.
The Boolean operator selects the stop condition during SQL generation.
There is no new runtime option or separate composition engine.
Native boundary checks and 18 baseline profiles precede the compiler change.
The profiles cover 2/8/32 alternatives for true, false, and unknown outcomes, with and without a fallback.
Results are pending.

Native preflight passes before compiler changes:

- Three true alternatives stop after the first search: two charges and one value read.
- Three false alternatives consume 12 charges and read each of two candidates once.
- Three unknown alternatives also consume 12 charges and read each candidate once.
- Two overlapping roots with true searches consume four charges and one value read.
- One fewer charge fails in each case. Failure on the second root discards the first provisional match.

The baseline driver now records SQL, EXPLAIN, VM instructions, preparation time, and typed return cells.

The ungrouped 32-alternative baseline exposes duplicated SQL even when native utility answers are cached.
Across two files, false alternatives use 185,057 SQL bytes, 623 materialization sites, 81,215 VM instructions, and 904 typed return cells.
True alternatives stop early but still compile the same 623 materialization sites and use 6,699 VM instructions.

The implementation now selects the stop predicate through a compile-time Boolean enum.
It preserves the existing conjunction SQL verbatim and adds the alternative stop condition to the same replay projection.
The group profile records `stopTruth` for review.
The ordinary alternative barrier still owns unknown accumulation, fallback reachability, and capture publication.
Qualification is pending.

The first semantic run passed six tests and failed the later-fallback fault case.
At zero allowance, SQL reported Work failure while the native test oracle reported Value failure without a reached value call.
The reached-call probe suppressed cleanup calls from its trace but still injected faults into those calls.
A cleanup probe therefore replaced the original Work failure with an injected Value failure.
Production value decoding preserves an existing failure.

The test probe now suppresses both recording and fault injection after a limit in reached-only mode.
The full-call probe mode keeps its existing behavior. Production matcher behavior is unchanged.
The failing fallback case remains in the regression suite.
The retained first-run log is `/tmp/rerail-repeat-any-first-tests.log`.

The corrected probe and compiler pass all 127 relational tests.
Seven new comparison tests cover 973 jobs, including 18 profiles. A separate SQL test covers signed-limit arithmetic.
The suite retains 4,701 publication traces.
Every new profile has one matching statement and zero legacy frame publications.
The complete cold-job execution budget remains unproved.

| 32-alternative fixture, across two files | Before VM | After VM | Before return cells | After return cells | SQL bytes after |
| --- | ---: | ---: | ---: | ---: | ---: |
| True | 6,699 | 2,495 | 148 | 24 | 8,766 |
| False | 81,215 | 3,629 | 904 | 36 | 8,766 |
| Unknown | 81,609 | 3,655 | 904 | 36 | 8,754 |
| True with fallback | 6,804 | 2,600 | 148 | 24 | 9,319 |
| False with fallback | 81,458 | 3,872 | 904 | 36 | 9,319 |
| Unknown with fallback | 81,842 | 3,888 | 904 | 36 | 9,307 |

For false alternatives, VM work falls by 22.4× and SQL size falls by 95.3%.
EXPLAIN materialization sites fall from 623 to 35, or from 625 to 37 with a fallback.
The preparation sample falls from 142.41 ms to 2.85 ms for the false fixture.
Concurrent activity affects these instrumented wall-time samples, so they do not establish a production speed ratio.
The VM counts and SQL text provide the structural comparison.
All six dimensions retain constant return cells and at most one extra VM instruction across 2/8/32 alternatives.

Final semantic and workload qualification passes 139 unique tests.
These include 127 relational tests, one workspace test, nine B023 fixtures, one adversarial test, and one corpus test.
All 141 preceding profiles retain identical SQL, outputs, and VM counts.
The new 18 profiles also retain the baseline's native traces, charges, and reached value calls.
Formatting, workspace Clippy, and Clippy with the profiling feature pass. Measured source and SQL artifact hashes match the committed evidence.

The frozen B023 query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
It reports 138 files, two matches, and 2,812,255 generated VM instructions in all three pairs.
Median generated CPU is 122,091,875 ns versus 66,254,875 ns for the reference: **1.842761×**.
Both gates pass in this run. Prior timing failures remain recorded, and unchanged SQL and VM counts show no structural B023 improvement.
The optimized test build uses the recorded jemalloc shim and disables local test-executable LTO.

Artifacts:

- `benchmark/relational-repeat-any.py`: native comparisons, group-shape assertions, EXPLAIN, counters, and pinned source/binary hashes.
- `benchmark/sql-relational-repeat-any-evidence.json`: 973 traces, 18 before/after profiles, 141 unchanged controls, and frozen gates.
- `benchmark/sql-relational-repeat-any.sql`: the 32-alternative false group.
- `benchmark/sql-relational-repeat-any-true.sql`: the true group with early stopping.
- `benchmark/sql-relational-repeat-any-unknown-fallback.sql`: unknown outcomes followed by a successful fallback.
- `benchmark/sql-relational-repeat-any-before.sql`: the previous false-group query, with all 32 traversal programs.
- Raw logs: `/tmp/rerail-repeat-any-20261023-{before,final}` and `/tmp/rerail-repeat-any-b023-20261023-final`.

The next phase lowers each acyclic utility body once and retains its charge at every call site.
It must expose distinct plan and terminal counts and prove the current expansion-limit failure before changing admission.
Production integration, native depth limits, deadlines, general recursive outcomes, and the complete-job 100-execution gate remain open.

### 2026-10-23 — Shared immutable utility plans

This phase removes repeated utility-body lowering before SQL generation.
The baseline fixture has 32 identical descendant searches in `all` or `any`.
Its outer utility contains eight distinct regex predicates and one value-bearing inner utility.
The expected native boundaries are 96 charges for true `all`, three for true `any`, six for false `all`, and 192 otherwise.
The existing compiler must demonstrate its 256-expanded-operator rejection before implementation changes.

The planned representation stores one immutable utility body per table identity and utility index within a clause.
Every source call retains its own charge operation and metadata.
Cached effect summaries prevent recursive analysis from expanding the shared graph again.
A separate composition bound must reject excessive SQL expansion after the existing repeat-group proof applies.
Distinct lowered operators, capture width, traversal depth, and SQL size retain explicit bounds.

Baseline instrumentation now reports lowered operators, utility calls, distinct bodies, body lowerings, and atomic terminals.
Compiler wall time is separate from SQL preparation and execution.
Baseline semantic checks and profiles are pending.

Native preflight passes at every expected boundary. One fewer charge fails in each case.
All six wide 32-search fixtures fail compilation with `expanded operator count exceeds 256` before the change.
The baseline retains 30 admitted profiles and the six admission failures.
For 32 narrow searches, the frontend lowers 227 operators, enters 64 utility bodies, and registers 32 value terminals.
For eight wide searches, it lowers 123 operators, enters 16 bodies, and registers 16 atomic terminals.

The implementation stores completed utility bodies in immutable shared plans within each clause.
Call sites retain their charges, capture metadata, and traversal filters. Incomplete bodies never enter the cache, preserving cycle rejection.
Effect and walk-depth summaries combine immediate child summaries once during lowering.
The single-clause entry point also reuses its lowered plan instead of lowering a fallible clause again.

A preflight counts composition operators after the existing repeat-group proof applies and rejects counts above 256.
Other utility calls conservatively count their full bodies. This bound does not claim that all possible compact plans are admitted.
It prevents shared graphs from causing excessive expansion before SQL-size checks run.
The 256-distinct-operator, 128-capture, traversal-depth, and 256,000-byte SQL limits remain explicit.
Semantic and performance qualification is pending.

The first comparison run passes all semantic cases and fails an unguarded cycle fixture at the policy parser.
That fixture never reached SQL lowering. Traversal-guarded self and mutual cycles now exercise the SQL cycle check and pass.
The initial log remains at `/tmp/rerail-utility-plan-first-tests.log`.

All 133 relational tests pass. Six new tests include 750 comparison jobs, shared-body identity checks, and explicit admission limits.
The suite retains 5,451 publication traces.
All 36 profiles have one matching statement and zero legacy frame publications.

| 32-search fixture | Before | After |
| --- | ---: | ---: |
| Narrow utility: lowered operators | 227 | 134 |
| Narrow utility: body lowerings | 64 | 2 |
| Narrow utility: value terminals | 32 | 1 |
| Wide utility: admission | Rejected at 256 expanded operators | Accepted |
| Wide utility: distinct lowered operators | Unavailable after rejection | 142 |
| Wide utility: composition operators after grouping | Unavailable after rejection | 18 |
| Wide utility: body lowerings | Unavailable after rejection | 2 |
| Wide utility: atomic terminals | Unavailable after rejection | 9 |

The wide true conjunction uses 14,799 SQL bytes, 3,994 VM instructions, and 32 typed return cells across two files.
The wide unknown alternative uses 14,882 bytes, 6,628 instructions, and 52 cells.
All dimensions retain constant return cells and at most one extra VM instruction across 2/8/32 searches.
The 30 previously admitted fixtures retain identical SQL and VM counts.
This phase removes frontend duplication and an admission failure. It does not claim faster execution for those existing queries.

Final qualification passes 145 unique tests.
These include 133 relational tests, one workspace test, nine B023 fixtures, one adversarial test, and one corpus test.
All 159 preceding profiles retain identical VM counts, outputs, native charges, reached value calls, and typed return counts where recorded.
Of these profiles, 141 retain exact SQL text. The other 18 differ only in terminal IDs within `node_regex` and `relation_value_budget` calls.
The evidence retains these renumberings and both query hashes.
Formatting and Clippy pass for workspace targets and the profiling feature.
For example, a fallback regex moves from terminal 32 to terminal 1 after repeated value terminals disappear.

The frozen B023 query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
It reports 138 files, two matches, and 2,812,255 generated VM instructions in all three pairs.
The VM gate passes, but the CPU gate fails at **2.071611×**.
Median generated CPU is 134,221,000 ns, versus 64,790,625 ns for the reference.
The failed run remains recorded without a retry. Its unchanged SQL and VM counts do not establish a structural execution regression or improvement.
This phase does not qualify B023 CPU performance.

The compiler wall-time sample for the narrow 32-call true conjunction is 0.339 ms, versus 0.370 ms before sharing.
The newly admitted wide counterpart takes 0.523 ms to compile.
These are instrumented single samples on a shared machine, not a production speed claim.
Policy parsing precedes this timer; SQL preparation and execution use separate counters.
The driver accepts baseline and current profile counts by default. Its interning assertion validates all current sharing invariants.
The profile run was repeated after this driver correction, with unchanged compiler sources and executable. The failed B023 timing gate was not rerun.

Artifacts:

- `benchmark/relational-utility-plans.py`: admission and semantic checks, compiler counters, EXPLAIN, and pinned source/binary hashes.
- `benchmark/sql-relational-utility-plans-evidence.json`: six baseline rejections, 12 native boundary observations, 750 comparison traces, 36 profiles, prior controls, and frozen gates.
- `benchmark/sql-relational-utility-plans.sql`: the formerly rejected 32-search true conjunction.
- `benchmark/sql-relational-utility-plans-unknown.sql`: the formerly rejected 32-search unknown alternative.
- Raw logs: `/tmp/rerail-utility-plan-20261023-{before,final,accepted}` and `/tmp/rerail-utility-plan-b023-20261023-final`.

The next phase aligns composition admission with existing local memo sharing for direct node-local utility calls.
The transition plan specifies the failing fixture, native charge boundaries, generation bound, and profile controls.
The outcome kernel remains test-only. Production integration, native depth limits, deadlines, general recursive outcomes, and the complete-job 100-execution gate remain open.

### 2026-10-23 — Composition admission follows emitted sharing

The next phase replaces conservative body expansion with a shared generation budget.
The limit remains 256 composition operators per clause. SQL generation now consumes that budget after selecting a miss body, replay, or scalar program.
Every nested program shares its parent counter. A new clause receives a new counter.
The existing memo planner still decides which answers can be reused. There is no second planner or runtime option.

**Failure first.** The six 32-call fixtures use direct `matches(v)` operands under `all` or `any`.
`v` contains eight regex predicates and a call to a value-bearing inner utility.
All six fail the preceding composition preflight, although native execution reads one value per file.
Native boundaries are 96 charges for true `all`, three for true `any`, three for false `all`, and 96 otherwise.
The retained preflight includes both sides of each boundary and all six admission errors.

**Generation bound.** A proved replay consumes one operator. A generated miss body consumes its own operators.
Fused scalar and ordered programs consume their emitted trees. Traversal repeat groups retain their existing proof and single body.
The `compositionOperators` profile now reports this generation count, replacing the conservative preflight estimate.
Distinct lowering, SQL byte, capture width, traversal depth, and local answer-storage limits remain unchanged.
A non-value utility graph still expands through nested child programs and fails at the shared generation limit.
Capture-bearing calls do not acquire a replay proof from utility identity alone.

The old preflight also bounded metadata traversal indirectly. Removing it required explicit handling of shared graphs.
Call-count analysis now caches each distinct plan and combines exact logical multiplicities with checked arithmetic.
Capture-key collection visits each distinct plan once. Neither analysis expands every logical call.
A metadata test represents more than one trillion calls with 121 plan nodes, then checks count-overflow rejection.

**Measured shape.** All 18 direct-call profiles now compile and match native execution.
Each has two utility bodies, nine terminals, one outer miss body, and one value-read site.
At 32 calls, composition uses 46 operators. Typed returns stay at 12 cells across two files for every 2/8/32 profile.
True `all` uses 38,320 SQL bytes and 7,773 VM instructions at 32 calls.
Unknown `any` uses 39,818 bytes and 7,729 instructions.
These are newly admitted shapes, not a measured runtime speedup over a previously executing baseline.

The profiles expose the next structural cost. Every static replay still creates its own projection and Boolean barrier.
True `all` grows from 14,970 bytes and 3,303 instructions at two calls to 38,320 bytes and 7,773 instructions at 32.
Even false `all` grows from 3,161 to 7,031 instructions, although its first false result stops semantic evaluation.
The next action is to combine adjacent proved direct replays into one guarded cost calculation.
It must use the existing memo proof, preserve failed and unknown outcomes, and keep exact charge exhaustion.

**Qualification.** The complete relational suite passes 139 tests. Workspace publication, nine B023 fixtures, the adversarial test, and the frozen corpus bring the total to 151 unique tests.
Six new tests include 538 native-comparison jobs. The full suite records 5,989 publication traces.
The 195 preceding profiles retain identical SQL, VM counts, native outputs, charges, reached calls, and typed return counts.
Formatting and both Clippy configurations pass: workspace all targets and `code-rule-profile` all targets.

Clippy initially found duplicated expected-value branches in the new test helper.
The cleanup simplifies that assertion and removes an unused fixture parameter. No engine source changes follow the measured B023 run.
The evidence retains the initial diagnostic, source hashes, and executable hash. The cleaned tests pass a separate optimized build and all 139 relational tests.
All 18 profiles retain identical SQL, VM counts, typed returns, native results, charges, and reached calls after the cleanup.
The initial 32-call unknown-`any` compiler sample took 45.453458 ms. That outlier remains in the evidence; these samples do not establish production compiler latency.

**Frozen B023.** Both gates pass in this run across 138 files and two matches.
All three generated samples use 2,812,255 VM instructions.
Generated median cold-kernel CPU time is 124,086,375 ns; reference median is 66,864,917 ns, a ratio of 1.8557769988707231.
The query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
Previous CPU-gate failures remain recorded. Unchanged SQL and VM counts provide no evidence of a structural B023 speedup or stable CPU qualification.
The CPU gate is not repeated for the test-helper cleanup.

Artifacts:

- `benchmark/relational-composition-budget.py` reproduces the 18 profiles and checks emitted misses, replays, terminal counts, and the generation bound.
- `benchmark/sql-relational-composition-budget-evidence.json` records the failed preflight, native boundaries, traces, full EXPLAIN, controls, and measured source hashes.
- `benchmark/sql-relational-composition-budget.sql` records 32 true conjunction calls.
- `benchmark/sql-relational-composition-budget-unknown.sql` records 32 unknown alternatives.

The bounded outcome kernel remains test-only. Production integration, general recursive outcomes, native depth limits, deadlines, and the whole cold-job 100-execution gate remain open.

### 2026-10-23 — Adjacent direct utility replay groups

This phase targets the repeated barriers exposed by composition admission.
The preceding 32-call conjunction needs 38,320 SQL bytes, 103 EXPLAIN materialization sites, and 7,773 VM instructions.
Its two-call counterpart needs 14,970 bytes, 43 materialization sites, and 3,303 instructions.
These recorded profiles fail the next phase's constant-shape gate.

The compiler now asks the existing memo planner for a replay proof after the first call.
It combines adjacent identical calls only when the body is node-local and the demand has no input bindings or capture effects.
The local memo planner supplies its existing key-version proof. Shared Boolean composition supplies its existing dominance proof.
The first call retains its normal miss or replay emission. One additional projection applies the remaining semantic costs.
One Boolean barrier commits the group result.

Traversal groups and direct groups use the same integer replay expressions.
Division checks the remaining allowance before multiplication. False or failed conjunction answers stop the group.
True or failed alternative answers stop the group. Noncapturing unknown answers continue for either operator.
An unreached first call retains SQL NULL as its tested result and leaves the work allowance unchanged.

Grouped calls consume their logical remaining uses, including uses inside nested utility bodies.
Typed answers retire at their last use. Later calls and optional branches retain their existing answer lifetime and reachability rules.
The generation budget counts one replay group. Native semantic work still counts every reached call.
There is no new runtime option, native fallback, or second memo planner.

**Measured shape.** All 36 profiles pass the constant-shape gate for 2/8/32 calls.
SQL text grows by three bytes. Materialization sites and typed return cells stay constant.
VM work differs by at most one instruction for a new integer constant.
Each profile retains one value-read site and one replay group. The wide profiles retain one outer miss body and nine terminals.
Generation uses 16 operators for the wide fixture and six for the simple value fixture, independent of group size.

| 32-call fixture | SQL bytes before → after | Materialization sites before → after | VM instructions before → after |
| --- | ---: | ---: | ---: |
| True `all`, eight regex predicates and inner utility | 38,320 → 14,555 | 103 → 42 | 7,773 → 3,219 |
| Unknown `any`, same nested body | 39,818 → 15,225 | 103 → 42 | 7,729 → 3,185 |

The preceding simple memo repetition profile falls from 20,538 to 4,213 SQL bytes and from 5,059 to 1,375 VM instructions.
Across 213 preceding profiles, 42 use fewer VM instructions and 171 retain identical SQL and VM counts.
None increase VM work. All retain native outputs, semantic charges, reached calls, and typed return counts.
The changed profiles belong to the direct-call, simple memo, and nested memo families.
Traversal repeat SQL remains identical after the arithmetic extraction.

**Semantic qualification.** Six new tests cover 684 comparison jobs and direct replay arithmetic at signed limits.
Coverage includes failed first calls, unreached groups, unknown fallbacks, prefix rejection without value reads, nested answer reuse, and independent clauses and files.
Negative cases retain existing emission for captures, changed keys, distinct utilities, optional branch boundaries, and traversing utility bodies.
The complete relational suite passes 145 tests and records 6,673 publication traces.

**Next concrete target.** The wide fixture still has eight scalar stages and eight sequencing barriers for its pure regex prefix.
The transition plan now specifies one guarded Boolean expression and one barrier for that run.
The acceptance target is 28 materialization sites for the current 42-site shape, with unchanged value boundaries and charges.
Unknown tests, capture effects, utility charges, and traversal boundaries remain outside that proof.

**Frozen B023.** Both gates pass across 138 files and two matches.
Generated median cold-kernel CPU time is 121,614,833 ns. Reference median is 65,453,792 ns, a ratio of 1.858025780996768.
All three generated samples retain 2,812,255 VM instructions.
The query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
This query has no eligible direct utility group. The change provides no structural B023 speedup.
Earlier failed CPU gates remain recorded. This passing run does not establish stable CPU qualification.

**Final qualification.** Workspace publication, nine B023 fixtures, the adversarial test, and the frozen corpus bring qualification to 157 unique tests.
Formatting and both Clippy configurations pass: workspace all targets and `code-rule-profile` all targets.
The evidence pins one optimized executable and the measured source files for both profile and frozen-workload runs.

Artifacts:

- `benchmark/relational-direct-replay.py` reproduces all 36 profiles and enforces the constant-shape checks.
- `benchmark/sql-relational-direct-replay-evidence.json` retains the baseline failures, comparisons, native traces, full EXPLAIN, source hashes, and frozen gates.
- `benchmark/sql-relational-direct-replay.sql` records the 32-call true conjunction.
- `benchmark/sql-relational-direct-replay-unknown.sql` records the 32-call unknown alternative.
- `benchmark/relational-composition-budget.py` accepts either individual proved replays or the new group when checking its earlier 18-profile matrix.

The bounded outcome kernel remains test-only. Production integration, general recursive outcomes, native depth limits, deadlines, and the whole cold-job 100-execution gate remain open.

### 2026-10-23 — Consecutive total predicates: implementation and qualification

The preceding wide fixture uses 42 materialization sites and 3,219 VM instructions for 32 true utility calls.
Its eight pure regex predicates each create a scalar stage and a sequencing barrier.
The target is one scalar stage and one barrier for that run: 28 materialization sites in this fixture.

The shared Boolean emitter now groups consecutive `Op::Predicate` operands.
An ordered `CASE` stops at the first decisive predicate. The outer guard preserves unreached results as SQL NULL.
The existing scalar lowering supplies each predicate expression. Bindings and remaining semantic work pass through unchanged.
Values, charges, unknown tests, captures, negation, and traversals end a run.
The generation budget counts every predicate plus the combined Boolean expression, consistent with scalar trees.

Four new tests pass 642 native-comparison jobs, including 54 profiles.
Predicate width and utility repetitions vary independently across 2, 8, and 32 for both conjunction and alternative utility bodies.
Comparisons cover early decisive predicates, unknowns, reached failures, capture rollback, nested reuse, and independent clauses and files.
The first profiling script passed the Rust tests and shape checks, then failed while parsing the test-result line.
The corrected script produced the pinned evidence. This correction required no engine change or frozen gate rerun.

**Exact preceding workload.** These measurements retain the eight-regex conjunction utility under both conjunction and alternative callers.
All samples use 32 utility repetitions across two files.

| Case | SQL bytes, before → after | VM instructions, before → after | Materializations, before → after |
| --- | ---: | ---: | ---: |
| True conjunction | 14,555 → 8,366 | 3,219 → 2,237 | 42 → 28 |
| False conjunction | 14,555 → 8,366 | 3,077 → 2,095 | 42 → 28 |
| Unknown conjunction | 14,551 → 8,362 | 3,121 → 2,139 | 42 → 28 |
| True alternative | 15,229 → 8,695 | 3,263 → 2,253 | 42 → 28 |
| False alternative | 15,229 → 8,695 | 3,165 → 2,155 | 42 → 28 |
| Unknown alternative | 15,225 → 8,691 | 3,185 → 2,175 | 42 → 28 |

The 54 new profiles retain 28 materialization sites and 12 typed return cells at every predicate width and repetition count.
Each emits one value-read site, reached once per file. Atomic terminal registration remains unchanged.
Across utility repetitions, VM work differs by at most one constant-register instruction.
Across predicate widths, VM work grows with reached predicates, while materializations stay constant.

**Prior controls.** All 150 final relational tests pass. All 231 preceding profiles retain native results, charges, reached calls, and typed returns.
The 18 composition profiles improve. The other 213 profiles retain identical SQL and VM counts.
The controls verify unchanged atomic terminal counts. No VM regression occurs.

**Remaining shape.** The true-conjunction profiles use 2,063, 2,237, and 2,933 VM instructions at widths 2, 8, and 32.
Their EXPLAIN reports contain 7, 13, and 37 correlated scalar subqueries.
The general condition emitter still wraps each atomic regex in a constant-row existence query.
The transition plan targets direct atomic expressions next, retaining relational emission for actual set joins and traversal.

**Qualification review.** The initial frozen run passed both gates at 1.976× reference CPU time.
Generated and reference median CPU times were 124,780,417 ns and 63,152,500 ns.
Query text and 2,812,255 VM instructions remained unchanged.
Review then found that the fused result bypassed the preceding projection's 512-column check.
The fused path now uses the same width check and reports its typed width.
A boundary test admits 512 columns and 256 generated operators, and rejects either limit plus one.
The final profile run passes all five new tests. All 54 SQL hashes, VM counts, and 642 native traces match the initial implementation.
The final relational suite passes all 150 tests. The initial passing frozen run remains in the evidence.
The lint command uses the existing diagnostic jemalloc shim, as does the optimized test build.
An initial lint attempt omitted that override and stopped in the dependency configure script before engine lint checks.
This change remains in the test-only outcome compiler. Production integration and the complete cold-job execution gate remain open.


**Final frozen workload.** The 138-file corpus retains two matches and exact parity.
Generated median CPU time is 130,234,292 ns. Reference median CPU time is 64,427,209 ns.
The ratio is 2.021×, which fails the 2× CPU gate. The VM gate passes at 2,812,255 instructions in all three samples.
The query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
The initial source passed at 1.976×. Both runs remain recorded, and no repeat attempts replace the final failure.
B023 has no eligible shared predicate run. Neither run establishes a structural B023 speedup or stable CPU qualification.

**Final checks.** All 162 unique tests pass: 150 relational tests, nine B023 fixtures, adversarial and corpus tests, and workspace publication.
Formatting and both Clippy configurations pass. Five new tests cover 642 native-comparison jobs and the compiler limits.
The complete relational run records 7,315 publication traces.
Source hashes pin the final executable and measured source files. The initial source, query hash, and timing gates remain in the evidence.

Artifacts for this phase:

- `benchmark/relational-predicate-run.py` reproduces the 54-profile matrix and enforces the materialization, width, and repetition checks.
- `benchmark/sql-relational-predicate-run-evidence.json` records the failed baseline target, comparisons, native traces, EXPLAIN, source hashes, and frozen gates.
- `benchmark/sql-relational-predicate-run.sql` records the preceding 32-call true conjunction after fusion.
- `benchmark/sql-relational-predicate-run-unknown.sql` records the preceding 32-call unknown alternative after fusion.
- `benchmark/relational-direct-replay.py` now accounts for the combined predicate expression when checking the generation budget.

### 2026-10-23 — Direct atomic predicate expressions

The preceding 54-profile matrix uses one correlated existence subquery for each regex and one for its root kind.
At predicate widths 2, 8, and 32, EXPLAIN reports 7, 13, and 37 correlated scalar subqueries.
The target is four remaining return/publication subqueries, independent of predicate width.
Materializations must stay at 28, with 12 typed return cells and one value-read site.

A shared outcome helper now emits direct expressions for kinds, regexes, empty sets, and all-node sets.
Scalar outcomes and ordered Boolean tests use this helper. The general relational emitter still handles patterns, references, compound sets, and traversals.
The surrounding guards, integer truth conversion, semantic charges, and generation limits remain unchanged.

The focused run passes eight tests and 882 native-comparison jobs, including the preceding 54-profile matrix.
Three tests are new: two add 240 native-comparison jobs, and one covers 96 SQL comparisons against the preceding atomic expression lowering.
The new comparisons cover shared utilities, unknowns, captures, failures, ordered walks, stops, counts, and pattern witnesses.
The SQL test also verifies that each reached regex executes once, and unreached regexes execute zero times.

All 54 profiles reduce VM work and preserve native results, charges, reached calls, lowered operators, terminals, and typed returns.
Correlated scalar subqueries fall to four at every predicate width. Constant-row scans fall to two.
Materialization sites remain at 28, typed return cells remain at 12, and each profile retains one value-read site.
Across utility repetitions, VM work still differs by at most one constant-register instruction.

| True conjunction, 32 utility calls | SQL bytes, before → after | VM instructions, before → after | Correlated scalar subqueries, before → after |
| --- | ---: | ---: | ---: |
| Two predicates | 7,742 → 7,640 | 2,063 → 2,021 | 7 → 4 |
| Eight predicates | 8,366 → 8,060 | 2,237 → 2,111 | 13 → 4 |
| 32 predicates | 10,885 → 9,763 | 2,933 → 2,471 | 37 → 4 |

The remaining general set expressions still add outer existence queries around already bound nodes.
B023 contains eight such layers, including relation membership, region membership, and negated structural tests.
The next phase targets these layers while preserving the actual witness joins.

All 153 relational tests pass. Each of the 231 preceding profiles reduces VM work.
Every comparison preserves native results, charges, reached calls, terminal counts, and typed returns.
Together with the width/repetition matrix, all 285 profiles improve. No VM regression occurs.
**Frozen B023.** The 138-file corpus retains two matches and exact parity.
Generated median CPU time is 128,511,125 ns. Reference median CPU time is 66,211,959 ns.
Both gates pass in this run at 1.941× reference CPU time and 2,812,255 VM instructions.
The query hash remains `84116309b7048f44b706bb74955b99a1bcb5b512d5b365b30a73f78992026aa1`.
B023 already uses direct atomic tests. This phase provides no structural B023 speedup.
The preceding 2.021× CPU failure remains recorded. This passing run does not establish stable CPU qualification.

**Final checks.** All 165 unique tests pass: 153 relational tests, nine B023 fixtures, adversarial and corpus tests, and workspace publication.
The complete relational run records 7,555 publication traces.
Formatting and both Clippy configurations pass with the existing diagnostic jemalloc shim.
Source hashes pin the final executable and measured files. The 231 baseline query hashes and VM counts match the preceding committed evidence.

Artifacts:

- `benchmark/relational-atomic-predicate.py` reproduces the focused tests and 54 profiles, and rejects redundant atomic subqueries or VM regressions.
- `benchmark/sql-relational-atomic-predicate-evidence.json` retains baseline target failures, all comparisons, native traces, EXPLAIN, source hashes, and frozen gates.
- `benchmark/sql-relational-atomic-predicate.sql` records eight predicates with 32 true utility calls.
- `benchmark/sql-relational-atomic-predicate-wide.sql` records 32 predicates with 32 true utility calls.

The bounded outcome compiler remains test-only. Production integration and the complete cold-job 100-execution gate remain open.

### 2026-10-23 — Bound set predicates without redundant outer queries

The preceding frozen B023 SQL contains eight `NOT (NOT EXISTS (SELECT 1 WHERE ...))` layers.
These layers surround relation membership, region membership, and negated structural tests.
The baseline uses 2,812,255 VM instructions. This phase requires zero redundant layers and strictly fewer instructions.

The emitter now separates each arm's sources, conditions, and projection from SQL formatting.
An arm without additional sources emits its Boolean condition directly. A witness arm retains an existence query.
Union membership combines arm predicates with `OR`. Intersection retains arm conditions and witness correlations.
Negation uses the same bound membership emitter. Atomic outcome expressions now use this shared implementation.
Existing reached-call guards and failure/capture publication barriers remain in place.

New SQL comparisons exercise empty and overlapping unions, negation, regions, recursive references, and structural witnesses across two files.
The existing atomic comparisons retain the old existence wrapper as their reference and check reached regex calls.
The new test passes 38 SQL comparisons. The retained atomic test passes 96 comparisons, including guarded regex call counts.
All 154 relational tests pass, with 7,555 publication traces.
The focused run passes nine tests and 882 native-comparison jobs.
All 54 width/repetition profiles and 231 preceding controls retain identical SQL and VM counts.
They preserve results, charges, reached value calls, terminal counts, and typed returns.

**Frozen B023.** All eight redundant outer layers are gone. The 138-file corpus retains two matches and exact parity.
Captures, outcomes, intermediate answer cardinalities, and typed returns match the preceding run.
Terminal call sites remain unchanged. Pure regex probes fall from 3,092 to 3,090 as Boolean alternatives skip unnecessary work.

| Metric | Before | After |
| --- | ---: | ---: |
| VM instructions, each of three samples | 2,812,255 | 2,804,023 |
| SQL bytes, excluding final newline | 22,020 | 21,882 |
| Node filters | 53,446 | 53,356 |
| Node advances | 87,702 | 87,692 |
| Materialization sites | 33 | 33 |
| Automatic-index rows | 54,156 | 54,156 |
| Full-scan steps | 35,379 | 35,379 |
| Correlated scalar subquery sites | 34 | 60 |
| Constant-row scans | 58 | 46 |

The VM reduction is 8,232 instructions, or 0.29%. This is a small improvement.
Boolean alternatives expose separate witness subqueries, so their static count rises despite lower runtime work.
The unchanged materialization, index, and scan counters show that this phase leaves the larger sources of work intact.
The next phase will measure upstream recursive and binding producers before choosing another optimization.
Repeated parent-kind probes remain a candidate; their impact still needs measurement.

Generated median CPU time is 124,714,834 ns. Reference median CPU time is 68,140,376 ns.
Both gates pass at 1.830× reference CPU time. This single run does not establish stable CPU qualification.
Earlier failed gates remain recorded. No unchanged gate was rerun to replace a failure.
The final query hash is `f1ffb7eb4f48dcf2552ae0b7569f736330b0fe4be99842a666f4e01488c76c73`.
The executable hash is `c120a6b7597cc8228ba7c122ad8a1f989141dbb8864546ec79d2e9a6fa7e29da`.

**Final checks.** All 166 unique tests pass: 154 relational tests, nine B023 fixtures, adversarial and corpus tests, and workspace publication.
Formatting and both Clippy configurations pass with the existing diagnostic jemalloc shim.
The measured source files and executable are pinned by hash. All 231 baseline query hashes and VM counts match the preceding committed evidence.

Artifacts:

- `benchmark/relational-bound-predicate.py` reproduces the focused comparisons and checks the frozen wrapper, parity, VM, and CPU gates.
- `benchmark/sql-relational-bound-predicate-evidence.json` records native traces, controls, full frozen evidence, EXPLAIN, source hashes, and comparisons.
- `benchmark/sql-relational-bound-predicate-b023.sql` records the final frozen workload query.

Reproduce the frozen run with `benchmark/relational-complete-spike.py`, the frozen corpus config, and the optimized test executable.
Pass that run's `evidence.json` to `benchmark/relational-bound-predicate.py` through `--frozen-evidence`, alongside `--binary` and a new output directory.

The compiler remains test-only. Production integration and the complete cold-job 100-execution gate remain open.

### 2026-10-23 — Audit of the bound-predicate performance claim

The user challenged whether the preceding phase had gates tied to its observed performance problem.
The audit confirms that the shape and correctness gates existed before implementation.
The performance gate required only a strict VM decrease. A one-instruction decrease would pass.
The existing 2× cold-CPU gate also passed before the change.
Neither gate required evidence that the eight wrappers caused a substantial part of the CPU gap.

All 285 unchanged profile queries were regression controls. They did not exercise the changed SQL shape as a performance target.
The new 38 SQL comparisons checked set semantics. They did not measure wrapper cost.
The phase also combined wrapper removal with union-membership rewriting without separate cost experiments.
The observed 8,232-instruction saving therefore cannot be attributed solely to wrapper removal.

**The overlooked measurement.** The frozen artifact already contains separate preparation and execution CPU times.
`whole_call/run.rs::execute_statement` stops the preparation timer before it starts the execution timer.
Both timers measure thread CPU time. The recorded execution time excludes preparation.

| Median CPU phase | Generated SQL | Reference SQL |
| --- | ---: | ---: |
| SQL generation | 0.972 ms | 0.521 ms |
| SQLite preparation | 47.613 ms | 3.245 ms |
| Execution | 73.578 ms | 64.110 ms |
| Complete cold kernel | 124.715 ms | 68.140 ms |

Independent medians are not additive. In the three paired samples, excess preparation accounts for 82.1%, 76.5%, and 76.1% of the cold-CPU gap.
The preceding query already showed the same problem: median preparation was 48.275 ms versus 3.040 ms.
The preceding work selected an execution micro-optimization while the main measured gap was preparation.
The subsequent plan to start with producer execution counters repeated that priority error.

This evidence identifies the costly phase. It does not identify the internal cause within SQLite preparation.
CTE expansion, planning, virtual-table index selection, and bytecode generation remain hypotheses to test.
Static subquery counts cannot select among them. The captured `EXPLAIN QUERY PLAN` is not a per-operator runtime profile.

**Corrected status.** The change passes correctness and shape qualification and provides a small measured VM reduction.
It does not qualify as a demonstrated solution to the structural CPU bottleneck.
The original evidence stays unchanged. The new audit records this distinction explicitly.

`benchmark/relational-phase-audit.py` reproduces the phase breakdown from the saved evidence.
It introduces a separate preparation target: generated median preparation CPU must be at most twice the reference median.
This target is new; it was not an acceptance gate for the preceding change.
The retained baseline fails: 47.613 ms exceeds 6.490 ms, at 14.67× reference preparation time.
`benchmark/sql-relational-bound-predicate-audit.json` retains the failure and the original passing gates.
The audit command exits unsuccessfully as intended. No benchmark rerun replaces any prior result.

The transition plan now starts with preparation-only profiling and controlled query-shape experiments.
Execution counters and full semantic comparisons remain regression gates.
This diagnosis covers frozen B023, not the original 818-clause workload or the complete-job 100-execution requirement.

### 2026-10-23 — Match reference structure; measure prepared execution

The user clarified that preparation is an installation cost and must not count toward repeated matching performance.
This supersedes the preceding preparation-performance target.
The benchmark now installs one prepared handle and executes it three times, with separate installation timing.
Each sample must execute one statement, preserve exact results, and report zero preparation CPU.

The structural baseline is 21,882 SQL bytes versus 8,196 reference bytes.
EXPLAIN reports 4,760 versus 1,947 static opcodes, 33 versus 20 materializations, and 60 versus 18 correlated subqueries.
Temporary B-tree sites total 27 versus nine. These are explicit failures before the compiler changes.

The acceptance gates are declared before measuring the implementation:
SQL bytes at most 1.25× reference, static opcodes at most 1.5× reference, materializations and correlated subqueries at most reference plus four, and temporary B-trees at most reference plus three.
Prepared execution median CPU must be at most 1.25× reference, with VM work no higher than 2,804,023.
The full native semantic suite and preceding profile controls remain required.
The benchmark records every gate before asserting failure.

The implementation shares functional parent/previous/next paths, retains independent child witnesses, and applies large root predicates after branch choice.
It removes empty branches and child-expansion ranking where key uniqueness already proves one result.
Demanded seed alternatives share their root source and functional witnesses.
No rule IDs, kind numbers, or B023-specific compiler recognition are used.

Five fixture candidates failed the SQL-size gate before full qualification.
Their sizes were 15,240, 12,784, 12,403, 10,899, and 10,269 bytes; the limit is 10,245.
The rejected-candidate artifact retains their exact SQL, hashes, native parity, and fixture VM counts.
Intermediate source and executable pins were not retained for these five builds.

The sixth candidate passed the full corpus gates with 10,231 SQL bytes.
It used 2,203 static instructions, 23 materialization sites, nine correlated subqueries, and eight temporary B-tree sites.
The reference uses 1,947 static instructions, 20 materialization sites, 18 correlated subqueries, and nine temporary B-tree sites.
Prepared execution medians were 32.514 ms and 31.997 ms, respectively.
All 18 prepared executions preserved results; each query handle executed three times after one preparation.
The generated query used 2,294,480 VM instructions, versus 2,324,796 for the reference and 2,804,023 for the preceding compiler.

The emitted stages now correspond directly to the reference:

| Reference stage | Compiler stage | Structural change |
| --- | --- | --- |
| `functions`, `safe_lambda` | `r1`, `r11`, `needed11` | One demanded seed source with shared parent and sibling joins |
| `function_loops`, `demand` | `pairs5`, `capture_demand` | Shared ancestor and binding choices; no unused frontier projection |
| `target_slot`, `store_slot` | `r12`, `r13` | One seed predicate per recursive relation; direct reuse of the target set |
| `stores`, `unbound` | `answer2`, `answer1` | One leaf/value set and range anti-joins; no function-by-store expansion |
| `parameter_walk`, `parameters` | `answer3_walk`, `answer3` | Shared transitions and an inlined acyclic entry state |
| `bodies`, `loads` | `answer4`, `answer5` | Combined child alternatives and one inclusive descendant choice |

These are compiler transformations over set and answer operators. The reference SQL is not substituted into execution.
Pure parent and sibling predicates share functional joins. Child predicates retain independent existential witnesses.
The exact nonempty-text regex lowers to a byte-span test; tests cover empty, Unicode, and multiline input.
Materialization remains at shared set and capture-choice boundaries.
The compiler retains three more materialization sites than the reference; matching the reference does not require identical SQL text.

Code review found an ordering gap in the inclusive-descent proof.
Two direct branches can match the same root but publish different witnesses.
The optimization now requires their order to equal the admitted descendant-source order.
A SQL regression test checks both orders and their different expected witnesses.
A separate native comparison reverses the original policy's direct-body and descendant branches.
These changes preserve the measured B023 SQL exactly.

SQLite automatic re-preparation is now instrumented and must remain zero for every repeated execution.
The final source passes the full qualification after this correctness change.
The first passing qualification remains in the evidence; this is not a retry of a failed CPU gate.

The final run reports 29.422 ms generated and 32.633 ms reference median prepared CPU.
Both runs support comparable speed; their variation does not establish a stable generated-query speedup.
All 18 final executions report zero automatic re-preparations and one matching statement.
Each generated execution uses exactly 2,294,480 VM instructions: 18.2% below the preceding compiler and 1.3% below reference.
SQL size falls 53.2%, static instruction count falls 53.7%, and all five structural gates pass.
The frozen workload retains 138 files, two matches, exact native captures, and zero frame publications.

Preparation remains visible as installation timing and is excluded from the execution gate.
Final median installation CPU is 2.927 ms generated and 2.510 ms reference.
The direct diagnostic runs and prepared runs use different instrumentation; their timings are not an improvement comparison.
All 170 unique scoped tests pass, including 157 relational tests, ten compiler fixtures, adversarial coverage, the frozen corpus, and workspace failure publication.
The 54 focused profiles and 231 preceding controls retain identical SQL hashes and VM counts.
The focused run retains 882 native comparison jobs. Formatting and both Clippy configurations pass.

The remaining counters are mixed and are retained in the evidence.
Automatic-index rows fall from 54,156 to 21,781, while full-scan steps rise from 35,379 to 42,264.
Regex requests fall from 3,090 to 2,452; pattern requests rise from nine to 142.
The shared Boolean predicates change which pure leaf probes SQLite evaluates.
Total VM work and prepared CPU pass their gates; this does not establish that every individual access path improved.

Artifacts:

- `benchmark/relational-reference-shape.py`: prepared-execution and structural gate driver.
- `benchmark/sql-relational-reference-shape-b023.sql`: exact qualified generated query.
- `benchmark/sql-relational-reference-shape-evidence.json`: source, executable, query, and corpus hashes; both full qualifications; EXPLAIN; controls.
- `benchmark/sql-relational-reference-shape-rejected.json`: five rejected fixture queries and their measured size failures.

This qualifies the test-only B023 compiler shape, not production integration or the complete-job 100-execution gate.

### 2026-10-23 — Restrict origin and value demand before traversal

The user authorized the next candidate-production changes before the main compiler replacement.
The baseline is commit `160a0076`, with 2,294,480 VM instructions for frozen B023.
The current query walks broad origin sets and carries unused capture values through later stages.
Reference parity alone does not qualify the amount of work.

The first change adds SQLite loop measurements to diagnostic execution.
Prepared timing disables these measurements and retains separate preparation timing.
The baseline profile and new scaling failures must be saved before compiler changes.

Acceptance requires exact native roots and captures, one matching execution, and zero frame publications.
The initial frozen-workload target is at most 250,000 VM instructions, approximately nine times less than baseline.
This is an engineering target, not a predicted result.
Unrelated functions must add no VM work in the scaling fixture.
Unused assignment names must not increase capture-demand rows beyond the single used value.
The scaling sizes are 32, 128, and 512.
Existing semantic and prepared CPU gates remain required.
All failed candidates remain recorded.

The implementation must retain ancestor stops, inclusive stops, first assignment choice, and nearest successful ancestor choice.
Demand filtering cannot select a different witness after a later predicate fails.
Main compiler replacement remains the subsequent task and needs explicit admission and resource-limit coverage.

The instrumented baseline confirms the scaling failures.
Unrelated functions increase VM work from 5,001 to 51,081 across sizes 32 to 512.
Unused bindings increase capture-demand rows from 33 to 513.
The baseline profile records 23,208 ancestor-walk rows and an 8,513-row boundary index.
Scan instrumentation adds VM instructions, so the absolute budget uses ordinary prepared execution.

Candidate 1 restricts origin seeds and builds used-value keys before capture choice.
It preserves fixture and adversarial native results.
Prepared VM work decreases to 552,182, but misses the 250,000 target.
Its 10,992-byte query also misses the preceding SQL-size limit.
The origin walk produces 280 rows, and capture demand decreases to 34 rows.
Unused-binding scaling retains one capture-demand row at every size.
Unrelated-function scaling still fails because the binding walk materializes all function boundaries.

Candidate 2 removes that boundary set from the physical plan.
It uses subtree ranges and an indexed test for intervening stop nodes to select assignment witnesses.
The first successful witness still uses the minimum preorder node ID.
It also evaluates ancestor predicates on reached nodes and removes unused relation definitions through typed dependency tracking.
Both scaling gates pass, but prepared execution still uses 318,817 VM instructions.
Its query uses 10,300 bytes, so both the VM and SQL-size gates fail.
Loop measurements expose 5,776 AST point lookups for 361 binding nodes.
SQLite flattens the binding source and repeats those lookups across 16 relevant loops.

Candidate 3 materializes the narrow binding keys once and starts capture choice from used values.
It also removes an answer projection by giving capture demand the standard answer key.
Prepared VM work decreases to 246,385, within the 250,000 target.
Its 10,259-byte SQL still fails the unchanged 10,245-byte size limit.
Both scaling gates and native fixture comparisons pass.

Candidate 4 removes seed DISTINCT operations where an enclosing UNION already removes duplicates.
This changes the emitted relational operations, rather than shortening aliases or removing diagnostic comments.
It passes the frozen workload with 246,364 VM instructions and 10,232 SQL bytes.
The VM reduction is 89.3%, or 9.3 times less work, against the 2,294,480-instruction baseline.
All nine prepared executions report the same VM count and zero automatic re-preparations.
Generated median prepared CPU is 4.264 ms, versus 30.218 ms for the unchanged reference.
Preparation and installation remain outside this execution comparison.

All five structural gates pass without changing their limits.
The generated plan has 2,195 static instructions, 22 materialization sites, nine correlated subqueries, and ten temporary B-tree sites.
Exact native results remain two matches across 138 files, with one matching statement and zero frame publications.

The loop profile attributes the reduction to smaller input relations:

| Operation | Before | After |
| --- | ---: | ---: |
| Origin ancestor-walk rows | 23,208 | 280 |
| Capture-demand rows | 1,029 | 34 |
| AST point lookups for binding keys, candidate 2 versus candidate 4 | 5,776 | 361 |
| AST node advances | 79,812 | 11,421 |
| AST node filters | 54,038 | 10,957 |

The query still scans 3,637 loop candidates through kind postings.
No claim of minimal remaining work is made.
Unrelated-function scaling uses 1,894 VM instructions at all three sizes.
Unused-binding scaling retains one capture-demand row at all three sizes, but input processing still grows linearly.

The scan-enabled build retains complete loop records and hardware-cycle measurements.
Prepared handles disable scan instrumentation before preparation; the final qualification also uses an ordinary build.
The final query and prepared VM count agree across both builds.
Loop cycle values can include child operations and lazy native index construction. Do not add them as independent CPU totals.
Parsing and native index construction remain dependent on input size, even when SQL VM work stays constant.

The cutover audit compiles every AST grammar in all 138 Python policies, with no exclusions.
Only 19 of 146 grammars are admitted; 127 fail with recorded reasons.
The audit test passes because it records coverage. The complete-catalog admission gate fails.
Admission is not execution parity or resource-limit qualification.
The transition plan now names these missing contracts before the main compiler replacement.
The historical full-engine VM count is not improved by this test-only change.

All 173 unique qualification tests pass, including the admission audit that records the failed readiness gate.
The separate diagnostic profile test also passes.
All 54 focused profiles and 231 preceding controls retain identical SQL hashes and VM counts.
The focused suite retains 882 native comparison jobs. Formatting and both Clippy configurations pass.
A Clippy failure for a test-only temporary vector was corrected before final qualification.

Artifacts:

- `benchmark/relational-candidate-demand.py`: unchanged structural and CPU gates, plus the 250,000-VM and scaling gates.
- `benchmark/sql-relational-candidate-demand-b023.sql`: the exact qualified query.
- `benchmark/sql-relational-candidate-demand-evidence.json`: final qualification, source and binary pins, and preceding controls.
- `benchmark/sql-relational-candidate-experiments.json`: the baseline and all four candidates, including rejected gates, exact source patches, SQL, and loop profiles.
- `benchmark/sql-relational-cutover-admission.json`: policy hashes, each grammar's admission result, and missing-contract counts.

### 2026-10-23: Catalog contracts before compiler replacement

The next phase uses three bounded agent mandates: pure relation coverage, atomic outcome contracts, and catalog validation.
The root agent reviews their changes, owns integration, and runs the shared build and validation.
No agent may change the production compiler, relax a gate, or add a native composition fallback.

The coverage gate now visits all 146 grammars in 138 checked-in Python policies.
A separate execution gate compares admitted grammars with native clause execution on all 595 upstream Python fixtures.
It checks ordered roots, named capture payloads, and failure publication, and records fixture and policy hashes.
A successful admission audit does not imply replacement readiness.
The complete-coverage test deliberately fails while any grammar is refused.
Publication features and finite resource limits still require separate qualification.

The first checkpoint adds capture-free ancestor and sibling walks, including inclusive rule stops.
The recursive relation contains node keys, without an origin-by-depth product.
Four focused pure-relation tests pass.
Catalog admission increases from 19 to 31 grammars; all 31 pass the 595-file native comparison.
The complete-coverage gate fails with 115 refusals, as required.

The atomic checkpoint admits explicit resolution, capture-free layout, absent-argument, and followed-value contracts.
Three of five focused tests pass initially.
The followed-value trace test fails, and the parsed-literal refusal fixture uses an invalid language.
These failures are retained for investigation; this checkpoint is not qualified.
The first build also found an unsupported SQLite `usize` extraction in the new test runner.
The runner now extracts a checked SQLite `u32` file key before conversion.

The next integration adds typed node witnesses for atomic patterns and ordered SQL capture constraints.
List captures, parsed subdocuments, recursive outcome components, and constraint-only count publication remain explicit refusals.
The planner selects representations through typed applicability before SQL emission.
Invalid graphs, generation limits, and emission failures remain errors.
The existing frontier plan remains subject to its frozen SQL-size and VM-work gates.
Production replacement has not occurred.

The second diagnostic build admits 37 grammars, and all 37 pass the complete fixture comparison.
The remaining refusals are 34 list-capture contracts, 46 recursive outcome contracts, 28 other operator contracts, and one recursive-negation contract.
These are first-refusal counts; a grammar can need several contracts.
The complete-coverage gate still fails, with 109 refusals.

The second focused run passes 186 tests and fails six.
Two failures are invalid new fixtures: the parsed-literal language and an unconstrained zero-minimum count.
Two old rejection tests now target supported behavior; they must test the narrower unsupported contract instead.
One pattern test assumed that every anonymous-ellipsis alignment succeeds. It now compares each bound input with native matching.
The remaining failure reveals a real finite-budget defect in followed-value utility replay.
Native memoization excludes scope-cache construction from replay cost. SQL had replayed the entire first-call cost.

The proposed correction brackets each reached utility miss, records its native cache-fill delta, and subtracts that delta from replay cost.
The bracket belongs to one invocation, rather than a materialized batch of inputs.
Fill-bearing utilities use keyed clause/file memo storage; unchanged total-cost proofs remain available for other utilities.
Constraints also use keyed memo storage when they evaluate utilities at a captured node.
Additional tests cover nested fills, multiple files, shared utilities, and repeated traversals.
A separate two-clause test checks that scope costs are charged to each clause independently.
These corrections await the next build and qualification.

The source audit finds reachable recursive utilities in 70 refused grammars across 64 policies, including shared definitions and parameterized calls.
That count overlaps the operator counts above. It is not an estimate of immediate admission gains.
A neighbor-field contract is the next bounded addition; end and rule-stopped field traversals retain explicit refusals.

Scope-sensitive atomic leaves now carry immutable clause ownership through a native guard.
The guard selects the scope-charge key and restores it after the atomic call. It does not begin another clause.
The current token is unique within one compiled plan and one `BatchScopes` job.
A fresh job must reset that scope lifetime, including when it reuses a prepared statement.
Separately compiled groups sharing one lifetime will need job-global tokens before production cutover.
The compiler profile records this restriction.

The neighbor-field addition selects `node.field` with `node.ffirst` before evaluating its child plan.
It preserves the caller key and carries only the selected node into the child demand.
Its tests include missing and repeated fields, captures, unknowns, reached budgets, and B303 fixture parity.
A separate adversarial test combines constrained candidates, nested traversal, private stops, and failed-file rollback.
The next build found a test oracle calling `unwrap` on a native stop type without `Debug`.
That test now reports an explicit oracle failure without changing the production stop type.

The next checkpoint passes all 207 tests, including the constraint-core memo-sharing regression.
All 38 admitted grammars pass native comparison across the 595 fixtures, with positive witnesses for each grammar.
The complete-coverage gate fails with 108 refusals: 34 list-capture contracts, 49 recursive outcome contracts, 24 other contracts, and one recursive-negation contract.
The gate retains every policy and grammar. No refusal becomes a fallback.

Frozen B023 still uses 246,364 VM instructions and 10,232 SQL bytes.
Its median prepared CPU is 4.311 ms, versus 30.628 ms for the reference.
All nine prepared executions agree, and all existing structural limits pass unchanged.
All 231 preceding controls and 54 focused profiles retain their SQL hashes and VM counts.
The focused profiles also retain their native outputs, reached calls, and work traces.

A subsequent shape review finds a new amplification defect in standalone composite patterns.
A rule such as `f($X)`, without an explicit kind predicate, scans every AST node before the atomic cursor rejects unrelated kinds.
The source pattern already supplies the root-kind information, but the new outcome operator does not carry it to physical planning.
The next gate fixes two call nodes and adds unrelated assignments at sizes 32, 128, and 512.
It requires two pattern requests, one node filter, and two node advances at every size.
This gate will first run against the uncorrected shape.

The new gate fails against the uncorrected shape, as required.
With 32, 128, and 512 unrelated declarations, it performs 241, 913, and 3,601 pattern requests.
VM instructions grow from 13,089 through 49,377 to 194,529. The root node scan has no kind constraint.
The pattern operator now carries the terminal's existing root-kind hints into demand planning.
The first corrected run holds pattern requests and node advances at two for every size.
Constraint plans also inherit necessary root kinds from their core, before constraints run at captured target nodes.
That propagation retains the existing effect-order proof and never uses a constraint target's kind to filter roots.
A separate regression checks constant pattern requests, exact native outputs, reached calls, and remaining finite work.
Final native lint and qualification are in progress.

The corrected standalone query uses 184 VM instructions at all three sizes.
At 512 unrelated declarations, this removes 99.9% of its former VM work while retaining exact native captures.
This is a scaling result for the new pattern contract, not a production-engine speedup.
The bounded independent review finds no blockers in pattern hints or constraint-core propagation.
Formatting, workspace Clippy, and profiling-feature Clippy pass.

Final qualification passes 209 relational compiler tests, ten B023 fixture checks, two candidate checks, one adversarial check, and the frozen corpus check.
The 231 preceding controls and 54 focused profiles retain their exact SQL hashes and VM counts.
Frozen B023 retains 246,364 VM instructions, 10,232 SQL bytes, one matching statement, and zero frame publications.
Its median prepared execution is 4.339 ms, versus 32.771 ms for the reference, across nine executions.
Preparation remains a separate one-time measurement. All existing shape limits pass without adjustment.

All 38 admitted grammars again pass the 595-file fixture batch, with a positive witness for each grammar.
The complete-coverage test still fails with 108 refusals. Production replacement remains blocked.
Their combined VM count is unchanged at 44,259,715 before and after root-kind pruning.
This sum covers 38 separate grammar executions over all fixtures; it is not a fused complete-job measurement.
These admitted rules already constrain their roots. The new physical improvement applies to standalone patterns without that explicit constraint.

The committed coverage ledger preserves every refusal, fixture hash, output hash, VM count, and native file trace.
File traces use groups with inclusive file-index ranges; those groups retain every original trace entry.
The qualification artifact records earlier failed checks and final source, binary, corpus, and query pins.
The pattern experiment preserves the failing baseline and corrected SQL, explained plans, counters, and scaling samples.
Artifact paths and rerun commands are listed in `docs/sql-relational-transition-plan.md`.

## Catalog closure: typed lists and binding predicates

The starting checkpoint admits 38 of 146 grammars, with exact parity across 595 fixture files.
Its first refusals include 34 list contracts, 49 recursive outcome contracts, 24 other contracts, and one recursive-negation contract.
These counts overlap later missing features and do not predict admission gains.

Three GPT-6.1 Sol agents received separate read-only audits before any implementation mandate.
The root reviewed their proposals and assigned disjoint files for typed transport, atomic list patterns, and capture-independent binding predicates.
The root owns compiler integration, test builds, catalog checks, performance checks, and publication.

Lists use a canonical SQL blob containing the file key and exact ordered node sequence.
NULL represents an absent capture; a present empty list has a valid header with zero nodes.
Atomic patterns receive existing bindings before native matching, so greedy ellipsis alignment retains native behavior.
SQL still owns composition, capture transactions, reached work, and failure publication.
The transport cursor and memo table accept typed lists without environment handles or additional SQL executions.

The first recursive audit separates pure structural SCCs from capture, value, and parsed-tree components.
Boolean set membership alone cannot preserve reached utility costs or depth limits.
Recursive admission therefore remains closed while the next component contract is designed and tested.
The first integration run will test typed lists, native binding predicates, and every preceding relational contract.

The first integration admits 67 grammars. Exact fixture parity passes for 66 and fails for B413 during SQL preparation.
SQLite reports more than 65,535 references to `node` while resolving B413's 40 ordered pattern alternatives.
Each old Boolean step references its predecessor through both the carried state and the reached pattern demand.
The proposed correction uses one correlated typed return for each atomic pattern operand, with one predecessor reference.
A new regression scales flat alternatives through 8, 16, and 32 operands and checks the real B413 policy.

The focused run passes 21 tests and fails five.
Two new fixtures are invalid: public policy validation rejects list constraints and an unbound region capture.
Three failures reveal a test-harness assumption: every budgeted atomic call was treated as a value probe.
The corrected harness records the actual value probes reached within each atomic invocation.
It still checks exact value-call order, outputs, remaining work, and clause/file failures.
A compiler warning also identifies an intentionally discarded decoder iterator; the code now discards that result explicitly.

The next integration adds a typed, finite-budget argument leaf using native atomic binding semantics.
It accepts no arbitrary child matcher and exposes explicit input/output node columns to SQL.
A separate recursive-chain experiment deduplicates demanded keys and computes reverse truth/cost summaries.
It remains outside catalog admission until incoming utility depth and native stack limits have a compatible contract.

The second diagnostic run passes all 70 admitted grammars on all 595 fixtures.
Each admitted grammar has a positive witness; 76 remain refused.
Later contracts now expose 54 recursive-outcome refusals, 21 other refusals, and one recursive-negation refusal.
This is grammar-level parity, not complete-job batching or production qualification.

The focused run passes 36 tests and fails one at 32 argument bindings.
SQLite requests a hidden input column from the bound cursor; that projection was unsupported.
The cursor must return the actual typed hidden values, as the existing pattern cursor does.
Argument replay coverage also now requires three invocations and an observed memo hit.
New TypeScript and PHP tests cover finite work, decoding, and clause/file ownership.

B413 now prepares: its focused sample uses 30,246 SQL bytes and 248,797 VM instructions.
However, 2,995 pattern probes for nine imports reveal a second shape defect.
Capture-free alternatives discard their native necessary-kind proof during lowering.
The correction retains that proof as a typed predicate conjunction and propagates it through alternatives.
An unrelated-AST growth test must prove constant matching work for the real policy.
The flat list-alternative sample grows from 7,427 to 28,017 SQL bytes and 3,283 to 11,923 VM instructions across 8 to 32 alternatives.

The standalone recursive chain passes its narrow tests: 1,716 SQL bytes at every depth.
Depths 32, 128, and 512 use 8,874, 33,258, and 130,794 VM instructions.
Adding 512 unrelated statements leaves those counts unchanged.
Duplicate roots share demanded keys and suffix summaries; no origin-by-descendant relation is built.
Native completed-answer/work comparisons cover costs through 129; larger chains test early work failures only.
Native stack limits remain unverified, so this experiment does not admit recursive catalog grammars.

The next full semantic run passes 248 tests and fails five.
Three old refusal fixtures still expect newly supported argument, layout, or End-field contracts to fail.
Their replacement tests check positive native parity; parsed composition and Rule-stopped fields remain refused.
A new comparison fixture incorrectly selects Python/PHP structural modes in TypeScript; those cases move to Python.
The B413 scaling assertion also confuses total node-cursor advances with root visits.
That counter includes point lookups inside scalar pattern operands, so nine roots produce 153 total advances.
The corrected gate bounds advances and pattern probes by nine roots times 40 alternatives, and requires growth invariance.
It also imposes a 30,000-VM limit; the preceding 248,797-VM sample fails this limit.

The necessary-kind correction passes all 70 grammars again.
B413 drops from 417,040,778 to 3,987,126 VM instructions on the full fixture batch, with identical output hashes.
Across these same 70 individually executed grammars, the sum drops from 1,055,627,750 to 121,697,536 instructions.
The preceding 38 grammars retain their combined 44,259,715 VM instructions.
These are separate matching statements and cannot certify the complete-job execution budget.

The End-field review finds another amplification source: a left join scans every direct sibling after finding the requested first field child.
The corrected seed uses a limited scalar lookup, then fetches the selected subtree bound by node ID.
A regression adds 32, 128, and 512 later import aliases while holding the selected subtree fixed.
Ancestor walks still inspect each ancestor edge; field-subtree walks still inspect each bounded structural node.
Further posting seeks remain a physical-planning opportunity.

The remaining resolver-site audit identifies ordered child transactions, not an atomic Boolean leaf.
Binding resolution must return typed site facts, then SQL must charge and execute the reached child program.
A scope resolver can spend work without reading values; conflating those effects would introduce incorrect utility memoization.
The exec/eval catalog rules also contain recursive callee utilities, so site lowering alone would not admit them.

After the corrections, all 32 focused list, argument, predicate, field, and cursor tests pass.
The argument suite now includes Python, TypeScript, and PHP, actual memo hits, and 32-column cursor reuse.
List cells currently have an explicit 1 MiB transport limit; mapping that bound into the final job resource contract remains required before cutover.
The final refusal messages identify the missing typed operator instead of grouping every nonrecursive failure under one generic message.

A final independent resource review identifies a limit to this checkpoint's evidence.
The legacy `integerCellsStored` memo counter counts sparse tuple fields; it does not measure blob bytes.
Memo entries have no aggregate byte bound, and keys/answers can retain repeated list copies.
A 128-slot row can hold up to 128 MiB of blob payload before SQLite storage and nested copies.
The pattern cache's 32 MiB accounted-entry cap does not bound those other allocations.
Current parity and VM checks do not establish a production memory bound.
Before cutover, add explicit retained/copied byte telemetry and an aggregate resource-failure contract.
Do not silently evict memo entries: replay changes can affect finite work and failure behavior.

Final qualification passes all 254 relational tests, ten B023 fixture checks, two candidate checks, one adversarial check, and the frozen corpus check.
The 231 preceding and 54 focused controls retain their exact SQL hashes and VM counts.
Frozen B023 retains 246,364 VM instructions, 10,232 SQL bytes, one matching statement, and zero frame publications.
Median prepared execution is 4.211 ms against 31.168 ms for the reference, across nine executions per query.
All existing CPU, VM, and shape gates pass without changes; preparation remains a separate one-time cost.
Formatting, workspace Clippy, and profiling-feature Clippy pass.

The final catalog run passes all 70 admitted grammars on all 595 fixtures, with a positive witness for each grammar.
The complete-coverage gate still fails with 76 refusals: 55 recursive outcomes, nine parsed-tree compositions, six format-argument compositions, three binding-site compositions, two parameterized utility calls, and one recursive-negation dependency.
Source and executable hashes, fixture inventory, exact output hashes, per-file work traces, and generated SQL hashes are retained in the coverage and qualification artifacts.
`benchmark/sql-relational-catalog-expansion.json` retains both B413 queries and the matching comparison for the same 70 grammars.
The checkpoint remains test-only. Production replacement, complete-job batching, recursive resource equivalence, and aggregate list-memory qualification are not complete.

## Catalog continuation: shared recursive suffixes

The next contract admits capture-free, functional recursion through a parent or adjacent sibling.
Two ordered alternatives define the component: a total base predicate, then a guarded call to the same utility.
The compiler proves utility identity, predicate purity, capture independence, and a finite kind filter for continuation.
Unsupported recursive shapes retain an explicit refusal.

Three GPT-6.1 Sol audits had restricted assignments: recursive components, parsed-tree composition, and format-argument composition.
The recursive agent then changed only the recognizer, typed operator, lowering cache, and effect summary.
A second agent added focused integration tests in one file.
The primary agent owns statement-level emission, depth propagation, integration review, and all test execution.

The shared suffix relation has the key `(file,node)` and five columns: file, node, truth, cost, and peak depth.
It lives at statement scope, outside correlated programs for individual roots.
Its domain contains guard-eligible nodes in files with candidate roots.
The node kind index selects that domain, and recursive propagation visits each suffix once.
Each reached invocation still pays its full native logical cost.
The setup cost is linear in the guard domain, including eligible nodes outside reached branches.

The integration review found a separate memo boundary.
A value-reading utility that contains a chain can reuse an answer from a shallower invocation.
The existing memo stores cost but no peak depth, so that combination requires an explicit refusal until the memo contract changes.
Logical depth and actual stack bytes remain distinct contracts.
The SQL plan enforces the 256-level logical limit, but native stack-byte equivalence remains unqualified.
This checkpoint remains test-only and does not permit production replacement.

Parsed-tree and format-argument audits confirm that both require child composition in SQL.
Atomic native helpers can produce parsed documents and typed candidate facts, but cannot execute arbitrary child rules.
Five format policies have capture-free parsed children, but their host utilities also require downward recursive scheduling.
Binding-site composition similarly needs typed target facts and ordered child transactions.
These contracts remain separate from the functional-chain implementation.

The first diagnostic run passes four focused tests and rejects one invalid test fixture before SQL execution.
The fixture requested an anonymous comma as a public kind, but the public DSL accepts only named kinds.
The corrected sibling tests use named statements and retain a separate anonymous-comma barrier check.
The next run passes all six integration tests and the SQL logical-depth boundary test.
The latter checks incoming levels 0, 1, 253, 254, 255, and 256 against an entry-by-entry oracle.
It does not substitute for native stack-byte qualification.

Shared roots at nesting depths 8, 32, and 128 use 8,098, 29,602, and 115,618 VM instructions.
The SQL remains 6,224 bytes.
Adding 512 unrelated declarations leaves the small sample at 1,523 instructions and 20 node advances.
Adding 8, 32, and 128 disconnected guard groups raises work to 3,279, 9,423, and 33,999 instructions.
This result exposes the linear cost of the broader guard domain.
Explained plans show indexed reverse propagation on `(file,next)` and indexed invocation lookups on `(file,node)`.
A skipped branch uses three node advances and does not scan its recursive suffix domain.

The diagnostic catalog run admits 76 grammars and passes all 76 on 595 files.
Every admitted grammar has a positive witness.
The six additions are ASYNC210, ASYNC222, ASYNC250, ASYNC251, F622, and F633.
The preceding 70 grammars retain identical SQL hashes and VM counts.
These are separate grammar executions, not a fused complete job.

The broader query shape still requires work.
On the full fixture batch, each added ASYNC grammar uses approximately 32.85 million VM instructions.
F622 uses 1,064,442 instructions, and F633 uses 124,571,068 instructions.
Shared suffixes remove repeated recursion, but the surrounding ordered root programs remain expensive.
These measurements establish new coverage, not overall performance qualification or production readiness.

The expanded zero-work probe exposes unnecessary summary materialization.
A reached invocation with no allowance uses 877 instructions with no disconnected guard groups and 31,342 with 128 groups.
The outer-join condition preserves the logical result but does not prevent SQLite from preparing the shared summary.
The correction moves the lookup into a correlated typed return with an entry guard before summary access.
The new gate requires constant VM instructions and node advances for both skipped and exhausted invocations.

The guarded lookup passes the new regression.
Exhausted calls now use 588 VM instructions and three node advances at every tested guard-domain size.
Wholly skipped branches retain 699 instructions and three advances.
The largest exhausted sample drops from 31,342 instructions to 588.
Successful shared-root samples use 8,283, 30,315, and 118,443 instructions, with 6,386 SQL bytes.
The guard adds approximately 2.4% to those successful samples and prevents unnecessary preparation on immediate failure.
Disconnected guard groups still produce linear setup work: 3,335, 9,575, and 34,535 instructions.

Final qualification passes all 263 relational tests, ten B023 fixture checks, two candidate checks, one adversarial check, and the frozen corpus check.
All 231 preceding and 54 focused SQL/VM controls remain unchanged.
Frozen B023 retains 246,364 VM instructions, 10,232 SQL bytes, one matching statement, and zero frame publications.
Median prepared execution is 4.264 ms against 32.294 ms for the reference, across nine executions per query.
Preparation remains a separate one-time cost, and every existing CPU, VM, and shape gate passes without threshold changes.
Formatting, workspace Clippy, and profiling-feature Clippy also pass.

The final catalog run passes all 76 admitted grammars on all 595 fixtures, with a positive witness for each grammar.
The preceding 70 retain their exact SQL hashes, VM counts, and output hashes.
The complete-coverage gate still fails with 70 explicit refusals.
The final ASYNC samples use approximately 33.05 million VM instructions each.
F622 uses 1,073,630 instructions, and F633 uses 124,812,572 instructions.
Those remaining costs require further structural work outside the shared-chain component.

`benchmark/sql-relational-chain-integration.json` retains the new SQL, explained plans, failing zero-work measurements, corrected growth samples, and grammar comparisons.
The coverage and qualification artifacts retain source and executable hashes, exact output hashes, fixture inventory, and per-file native work traces.
The code and evidence remain on the personal branch, separate from Tim's branch.
Production replacement, native stack-byte equivalence, general recursion, aggregate memory qualification, and complete-job batching remain incomplete.

## Catalog continuation: ordered downward components

Three GPT-6.1 Sol subagents received restricted assignments for native recursion semantics, SQL shape, and parameterized calls.
The implementation assignments covered a recognizer, a SQL builder, and an isolated test file.
The primary agent owns integration, review, test execution, and qualification.

The next component recognizes a pure base followed by a guarded neighbor-child call to the same utility.
Its recursive state contains file, node, tree, and depth.
All eligible child edges form a forest, so each domain node occurs once.
Window queries calculate preorder positions, subtree ends, and the first successful base.
Each invocation uses the interval through that first success, or its complete subtree on failure.
This preserves failed sibling costs without an origin-by-descendant relation.

A correlated depth lookup proved unsuitable during SQL review.
SQLite creates an equality index but scans and sorts the matching depth bucket for each root.
The replacement merges query events and depth markers, then uses a forward window minimum.
Each statically used caller depth adds one linear event sweep, with an explicit limit of 16 distinct depth domains per component.
The first forbidden entry determines the completed charges before a depth failure.
Depth wins when work equals those completed charges, because native checks depth before the next entry charge.

The native-semantics review corrected an initial assumption about anonymous children.
Two-valued matching can derive a finite child-kind filter from the base and guard, even through the recursive utility.
That filter skips anonymous children before utility entry.
Three-valued matching disables it, and an unbounded regex base provides no filter.
The compiler now applies the native filter to forest edges while preserving direct invocation roots.
Tests cover filtered, unfiltered, and three-valued calls with exact remaining work.

The parameterized-call audit found two PIE796 clauses behind the same missing frame contract.
They need private callee captures, mapped parameters, ordered output unification, and call-site memo identities.
They also reach downward recursive utilities, so call-frame lowering alone cannot close those grammars.
The current implementation therefore concentrates on the shared downward prerequisite.

The first focused run passes eight tests and fails one invalid refusal fixture.
Policy validation rejects its immediate mutual cycle before SQL compilation.
The corrected fixture puts the mutual dependency through child walks, so the SQL refusal remains observable.
Filtered and unfiltered child work, branching order, publication, and logical-depth tests pass.
Shared-root samples at depths 8, 32, and 128 use 14,446, 51,934, and 201,886 VM instructions.
SQL remains 9,370 bytes.
Skipped and exhausted invocations retain constant work as the unused guard domain grows.

Catalog compilation admits B110 and B112, which raises coverage from 76 to 78 grammars.
F502, F503, and F508 now reach their missing parsed-tree contract.
SIM910 now reaches its missing binding-site contract.
These are first-refusal advances, not additional completed grammars.
Many other pure wrappers already used the total-relations path, so the eligible utility count overstated the possible coverage gain.

The physical review identifies unnecessary depth-event work for shallow forests.
A constant guard now checks whether any forest node reaches the available logical depth.
If none does, SQL skips both depth-event streams and their window sort.
A separate wide-sibling regression checks that repeated roots at the same depth retain linear work.

The shallow-forest guard reduces the depth-128 sample from 201,886 to 175,967 VM instructions.
The largest disconnected-domain sample drops from 200,628 to 148,579 instructions.
Shared-root depths 8, 32, and 128 use 12,647, 45,311, and 175,967 instructions, with 9,562 SQL bytes.
The wide-sibling samples use 36,081, 138,777, and 549,561 instructions for 8, 32, and 128 groups.
Their SQL remains 9,535 bytes.
These gates demonstrate linear growth across shared roots and branching width.
Exhausted invocations remain at 600 instructions, and skipped invocations remain at 703, regardless of unused guard-domain size.
The corrected refusal test passes.

The final frozen build passes all 272 relational tests.
All 231 preceding and 54 focused SQL/VM control profiles remain unchanged.
Formatting, workspace Clippy, and profiling-feature Clippy pass.
The independent final review finds no additional material defect in the depth guard or window boundaries.

Frozen B023 passes every existing CPU, VM, and query-shape gate without threshold changes.
It retains 246,364 VM instructions, 10,232 SQL bytes, one matching statement, and zero frame publications.
Median prepared execution is 4.104 ms against 30.787 ms for the reference, across nine executions per query.
Preparation remains a separate one-time cost.

Final catalog qualification passes all 78 admitted grammars on all 595 fixtures, with a positive witness for each grammar.
The preceding 76 retain exact SQL hashes, VM counts, and output hashes.
B110 has 33 matches and uses 1,042,522 VM instructions across the batch.
B112 has eight matches and uses 1,032,132 instructions.
Each grammar uses one matching statement and zero frame publications.
The shallow-forest optimization improves synthetic recursive workloads but does not change these two catalog VM totals.

The complete-coverage gate still fails with 68 explicit refusals.
`benchmark/sql-relational-downward-integration.json` retains the SQL, explained plans, before/after samples, grammar comparisons, and four first-refusal advances.
The coverage and qualification artifacts retain source and executable hashes, fixture inventory, exact outputs, and native per-file work traces.
The checkpoint remains test-only.
Production replacement, effectful recursion, native stack-byte equivalence, aggregate memory qualification, and complete-job batching remain incomplete.

### Larger catalog batch: integration in progress (2026-10-24)

The user requested larger batches instead of repeated two-grammar checkpoints. This batch integrates four contracts before catalog qualification: binding targets and occurrences, parsed trees, parameter frames, and format arguments. Three GPT-6.1 Sol agents own separate implementation areas. The parent reviews shared changes and runs the integrated gates.

The binding resolver returns facts only. SQL sequences target children and occurrence candidates. Parsed trees use the existing subfile registry and atomic budget accounting. Parameter calls isolate private captures and use call-site memo keys. Format analysis returns ordered candidates; SQL evaluates their child rules.

The first compile found an overly narrow visibility declaration in the new parse helper. No execution evidence came from that build. The declaration was corrected before the second build. Tests and catalog counts remain unqualified until the integrated binary passes.

The next recursive scheduler keeps a shared structural forest and evaluates semantic base cases only when reached. Its first contract excludes native memoized recursive utilities. It must preserve ordered children, entry charges, logical depth, unknown, and capture rollback. Native stack-byte equivalence remains outside the existing qualification.

The first executable checkpoint passed catalog parity for 91 of 146 grammars, compared with 78 at the previous commit. All admitted grammars had positive fixture witnesses. The new admissions cover binding-backed exec/eval, six format-argument rules, three semantic resolution wrappers, repeated variable keys, and debugger calls. These are diagnostic results from `/tmp/rerail-broad-phase1/engine`, not the final source qualification.

Targeted binding, occurrence, call, format, and recursive-search suites passed at this checkpoint. Three parsed-tree tests failed because lowering still rejected literal-value leaves inside parsed documents. The fix restricts that admission to parsed children and associates their instrumentation with the host file. A new test checks parsed value probes and utility replay across two files.

Review also found that private call namespaces could prevent native-equivalent memo sharing with ordinary utilities. The next checkpoint normalizes memo identities and capture columns across frames. It preserves physical capture isolation. The native utility scope determines which logical captures enter each key.

The second catalog checkpoint passes all 101 admitted grammars, including 23 new admissions. Nested semantic bases now compose with pure chain and downward summaries at runtime utility depth. A direct depth oracle checks failure precedence at incoming levels 0, 1, 253, 254, 255, and 256.

The four-walk proxy limit was replaced with a budget on actual active typed returns. The bound is 128 cells. A scaling test initially expected a 32-level fixture to fit, but that fixture exceeds the bound. The test now records this expected resource refusal and measures admitted depths separately. The bound was not raised.

Review found two additional memo contracts. Binding context must identify the original child rule across private capture namespaces. Parsed-document utilities must bypass memo storage. New probe and finite-work tests cover both cases. The final focused checkpoint passes seven call-frame tests, seven parse tests, three format tests, and three deeper-walk tests.

The final source also removes a redundant one-arm base alternative in semantic recursion. At widths 8, 32, and 128, VM instructions fall from 8,948 / 28,316 / 105,788 to 7,939 / 24,883 / 92,659. SQL remains constant across widths and shrinks from 8,018 to 7,308 bytes.

Native lint initially found three mechanical issues: a needless borrow, a complex inline type, and a redundant string conversion. Those were corrected before the frozen qualification build. The qualification driver now pins all new atomic helper sources and the utility-scope helper.

The first full regression run reported 305 passes and one obsolete refusal assertion. It expected a whole-node capture base to be rejected. That base is now supported. The assertion was replaced with finite-work native parity for capture-before-continuation behavior. Refusals for memoized bases and general cycles remain tested. Catalog parity still passed for all 101 admitted grammars during that run. Final qualification is repeated from a new frozen binary after this test correction.

The corrected full semantic suite passed 307 tests. A later key-layout review found an additional cross-namespace collision: a node capture and a count can share a name and integer carrier value. Native memo keys distinguish those types. Canonical SQL columns now use both logical name and capture kind. An adversarial test forces equal carrier values and checks two native and SQL probes in both invocation orders. Qualification is repeated after this correction; the preceding run is retained as diagnostic evidence.

The typed-key regression initially failed policy validation because its zero-minimum count omitted a maximum. The fixture now specifies an exact zero count before adding the node-ordinal offset. The corrected regression passes both invocation orders. This fixture correction did not change the engine contract or resource bounds.

Final frozen qualification passes 308 relational tests, all 285 preceding SQL/VM controls, and all 101 admitted catalog grammars. The catalog uses 595 files and has a positive witness for every admitted grammar. The preceding 78 retain exact SQL hashes, VM counts, and output hashes. The 23 new grammars each use one matching statement and zero frame publications.

Frozen B023 retains 246,364 VM instructions and 10,232 SQL bytes. Its nine prepared executions have a median CPU time of 4.085 ms, compared with 32.029 ms for the reference. Preparation remains a separate one-off cost. All existing CPU, VM, and query-shape gates pass. Native formatting and both clippy configurations also pass.

The full-coverage gate still fails as intended with 45 explicit refusals. Production replacement, complete-job execution limits, actual native stack-byte parity, and aggregate storage-byte limits remain unqualified. E721 still uses 102,275,790 VM instructions; ASYNC221 uses 41,753,849 and ASYNC230 uses 38,910,778. These measurements expose remaining outer-program overhead rather than evidence of native-speed parity.

`benchmark/sql-relational-broad-catalog-integration.json` records the new grammar SQL, scaling samples, resource contracts, unchanged controls, and remaining refusals. The coverage ledger and qualification evidence contain the frozen executable and source hashes. The transition plan now specifies the next failing test and return-phase contract for memoized recursion.

### Remaining catalog contracts: broad recursive batch (2026-10-24)

The user requested sustained work across the remaining 45 refusals. This batch covers structural recursion, memoized semantic recursion, and parsed capture transport together. Three GPT-6.1 Sol agents have bounded implementation assignments. The parent reviews their changes and runs the integrated validation.

Structural recursion uses a finite SQL control table and numeric continuation records. The compiler proves monotone structural progress for each recursive component. It refuses cycles that can repeat without moving. Utility-free predicates remain fused SQL expressions.

Memoized semantic recursion needs entry and return events. Completed child answers must become visible before later siblings run. Its scheduler stores seven numeric control fields per suspended caller. Original captures remain in the correlated outer demand. Successful exports cross return boundaries. Indexed immediate-child seeks avoid an origin-by-descendant relation.

Peak tracking records utility entry before the work charge. A cached answer fits only when its stored peak fits the current depth. A deeper invocation recomputes instead of returning an immediate depth failure. Failed calls unwind their control records and do not publish memo answers.

Parsed capture cells preserve decoded text, source coordinates, and an optional original node. Host hooks evaluate SQL at the original host node. Atomic callbacks provide facts and typed transport; they do not evaluate child rules.

These changes are not yet qualified. The baseline remains 101 admitted grammars, 308 passing relational tests, and 45 explicit refusals. The next checkpoint must check finite work, depth boundaries, unknown, capture rollback, native probes, statement count, VM growth, and continuation cleanup. Preparation remains outside the execution timing interval.

The first executable checkpoint admits 127 of 146 grammars at compile time, an increase of 26. This is not a parity claim. The semantic suite reports 332 passes and four failures. Three failures share one SQL preparation defect: an outer correlated alias cannot use `alias.*` in the scheduler seed. The seed now names each column explicitly.

The fourth failure detects an incorrect peak update on a pure recursive return row. SQLite can evaluate a `WHERE` expression before the code-table join. The peak update now checks the actual call instruction inside its guard. The independent oracle and expected peak remain unchanged.

Review also required native-equivalent memo replay for parsed capture shadows. Native replay can retain a node and reserved range without transformed text. The typed cell now represents that state. Additional tests cover preserved hidden text and range-only replay. Non-span plans must retain their previous SQL projections and VM counts.

Work continues beyond this checkpoint. The next extensions preserve recursive-first branch order, filter children before utility entry, and supply captured inputs to binding facts. A separate mixed-control compiler will compose finite semantic SQL regions inside general structural recursion. Its initial contract excludes capture writes inside recursive components.

The second executable checkpoint admits 132 grammars. All 231 preceding controls and 54 focused controls retain exact SQL hashes and VM counts. The semantic suite reports 343 passes and four failures. Two tests still expect newly supported recursion to fail compilation. One fixture regex excludes the nested array it intended to visit. One depth oracle starts its second series cold while the native literal cache remains warm. Each correction preserves the tested engine contract.

Recursive base alternatives now have separate SQL phases. This preserves whether the particular unknown arm can bind captures. Combining their write sets had made that distinction unavailable. The same scheduler now handles first or last recursive arms and all four neighbor axes. Non-child continuations use a visited flag, because node zero is a valid ancestor.

Review found a physical access problem in sibling traversal. With `parent = P AND id > previousChild`, the lower bound can lie inside the previous child's subtree. The node cursor then starts again at P's first child. The new seek uses the completed child's subtree endpoint and P's upper bound. Nested-sibling scaling tests measure this case explicitly.

The two newly admitted enum grammars exposed a missing file-condition contract. The planned atomic file guard evaluates metadata once per file and filters roots before work or probes. The catalog oracle must apply the same native file condition. This extends engine behavior; it does not exclude difficult fixtures from validation.

The third executable checkpoint passes native parity for all 133 admitted grammars. The existing stub enum fixture contains only values that the rule must skip, so successful stub publication still lacked a witness. A separate fixture under `fixtures/rerail/sql_compiler` adds duplicate and distinct enum values, unknown placeholders, and automatic values. The next catalog run uses all 596 fixtures. Baseline comparisons must rerun the prior frozen executable on this same expanded inventory.

The wider memo publication in ASYNC221 exposed SQLite residual checks beyond the first 16 virtual-table arguments. The cursor now retains and returns its original bound inputs for those checks. Tests cover 17, 32, and 72 inputs, including typed spans. No key constraints are removed.

Compiler resource refusals now report emitted operator categories. This shows where physical sharing is still missing without increasing the 256-operator bound. Pure recursive control is being reduced by fusing adjacent total predicate regions. Semantic regions that contain effects remain ordered.

The fourth executable checkpoint admits 134 grammars and passes native parity for all 134 across 596 fixtures. The separate contract suite still has 12 failures. Several are invalid test fixtures. Mixed recursion has a real preparation failure: an unparenthesized `OR` escapes a virtual-table join constraint. The planner then loses its required mode constraint. SQLite reports the outer tuple width instead of the underlying constraint failure. Numeric cell expressions now have explicit parentheses.

Predicate fusion has controlled before-and-after evidence. One tracked conjunction shrinks from 9,766 to 7,784 SQL bytes and from 680,924 to 564,656 VM instructions. A disjunction shrinks from 8,838 to 7,822 bytes and from 631,365 to 569,519 instructions. These results use the same plan and a private test-only deoptimization control. They do not imply catalog-wide speed parity.

The ordered recursive scheduler now represents each original alternative explicitly. Multiple recursive tails and field selectors retain their source order. A suspended caller records its resume phase as an eighth numeric cell. Failed alternatives restore the original capture inputs. A successful base also retains caller captures that another base could have written. This removes the single-tail restriction without expanding a recursive component into repeated SQL bodies.

Review found that sibling classification also needs a transaction boundary per reached demand. Materializing all lookups before all fills makes every row observe an empty cache. The lookup, affordable classification, and completed publication now execute inside one correlated typed row. A batched native oracle varies each row's allowance and checks empty completed results and failed prefixes.

The fifth checkpoint admits 135 grammars. All 135 pass native parity with positive witnesses across the expanded fixture inventory. All 285 prior SQL/VM controls remain exact. The contract suite reports 376 passes and eight failures, all in mixed scheduling or fixtures that select it.

The mixed scheduler exposes a separate materialization error. Its return dispatcher refers to one correlated side-effecting CTE from five `UNION ALL` arms. The frozen executable pops the same continuation more than once. Marking that CTE `MATERIALIZED` does not ensure one instantiation across those correlated arms. A debugger probe that replaces pop with peek lets the schedule proceed, confirming the repeated-pop diagnosis. The replacement dispatch uses one `SELECT` with guarded expressions and one pop, close, and publication sequence.

A separate review found incomplete span propagation through nested parameter calls. An outer parsed span can acquire its type after a nested body has been lowered. The final layout now closes span capability over the actual parameter alias edges. This preserves the existing SQL for components that cannot receive spans. A nested A-to-B call regression checks node-bound access and memo probes for two parsed ranges on the same host node.

The single-select dispatcher passes a live debugger check before recompilation. Rewriting every SQL prepare in the frozen depth test makes all 16 static, dynamic, cached, depth-limited, and zero-work cases pass. Each case leaves no numeric continuation rows or open peak records. The next compiled checkpoint must confirm the same result without debugger rewriting.


### Broad recursive batch: final qualification

The final frozen executable passes all 385 relational tests. All 135 admitted catalog grammars pass native parity across 596 files. Each has a positive witness. This admits 34 additional grammars in one batch and leaves 11 explicit refusals. The full-coverage gate still fails as intended.

Both native lint configurations pass. All 231 preceding controls and 54 focused controls retain exact SQL hashes and VM counts. Frozen B023 retains 246,364 VM instructions and 10,232 SQL bytes. It executes one matching statement and publishes no capture frames. Prepared median CPU is 4.479 ms against the reference's 32.460 ms. Preparation is reported separately.

The mixed scheduler now passes the compiled depth, cache, work, unknown, mutual-recursion, and cleanup tests. Its width samples use 15,607 SQL bytes at sizes 8, 16, and 32. VM instructions are 234,929, 461,097, and 913,433. The separate oracle checks empty continuation storage and a balanced peak stack after each invocation.

Baseline catalog comparison uses the prior frozen executable on the same 596-file inventory. All 101 previous grammars retain exact native and SQL outputs. Of those, 96 also retain exact SQL and VM counts. Five change:

| Grammar | Previous VM | Current VM | Previous SQL bytes | Current SQL bytes |
| --- | ---: | ---: | ---: | ---: |
| try-except-pass | 1,042,522 | 1,053,807 | 27,700 | 27,116 |
| try-except-continue | 1,032,132 | 1,043,753 | 27,700 | 27,116 |
| B507 | 22,062,220 | 25,073,720 | 82,983 | 69,691 |
| ASYNC221 run-process | 41,775,854 | 50,072,645 | 85,567 | 103,533 |
| unnecessary-return-none | 17,915,511 | 11,334,102 | 69,986 | 64,279 |

These regressions stay visible in the evidence. Exact peak tracking and native memo publication add work to some existing plans. Predicate fusion reduces SQL and VM work in its controlled samples, but does not establish a catalog-wide improvement. Several clauses still execute tens of millions of VM instructions.

The remaining refusals are six recursive-context contracts and five resource limits. Shared finite functions need lazy typed calls with distinct native memo identities. Recursive parameter frames and cross-document capture returns need explicit contracts. The compiler's source and emitted operator limits remain 256; active typed returns remain limited to 128 cells.

`benchmark/sql-relational-recursive-catalog-integration.json` records new grammar SQL, controlled profiles, unchanged cases, and cost regressions. The coverage and qualification records pin the source, executable, fixture inventory, and failed checkpoints. The transition plan describes the next two implementation tracks. Production replacement, native stack-byte equivalence, aggregate storage-byte bounds, deadlines, and the complete-job 100-execution gate remain unqualified.
