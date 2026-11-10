# Async family integration comparison

## 2026-11-08: scope and baseline

The comparison branch starts at `49035cc9098d3cafb19a1e4c33bc45ab796dce28`.
The integration worktree has uncommitted changes. They are outside this comparison.
The historical reference is `jeremy/shared-structural-execution` at `e1ffeaf25`.
Its measured source is `a55956e33`.

The corpus is `neralabs/nera-data-layer` at `fce4dc7208c0b892c11a549c8be43fc285580d6b`.
Python blobs were recovered from Git without changing a source worktree.
The recovered corpus contains exactly 5,725 files and 95,697,853 bytes.
The manifest records each path, Git blob, byte count, and SHA-256 digest.

The four rules are ASYNC210, ASYNC221, ASYNC222, and ASYNC230.
A compiler comparison requires all four clauses. Silent omissions fail admission.
The negative corpus cannot prove coverage; positive fixtures check each rule and dependent branch.
The reference, its generated SQL, retained indexed native, and current native must use the same inputs and build.
Preparation, execution, and diagnostics have separate measurements.

Initial source audit found two opportunities:

- The integrated Python family starts from call postings, but builds arguments together with all call facts.
- The selective argument producer and import screen exist in the integration, but their current consumers are the reference tests.

The admission probe will record exact compiler refusals before execution timings are accepted.
No specialized reference plan will be presented as output from the generic compiler.

## Comparison paths

The JSON matcher slice and the authored `.rr` compiler are different paths.
A refusal in the former does not establish a limitation in the latter.
The four authored `.rr` rules exist and can run as one combined program.
The comparison uses that combined path, not four independent `run-policies` calls.

The `.rr` rules read `misc2-async.rr`, `ruff.rr`, `pyflakes.rr`, and `pathlib.rr`.
They derive context and resolution through those libraries.
They do not read the copied `py.*` families or the reference's selective semantic producers.
This makes a production measurement necessary: the reference's two statements are not the production plan.

The harness retains both native oracles.
The retained corpus-indexed oracle matches the historical measurement boundary.
Today's per-file oracle includes per-file index construction inside its execution measurement.
Both must produce the same raw finding keys as the reference.
The authored `.rr` program also gets a positive test with 24 files and 120 expected findings.
Reference edge fixtures can be exported for a separate `.rr` comparison.

The delegated implementation was reviewed before wiring.
The review removed a nested profiler mutex that would deadlock the corpus runner.
It also separated profiled diagnostics from unprofiled timing runs.
Missing output files now fail instead of being interpreted as empty findings.

## Build identity

The system Homebrew Rust installation cannot start because its LLVM library has changed.
This run uses the installed rustup `1.97.1-aarch64-apple-darwin` toolchain.
The build uses the repository's `profiling` profile, with optimization level 3.
All fresh comparison arms use the same test binary.
The historical release binary used different code-generation settings and dependencies.
Historical timings therefore remain a reference; fresh timings compare current arms directly.
The build cache was cloned into a separate artifact directory before compilation.
Neither the integration worktree nor its target directory was changed.

## Reusable mechanisms from the older branch

The audit also checked `jeremy/sql-capture-demand-spikes`, not only the reference branch.
Commit `c6ff81104` contains three small changes worth retaining as candidates:

- Cache Python reference resolution, including negative results, within an immutable scope table.
- Reject an impossible attribute suffix before walking and allocating the full import chain.
- Pass the constructed import chain to `is_import` instead of constructing it again.

The second item needs a precise qualification.
Current `import_chain` already rejects a mismatching suffix before scope resolution.
The missing fast path saves chain construction; it does not introduce the existing rejection semantics.
The resolution cache also needs adaptation to current `resolve_load_pair` and global-binding behavior.
These are candidates, not measured wins on the current integration.

Common-label factoring, capture-delta memoization, scope sharing, and hardware counters already exist in the integration.
The old continuation/cohort scheduler is not a missing optimization.
Its recorded full workload cost was 1.788 times native CPU.
The fixed-pattern experiment saved 11 of 1,387,324 executions and was removed.
Those experiments should remain evidence, not become another execution path.

## First measurements

The six reference checks passed, including all 120 positive findings on 24 documents.
The full reference comparison passed on all 5,725 files.
Median independent/reference CPU was 265.988 ms; generated SQL CPU was 263.694 ms.
Every generated run used two statements, 12 semantic requests, and 1,886 VM instructions.
All three full runs found zero findings, with agreement between both native oracles and the reference.

The retained corpus-indexed oracle used 269,128 candidate roots.
Its matching median was 1,025.534 ms, but its selection cost was 33–40 seconds on every run.
The current per-file native oracle used 7,484,956 roots and a median 5,098.691 ms, including selection.
Its median selection cost was 411.089 ms.
Thus matching cost alone hides a substantial selection-cost tradeoff.

The inherited harness incorrectly labelled every repeat as a cached frontier.
The recorded selection times contradict that label.
The published analysis uses the measured times, not that boolean.
After measurement, the harness metadata was corrected to record the repeat index without claiming a cache hit.
`measured-tests.rs.gz` preserves the exact harness source used by the measured binary.
This metadata-only correction changes no executed measurement or comparison logic.

The authored `.rr` program passed the same 24-file positive case, with all 120 findings.
Its unprofiled CPU was 692.486 ms, including 546.878 ms in the initial stats/preparation phase.
The full-corpus production run is still in progress.
The cache inspection during that run found over 65,000 Python tapes, far more than the supplied files.
Cached fragment text includes ordinary prose, wrapped as Python expressions.

## Request liveness risk found during the production run

The source audit found a general request-liveness problem:

1. `logic/rr/lower.rs::demand` merges requests from different consumers into `rr_embed_req`.
2. `logic/plan/corpus.rs` checks whether any output-reachable rule reads guests.
3. If one does, the runner promotes the shared request relation to an output.
4. Relation-level pruning then retains all clauses producing that request relation.

The request no longer records which consumer made it.
A request from an unused library rule can therefore survive because another consumer is live.
A minimal fixture should compare a live, text-restricted `parsed` rule with and without an unused, unrestricted `parsed` rule.
The unused rule must not cause an unrelated string to be parsed.
This fixture is a proposed regression test; it has not been executed in this comparison.

The generic repair must preserve consumer provenance until liveness is established.
Request dependencies need a closure for recursive parsing.
Only then should the compiler merge equivalent requests.
The same concern applies to parse, splice, and file-system requests.
The full-run profile must establish which producers cause the observed fragment work.
The source finding alone does not establish that `annotations.rr` caused it.

## Shape comparison supported by the reference measurements

The reference visits 691,328 calls and selects 65,399 calls in async contexts.
It screens 65,292 primary references and admits none for full name resolution on this corpus.
It still runs 12 secondary semantic requests across five scopes for the dependent receiver cases.
No target requires argument extraction, so argument calls and rows are both zero.
This is a stronger boundary than merely sharing library code across the four rules.
The authored rules derive general name and scope relations before their target API filters.

A brief live stack sample was taken during the long production run.
It found the evaluation thread in `logic::native::setfix::Rel::push`, while the next-chunk worker waited.
The sampler reported an 8.2 GB peak footprint.
Only two stack samples were collected, so this is evidence of a live execution path, not a statistical CPU attribution.
The run's timing includes this external sampling interval; the profiler process's CPU is not included in the engine's CPU clock.

## Correction to the retained native adapter

The repeated selection cost has a specific cause in the copied adapter.
`Native::run` constructs new `Arc<AnchorPlan>` values on every call.
The retained corpus keys its prepared candidate cache by those pointers.
Thus the adapter defeats reuse, unlike the historical compiled grammar's stable plans.
The comparison branch now retains the adapted plans for the oracle's lifetime.
The full reference/native comparison must be rerun after rebuilding this correction.
The first report is retained as evidence of the adapter defect, not as a warm native benchmark.
This change affects only the test oracle, not the production compiler or matcher.

## Integration advanced during measurement

The sibling reached `6ae30c48a7d9c7cde74248ccecc0c4835ba54308` during this run.
Three commits followed the pinned baseline:

- `92b5bbe2c`: reuse the collection lease instead of opening a lock file for each guest parse.
- `e88cdd20d`: read a source row's columns after locating its document once.
- `6ae30c48a`: cache the combined program by its module graph and extend per-file memo coverage.

These changes can reduce costs in the measured paths.
They do not change the async rule definitions, selective semantic producer wiring, or request-provenance issue.
Timings in this comparison remain pinned to `49035cc909`; they are not timings of the newer head.
The newer worktree also has an uncommitted preparation-cache edit, which remains outside this comparison.

## Completed production and producer measurements

The unprofiled combined `.rr` run completed on all 5,725 files and 19,007,630 base nodes.
It produced zero findings, as did native, the reference, and a separate Ruff 0.15.7 check of all 5,725 files.
It used 1,058.754 seconds of process CPU and 2,053.364 seconds of wall time.
The program was read from the preparation store; initial stats/preparation cost was 0.493 seconds CPU.
The file-local phase used 998.581 seconds CPU. That phase includes planning, guest processing, and derivation.
Cross-file work used 32.359 seconds CPU.
The runner executed 88,545 SQL statements in 234 chunks and reported a peak RSS of 11,658.461 MB.
The detailed profiled run is separate and must not replace these unprofiled timings.

The producer microbenchmark compared exact async call IDs in every document.
The posting-based producer's median was 49.040 ms; the reference structural scan's median was 134.972 ms.
The former built all call families and 1,005,452 argument rows.
The reference's selected demands need zero argument rows on this corpus.
Core-fact construction cost another 1,057.690 ms and is reported separately.
Thus the posting advantage depends on reusing core facts; it is not a free cold-run speedup.
The production plan should combine posting access with selective argument and semantic demands.

## Semantic differences, checked against Ruff

The authored `.rr` rules disagree with the reference on the exported edge fixtures: seven missing keys and four extra keys.
One pair is a report-line difference for the same parenthesized HTTP call.
Ruff 0.15.7 was run with isolated configuration, no cache, and only the four async rules.
It confirms that three `.rr` additions are real calls missed by the native/reference path around comments or parentheses.
The reference also misses a Unicode spelling of `open`.

Against Ruff, the authored `.rr` rules miss seven calls and place one call at the wrong line.
The missed calls include parenthesized or starred unpacking targets, an annotated assignment, and a parenthesized `with` target.
Both paths miss the Unicode attribute spelling.
These results require explicit semantic decisions; a zero-finding corpus does not establish equivalence.
The edge sources, raw results, and Ruff comparison are preserved with the benchmark.

## Final reference rerun and dominant structural cost

The stable-plan adapter correction was rebuilt and the complete reference stage rerun.
Admission accounting, all six positive/edge reference checks, and the full three-run comparison passed.
Final medians are 643.561 ms indexed-native matching, 166.865 ms independent reference, and 163.056 ms generated reference execution.
Current per-file native execution is 3,221.845 ms, with 7,484,956 matched roots.
Repeat indexed selection now costs 0.300 and 0.318 ms, confirming actual reuse.
All generated runs retain 12 semantic requests, two SQL statements, and 1,886 VM instructions.
Timing varies substantially from the first measurement; stable counters support the shape comparison.

The complete profiled production job costs 863.727 seconds CPU and 1,386.382 seconds wall.
It partitions into 391 chunks and executes 141,771 SQL statements with 58,047,536 VM instructions.
It is a separate diagnostic run, not an improvement over the unprofiled measurement.

The main cost is now mapped to source and emitted plans.
`pyflakes.rr` tests whether any outer scope binds a name with a count-equals-zero aggregate.
The lowering builds `rr_agg59(p,s,x)` without the consumer's demanded `(s,x)` keys.
Its ancestor join emits 1,342,302,187 bindings; the annotation-world copy emits the same number.
These counters sum join outputs across chunks, before final deduplication.
Their exclusive join CPU is 217.614 and 212.441 seconds.
Loading and indexing inputs for `unbound_ref` costs another 181.202 seconds.
Together these account for about 71% of profiled CPU, outside the SQLite VM instruction counter.
Planning the three strata costs only 0.006, 0.008, and 0.045 seconds.

The consumer also places selective nearest-scope checks after aggregates.
One branch reduces 661,051 intermediate bindings to 22 only at its last step.
Another reduces 725,605 to 96, then rejects those by scope kind.
The comparison report specifies demand-key propagation, early structural guards, and anti-existence probes as the repair.
It keeps source/annotation semantics distinct and does not assume that identical measured counts establish equivalence.

The report and compressed raw profiles are captured with binary/source provenance.
The recommendation is to port this harness into the active integration and repair its demand planning.
The posting producer is a measured integration advantage worth retaining.
The older scheduler is not a missing opportunity, and no production compiler replacement is included in this comparison.

## Final validation

The optimized test binary built successfully. The final reference stage passed admission accounting, six reference tests, and the full corpus comparison.
The production positive test passed earlier; exported edge parity deliberately remains failed and documented.
The corpus restore tool rechecked every blob and SHA-256 against the existing recovered corpus.
All evidence JSON and compressed profiles decode, and the summary retains both production execution-budget failures.
Python scripts pass syntax checks. No production engine changes or sibling worktree edits were made.

## Final sibling-head check

The integration advanced again to `d8fad367bc743e40b78e9b8b7ddcaa9e163a25df` before publication.
Commit `24d05da80` persists the prepared file/cross split; `d8fad367b` documents the O143 results.
The previously uncommitted preparation change is now committed; the sibling is clean.
Its 8.7-second warm and 61.3-second cold Python-pack results cover 534 programs over 486 files.
Those measurements have a different workload and memo boundary from this 5,725-file comparison.
The O143 annotation join-order fix, `5e335eb4d`, is already an ancestor of our measured base.
The final report distinguishes that repaired issue from the aggregate-demand expansion found here.

## 2026-11-09: aggregate and semantic demand repair, TDD

The first three aggregate tests failed against the unchanged lowering, as required.
They expose an unrestricted count/sum helper, a retained zero-count scalar, and the absence of a safe demand for a recursive consumer.
The red build took 11m26s on the shared host. The tests themselves completed immediately.
The design is in `aggregate-demand-design.md`; statement reuse remains a separate follow-up.
The implementation now restricts private helpers by consumer keys and converts scalar zero-count tests into anti-joins.
An execution test checks exact counts, sums, empty groups, world isolation, and helper row counts.
A second red gate requires the actual async policies to use demanded semantic producers instead of global `unbound_ref` derivation.

The four aggregate tests now pass, including actual SQL execution and exactly three helper rows per aggregate.
The next gate fails as intended: ASYNC210 still depends on global `unbound_ref`.
The next change adds reusable Python qualification, value-origin, and argument producers with explicit request relations.
The posting-based call producer remains the structural source. Its argument rows are now separate.
The core import screen shares admission logic with the reference screen and avoids scope construction for rejected requests.

The production code now passes `cargo check --tests --profile profiling`.
The semantic adapter uses the production tape-restore API, not a test-only restore helper.
New tests check selective scope creation, shadowing across documents, cached facts, and demanded arguments.
The aggregate execution test now also checks intermediate native join outputs.
Multiple-consumer and recursive-only demand tests cover the pass's union and refusal rules.
A separate result directory preserves the old measurements and fixtures.
Its tape cache starts from a consistent SQLite backup of the baseline cache; result memos remain disabled.

The first semantic build passes posting-producer parity, all import-screen tests, and the selective cache test.
The production positive test caught a real integration failure: native operators were all classified as cross-file.
Their requests reached a stage with no syntax batch. Python fact operators now enter the ordinary document-local proof.
Their inputs must still pass dependency and location checks; a cross-file target relation cannot be treated as local.
The split now exports native inputs only for operators that actually run cross-file.

The native aggregate trace exposed another gap. The planner scanned 201 bindings before applying three requested keys.
It did not create the full ancestor cross-product, but it did not follow the required demand-first shape either.
Aggregate demand relations now drive witness enumeration. Explicit recursive deltas retain priority.
The growth test also had a TSV-column error; it now inspects the plan column and checks every intermediate output.
A new gate checks the actual pyflakes outer-binding helper for load and nearest-scope demand guards.

The repaired build passes all targeted gates: seven aggregate tests, posting parity, four import-screen tests,
selective semantic caching, actual policy shape, native locality, and all 120 positive findings.
The native work-growth gate now passes with at most three intermediate rows per helper step.
The original pyflakes helper retains both load and nearest-scope guards.

All four edge fixture jobs complete. They match all 178 recorded Ruff 0.15.7 findings, with no missing or extra keys.
The old reference comparison still flags four additional findings; those are the reference's documented misses.
The old expected files remain unchanged so this distinction remains visible.

The RR regression run passes 73 tests with two ignored tests.
One memo test failed because the benchmark environment disabled result memos. It passes with normal memo settings.
All eight file-local regression tests pass; one device-dependent test is ignored.

The complete updated four-rule job takes 13.071 CPU seconds and 13.725 wall seconds over 5,725 files and 19,007,630 nodes.
It runs 72 chunks and 10,272 SQL statements. The prepared program comes from the store.
The baseline takes 1,058.754 CPU seconds, 2,053.364 wall seconds, and 88,545 statements in 234 chunks.
Both jobs report zero findings. The peak RSS falls to about 648 MB.
Detailed profiling and the unchanged legacy-policy measurement are still pending at this point.

The unchanged original policies complete their diagnostic run in 301.200 CPU seconds and 600.802 wall seconds.
This run includes cold preparation and 291 chunks; it is not the primary warm-program timing comparison.
Each former 1,342,302,187-row outer-binding join now follows 19,820 demand keys to 54 ancestors and one witness.
Their combined exclusive join CPU falls from 430.055 seconds to 0.0057 seconds.
The consumer's input loading and indexing falls from 181.202 seconds to 0.0151 seconds.
All four legacy edge outputs remain exactly unchanged, and its 120-positive gate passes.

The updated-policy profile records 15.356 CPU seconds, 14.044 wall seconds, and 2,283,088 SQL VM instructions.
It builds five scope tables and projects ten semantic nodes. Secondary constructor queries also pass through the target screen.
The existing posting producer remains in use, and no argument rows are requested on the full corpus.
Publishing and repeatedly reading `async_reference` accounts for 77.4% of remaining VM instructions.
The result report identifies this boundary and the remaining table lifecycle work without claiming the 100-execution gate has passed.

Final review found a separate correctness bug in generated native helper names.
The contracts `(a_b, c)` and `(a, b_c)` generated the same concatenated name.
An adversarial runtime fixture returned only one of two expected findings, confirming the failure before the fix.
The allocator now compares the complete native contract and disambiguates name collisions.
Identical contracts still share their helper. Ordinary async helper names remain unchanged.
A permanent regression checks both distinct contracts and exact-contract reuse; the final build and runtime check are pending here.

The final optimized build and all 17 targeted tests pass.
The adversarial helper-collision fixture now returns both expected findings, with exact-contract sharing retained.
The final unprofiled job takes 14.315 CPU seconds and 37.055 wall seconds in 73 chunks, with 10,414 executions.
The earlier 13.071 CPU-second sample remains in the evidence archive. Both use stored prepared programs and disabled result memos.
The final detailed profile takes 15.753 CPU seconds and 18.802 wall seconds in 72 chunks, with 10,272 executions.
It reproduces exactly 2,283,088 VM instructions, ten semantic projections, and five scope tables.
All full-workload and edge comparison gates pass. The 100-execution gate remains explicitly unmet.
The legacy diagnostic used the preceding binary; its exact source hashes and the two later-changed source files are preserved separately.
The final allocator correction does not affect that diagnostic's aggregate plans or legacy rules.

## Retained semantic demands after the successor merges

The literal Pulling-it-all-together tip was already an ancestor.
Merged the committed integration hub `aa2ef00ef` and executor `8bd924c72` into this branch.
Their uncommitted edits remain untouched.
The export-read conflict keeps the hub's keyed `rows_of` implementation.

The boundary design and gates are in `retained-semantic-demands.md`.
The first gate fails on the preceding profile: executed SQL mentioning `async_reference` consumes 1,773,972 VM instructions.
That includes setup and statistics; publication and the five major reads account for 1,766,340 instructions.
The merged baseline build is in progress before the boundary changes.
The implementation will use the hub's retained relation APIs and preserve SQL fallback consumers.

The merged build completes. Sixteen targeted checks pass; the legacy-extension aggregate check fails with `unknown join item sum`.
The merged planner now emits numeric reductions that the frozen text-symbol extension does not implement.
The adapter now declines those reductions for that extension, so SQL evaluates them.
The work-growth test also runs the production atom-backed engine and requires its native sum reduction.
The merged async positive fixture passes before the retention change.

The implementation reads each completed unary demand relation once and shares its snapshot among exact-relation consumers.
Semantic results enter the existing retained relation store with zero published rows.
The ordinary liveness schedule releases demand snapshots. SQL consumers still trigger materialization.
The regression covers separate target sets, duplicate keys, document identity, empty inputs, repeated runs, and both native and SQL joins.

The merged baseline's three warm CPU samples are 13.682, 13.596, and 13.942 seconds.
Median wall time is 28.910 seconds. All runs process 5,725 files with zero findings.
Its profile reproduces exactly 2,283,088 VM instructions and 10,272 executions.
The retained-candidate gate fails on this merged profile too, with 1,773,972 candidate-related VM instructions.
The first implementation build caught a `Vec<String>` membership type mismatch; the corrected build is in progress.

The first retained full-corpus profile passes the boundary gate: zero SQL mentions `async_reference`.
It reads 71 native snapshots and records 213 cache hits across 71 chunks.
Total SQL falls to 3,794 executions and 334,247 VM instructions. This first run fills the revised tape cache.
All 17 previous targeted checks pass. Three new retention checks pass; the integration fixture needs measured node statistics.

The wider RR suite passes 91 tests and exposes one obsolete shape assertion from the merge.
That assertion expects the inner count-zero expression instead of its anti-existence lowering.
The updated assertion retains its outer-key and context requirements.
A new execution test checks the nested example under SQL and native joins, with independently specified results.
The nine file-local tests, scalar reduction parity, and legacy native goldens pass.
The final build includes only these test corrections since the successful retained production profile.

The final build passes all 22 focused tests and all 93 active RR tests; two existing RR tests remain ignored.
The four edge jobs match all 178 recorded Ruff findings. The old reference-only gate still reports its four known misses.
The final profile records 336,517 VM instructions and 3,849 executions in 72 chunks.
No executed SQL mentions `async_reference`; 72 native snapshots serve 216 additional consumers.
Semantic projection remains ten nodes across five scope tables.

The initial separate timing series appeared to reduce CPU from 13.682 to 8.575 seconds.
Parsing and batch construction moved by a similar factor, so this was not accepted as the fix's speedup.
Three alternating pairs give median CPU 9.358 → 8.510 seconds and wall time 8.283 → 7.332 seconds.
Median local execution CPU is 0.674 → 0.567 seconds. Both timing series are preserved.
The 100-execution budget remains failed. The report identifies batch projection as the dominant CPU cost and records the remaining SQL setup work.

## SQL composition boundary audit

The user requested a structural audit before further local optimization.
Inspected source `eee4b8cd1`, its saved full-corpus profile, the checked CTE composer, and the merged executor's phase builder.
The findings and concrete next implementation are in `sql-composition-boundary-audit.md`.

The 3,849-execution profile contains no wave scheduler SQL.
Its production path computes most rules natively, then repeats SQL setup and publication across 72 chunks.
The general fallback separately executes per-rule inserts and per-relation wave folds.
Both boundaries need correction; changing only the fallback does not fix the measured async job.

The existing retained-candidate gate reports the 100-execution budget but does not enforce it.
The proposed acceptance gate must enforce that budget and require two relational evaluation queries for the complete four-rule job.
This audit changes documentation only. It uses saved measurements and claims no new runtime result.

## Production composition implementation

Merged integration hub `a27994c6a` as `abf75bb79` after preserving the audit in `e803a0f81`.
The hub includes the committed executor successor, parser cache, phase checks, and planner sketch work.
Resolved native exports, demand-first planning, and semantic locality against the hub's worklist implementation.
The first build also found a merged test initializer missing the new retained-statistics field; corrected that initializer.

Surveyed the executor, cross-file maintenance, shared structural execution, parser, and acceleration branches.
Keep the hub's source/index/statistics ownership and the executor's frozen relation contracts.
Do not import incremental support bookkeeping into cold relational queries.
The executor and cross-file worktrees have uncommitted work; this branch does not modify those files.

The saved baseline fails the new whole-job gate: 3,849 executions exceed 100.
The failing result is in the external `composed/budget-red.json` artifact directory.
The full-corpus benchmark now enforces the budget after writing its report.

The implementation builds CTE definitions before executing a statement.
Constants become query relations; all outputs share a tagged result stream.
Semantic leaves accept complete IN demands through SQLite's batch virtual-table interface.
The same statement can therefore compose their results without a host loop over semantic stages.
Sources still use bounded memory chunks. Initial validation must establish the whole-job budget before any claim of completion.

The initial composed build passes constants, fan-out/fan-in, aggregate, and anti-join tests.
The real four-rule positive fixture also passes.
Its full-corpus profile fails the hard budget with 114 executions: 72 composed queries and 42 remaining setup/final-assembly statements.
Final assembly still takes the old route because it uses input lifecycle hooks.
The new route now loads those inputs before the complete query and releases them afterward.
Adopted native inputs bind through the existing source cell, preserving prepared cursors and retained versions.

The first query profile records 5,055,466 evaluation VM instructions.
This exceeds the previous mostly-native path and remains visible as a regression to investigate.
The first build also enables `code-rule-profile`; final timing comparisons must use the baseline's default feature set.
Four qualification consumers rebuild the same IN key set.
The revised compiler collects a shared demand frame once and passes its handle to native producers.
The frame's scope ends with the statement, including errors; consumers share immutable keys and keep distinct target predicates.
Per-input cursor counters and EXPLAIN output supplement the whole-statement profile.

The frame version reaches 80 executions: 72 local queries, one cross query, and seven setup statements.
It still exports `async_reference` because dead native metadata survives rule pruning.
Removing that metadata and pruning again after simplification removes the export.
The next profile passes the complete-job shape gate at 80 executions and 4,944,696 VM instructions.
That VM count remains above the previous mostly-native path; this is not a performance win claim.
The input counters identify 65,412 call-site filters and 50,255 text filters inside the composed query.

The broader RR suite finds a separate semantic regression: failed scalar conversion produces a NULL-valued row.
The previous derived tables discarded these rows through non-null column constraints.
The new lowering rejects failed computed bindings and computed head values explicitly.
A focused regression also covers failure when the computed value is later projected away.
The pre-fix failure is preserved in `scalar-null-red.log`.

The edge jobs match all 178 recorded Ruff findings.
The original reference comparison retains its four known misses; its failure is not treated as a new engine regression.
The implementation and remaining whole-corpus and structural-prefix gaps are recorded in `composed-production-queries.md`.

Final validation passes 115 active tests; three pre-existing tests remain ignored.
All four edge jobs match all 178 recorded Ruff findings.
The final full profile passes at 80 executions and 4,965,288 VM instructions.
It compiles the local query once and the cross-file query once; SQLite preparation totals 4.842 ms.
Three unprofiled runs each execute 80 statements and reuse the prepared-program store.
Median corpus-run CPU is 13.221 seconds; median wall time is 17.894 seconds.
The median local phase uses 1.052 CPU seconds. Peak process footprint is about 664 MB.
The earlier timing series used another merged snapshot and was not interleaved; no causal speed ratio is claimed.

The final source and binary hashes, emitted SQL, EXPLAIN output, cursor counters, failure evidence, and results are checked in under `benchmark/async-family-integration/composed`.
The remaining SQL VM regression and the two-query whole-corpus target are explicit open items, not passing gates.

Implementation commit `90398a0d0` and the integration merge were published to the owned comparison branch.
The final ancestry check found that the executor tip's contents were present, but its merge commit was not an ancestor.
Merge `b34b0376b` records the requested executor tip `735bc4422` and changes no files.
The tested source hashes remain unchanged. Both requested committed tips now appear in branch ancestry.

## Investigation of the composition regression

The user rejected the VM regression and requested a causal investigation with profiling.
The change did more than batch existing SQL: it replaced native relational evaluation with SQL evaluation for supported graphs.
A comparison between historical whole-job timings cannot isolate that change because the merged source and host load also changed.

Added a test-binary-only SQL shape lab over the actual bound production sources.
It records per-case VM instructions, CPU, preparation, automatic indexes, sorts, input cursor counters, EXPLAIN, and result parity.
The cases vary join order, CTE materialization, and a duplicate-removal boundary.
Separate root queries measure inclusive producer costs; their costs are not additive.
Snapshot cases retain exact structural or semantic rows to measure the downstream query alone.
Snapshot capture costs are recorded separately and must not be reported as free native production.
A test-only native execution comparison preserves the same build, prepared program, source bindings, and planner cache.
The normal production budget test remains unchanged; diagnostic replays use a separate ignored test.

The same-build native/composed comparison completes three alternating pairs.
Median whole-run CPU is 12.933 versus 13.338 seconds; local user-plus-system CPU is 0.901 versus 1.128 seconds.
The earlier progress update quoted local user time alone, 0.825 versus 1.028 seconds.
The VM ratio includes Rust work moved into SQLite, so it is not a CPU ratio.

Exact-input replays identify a lost join order: preserving demand anchors removes the candidate automatic index.
It reduces attribute candidates from 268,338 to 31,393, and text filters from 49,513 to 31,393.
Blanket inlining is rejected: it raises VM work from 4.90 million to 75.91 million instructions.
Snapshot captures isolate downstream costs but do not establish free or equivalent native production.

The positive fixture exposes a separate defect in ASYNC222.
A constant prefix equality appears after inverse concatenation, causing enumeration of 4,096 positions before filtering.
Bounded positions improve the experiment; production uses the known nonempty prefix directly.
The initial focused test bypasses production simplification and fails planning before reaching this lowering.
The corrected test applies the same simplification and covers Unicode, long prefixes, and an already-bound suffix.
Review also restricts the optimization to unbound suffixes, preserving the existing bound-suffix check.

The pre-fix profiles fail both new VM ceilings: 4,971,727 on the corpus and 806,490 on the positive fixture.
The first production correction passes at 3,393,841 and 80,372 respectively.
The whole-job statement budgets remain 81 and nine. No gate is relaxed.
All four edge jobs match 178 recorded Ruff findings; the reference's four known misses remain recorded.
The full-profile query CPU barely changes, so the instruction improvement is not reported as a comparable CPU speedup.

The final build passes all nine composition tests, including the normalized prefix and bound-suffix cases.
Its full profile passes at 80 executions and 3,389,156 VM instructions; the positive profile remains 80,372 instructions.
Adaptive batching accounts for the one-chunk difference from the first fixed profile.
Three alternating before/fixed pairs show local CPU of 1.132 versus 1.111 seconds and whole CPU of 12.920 versus 12.953 seconds.
The fixes remove SQL waste but do not close the native CPU gap.

A further membership experiment reduces attribute cursor openings from 65,399 to 71, but warm query CPU doubles.
The native batch lookup still performs separate binary searches per key, while SQL constructs the membership list.
The result rows remain identical on both the full corpus and positive fixture. The rewrite is rejected.
Two initial lab launches fail configuration checks: a missing case file and a cache-specific atom literal in a replacement.
Those attempts are stopped and retained as configuration failures; the valid replacement leaves atom IDs untouched.
The runner now checks case-file existence and JSON before launching the engine.

## Candidate selection and bulk index spikes

The user requested concrete validation of earlier candidate screening and genuine bulk index traversal.
The plan and gates are recorded in `candidate-and-index-spikes.md` before implementation.
The experiment keeps existing semantic resolution and rule composition, and changes the structural producer boundary only.
Separate lookup experiments compare point probes, bounded merge, galloping search, and adaptive traversal.

The first positive fixture rejects the candidate rewrite with a circular CTE dependency.
A suffix search included semantic result relations among target inputs.
The corrected spike reads target relations from typed `Native::Python` metadata and preserves the failure log.
All four initial producer modes then preserve all 178 recorded Ruff findings.

Initial producer profiles reduce SQL work from 3.39 million to about 0.92 million VM instructions.
The range variant visits 67,385 calls instead of 691,328. It retains exact context checks for nested execution boundaries.
Screening every call first performs 690,248 qualification screens and takes 0.434 producer CPU seconds.
Context ranges perform 65,292 screens and take 0.070 producer CPU seconds.

The range producer still admits 3,083 direct candidates whose core qualification shape is absent.
The core helper conflates well-formed non-name heads with unknown syntax through `Option`.
The reference exposes `Known`, `NotApplicable`, and `Unknown` separately.
A refined spike preserves that distinction and passes a node-by-node comparison against the reference, including malformed syntax and Unicode.
An independent SQL guard checks for nonempty demands before semantic producer invocation.

The index matrix confirms that dense and clustered requests benefit from a merge; sparse requests do not benefit from scanning the range.
The complete-job profile measures only 4.91 ms in the original attribute index lookup and 1.30 ms with adaptive traversal.
Key-list decoding costs about 2.6 ms. These costs cannot explain the remaining local execution time.
The lookup variants therefore remain experiments; reduced comparisons do not establish an end-to-end engine win.

Alternating unprofiled runs reject the initial range producer as a performance winner: local CPU rises from 0.990 to 1.145 seconds.
This contradicts an interpretation based only on lower VM counts and isolated producer CPU.
A stack sample taken after process startup locates 373 of 429 composed-executor samples under `sqlite3Reprepare`.
An earlier sample captured only dynamic-loader startup and is retained as unusable profiling evidence.

Batch setup repeatedly registers native, symbol, string, and sum functions before cached statements execute.
The next spike uses SQLite connection-owned client data to install these functions once per connection.
It adds an automatic-reprepare counter and a focused invalidation/lifetime test.
A separate demand-frame gate tests an explicit empty outer relation instead of duplicating demand expressions through `EXISTS`.


The full-corpus counter records 71 automatic reprepares in the baseline and zero with stable setup.
The positive gate and all 178 edge findings pass with the demand-frame gate.
However, the nine-test composition suite catches stale adopted inputs with stable registration: the second run reads seven instead of nine.
The same test passes without stable setup.

The composition path creates a local `Modules` owner for adopted inputs and drops it after each run.
That removes the current binding from the registry while cached SQLite statements retain the older binding cell.
Repeated function registration had forced recompilation and masked this lifetime dependency.
The revised spike keeps those binding owners in connection-owned state and continues rebinding their current sources.
The existing replacement test is the regression gate; cached statements must observe new versions without recompilation.


The connection-owned binding correction passes all nine composition tests, including the previously failing input-version replacement test.
The registration unit test, positive 120-key fixture, and all four edge jobs also pass on the final binary.
The final series compares stable registration alone, typed candidates, independent `EXISTS` guards, and nonempty demand-frame drivers.
The runner records automatic recompilations and keeps alternating unprofiled timings separate from instrumented profiles.


The final profile records 80 executions and 732,887 VM instructions, with zero automatic recompilations.
The baseline records 80 executions, 3,386,852 VM instructions, and 71 automatic recompilations.
The frame-driven query takes 250 ms, including native leaves and initial preparation, against 774 ms for the baseline.
An independent `EXISTS` guard raises initial SQLite preparation from 8.1 to 31.2 ms, even after setup becomes stable.

Three alternating unprofiled pairs reduce median local CPU from 1.090 to 0.503 seconds, and whole corpus-run CPU from 12.387 to 11.765 seconds.
The acceptance script passes all gates, including the original statement budget and positive VM ceiling.
The final profile locates most remaining cost before SQL: projection/interning costs 5.40 exclusive CPU seconds, and text tests cost 0.86 seconds.
The SQL-only experimental rewrite occurs after `Projection::of`; production integration must lower typed candidate demands before source-demand analysis.
Deferred projection itself remains unbenchmarked by these spikes. The report distinguishes that integration requirement from the measured SQL-boundary improvement.


## Candidate demands before projection

Merged the integration hub at `cd0d66944`, preserving its structured component specs, scheduler, retained statistics, parser, and typing changes.
The first merge build catches two incomplete integrations between structured specs and level batching.
The repair carries typed join specs through seed batching and hashes their typed values, without converting them back to text.

The implementation introduces `python_candidates` during authored-rule lowering, before pruning, combination, preparation, and projection planning.
Its inputs specify a document-scope relation, a qualified-target relation, an execution context, and a receiver member name.
The async library supplies one shared conservative family target relation. Rule-specific exact qualification still determines findings.
Only producers with identical input relations and literal parameters share a result.
The candidate relation contains call, report, direct-reference, and receiver identifiers. Absent direct or receiver demands use the existing negative-node sentinel.

Projection planning then sees no broad attribute-text dependency for this family.
The batch builder skips per-node scalar materialization when none of those relations is read, while retaining the existing node identity mapping.
A narrow projection must preserve requested node and line facts and leave unrelated text, operator, and literal relations empty.
General full-projection fixtures remain a parity gate.
The accepted nonempty demand-frame and connection-lifetime fixes become the production path, without runtime experiment switches.

Validation compares the merged baseline, its accepted late spike, and production early lowering on the same corpus.
The gates cover scope isolation, mixed direct/receiver candidates, cached input replacement, serialization, all 178 edge findings, and 120 positive findings.
Profiles must show reduced projection strings and worker CPU, not only fewer SQL callbacks.

The merged baseline built successfully after the typed-spec compatibility fixes.
The executable and both rule-library directories were frozen before the candidate comparison.
The first candidate build caught an outdated relation-argument array length.
The registry length now includes the new operator.
Design and acceptance criteria are in [early-candidate-demands.md](early-candidate-demands.md).

The production build passed 32 focused tests, including full-projection behavior and cached adopted-input replacement.
All 120 positive findings and all 178 recorded edge findings match.
The first whole-corpus profile falls from 10.521 to 4.222 CPU seconds against the accepted late experiment.
Projection own CPU falls from 3.993 to 0.407 seconds. The profile records zero scalar AST nodes and 23,777 interned strings.

The profile also catches a modest SQL increase: 732,887 to 785,573 VM instructions, and 80 to 81 executions.
The document-scoped production contract adds one document scan per chunk.
Adaptive packing starts with three small probe chunks instead of two, yielding 73 chunks rather than 72 over identical input nodes.
The sharing gate now checks one producer per reported chunk. The existing 100-execution and 800,000-VM ceilings remain unchanged.
Repeated unprofiled runs are in progress. Profiled and unprofiled measurements remain separate.

Three paired unprofiled rounds confirm the gain. Median corpus-run CPU is 11.104 seconds for the merged baseline,
10.236 seconds for the accepted late experiment, and 3.714 seconds for production early lowering.
Median worker batch CPU falls from 5.858 to 0.418 seconds against the late experiment.
All fourteen acceptance gates pass, including the unchanged execution and VM ceilings.
The host remains busy, so CPU is the primary comparison. Wall-time samples and warmups remain in the evidence.
The final report separates profiled costs, repeated timings, preparation cost, and remaining tape/core-fact work.


## Built-in structural profile analysis

The profiler now writes `analysis.txt` and `analysis.json`, and places the summary first in `profile.txt`.
It reports attributed CPU, evidence, confidence, and a concrete action for each finding.
It does not execute diagnostic queries or change the physical plan.

The first test run has six expected failures and two passing negative controls.
The implemented analyzer then passes ten direct tests and three production-adapter tests.
Two fixture rows come from the original archived production profile, preserving both 1,342,302,187-row joins.
The negative controls reject the assumption that zero findings or a large scan alone prove wasted work.
Automatic indexes and point probes produce investigation advice, not an automatic index or batching prescription.

Projection planning records consumer relation paths before input construction.
Tape-cache instrumentation distinguishes hits from misses. Existing join, cursor, SQL, projection, and reprepare counters supply the other evidence.
The diagnostics distinguish observed behavior from a possible optimization. Reported CPU is not a claim of recoverable savings.

Both full 5,725-file runs pass the existing execution budget and finding checks. The positive fixture retains all 120 findings.
The late rewrite reports `LATE_PROJECTION`: 3.944 own CPU seconds, 12,535,139 scalar visits, and 4,187,794 batch strings.
Its consumer path is `text <- async_open_receiver <- p3_ASYNC230`.

The current run clears late projection, unrequested scalars, and automatic-reprepare warnings.
It reports 1.546 own CPU seconds in tape restoration and core-fact construction, plus 0.427 seconds in identity projection.
The cache counters include 5,725 corpus tapes and two rule-program tapes. All are hits.
Identity projection still constructs 12,535,139 node identities. These are measured costs, not proven savings from a cache.
Both runs execute 81 statements. Current VM instructions remain 785,573.

[The checked-in reports](../../benchmark/async-family-integration/analysis/current-analysis.txt) include the current findings.
The sibling files contain the failing-shape report, raw profiles, test logs, command environments, and source hashes.
Normal `RERAIL_PROFILE=<directory>` runs produce the analysis automatically. No separate analysis script or experimental engine path is required.

## 2026-11-10 — Input lifecycle investigation after integration merge

Merged integration hub `4ab83ed5190d1af7675f6a66544f9b35b972d9b1` as `547a99099`.
The merge preserves candidate demands before projection and accepts the hub's typed joins and parallel input construction.

The prior 0.426826-second finding overstates identity-map attribution.
It measures the parent projection span, which also contains token spans and line setup.
Added separate spans for identity maps, token spans, projection lines, node columns, children, and postings.
A regression test first failed because child spans disappeared from the analyzer's phase totals.
The analyzer now sums exclusive costs across each phase's subtree and prices identity maps only from their exact span.

Test-only experiments compare interval-based line lookup, counting placement for kind postings, and exact vector reservations.
Production keeps one execution path. Measurements use the merged compiler and pin `RERAIL_THREADS=4`.
The corpus has 5,725 files but only 182 redundant source bytes across duplicate contents.
An intra-job content cache therefore has little opportunity on this workload.
Results and costed opportunities follow in `input-lifecycle-opportunities.md` after the runs finish.

The optimized build passed. Baseline and combined variants each passed 45 focused tests, including all 120 expected positive findings.
All 47 corpus executions preserved findings and the 81-execution budget. The eight profiles each recorded 785,573 VM instructions.
Warm restore/core-fact costs are 1.392–1.466 CPU seconds. Exact identity-map costs are 0.166–0.173 seconds.
Counted kind postings and interval line lookup reduce their measured phase costs by approximately 0.19 and 0.13 seconds.
Three adjacent combined pairs save a median 0.263 CPU seconds and 0.133 wall seconds, with one small regression.
The earlier 0.758-second difference between nonadjacent medians is not a supported speedup claim.
Exact reservations save 87.7 MB of cumulative capacity but show no reliable CPU gain.
Full retention costs roughly 1.5 GB or more, while the existing result memo already bypasses unchanged-file input work.
The costed recommendation is kind buckets, then demanded line columns, then a direct validated tape reader.
The complete evidence, overlap limits, effort estimates, and rejected cache assumptions are in `input-lifecycle-opportunities.md`.

## 2026-11-10 — Production input lifecycle conversion

The accepted scope is counted kind postings, demanded line columns, direct validated tape decoding, demanded structural projection, and exact reservations.
A full retained cache is excluded.

Counted buckets now build kind postings in production. The test-only selection switches are removed.
Core facts no longer allocate start/end-line arrays. The T-shaped line reader looks up demanded positions in the existing line table.
A shared tape visitor validates both matcher restoration and direct fact construction.
The cached-input path writes facts from that visitor, without intermediate matcher records or sibling-link reconstruction.
Validation still checks source identity, grammar codes, aliases, abstract classes, missing/error nodes, UTF-8, tree boundaries, and sibling ordering.
The visitor additionally rejects a child subtree whose declared end escapes its parent.

Projection retains its dense core/local ID maps and reserves the core map before filling it.
Parent IDs derive from existing core columns and the local map.
Ordinal, subtree-size, depth, and field arrays allocate only when read.
Token counts preserve the existing ID space. Token ranges materialize only when a token span is read.
Source and line spans use address arithmetic rather than duplicate range storage.
Projection line tables share the source and build on first use.
Regex capture spans remain owned, with unchanged offsets after tokens, source, and lines.

Added direct-versus-restored column checks, malformed-input admission checks, and independent eager projection/ID checks.
The optimized build and full-workload measurements are pending.

Scope audit: uncached corpus and guest construction now use the same direct reader.
Callers with an existing matcher tape retain `from_tape`, which does not reparse their source.
The first new column, corruption, and projection-ID tests passed.
A borrowed line-table helper now indexes without copying source text. Shared projection line tables remain lazy.
The final source hashes are captured before the final build and corpus comparison.

The first full validation passed 107 focused test executions, all 120 positives in both ID modes, and all 178 edge-fixture findings.
Three adjacent warm corpus pairs improved CPU by 1.018–1.196 seconds. SQL stayed at 81 executions and 785,573 VM instructions.
The uncached 5,725-file run also passed, with 5,725 direct admissions and no input-tree restoration.

A containment regression check exposed repeated lazy-column checks in bulk loops.
Its exhaustive reference loop rose from about 1.13 to 1.40 CPU seconds.
A separate real nearest-scope job over 100 files and 3,791,229 nodes improved overall, but local CPU rose from 0.13 to 0.15 seconds.
Bulk containment/statistics readers now borrow their demanded column slices once outside the loop.
The reference check does the same while retaining its exhaustive pair comparison.
Initial measurements are retained under `pre-bulk-*`. Final measurements follow the rebuilt bulk readers.


Final production measurements pass the full validation set again: 107 focused checks and 178 edge-fixture findings.
The three adjacent corpus pairs save 0.793, 1.105, and 1.124 CPU seconds.
Median corpus CPU falls from 3.926 to 2.934 seconds. Median subprocess CPU falls from 4.014 to 3.035 seconds.
Every full-corpus report retains 5,725 files, 19,007,630 nodes, zero findings, and 81 SQL executions.
Both profiles retain 785,573 VM instructions.
Restore plus fact construction falls from 1.805 to 1.025 CPU seconds. Projection falls from 0.754 to 0.301 seconds.
The publication metrics show 576.2 MB less cumulative payload/capacity, not peak resident memory.
Token spans allocate for 34 documents. Projection line tables still materialize for every document and cost 0.094 CPU seconds.
The uncached corpus passes with 5,725 direct admissions and no input-tree restoration.

The largest-file nearest-scope query preserves all 140,230 rows and improves total phase CPU from 1.02 to 0.86 seconds.
Its local CPU changes from 0.22 to 0.23 seconds.
The exhaustive containment test remains slower after bulk access: about 1.10 versus 1.97 subprocess CPU seconds.
Sampling lands in its quadratic reference calculation but yields only two samples per arm.
Added separate thread-CPU measurements around reference construction and relation checks to determine the actual split.
This last change affects only test reporting. The measured production implementation stays unchanged.


The final optimized build passes. Both instrumented containment runs pass for 8,416 nodes.
Reference construction costs 1.425–1.484 CPU seconds. Relation execution and comparisons cost 0.0298–0.0301 seconds.
About 98% of the measured test phases belong to the quadratic reference calculation.
This separates the correctness-test runtime regression from the engine query cost.
The final report retains both results and the source/binary identity of each measurement.


## 2026-11-10 — Integration merge and sibling-workstream survey

Selected the integration hub's committed local tip `b4f0aa04b` after fetching origin.
The remote tip was `3ba1008cf`. The local tip adds the committed definition-ID integration.
The hub also has an unfinished executor merge from `72a3a658b`; its working changes are outside this merge.

Resolved conflicts in projection numbering, projection tests, and typed native start specifications.
Definition/granule IDs remain the default, including guests. The explicit `node_order` relation remains available.
Direct validated fact admission, counted postings, lazy columns, and lazy token spans remain in place.
Granule digests find subtree boundaries through ordered core IDs, without demanding the full projection-size column.
Both sets of tests remain present. The native start specification retains `Hash` and the integration's level semantics.

Surveyed 22 worktrees through their committed tips, status, notes, and working diffs.
The detailed findings are in `sibling-workstreams-2026-11-10.md`.
The snapshot is under `benchmark/async-family-integration/merge-b4f0aa04b/`.
No sibling working files were changed.

The SQL/store-wait profiler already exists in `More-ergonomic-rule-syntax` at `d3ef2ab8f`.
RER-131 owns the changing tape/token representation. The two cache branches own retention and relation identity.
The executor branch owns the shared pool and placement of work. Typing-support's stable IDs enter through this merge.
The singleton tape lookup remains in all six relevant working copies inspected.
Bulk tape admission remains a useful scope here, with no new cache or private executor.
The old 25% timing result does not describe this merged default-ID configuration.


The optimized merge build passes. All 110 focused test executions pass.
The positive async fixture passes with the new default IDs and with explicit preorder IDs.
All 178 edge-fixture finding keys remain equal.
The full workload retains 5,725 files, 19,007,630 core nodes, zero findings, 81 rule executions, and 785,573 VM instructions.
Projection sizes remain demanded for only 35 documents, depths for 25, and token spans for 34.
The full run is profiled validation, not a paired performance comparison.
Merge evidence and the source hashes are under `benchmark/async-family-integration/merge-b4f0aa04b/`.

## 2026-11-10 — Fact admission and reuse

Merged integration tip `013068e01` by fast-forward. Integration already contains this branch's previous changes.
Sibling working changes remain untouched.

The baseline reads frame metadata once per relation. Consumers then load individual frame payloads during execution.
Both paths call `settle`, which can wait for queued writes.
New tests count actual SQL executions and read queued empty facts while a test blocks the writer.

The intended boundary is one admission per known relation demand batch, before its consumers run.
Admission keeps compressed payloads. Consumers decode only the relations they read, without more SQL.
Queued facts use the existing writer's pending values, including deletion markers. No separate transformation cache is added.
The tests must preserve space, relation identity, partition, empty answers, and restarted-process reads.

The first baseline test build failed because a duplicate comparison helper called removed matcher APIs.
The query reference now uses the integration's existing native oracle helper.
The shell's Homebrew compiler also failed to load LLVM. Builds use the installed Rust 1.97.1 toolchain directly.
Performance measurements and test outcomes follow below when available.

The frame tests failed on the baseline: 257 relations caused 258 statements, and a queued empty relation waited for the blocked writer.
All six frame tests now pass. Metadata uses at most four statements for those 257 relations.
The unchanged-output reuse test passes on integration `013068e01`, after the comparison helper correction. No reuse-test workaround was necessary.

Payload admission now uses a conservative dependency pass. It requests outputs and retained inputs of potentially changed consumers.
Unchanged intermediate relations need metadata only. A changed frame between metadata and payload admission invalidates the cut and forces evaluation.

Imported rr-align commit `d589e41fa` as a focused change, preserving integration's newer cache semantics and shared executor.
Its native-answer validation replays batched external requests before reusing their derived relations.
That protects against changed directories and loaded files. It does not yet satisfy zero native-answer calls on an unchanged run.

The five full-workload baseline runs pass, each with 5,725 files and zero findings.
Cold CPU is 21.899 seconds. Warm CPU is 9.987 seconds, and warm CPU after a comment edit is 9.914 seconds.
The async job uses a composed query and does not exercise retained frames.
Its warm profile attributes nearly all SQL hazard time to 23 per-file memo batch reads.

EXPLAIN shows that SQLite drives each memo join from all rows of the plan, then scans the JSON demands.
On the real store, 256 demands execute about 7,418,800 VM instructions. A demand-first join executes about 12,000 and returns identical rows.
The new production regression test fails at 2,648,717 VM instructions for 128 demands against 2,048 stored rows.
The fix uses CROSS JOIN to keep demands outside the indexed lookup. The work ceiling is 100,000 VM instructions.
All eleven memo tests pass with the demand-first join. The native retention tests also pass.
An unchanged recursive transformation evaluates zero strata. It admits its final aggregate, without loading the retained closure.
Publishing replacement rows between metadata and payload admission forces recomputation.

The five candidate workload runs pass, with the same 5,725 files and zero findings.
Cold-store CPU falls from 21.899 to 16.502 seconds. Warm CPU falls from 9.987 to 0.523 seconds.
The comment-edit run falls from 10.197 to 0.561 CPU seconds. These are separate processes, including startup and exit.
Core SQL remains at 20 executions and 74,618 VM instructions in both warm profiles.
The storage profiler still records 80 SQL executions, including 23 memo batch reads.
Those reads fall from 27,603 to 138 milliseconds of SQL wall time. The main improvement is storage lookup shape, not frame admission.

Default-mode validation exposes pre-existing composed-query/session failures.
The saved integration binary reproduces the `.cache` session failure: its consumer cannot find the `info` relation.
The composed path decodes outputs without retaining their fact relations for session maintenance.
Other session fixtures use `back`/`BACK` and `leaf`/`LEAF`; SQLite rejects these as duplicate CTE names.
Retained-strata tests expect scheduler reports that the composed path does not produce.
The native-path retention tests pass with the existing test-only native selector. Default failures remain recorded, not suppressed.

The architecture checks also found test fixtures mistaken for production tables and an unclassified profiling counter.
Moved the fixtures to a test-only file. Classified `SIZES` as reporting state; the legacy-cache allowances remain unchanged.
The remaining integration issues and measured limits are recorded in `fact-admission-2026-11-10.md`.


The final review also measured the new frame query's work, beyond statement counts.
SQLite chose the same bad join order: 128 demands against 8,192 frames took about 5.31 million VM instructions.
One SQL statement was not enough. Added a production VM-budget test and changed this join to demand-first too.
The isolated query falls to about 2,500 VM instructions. The production test fails at 5,311,431 before the fix.
All seven frame tests pass after the fix, including the 100,000-instruction work ceiling.
This correction affects retained frames, not the previously measured async job, which uses composed execution.


The final engine build passes. Both architecture guards and all four relocated native-demand tests pass.
The independent full-corpus native/reference harness passes parity and its timing gates.
Its generated-compiler qualification remains false; this is supporting reference evidence, not a production cutover claim.
The default-mode session failures remain open and are included in the evidence.


## 2026-11-10 — Composed session retention and bulk publication

Merged integration tip `5ea1d2f75` as `d65a24d47`.
Resolved the oracle helper conflict with the shared helper and integration's feature gate.
Removed duplicate dependency declarations introduced by the merge.
Integration now selects composed execution with `RERAIL_COMPOSE=1`.
Validation explicitly sets that selector. The native default cannot prove that composed retention works.

The repair retains typed output rows in ordinary resident relations before decoding them.
The compiler assigns distinct physical CTE names when logical relation names differ only by case.
The rewrite traverses typed queries, including aggregates and correlated subqueries. It does not replace SQL text.

Frame publication will use bounded insert, delete, and usage-update batches in the existing transaction.
The new regression publishes, deletes, and touches 257 keys in a space of 8,192 frames.
It checks both statement count and VM work, plus repeated keys and nullary replacement.


The merged baseline fails all three targeted composed checks: session cache output, case-distinct relations, and durable reuse reporting.
The publication regression fails at 782 statements before batching.
The new writer executes 18 statements and 74,776 VM instructions. All eight frame tests pass.
Its work ceiling is now 100,000 VM instructions.
The trace resets VM counters at statement start. Without that reset, cached statements include untraced fixture setup.

Composed durable retention now records requested outputs and their transitive input digests.
An unchanged output can supply another program's reads. Internal CTEs stay inside the query.
Outputs with unversioned native or tree dependencies run again; their explicit demands alone do not prove unchanged document state.

### Session repair: preparation expansion and restored statistics

The first repaired build passes all ten direct composition tests, both cache-version tests,
and durable reuse. Six of the thirteen explicit composed session checks still fail.
Five reach SQLite's 65,535-reference limit; one restored shared relation lacks `uses.c0` statistics.
The native baseline passes all thirteen checks.

Captured the failing negation-maintenance SQL from `sqlite3_prepare_v3` under LLDB.
It has 44,239 bytes and 38 CTEs. Recursive substitution of its FROM/JOIN dependencies
counts 5,850,486 references across the definitions. This is preparation expansion, not
an executed-row measurement. `MATERIALIZED` does not bound that preparation work.

Added typed dependency counts and bounded shared publication stages. Each stage exports
only relations used after the cut, together in one query. Ordinary plans remain one query.
The compiler rejects plans beyond its stage budget instead of silently scheduling each relation.
Before planning, empty positive joins are removed; negation and aggregates retain their semantics.
A proven quiet maintenance relation shares its old resident fact and indexes.
Restored and staged facts supply their existing statistics to the planner.
Profile output now records generated SQL before preparation, so a failed preparation leaves evidence.
Validation of this second repair is pending.

### Quiet relations and a separate native budget failure

Bounded CTE stages and restored statistics repair twelve of thirteen session checks.
The remaining check has correct findings but reports zero quiet strata.
The SQL needs the net change relations before it can prove that a kept relation is unchanged.
The repair requests those relations together, then shares old resident facts when both changes are empty.
A direct regression requires the same resident object, so publication cannot discard its indexes by copying rows.
The stage budget is shared by change admission and result evaluation.
Compilation now moves the planner statistics through stages instead of cloning the input statistics at each boundary.

The native whole-job comparison exposed a distinct integration limitation: the cold 5,725-file run
executes 396 statements (373 in the local phase) and fails the existing ceiling of 100.
The result keys remain correct. Warm and edited runs execute 30–32 statements.
This failure is preserved separately; neither the engine default nor the ceiling changes.
The comparison runner can finish recording after a budget failure, but still exits with failure.

### Whole-job gate catches an over-eager stage cut

All thirteen existing session/cache tests now pass in both modes. The positive four-rule fixture
and the three cache architecture checks also pass. The new direct tests initially selected the
legacy SQL-table input mode; corrected their inputs to the production atom-backed configuration.

The 5,725-file composed cold run then fails the unchanged job ceiling: 154 executions.
Its trace shows 73 chunks with two queries each, plus eight setup/cross statements.
The initial 4,096-reference planning threshold split a graph that fits SQLite safely.
Reconstructing dependencies from the profiled queries gives about 5,600 expanded references.
Raised the internal threshold to 8,192, still well below SQLite's 65,535-reference limit.
This restores fusion for the async workload while keeping boundaries for the pathological
maintenance graphs. The job ceiling remains 100. Final validation is pending this rebuild.

### Preparation decides whether a risky graph needs a cut

The 8,192-reference threshold still gives 154 executions. The earlier reconstruction from
separate stage queries is not a reliable measurement of the original graph: generated demand
helper names belong to each statement's scope. Reconstructing across those scopes loses identity.

Ran the earlier uncut compiler on the same 5,725-file workload with profiling.
It passes the unchanged ceiling at 81 executions. Its local query is 23,416 bytes,
runs 73 times, and accounts for 719,167 VM instructions. SQLite can prepare this graph.

The physical planner now uses the reference estimate to identify risky graphs, then asks SQLite
to prepare the complete query. A successful preparation keeps it fused. A structural-limit error
selects shared stages. That result is cached with the compiled plan, so later chunks do not repeat
a failed preparation. Other SQL errors are returned unchanged. Preparation has its own profile span
and does not count as execution. The external ceiling remains 100.
The dense-DAG regression now starts from a bound input and requires SQLite to reject the full query
before the staged form passes its execution and result gates.

### Final qualification and publication

The preparation-based decision restores fusion without changing the job ceiling.
All thirteen existing composed session/cache checks pass, as do all thirteen direct compiler tests.
The same existing session suite passed in native mode. Eight storage tests, three cache architecture
checks, and the 120-finding positive fixture also pass.

On the full 5,725-file workload, composed execution uses 81 cold statements, 19 warm statements,
and 10 after the comment edit. All five conditions pass the 100-execution gate.
Whole-process CPU is 18.514 s cold, 0.652 s warm, and 0.662 s after the edit.
The same final binary's native arm uses 18.333 s, 0.718 s, and 0.663 s respectively.
These single observations do not establish a timing improvement.
The native cold arm still fails its inherited budget at 396 executions; warm and edited runs pass.

Frame publication changes 782 statements to 18, with 74,776 measured VM instructions.
The regression ceilings remain 20 statements and 100,000 VM instructions.
Both requested defects are resolved. The separate native cold-run budget failure remains documented.
The final build disables incremental compilation after a stale-object linker failure.
Only inactive engine incremental caches in this task's target directory were removed.
Reproduction artifacts and the final report are under `benchmark/async-family-integration/session-publication`
and `docs/comparisons/composed-session-publication-2026-11-10.md`.


## 2026-11-10 — Native cold execution budget

Merged integration `1e1ec60a3` as `40443adfd`. Its optional index experiments remain off.
Reproduced the existing 100-execution failure with the previous verified binary (`c05e28a75` source):
396 core executions across all 5,725 files, including 373 local executions.
The profile is in `native-budget-1e1ec60a3/baseline-profile` under the comparison artifact directory.
This baseline predates the merge; validation of the fix uses the merged sources.

Across 73 chunks, the profile records 73 demand-view scans, 73 target-table creations,
73 target reads, 73 target-table drops, and 73 temporary-schema scans.
The last two categories include final cleanup in the cross phase.
These round trips have no useful relational work to perform: their inputs are native facts.

Source inspection found a lifetime defect. Scheduling includes native operator dependencies,
but first-read and last-read analysis includes only rule-body atoms.
A native producer can therefore lose its derived targets before consumption.
Recreating an empty target table loses the requested API set.
Inspection of import screening shows that known matching APIs can then be discarded,
while opaque or unresolved calls can still pass. The initial broadening-only diagnosis was incorrect.
The empty finding set in the full corpus cannot detect this defect alone.

The implementation uses the existing complete dependency analysis for relation lifetimes.
Native demand projections remain native relations instead of becoming SQL views.
Python producers read both demand keys and target keys from resident facts, with SQL fallback
for SQL-owned inputs. Target symbols decode through the shared atom arena.
This uses the existing per-run demand snapshot, without another cache or execution mode.

Regression coverage includes metadata-only dependencies, view-input lifetimes, derived target
filters, repeated runs, independent target sets, and the absence of demand/target SQL publication.
The existing whole-job ceiling remains 100. Build and final measurements are pending.


### Native budget qualification

The merged-source build and all selected checks pass: five demand/lifetime tests,
the 120-finding native positive fixture, and all thirteen native session/cache checks.
The previous binary also passes the positive fixture. It cannot expose the target-lifetime defect alone.
The new metadata dependency regression checks that defect directly.

All five full-corpus conditions pass the unchanged ceiling: 20 cold statements, 32 warm,
32 warm with profiling, 22 after an edit, and 32 after warming the edit.
Cold local execution now uses zero SQL statements. Seven statistics statements and thirteen cross-file
statements account for the remaining twenty. Whole-process cold CPU is 19.401 s; wall time is 25.821 s.
These single observations under shared load do not establish a timing improvement.

The separate cold profile records 74,705 SQL VM instructions, versus 116,245 before.
It confirms resident reads of all 47 target keys and no demand-view or target-table SQL.
Both profiles screen 65,292 calls and emit ten conservative candidates.
The fixed profile has 72 worker chunks, versus 73 before; the work and file set are unchanged.
The remaining document-range statistics insert accounts for 74,433 VM instructions.
The profiled local CPU is 0.704 s, versus 0.645 s before. No CPU speedup is claimed.

Results, reproduction commands, and limits are recorded in
`docs/comparisons/native-cold-budget-2026-11-10.md`.
Artifacts include the before/after profiles, workload runs, validation logs, and source/binary digests
under `native-budget-1e1ec60a3` in the comparison artifact directory.
