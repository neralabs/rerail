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
