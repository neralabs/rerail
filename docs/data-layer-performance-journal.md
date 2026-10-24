# Data-layer performance journal

The [incremental SQL preparation design](sql-preparation-architecture.md) and its
[implementation spikes](sql-preparation-spikes.md) distinguish accepted changes from rejected retention experiments.

This journal records measurements, explanations, and implementation decisions for the data-layer ports.
Each entry identifies its implementation and evidence. Historical results remain intact after a new baseline.
Experiments stay in isolated worktrees or standalone harnesses until a keep-or-reject decision.
The engine retains one chosen execution path. Profiling adds observation and does not select another algorithm.

## Recovery TDD: 2026-10-23

The first guard test exposes a real acceptance bypass.
A successful profiling child without an execution report makes the CI launcher return zero.
The new test runs the actual launcher against a temporary controller that omits or bypasses enforcement.
It also covers a forged passing summary with 101 actual executions in the full report.

The red invocation is `.rerail/profiles/sql-recovery-tdd-20261023/guard-red`.
It fails because the launcher returns zero for the missing-report case, where failure is required.
This is a behavioral failure, before the launcher implementation changes.

Red commit: `640b74a2`. The launcher now requires the complete report and independently recomputes acceptance.
It rejects missing evidence and forged passing summaries. The allowance remains fixed at 100.
The green invocation is `.rerail/profiles/sql-recovery-tdd-20261023/guard-green`.
All 22 Node profiling tests pass, including the 100/101 boundary and cross-worker accounting tests.
The real workload remains a budget failure at 24,159 executions; this fix does not claim runtime recovery.

## SQL execution limit restored: 2026-10-23

The source branch defines a fixed 100-execution job acceptance limit in commit `c109a915`.
The earlier profiler import omitted its gate and started/completed coverage counters.
The existing producer-shape gate measures selected native passes and never enforced that SQL limit.

The restored gate counts the complete cold process across workers and connections.
It includes compilation setup, schema DDL, publication, transactions, matching, and cleanup.
Each execution inside `execute_batch` counts separately. Cached statement executions also count separately.
Diagnostic EXPLAIN and enclosing native timing spans do not count.
The gate reports failure after complete evaluation. It never truncates findings at execution 101.

Connection-open and connection-close receipts detect missing whole sessions.
Native started/completed counters detect incomplete statement streams.
Missing coverage, incomplete operations, or failed parity prevent acceptance, including captures with small counts.
Ordinary SQL capture and report commands preserve evidence and return nonzero when the gate fails.
A mandatory CI step uses original positive and clean state-tripwire observations.

The new [evidence](../fixtures/host/data-layer-port/profiling/sql-execution-budget-20261023.json) records a complete real acceptance attempt.
Two original observations run through both snapshot and workspace paths in one cold process.
Both observations pass parity, with one positive, one clean, and no engine or execution-path failures.
The 136 connection lifecycles and their statement counts reconcile, without coverage errors.

| Complete-job SQL category | Actual executions |
| --- | ---: |
| Setup, including DDL and ANALYZE | 17,686 |
| Transactions | 168 |
| Matching, publication, and cleanup | 6,305 |
| Total | **24,159 / 100** |

The trace contains 11,544 CREATEs and 5,508 DROPs.
Execution 101 creates `syntax_21` during compilation schema validation, with an empty context on the main thread.
This result identifies repeated session setup and schema construction in addition to the earlier candidate-preparation regression.
Index publication also repeats SQL over kind catalogs, fields, terms, and files.
The count includes 1,100 executions of `INSERT INTO kind` and 128 executions of `INSERT INTO field`.
Those execution totals are not CPU attribution.

The real workload exits nonzero because the fixed budget fails. Complete findings remain available.
Two instrumented attempts complete in 62.872 and 88.896 seconds, within the 115-second bound.
Both record the same 24,159 executions and complete parity.
The second attempt also preserves separate target and acceptance exit codes, so offline reporting retains successful parity.
Instrumentation and deferred explanations make its CPU unsuitable for an aggregate speed comparison.
The measured addon has SHA-256 `bf671d4a7ade25aa484fa54994fd611d093c671129b17e043d18b6a2056b629f`.
No rule algorithm or production execution option changes in this step.

The [recovery workplan](sql-performance-recovery-workplan.md) orders accounting, eligibility, publication, sharing, bounded SQL, failure correction, and aggregate acceptance.
WP1 remains pending and starts within recovery step R4 in this same worktree.
The restored acceptance failure is not waived while that work proceeds.

Tool checks pass 21 Node tests, 75 Python tests, and six production-tracer tests through the standalone host.
The standalone tracer Clippy check and repository typecheck/static checks pass.
The broad native Clippy command reaches the 112-second controller deadline and remains incomplete.
The focused native library Clippy check passes with the profiling feature.
The existing producer-shape checks also pass, separately from the failing SQL execution gate.
The standalone host gains its missing test-only `tempfile` dependency. Existing dependency versions remain unchanged.
The initial gate-test invocation fails before implementation because its module is absent.
The first standalone native invocation exposes that missing dependency rather than a behavioral test result.

Local captures and validation reports remain under `.rerail/profiles/sql-budget-recovery-20261023/`.

## Aggregate optimization checkpoint: 2026-10-23

Status: repository comparisons are complete, with substantial CPU regressions and failed broader gates.
This is an evidence checkpoint, not a passing performance release. WP1 has not started and will use the same worktree.

The archived pre-SQL reference has SHA-256 `52692a151eea8bab2add9c1c3aeb8400bf0cf1e59e743f5ffdf176265b6f945b`.
Its archived changed-source manifest matches the corresponding files at `5616076a`.
The interrupted first attempt uses addon `4df3c64a06c079d4199e942fbf3daa46dba5405e635c6228f7deb199f57c92a6`.
The final SQL reference is `dab45422f19898d582f330c38fd402aadffc0bd4edb8d916827d85cd63c90419`.
The [aggregate evidence](../fixtures/host/data-layer-port/profiling/aggregate-checkpoint-20261023.json) records current validation progress and frozen source identities.
Both references use engine optimization level 1, dependency level 3, no cross-crate LTO, 256 codegen units, and the system-malloc shim.

Serial comparisons cover the original first-50 and combined 60-rule repository snapshots.
The source pin, file membership, workers, ingestion batches, and expected findings remain fixed within each comparison.
The baseline uses its committed policies. The new reference uses the optimized policy definitions.
This measures aggregate engine and authored-SQL changes. It does not isolate one optimization.
Tracing, SQL capture, and explicit allocation census remain off during CPU timing.

The initial complete native run passes 1,547 tests, fails ten, and ignores 23.
Nine failures come from fixture calls to `git add .`, which the environment rejects.
Explicit fixture filenames correct those calls, and all 17 Git workspace tests then pass.
The remaining allocator-specific test requires jemalloc arena controls absent from the system-malloc reference shim.
The cached genuine jemalloc library then passes the allocator-specific test.
Final native checks use that library; matched CPU references retain the same system-malloc shim.
The run archive preserves the historical shim binary and its hash; its source is unavailable in this workspace.

The broad public-suite attempt reaches its 112-second controller deadline and is not a completed suite result.
It exposes additional Git fixture calls and two stale catalog assertions.
The corrections preserve explicit fixture inputs, reject an actually unknown predicate, and compare grammar membership independently of enum declaration order.
Typechecking and all repository checks pass before those test-only corrections.

Local runs and frozen inputs are under `.rerail/profiles/aggregate-checkpoint-20261023/`.
Runtime source is committed at `5b09437c`; fixture and runner corrections are committed at `b9c4fc67`.
The final measurement archive pins the addon and all 470 native source files.

| Workload | Reference | Cold CPU | Resource high-water RSS | Parity |
| --- | --- | ---: | ---: | --- |
| First 50 rules | Pre-SQL | 125.804 s | 6.441 GiB | 24,820 findings, pass |
| First 50 rules | Final SQL | 2,507.496 s | 6.143 GiB | 24,820 findings, pass |
| Combined 60 rules | Pre-SQL | 709.151 s | 16.102 GiB | Findings match; 23 failures, two failed incremental checks |
| Combined 60 rules | Final SQL | 7,211.657 s | 14.997 GiB | Findings match; 34 failures, two failed incremental checks |

The fresh pre-SQL first-50 reference completes all 9,021 files with 24,820 findings and no parity failures.
Cold CPU is 125.804 seconds. This replaces historical CPU values as the matched comparison reference.
The corrected first-50 SQL reference completes with all 24,820 findings and no parity differences.
Cold CPU is 2,507.496 seconds, or 19.93 times the matched pre-SQL reference.
Reported resource high-water RSS falls from 6.441 to 6.143 GiB, a 4.6 percent reduction.
Separate `ps` sampling records peaks of 6.441 and 6.217 GiB.
Sampled RSS and `process.resourceUsage().maxRSS` do not agree exactly in these reports; the evidence preserves both measurements.
Some ingestion boundaries also report current RSS above the resource high-water counter.
These observations prevent a strict bound between the two APIs, without changing the large CPU regression.
This aggregate result is a substantial CPU regression, rather than an optimization success.
The prior 60-rule reference completes with all 24,993 expected findings and no finding differences.
It reports 21 file-budget failures, two incomplete-input failures, and two failed incremental checks.
It is not a successful parity reference.
The failures include serverplan-target-architecture clauses on `commit_branch_steps.py`.
The final SQL 60-rule run completes with all 24,993 expected findings and no finding differences.
It reports 34 relational failures in `serverplan-state-tripwires`: 12 regex-budget errors and 22 query interruptions.
The unchanged and trailing-newline edit checks retain those failures, with zero finding differences.
Matching findings do not establish complete execution when failures remain.
The regex and SQLite progress handlers share a deterministic work allowance; these errors are not command wall-time deadlines.
The full-repository result therefore fails, despite the passing source-corpus tests below.
The two 60-rule references fail differently, so their times do not establish a clean performance ratio.

The final 60-rule cold CPU splits into 6,782.080 seconds for ingestion and 429.207 seconds for the global apply.
The failed prior reference uses 202.550 and 506.184 seconds respectively.
The final unchanged check uses 0.517 seconds CPU; the edit uses 121.587 seconds, versus 62.576 seconds previously.
Total OS process CPU is 7,341.770 seconds, including startup, compilation, and incremental checks.
Sampled peak process RSS is approximately 14.997 GiB, close to the reported resource high-water value for this run.
The controller records 15,877.030 seconds wall time, including the recorded pause and diagnostic interference.
The OS `time` report instead records 3,516.32 seconds as `real`; this clock discrepancy remains unresolved.
Both raw reports remain preserved. Neither wall value supports a comparative performance claim.
Its 288 ingestion batches execute 1,363,392 candidate data statements and 1,359,072 materialization actions.
They prepare 197,856 plan memberships: 687 plans per batch, versus 377 in the first-50 run.
The same 28,230,245 indexed nodes feed both rule sets. Adding ten rules broadens preparation work substantially.
Native anchor-selection counters report 6,421.643 seconds; nested thread counters are not an additive process-CPU breakdown.

The controller restarts after the completed references to remove its arbitrary three-hour cap for the final full run.
That run uses the existing approval to finish and records OS process CPU and peak RSS alongside driver phase CPU.
The briefly started, interrupted 60-rule SQL trial remains excluded.
The final SQL trial is allowed to finish despite its measured CPU regression.
Intermediate process samples include startup and do not replace completed driver phase measurements.
During that trial, host free space falls below four GiB.
Cleanup removes 0.782 GiB of unopened, orphaned temporary index files after two batched open-file checks.
Transparent APFS compression saves another 4.519 GiB across 167 old text captures.
SHA-256 checks verify unchanged capture contents; their paths remain intact.
The active database, reference binaries, and source inputs remain unchanged.
This storage maintenance can add host contention during the 60-rule trial; the completed first-50 comparison precedes it.
Host free space later recovers above 550 GiB without a storage pause of the target process.

The completed first-50 reports locate the CPU regression in file ingestion.

| Cold phase | Pre-SQL CPU | Final SQL CPU |
| --- | ---: | ---: |
| File-input applies | 73.350 s | 2,456.015 s |
| Global result apply | 52.144 s | 51.145 s |

The current native counters report 2,247.966 seconds in anchor selection and 125.776 seconds in anchor index construction.
Parsing uses 11.087 seconds. These counters use thread CPU and can nest; they do not form an additive process-CPU breakdown.
Historical native counters need their original clock semantics before a direct phase comparison.
The driver apply measurements above use process CPU on both references.
The first-50 cold ingestion executes 288 candidate batches, 492,480 data statements, and 489,024 materializations.
Those batches publish 28,230,245 indexed nodes across 9,010 eligible file memberships.
Their plan memberships total 108,576: 377 plans per batch.
These are runtime counters from the complete run, rather than static SQL placement estimates.

The first-50 run exposes an ingestion CPU regression absent from the earlier ten-rule controls.
At 576 ingested files, sampled process CPU reaches about 209 seconds, while retained process RSS remains below one GiB.
The historical first-50 run completes all 9,021 files in 212.6 cold CPU seconds under different reference-build settings.
That observation signals a regression but does not establish a matched full-run ratio.
The new run's ingestion counters attribute most active work to candidate SQL preparation and execution.

### Broad-run access-path regression

The initial first-50 comparison stops after 960 ingested files and approximately 560 sampled process CPU seconds.
It remains an interrupted diagnostic, without a complete parity or CPU result.
One external sampling window contains 730 active native samples, all under candidate SQLite work.
The sample records VM execution, B-tree traversal, and file I/O, rather than mostly SQL compilation.
It does not assign whole-run CPU percentages.

The bundled `fts5VocabBestIndexMethod` accepts term constraints and ignores document constraints.
The candidate query joins requested files after the vocabulary instance stream scans corpus-wide postings.
Later ingestion batches therefore repeat work over an increasing corpus.
This access-path cost was absent from the earlier tiny-corpus timing controls.

A regression test requests two identifier nodes from one structurally completed file.
Adding an unrelated file with 2,000 statements increases its progress-counter ticks from 1 to 361, with 100 VM instructions per tick.
An initial decoded `(fid,kind,pre)` index passes the covered-file test but fails the representative sparse ingestion probe.
The 112-second diagnostic reaches approximately 256 files, so this trial is rejected.

The selected correction adds compressed positions to existing SQL `file_kind_counts` rows.
The existing `(fid,code)` primary key seeks each requested file and kind before the postings table expands positions.
The existing record pass builds these segments alongside counts, without additional SQL rows or per-node queries.
The original FTS index remains intact. Sparse files need no eager decoded property rows.
An atomic version-2 store upgrade derives positions from persisted records, without source reparsing.

All ten SQL preparation tests pass, including four new access-path and upgrade tests.
They cover sparse and completed files, unrelated-file growth, error and missing syntax, edits, removals, and failed-upgrade rollback.
The intermediate kind-only addon has SHA-256 `f78fad34aab59e19be3e202b450887341156d281279a2ee153d25fa7148ceed6`.
Its native source manifest is frozen before repository timing.
This trial remains provisional: repository ingestion still exposes corpus-growth cost.

Before the value regression test, the complete native suite passes in three disjoint groups: 1,561 pass and 23 ignore.
The allocator-specific case runs against genuine jemalloc.
Typechecking, boundaries, structure, Windows paths, and profiling-tool checks also pass.
The full repository comparisons and complete source-corpus checks remain pending.

The corrected-kind trial stops after 480 files rather than locking down an unexplained ingestion regression.
Its external sample again shows VM execution and file reads as the main active native work.
A second bounded-work test now checks exact value lookup as unrelated files increase.
The exact-value test fails: 200 unrelated files increase work from 1 to 19 progress ticks.
The selected SQL correction fences term and requested-file inputs before the existing `vidx` primary-key seek.
The missing-value fallback likewise starts with requested files.
All eleven preparation tests then pass, including the new value growth check.
The rebuilt reference has SHA-256 `e3b6541c2534a335cfeb2863b6af623bbfa0473ae1f8f0cd68070c142177bbf4`.
It is now frozen for final public checks, source replay, and repository measurements.

The final native test inventory contains 1,585 tests: 1,562 pass and 23 ignore.
A bounded group times out with three slow query checks pending; separate bounded invocations complete those checks.
The coverage manifest accounts for each test name rather than treating the interrupted group as a suite pass.
Both the execution-shape gate and all 25 structure metrics pass, including newly added files.
The profiler also gains tested text-summary support for source and incremental replay reports.
Earlier summary-format exceptions leave successful replay results intact; regenerated summaries use those preserved results.

Some public test groups exceed their bounded controller deadline and require smaller independent groups.
The node-pack scale test passes in isolation on both reference addons; it is not itself a proven regression.
One audit test fails in an initial group and passes in isolation.
Final public verification runs independent files and individually selected source-wrapper tests.
The profiler retains its 115-second command budget; the public invocation raises the default per-test timeout to 90 seconds.
Three source wrappers initially exceed the five-second default and pass with that invocation.

The serverplan-steps recording has 746 replayable observations, so one per-rule job cannot meet the command budget.
It now uses 63 disjoint, balanced jobs, with original observation identities preserved exactly once.
Absent source fields remain absent on retained observations; original recordings remain unchanged.
The combined positive and clean coverage remains mandatory for every rule.
The public pack-validation wrapper also exceeds the command budget.
Per-pack invocations retain all examples and fixture checks, but JavaScript, Node, Python, React, TypeScript, and Vue still hit the 112-second controller limit.
These remain incomplete broader checks, without a passing suite claim.
The SQL profiling Node test now stays outside Vitest collection and passes under its dedicated runner.
One unrelated TypeScript scale check exceeds its five-second wall-time assertion on the busy shared host.
That assertion remains unchanged. The paired baseline addon passes the same test.
The corrected addon still fails in isolation; an instrumented retry measures 8,428 ms wall and 3,849 ms CPU for mixed evaluation.
The source case contains 1,000 files and 2,293,760 bytes.
This is a broader performance regression, without a source-parity failure.
The test reports CPU beside wall time without relaxing its existing ceiling.
The complete corpus now passes all 133 bounded source jobs.
They cover 3,210 native observations across all 60 rules: 1,631 positive and 1,579 clean.
Three host signature guards remain separate from native execution counts.
False positives, false negatives, engine failures, and snapshot/workspace differences are all zero.
All 40 edit transitions also pass.
The evidence verifies original observation identities across shards without summing repeated collection metadata.

### Retained-property join regression

The post-value first-50 attempt still stops after 480 files and 175 sampled process CPU seconds.
It remains an interrupted diagnostic, without a complete comparison result.
A new test reads structural parents for one requested file, then retains 2,000 statements in an unrelated file.
Identical results require 806 progress ticks instead of six.
Each tick represents 100 SQLite VM instructions.

The property views use ordinary joins between demanded keys and retained decoded rows.
The selected correction makes requested keys the outer input with `CROSS JOIN` fences.
It covers universal node reads and keyed node, span, and child-property reads.
This preserves the existing indexes, row semantics, and batched execution interface.
All 60 active SQL anchor tests pass after the correction; one benchmark test remains ignored.
The rebuilt addon has SHA-256 `dab45422f19898d582f330c38fd402aadffc0bd4edb8d916827d85cd63c90419`.
All 1,586 native tests are accounted for: 1,563 pass and 23 ignore.
The forty edit transitions also pass on this final build.
All 133 bounded source jobs pass again on this final build, with the same 3,210 native observations and three host guards.
False positives, false negatives, engine failures, and snapshot/workspace differences remain zero.
All six larger pack retries still hit the command deadline and remain incomplete broader checks.
Typechecking, both Clippy configurations, execution-shape checks, and all-file structure checks pass.
The TypeScript wall-time guard still fails during these contended checks: 12,705 ms wall and 4,307 ms CPU.
Its correctness assertions pass, and the wall-time ceiling remains unchanged.

### Preparation frontier found during the full comparison

The workspace gathers anchor plans from every compiled rule before it publishes the eligible source jobs.
It filters publication when a file has no eligible AST rule, but then prepares every language-compatible plan for each published file.
The corpus API receives language and plan identities, without per-rule file eligibility.
This makes the plan/file frontier broader than the final matcher work.

One first-50 batch publishes 32 files and prepares 377 plans with 1,710 statements and 1,698 materializations.
Its profile reports 128 rule-file entries and 128 rejections.
The SQL frontier therefore performs substantial work before final filtering.
This observation does not quantify its removable CPU share; a scoped-frontier experiment remains future work.
Here, a rejection means zero selected roots. It does not prove that scope or condition analysis could skip all preceding work.
Across the completed first-50 run, 83 applies report zero roots for every measured rule-file selection.
They still execute 140,934 materializations and use 697.407 seconds of anchor-selection CPU.
That cost includes work needed to prove absence. It is not automatically removable.

`SelectWhen` planning also collects both queries before the runtime condition selects consumers.
The next execution design must preserve eligibility and conditional demand while retaining broad batched launches.
A complete static plan inventory does not require execution of every plan/file pair.
Repeated selector construction around retained facts remains another measured target.

This separates index join order from the planned cross-rule producer work; WP1 remains pending.

### Large-corpus pager pressure

Across four equal groups of 72 first-50 ingestion applies, anchor-selection CPU per indexed node rises from 50.2 to 115.3 microseconds.
The last group processes fewer nodes than the first. Source content also changes, so this comparison alone does not establish a corpus-size cause.

A one-second external sample observes the final 60-rule run near 1,600 ingested files.
Of 749 selected non-parked stack observations, 537 end in `pwrite` and 65 end in `pread`.
The write callers include `pagerStress`, `pagerWalFrames`, and `subjournalPageIfRequired` under candidate materialization.
These are wall-time stack observations, including possible I/O waits, rather than measured CPU shares.
SQL compilation appears in seven observations. The window identifies pager and statement-journal traffic as a concrete investigation target.

The production store sets its main page-cache target to 1,024 KiB and its temporary target to 512 KiB.
The main database already exceeds one GiB during this sample interval.
Cache-budget and publication-boundary experiments must preserve rollback guarantees and measure cache misses, writes, spills, CPU, and memory together.
The isolated cache experiment below tests this proposal. The native implementation stays frozen.

An APFS clone preserves the main database and WAL for later isolated replay.
The target process pauses for 72 milliseconds during the clone, then resumes immediately.
The snapshot needs SQLite recovery and validation before replay.
The 60-rule timing therefore includes external sampling, this brief snapshot pause, and the storage maintenance above.
The completed first-50 controls precede these diagnostics and remain separate.

The profiling report now labels I/O stack observations explicitly to prevent their interpretation as CPU percentages.
Its new counter test passes. The profiling suite now passes 74 Python checks and 14 Node checks.
The standalone replay host also gains a `cache_budget` command with cheap cache counters and explicit physical budgets.
It reuses scheduled SQL execution and avoids the ownership observer's full-table `dbstat` scans.
Its row-bag and file-scope test passes with the other 18 standalone Rust checks, and the command builds successfully.
This addition changes profiling tools only; production cache targets remain frozen.

### Controlled cache replay

The full process pauses for 1,022.218 seconds while isolated replay runs, then resumes with its computation intact.
This pause affects the reported 60-rule wall time. It does not add CPU to that stopped process.

The fixture combines the recovered native snapshot with a native-recorded state-rule input, without rebuilding its indexes.
It retains 1,886 files and 6,223,803 nodes, and requests `_conditional_compile.py` with its original 18,993-node index representation.
Source digests match the pinned checkout, and SQLite `quick_check` passes.
This query-scaling fixture holds the requested program and index representation constant; it does not measure publication or complete engine execution.

Twelve replays use three rotated observations for each physical cache pair.
All preserve the exact 146,136-row candidate multiset, including multiplicities, after file-ID normalization against the small native capture.
Every large-fixture replay records 59,590,722 VM steps, 3,294,894 full-scan steps, and 4,979 sorts for stage and result statements.
Those VM totals exclude index and release batches.

| Main / temporary cache target | Median SQL CPU | Median process peak RSS | Cache misses | Cache writes | Cache spills |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 MiB / 0.5 MiB | 1,531.078 ms | 112.656 MiB | 66,709 | 14,949 | 14,630 |
| 64 MiB / 0.5 MiB | 1,336.781 ms | 143.563 MiB | 10,814 | 5,427 | 1,721 |
| 1 MiB / 16 MiB | 1,402.635 ms | 109.344 MiB | 62,595 | 13,119 | 12,909 |
| 64 MiB / 16 MiB | 1,403.610 ms | 150.484 MiB | 6,700 | 3,597 | 0 |

The combined larger budgets eliminate measured spills and reduce median SQL CPU by 8.3 percent in this fixture.
CPU varies across trials; these observations do not establish a preferred full-engine budget.
They also do not explain the 19.93-fold aggregate CPU regression.
The unchanged VM work remains substantial for one requested file, so demand scope and repeated query work remain the main architectural targets.
Production settings remain unchanged for the completed checkpoint comparison.

### Full-run memory observation

An external `vmmap` inspection during 60-rule ingestion completes in 3.023 seconds.
It reports a 6.6 GiB physical footprint, with 4.5 GiB resident and 1.9 GiB reported swapped in `MALLOC_SMALL` regions.
Its allocator-zone view reports 15,992,992 allocations and 6.2 GiB allocated.
These rounded region and allocator views overlap; they are not an ownership census.

Prepared candidate payloads account for about 218 MiB near this interval.
The completed first-50 run peaks at 86.565 MiB of compact candidate payloads.
Those counters exclude map metadata, but the payloads alone cannot explain the process footprint.
The explicit workspace estimator also omits heap storage owned by retained tape, scope, and preparation fields.
Attribution of the remaining native allocations remains open; this observation does not establish a leak in any specific structure.

The profiling framework now extracts these external memory fields into structured reports while preserving the raw output.
The full 60-rule timing includes this additional external observation window.

A second external inspection runs after cold completion, during the unchanged/edit boundary, and takes 7.527 seconds.
Its rounded allocator-zone view reports 31,521,959 allocations and approximately 11.3 GiB allocated.
Physical footprint is approximately 12.7 GiB, with a reported peak of 13.9 GiB.
These views remain distinct from peak RSS and from Rust ownership estimates.
The larger retained allocation count reinforces the need for ownership attribution before a memory improvement claim.

### Final diagnostic and tooling checks

The profiling suite passes 75 Python checks and 14 Node checks, including the new external-memory parser test.
All 19 standalone SQL replay tests pass. Standalone format and Clippy checks also pass.
The frozen manifest still matches all 470 native files after the full run.
Final typechecking, boundary checks, structure checks, Windows-path checks, and execution-shape checks pass.
The structure check also passes after the new replay command enters the tracked file inventory.
The broader public-suite failure and six timeouts above remain unresolved; these checks do not supersede them.

The bounded ten-rule/two-file diagnostic preserves all 31 expected findings with no execution failures.
It records 11,594 trace events and a successful external memory inspection.
Two external sample commands exit with signal 9, so their observer wrapper fails despite target success.
The third sample succeeds; its 823 selected non-parked observations include 299 I/O syscall leaves.
These wall-stack observations include profiler and trace overhead and do not establish CPU percentages.
The separate state-rule SQL capture also passes, recording 6,095 trace events with query plans.
Neither instrumented workload replaces the untraced aggregate comparison.


## Shared producer implementation plan: 2026-10-23

The [workplan](shared-producer-workplan.md) defines eight runtime milestones after the existing offline tooling.
The first execution milestone shares span preparation across actual rules and result packets.
Later milestones cover structural facts, compiled frontiers, cross-clause SQL owners, pack definitions, and reviewed derived producers.
An early connection check establishes the storage assumptions before cross-clause integration.

Each runtime change requires parity evidence and matched CPU and allocation measurements.
The plan retains the existing installed baseline and separates static placement counts from runtime construction counters.
This entry records planning only. The runtime milestones remain pending, and no new benchmark or Linear issue enters this entry.

## Named producer discovery and declaration checks: 2026-10-23

The selected [contract design](shared-producer-contracts.md) replaces general equivalence discovery as the primary sharing mechanism.
Contracts live beside their implementations, while a central registry references them across consumers.
Pack-specific derived work belongs in a pack producer library. Private rule stages retain qualified local identities.
The runtime registry and compiler export remain implementation work.

The new [CLI](../scripts/profiling/shared_work.py) discovers repeated work in candidate captures and checks explicit contract manifests.
Clash reports contain both origins and exact field differences.
Binding diagnostics explain why different options, generations, snapshots, or domains prevent sharing.
Different demand sets can share a keyed producer. Unknown semantic fields fail declaration checks.

The [evidence](../fixtures/host/data-layer-port/profiling/shared-producer-discovery-20261023.json) retains capture hashes and discovery witnesses.
The tool processed the earlier state and ten-rule captures, without new engine execution.
Both reports cover every host statement, without skipped scopes.

| Captured workload | Host statements | Materialized placements | Definitions in multiple hosts | Extra host placements |
| --- | ---: | ---: | ---: | ---: |
| State | 1,716 | 10,320 | 157 | 1,240 |
| Ten rules | 3,216 | 19,001 | 255 | 2,048 |

These counts are lower than the earlier audit because the new tool distinguishes mutable-write epochs and physical CTAS projections.
It also preserves quoted literals and resolves local dependency context.
The earlier journal entry retains its original method and results.
These are static placement counts, without runtime build counts or CPU savings claims.

The 25 new checks cover false sharing, dependency aliases, conflicting names, typed options, snapshots, domains, cycles, and digest collisions.
The complete profiling-tool suite passes, and CI includes the new checks.
Production engine source and the installed addon remain unchanged.

## Compiled stage execution design: 2026-10-23

The [execution design](sql-execution-stages.md) selects a general planning graph with a specialized runtime controller.
The compiler fixes dependencies, operation families, storage bindings, and potential consumer lifetimes.
Runtime frontiers resolve actual demand, conditional gates, coverage, and capacity admission.
Recursive algorithms remain inside declared stages. SQL retains relational execution.

The design combines candidate lifetimes and clause producer waves under one evaluation plan.
Demand-source leases prevent retirement before later consumers discover their inputs.
Shared SQL backing uses a workspace writer and scoped clause readers without fact imports.
The connection contract requires an explicit isolation and cancellation proof before cross-clause promotion.

This entry records design work only. Production source and addon binaries remain unchanged.
The existing CPU and allocation measurements remain the implementation baseline.

## Materialization ownership design: 2026-10-23

The [target design](sql-materialization-shape.md) assigns one owner to every selected producer version and coverage partition across rule consumers.
Statements and result packets read those owners without embedding another materialized copy.
The design covers demand waves, empty completion, semantic domains, clause sharing, and retirement.
It retains indexed base access and single-use pipelines without blanket materialization.

The [static placement audit](../fixtures/host/data-layer-port/profiling/sql-materialization-design-20261023.json) resolves local dependencies and typed parameters before comparing definitions.
It identifies 248 state definitions and 420 ten-rule definitions present in multiple generated statements.
One demanded-span definition appears in 61 state statements and 90 ten-rule statements.
These are SQL placement counts, not measured execution counts or CPU savings.
The audit also identifies repeated empty-set materializations, which require simplification rather than another retained cache.

This entry records design work only. Production code, addon binaries, and execution remain unchanged.
Earlier benchmark results remain the baseline for implementation.

## Candidate SQL relation lifetimes: 2026-10-23

**Decision:** retain one dependency schedule with producer construction before demand and release after the last SQL consumer.
This bounds temporary storage. CPU and process RSS remain mostly flat on the measured snapshots.

The [implementation](sql-composition-implementation.md#candidate-relation-lifetimes) describes the execution contract.
The [evidence](../fixtures/host/data-layer-port/profiling/sql-relation-lifetimes-20261023.json) retains trials, source hashes, storage measurements, allocation peaks, and corpus results.
The reference addon has SHA-256 `7a193b2a060803b6e5f73e6760066e72c43d962aa673c2fd81f51f38405f6c3b`.
The final addon has SHA-256 `86846fe45e731c10b44beecb54cbfb35b7610c2eafb71f199ce3840969c07b6d`.
The data-layer source remains at `2b1504adc4291daaf080b2e932962b134da1a69a`.
No policy changes enter this comparison.

### Implementation

Previously, the executor constructed every temporary candidate relation before the first result packet.
It retained every relation until all packets finished.
The new compiler derives dependencies from generated SQL and retains one schedule in each cached template.
It preserves ordering between retained-property reads and writes, including vocabulary maintenance.
Physical table reads determine storage lifetime separately from execution ordering.
Shared producers run once across output packets. Output order remains unchanged.
Adjacent releases share one SQL call. Error cleanup releases remaining tables and preserves the original failure.
The streaming adapter and retained decoded properties remain in use.
Production has one execution path and no new algorithm switch.

Two TDD ladders cover the schedule and its ordering constraints.
The second ladder prevents write ordering from extending an unused snapshot's storage lifetime.
Additional checks cover shared inputs across packets, delayed construction, maintenance writes, and retry after a later packet fails.
SQL profiling explains new statements before their inputs disappear, without repeated report serialization.

### Storage and paired process measurements

The replay host uses captured production SQL, codecs, database settings, and the production schedule.
Its eager comparison exists only in the diagnostic binary.
Both layouts emit identical candidate rows in identical order.
The state replay emits 146,136 rows. The ten-rule replay emits 726,444 rows.

| Snapshot | Peak live tables, before → after | Peak temporary storage, before → after |
| --- | ---: | ---: |
| State, one file | 1,029 → 206 | 14.00 → 2.66 MiB |
| Ten rules, two files | 1,905 → 437 | 48.10 → 8.99 MiB |

The following numbers are medians from three observations per side.
Cold CPU includes process user and system time. Compilation CPU remains separate.
Peak RSS includes setup and compilation.

| Snapshot | Cold CPU, before → after | CPU change | Peak RSS, before → after | RSS change |
| --- | ---: | ---: | ---: | ---: |
| State, one file | 2.582 → 2.597 s | +0.6% | 453.2 → 452.5 MiB | −0.2% |
| Ten rules, two files | 13.317 → 13.106 s | −1.6% | 563.1 → 553.3 MiB | −1.7% |

State CPU observations range from 2.534–2.729 seconds before and 2.577–2.647 seconds after.
Ten-rule CPU observations range from 13.005–14.127 seconds before and 12.994–13.388 seconds after.
Compilation CPU medians are 0.301 → 0.307 seconds for state and 0.759 → 0.747 seconds for ten rules.
These measurements establish bounded temporary storage, without a substantial CPU or RSS improvement.

Each workload uses the same selection, source hashes, policy digests, allocator, four workers, and file batch size of 32.
The serial order is before-0, after-0, after-1, before-1, before-2, after-2.
The benchmark runs without concurrent builds or checks from this task.
Tracing, SQL capture, allocation accounting, and explicit ownership census remain off during timing.
Each workload command uses the 115-second controller deadline.
Long compilation commands use the earlier build exception.
Shared-host load varies. Three trials per side do not establish statistical significance.
These snapshots do not establish full-repository performance.

### Why process memory remains high

New trace boundaries distinguish live objects, table bytes, index bytes, reserved pages, free pages, and SQLite resident memory.
They also record current RSS, peak RSS, and available allocator statistics.
Each snapshot retains only the largest 20 objects and totals for all objects.
Memory gauges retain maxima rather than sums.

The trace-only state run ends candidate preparation with zero live temporary candidate tables.
Its AST-index connection retains about 1.54 MiB of SQLite cache and about 2.65 MiB of free temporary pages.
Allocator usage changes from about 65.9 to 66.6 MiB across candidate execution.
Managed allocator space grows from about 138.4 to 210.4 MiB.
Current RSS grows by about 71.7 MiB across the same boundary.
Managed allocator space describes address space, not resident physical memory.
These observations indicate transient allocation pressure and retained allocator space rather than large remaining physical candidate tables.
Later rule fact construction raises the process peak further.

The isolated replay enables SQLite allocation accounting before initialization.
It separates SQL preparation peaks from execution peaks and compiled statement storage.
This accounting does not change production settings or enter the paired timings.

| Snapshot | Largest SQL preparation peak | Largest SQL execution peak |
| --- | ---: | ---: |
| State | 10.1 MiB | 68.5 MiB |
| Ten rules | 11.8 MiB | 98.6 MiB |

The state execution peak occurs in the first result packet and returns to about 2.1 MiB afterward.
Its compiled statement occupies about 3.3 MiB.
The query contains 1,101 inline materialized CTEs.
Its captured plan reports 1,101 materializations and 983 temporary B-trees.
These inline working sets remain outside the physical-table schedule.
The allocation peak belongs to execution, not preparation alone.
The measurements do not assign bytes to each individual CTE.

The next planning change must include inline materializations in the working-set budget.
Candidate approaches include scheduled physical producers and result packets bounded by their actual producer footprint.
Any packet change must retain sharing and measure repeated work across packets.
The next comparison must measure statement execution peaks, CPU, and process RSS together.
Physical table counts alone cannot establish a memory improvement.

SQL profiling retains large plans and increases diagnostic RSS.
The ownership and allocator discussion therefore uses the separate trace-only run.
An earlier replay reported zero global SQLite counters because memory accounting was disabled.
The corrected replay enables accounting before SQLite initialization and records nonzero preparation and execution peaks.
The evidence excludes intermediate addons and superseded diagnostic runs from the final comparison.

### Checks

The final addon passes all 269 affected original corpus calls: 194 positive calls and 75 clean calls.
There are no false positives, false negatives, or engine failures. Workspace and stateless results agree.
The ten-rule smoke replay passes 20 calls, including 10 positive and 10 clean calls.
The affected public suite passes 324 tests, with two existing skips.
Focused native checks pass 82 matcher tests, 10 profiling tests, and 20 relational tests.
The standalone harness passes 18 tests.
Typechecking, profiling-tool tests, formatting, boundaries, structure checks, and Clippy pass.
Both production and profiling Clippy checks use the pinned Rust 1.95.0 toolchain.
The installed addon also passes the multi-packet and guarded-root smoke checks.
The real Git index remains unchanged.

## SQL composition promotion: 2026-10-22

**Decision:** retain selected-forest intervals, explicit span demand boundaries, and one shared candidate catalog per grammar frontier.
Keep the streaming packed-node adapter. These choices add no production algorithm switch.

The [implementation](sql-composition-implementation.md) describes the interfaces and SQL execution boundaries.
The [evidence](../fixtures/host/data-layer-port/profiling/sql-composition-promotion-20261022.json) records artifacts, source hashes, trials, diagnostics, and corpus results.
The reference checkout remains `5616076ad60127d6ac3df0fbf60eff9f7a523ca0` with the existing uncommitted typed-fact changes.
The saved reference addon has SHA-256 `efa0dcf04b170a10588e4661c7843a846989ce98262e68b6bc6abbb6a7dca72f`.
The measured addon has SHA-256 `7a193b2a060803b6e5f73e6760066e72c43d962aa673c2fd81f51f38405f6c3b`.

### Implementation and integration failures

The forest adapter retains indexed positions and requested root intervals in SQLite.
It validates the selected graph before publication, including cycles, distinct parents, and disconnected components.
It preserves external numeric and text identities.
A 1,000-node chain retains 1,000 positions and 1,000 root ranges while exposing the same 500,500 logical pairs.
Consumers still pay for the rows they enumerate.
General directed graphs retain cycle-safe recursive SQL.

Root queries can select `__path,id,expand`.
False or null expansion keeps the reflexive pair.
If any duplicate request permits expansion, the root expands.
The four authored state clauses now use this representation and preserve their original root guards.
Syntax also exposes exclusive raw preorder bounds for SQL range predicates.
Raw bounds do not replace selected-edge graph validation.

Span selection remains materialized before costly source predicates.
All plans share one candidate definition catalog per grammar.
Output packets contain at most 128 arms and consume the same prepared relations.
The 535-output regression uses five packets with 128, 128, 128, 128, and 23 rows.
The v2 diagnostic capture and standalone replay preserve all 535 rows.

The initial integration exposed two regressions. Both required corrections before timing the selected implementation.
Sharing the full catalog amplified repeated copies and scans during SQL generation.
Indexed dependency traversal reduces template generation from 1,116.586 to 604.187 CPU ms in the diagnostic runs.
The old implementation used 600.219 CPU ms across its five smaller catalogs.
This correction removes the new regression. It does not establish faster compilation than the reference.

SQLite also reordered a correlated subtree probe to scan all identifiers before interval membership.
The affected `direct_events` stage increased from 774,666 to 16,328,922 VM steps.
Explicit `CROSS JOIN` order keeps each demanded subtree before its identifier lookup.
The corrected stage uses 985,870 VM steps.
This is an authored SQL change, and the generator emits it identically.

On the state diagnostic, candidate catalogs decrease from five to one.
Temporary candidate tables decrease from 1,784 to 1,029.
Candidate materialization and fill stages decrease from 3,044 to 1,712.
Result packets decrease from five to two for the 139-plan frontier.
These counts exclude other relational consumer tables.

### Measurements

Both versions use engine optimization level 1, dependency level 3, 256 code generation units, and no LTO.
Both addons use the same Rust 1.97.1 compiler, four native workers, and the system-malloc shim.
The repository checks use its pinned Rust 1.95 toolchain.
Timing controls disable tracing, SQL capture, external sampling, and memory census.
They run serially without our builds or tests.
CPU includes process user and system time during cold evaluation. Policy compilation remains separate.
RSS measures the whole process high-water mark, including setup and policy compilation.

Each result below is the median of three observations per version, with rotated execution order.
The state snapshot reads one 108,562-byte file and expects zero findings.
The smaller ten-rule snapshot reads two files, including required linter metadata: 178,826 bytes and 31 findings.
Fresh Python recording supplies its expectations.
Both versions preserve every expected finding without failures.

| Snapshot | Before CPU | After CPU | CPU change | Before peak RSS | After peak RSS | RSS change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| State, one file | 2.424 s | 2.120 s | -12.5% | 397.2 MiB | 451.5 MiB | +13.6% |
| Ten rules, two files | 13.338 s | 9.903 s | -25.8% | 497.1 MiB | 540.9 MiB | +8.8% |

The shared host causes substantial variation.
State CPU ranges from 2.375 to 5.619 seconds before and from 1.952 to 5.630 seconds after.
Ten-rule CPU ranges from 11.841 to 21.036 seconds before and from 9.588 to 12.019 seconds after.
The medians show a CPU reduction on these snapshots. They do not establish a stable full-repository speedup.

The original five-file ten-rule snapshot exceeds the deadline for both versions.
The first smaller after run also exceeds the deadline. Its completed retry remains identified in the evidence.
Censored runs and diagnostic runs do not enter the medians.
An external sample attempt also expires during symbol processing and produces no usable bottom-up attribution.
It supplies no evidence for a CPU attribution claim.

Policy compilation uses median CPU of 0.235 versus 0.344 seconds on the state snapshot.
It uses 0.658 versus 0.585 seconds on the smaller ten-rule snapshot.
These values do not enter the cold CPU table.

Peak memory increases on both snapshots.
State RSS after global evaluation increases from 221.5 to 400.5 MiB.
The smaller ten-rule snapshot increases from 446.1 to 489.4 MiB at that boundary.
Fewer SQL constructions do not guarantee lower allocator residency or a smaller live relation frontier.
The comparison does not yet separate those causes.
The next memory investigation must measure overlapping relations, indexes, and allocator residency across the shared catalog.
The system-malloc shim does not establish production jemalloc memory behavior.

These selections do not compare all 50 rules against the historical full-repository run.


### Validation and limits

The complete affected source corpora pass: 77 state calls and 192 quality calls.
They include 194 positive and 75 clean calls, with no false positives, false negatives, or engine failures.
Workspace and stateless execution agree.
Each bounded replay keeps the complete input of each original call.
The split does not partition repository-wide analysis by file.
A separate smoke replay covers one positive and one clean call for each of the ten rules.

The corpus replay started on addon `93733caa969fbf178f55e76870736c7a5ef6c84839bdfbe3848b6ae57faa11d7`.
The final addon adds an aggregate counter correction for extra result packets.
It passes the remaining corpus groups, the complete smoke replay, and all 324 public regression tests.
Per-group binary hashes remain in the evidence.
The native checks pass 76 anchor tests and 20 relational tests.
The standalone checks pass 12 Rust tests, including dependency traversal and v1/v2 replay.
All ten generated policies reproduce the checked files.
Typechecking, profiling checks, source boundaries, Windows paths, formatting, and all 25 structure metrics pass.
Clippy passes with and without profiling on the pinned Rust 1.95 toolchain.
A Rust 1.97 check reports a newer lint in an existing Rust-language test.
It does not change the implementation.

Workloads retain a 115-second controller deadline.
The user approved longer compilation after bounded build attempts expired.
Timed-out commands and diagnostic runs remain recorded and do not enter the performance medians.


## SQL composition opportunities: 2026-10-22

**Decision:** pursue proven tree intervals, explicit demand boundaries, and shared definitions with physical cuts.
Keep the current streaming node table for CPU execution.
Extra copied column buffers do not establish a CPU improvement.
These are isolated experiments. They add no production execution switch or engine cutover.

The [design](sql-composition-opportunities.md) records the sibling survey and integration requirements.
The [evidence](../fixtures/host/data-layer-port/profiling/sql-composition-20261022.json) retains every measured trial, query, plan, source hash, and scope limit.
The baseline is `5616076ad60127d6ac3df0fbf60eff9f7a523ca0` with the existing typed-fact changes.
The engine addon remains `efa0dcf04b170a10588e4661c7843a846989ce98262e68b6bc6abbb6a7dca72f`.

The standalone host imports production packed decoders and SQLite adapters.
Each alternative has six fresh serial trials with rotated order.
CPU means process user plus system CPU.
The main source has 18,993 syntax nodes from the pinned `_conditional_compile.py` capture.
The terminal comparison uses three files and 68,035 nodes.

| Kernel and shape | Kernel CPU, ms | CPU with common setup, ms | Peak process RSS, MiB |
| --- | ---: | ---: | ---: |
| Tree: all-root closure | 267.068 | 282.436 | 9.875 |
| Tree: demanded-root closure | 73.356 | 89.109 | 8.062 |
| Tree: interval join | 0.994 | 17.677 | 6.359 |
| Demand: eager identifier text | 6.120 | 22.110 | 7.477 |
| Demand: selective SQL syntax | 2.943 | 18.458 | 7.438 |
| Demand: materialized keys | 2.267 | 18.768 | 7.539 |
| Utility depth eight: inline tree | 162.835 | 179.339 | 93.078 |
| Utility depth eight: named CTEs | 10.908 | 26.481 | 14.523 |
| Utility depth eight: physical cuts | 5.353 | 20.710 | 6.242 |
| Terminal: scalar functions | 24.780 | 88.933 | 14.234 |
| Terminal: current streaming table | 19.096 | 82.209 | 14.227 |
| Terminal: copied column waves | 20.394 | 84.406 | 17.547 |

Common setup copies captured typed facts and builds the same two indexes for every alternative.
The terminal query does not consume that copy. Its setup-inclusive total is not a production setup estimate.
Database copying, input shape checks, output serialization, and diagnostic EXPLAIN remain separate.
Peak RSS includes the complete parity result and diagnostic preparation.
These values do not compare engine memory with Python linter memory.

### Explanations

The demanded-root control separates root pruning from the interval representation.
Intervals avoid closure rows, deduplication, and two closure indexes.
VM steps decrease from 9,529,897 to 72,996.
The real consumer returns the same 694 root, count, and first-witness rows.
An exhaustive fixture also compares all containment pairs, including root self-membership and root stops.
The input check rejects a graph without contiguous preorder tree semantics.

Selective SQL syntax alone still decodes all 4,612 identifiers.
SQLite evaluates the native predicate before the selective edge joins.
A materialized demand relation reduces text and span decoding to 244 identifiers.
VM steps increase from 41,523 to 45,003, while kernel CPU decreases.
Setup-inclusive totals overlap and do not establish a further complete-pipeline gain over selective SQL.
The real source is clean for this predicate. Fixtures cover a positive, a clean file, and incorrect fields.

The utility stress uses distinct consumers of shared syntax keys.
It is a controlled compiler experiment, not a complete data-layer rule.
From depth four to eight, named SQL grows from 709 to 1,333 bytes.
Preparation CPU still grows from 0.392 to 6.075 ms.
Physical cuts keep preparation at 0.274 and 0.431 ms.
Naming alone does not bound SQLite preparation work.
The existing candidate compiler already supports common expressions and physical cuts.
The proposed extension shares semantic definitions across the whole policy.

The column table uses one cursor and two bounded CPU chunks for three files.
All terminal alternatives read the same 408,210 record bytes and 68,035 flag bytes.
The column table retains per-row SQLite iteration and adds column allocation and writes.
It therefore does not improve the current CPU path.
Future GPU producers still need immutable descriptors, bounded waves, and shared output columns.
CPU chunks do not define GPU launches. Broad count, scan, and scatter passes cover the complete task set at each dependency wave.
This experiment measures no GPU launches or device transfer costs.

Several small timings retain more than 5% variation after matched trials.
The evidence records the full range and coefficient of variation.
Small differences require caution. The closure and utility results have much larger margins.
No whole-rule or full-repository speedup follows from these kernels.

### Checks and next integration

Ten Rust checks cover tree shape, complete pair parity, positive demand, clean demand, utility keys, decoder parity, and batch lifetime.
Two Python checks reject lost positive multiplicity and incomplete execution.
Release Clippy passes with warnings denied.
The profiling suite passes all 59 checks, and all 25 repository structure checks pass.
The complete engine suite was not rerun because the spikes leave production execution unchanged.

The first production change is a typed subtree boundary for eligible authored descendant stages.
That change needs selected-edge semantics, capture identity parity, and positive corpus checks before broader timing.
Demand boundaries and shared utility definitions follow that change.
The copied column prototype remains a rejected CPU alternative in the isolated lab.

## Typed projected facts and SQL input streaming: 2026-10-22

**Decision:** keep typed column storage, SQL column demand, deferred preparation, and bounded SQL import.
These changes remove per-row JSON maps from normal AST projections. They do not establish an end-to-end performance improvement.
SQLite still executes the rules. The engine has one execution path and no new algorithm switches.
The [evidence](../fixtures/host/data-layer-port/profiling/typed-facts-20261022.json) records individual trials, hashes, counters, failures, and reproduction commands.

### Implementation

- [Projected facts](../native/src/code_rule_dsl/violations/projection.rs) share schemas, source slices, and immutable typed column batches.
- SQLite validation records actual column reads, including nested predicates and wildcards. Consumers omit unused projected columns without cloning matcher trees.
- SQL import binds column positions once per schema. It borrows scalar values instead of rebuilding JSON objects or copying each text value.
- The scheduler queues consumer descriptions. It restores file-local inputs during session preparation, rather than preparing every consumer before admission.
- [The input sink](../native/src/code_rule_dsl/relational/inputs.rs) inserts completed column batches and releases them during matching.
  It retains anchor metadata. Cached cross-file evidence also uses typed storage and borrowed SQL bindings.
- SQL input witnesses retain only the capture needed by their anchor. Messages, fixes, and unrelated capture spans are not constructed.

JSON remains the public wire format. Existing JSON-semantic cross-file operations still convert values at their boundary.
Large regex-group enumeration can still construct a full regrouping environment before output.
The new bound applies to projected SQL columns, not every matcher allocation.
Global source-reading clauses still retain source snapshots; this change does not bound every source allocation.
Producer waves and the SQL consumer scheduler retain their existing batched structure.

### Measurements

Both matched binaries use release dependencies, engine optimization level 1, 256 code generation units, no LTO, and profiling support.
Both use the same system-malloc shim, one worker, and 32-file ingestion batches.
Tracing, census, external sampling, and SQL capture are disabled in timing controls.
These RSS results do not establish production jemalloc behavior.
CPU excludes policy compilation; the evidence records compilation CPU separately.
Every command remains below two minutes.

The small state workload reads one actual data-layer file: 108,562 bytes, with zero expected findings.
Positive source tests provide separate rule coverage. Values below are medians of three alternating before/after pairs.

| Measurement | Before | Typed facts and streaming | Change |
| --- | ---: | ---: | ---: |
| Cold process CPU | 3.202 s | 3.228 s | +0.8% |
| Process peak RSS | 395.8 MiB | 461.6 MiB | +16.6% |
| RSS after global evaluation | 288.3 MiB | 222.2 MiB | -66.1 MiB |

CPU is effectively unchanged. Retained resident memory falls, but the process high-water mark increases.
The evidence does not support a memory or speed success claim for the full execution.

A separate ten-rule snapshot reads five files, including required linter metadata: 257,073 bytes and 44 findings.
One observation per version measures 24.695 s before and 23.529 s after in cold CPU.
Peak RSS measures 535.3 MiB before and 544.5 MiB after.
This single pair does not establish a speedup. Both versions match all 44 findings without engine failures.
Missing-file contracts contribute to those findings; the source replay also checks positive cases within actual test inputs.

The matched single-worker Python observations measure 0.264 s CPU for the state file and 1.429 s for the ten-rule snapshot.
Native execution remains approximately 12 and 16 times slower on these selections.
These are small selections, not the full 50-rule repository run or a comparison against its historical 1,400-second result.
The checked [state selection](../fixtures/host/data-layer-port/profiling/typed-facts-state-one.json) and
[ten-rule selection](../fixtures/host/data-layer-port/profiling/typed-facts-ten-four.json) preserve source hashes and snapshot expectations.

### Storage proof and remaining costs

The baseline queues 11,272 input witnesses with 198,758 JSON fields and an estimated 19,624,497 bytes of projection heap.
The final queue holds two consumer descriptions and zero materialized input rows.
The active consumer streams 11,272 rows with at most 256 buffered rows and zero retained projected rows after SQL import.
An intermediate typed census measured 4,208,391 bytes of column storage before the streaming handoff was added.
These counters describe logical storage. They are not exclusive RSS estimates.

Three external sample windows attribute 98–100% of selected native samples to SQLite.
Bottom-up stacks show VM execution, B-tree cursor movement, record comparison, and file reads and writes.
Sample shares are not exact process CPU fractions. External diagnostics are excluded from timing controls.
The vmmap snapshot also shows substantial allocator residency and slack.

New native memory probes record process high-water marks and macOS allocator statistics around selected stages.
They identify large absolute growth during `descendants` materialization and its two indexes.
In that diagnostic, the allocator holds about 266 MiB with about 133 MiB in use after the later index stages.
The large remaining CPU frontier is candidate SQL construction and execution.
The memory frontier includes overlapping SQL tables, indexes, and allocator residency during global evaluation.

The probe identifies where the new execution grows. It does not establish why its peak differs from baseline.
That differential remains unresolved. Replacing the database or adding another execution backend is not justified by these results.

### Validation and exclusions

Validation passes 1,234 native DSL tests, 67 workspace context tests, and 127 other workspace tests.
The runtime suite passes 320 tests. Existing skips remain: 23 native tests and two runtime tests.
Three large Python shortcut corpus tests were excluded from this bounded native run.
The source replay checks ten positive and ten clean cases across all ten rules, through stateless and workspace execution.
There are no false positives, false negatives, engine failures, or execution-path differences.
The instrumented addon also passes that source replay.
Profiling report tests, TypeScript checking, repository checks, formatting, and Clippy checks for production and profiling builds pass.

The original ten-rule twelve-file attempt and two smaller alternatives exceed the 112-second workload deadline.
They cannot establish performance or parity. One state control cannot write its report because the filesystem fills.
That run is excluded. The evidence retains these failures instead of treating incomplete output as a benchmark result.
The first memory-instrumentation build also reaches the deadline; its isolated retry succeeds within the limit.

## SQL preparation and retained-layout experiments: 2026-10-22

**Decision:** keep shared preparation, duplicate-expression removal, and incremental vocabulary matches.
Reject retained expanded postings and generic utility sets for the tested workloads.
SQLite still executes all candidate relations. There are no runtime backend switches.

The base commit is `5616076ad60127d6ac3df0fbf60eff9f7a523ca0`, with the existing value-index work still uncommitted.
The baseline archive captures that complete pre-experiment SQL source and its native binary.
Both rebuilt comparison binaries use release dependencies, engine optimization level 1, 256 code generation units, and no LTO.
Both use the same system-malloc shim, bundled SQLite settings, one worker, and 32-file ingestion requests.
Parser generation now builds at optimization level 3 to fit the bounded build deadline; this changes build-tool execution only.
The [evidence file](../fixtures/host/data-layer-port/profiling/sql-preparation-20261022.json) records binary and source hashes.

Each cell below is the median of three fresh serial runs, with before/after order alternated.
Detailed tracing, SQL capture, census, and external sampling are off during timing runs.
CPU includes ingestion, new preparation, selection, and final matching. It does not move preparation outside the cold timer.

| Workload | Phase | Before CPU | After CPU | Change |
| --- | --- | ---: | ---: | ---: |
| Two rules, twelve files, 61 findings | Cold | 2.476 s | 1.936 s | -21.8% |
| Two rules, twelve files | Edit | 0.543 s | 0.418 s | -23.9% |
| Quality rule, five files | Cold | 3.458 s | 3.268 s | -5.5% |
| Quality rule, five files | Edit | 1.007 s | 0.933 s | -7.3% |

Compile-plus-cold CPU decreases from 2.511 to 1.960 seconds on positive and from 3.707 to 3.513 seconds on quality.
Median peak RSS is 366.6 to 326.0 MiB on positive and 517.0 to 519.3 MiB on quality.
Unchanged requests remain approximately 3 ms and perform no candidate SQL or fact preparation.
All trials have exact findings and zero incremental differences.
Cold CPU variation stays below 5% within each three-trial group.
Peak RSS varies more on this shared host. Individual RSS measurements remain in the evidence; they do not prove a stable memory reduction.
The matched baseline is a fresh rebuild, so its quality CPU differs from the historical 3.192-second run.
The changes miss both proposed cold-CPU targets: 30% on positive and 10% on quality.

### Accepted execution changes

Preparation collects universal structural demands across all 32-plan groups before running them.
Fresh files stream typed tape columns into the retained SQL properties table.
Partly prepared files decode missing keys only. Changed files invalidate their own facts.
Sparse span and child demands retain their dependency order and sparse coverage.
No per-node packed row allocation or five-call scalar extraction remains in universal structural preparation.

The compiler removes equivalent generated expressions before it creates physical temporary tables.
Canonical definitions include typed parameters, child identities, language, column shape, and request dependencies.
Complete definition equality resolves hash collisions. Exactness metadata remains separate from row sharing.
Fixed SQL request tables receive file IDs in batches of 128 rather than altering every generated expression.
Compiled preparation templates survive file edits.
Final candidate SELECTs must be read-only; tests reject writes and check rollback.

Regex matches now persist by language, expression, and vocabulary term.
New predicates check current vocabulary; existing predicates check new terms.
Foreign keys remove matches when the last term occurrence disappears.
Empty completed predicates do not repeat the full vocabulary scan.
A rejected first implementation compiled a dynamic regex per term and increased positive cold CPU to 14.336 seconds.
The corrected callback caches one current predicate per SQL expression, and a regression requires two compilations for 200 evaluations.

### Rejected retention layouts

The standalone replay host imports production SQLite adapters, codecs, connection settings, and the same layout experiment used by native tests.
It compares normal production SQL after duplicate-expression removal, rather than the older uncompacted descriptions.
Each trial rotates layout order and requires exact candidate rows, including multiplicity.
Captures with equal file domains and revisions form one sequence, preserving reuse across real plan batches.

| Workload | Temporary stages | Retained postings | Retained shared sets |
| --- | ---: | ---: | ---: |
| Two rules, twelve files | 1.146 s | 1.501 s (+31.0%) | 1.164 s (+1.6%) |
| Three rules, twelve files | 1.948 s | 2.427 s (+24.6%) | 2.252 s (+15.6%) |

These are median replay thread CPU totals, not complete execution CPU.
They include setup, retained construction, writes, indexes, and consumers.
They exclude parsing, file publication, earlier structural preparation, and final matching.
The three-rule selection adds the quality rule to the positive control and preserves all 61 findings.
Its two file domains each execute two real plan batches.

Retained postings store 303,285 rows on the two-rule control with no reuse.
They store 410,038 rows and reuse 32 constructions across the three-rule batches.
Retained shared sets store 163,552 rows and reuse 38 constructions on that workload.
The reuse does not offset retained writes, wider keys, completion joins, and subsequent reads.
The engine keeps temporary physical boundaries for these sets.
The isolated host retains the experiments for reproduction; the engine has no selectable retention path.

An initial retained key placed revision between file ID and preorder.
SQLite then failed to use the ordinal suffix unless the join supplied revision at the right point.
The corrected primary key is `(relation_id,fid,pre)`, with revision in the payload and a cascading foreign key.
A 2,048-interval regression decreases range work from about 39.9 million to 79,000 VM steps.
Membership work decreases from about 16.9 million to 92,000 VM steps.
Both ordinal bounds and indexed membership are explicit test requirements.
The corrected layouts above still lose; the key correction does not justify promotion.

### Remaining frontier and reproduction

The final positive diagnostic executes 500 candidate data statements, including 480 physical materializations, versus 550 statements before this work.
Structural preparation decodes 340,517 nodes once, costs about 155 ms of diagnostic thread CPU, and performs no source-text extraction.
Selection performs no structural decoding.
Other candidate materializations still cost about 734 ms, temporary index construction 116 ms, and final candidate queries 250 ms.
These instrumented API timings must not be subtracted directly from untraced process CPU.
Separate matched SQL captures reduce total observed VM instructions from 141.6 million to 98.9 million.
Those captures include the new structural preparation statements.
The final ownership diagnostic reserves 16.97 MB in the main database, 12.13 MB in temporary pages, and 9.36 MB in WAL.
SQLite reports approximately 1.77 MB resident for that connection. These bytes do not represent total process memory.
The separate authored relational executor has not acquired a shared relation cache.

The external bottom-up sample still finds SQLite VM execution, B-tree comparison, and file I/O.
It does not identify the Rust structural decoder as the dominant remaining cost.
The next experiments should reduce demanded/intermediate row volume and consumer work before expanding another index or cache.
Their oracle must include full preparation and maintenance CPU.

Commands use `python3 scripts/profiling/run.py --deadline 115`; no command needs the earlier full-run exception.
Timing artifacts are `.rerail/profiles/sql-preparation/matched-{before,after}-{positive,quality}-{1,2,3}`.
Use `--recording` with the pinned selections in the evidence, `--trace off`, and `--allocator system-shim`.
Set `RERAIL_ENGINE_PATH` to the corresponding archived binary.
The [replay host instructions](../scripts/profiling/preparation/README.md) describe the three rotated storage trials.
Diagnostic and external sample artifacts are separate from all timing medians.
Discarded overlapping trials and build timeouts are not timing evidence.

The public workspace still requires rules before evaluation. New-plan reuse tests use the internal corpus API.
No public policy reload, durable cross-process cache, GPU implementation, or full-repository speed claim forms part of these results.

### Validation scope

The final native binary passes 1,223 focused DSL tests, including the new preparation and retention regressions.
The broader native run passes 1,520 tests and fails ten environment-dependent tests; 23 remain ignored.
Nine Git fixture tests use `git add .`, which the local staging guard rejects.
The allocator purge test requires jemalloc controls that the matched benchmark's system-malloc shim does not implement.
No failing SQL, DSL, or workspace assertion appears in that run.
Three separate bounded commands then pass the complete F811, F821, and F841 shortcut-parity fixture corpus.
The complete native run is therefore not all green.

The final addon also passes 320 public API tests, with two skipped tests.
Recorded source replay covers ten positive and ten clean calls, with zero diagnostic or execution-path differences.
The replay host's real-FTS smoke test checks empty sets, bag equality, and reuse.
Profiler tests, typechecking, repository structure checks, Windows path checks, formatting, and Clippy pass.
Clippy covers ordinary and profiling builds, plus the standalone replay host.
The evidence records each command, result, log digest, and parity scope.

## Value postings, vocabulary access, and shared SQL properties: 2026-10-21

The corpus now stores case-exact value postings and a persistent distinct-term vocabulary.
File publication creates the value index beside the positional kind index.
Replacement and deletion update both indexes in the same transaction.
Partition evaluation consumes prepared selections and does not rebuild either index.

The value layout follows the reference's term, posting, vocabulary, and coverage separation.
SQLite B-tree keys retain complete terms instead of using the reference's linear-hash buckets.
Each term, file, and kind has a delta-varint posting segment.
Full term keys avoid hash collisions and their source-confirmation path.
A small virtual table streams these packed ordinals. SQL owns filtering, joins, and matching.

Coverage includes every clean token, including keywords, numbers, and anonymous grammar tokens.
The clean-token limit remains 96 Unicode characters.
Non-clean leaf residue has a corpus-wide limit of 4,096 occurrences per language and kind.
A publication also limits residue text to 384 KiB per kind.
Exact uncovered ordinals retain confirmation for interiors and overflow without scanning every indexed leaf.

Literal predicates seek their terms. Regex predicates test distinct vocabulary terms once per batch.
The compiler shares vocabulary hits and restricts posting segments by known kind domains.
Bare kind domains use value postings directly. Other domains use indexed membership checks.
The kind index remains the reference's contentless positional FTS5 layout.

Shared decoded properties live in ordinary SQL columns, keyed by file and preorder.
Separate tables retain structural properties, span offsets, and first-child field lookups.
Batched fills decode missing keys and reuse the rows across later statements and batches.
File replacement invalidates these rows through foreign keys.
Source text is not retained for interior nodes.

The first implementation passed parity but regressed the positive control from 1.854 to 9.112 cold CPU seconds.
Instrumentation identified an affinity error in the new access path.
INTEGER bounds from decoded properties met temporary ordinal columns with empty affinity.
SQLite consequently omitted ordinal range constraints and repeatedly scanned complete file postings.
Two statements alone executed more than 660 million VM steps.

The compiler now gives materialized numeric columns explicit INTEGER affinity.
An isolated 2,048-interval case decreased from approximately 35,664,900 VM steps to 32,700.
The regression requires both ordinal bounds in the query plan and limits total VM work.
The affinity correction preserves the candidate and finding sets.

Further profiling identified repeated base-posting scans and tiny incremental BLOB reads.
The compiler materializes shared kind postings and enumerates unrestricted ordinals with the standard series virtual table.
Small record and span buffers amortize reads inside batched SQL decoding.
Decoded facts remain in SQL, and these transport buffers do not form another in-memory index.
The engine retains one execution path and introduces no runtime backend switches.

A live all-node property demand covers every subset demand for the same batch.
The compiler hoists that fill before its consumers and records file completeness in SQL.
Later batches skip completed files. Sparse demands retain sparse preparation.
We rejected an experiment that removed source materialization without this coverage proof after execution became substantially slower.

Three serial fresh-process trials per version give these median cold CPU totals:

| Workload | Before | After | Change |
| --- | ---: | ---: | ---: |
| Two rules, twelve files, 61 findings | 1.874 s | 2.504 s | +33.6% |
| Quality rule, five files | 3.004 s | 3.192 s | +6.2% |

Both controls preserve findings, multiplicity, and diagnostics.
The evidence records the matching workload and policy digests.
These complete-pipeline measurements include index publication and property construction.
They establish a remaining CPU regression, not a completed performance improvement.

The positive-control diagnostic retains all 11,763 candidate rows.
Text extractions decrease from 517,994 to 13,739.
Incremental BLOB reads decrease from 2,791,126 to 51,444.
Candidate VM steps increase from 111,766,482 to 130,976,638, despite fewer sorts.
The pipeline executes 550 SQL data statements instead of 72.

Shared property construction, temporary relation copying, and index construction now form the next performance frontier.
The external profiler runs separately with detailed tracing disabled.
Its two-second window contains substantial SQLite VM, comparison, and file-write activity.
Those samples identify execution paths. They are not an exact allocation of process CPU time.
Median recorded peak RSS decreases from 360.7 to 341.0 MiB on the positive control.
Host memory pressure can affect resident measurements, so this comparison does not establish an allocator-memory reduction.

Validation passed 1,404 native DSL/workspace tests, with 22 existing ignored tests.
The public native suites passed 320 tests, with two existing skips.
The source smoke replay passed ten positive and ten clean calls without differences.
Both profiling and production configurations passed Clippy. Repository checks and profiling-tool tests also passed.

Evidence: [value access and decoded properties](../fixtures/host/data-layer-port/profiling/value-access-20261021.json).

## Candidate SQL CPU root cause: 2026-10-21

The expensive path performs much more work than an index lookup.
The kind postings match `ast-index-spike`'s encoding, but the value access path is missing.
Rust retains per-kind value dictionaries and cached text selections.
The reference spike persists value postings in `vidx` and a distinct-term vocabulary in `vlex`.
It evaluates text predicates against vocabulary terms and seeks their postings.
Our SQL predicates instead decode and test individual occurrences.

The reference implements coverage checks and bounded residue handling for indexed values.
Those checks keep incomplete vocabulary coverage from producing false absence results.
The present investigation identifies this missing performance functionality; it does not implement the value index.
The comparison measures complete implementations with different available access paths.
It establishes no inherent database-engine disadvantage.

The positive control runs two data-layer rules over twelve files and retains all 61 findings.
Its candidate pipeline originally executes 164,171,085 SQLite VM steps and 2,174 sorts.
One final statement contains 710 CTEs and executes 536 sorts.
Most generated relations explicitly request materialization.
Repeated kind restrictions create additional set intersections and sorting.
The same low-cost base CTEs also occur in multiple physical statements.

For example, one generated chain computes `s162 = s160 INTERSECT s5`, then intersects that result with `s5` twice more.
The last two intersections add no restriction.
The compiler now reuses identical generated relations and canonical intersections.
It preserves conservative exactness metadata separately from shared row sets.
A regression also corrects subtraction of a set from itself under nested negation.
Another regression preserves deduplication when a union receives projected duplicate rows.

| Candidate work | Before | After SQL cleanup |
| --- | ---: | ---: |
| SQL data statements | 82 | 72 |
| VM steps | 164,171,085 | 111,766,482 |
| Sorts | 2,174 | 1,292 |
| Candidate rows returned | 11,763 | 11,763 |
| BLOB reads | 2,858,925 | 2,791,126 |
| Text extractions | 519,033 | 517,994 |

The cleanup removes approximately 32% of VM steps and 41% of sorts.
Most text decoding remains.
The original reads transfer 102,258,042 bytes through SQLite's BLOB API.
Repeated packed-span blocks account for 82,960,502 bytes.
Source text accounts for 8,637,096 bytes.
The pipeline indexes 340,517 AST nodes once.

New API-boundary timers separate lowering, SQL preparation, materialization, indexing, result execution, and cleanup.
A diagnostic before cleanup attributes 1.919 CPU seconds to materialization and 0.917 seconds to result execution.
SQL preparation takes 0.035 seconds; temporary-index construction takes 0.024 seconds.
Lowering takes another 0.054 seconds.
These diagnostic figures use an engine build at optimization level zero, with unchanged SQLite dependencies.
They describe attribution; production comparisons use optimization level one on both sides.

An external two-second sample captures 1,569 active native samples.
FTS-related frames occur in 29 samples; packed-BLOB decoding occurs in 219.
Sorting and record-comparison paths occur in 278.
The remaining samples include VM dispatch and generic B-tree work.
These mutually exclusive categories describe one sampling window, not an exact process CPU partition.

The existing SQL callback clocks also contain a measurement error.
An internal FTS cursor remains open while other statements execute.
Its accumulated lifetime reports 14.5 CPU seconds during a cold phase that uses only 2.547 CPU seconds.
That lifetime is not an exclusive index-execution cost.
The report now withholds CPU priorities when statement lifetimes cannot reconcile with the apply CPU budget.
Direct API timers provide the execution breakdown instead.

Three fresh production trials compare each implementation on each workload.
These serial runs alternate version order and pass identical workload and policy fingerprint checks.
All trials retain exact parity.

| Workload | Before cold CPU | After cold CPU | CPU change | Before peak RSS | After peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Two rules, twelve positive files | 2.216 s | 1.783 s | -19.5% | 392.44 MiB | 410.00 MiB |
| Quality rule, five files | 2.948 s | 2.984 s | +1.2% | 602.62 MiB | 611.89 MiB |

The quality control does not show a meaningful CPU change.
Neither control establishes a memory improvement.
The positive control confirms a useful SQL cleanup, while its remaining text-decoding work remains substantial.

The `serverplan_target_architecture` port provides a concrete value-index case.
Its AST branches repeatedly match identifier text such as `^(?:step_tags)$`.
The current path reads kind postings and decodes text to evaluate those predicates.
The reference value index can select matching vocabulary terms before locating occurrences.
Porting that access path is the next priority.
Sharing demanded decoded properties across SQL statements is the next remedy for remaining scalar-read amplification.

The final code retains batched preparation and one execution path.
It adds no optional execution backend.
The [root-cause evidence](../fixtures/host/data-layer-port/profiling/index-root-cause-20261021.json) records source identities, reference files, traces, counters, experiments, and validation.

Validation passes 1,202 DSL tests and 194 workspace tests, with 22 existing native skips.
The public suites pass 320 tests, with two existing skips.
The recorded linter source corpus passes ten positive calls and ten clean calls.
Profiling tests, typechecking, repository checks, and Clippy pass.
Several bounded build attempts reach the deadline before a later attempt completes.
The evidence records those attempts separately from completed checks.

## Repeated index comparison: 2026-10-21

Three fresh trials compare each implementation on both bounded workloads.
Each trial runs alone, with identical source bytes, policies, ingestion batches, and allocator settings.
The second round reverses the order; the third rotates it.
The figures report median cold process CPU and median process peak RSS.
Compilation CPU is excluded.
All 24 trials pass parity.

| Implementation | Positive control CPU | Positive peak RSS | Quality control CPU | Quality peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Rust index | 0.462 s | 305.12 MiB | 2.648 s | 497.36 MiB |
| SQL, index rebuilt per partition | 2.339 s | 303.06 MiB | 3.196 s | 496.27 MiB |
| SQL, shared corpus with individual queries | 2.633 s | 301.22 MiB | 3.025 s | 492.66 MiB |
| SQL, current batched candidates | 2.136 s | 399.39 MiB | 2.956 s | 614.05 MiB |

The positive control runs two rules over twelve files and retains all 61 findings.
The quality control runs one rule over five files and expects no findings.
The percentages compare current batched SQL with each reference.
Positive percentages mean more resource use; negative percentages mean less.

| Reference | Positive CPU over/under | Positive RSS over/under | Quality CPU over/under | Quality RSS over/under |
| --- | ---: | ---: | ---: | ---: |
| Rust index | +362.9% | +30.9% | +11.6% | +23.5% |
| SQL, index rebuilt per partition | -8.6% | +31.8% | -7.5% | +23.7% |
| SQL, shared corpus with individual queries | -18.9% | +32.6% | -2.3% | +24.6% |

Current SQL remains substantially more expensive than Rust on the positive control.
Batching reduces CPU against the earlier SQL implementations, but increases memory.
The quality difference against shared-corpus SQL is too small for a strong speedup claim.
These repeated comparisons replace the earlier single-run comparison for these small workloads.

The Rust binary retains its earlier SQLite page-cache compile option.
The comparison covers complete implementations and does not isolate the candidate index alone.
The ten-rule batched control has no completed result within the two-minute cap.
It therefore has no valid total CPU over/under result.

The [comparison evidence](../fixtures/host/data-layer-port/profiling/index-comparison-20261021.json) records every trial, binary identity, fingerprint, metric, and percentage.

## Batch candidate preparation: 2026-10-21

The corpus index previously retained postings but queried them separately for each file and rule.
That query boundary defeated batching.
The engine now prepares candidate sets before matching starts.
Matching and relational partitions consume those sets without issuing candidate SQL.

The preparation API accepts all rule plans together.
Each SQL batch covers up to 32 plans and 16,384 files of one language.
The existing ingestion limits determine each incoming file group.
One workspace corpus retains the prepared sets across partitions and revisions.
A source or grammar change invalidates that file's sets.
An unchanged request issues no candidate SQL.

The index still uses bare positional FTS5 kind tokens and the tape columns from `ast-index-spike`.
Publication uses existing corpus metadata without per-file identity SELECTs.
SQL executes the candidate predicates.
The transport stores empty, complete, sparse, or bitmap selections.
These representations compress results without implementing another rule executor.
The preparation barrier carries parsed documents, line tables, metrics, and work budgets into matching.

Compiler cleanup removes dependencies that discarded rule branches no longer use.
Each materialization statement also contains only its required CTEs.
An interrupted batch publishes no partial absence result and can retry.
Missing prepared results retain authoritative matching and increment an explicit counter.

`anchorBatches` counts preparation batches.
`anchorSqlDataStatements` counts their final SELECTs and temporary-table materializations.
`anchorSqlMaterializations` reports the latter separately.
These counts exclude schema operations and SQLite's internal FTS statements.
`anchorCacheMisses` and `anchorBatchFailures` expose incomplete preparation.
`peakPreparedCandidateBytes` estimates compact result payloads; it excludes maps, documents, and SQLite memory.

A regression compares one file with 64 files under the same rule plans.
Both workloads issue the same number of SQL data statements.
Three repeated consumption passes issue none.
Another regression prepares 65 plans across 64 files in three batches and reuses every result.
Budget, retry, and discarded-dependency tests also pass.

The final measurements and binary identity appear in the [batch evidence](../fixtures/host/data-layer-port/profiling/sql-batch-candidates-20261021.json).
The two-rule positive control contains all twelve source files and retains all 61 expected findings.
It has no false positives, false negatives, or engine failures.
The five-file quality control also checks unchanged and edited revisions.

| Workload | Previous cold CPU | Batched cold CPU | Previous peak RSS | Batched peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Two rules, twelve positive files | 2.620 s | 2.198 s | 298.39 MiB | 376.33 MiB |
| Quality rule, five files | 2.606 s | 3.023 s | 488.83 MiB | 614.84 MiB |

The positive control uses two batches containing 82 SQL data statements, including 80 materializations.
Its relational consumption pass issues zero candidate SQL statements.
Selection uses 1.783 CPU seconds, compared with 2.212 seconds previously.
The quality control uses one batch containing nine data statements.
Both controls report zero failed batches and zero missing prepared selections.

The quality control slows down, and both controls use more peak memory.
Batching therefore does not resolve the overall CPU and memory problem.
An incremental quality run publishes files in counts of five, zero, zero, and one across its four applies.
Its batch counts are one, zero, zero, and one.
The unchanged revision executes no candidate data statements; the edited revision executes nine.
The compact result payload peaks at 47,052 bytes for the positive control and 381,152 bytes for the quality control.

The measured final binary SHA-256 is `03125d5d0097443a55f2825f65074439951f2f295b12f5d07dde020e44ce6646`.

These are single runs on a shared host, with matching workload and policy digests.
Both controls use the same system allocator shim and native build optimization.
The larger ten-rule control reaches the 112-second deadline in the recorded intermediate implementations.
Those timeouts provide no completed CPU result and support no full-workload speedup claim.
The final source differs from the last timeout only in counters and error handling.

The external sample covers an initial batched implementation before dependency pruning.
SQLite accounts for 98.9% of its active native samples.
Sorting, record comparisons, memory copies, and temporary-file writes appear among its expensive operations.
This sample motivates the compiler cleanup; it does not establish the final distribution.

Large SQL batches and broad preparation remain performance problems.
The current inventory prepares the full language-specific plan group across published files.
More selective rule/file demand and intermediate SQL lifetime require further measurement.

Validation passes 1,198 DSL tests, 194 workspace tests, and 320 public tests.
The suites retain 22 native skips and two public skips.
The recorded linter source corpus passes ten positive calls and ten clean calls.
Typechecking, repository checks, profiling tests, and Clippy also pass.
The previously documented Git-fixture and allocator-probe environment limits remain outside these checks.

## Corpus index lifetime correction: 2026-10-21

The previous adapter rebuilt a temporary SQL index for each file partition.
That lifetime defeated index reuse and invalidated the architectural comparison with Rust.
The measurements below remain historical evidence of that rejected adapter.
They do not compare equivalent index lifetimes.

The workspace now owns one file-backed corpus database.
A file's source digest and grammar determine whether its postings need replacement.
Unchanged documents reuse their existing rows, including after tape restoration and document-cache eviction.
Relational preparation publishes missing files before processing partitions.
File removal deletes the corresponding rows.

Workers lease separate connections to the same database.
Each connection has independent temporary SQL tables and a bounded page cache.
WAL permits publication while other workers read coherent snapshots.
The workspace owns the database through the final connection lease.

Kind counts stored by file support replacement and deletion without scanning the corpus vocabulary.
The index reuses the validated tape's source digest.
Its identity check does not export the tape or hash the source.
Unchanged sources also retain their postings through context changes and retries after interrupted rules.
The database survives workspace revisions.
It does not survive process restarts.

`indexedFiles` and `indexedNodes` now count committed publications.
`anchorIndexViews` and `anchorIndexViewNodes` count logical readers, including reused entries.
`sqlIndexWrites`, `sqlIndexWrittenNodes`, and `sqlIndexReuses` make the distinction explicit.
Preparation and selection retain separate CPU timers.

The twelve-file control publishes 12 files and 379,809 nodes once.
Its 76 later partition lookups publish nothing.
The rejected adapter constructed 88 indexes containing 3,846,259 nodes across those same requests.
Preparation falls from 0.677 to 0.062 CPU seconds.

| Workload | Rust cold CPU | Corpus SQL cold CPU | Rust peak RSS | Corpus SQL peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Ten rules, 12 positive files | 32.436 s | 122.420 s | 2.474 GiB | 2.514 GiB |
| Quality rule, five files | 2.631 s | 2.606 s | 497.80 MiB | 488.83 MiB |

The positive control retains all 61 findings without false positives, false negatives, or engine failures.
These are matched single runs on a shared host, with identical policy and workload digests.
Both binaries use the same system allocator shim and build optimization.
The saved Rust binary retains its earlier SQLite page-cache compile option.
The comparison covers the complete branch change.
The five-file CPU difference is too small to support a speedup claim.

A separate five-file incremental probe publishes five files initially, zero for an unchanged request, and one for an edit.
The unchanged request uses 0.002788 CPU seconds.
The edit uses 0.816207 CPU seconds.
Parity remains clean through both requests.

The corrected index lifetime does not resolve expensive SQL selection.
Selection uses 93.322 CPU seconds, against 3.722 seconds in the saved Rust control.
The same run records 1,258 compiler broadenings and 8,990,941 candidate root pairs.
File filtering also has an access-path problem: SQLite's vocabulary table seeks by term, then tests the file identifier.
The bundled SQLite implementation exposes no document constraint in `fts5VocabBestIndexMethod`.

A standalone diagnostic keeps 100 requested postings constant and adds 32 unrelated files.
The query grows from approximately 700 to 160,700 SQLite VM steps while returning the same 100 rows.
That diagnostic uses SQLite 3.54.1.
The engine bundles 3.53.2 and has the same constraint limitation.
It establishes an access-path limitation, rather than attributing the full timing difference to one cause.
Corpus batching and efficient file-constrained posting access remain the next query issues.

Validation passes 194 workspace tests, 1,194 other DSL tests, and 320 public tests.
The native DSL suite retains 22 existing skips.
Public tests retain two.
The recorded source corpus passes ten positive and ten clean calls.
Clippy passes with profiling enabled and disabled.
Typechecking, repository checks, and profiling tests also pass.

Broader native checks pass another 103 tests and encounter ten environment limits.
The local Git safety wrapper blocks nine fixtures that invoke `git add .`.
Its stated reason is that mass staging can include unrelated files.
The allocator shim cannot run the jemalloc arena probe.
These unavailable checks are recorded separately from passing tests.

Workspace tests cover unchanged requests, retries, edits, deletion, and independent workspaces.
SQL tests cover shared connections, snapshot consistency, and cleanup after the final reader.
Broader validation exposed a missing budget poll on empty candidate results.
Selection now observes exhaustion before and after SQL, preserving clause failure and eviction behavior.
A budget fixture contains enough roots to exhaust its allowance after shared selection.

The saved Rust baseline also reproduces an existing Python NFKC false negative.
The semantic name prefilter now retains non-ASCII names for authoritative resolution.
The existing identifier test passes after that correction.

The [corpus evidence](../fixtures/host/data-layer-port/profiling/sql-corpus-index-20261021.json) records source hashes, binary identities, controls, lifecycle counters, validation, and diagnostic bytecode.
The measured binary SHA-256 is `b8347b9f50101a7a9c2a089840c9eb806365d020e5509405c8193734bbea7f46`.
Every command remains below the two-minute limit.
Bounded runs stop after 112 seconds.

## SQLite positional kind anchors: 2026-10-21, after `5616076a`

**Use the positional FTS5 kind index from `ast-index-spike`.**
The implementation follows `bench/tape_store.py::_build_kind_fts` and its thin file-tape columns.
Each file has one contentless FTS5 row.
Each preorder node contributes one bare `k<base36(language * 1024 + kind)>` token, including anonymous nodes.
The FTS5 instance vocabulary returns file identifiers and preorder positions.

The schema enables contentless deletion for file replacement and removal.
It adds no field tokens or persistent row per AST node.

The database uses a real temporary file and a 1 MiB main page-cache target.
Temporary SQL storage uses files, the temporary page-cache target is 512 KiB, and mmap is disabled.
A storage test writes a 4,816,896-byte database.
SQLite reports 1,114,704 bytes for its page cache, schema, and prepared statements after those writes.
These counters exclude source tapes, allocator metadata, operating-system file caches, and query result arrays.
The cache setting is a target, not a limit on total process memory.

SQL selects candidates through kind postings, text predicates, field constraints, and parent relationships.
The existing matcher constructs captures and checks complete rule semantics.
Sibling constraints currently retain conservative candidate sets.

SQL combines language and path filters with candidate selection across indexed files.
An authorization test forbids tape-column reads during kind and path filtering.
Existing repository scope matching retains its current glob semantics.

This rejected adapter created an index for each file partition.
The storage API supported multiple files, replacement, deletion, and reopen.
The correction above replaces that ownership model.

The previous Rust candidate index remains a test oracle and saved benchmark binary.
The production path has no backend selection flag.
SQLite connections use the existing SQL profiler.

### Rejected trials and corrections

The first SQL trial passed the five-file parity probe but repeatedly loaded complete tape BLOBs.
It used 16.005 cold CPU seconds, against 2.631–2.848 seconds for Rust.
An external sample attributed 76.33% of active samples to `pread` and 12.23% to memory copying.
Incremental BLOB reads reduced this trial to 3.867 cold CPU seconds.
That trial used 468.73 MiB peak RSS, against 497.16–497.80 MiB for Rust.

The positive workload then exposed SQLite's CTE reference expansion during preparation.
The trial logged 178 table-reference-limit errors and reached the 112-second harness deadline.
It is not a passing performance result.
The compiler now places SQL temporary tables at shared dependencies with large expansion estimates.
These physical boundaries prevent repeated dependency expansion.

Temporary indexes support file and preorder lookups within those candidate sets.
Small dependencies remain in one SQL statement.
Ancestor selection also shares ancestor edges across matching descendants.

Incremental readers close before SQL schema changes and file updates.
Tests cover overflow records, span checkpoints, stale cursors, and cancellation.
A node-text lookup reads fewer than 256 bytes from a file with more than 65,535 nodes.
A dependency regression executes a 20-level shared graph twice and checks temporary-table cleanup.

A second positive trial avoided the CTE reference errors but still reached the harness deadline.
An external sample attributed 44.84% of active samples to mutex wakeups during SQLite page-cache access.
The bundled `SQLITE_ENABLE_MEMORY_MANAGEMENT` option forces all connections into one shared page-cache group.
The build now disables that option and retains separate bounded page caches.
Workspace eviction already closes idle content-store connections and releases their pages.
The obsolete global page-cache release call is removed.

The BLOB-handle map also uses the engine's existing fast hasher.

Direct field relations now select their structural context before inner text and pattern predicates.
The child accessor skips subtrees in the stored tape and returns the first requested field edge.
It retains no Rust posting index.
This follows the spike's child-stepping approach and the prior Rust selector's anchor discipline.
A node-budget regression also preserves exhaustion when the index refuses an oversized file.
The two public exhaustion tests pass after that correction.

The next positive trial still exceeded the deadline during candidate selection.
Pattern plans repeatedly scanned postings for each candidate root.
Their anchor estimates also counted kinds instead of the selected literal postings.
Pattern plans now use actual posting counts and indexed SQL temporary tables for subtree probes.
A materialized span relation computes each candidate's subtree end once.
A regression bounds SQLite VM steps for 2,001 similar calls with one rare target.

The remaining ancestor walks still repeated work for deeply nested AST nodes.
AST subtree ordinals form intervals, so descendant predicates now use indexed SQL range queries.
Inside predicates remove nested intervals with a SQL window before probing candidate positions.
Field predicates use the first field child's interval.
Direct parent predicates retain one parent lookup.
General graph reachability retains its existing recursive SQL.

A second VM-step regression covers 200 nested calls, including field relations.

### Rejected lifetime comparison and query observations

The first completed positive control retains all 61 expected findings and reports no engine failures.
Its cold CPU time is 113.949 seconds, against 32.436 seconds for the saved Rust build.
Peak RSS is 2.507 GiB, against 2.474 GiB for Rust.
These controls use identical source bytes, policies, ingestion batches, build optimization, and allocator settings.
The source selection contains 12 files and ten rules.
They are single runs on a shared host.

The final rebuild retains the same algorithm and adds profiling counters and configuration guards.
Its matched controls are:

| Workload | Rust cold CPU | SQL cold CPU | Rust peak RSS | SQL peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Ten rules, 12 positive files | 32.436 s | 105.663 s | 2.474 GiB | 2.546 GiB |
| Quality rule, five files | 2.631 s | 3.113 s | 497.80 MiB | 486.13 MiB |

The positive control retains all 61 findings without false positives, false negatives, or engine failures.
An earlier Rust five-file control used 2.848 CPU seconds.
Both controls finish within the two-minute command limit.
The SQL build also disables SQLite's global page-cache group, while the saved Rust control retains it.
The comparison covers the complete branch change and does not isolate index storage alone.
The [evidence](../fixtures/host/data-layer-port/profiling/sql-ast-index-20261021.json) records binary identities, source hashes, rejected trials, external samples, and passing checks.

This SQL implementation is a storage and comparison baseline, not a performance improvement.
Candidate selection consumes 81.422 CPU seconds, against 3.722 seconds for Rust.
Initial ingestion selects 5,176,955 root pairs, against 1,604,003 for Rust.

The next frontier is SQL candidate-plan construction, shared materialization, and lost selectivity.
The profiler now counts compiler plan broadenings separately from incomplete indexes and query fallbacks.
The positive control records 1,258 compiler broadenings.

The compiler currently broadens plans that exceed its CTE bound.
This behavior preserves findings but weakens filtering.

Validation includes 86 native tests and 320 public tests, with two existing skips.
The recorded source corpus includes ten positive cases and ten clean cases.
These cases preserve findings and execution-path parity.
Clippy passes with profiling enabled and disabled.
Typechecking and repository checks also pass.

The existing projected-row queue remains unchanged.
Moving the kind index to disk does not remove that queue.
Persistent corpus indexing, SQL scope integration, and the spike's later value-posting indexes remain separate work.

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
