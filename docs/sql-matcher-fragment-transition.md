# Batched matcher fragments

Current status: the public SQL engine uses typed matcher fragments on `jeremy/sql-capture-demand-spikes`.
The qualified full-workload result is 46 matching executions and 2.1% CPU overhead against native, with complete finding parity.
See **Qualified integration result** for the gates, evidence, and remaining scope.
Earlier journal entries describe the preceding spike and its superseded limits.

## Decision, 2026-10-25

The generic SQL state machine is a rejected compiler architecture.
SQL must compose typed relations with joins, exclusions, aggregation, and structural closure.
Coherent matching operations remain inside batched native matcher fragments.
The compiler must not convert ordinary composition into program counters, call stacks, or continuation rows.

This decision supersedes the shared-function scheduler in the earlier transition plan.
The qualification harness, native oracle, and recorded performance ceilings remain authoritative evidence.
Old coverage reports describe the old executable. They do not qualify this replacement.

## Evidence and scope

Before commit `049c5873`, eleven catalog grammars remained refused.
Five exceeded compiler operator limits, including B615 and SIM905.
The exact clause names are `b615`, `duplicate-isinstance-call`, `enumerate-for-loop`, `multiple-starts-ends-with`, and `split-static-string`.
Six lacked recursive context contracts: B020, F405, `is-literal`, and the three forward-annotation clauses.
These were SQL compiler limitations. The native matcher already executes these constructs.

The wrong decision introduced shared instructions and return continuations to solve expansion and semantic problems together.
The replacement separates those problems. Relational factoring addresses duplication. Matcher fragments own sequential semantics.

## Existing capabilities

`AstMatcher` supports patterns, ordered alternatives, captures, searches, recursive utilities, parameterized calls, and semantic predicates.
`run_clause` preserves root order and utility allowances within one file.
`enumerate` implements repeated capture enumeration through exclusion and rematching.
The pattern virtual table already accepts a complete input set through SQLite's IN constraint interface.
Its cursor advances over completed output. It does not ask SQL to execute each matcher instruction.

The integration worktree has a typed producer registry with ordering, multiplicity, identity, and completion contracts.
The spike must inform that interface. It does not introduce another production registry.

## Fragment contract

A compiled fragment retains its matcher, utility definitions, capture schema, and context requirements.
Demand contains a request identity, document identity, root node, and required input captures.
Output contains completion, an explicit outcome, match order, and typed exported captures.
False, unknown, incomplete, and failed outcomes remain distinct.
Lists and parsed spans retain document identity and native capture semantics.

One filter invocation consumes an operator's whole demand batch.
Native code evaluates roots within each file and context. SQLite receives completed output only.
Native code never asks SQL to evaluate a child matcher or utility.
File failure discards provisional output for that file, without discarding successful files.

Fragment boundaries follow semantics, not individual terminal nodes.
Keep capture rollback, recursive utility calls, and ordered searches inside a fragment when they share state.
SQL composes independently useful match relations. A whole-rule native control alone does not prove this composition.

## Open semantics

Exact work charges, cache-fill accounting, depth peaks, and clause-wide memo history can constrain fragment boundaries.
The initial native binding preserves them within a complete execution region.
Splitting that region requires an explicit session contract or a separate semantic decision.
The spike must not claim exact accounting across independently executed fragments.

Each logical contract needs a pure-SQL binding under the agreed architecture.
The native spike does not establish that binding for every fragment.
Unresolved SQL bindings remain explicit. A generic SQL interpreter is not an acceptable implementation.

## Planning and measurement

Production planning reads kind and edge statistics from SQL-held indexes or aggregate tables.
Compiled output uses a document digest and compiler version, with recorded statistics dependencies.
Preparation and compilation are separate from warm execution.
Sharing requires compatible semantic inputs and candidate scope, with measured reuse.

Acceptance requires native parity, boundary invocation counts, intermediate cardinalities, and CPU and wall time.
VM counts remain diagnostics. Native matcher work must appear in timing even when SQLite executes few instructions.
Full repository and per-edit comparisons remain required before production adoption.

## Journal

### Start

Read the matcher, terminal batching, catalog refusal evidence, and integration producer contracts.
Preserved unfinished interpreter optimizations in `/tmp/rerail-before-fragment-transition/` before removal.
The spike starts from this branch. It is not a production cutover or a completed rebase onto the integration worktree.
The integration worktree contains substantial staged work and remains untouched.

The implementation will remove generic scheduler emitters, then test batched fragments against all eleven original refusals.
Independent composition and capture-bearing demand controls will distinguish a fragment interface from a whole-rule wrapper.

### First boundary experiment

Removed the generic finite, captured-recursion, parsed-recursion, and pure instruction-loop SQL emitters.
Removed the scheduler-only search, binding, and projection adapters.
Historical control IR remains for diagnostics. Its emission entry points return explicit refusals.
The compiler no longer selects shared-function interpreter alternatives after a size refusal.

Added a native fragment table with one batch filter, typed captures, explicit unknown outcomes, and file-isolated failure publication.
The composition control passes: SQL joins two fragment relations, excludes matches, and aggregates their outputs.
The first B615 run preserves native output but fails the work-accounting comparison.
The experiment reused scope caches from the preceding native run, which invalidated the accounting and timing comparison.
The corrected harness starts both executions with fresh scope caches.

The native baseline now groups request keys once, rather than scanning every request again for each file.
That correction prevents benchmark overhead from inflating the native time.
False results complete their demand without producing SQL rows. Unknown and failed outcomes remain explicit.
The enumeration control uses native exclusion and rematching inside one batch, with an explicit output ordinal.
Enumeration with prebound capture inputs remains refused until its exclusion contract is defined.

### Qualified fragment boundary

The corrected run passes five focused tests, including all eleven original refusals across 597 fixture files.
Every grammar preserves native matches, exported captures, and remaining work within its file execution region.
The run covers 196,678 candidate roots and returns 226 matches.
Each grammar enters its native table once. No matcher instruction crosses back into SQL.

The sum of per-grammar median execution CPU is 232.1 ms native and 240.5 ms through SQL.
SIM905 takes 43.6 ms native and 44.0 ms through SQL, with 1,245 SQLite VM instructions.
These figures use three paired executions in an opt-level-1 test build.
They exclude common tape setup and preparation. They are not production or full-repository performance claims.

The next measurement runs all eleven grammars as one job over the same files.
It includes fresh tape parsing, candidate discovery, batch setup, preparation, result assembly, and teardown.
The native control processes files and rules directly, without constructing SQL node facts.
The SQL job preserves separate candidate scopes and combines only final outputs in one statement.
Policy compilation is reported separately.

The spike does not reopen the old pure-SQL catalog qualification gate.
The native fragment binding is explicit in the evidence, and no production selector or opt-in flag is added.

### Combined job result

The final run passes all five focused tests, including the combined eleven-grammar job.
The job covers all 597 fixture files and preserves 226 native matching results.
Each native fragment receives one batch. The job executes one matching SQL statement.
Final publication uses 1,625 SQL bytes and 11,873 SQLite VM instructions.
The producers keep separate candidate scopes and narrow schemas. Only final publication has a common union schema.

Three alternating-order cold runs give these medians:

| Measurement | Native | Native fragments through SQL |
| --- | ---: | ---: |
| Complete job CPU | 288.2 ms | 287.5 ms |
| Complete job wall time | 828.9 ms | 761.9 ms |
| Batch setup CPU, including parsing | Not separately instrumented | 84.7 ms |
| SQL preparation CPU | Not applicable | 0.175 ms |
| Matching and result decoding CPU | Not separately instrumented | 200.6 ms |

The CPU difference does not establish a speed advantage. The spike runs at approximately native cost on this workload.
Wall time includes shared-host scheduling noise. VM counts describe SQLite work only, not native matcher work.
The complete job includes parsing, candidate discovery, setup, preparation, matching, result decoding, and teardown.
Policy compilation is separate and recorded per policy.

The [evidence](../benchmark/sql-matcher-fragment-spike.json) includes every run, boundary counters, EXPLAIN output, fixture hashes, and source hashes.
The [combined SQL](../benchmark/sql-matcher-fragment-spike.sql) shows the complete publication statement.

### Remaining transition work after the first spike

This is a native-binding spike, not a restored pure-SQL compiler qualification or a production cutover.
The ordinary compiler retains explicit refusals for cases that required the removed interpreters.
Legacy scheduler-specific tests no longer describe a supported emitter. Their old ceilings and evidence remain unchanged.
The focused checks retain native result, capture, work, unknown, enumeration, and failure controls for the new boundary.

The eleven-case test uses complete AST matching regions. A separate control proves SQL joins, exclusion, and aggregation across independent fragments.
Automatic fragment partitioning and production integration with the shared producer registry remain separate work.
Capture-bearing requests enter through immutable batch data. General SQL-produced capture demands still need a relation-handle contract.
The pure-SQL bindings, cross-fragment accounting, full-repository CPU and RSS, and per-edit latency remain unqualified.
The integration worktree and its staged changes remain untouched.

### Reproduction

Build the test executable with the recorded profile and allocator settings.
Run this focused test filter:

```sh
RUST_MIN_STACK=67108864 \
RERAIL_FRAGMENT_EVIDENCE=/tmp/sql-matcher-fragment-spike.json \
  /path/to/test-executable fragment_spike_ --nocapture --test-threads=1
```

The test refuses missing positive witnesses, output differences, work differences, repeated batch invocation, or extra matching statements.

### Integration work, 2026-10-25

The direction document now resolves the earlier semantic question.
Findings form a set. Budget exhaustion means `Unknown` for the affected rule and file.
Exact work charges and witness order remain diagnostic controls, not requirements for relational composition.

Consulted the direction document, findings contract, and prior-work indexes before this transition.
The materialization decisions follow ast-index-spike S053, S129, S148–S149, and S155.
Demand carries node identities and typed captures. It never carries a tape or serialized matcher state.
Only an aggregate handle crosses from SQL into a matcher fragment.
The aggregate builds immutable demand, including an explicit empty demand.
Expired handles fail. A batch clears its handles on success and failure.

Enumeration now starts each rematch from the original input captures.
The exclusion set remains native. Output captures from an earlier match do not become inputs to the next match.
Node, list, count, and exported-span inputs retain their typed transport.

The integration replaces the public SQL compiler and session entry points on this branch.
The old frame executor remains available to historical tests during qualification.
It is not an alternative production selection.
The compiler keeps recursive utilities, capture dependencies, and ordered searches inside coherent matcher regions.
Independent structural predicates lower to ordinary SQL over the node relation.
The same structural contract also has a native binding for differential checks.

The session prepares bounded publication statements once and replaces batch slots between executions.
The statement split follows SQLite compound limits, not the number of files or candidate roots.
Compilation uses a bounded cache keyed by the query digest, compiler version, and enumeration contract.
Cached matcher fragments retain their utility definitions, including weak-reference targets.
No producer sharing is assumed from a common query digest. Each clause retains its requested candidate scope.

The shared producer registry comes from the integration worktree's existing implementation.
This branch does not add a second registry. The integration worktree remains untouched.
All results in this section remain unqualified until the focused controls and workload comparison finish.

The first integration run passes seven focused checks.
These include the eleven original refusals, SQL-produced capture demand, bound enumeration, and prepared-session reuse.
The combined refusal workload remains near native CPU cost.
This run precedes the complete catalog and workspace gates.

Production planning now reads requested kind counts through `fts5vocab` over the existing content-store index.
The lookup is one SQL statement over the requested terms. It does not recount a batch in Rust.
Numeric kind tokens overlap across dialects, so these store-wide counts are cost estimates, not exact language frequencies.
The planner uses them only to choose between equivalent structural root and descendant anchors.
A session rechecks the recorded dependencies. It replaces prepared SQL only when the anchor choice changes.
Edge-frequency planning remains unnecessary for this restricted structural family.

The integrated workspace passes each file's suspended budget into the native batch boundary.
Utility allowances remain file-local. Deadline and cancellation outcomes become explicit file failures.
The end-to-end harness uses separate processes for native and SQL, so each peak RSS measurement has its own process lifetime.
It measures the initial job and three single-file edits. Compilation is separate.
The original twelve external policies remain unavailable at their recorded path.
The local full-repository gate therefore uses the shipped Python catalog, with that workload named explicitly.

The structural binding checks pass for descendant and ancestor search, negation, sibling search, and counts.
The two statistics-selected anchor shapes return the same results.
File-local utility exhaustion discards that file's provisional matches. Another file and the next session run still succeed.

Two qualification-harness defects stopped acceptance of their runs.
The catalog runner compared a display name with `python` and selected no grammars.
It now compares the dialect enum and refuses an empty catalog.
The workspace runner read findings outside the API response's `evaluation` envelope.
It now checks that envelope before it records findings, and saves a cold-run checkpoint before edit measurements.
No timing from either failed harness run qualifies the integration.

The complete catalog run covers 146 grammars and 597 files.
Its first integration result reports 1,028 ms native CPU and 1,164 ms SQL CPU, with no differences on compared outputs.
It also exposes a completion bug: the adapter maps three-valued matcher results to incomplete file coverage.
A logical unknown is a completed matcher verdict. It is not resource exhaustion.
The corrected publication retains true findings and resource failures, while logical unknowns remain available inside relation composition.
The regression check requires a known match beside a logical unknown to survive with complete coverage.
The catalog gate now rejects an increase in incomplete coverage on this unlimited-work control.

The corrected catalog compares all 87,162 rule/file pairs. Neither engine reports incomplete coverage.
The first complete workspace comparison uses 5,725 tracked Python files and 95,697,853 source bytes.
Both engines return the same 97,798 finding identities, with no failures, including three subsequent single-file edits.
SQL executes eight matching statements across four worker partitions.
Initial-job CPU is 113.90 seconds SQL versus 109.74 seconds native. Compilation is measured separately.
Peak RSS is 2.95 GB SQL versus 603 MB native. This memory regression prevents acceptance of that implementation.

The publication query has an unnecessary global `ORDER BY` after `UNION ALL`.
SQLite merges the arms and can retain many producer cursors at once.
Candidate cursors also retain their request vectors until statement reset, even after exhaustion.
The next implementation removes the global merge and releases exhausted cursor buffers.
The adapter sorts each file's final matches by root, preserving enumeration order for equal roots.
This changes publication storage, not matcher semantics. The full workspace gate must measure its effect.

The hard-case suite passes all eleven checks through the public compiler.
Former refusals now use native fragment semantics, including a typed depth failure for a recursive loop.
The three-valued suite retains its semantic checks. Its old fact-table invocation assertion no longer describes the production boundary.
The fragment suite checks invocation counts at the new boundary instead.

The memory fix also bounds unbound request construction to one file inside each batch invocation.
This does not split SQL execution. One virtual-table filter still consumes the whole requested batch.
Instrumentation reports peak request rows separately from total roots.
SQL-produced demand handles transfer ownership to their consumer and reject a second consumption.
This releases each intermediate request relation after matching, before the whole job ends.

The edit gate now adds one finding, adds another, and restores the original source.
It checks changes and retractions, complete coverage, and native parity after each edit.

The merge-removal rerun does not reduce peak RSS materially: it remains about 3 GB.
Do not attribute the memory regression to the publication sort.
Allocator snapshots then identify 760,717,400 bytes of restored navigation data across the four partitions.
Releasing parked scopes frees roughly 800 MB across those partitions.
These snapshots are process-global; navigation bytes are exact per-partition counts.
The principal problem is retaining every file's decoded tape and semantic caches together.

The workspace now targets 4 MiB of source per resident partition and reuses the prepared publication plans.
The remaining matching-execution budget caps the partition count. It is never a per-file SQL loop.
One atomic budget covers the apply's producer and consumer passes. The 101st execution is refused before SQL runs.
The producer pass reserves half the available budget when a consumer pass will follow.
The source target is a planning estimate, not a strict memory bound. An unusually large file can exceed it.
An evicted source receives a conservative size estimate until its stored contents are restored.
The full workload gate must validate memory, CPU, coverage, and statement count together.

### Qualified integration result

The resident-partition implementation passes the full workload gate.
The workload contains 146 Python AST grammars, 5,725 files, and 95,697,853 source bytes.
Both engines return 97,798 finding identities with no failures.
All three edits agree. Adding findings changes the result; restoring the source retracts them.

| Measurement | Native | SQL fragments |
| --- | ---: | ---: |
| Initial-job CPU | 109.53 s | 111.79 s |
| Initial-job wall time | 107.49 s | 60.27 s |
| Initial-job peak RSS | 684 MB | 1,233 MB |
| Median edit CPU | 180.10 ms | 202.62 ms |
| Matching SQL executions | — | 46 |

These are single-run measurements from the same optimized test build: optimization level 1, real jemalloc, and local test LTO disabled.
The development host had other activity. CPU is the comparison gate; wall time is recorded without claiming an isolated speedup.
Compilation and preparation appear separately in the evidence. Preparation remains cached across partitions and edits.
The peak-memory result improves from 2,933 MB before resident partitioning to 1,233 MB afterward.
The source target produces 23 partitions. Each runs the same two prepared publication statements.

The [qualification checker](../benchmark/check-matcher-fragment-integration.py) enforces explicit ceilings:
100 matching executions, 1.30 times native initial CPU, 1.50 times native median edit CPU, and 1.5 GiB initial peak RSS.
It also requires complete coverage, finding parity, positive edit controls, and complete partition traces.
The executable regression test runs 100 matching statements, refuses statement 101, and succeeds after the next job resets the budget.

The [integration evidence](../benchmark/sql-matcher-fragment-integration.json) records source and binary hashes, input identity, ceilings, and timings.
The [catalog trace](../benchmark/sql-matcher-fragment-integration-catalog.json.gz) retains SQL, EXPLAIN plans, boundary counters, and every comparison result.
The [generated SQL](../benchmark/sql-matcher-fragment-integration.sql) contains the two publication statements.
The catalog compares all 87,162 rule/file pairs and 2,477 raw matches, with no capture differences or incomplete outcomes.

Focused validation passes 16 integration checks, 11 hard cases, 10 three-valued cases, and four registry checks.
Recorded differential cases pass. A 100-case generated run also checks 64 binding-recursion cases, with no utility-limit exclusions.
A workspace regression confirms that changing a dependency retracts an unchanged caller's finding, then restores it.
The ordinary library build passes without test-only features.
Historical frame-compiler tests and old performance artifacts remain separate evidence. They do not qualify the new public compiler.

The remaining matching cost is mainly native execution: boundary instrumentation records 79.50 seconds of native CPU.
The largest measured fragments are ASYNC221, B023, F405, B615, and B007.
The trace records 73,275,953 SQLite VM instructions. They are diagnostics, not a substitute for end-to-end CPU measurements.
There is no SQL instruction driver in this execution path.

### Scope after integration

The public SQL compiler and both stateless and workspace adapters use the new boundary on this branch.
There is one SQL compiler selection. Structural predicates use SQL; coherent sequential fragments use native matcher bindings.
The shared registry, immutable capture demand, enumeration, resource failures, cached compilation, prepared sessions, and SQL-held statistics are integrated.
Pure-SQL bindings cover the structural subset. Full pure-SQL implementations of sequential contracts remain unresolved.
The original twelve translated policies are still unavailable; these measurements use the shipped Python catalog explicitly.
The integration worktree's staged changes remain untouched. This work has not been rebased onto current main.

Reproduce the workload with the ignored `fragment_integration_workspace_job` test and `RERAIL_FRAGMENT_CORPUS` set to the recorded corpus revision.
Set `RERAIL_FRAGMENT_E2E_DIRECTORY` to an empty output directory. The test starts one subprocess per engine.
Generate catalog evidence with `fragment_integration_catalog_job` and `RERAIL_FRAGMENT_INTEGRATION_EVIDENCE`.
Run the qualification checker with those catalog, native, SQL, corpus, and executable paths.

The final review also runs the existing SQL suite against the new public entry points.
It finds thirteen stale assertions about rejected recursion, frame advances, packed probes, or SQL phase drivers.
Their semantic controls remain. Their physical assertions now check whole-batch native invocation, confirmed outputs, and absence of SQL matcher-frame traversal.
A count threshold provides a concrete regression case: native completes after proving its lower bound, while the old SQL test demanded rejection.
Recursive loops that cannot short-circuit now compare native resource-failure outcomes instead of expecting compiler refusals.
These final changes affect tests only. The workload measurements above retain their original executable and source hashes.

One migrated recursion case differs only in an attempted-witness metric after resource exhaustion.
Native records one attempt; the batch boundary discards provisional output and records zero rendered witnesses.
Final findings, coverage, and the resource-failure reason agree.
That test compares these outcomes and excludes execution metrics. It does not restore discarded-work accounting as a semantic requirement.

The four sessions prepare eight statement instances once, using 6.32 ms of preparation CPU in the measured job.
Compiler lowering and final assembly use 2.87 ms of CPU; policy compilation is recorded separately.
Matching uses 83.56 seconds of CPU, including 79.50 seconds inside native boundaries and 4.06 seconds in SQL and boundary overhead.

Final SQL-suite result: 176 passed, zero failed, and 23 explicitly ignored external or diagnostic cases.
The focused fragment checks also pass all 16 cases. Formatting and the production library check pass.
The evidence separates the workload executable from the final test executable and records the later test-only source changes.

## Joint SQL and native batch audit

The integration commit `c17d2e47` is published on `jeremy/sql-capture-demand-spikes` for review.
The subsequent investigation examines repeated native work and SQL admission together.
The [audit and implementation plan](sql-matcher-batch-optimization.md) records the findings, contracts, and remaining experiments.

The full integration trace contains 73,199,829 native root evaluations across 3,289 fragment filters.
The compiler batches statement publication, but each native virtual table still owns one clause.
Of 146 clauses, 129 have no SQL prefilter; 14 combine SQL predicates with native matching.
Matching CPU is 83.56 seconds, including 79.50 seconds within native boundaries.
The trace's planner statistics are empty. It does not measure statistics-driven planning.

A new ignored diagnostic compares memo lifetime, shared async prefixes, and individual SQL arms on the 597 catalog fixtures.
It uses three alternating pairs for the memo and producer comparisons, with preparation outside the producer timers.
Every comparison checks nodes, captures, order, and resource-failure outcomes.

Keeping the native file memo across clauses reduces median catalog CPU from 1,286.61 ms to 1,019.49 ms.
The native matcher already supports stable cross-rule utility identities. The SQL fragment's memo lifetime prevents that reuse.
This experiment changes memo lifetime within the native file-first runner. It is not a production SQL speed measurement.

Six async rules have the same capture-free prefix and exactly equal candidate domains on these fixtures.
One shared prefix admits 492 of 24,151 nodes. Six residual matchers then process 2,952 roots.
Total root evaluations fall from 144,906 to 27,103. Median CPU falls from 156.69 ms to 29.42 ms.
The timer includes producer execution, demand construction, and consumer execution.
Shared input-domain construction precedes the repetitions. Its production cost still needs a whole-job measurement.
All results agree, with no resource failures. Full-workload domains differ, so this gain cannot be extrapolated directly.

Five SQL predicates are provably implied by their anchor kind sets, including compound and negated kind expressions.
Removing them in the diagnostic reduces their combined VM count from 806,803 to 1,529.
B028 falls from 362,379 VM instructions to 91, but sampled CPU changes only from 33.20 ms to 32.59 ms.
This result reinforces the need to measure CPU alongside VM instructions.

F707 consumes the most VM instructions among isolated SQL arms on these fixtures: 1,707,968.
A handler-first relation is the next shape to measure. Efficient candidate-domain intersection is a missing virtual-table capability.
Existing ancestor and sibling joins already use native navigation paths.

The recommended implementation combines a multi-rule native boundary, file-local memo reuse, shared selective producers, and SQL-owned planning statistics.
It preserves typed outcomes and native sequential semantics. No SQL interpreter is needed.
The diagnostic does not alter production execution or add runtime feature switches.
The existing whole-job CPU, memory, parity, and 100-statement gates remain unchanged.

The [summary](../benchmark/sql-matcher-batch-audit.json) separates full-job evidence from catalog experiments.
The [compressed trace](../benchmark/sql-matcher-batch-audit.json.gz) retains measured plans, metrics, source hashes, and the diagnostic executable hash.
Run `fragment_batch_optimization_audit` with `--ignored` and `RERAIL_FRAGMENT_BATCH_AUDIT` set to its output path.
The diagnostic passes. The existing catalog integration check also compares all 87,162 rule/file pairs against native.

## Reusable batch implementation: compatibility review

The implementation follows the sibling `Pulling-it-all-together` worktree's producer contracts and fusion specification.
That worktree contains substantial staged work. This change reads its current files and leaves them untouched.
Its `architecture.md` assigns this branch's transport to `engine/vtab`, planning to `engine/plan`, and publication SQL to `engine/compose`.
Its `spec-fusion.md` requires recursive prefix sharing within identical candidate scopes.
Its shared-producer contract compares complete definitions and keeps the registry separate from execution.

The reusable prefix transformation lives in `producers/prefix.rs` and accepts arbitrary ordered semantic keys.
It constructs shared stages through nested divergences and retains each consumer's residual offset.
The matcher adapter in `matcher/factor.rs` proves closed syntactic predicates and retains complete transitive utility definitions.
It excludes capture inputs, capture outputs, semantic operations, parameterized calls, and enumeration from extraction.
The SQL adapter owns demand handles, typed output columns, and batch invocation. It contains no catalog rule names.

The native batch groups actual ordered root domains by complete equality, dialect, and resource options.
It processes files individually, retaining one native memo across compatible clauses.
Stable compiled matcher objects remain alive throughout that memo's lifetime, because native caches can use object addresses as keys.
Shared-prefix work reduces each consumer's remaining allowance. Failures remain explicit and local to the affected file.
Semantic caches are released after the batch's consumers finish that file.

SQL generation now removes anchor-implied kind predicates and supports candidate-domain point lookup and file enumeration.
Selective parent plans can therefore intersect their results with actual requested roots without materializing the entire candidate domain.
Bounded count predicates stop at their decisive threshold.
The compiler version changes so earlier templates cannot enter the new execution path.

Qualification is in progress. The following journal entries will record measured results and any changes required by failures.

### First batch measurement and correction

The first integrated batch passes all 87,162 catalog comparisons and the full workload's finding and edit comparisons.
It reduces matching statements from 46 to 23 and VM instructions from 73,275,953 to 25,216,404.
Its whole-job SQL CPU is 109.49 seconds. This first run has no new paired native baseline.

Instrumentation exposes 4,429,551 successful shared-prefix rows.
Most come from direct kind predicates already guaranteed by candidate admission.
This is unnecessary materialization, not useful reuse.
The corrected matcher adapter removes those direct predicates before extracting prefixes.
The corrected batch also stops cloning cumulative metric maps for every file.
Candidate membership now indexes files and binary-searches sorted root domains instead of repeatedly scanning them.

The revised trace records each shared producer's terms, consumers, roots, rows, CPU, and files with work.
It also records native memo hits and misses.
Per-rule CPU alone can misattribute shared setup: retaining a file cache moves its first construction cost between consumers.
The qualification checker therefore adds shared-prefix CPU to residual CPU and reports both separately.

### Qualified reusable batch result

The revised implementation passes the unchanged full-workload gate.
Both engines return 97,798 finding identities across 5,725 Python files and 146 AST grammars.
Neither engine reports incomplete coverage. All three edit results agree.
The catalog comparison also preserves all 2,477 raw matches and captures across 87,162 rule/file pairs.

| Measurement | Fresh native run | Revised SQL batch |
| --- | ---: | ---: |
| Initial-job CPU | 115.63 s | 110.24 s |
| Initial-job wall time | 79.50 s | 69.00 s |
| Initial-job peak RSS | 903 MB | 1,261 MB |
| Matching executions | — | 23 |
| Prepared statement instances | — | 4 |

SQL CPU is 0.9534 times native CPU; median edit CPU is 1.0644 times native.
The host runs other development work. These are paired single-run measurements, not isolated speed estimates.
The earlier SQL integration measured 111.79 seconds, so the end-to-end gain remains modest despite the larger VM reduction.
No performance ceiling was relaxed.

Matching consumes 82.17 seconds of CPU: 73.56 seconds in residual matchers, 2.47 seconds in shared prefixes, and 6.14 seconds outside those boundaries.
The earlier integration's corresponding boundary overhead was 4.06 seconds.
Batch construction and transport still cost CPU; reducing statements does not eliminate that work.
The new trace records 8,277,372 memo hits and 64,472,240 misses.

Actual native root evaluations fall from 73,199,829 to 64,235,386, including shared producers.
The shared async prefix evaluates 1,871,239 roots and admits 65,292 rows.
It shares only equal domains. ASYNC221 retains additional private work where its domain differs.
The catalog's fivefold prefix reduction must not be projected onto the full workload.
The largest remaining residual costs are B023, F405, ASYNC221, B007, and B615.

The [first qualification summary](../benchmark/sql-matcher-batch-first-integration.json) records this run's gates, hashes, timings, and producer metrics.
The following trace links contain the final qualification after the additional changes recorded below.
The [catalog trace](../benchmark/sql-matcher-batch-integration-catalog.json.gz) retains captures, SQL, and EXPLAIN plans.
The [workspace trace](../benchmark/sql-matcher-batch-integration-workspace.json.gz) retains both engines' complete job and edit reports.
The [generated statement](../benchmark/sql-matcher-batch-integration.sql) shows the actual demand and publication shape.

Focused validation passes six new batch tests, five producer tests, seven integration tests, eleven hard cases, and ten three-valued tests.
The integration tests include cross-file dependency invalidation, prepared-plan reuse, statistics-driven shape changes, and rejection of execution 101.
A broad SQL test filter also entered the retired interpreter test suite and was stopped.
For example, its composition-choice test still requires three interpreter alternatives; the retired selector now visits one.
Those historical tests do not qualify this engine, and this change does not reinstate their execution model.

The [implementation design](sql-matcher-batch-implementation.md) defines reusable interfaces and remaining scope.
The worker count and byte-bounded partition scheduler remain unchanged.
Generic call-site/scope relations, capture-dependent factoring, and cross-edit producer caches remain separate contract work.

### Storage-backed statistics qualification

The first attached-store run fails an existing benchmark assumption in the native child.
After an edit, the storage-backed API can return a delta rather than the complete finding set.
Comparing that delta with the initial snapshot falsely reports that restoring the source removed all findings.
The harness now reads the public result pages for attached stores, including evicted files and failures.
It includes paging CPU in those measurements and records the SQL planning statistics after edits.
This correction changes only qualification code. It does not change the measured production engine above.

The corrected 512-file attached-store comparison passes finding and edit parity, but its kind counts remain zero.
Inspection shows that search indexing runs on a 30-second maintenance timer, after tape publication.
The diagnostic finishes before that work occurs. Merely attaching the store does not exercise data-driven planning.
The harness now completes the existing stored-tape indexing operation before the edits and measures that preparation separately.
It requires nonzero SQL-held counts in a subsequent SQL edit profile.
The production compiler still reads the stored vocabulary; it does not count batch nodes in Rust.

The completed 512-file statistics check passes 11,648 finding identities and all three edit comparisons, with no failures.
The SQL edits read nonzero counts for 47 of 49 requested kinds, totalling 689,701 indexed occurrences across those kinds.
The planner changes from one parent-driven arm to four after indexing.
Index preparation consumes 347 ms of CPU in the SQL subprocess and is reported separately.
These are store-wide cost estimates; they are not exact candidate cardinalities.
The [statistics summary](../benchmark/sql-matcher-batch-statistics.json) and [raw trace](../benchmark/sql-matcher-batch-statistics.json.gz) retain the evidence.

### Final physical-plan corrections

EXPLAIN shows SQLite moving F707's parent-level sibling test after child expansion in the first parent-driven query.
The compiler now materializes qualifying parents before expanding their children when the parent predicate contains more than a kind test.
The focused 256-group test preserves findings and reduces VM instructions from 34,360 to 17,983.
Its sampled statement CPU falls from 1.94 ms to 0.62 ms.
The test requires both fewer VM instructions and the intended materialization in EXPLAIN.

The batch also uses the generic prefix planner to identify potential shared consumers when a session is created.
Unique prefixes bypass runtime domain-key construction.
This preliminary filter does not authorize sharing; the batch still checks complete domains and options before execution.
The new `grouping_roots` metric exposes the remaining equality-planning work.

### Final publication qualification

The final engine passes 13 active compiler checks, including the complete catalog comparison and seven batch tests.
The ordinary library build also passes without test-only features.
The full job passes every unchanged qualification ceiling, with 97,798 finding identities, no failures, and parity through all three edits.

| Measurement | Native | Final SQL batch |
| --- | ---: | ---: |
| Initial-job CPU | 112.56 s | 110.41 s |
| Initial-job wall time | 84.04 s | 129.93 s |
| Initial-job peak RSS | 853 MB | 985 MB |
| Median edit CPU | 190.20 ms | 211.92 ms |

The final SQL plan executes 23 times and uses 25,216,565 VM instructions.
The former clause-at-a-time integration used 46 executions and 73,275,953 VM instructions.
Native root evaluations fall from 73,199,829 to 64,235,386, including the shared producer.
Only 13,922,809 candidate roots now enter domain grouping.
The earlier implementation did not record grouping separately; its total candidate count is not a measured grouping baseline.

The CPU ratio is 0.9809 and the median edit CPU ratio is 1.1142.
Wall time worsens in this final run, while CPU remains close to native.
Other work runs on the development host. These observations do not establish a repeatable wall-time improvement.
The end-to-end CPU improvement is modest; the larger structural counter reductions must not be presented as an equivalent speedup.

Matching CPU is 82.24 seconds: 73.24 seconds in residuals, 2.64 seconds in shared prefixes, and 6.37 seconds elsewhere in the boundary.
The final boundary has 398,868 timed residual spans and 1,494 timed producer spans.
A separate clock-cost check measures approximately 0.11 seconds for their 800,724 CPU-clock reads.
Clock instrumentation alone does not explain the remaining boundary overhead.

The F707 materialization reduces catalog VM instructions but adds 161 VM instructions to this whole workload compared with the first batch query.
Its benefit depends on candidate scope and parent fanout. The focused amplification test does not establish a whole-workload speedup.
The materialization remains bounded to qualifying parents and protects the demonstrated repeated-parent case.

The [final qualification summary](../benchmark/sql-matcher-batch-integration.json) records the exact source and executable hashes and all gate results.
The linked catalog and workspace traces above now contain this final run.
The separate attached-store trace qualifies statistics refresh and stored-result paging on 512 files.
It is not a substitute for the full 5,725-file performance run.

## Native operation batching: moving below the fragment boundary

The next request requires batching at the transducer operation level, across both files and programs.
The preceding implementation still calls `fragment_at` once per residual root.
Its SQL submission count does not describe that native dispatch granularity.

The new implementation introduces `matcher/wave.rs` as a SQL-independent CPU executor.
Its input lanes contain numeric program, document, and preorder IDs.
Immutable program tables contain closed structural operations and utility references.
Operation cohorts process many lanes together, including lanes from different programs and documents.
Kernels contain no scalar `AstMatcher` calls and use no thread-local matcher context.

The lowered operations cover kind and regex tests, booleans, tree navigation, stop predicates, counts without exports, and closed recursive utilities.
Navigation advances one cursor per active lane. It does not materialize a Cartesian relation of all descendants.
Short text predicates cache results by complete text and predicate identity within a submission.
Utility results carry truth, work, and depth. Incomplete results cannot enter that cache.
The executor compacts active lanes after decisive predicates and scatters outputs back to the original request order.

The native batch now tiles both file and program axes before executing closed prefixes.
Root demand stays explicit; different consumer domains are not replaced by an unrestricted scan.
Per-key sharing is legal here because a closed syntactic predicate has no capture or semantic inputs.
Each consumer retains its own roots and allowance. Exhausted or incomplete computations request native replay for that file and clause.
This is a keyed fact contract, distinct from the earlier domain-complete producer contract.

Complete closed programs can publish their typed outputs without a scalar fragment call.
Capturing and semantic residuals retain their native implementation and separate instrumentation.
This CPU executor is not a Metal backend. Its lane interface and operation cohorts establish the dispatch boundary such a backend needs.
The sibling transducer specification's document table and `(group, document, root)` task model guide this interface.

Qualification is in progress. Required evidence includes cross-program and cross-document cohorts, native verdict and work parity, catalog parity, and whole-job CPU.

### First operation-wave measurements and deeper admission

The first full run preserves 97,798 findings and all three edits over 5,725 files.
Native cold CPU is 109.88 seconds; SQL cold CPU is 109.34 seconds.
SQL uses 23 matching statements. Peak RSS is 1.07 GiB.
This is a diagnostic run, not the final qualification of subsequent changes.
Raw evidence is `/tmp/rerail-wave-full-v3/{native,sql,comparison}.json`.

There are 16.67 million submitted lanes and 12.22 million distinct root lanes within submissions.
The executor performs 184.43 million operation lanes through 4.30 million kernel dispatches.
Submissions mix up to 24 documents and 13 programs; maximum kernel width is 2,048.
Scalar residual roots fall from 62.36 million to 58.08 million. That reduction is insufficient.
The largest residual costs remain binding, scope, and mixed syntactic/semantic rules.
The 10-second process sample is `/tmp/rerail-wave-sql-v3.sample`.

The initial catalog wave run regresses despite batching. Dispatch and cache overhead require attention.
Changes remove repeated document-set construction, redundant single-child composites, and per-lane filter reference clones.
Literal regex forms use byte predicates after checking the regex syntax tree. Other regex forms retain the regex engine.
Bounded utility and text caches now survive submissions over the executor's immutable document revision.
Each cache has a fixed 32,768-entry ceiling. Executor disposal invalidates the revision.

The next pushdown handles negative admission proofs inside mixed alternatives and utility definitions.
A false proof means native execution would stop before reaching a capture or semantic operation.
A possible match still executes the original native program. It cannot publish a finding directly.
Conjunction proofs stop at the first opaque term. A disjunction needs a proof for every branch.
Partial proofs cannot pass through negation. They cannot reorder capture reads, writes, or unknown-producing operations.
Only rejected paths charge their proven work; admitted paths retain the original program's charges.
This extends batching below the top-level prefix without treating uncertain results as false.

A focused operation test compares both verdicts and work charges, including recursion and both walk-admission policies.
Cancellation tests ensure that a cancelled document does not cancel other documents in its cohort.
A separate negative-proof test checks that rejected mixed branches preserve native work charges.
Production tests require mixed document/program submissions and zero scalar calls for fully closed native programs.
The full-workload checker now reports scalar roots separately and rejects effectively scalar kernel dispatch.

### Review corrections and navigation amplification gates

A constrained, read-only GPT-6.1 Sol review found repeated relation walks, a cancellation gap, and missing demand-node validation.
All three findings led to implementation changes.
Ancestor and sibling end-walks now build a shared suffix graph. Each demanded suffix is evaluated once and then scattered.
Filtered descendant end-walks use preorder postings from the existing native kind index.
Large postings remain shared within a dispatch even when they exceed the cross-dispatch retention ceiling.
Filtered walks poll cancellation even when no candidate reaches a child kernel.
Supplied node IDs are validated before tape access.

Repeated unfiltered descendant walks need a separate physical shape.
After seven failed probes, the executor can materialize a zero-cost local predicate over the demanded subtree union.
It requires at least sixteen active requests and more than four times as much remaining traversal as unique nodes.
Early matches therefore retain short-circuit execution. Completion ranges prevent uncovered nodes from becoming negative results.
This path excludes utilities, exported counts, navigation in the child predicate, and semantic operations.
The deep-tree gate covers shared suffixes, filtered postings, and unfiltered predicate materialization.
It checks navigation operations, not wall time or dispatch width alone.

The expanded work-charge parity test then caught a sibling-anchor discrepancy.
Native sibling iterators normalize the initial position by source bytes, even at the document root.
Structural links alone miss that behavior and zero-width token handling.
The tape now exposes the native initial-position operation. Suffix continuation still follows structural links.
The graph's internal suffix keys distinguish an inclusive structural cursor from a user root.
The regression test includes malformed source and both three-valued admission modes.

### Final native-wave qualification

The final paired run uses the unbounded-work test feature, loop profile, optimization level one, and the real jemalloc archive.
The local test executable omits LTO, as in the preceding qualification.
The corpus remains `/tmp/nera-data-layer-fce` at `fce4dc7208c0b892c11a549c8be43fc285580d6b`.

All four focused operation tests pass, including native work charges, malformed-source sibling anchors, and the navigation amplification gate.
All fourteen active compiler integration tests pass; two diagnostic/repository tests remain intentionally ignored in that namespace.
The separate complete workspace test passes. A normal library check also passes.
The constrained reviewer checked the suffix graph, sibling normalization, completion domains, and cache identities after the fixes.

The full job preserves 97,798 findings over 5,725 files, with matching failures and three matching edits.
The 597-fixture catalog preserves raw matches and captures across all 87,162 rule/file pairs.
There are no unknown catalog outcomes and no resource-triggered native replays in the full workload.
The existing ceilings remain unchanged. All pass, including the 100-statement ceiling; the job executes 23 matching statements.
VM instructions remain 25,216,565 because this work changes native execution, not the published SQL result shape.

Native cold CPU is 113.851 seconds; SQL cold CPU is 111.716 seconds.
Native wall time is 66.797 seconds; SQL wall time is 69.670 seconds.
SQL peak RSS is 1,184,333,824 bytes. Median edit CPU is 213.174 ms, versus native's 202.819 ms.
These are single paired measurements on a shared host, not a claim of a repeatable wall-time speedup.

The executor processes 30,044,209 lanes through 14,759 submissions.
Of those submissions, 14,614 mix documents and 14,749 mix programs.
There are 242,632,839 operation lanes and 5,726,033 kernel dispatches, averaging 42.37 lanes per dispatch.
These are operation counts, not native root calls or SQL VM instructions.
Partial admission proofs reject 16,693,510 of 25,424,539 tested roots.
Some of that work replaces earlier exact-prefix filtering, so it is not an equivalent reduction in scalar root calls.

Scalar roots fall from 62,364,147 to 53,109,000: 14.8% fewer calls.
Residual CPU falls by 8.75 seconds, but wave preparation and execution add 10.47 seconds relative to the previous prefix implementation.
Total matching CPU changes from 82.244 to 82.637 seconds. Whole-job CPU remains near the previous SQL result of 110.415 seconds.
This is a qualified change in native execution granularity, not a demonstrated whole-job performance win.
The trace still exposes 53.11 million scalar roots. Capturing and semantic residuals have not become device-ready transducers.

The suffix graph processes 619,608 closure nodes. Filtered descendant execution performs 404 posting seeks.
Adaptive unfiltered subtree materialization is covered by the deep-tree gate but does not activate on this workload.
Its structural guarantee must not be presented as a measured contributor to these timings.

The [qualification summary](../benchmark/sql-native-wave-integration.json) records the complete measurements and gate results.
Compressed raw catalog, native workspace, and SQL workspace reports are published beside it.
The [implementation document](sql-matcher-batch-implementation.md) describes the current APIs, physical choices, and remaining scalar boundary.

The checker now requires wave evidence, cross-document and cross-program submissions, and more than four operation lanes per kernel dispatch.
It also ratchets scalar native root calls at 53,109,000 for this fixed workload.
Removing batching or restoring the old scalar call count therefore fails qualification, even if SQL execution counts remain low.
