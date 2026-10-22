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
