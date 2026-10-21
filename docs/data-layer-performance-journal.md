# Data-layer performance journal

This journal records measurements, explanations, and implementation decisions for the data-layer ports.
Each entry identifies its implementation and evidence. Historical results remain intact after a new baseline.
Experiments stay in isolated worktrees or standalone harnesses until a keep-or-reject decision.
The engine retains one chosen execution path. Profiling adds observation and does not select another algorithm.

## Field-aware anchors and shared input indexes: 2026-10-21, after `a92c126a`

**Keep selective anchors and share one index across each file's relational inputs.**
The [evidence](../fixtures/host/data-layer-port/profiling/anchor-execution-20261021.json) records the controls, rejected trial, navigation counters, external samples, and tests.

The implementation uses the existing kind and value postings, plus sparse postings for requested AST fields.
Each field posting identifies the first child in that field.
Stored parent links support reverse navigation from those children.
Utility plans retain their relation constraints.
File-local fact extraction now uses these selections before the matcher constructs captures and projected rows.
Kind-only queries continue to use kind postings directly.

The tape records first-field occupancy during restoration.
This marker occupies existing padding: each navigation record remains 40 bytes.
Ancestor checks read the known child edge directly and preserve first-child behavior for repeated fields.
Nested relations, stop rules, negation, damaged trees, and capture order retain their existing semantics.

These candidate indexes remain in Rust because the existing AST matcher consumes the tape before SQL input construction.
SQL continues to use indexed syntax virtual tables for kind, parent, identity, ownership, and byte ranges.
Their existing field-access index remains available to SQL projections.
This change adds no execution backend or configuration switch.
Moving AST candidate selection into SQL requires a separate integration with that syntax source.

### Matched controls

Both builds use engine optimization level 1, dependency level 3, no cross-crate LTO, 256 codegen units, and the system-malloc shim.
The baseline build comes from a clean `a92c126a` worktree.
The source pin, source hashes, rule definitions, ingestion size, and expected findings remain identical.
These builds differ from the earlier thin-LTO reference.

| Workload | Before CPU | After CPU | Before peak RSS | After peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Ten rules, 12 files, tracing off | 38.155 s | 32.436 s | 2.564 GiB | 2.474 GiB |
| Quality rule, 64 files, tracing on | 9.184 s | 6.947 s | 656.69 MiB | 639.17 MiB |

The positive workload retains all 61 expected source findings with no engine failures.
The quality workload retains zero findings and no failures.
The CPU reductions are 15.0% and 24.4%, respectively.
The table uses the final rebuilt binary.
An earlier run of the retained algorithm measured 33.464 seconds and 7.735 seconds, respectively.
The evidence preserves both runs and their binary identities.
The baseline has one control run on the shared host.
The positive workload's elapsed times range from 48.831 to 65.662 seconds, against the 51.631-second baseline.
These comparisons use process CPU and do not establish an elapsed-time improvement.
No new full-repository or production-build result is claimed.

### Navigation and index construction

| Quality probe counter | Before | After |
| --- | ---: | ---: |
| Field lookups | 16,140,707 | 614,645 |
| Child examinations | 427,807,885 | 1,403,498 |
| Child examinations in failed lookups | 423,856,845 | 808,916 |
| Direct checks of known field edges | 0 | 2,050,091 |

Child examinations decrease by 99.67%.
The first implementation rebuilt the structural index for each input.
It performed 695 builds over 31,441,607 nodes during positive-workload preparation.
One state-flow clause then exhausted its work budget, despite the correct finding count.
That run is rejected as a passing benchmark.

The final implementation builds 76 indexes over 3,466,450 nodes for the same input partitions.
Each batch shares kind, value, and field postings across its input plans.
Selections retain compact ordinal arrays after the shared index is released.
The selection cache uses the compiled plan and current source identity.
A source change clears the cache, and the evaluation scope releases it.
The original work limits remain unchanged.
A regression test requires two distinct inputs to share exactly one index and preserve separate selections.

### Remaining costs and validation

The final diagnostic retains 80 queued jobs, 577,199 projected rows, and 9,833,801 JSON fields.
Its referenced row-storage estimate remains 1,256,398,807 bytes before SQL admission.
The representation problem from the previous entry remains.

External samples show allocation and matcher overhead during preparation.
Two later sampling windows contain 81.4% and 99.7% SQLite samples.
Their leaf functions include record comparisons, B-tree lookups, memory copies, and sorting.
Those percentages describe sampling windows, not total execution CPU.
The diagnostic attributes 22.052 seconds of CPU to the SQL batch and 5.785 seconds to input preparation.
The largest descendant materializations and their indexes remain substantial costs.

Validation covers 69 focused Rust tests, 320 public native tests, and 17 profiling tests.
Two existing public tests remain skipped by their existing gates.
The source-corpus sample covers one positive and one clean observation for each of the ten rules.
All 40 stateless and workspace evaluations pass without differences.
TypeScript, repository checks, formatting, and Clippy run separately from the timed controls.
Every command retains a deadline shorter than two minutes.

## CPU and memory attribution: 2026-10-21, after `4def200d`

**The remaining problem is large fact construction, repeated AST navigation, and large SQL intermediates.**
The earlier 1,114.637-second reference also contains substantial profiling work.
Removing that measurement flaw does not remove the engine's preparation costs.
The [evidence](../fixtures/host/data-layer-port/profiling/cpu-memory-attribution-20261021.json) records controls, ownership counts, samples, and validation.
The [profiling guide](code-rule-profiling.md#measure-cpu-and-memory) describes the new tools.

### Matched Python comparison

Both engines evaluate the same ten sixth-batch rules on 12 files containing 1,984,216 source bytes.
The source pin remains `2b1504adc4291daaf080b2e932962b134da1a69a`.
Both produce the same 61 source findings, including multiplicity.
Thirty findings report required files absent from this restricted snapshot; 31 are detections from the repository recording.

| Control | Analysis CPU | Peak process RSS |
| --- | ---: | ---: |
| Upstream shared Python fact engine, one worker | **4.320 s** | **196.70 MiB** |
| Rerail, automatic residency scans removed | **37.237 s** | **1.681 GiB** |
| Rerail / Python | **8.62×** | **8.75×** |

Native CPU excludes 0.714 seconds of policy compilation.
Python CPU includes `run_plugins`, waited-for child CPU, and the source comparison adapter.
Peak RSS includes process setup for both engines.
The native reference uses engine optimization level 1, dependency level 3, thin LTO, 256 codegen units, and the system-malloc shim.
It is not a production optimization-level-3 benchmark.
Tracing, SQL capture, external sampling, and explicit censuses are off for both timing controls.
The profiling feature remains compiled into the native reference, with aggregate CPU accounting enabled.

The native driver attributes 7.558 seconds to ingestion and 29.675 seconds to global preparation and SQL.
These are disjoint process CPU measurements from the completed control.
The host is heavily shared; CPU comparisons are more useful than elapsed-time comparisons here.
All new commands remain bounded below two minutes.
Larger captures and a later twelve-file repetition hit their deadline and remain partial evidence, not completed benchmarks.

The initial subset incorrectly filters expectations from the full repository.
That loses the 30 required-file findings and marks correct detections as extra findings.
The corrected upstream snapshot oracle retains those findings.
The cross-runtime tool verifies file hashes and reconstructs the timed run's complete finding multiset from its original expectations and differences.
The resulting comparison passes; the original timing report remains unchanged.
Generated-file freshness remains a separately recorded upstream integration contract, as established in the sixth-batch port.
The restricted state oracle uses unchanged upstream source conversion and signed allowlist functions.
It preserves the freshness reporter observation instead of counting it as a native source finding.

### The queue exists before memory admission

A diagnostic capture reaches SQL admission with 80 jobs and 577,199 projected rows already constructed.
Those rows contain 9,833,801 JSON object fields.
Their referenced heap estimate is **1,256,398,807 bytes**, excluding supplied syntax, snapshots, and allocator overhead.
This estimate is not an exclusive RSS measurement.
The native timing control peaks at 1,805,041,664 RSS bytes.

| Consumer | Queued rows |
| --- | ---: |
| State flow | 196,089 |
| Legacy execution graphs | 162,195 |
| State signatures | 118,589 |
| Remaining consumers | 100,326 |

The first three consumers account for 82.6 percent of rows.
Projected field names alone contain 103,706,252 bytes; projected string values contain another 82,247,225 bytes.
Maps, row metadata, capture spans, paths, and allocation capacity dominate the representation.
All 577,199 projection references identify different row objects within the queue.
Shared SQL stages and the packed producer arena do not eliminate these upstream objects.

The 256 MiB SQL admission target starts after this queue exists.
It therefore cannot bound this preparation peak.
The 64 MiB document cache also cannot release syntax objects retained by queued jobs.
On the separate 64-file quality capture, the queue references 57 syntax objects with a 247,446,912-byte cache estimate.
That estimate can overlap document storage and uses the existing syntax-cache heuristic.
It is not another independent RSS category.

### Field navigation performs hundreds of millions of scans

The 64-file quality capture records:

- 16,140,707 field lookups.
- 427,807,885 child examinations.
- 423,856,845 examinations inside failed lookups: **99.08 percent**.
- A maximum scan of 400 children for one lookup.

`TapeNode::child_by_field_id` searches the sibling list for every request.
Many requests name fields absent from that node.
Two external ingestion windows attribute 43.0 and 63.7 percent of self samples to this function.
These percentages describe sample windows, not the entire run.
The broader source capture also shows repeated anchor selection, sorting, projection, and allocation work during input construction.
Python's fact engine shares one AST traversal across collectors and accesses AST fields directly.

The next navigation change needs an indexed field lookup or a grammar-aware rejection before scanning children.
It must preserve first-child order, repeated fields, aliases, anonymous nodes, and existing tape parity.
The ownership postings fixed earlier are a separate interface; these measurements concern the AST matcher adapter.

### Profiling was adding quadratic work

The previous profiling feature serializes all resident witness payloads after every ingestion request, even with tracing disabled.
The large reference has 288 ingestion revisions.
Its retained-result estimates sum to approximately 316 GB across those repeated scans.
That sum is neither resident memory nor an exact serialized-byte count.
An earlier external sample places 74 of 96 active native samples inside this accounting routine.
The old 1,114.637-second result consequently cannot establish clean production execution time.

Automatic residency scans are removed.
An explicit census now counts shared projections once and reports its own CPU cost.
The normal apply path no longer performs a complete residency traversal for profiling.
On the small 64-file control, cold CPU decreases from 7.630 to 7.245 seconds.
This single shared-host comparison establishes neither a repository speedup nor the full profiling overhead.

### SQL remains a substantial second cost

The completed twelve-file control spends 29.675 seconds in global preparation and SQL together.
The separate diagnostic records 7.797 seconds of queue preparation, including its ownership census.
These measurements use different observation modes and must not be subtracted to estimate SQL CPU.
Its largest SQL stages include state and legacy descendant tables and their indexes.
One state-signature descendant materialization alone executes 23,860,081 VM instructions in an observed partition.
The detailed evidence ranks the measured stages and preserves their rule, clause, and file context.

The small forest kernel optimized earlier does not account for these broad input tables or repeated descendant materializations.
Optimizing that kernel again would leave the main allocation and preparation costs intact.

### Required implementation order

1. Queue versioned descriptors before construction; admit capture and syntax leases within the same memory boundary as SQL consumers.
2. Build shared capture columns per file version, with row-selection views and a union of demanded columns across consumers.
3. Keep source text as bounded references until SQL requests it; avoid repeated field-name strings and unused capture-span maps.
4. Index AST field access and reject impossible fields before sibling scans.
5. Reduce demanded descendant roots and repeated SQL intermediates after the input representation changes.

Admission must retain broad producer waves and the existing shared pass allowance.
SQL continues to own rule semantics.
These requirements do not justify a rule-specific Rust evaluator or another optional execution backend.
No Linear tickets are created.

### Validation

The focused native suites pass 67 distinct checks, including shared projection accounting through edits, removal, and workspace closure.
The mandatory producer-shape gate passes all five checks.
Twenty recorded source observations cover one positive and one clean case for each linter.
All twenty match on both stateless and workspace execution, with zero engine failures or finding differences.
The complete profiling checks, typecheck, repository checks, formatting, and production and profiling Clippy checks pass.

Two bounded attempts to build the full Rust unit binary stop before test execution.
The memory invariant instead has a public binding regression, which verifies actual evidence sharing and invalidation.
No Rust unit execution pass or new full-repository benchmark is claimed.

## Batch six diagnosis: 2026-10-21, after `71ea1606`

**All 60 rules now pass together over 9,021 files: 24,993 findings and zero engine failures.**
Cold, unchanged-file, and trailing-newline edit comparisons have zero missing or extra findings.
The [diagnosis evidence](../fixtures/host/data-layer-port/profiling/batch-six-root-cause-20261021.json) contains reports, source hashes, test receipts, SQL counters, and external samples.

Both combined runs use the same 137,924,324-byte pinned snapshot and reference harness.
CPU includes process user and system time, including the profiling build's residency accounting.
Peak RSS covers the complete process, including incremental checks.

| Combined 60-rule run | Cold CPU | Cold elapsed | Unchanged CPU | Edit CPU | Peak RSS | Result |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| After capacity admission, before remaining SQL fixes | 1,076.563 s | 1,054.093 s | 2.102 s | 53.083 s | 28.12 GiB | Fail: conditional budget exhaustion |
| Final | **1,114.637 s** | 1,277.989 s | 2.109 s | 63.196 s | **18.30 GiB** | **Pass: 24,993 findings** |

Final compilation uses 2.163 s CPU, separately from cold evaluation.
The final run completes analysis that the preceding run interrupts. These results do not establish a passing CPU speedup.
The observed memory peak decreases, but full input preparation remains outside admission and still needs architectural work.
The historical 951-second passing run covers only ten rules and records no CPU time.

The preceding measurements are committed and pushed in `71ea1606`.
This investigation identifies two independent causes of the 890 repository failures.
The fixes retain SQLite evaluation, existing limits, and one production execution path.
The [execution contract](gpu-producer-execution-contract.md) describes capacity admission and its memory boundaries.

### Shared producer capacity and eager SQL preparation

The previous driver creates every SQL session before it advances any consumer.
Each session retains its input tables and intermediate state while the driver prepares its peers.
That SQLite storage is outside the producer's 64 MiB packed-input arena.
Once aggregate producer input exceeds capacity, preparation rejects every ready relation.
This explains the 889 identical `quality-contracts-1` failures.
It also permits unbounded suspended SQL storage, but the preceding 21.33 GiB process peak does not isolate that storage from prepared inputs.

The driver now prepares consumers as capacity permits.
Admission counts shared packed columns once and checks SQLite cache, schema, and statement storage through its 64-bit status API.
SQLite admission measures storage at the first producer suspension; later SQL stages can grow it.
Ready consumers share broad producer waves within each capacity partition.
Finished consumers close before admission continues.
The driver preserves original result identities and keeps one 100-pass allowance across all capacity partitions.
It does not reset work limits when it releases physical storage.

The SQLite admission target is 256 MiB, with one prepared candidate retained while the preceding partition drains.
A single consumer can exceed that target under its existing work and output limits.
Projected evidence, prepared syntax facts, publication indexes, and document caches are outside this target.
It therefore does not bound process RSS.
The producer retains its 64 MiB input and 256 MiB numeric-output limits.
An individually oversized relation fails without rejecting fitting peers.

Four focused Rust tests cover capacity partitioning, SQL admission, peer isolation, and exhaustion of the shared stage allowance.
A separate layout test checks shared-column accounting and transactional admission.
The mandatory producer-shape gate preserves stage counts as independent roots, files, and policies increase.

### A file gate cannot preserve selected AST rows

The conditional rule needs declarations across the repository, plus selected nodes within particular bodies.
Its authored AST input selects individual rows and preserves selected ancestry.
The native syntax lowering replaces that input with a file gate and whole-file syntax.
Since the selection includes module declarations, every file qualifies.
The lowering therefore expands a selected relation into the entire repository's syntax and source demand.
The 32 MB repository-source guard then correctly rejects the 137 MB snapshot.

The initial experiment moves byte accounting after file selection.
A full quality-only run still fails: the file gate admits every repository file.
That experiment is removed. Source-budget behavior and limits remain unchanged.
The actual fix restricts native syntax lowering to file-partitioned contract clauses.
The global conditional clause uses its existing projected AST inputs and ordinary SQLite stages.
Its 20 typed input partitions share one AST selection utility instead of repeating the selection definition.
A generator regression test ensures global node selection cannot become a whole-file gate.
No new engine feature or Linear ticket is required for this correction.

All 1,259 sixth-batch recorded comparisons pass, including 417 positive and 842 clean observations.
These comparisons check both stateless and workspace execution.
The quality rule's 200 observations include 135 positives and pass separately.
The 12 focused Rust relational tests and native relational and conformance suites pass.

### Measurements and remaining preparation costs

The first combined 60-rule rerun matches all 24,993 findings but still fails parity.
Its only failures are interruption and work-budget exhaustion in the conditional ownership stage.
All 889 producer-capacity failures disappear, and the conditional clause passes the preceding source-budget guard.
Cold CPU is 1,076.563 s, elapsed time is 1,054.093 s, and peak RSS is 28.12 GiB.
Unchanged CPU is 2.102 s. Edit CPU is 53.083 s; both retain the conditional failures.
This is another failed measurement, not a passing speedup.

The SQL capture then identifies wide window rows and graph normalization as additional costs.
The general column-nulling pass increases the five-file conditional capture from 9,574,525 to 10,680,292 VM instructions.
That experiment and its unused helper extension are removed.
The selected ordinal window instead sorts narrow keys before joining projected node values.
The SQL test preserves every node value and requires fewer VM instructions on wide input.
Declaration selection also uses materialized key sets before joining class and function values.
Tests preserve class ownership and the latest declaration when names repeat.
Together, these changes use 8,723,163 VM instructions on the same five-file conditional capture, about nine percent fewer.
Instrumented CPU does not improve on that small selection. These counters do not establish a CPU speedup.

A focused repository capture then completes ownership but exhausts work during descendant normalization.
It uses 784,639,688 measured SQLite VM instructions before failure.
The normalizer prepares graph keys for every selected file, although only files with requested roots can contribute descendant rows.
The authored conditional plan now materializes its roots once and limits its edge query to those file domains.
CST edges cannot cross files. The test preserves closure results and isolated roots while excluding an unrelated file's edges.
Engine forest validation and graph semantics remain unchanged; the authored edge relation carries the narrower domain.
One covering forward edge index replaces two overlapping indexes.
The unused outermost-function ownership index is removed.

Those changes still exhaust the allowance during graph-key compaction on the full conditional snapshot.
The capture records 67,410,860 VM instructions for the key sort and 18,962,193 for two key indexes.
Compaction then consumes another 12,967,702 instructions before interruption.
The key table stores a dense number separately from SQLite's own row identity.
The engine now declares that number as `INTEGER PRIMARY KEY` and lets SQLite assign it during the existing deduplicated insertion.
This removes the numbering window and the redundant numeric index.
The unchanged scalar-key union preserves SQLite equality and its selected key values.
The fresh table receives one insertion per unique key, so its identities remain dense for the existing forest adapter.
Its scalar-key index also carries the numeric identity through SQLite's row index.
A small derived-forest test reproduces the preceding budget failure without a repository run and compares completed output with recursive SQL.
The preceding engine fails that test at 25,000 relational work units. The new engine passes all 28 reachability tests.
A separate SQL probe extracts the actual normalization statements from both engine versions.
It preserves decoded scalar types and values, including text, numbers, and blobs, and retains dense identities.
For 5,008 keys, normalization decreases from approximately 921,600 to 699,000 VM instructions.
This isolated probe uses Python's SQLite and does not establish a native workload CPU gain.

File-domain pruning alone still includes unrelated components within requested files.
The selected conditional plan therefore prepares the union of requested subtrees through one unlabelled SQL frontier.
Overlapping roots share that preparation. The existing producer then computes the separate closure for each root.
The regression test excludes an unrelated component within the same file while preserving all oracle closure rows and isolated roots.
This prepares demanded graph facts instead of normalizing every declaration's graph identity.

The corrected conditional clause passes over all 9,021 files, with zero findings expected and zero engine failures.
Its graph catalog contains 88,625 keys, compared with 824,441 in the preceding file-domain attempt.
The key insertion falls from 67,410,860 to 4,596,429 VM instructions.
The complete capture records 763,559,808 VM instructions, fewer than the preceding failed partial capture's 784,639,417.
Cold CPU is 199.837 s, elapsed time is 208.689 s, and peak RSS is 7.24 GiB.
This is a traced, conditional-only diagnostic with SQL capture and larger ingestion batches: 2,048 files or 16 MiB.
It is not the standard combined reference run, which retains 32-file or 1 MiB ingestion batches.
The full sixth-batch corpus also passes again on the new engine: 1,259 observations, including 417 positives and 842 clean observations.

The intermediate quality-only run eliminates all 889 producer-capacity failures.
It uses 140.094 s cold process CPU, 215.249 s elapsed time, and 2.79 GiB peak RSS over 9,010 files.
It still has the conditional source-budget failure and is not a passing performance result.
Its traced first partition contains 439 files and 950,054 roots in three producer passes.
Two capacity partitions suffice. The admission observations reach 268,454,464 SQLite bytes across 450 suspended sessions.
This counter measures admission storage, not the peak across subsequent SQL stages.
This quality-only measurement cannot establish a memory ratio against the preceding ten-rule run.

The corrected five-file quality probe passes at 3.888 s cold CPU and 1.068 s edit CPU, with 515 MiB peak RSS.
The earlier median is 1.940 s cold CPU and 0.710 s edit CPU for the same source selection.
The policy changes to preserve selected global evidence. This is a correctness cost, not an optimization gain.
Efficient native projection must preserve selected rows, ancestry, and column demand before it can replace that evidence path.

The full repository run also exposes profiling overhead during ingestion.
After every ingestion batch, `record_workspace_residency` walks all retained witnesses and serializes their projections to estimate bytes.
Repeated batches therefore rescan an increasing corpus.
A one-second external sample contains 96 active native samples; 74 include this accounting function.
This is a local ingestion observation, not a whole-run CPU percentage.
The projection captures retain spans and shared JSON projections; they do not retain complete source text for every capture variable.
Selected evidence still has substantial JSON storage and allocation overhead outside SQL admission.
Incremental residency accounting and compact demanded evidence columns remain preparation frontiers.

The workspace also prepares every dirty file-local job before it calls the SQL scheduler.
Its file-partitioned evidence index deliberately keeps eligibility witnesses, with projection and enumeration disabled.
Those gates are not complete cached input relations.
The consumer restores shared tapes, extracts the full AST inputs through `relational_rows`, and retains those rows alongside prepared facts in queued jobs.
Reusing the eligibility witnesses as complete input would lose findings.
Global projected-only producers also construct capture span maps before projection and then discard those maps through `retain_projection`.
Their next extraction path must preserve requested anchors and projected values while skipping unused capture metadata.
The new admission boundary is downstream of that preparation.
A second one-second sample contains 89 active native samples; 87 include `relational_rows` and `stateless_rows`.
Thirty-eight include witness construction, and 27 include evidence projection. These inclusive counts overlap.
This identifies full input extraction and queued input storage as another preparation frontier, separate from suspended SQL storage.
The next admission boundary must precede full projection, with queued descriptors holding document versions rather than complete input row vectors.
That change must preserve source versions, evidence completeness, cancellation, and broad producer batches.
The combined repository run exceeds 24 GiB despite SQL admission. End-to-end memory use remains unresolved.
A third one-second sample, after preparation, has 90 active native samples, all in SQLite under `Session::advance`.
The three samples distinguish ingestion accounting, repeated extraction, and SQL execution. They do not assign whole-run CPU shares.

The reference addons use engine optimization level 1, thin LTO, 256 codegen units, four workers, and the system-malloc shim.
The intermediate addon has SHA-256 `4c3f5357e27dfc054fa4e61a4139d71effb67430df7577e18890771774096631`.
The final addon has SHA-256 `26960ddfc2c0ecb06dcf7f25d639111f51c2a77d5499d261f1fa96fa39b3c1b9`.
Its build tools use optimization level 3 and 16 codegen units. These settings accelerate parser generation without changing runtime optimization settings.
Timing tracing is disabled, but profiling residency accounting remains enabled in that build.
Historical CPU timings remain unavailable. These results do not establish a CPU speedup over the historical 951-second passing run.

### Final checks and next work

The final addon passes all 1,259 sixth-batch observations, including 417 positives and 842 clean observations.
The global conditional entry contributes 28 observations, including 16 positives.
All 40 edit transitions pass. The 12 Rust relational tests, 28 reachability tests, and 40 native and signature conformance tests pass.
The mandatory producer-shape gate passes all five tests without skipping profiling assertions.
Its dependency test receives the same 110-second allowance as its independent-batch test after a shared-host five-second timeout.
Engine work limits do not change.
Clippy passes with and without profiling. Typecheck, repository checks, formatting, and profiling-script tests pass.

The next work starts before SQL session admission:

1. Queue document-version descriptors and admit full input extraction by capacity, instead of retaining all projected rows before consumers run.
2. Build demanded capture columns for global selected evidence and omit unused capture-span maps while preserving anchors and completeness.
3. Update profiling residency totals incrementally, instead of serializing the complete retained corpus after each ingestion batch.

The completed conditional forest walk takes about 56 ms CPU for 46,469 roots in 182 bounded batches.
The surrounding diagnostic uses about 200 s cold CPU. Faster graph traversal cannot remove the extraction and materialization costs around it.
The native producer gate preserves broad count, scan, and scatter passes.
Global projected AST extraction and its SQL demand frontier remain CPU preparation; these results do not measure whole-workload GPU launches.

## Full repository verification: 2026-10-21, execution `3285b48f`

**The first 50 rules pass at repository scale. Batch six fails at repository scale.**
The [complete evidence](../fixtures/host/data-layer-port/profiling/full-repository-20261021.json) preserves reports, failures, commands, source hashes, binary hashes, and corpus coverage.
This entry changes no engine implementation or limits.

### Repository results

All sources use data-layer commit `2b1504adc4291daaf080b2e932962b134da1a69a`.
The first 50 rules run together over 9,021 eligible files and 137,924,324 source bytes.
Batch six runs separately over its original 9,010-file, 137,896,473-byte snapshot for the historical comparison.
The additional 11 files are Rust grammar inputs.
There is no combined 60-rule repository result.

CPU values include process user and system time. Compilation is separate.

| Repository workload | Cold CPU | Cold elapsed | Unchanged CPU | Edit CPU | Peak RSS | Result |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| First 50 rules, current | **212.619 s** | 233.738 s | 0.801 s | 40.182 s | 6.60 GiB | Pass: 24,820 findings |
| Batch six, historical passing run | Not recorded | **951.004 s** | Not recorded | Not recorded | 11.14 GiB | Pass: 173 findings |
| Batch six, current | **538.452 s** | 645.958 s | 0.087 s | 11.344 s | 21.33 GiB | **Fail: 890 engine failures** |

The first 50 rules have zero extra findings, missing findings, or engine failures.
Their unchanged-file and trailing-newline edit comparisons also pass.
The edited file is `impl/common/l2200_sql_dialect/sql_connection.py`.

Batch six produces the expected 173 findings, but its failures prevent a parity pass.
Its unchanged and edited results also contain failures.
All repository findings belong to the direct-execution check. Matching them does not certify the failing quality check.
The shorter elapsed time therefore does not establish a passing speedup over 951 seconds.

### What the session history establishes

The remembered 1,400-second result is **1,456.276 seconds for batch six's ten rules**.
It has 84 extra findings and 46 engine failures.
The later passing run takes 951.004 seconds for those ten rules.
Neither historical result records process CPU time. Neither covers all 50 rules together.
No historical-to-current CPU speedup ratio is available.

The evidence identifies the session history file and its relevant lines.
It also preserves the historical reports, including the passing 40-rule and individual batch-five measurements.
Adding those independent durations would not reconstruct a joint 50-rule baseline.

### Full source-corpus coverage

All 60 rules pass **3,210 native observations: 1,631 positive and 1,579 clean**.
Each observation compares stateless and workspace execution.
Three additional signed-allowlist host checks pass separately.
Every rule has positive and clean coverage. There are zero diagnostic or execution-path differences and zero engine failures.
The 40 sixth-batch positive/clean edit transitions also pass.
The first-50 Vitest gate passes 25 tests, including deliberately incorrect detectors. Two signature tests also pass.

The evidence combines the complete first-50 gate with the all-batch replay.
It replaces 181 overlapping batch-five observations with that batch's full 312-observation result.
The overlap matches as a multiset of observation identities and outcomes. Both executions use the same native binary and policies.
Retained reporter contracts, synthetic graph cases, original I/O cases, and uncaptured-test metadata are not native passes.
This measurement replays recorded upstream inputs. It does not rerun upstream pytest or regenerate oracle expectations.

### Scale failures and the next frontier

There are **889 `producer capacity exceeded` failures** in `quality-contracts-1`.
The producer packs ready relations into a shared 64 MiB input arena.
It rejects every pending relation when preparation exceeds capacity.
The driver currently creates all dirty jobs and SQL sessions before the producer wave.
Automatic memory partitioning remains unimplemented, as recorded in the [execution contract](gpu-producer-execution-contract.md).
SQL session storage is outside the producer arena's limit.

One additional failure affects `quality-conditional-1`: `repository input budget exceeded`.
Global snapshot construction supplies all files in the rule's scope.
`Session::new` checks their combined source bytes against the 32 MB limit before selecting syntax-gated files.
The small profiling selections do not expose this full-snapshot failure.

A one-second external sample during the failed run contains 796 active native samples.
Of those, 789 are within SQLite stage construction, including B-tree work and memory copying.
This window identifies active materialization work. It does not identify the exact SQL statement or assign whole-run CPU percentages.

The next architectural work must partition execution by resident capacity and constrain preparation to demanded files and columns.
It must preserve repository semantics, completion outcomes, work allowances, and broad producer passes within each capacity partition.
Raising limits or suppressing failures would not establish parity.

### Measurement boundaries and reproduction

Timing runs use the preceding baseline's profiling reference addon: engine optimization level 1, thin LTO, and 256 codegen units.
The historical commands use a release build with thin LTO and 256 codegen units.
This comparison does not isolate optimization changes under identical compiler settings.
The ordinary correctness addon has no profiling feature and disables LTO. It supplies the corpus results, not performance measurements.
Both use the system-malloc shim. The evidence records all binary hashes.

Timing processes run sequentially. Host load is high; one observation is about 64 on 18 logical CPUs.
The failed batch-six timing includes the external one-second sample.
Cold means the first workspace evaluation in a fresh process, with no OS-cache flush.
Initial bounded attempts stop at about 112 seconds and do not enter the completed timings.
The user then authorizes full runs beyond the earlier two-minute limit.

Local complete reports are under `.rerail/profiles/full-repository-20261021/`.
`first-50-complete/parity.json` and `batch-6-complete/parity.json` retain the native driver output.
The first-50 oracle recording unions the pinned 40-rule and batch-five recordings.
Duplicate file paths must contain identical source. Every source is also checked against its pinned Git blob.
The union has no combined Python oracle timing. Its original recording hashes are in the evidence.

Use `conformance/data-layer/suite.ts --recording RECORDING --output RESULT --allocator system-malloc-shim --progress` through `node --import tsx`.
Select the recorded reference addon through `RERAIL_ENGINE_PATH` and preserve its allocator setup.
For fresh oracle recordings, use `scripts/dsl/data_layer_suite_record.py --batches 1,2,3,4,5` or `--batches 6` at the same source pin.
The evidence retains the actual invocation commands and ordered rule sets.

## Current baseline: 2026-10-21, execution `7da151b8`

This entry establishes the comparison point after removal of the experimental execution paths.
The [baseline evidence](../fixtures/host/data-layer-port/profiling/execution-baseline-7da151b8.json) contains all 15 timing runs, diagnostic summaries, and external samples.
The evidence includes policy, source, binary, allocator, and measurement-code hashes.

All times in this table are **process CPU milliseconds**, including user and system time.
Each value is the median of five fresh processes. Compilation is separate from cold evaluation.

| Pinned workload | Files / source bytes | Compile | Cold | Unchanged | Edit |
| --- | ---: | ---: | ---: | ---: | ---: |
| Quality | 5 / 1,026,452 | 88.2 | **1,940.1** | 1.96 | **710.1** |
| State | 7 / 1,123,712 | 298.3 | **12,512.3** | 1.61 | **1,137.8** |
| Dispatch | 1 / 136,810 | 46.8 | **982.7** | 1.43 | **972.5** |

Quality cold CPU ranges from 1,893.6 to 2,011.0 ms.
State ranges from 12,277.8 to 12,719.3 ms. Dispatch ranges from 954.9 to 1,005.1 ms.
Median process peaks are 444 MiB, 1,223 MiB, and 390 MiB, respectively.
These peaks include compilation and workspace storage. They do not identify particular heap allocations.

The quality result agrees with the promotion measurements of 1,967 ms cold and 707 ms per edit.
This entry records a new measurement series, with no implementation change or new claimed speedup.

### Method and limits

The implementation is `7da151b8e55dccbce883f100a9f941a09583b927`.
All source selections use data-layer commit `2b1504adc4291daaf080b2e932962b134da1a69a`.
The harness checks source hashes before evaluation and compares cold and incremental findings.
All 15 timing runs pass. No timing run is discarded.
Every measured command has a 115-second deadline.

The reference addon has SHA-256 `8a1f573e09b7837fabbf07073dbf7064b26ff298a7b104f0394acd2e8cac310f`.
Its source hashes match the committed implementation and promotion evidence.
It uses engine optimization level 1, dependency optimization level 3, thin LTO, and 256 codegen units.
The build includes `code-rule-profile`, with tracing, SQL capture, and external sampling disabled for timing runs.
The allocator is the existing system-malloc shim. The evidence records its library hash.
The runtime uses Node 26.8.1 and four native workers on an Apple M5 Max with 18 logical CPUs.

This is a reference build, not a benchmark of the standard optimization-level-3 production build.
The ordinary correctness build disables LTO and does not enter this timing series.
Cold means the first workspace evaluation in a fresh process. The procedure does not flush OS caches.
The workloads run sequentially, with alternating order between rounds, on a shared host.
Small differences within the observed ranges do not establish an optimization gain.

The three performance selections expect zero findings.
Positive coverage remains separate from timing: the quality policy and addon match the [promotion proof](../fixtures/host/data-layer-port/profiling/sql-shape-decisions.json).
That proof contains 200 upstream cases, including 135 positives, plus 219 production comparisons and 177 edit transitions.
This baseline reuses that proof. It does not claim another corpus replay.

### Diagnostic checkpoint

Three additional captures record SQL statements, plans, bytecode, counters, and native spans.
These captures pass parity and contain no incomplete executions.
They remain outside the timing medians.

| Cold diagnostic span CPU (ms) | Quality | State | Dispatch |
| --- | ---: | ---: | ---: |
| SQL materialization | 1,096 | 5,495 | 675 |
| Ordinary SQL indexes | 251 | 2,410 | 158 |
| Completed numeric publication | 20 | — | — |

These columns describe separate instrumented runs. They do not form percentages of the untraced timing medians.
The evidence retains the five most expensive SQL statements for each workload, including complete SQL and counters.
The local HTML reports provide the complete captures:

- Quality: `.rerail/profiles/rebaseline-7da151b8-20261021/quality/diagnostic/sql/profile.html`
- State: `.rerail/profiles/rebaseline-7da151b8-20261021/state/diagnostic/sql/profile.html`
- Dispatch: `.rerail/profiles/rebaseline-7da151b8-20261021/dispatch/diagnostic/sql/profile.html`

Quality's remaining targets are `scoped_nodes`, `scope_owners`, root collection, and ordinary SQL indexes.
Its capture records about 131 ms in `scoped_nodes` and 124 ms in `scope_owners`.
The main root query costs about 113 ms.

State is now the largest cold workload in this baseline.
Its leading SQL statements construct descendant relations, their indexes, syntax nodes, and ownership.
Those paths still reconstruct facts through the older evidence and SQL representation.
The quality implementation's shared syntax and completed numeric publication do not automatically cover those rules.

Three one-second external samples cover cold intervals in the state workload.
The first window also overlaps the compile boundary by about 14 ms.
The first two windows show matcher selection and allocation. The third shows SQLite execution, comparisons, and memory copies.
These windows expose both preparation and SQL costs. They do not estimate whole-run CPU proportions.
The evidence retains the top self costs and bottom-up caller chains.

### Reproduce this baseline

Use the implementation, source pin, build configuration, and allocator from this entry.
Select the matching addon through `RERAIL_ENGINE_PATH`.
Run each measurement in a fresh process and a new output directory:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/profiling/run.py \
  --trace off --deadline 115 \
  --source /path/to/pinned/nera-data-layer \
  --selection fixtures/host/data-layer-port/profiling/quality-five.json \
  --policies packs/data-layer/candidates/rules \
  --allocator system-malloc-shim \
  --output .rerail/profiles/new-baseline/quality/round-1
```

Use `state-seven.json` or `dispatch-one.json` for the other workloads.
Repeat each workload five times without overlap.
The evidence records the exact order, phase CPU values, ranges, and commands.
For diagnostic captures, add `--sql-profile --trace on --cold-only` and use separate output directories.
For external state samples, use `--external sample --sample-every 2 --sample-count 3 --sample-seconds 1`.
Select the native module through `--sample-module` if the addon has another filename.

## Experiment ledger

### 2026-10-20: shared SQL and reachability

**Decision:** keep SQL as the semantic engine and reuse shared facts before comparing execution backends.
The earlier specialized Rust evaluator did not provide a fair comparison against equivalently shared SQL preparation.
Its results do not establish a general native-engine speedup.

The reachability experiments compare recursive SQL, a Rust scalar function, and a Rust virtual table on the same requests.
The implementation at `d97e1239` keeps batched recursive SQL for general directed graphs.
Recursive UNION retains cycle termination and deduplicates converging paths.
UNION ALL is valid only with a proved unique-path invariant.
Scalar probes do not replace a broad batch interface.

Evidence: [shared SQL profile](shared-sql-performance.md), [reachability experiments](reachability-spikes.md), and [batched SQL implementation](batched-sql-performance.md).
The detailed reports retain the original build and workload boundaries.
Their results do not form a cumulative speedup series with later builds.

### 2026-10-20: generic forest traversal and storage

**Decision:** use native structural primitives where their graph contract is explicit, while SQL retains rule decisions.
The implementation at `f52817fc` adds strict forest traversal, deferred source text, and release after the last consumer.
General directed graphs retain cycle-safe SQL.
The IR-name propagation experiment is slower and does not become a native engine path.

The first budget model charges native visits too heavily and treats derived output as new AST input.
The corrected model retains limits and cancellation without those duplicate charges.
This result requires both positive corpus coverage and the larger pinned workload.

Evidence: [native relation experiments](native-relations-spikes.md), [implementation report](native-relations-implementation.md), and [measurements](../fixtures/host/data-layer-port/profiling/native-relations-implementation.json).

### 2026-10-20: shared syntax preparation

**Decision:** remove repeated preparation before adding more execution backends.
The implementation at `c4bd76bf` shares immutable syntax facts and uses direct parser identities and subtree boundaries.
SQL no longer reconstructs those facts for the quality groups.
This change addresses the expensive preparation that limited earlier native-access experiments.

Evidence: [requirement](shared-syntax-requirement.md), [implementation](shared-syntax-implementation.md), and [controlled measurements](../fixtures/host/data-layer-port/profiling/shared-syntax-metrics.json).

### 2026-10-21: broad producer passes

**Decision:** batch independent structural producers above their SQL consumers.
The implementation at `eec751d0` uses count, scan, and scatter passes for each dependency wave.
Resident packed input avoids repeated uploads or preparation between waves.
Tests keep the number of passes constant as roots, files, and policies increase.

The motivation includes the future GPU execution contract. This is a CPU reference, not a GPU speed measurement.
Batching alone does not establish a large CPU gain.

Evidence: [execution contract](gpu-producer-execution-contract.md) and [producer measurements](../fixtures/host/data-layer-port/profiling/batched-producer-evidence.json).

### 2026-10-21: complete SQL instrumentation

**Decision:** observe actual statement execution before another SQL-shape change.
The implementation at `e45dd638` imports statement capture, CPU counters, deferred plans, bytecode, timelines, and searchable reports.
Inclusive operation timings do not add to their child SQL timings.
VM instructions, sorts, and cursor visits supply explanations. They do not replace measured CPU.

Evidence: [profiler method and accounting](sql-workload-profiling.md).

### 2026-10-21: isolated SQL-shape experiments

The experiment implementation is `b6934ddf`.
Each row is a median of three fresh processes, with one binary and the pinned five-file quality workload.
All values are process CPU milliseconds. These are historical controlled measurements.

| Experiment | Cold | Edit | Decision |
| --- | ---: | ---: | --- |
| Baseline | 2,966 | 1,180 | Control |
| Method identity index | 2,822 | 1,011 | Keep |
| Forced owner-first joins | 2,842 | 1,048 | Reject forced order |
| Index and forced joins | 2,799 | 985 | Keep index alone |
| Return range index and bound | 2,943 | 1,120 | Keep |
| Narrow projections | 2,815 | 1,098 | Keep |
| UNION ALL root seeds | 3,027 | 1,197 | Reject |
| Materialized root CTE | 3,131 | 1,296 | Reject |
| Root unpivot | 3,080 | 1,247 | Reject |
| Virtual publication | 2,412 | 989 | Keep |
| SQL combination with UNION ALL roots | 2,548 | 835 | Retain as control |
| That combination and virtual publication | 1,996 | 718 | Retain as control |
| Accepted combination with original roots | 1,991 | 685 | Promote |

The method index lets SQLite select the better join order without a forced CROSS JOIN.
`self_stores` falls from about 8.1 million to 77,000 VM instructions.
The return index and upper start bound reduce excessive candidate scanning, while the sort count stays unchanged.
Narrow projections reduce unused column reads and retain row multiplicity and logical schemas.
They do not implement physical column removal or a shared cache of derived facts.

Virtual publication replaces output insertion and SQLite index construction with access to completed numeric arrays.
Diagnostic publication CPU falls from 671 ms to 19 ms, with some extra cost in SQL consumers.
The net workload gain is smaller than that publication saving.

All root rewrites regress in repeated CPU measurements.
UNION ALL removes sorts and lowers VM counts. The materialized CTE and unpivot reduce scans.
Those counter reductions do not offset their other execution costs.
The accepted combination retains the original root query.

Evidence: [experiment report](sql-shape-spikes.md) and [individual runs and counters](../fixtures/host/data-layer-port/profiling/sql-shape-spikes.json).

### 2026-10-21: promotion and removal of optional paths

**Decision:** one permanent execution path, implemented at `7da151b8`.
The method and return indexes, column narrowing, and completed-output virtual table become normal behavior.
The engine retains bounded 32-row insertion and always releases stages after their last consumer.
The three source domains remain separate.

Five execution switches, alternate input adapters, and older authoring controls disappear.
The profiling addon no longer depends on the experimental native-access library.
The module retains its source for SQLite reconnects and releases that ownership on DROP.
Live tables and cursors retain their own references.

Validation covers 400 native tests in each build, the producer gate, the positive corpus, production comparisons, and edit transitions.
Typechecking and Clippy pass in both build modes.
The [promotion evidence](../fixtures/host/data-layer-port/profiling/sql-shape-decisions.json) records the exact scope and hashes.

## Future entries

Each new entry must identify the hypothesis, baseline commit, workload hashes, build configuration, and exact commands.
It must record every timing run, parity scope, explanatory counters, and the keep-or-reject decision.
Diagnostic captures must remain separate from timing medians.
A different binary, allocator, workload, or build configuration needs an explicit comparison boundary.
New experiments must not add selectable execution paths to the engine.
