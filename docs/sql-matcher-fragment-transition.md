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

### Complete native cohort conversion — implementation in progress

The published wave path still called the scalar matcher for 53,109,000 residual roots.
This phase removes that boundary. Capture frames, utility calls, and ordered structural operations now belong to a native cohort scheduler.
Patterns use batched node operations. Semantic leaves project facts and submit child demands to that scheduler.
Parse, binding, and enumeration must not hide scalar child matching behind their batch interfaces.

The implementation keeps the existing admission wave as an early rejection stage.
The residual stage must mix documents and programs, retain per-clause resource limits, and discard provisional rows after resource failure.
A runtime audit will count scalar matcher calls inside semantic projections.
Qualification requires zero such calls and zero scalar residual roots, plus catalog parity and the existing whole-job ceilings.
These are completion gates, not results claimed for the unfinished integration.

The SQL adapter now routes grouped bindings and standalone fragments through the same capture cohort executor.
Enumeration runs as rematch waves from immutable input captures.
The adapter holds provisional rows per file and clause. A resource failure discards that owner's rows.
Document tiles retain sixteen files' request rows and semantic memos at a time.
This bound avoids retaining capture-bearing demands for the entire workspace.

Review found two integration defects before qualification.
The cancellation callback consumed the shared file allowance on every operation.
A separate cancellation probe now checks closure and deadlines without charging shared work.
The generic scheduler also assumed that every source was `String`.
Parsed literal documents use `SubSource`; the scheduler now accepts any byte-backed document source.

The first focused scheduler tests exposed disabled parser features in their fixtures.
The fixtures now use the repository's `GrammarDialect` parsers.
A capture-threading defect in conjunctions was also corrected before the integrated qualification run.
The tests compare successful capture environments, including native capture propagation through a failed `nthChild` relation candidate.
The qualification checker now rejects any scalar residual roots or scalar matcher calls from semantic projections.

### First complete-conversion measurement — rejected

All fourteen active compiler checks pass. The catalog preserves all 87,162 raw match/capture comparisons and 2,477 matches.
There are no unknown catalog outcomes or scalar matcher calls inside semantic projections.
The focused pattern, utility, capture, Parse, Host, FormatArgs, memo ownership, and cancellation tests also pass.

Performance fails. Catalog SQL CPU rises from the published 1.156 seconds to 5.941 seconds; native remains at 1.084 seconds.
The first full SQL run exceeds the CPU ceiling before completion.
This conversion is not qualified by parity alone.

A five-second process sample identifies scheduler overhead.
Collapsed self samples include 3,091 in the cohort driver, 646 in structural advancement, and 385 in limit-tree lookup.
Binding and parsed-literal reducers are small contributors in that sample.
The driver repeatedly scans completed lanes while a deeper lane continues.
It also moves full capture frames through dispatch buffers and resolves owner budgets for every operation.

The next implementation removes completed lanes from the active frontier and puts only lane IDs in dispatch buffers.
It checks failure once per context per round and stores dense owner budgets in an indexed vector.
Atomic semantic predicates retain the input capture frame when they make no capture changes.
These changes address measured costs. Their parity and performance checks are pending.

The first full run finishes with exact parity: 97,798 findings, no failures, and matching edit results.
It makes no scalar matcher calls from semantic projections.
Cold SQL CPU is 615.603 seconds; wall time is 528.631 seconds; peak RSS is 928,169,984 bytes.
The executor dispatches 2,061,689,692 outer operation lanes for 53,109,122 root and enumeration requests.
These results fail the performance gate despite preserving findings and memory bounds.

The active-frontier change reduces catalog SQL CPU to 3.658 seconds, with full parity.
Atomic-child fusion reduces it to 3.533 seconds. That small second improvement does not close the structural gap.
A focused generic-document test exposes native `nthChild` indexing through `binary_search` on node IDs.
The cohort used source-order lookup; tree-sitter pointer IDs reveal the difference even though tape ordinals do not.
The implementation must preserve that native indexing behavior while the contract remains unchanged.

Further comparison finds native physical plans missing from the conversion:

- Two-valued relation walks skip impossible kinds and use bounded kind/text postings for descendants.
- Sibling walks can select candidates by the outside capture's equality class, including unknown classes.
- Node-local relation walks retain completed, zero-work suffix and subtree answers.
- Utility memo keys include referenced captures; successful cache entries replay added bindings into the caller's frame.

The new executor initially walked every candidate and keyed utility results on the complete capture frame.
Its full run records 56,983,552 evicted utility entries and 1,711,437 hits.
The published native residual path recorded 4,989,311 hits and 46,505,686 misses.
The next implementation restores candidate-index projections and scoped capture deltas.
Relation closure sharing remains a separate required check against the native physical plan.

### Native physical plans restored in the conversion

The indexed-navigation and scoped-capture build preserves all catalog comparisons.
Catalog SQL CPU falls to 3.317 seconds, versus 1.185 seconds for native in that run.
This remains above the ceiling. It does not qualify the conversion.

The next build adds lazy relation closure sharing and native sibling-list caching.
Closure entries contain completed zero-work answers. Candidate evaluation remains ordered and batched.
A failed or charged walk cannot publish a clean answer for its affected prefix.
The host verifies document identity and excludes parsed documents and hosted parent searches where required.

Review also found a resource-accounting defect in sibling counting.
Cached sibling-list construction must charge actual work once, then exclude that fill from utility replay charges.
The selected node still runs its own predicate and pays its normal replay cost.
A focused test covers nested utility probes, cold and warm file queries, and uncached parsed-document queries.

Utility keys now use the native utility identity, including the existing digest for equivalent scoped definitions.
Entries that read clause-local state retain their owner. Other entries can share across equivalent references.
Atomic projections report clause-state reads explicitly through the cohort results.
These changes are awaiting integrated qualification; the published baseline artifacts remain unchanged.

The first closure build preserves all catalog matches but measures 3.432 seconds of SQL CPU.
Review then finds concurrent duplicate fills: same-owner roots start overlapping walks before any can publish cached answers.
The executor now admits roots in owner order while batching independent document and program owners.
A single-submission complexity test covers 256 overlapping roots across two owners.
It requires linear predicate visits and more than one active lane.

Linear closure navigation now advances a cursor instead of copying whole suffixes.
Review catches a zero-width-node distinction: native sibling iterators use a byte-positioned initial anchor and structural continuation.
The cursor retains that distinction with a shared parent index. Python and TypeScript zero-width fixtures compare both directions with native.
Sibling tables now share their counted vectors and have bounded retention.

### Ordered admission measurement and correction

A 500-file development slice preserves findings and all three edits.
Native cold CPU is 8.772 seconds; cohort SQL CPU is 28.668 seconds.
This slice diagnoses changes; it cannot replace the full-workload qualification.

The trace records 89,262,865 control-operation lanes and 37,688,464 dispatches.
Average width is only 2.37, below the batching gate.
All 4,669,373 semantic projections run in single-lane owner groups.
Blanket root ordering therefore removes useful batching and is rejected.
Ordering now applies only to program graphs that reach shared closure or sibling-table fills.
Other roots retain full-width admission.

A process sample attributes the largest active self counts to the cohort driver (1,044), structural advancement (610), frontier maintenance (325), and memory moves (275).
Frames now remain in their lane stacks during advancement. Actions carry only new children or completed answers.
Pattern context constraints compile once. Pattern results retain shared capture environments instead of cloning them into new frames.
The next development measurement will test this combined scheduler change.

The in-place frame build lowers the 500-file slice to 19.963 CPU seconds, with identical findings and edits.
It records 89.4 million operation lanes and 20.5 million dispatches.
Semantic projections now average 5.85 lanes per owner group.

The first persistent-refill build preserves parity but regresses CPU to 22.077 seconds.
Dispatches fall to 14.8 million, while semantic owner groups rise from 798,545 to 1,343,918.
Immediate one-slot arrivals reduce phase alignment. Lower dispatch count alone does not qualify this change.
The driver will refill in bursts and advance structural continuations directly until the next primitive operation or indexed-navigation boundary.
A separate counter will retain the number of fused control steps.

Review also finds retained historical enumeration sets in utility memo keys.
Memo keys now hold weak snapshot identities, which keep addresses unique without retaining old range sets.
Completion transfers the exclusion set into the next round when it is uniquely owned.
A focused retention test checks that memo entries do not keep an old range set alive.

Burst refill and control chaining preserve parity and reduce slice dispatches to 3,585,846.
The trace records 55,170,746 additional fused control steps.
SQL CPU is still 20.441 seconds. Moving control work between counters does not close the runtime gap.
A fresh sample still places structural advancement and control chaining first.

The compiler review identifies a missing rejection proof: pattern kind guards.
There are 40 pattern-first grammar-root declarations in the catalog trace; 35 have no explicit kind term.
The guard now uses the pattern's necessary kind set, without treating a passing guard as a match.
Template mode, unconstrained root metavariables, and ERROR kinds receive no guard.
Conjunction admission still stops at the opaque pattern. Disjunction rejection requires every branch's proof.
This change targets roots that previously entered the full capture executor only to fail immediately.

The next build also groups semantic context installation by file.
Each rule retains its own clause identity and allowance; file scopes and parked facts are installed once for the operation group.
The budget-isolation integration check now exercises both a closed utility and a capture-bearing utility.

### Closed subprogram dispatch inside capture execution

The pattern guard and file context grouping preserve raw catalog parity.
On the 500-file slice, SQL CPU is 20.847 seconds, versus the prior native measurement of 8.772 seconds.
The extra pattern guards reject approximately 92,000 roots. This does not close the performance gap.
File grouping reduces 800,954 clause projections to 625,725 file context installations.
The trace still records 41.6 million operation lanes and 55.4 million additional control steps.

The next structural change reuses the numeric wave executor inside mixed programs.
A closed subprogram needs no capture frames, but the capture scheduler currently expands all its structural operations.
The compiler will mark these closed regions and submit them as numeric lanes across documents and programs.
The surrounding capture program retains its environment and ordered continuations.

The initial dispatch requires a file document and no active enumeration exclusion.
Parsed documents retain the generic operation kernels.
A completed closed answer can publish only when its work and depth fit the current allowance.
Otherwise, the existing operation scheduler retains responsibility for the precise resource failure.
The trace will report closed region requests and numeric kernel work separately.
This is an implementation experiment. Qualification remains pending, and the published baseline evidence remains unchanged.

Closed region facts now persist across arrivals within a streaming submission.
Documents register once, and each submission borrows its cancellation callback temporarily.
This avoids rebuilding whole-file kind postings for every operation cohort.
A focused test requires one posting build across repeated roots and checks cancellation after cached execution.

The same review identifies repeated capture-pattern producers.
Nineteen policies use `$F($$$ARGS)`, with 1,032,324 candidate roots on the 500-file slice.
The trace records 3,464,342 pattern requests and 13,455,621 pattern operations across all clauses.
Compiled plans now share a complete structural key, strictness mode, and contextual root constraint.
A bounded cache shares results only for the same document, node, and immutable input environment.
Different input environments execute independently. Constraints and resource checks remain specific to each owner.
Counters distinguish cache hits, misses, duplicate requests within a batch, and physical pattern executions.
Performance remains unqualified until the integrated measurement completes.

The combined closed-region and pattern-sharing build passes 24 cohort, 6 wave, and 4 pattern tests.
The 500-file slice retains finding parity but measures 21.242 CPU seconds, still above the ceiling.
Pattern operations fall from 13,455,621 to 4,428,212.
There are 1,402,493 unique pattern executions, 1,148,518 cache hits, and 912,949 duplicates shared within batches.
Closed regions accept 3,306,591 demands with no resource fallback and build only 102 posting lists.

The remaining cost is still structural control: 28,546,474 operation lanes plus 44,563,343 control steps.
The sample attributes its largest active self counts to advancement (752), control chaining (493), and the driver (424).
Numeric wave evaluation accounts for 90 self samples.

Two further changes follow from this evidence.
First, root ordering can skip closed regions when the numeric kernel is eligible.
Generic documents, enumeration, and cached sibling counts retain their required ordering.
Second, a constrained pattern can use a compact pipeline for its pattern and direct atomic constraints.
The current trace contains 1,605,950 constrained starts, each creating general child frames for these simple stages.
The pipeline must preserve constraint order, missing-capture skips, capture rollback, unknown results, and resource charges.
These changes remain subject to the unchanged parity and performance gates.

### Full conversion measurement after shared kernels

A fresh paired run covers all 5,725 files and all 146 clauses.
Native cold execution uses 111.246 CPU seconds and 45.632 wall seconds.
SQL cold execution uses 270.058 CPU seconds and 122.716 wall seconds: 2.428 times native CPU.
Both engines produce the same 97,798 findings and the same findings after all three edits.
SQL uses 23 matching statements, reports no incomplete files, and peaks at 1,332,985,856 bytes of resident memory.
The median edit CPU ratio is 1.176. The cold CPU gate fails.
Raw reports are in `/tmp/rerail-cohort-full-17`; these results do not replace the published qualification.

The trace contains 384,278,733 operation lanes and 609,406,882 additional control steps.
Closed regions accept 45,217,499 demands with no resource fallback.
Shared patterns execute 19,496,600 unique requests, with 13,916,588 cache hits and 11,992,590 duplicates shared within batches.
The full-run sample again places advancement, control chaining, and the driver ahead of the primitive kernels.

The next build compiles constrained patterns into batches of pattern and constraint stages.
It also removes singleton conjunction wrappers only when the enclosed constrained pattern has this proven pipeline form.
Program identities and capture analysis remain intact. Other conjunctions retain their existing semantics.
An independent review finds no concrete rollback, context, or resource-accounting defect in this lowering.
Focused tests and workload measurements remain necessary before acceptance.

The constrained pipeline passes 27 focused tests and preserves all findings on the 500-file slice.
It executes 909,440 pipeline rows and removes 1,816,336 child frames.
SQL CPU falls to 20.121 seconds, which remains far above native.
The broader catalog comparison preserves all 87,162 raw rule/file comparisons, including captures and unknown results.

An integration sharing assertion still fails: equivalent composite admission guards receive different numeric program IDs.
Interning their leaf kinds was insufficient because their conjunction wrappers remained distinct.
Composite partial proofs now share an ordered child-ID key, with separate conjunction and disjunction tags.
The caller still controls whether a result is an exact match or only admission evidence.
The candidate-domain test remains unchanged. The next build must pass it.

The next larger control change batches contiguous atomic and constrained stages inside conjunctions.
Opaque operations remain barriers. Unknown-capture withholding and transactional capture state remain specific to each conjunction.
New counters will classify every control step, including fused steps, and report frame sizes.

The conjunction-run experiment passes 29 focused tests and all 14 engine integration tests.
The composite admission-sharing assertion now passes. All 87,162 catalog comparisons retain raw parity.
However, the 500-file CPU measurement regresses from 20.121 to 22.785 seconds.
Operation lanes fall to 19.04 million and additional control steps fall to 29.82 million.
This is another direct counterexample to treating fewer dispatches or control steps as a runtime improvement.

The experiment enters 14.49 million conjunction rows but executes only 11.51 million simple stages, with 5.22 million opaque yields.
Most runs are too small to justify their table setup and separate primitive dispatch.
The conjunction-run implementation and its tests are removed. The proven constrained pipeline remains.
The new control-family counters remain because they identify the actual hot operations.

The counters attribute 10.58 million advances to alternatives, 6.74 million to utility entry/return, and approximately 9 million to relation startup/search.
A capture frame occupies 256 bytes; its continuation enum occupies 104 bytes.
The next layout experiment reduces copied node metadata and isolates the uncommon sibling-count continuation.
Both native and SQL use the node representation, so its performance comparison must measure both engines with the same build.

The compact layout passes 28 focused matcher tests and 36 tape tests, including existing navigation and embedded-language cases.
Node handles retain direct record access while reading cold metadata through the document.
The uncommon sibling-count continuation has one boxed allocation that survives all its probes.
Frame size falls from 256 to 160 bytes; common continuation state falls from 104 to 48 bytes.

A fresh paired 500-file run preserves findings and edits.
Native CPU is 8.746 seconds; SQL CPU is 18.822 seconds, a ratio of 2.152.
This improves the conversion but does not meet the ceiling. The compact layout remains in the working tree.

The next compiler change removes singleton conjunction wrappers while retaining their capture transaction.
A compiled rollback bit preserves false and unknown results without another conjunction frame.
Relation searches need a separate original-input snapshot because a failed sibling-count predicate can modify their working captures.
Bare sibling searches retain that native behavior. A surrounding conjunction must still roll those captures back.
The next profile also records hot program shapes, so remaining control work can be tied to concrete compiled structures.

The singleton transaction lowering passes 33 focused tests and preserves the 500-file findings and edits.
SQL CPU is 18.806 seconds, approximately unchanged from the compact-layout build.
It completes 557,139 normalized singleton conjunctions. The remaining conjunctions therefore require a different transformation.

The new exact control inventory identifies the largest shapes: `Util[Any]`, `All[Kind,Has]`, `All[Kind,All]`, `Inside[All]`, and `Has[All]`.
The next lowering extracts leading, exact kind predicates into entry guards and removes their interpreted stages.
Known-false alternatives can then skip child frames without changing branch order or work charges.
Guards cannot move across utilities, semantic operations, or other potentially charged work.
Pure kind disjunctions also require the native exclusion check when enumeration is active.

The original temporary corpus loses its Git metadata before the next measurement.
A new locked checkout at `/tmp/rerail-capture-corpus-fce4dc72` uses the same pinned commit.
All 5,725 Python files, their 95,697,853 bytes, and the complete content inventory match the published baseline.
The inventory digest remains `4525032ac7cf058a9134aad9250fd3a400eb4df78c69bfdd89d608057e1b9773`.

Profiler review finds 53,990 small nested submissions on the 500-file slice, performing only 208,503 operations.
They must not initialize and scan a counter array for the complete compiled catalog each time.
Small submissions now use exact sparse counters. Streaming submissions retain dense counters and visit only touched entries.
This changes instrumentation storage, not matcher execution or acceptance gates.

The leading-kind lowering passes all 14 engine integration tests and all 87,162 raw catalog comparisons.
The 500-file run preserves findings and reduces SQL CPU from 18.806 to 16.795 seconds.
Native CPU remains 8.746 seconds, so the ratio is 1.920 and qualification still fails.
Runtime counters record 3,405,201 guard rejections and 2,177,212 skipped alternative entries.
Operation lanes fall to 19,384,347; additional control steps fall to 30,151,871.

One scheduler test fails because its long fixture repeats an identical kind predicate 128 times.
The compiler now correctly reduces that fixture to one guard, so it no longer exercises incremental refill.
The fixture will use matching regular expressions as distinct execution barriers; its scheduling assertions remain unchanged.

The next bounded lowering moves kind membership across direct pattern or regex stages only.
These stages spend no logical work and cannot produce unknown. A failed conjunction restores its input captures.
Pure-kind alternatives also check enumeration exclusions. That check must stay after any preceding pattern that can change captures.
The implementation will preserve that check as a separate atomic predicate at its original position.
Utilities, semantic operations, and opaque operators remain barriers to this transformation.

The full leading-kind run preserves 97,798 findings and all three edit results across 5,725 files.
Native uses 107.112 CPU seconds; SQL uses 208.301 CPU seconds, a ratio of 1.945.
SQL wall time is 61.524 seconds, versus 28.765 seconds for native. SQL peak RSS is 1,408,745,472 bytes.
This improves the previous full conversion run at 270.058 SQL CPU seconds, but still fails the unchanged CPU gate.
The profile remains dominated by native continuation handling, frame copies, and allocation.
Generic relation searches eagerly collect complete ancestor or sibling sequences before testing their first candidate.
A lazy cursor can remove unused tail traversal while retaining candidate order, work charges, and capture transactions.
Review also identifies a neighbor-search discrepancy around zero-width nodes: native `next` differs from the first `next_all` candidate.
The cursor conversion must use the correct native initialization for each stop mode.

The pattern-prefix guard lowering passes 43 focused tests and preserves findings on the 500-file slice.
SQL CPU falls from 16.795 to 15.906 seconds; native remains 8.746 seconds.
This is still 1.819 times native CPU. The next changes target traversal allocation and utility rejection overhead.

The full trace records 37,322,752 utility memo evictions but only 665,004 hits.
A utility whose target entry guard rejects can return without a memo key or child frame.
It must first check the native depth limit and spend the one-unit utility entry charge.
The compiler can also give an alternative a negative membership guard when every branch has an enforced zero-work entry guard.
An opaque or charged branch blocks that proof. Positive admission retains ordered alternative execution and exclusion checks.

The next build also reduces the control-action payload.
A child action carries its request and depth instead of a fully initialized 160-byte continuation frame.
The stack constructs the continuation at its destination. Utility entry still increments depth exactly once.
This removes repeated copies of unused continuation state and accounting fields from the hot control path.
The profile records the resulting action size alongside frame and state sizes.

The lazy-walk, utility-rejection, and compact-action build passes all 49 focused tests.
The action payload is 88 bytes. Continuation frames remain 160 bytes.
The 500-file run preserves findings and reduces SQL CPU from 15.906 to 14.249 seconds, a ratio of 1.629 to native.
It performs 142,580 direct utility kind rejections from 955,157 eligible checks.
This is useful but does not explain the remaining gap: 27,839,897 additional control steps still execute.
The next full run will check whether these reductions hold across the whole workload.

The full compact-action run passes every qualification gate except cold CPU.
It preserves 97,798 findings and all edits, with 23 SQL executions and zero scalar matcher calls.
Native CPU is 116.546 seconds; SQL CPU is 212.614 seconds, a ratio of 1.824.
The median edit CPU ratio is 1.146. SQL peak RSS is 1,450,491,904 bytes.
Absolute SQL CPU does not improve over the previous full run; native CPU also rises on this host.
The paired ratio improves, but the slice result overstates that improvement. The accepted baseline remains unchanged.

The next scheduling experiment groups each logical member across the existing sixteen-file tile.
It preserves member order within each file, per-owner root order, enumeration priority, and the existing resident limit.
Compiled program IDs cannot define this order because residuals are compiled after all original programs.
Only production batch construction changes; the generic transport contract remains unchanged.
This may improve operation occupancy, but it may also increase file-context switches and cache reuse distances.
The experiment must measure CPU, context installations, kernel width, and cache reuse before acceptance.

Review finds that the scheduling experiment lacks a general resource-order proof.
The SQL compiler cache can reuse a template owner across clauses, retaining the same utility-table identity.
Sibling-count terms inside that table can therefore share file-local fill costs across clause owners.
Stable member order does not itself preserve execution interleaving or fill ownership.
The unbounded experiment can measure performance, but it cannot be accepted on finding parity alone.
A retained scheduler must prove independence or preserve the current order for affected work.

The member-major admission experiment is rejected and removed.
On the 500-file slice, SQL CPU rises from 14.249 to 15.457 seconds despite fewer dispatches.
Dispatches fall from 886,730 to 683,814, but file-context installations rise from 310,608 to 344,530.
Utility hits improve from 38,579 to 71,824 without improving total runtime.
Findings still match, which does not resolve the resource-order concern.
Review also finds charged semantic-text sibling classification outside the existing ordering predicate.
The original admission order remains; no scheduling switch or unproven ordering exemption is retained.

The next ownership change targets atomic reference counting in capture requests.
Each capture executor and semantic host remains inside one worker; the host already uses `Rc` for its mutable context.
Only plain output rows and metrics leave that executor. Compiled programs and immutable document objects retain shared ownership across workers.
Capture environments, enumeration snapshots, following-state vectors, and scoped capture fingerprints can therefore use local reference counts.
The conversion must preserve copy-on-write behavior, weak cache identities, pointer validation, and capture rollback.
Compiled patterns, utility tables, semantic plans, and parsed document objects remain `Arc` values.

The local-reference-count experiment passes 49 cohort tests and four pattern tests but regresses performance.
A fresh paired 500-file run uses 8.442 native CPU seconds and 15.570 SQL CPU seconds, a ratio of 1.844.
The SQL-only repeat uses 15.764 seconds. The earlier compact-action build uses 14.249 seconds on this slice.
Worker-local ownership is valid, but that proof does not establish a runtime benefit. The Rc conversion is removed.

The next build batches contiguous identical exclusion snapshots inside each semantic context group.
It retains per-lane depth, allowance, unknown state, cache-fill accounting, and scalar-call auditing.
The current implementation reinstalls exclusions for every semantic row, even when all rows have no exclusion.
No requests change order, and distinct snapshots still receive separate installations.

The compiler also stores compact operation classes beside its program table.
The executor can reuse its known dispatch class instead of decoding the same opcode again for instrumentation.
Conjunctions allocate their withheld-capture set only after an unknown result actually withholds captures.
Normal continuations no longer carry an empty tree-set header through every control step.

The compiled-class and exclusion-run build passes 50 focused tests.
Common continuation state falls from 48 to 32 bytes; the frame falls from 160 to 144 bytes.
On the 500-file slice, 4,670,316 semantic rows need 408,801 exclusion installations.
The fresh paired run uses 8.252 native CPU seconds and 14.629 SQL CPU seconds, a ratio of 1.773.
This remains above the gate; reduced state setup alone does not close the execution gap.

The next sharing experiment separates fixed pattern structure from caller capture unification.
It admits only internal patterns with no multi metavariables and no repeated capture names.
Such a pattern can produce candidate capture facts independently of the caller's environment.
Native insertion then unifies those facts with each input and preserves unrelated bindings.
Review rejects final-environment replay for repeated captures: native node comparison can accept equal node IDs across documents.
An ordered capture-write log would be required to extend the proof to those patterns.
Ellipsis patterns also retain exact-input execution because prebound captures can change backtracking.

The full compiled-class and exclusion-run run preserves 97,798 findings and all edits.
SQL CPU is 198.093 seconds; native CPU is 110.819 seconds, a ratio of 1.788.
SQL wall time is 56.954 seconds and peak RSS is 1,455,046,656 bytes.
The catalog also preserves all 87,162 raw comparisons. The CPU gate still fails.

The latest full profile still attributes most capture-executor CPU to control advancement and continuation handling.
A remaining structural candidate is the unparameterized utility wrapper.
Each cache miss currently creates a full target frame and later advances a separate utility-return continuation.
The proposed spike reuses that frame for the utility body and stores only its caller metadata in a lane-local resume stack.

The resume records the original program, depth, input environment, memo key, and owning frame index.
Utility entry must still check depth and spend one unit before reuse. Parameterized calls retain their existing frame mapping.
Body completion must first apply the body's transaction, then restore each utility's charges, memo result, and outer transaction.
Every completion path must use that operation, including cancellation and navigation failure.
This is not implemented yet. The spike must preserve cache-fill versus replay costs, failure captures, depth peaks, and clause-state isolation.

The fixed-pattern fact spike passes eight focused tests and preserves the paired slice findings.
Its CPU ratio is 1.773, effectively unchanged from the preceding build.
Only 303,358 pattern requests qualify. Compared with the existing exact-input cache, it saves eleven unique pattern executions out of 1,387,324.
Most reported fact hits and within-batch reuse already occurred in the existing cache.
The proof is useful, but this workload provides no meaningful additional reuse. The production change and its added cache path are removed.
The spike source remains in `/tmp/rerail-pattern-fact-spike-28.rs`; the original pattern executor is restored from its verified build-27 snapshot.

The published conversion checkpoint is explicitly unqualified.
The default-feature library check passes. Focused capture tests, the catalog, and full-workload finding/edit parity pass.
The unchanged qualification checker rejects only the cold CPU ratio: 1.788 exceeds 1.30.
The separate `sql-native-capture-cohort-rejected` report and compressed inputs retain that evidence.
The accepted `sql-native-wave` baseline is unchanged. Further work targets utility activation frames.

Utility frame reuse is now implemented for unparameterized cache misses.
A lane stores compact utility resumes and reuses its current frame for the body.
All completion paths apply the body transaction before utility accounting, memo publication, and the caller transaction.
Parameterized calls keep their capture mapping and use the same utility completion helper.

Review also found a failure-path depth bug in the previous executor.
Frontier cancellation absorbed a completed utility body's peak without adding the caller's utility level.
The shared completion helper now includes that level for both call forms, without double-counting work or caching failures.
All 54 focused tests pass, including four new groups for nested resumes, capture transactions, limits, and cancellation.

The unchanged build-27 full rerun uses 176.706 SQL CPU seconds and 96.783 native CPU seconds, a ratio of 1.826.
This differs materially from the earlier absolute timings. New comparisons must use paired runs and exact operation counts.
The 500-file rerun uses 8.733 SQL CPU seconds and 5.760 native CPU seconds, a ratio of 1.516.
The utility spike still needs measured acceptance. Resume byte counters describe occupied entries, not allocated vector capacity.

The first utility-reuse build removes 44,178,292 full utility frames and corresponding return steps on the full workload.
Findings still match. However, SQL CPU is 180.634 seconds against 98.021 native seconds, a ratio of 1.843.
The nearby unchanged run has a ratio of 1.826. The initial spike therefore demonstrates no CPU improvement.
Alternating 500-file runs also show no stable win.

The profile exposes new overhead in the shared completion helpers.
Every physical frame now calls `complete_frame` and `apply_transaction`, including frames without a utility resume.
The next revision keeps ordinary completion inline and separates utility unwinding from that common path.
It retains one physical-pop site and the reviewed transaction order.

The inline-completion revision also fails to improve CPU in alternating slice runs.
Its ratios are 1.909 and 1.770, against 1.733 for the intervening unchanged build.
Utility frame reuse is rejected and removed, including its added resume stack and counters.
The discovered frontier depth correction remains, with focused regression coverage.
The rejected implementations remain in `/tmp/rerail-cohort-utility-fusion-29.rs` and `/tmp/rerail-cohort-utility-fusion-30.rs`.

Review identifies two more direct sources of request-copy overhead.
Both `Frame::child` and `leaf_request` clone the input environment before replacing it with the intended environment.
Direct request construction removes those redundant reference-count operations without changing frame ownership or scheduling.

The next layout spike removes the redundant record slice from `TapeNode`.
Every node's slice equals its immutable document's record array; the sole constructor and all derived handles preserve this invariant.
The handle can therefore retain only the document reference and ordinal.
This reduces handle size but adds a record-access indirection. Both native and SQL execution must be measured.

The compact-node build passes 51 focused cohort tests and 36 tape tests.
The continuation frame falls from 144 to 128 bytes; its action payload falls from 88 to 72 bytes.
The paired 500-file run uses 10.591 SQL CPU seconds and 7.217 native CPU seconds, a ratio of 1.468.
Findings and edit results match. Full-workload measurement remains required.

The next migration removes immutable root metadata from continuations.
Each lane will own its context, truth mode, and binding history once.
Internal inputs will carry only program, node, capture environment, and exclusion state.
Completion will reconstruct the original source request by moving its saved input and lane metadata.

Pure atomic operations will consume borrowed leaf inputs instead of temporary owned requests.
Only semantic, comparison, and navigation boundaries will construct their required owned payloads.
File and memo eligibility callbacks will receive the current context and actual node directly.
Eligibility cannot be cached per root: constraints can select captures from another document.
Capture ownership, weak cache identities, clause order, and resource semantics remain unchanged.

The full compact-node run preserves 97,798 findings and all three edits.
SQL CPU is 150.963 seconds; native CPU is 92.021 seconds, a ratio of 1.641.
The nearby unchanged build has a ratio of 1.826. This improvement is useful but still fails the 1.30 ceiling.
SQL wall time is 42.159 seconds; peak RSS is 1,393,819,648 bytes.
The median edit CPU ratio is 1.173. Matching still uses 23 SQL executions and 25,216,565 VM instructions.
Both scalar-call audits remain zero. The catalog preserves all 87,162 comparisons, and the default-feature library check passes.
The updated rejected report retains these measurements and compressed inputs. The accepted baseline remains unchanged.

The lane-metadata migration now builds and passes 53 focused tests.
Internal inputs omit owner context and binding history. Each resident lane owns those fields once.
Pure atomic jobs borrow their node, environment, exclusion state, and metadata.
Semantic, comparison, and navigation boundaries construct owned requests only when required by their existing interfaces.
New counters distinguish borrowed leaf lanes from those owned boundary requests.

The source slot retains its original input separately from the mutable continuation.
Completion moves the original input and lane metadata back into the public request.
Tests verify original capture identity after rollback and root-slot reuse with different owners, truth modes, and binding histories.
A foreign-document constraint test verifies that file-cache checks inspect the selected node, not the original root.
The implementation received an independent semantic and lifetime review. Runtime acceptance still requires paired measurements.

The full lane-metadata run preserves all 97,798 findings, three edits, and 87,162 catalog comparisons.
SQL CPU is 123.338 seconds; native CPU is 82.797 seconds, a ratio of 1.490.
The preceding compact-node run has a ratio of 1.641. Both runs use the same corpus and qualification configuration.
SQL wall time is 34.482 seconds; peak RSS is 1,380,892,672 bytes.
The median edit CPU ratio is 1.198. Only the cold CPU gate still fails.

Frames are now 112 bytes and action payloads are 56 bytes.
The job submits 115,083,024 borrowed atomic lanes and constructs 59,187,631 owned requests at semantic, comparison, and navigation boundaries.
These counters have different populations: navigation requests are owned but do not enter the atomic-lane counter.
Control steps, fingerprint builds, utility hits, SQL statements, and VM instructions remain identical to the preceding build.
This improvement comes from cheaper execution of the same work, not reduced coverage or weakened resource accounting.
Both scalar-call audits remain zero. The default-feature check passes; the updated rejected report retains the failing CPU gate.

The next sharing spike targets derived capture fingerprints, not utility verdicts.
The existing cache separates utility program IDs even when their exact capture scopes are equal.
The compiler now interns exact optional scopes and uses their IDs in the derived-fingerprint cache.
`None` remains distinct from an explicit empty scope. Parameterized calls retain their sorted caller capture names.
Pool identity, weak environment validation, exclusion-name checks, and the cache bound remain unchanged.
Utility memo identity and resource charges remain unchanged.

The full workload currently builds 15,669,077 fingerprints. Empty input environments already share one fingerprint and provide no new opportunity.
New counters report scope counts and reuse across programs. Such hits do not themselves prove avoided builds or CPU savings.
Review found an invalid mutation in the new COW fixture: native capture insertion rejects conflicting node bindings.
The fixture now changes a reserved transformed binding instead, exercising COW invalidation without violating native unification.

The scope-sharing slice reduces fingerprint builds from 1,142,344 to 675,538, a 40.9% reduction.
Its largest program pool has 984 utility entries but only 15 distinct optional capture scopes.
Utility hits, evictions, control work, findings, and edits remain unchanged.
The paired CPU ratio is 1.419, compared with 1.434 for the preceding slice.
The 903,947 reported reuse hits across programs exceed the 466,806 avoided builds; they are different measurements.

The next revision also returns the existing empty fingerprint for an explicit empty scope without an exclusion name.
The shortcut runs after the existing empty-input check, so its counter measures only additional nonempty-input cases.
No binding, including reserved transformed data, can enter that fingerprint.
An absent scope or an active exclusion name still takes the normal path.
All 55 focused tests now pass, including the corrected COW fixture.

The full scope-sharing revision reduces fingerprint builds from 15,669,077 to 8,370,880.
It preserves all findings, edits, catalog comparisons, utility hits, and control work. The default-feature check also passes.
Its CPU ratio is 1.567. A subsequent unchanged build-32 run measures 1.926, versus its earlier 1.490.
The scope revision's slice ratio is 1.693, versus 1.419 before the constant-empty shortcut.
These timing differences do not establish a causal speedup or regression on this busy host.
Other processes consume substantial CPU, and the machine has six performance cores and twelve efficiency cores.
The exact work reduction is established; its CPU effect remains uncertain. The accepted baseline and 1.30 ceiling remain unchanged.

Assembly inspection identifies a more substantial control cost.
The build-32 `advance` function occupies 18,112 bytes and allocates a 1,024-byte stack frame on every call.
It decodes continuation state and opcode even though the caller already supplies the operation family.
List starts and returns account for 334,876,592 of 575,682,466 advances, or 58.17%.
The chain also copies returned aggregates and action payloads across this call boundary.

The next spike moves those list families into small inline handlers behind the existing family classification.
All other families retain a separate heavy handler. Common metrics and failure handling still run exactly once.
The list bodies are mechanically equivalent; scheduling, transactions, withheld captures, and the 64-step chain bound remain unchanged.
Independent review checks all three callers, including atomic list returns and the shared search/closure family.
Acceptance requires checking the generated assembly and whole-workload behavior, not assuming that extraction improves execution.

The list-dispatch build passes all 55 focused tests.
Assembly confirms that list paths bypass the heavy callee and its per-call frame.
The resident chain frame grows from 320 to 560 bytes; its code grows from 1,912 to 5,520 bytes.
The heavy fallback retains a 1,072-byte frame. Together, chain and fallback code grow by 1,668 bytes.
List paths avoid staging a returned aggregate for that call and avoid its full opcode decode.
The 56-byte action-buffer copy remains, as does the list state's local extraction.
These remaining copies must not be reported as eliminated. Runtime benefit still requires measurement.

The list split passes the slice CPU ceiling at 1.296 but fails the full job at 1.783.
Hardware counters provide a better comparison under the observed scheduling noise.
The same full workload, including setup and three edits, retires these process instruction counts:

| Implementation | Retired instructions | Elapsed cycles |
| --- | ---: | ---: |
| Native, build 32 | 1,238,262,733,524 | 374,591,149,911 |
| SQL, build 32 | 1,619,961,681,854 | 617,391,184,494 |
| SQL with scope sharing, build 34 | 1,595,856,340,281 | 594,896,442,425 |
| SQL with list split, build 35 | 1,610,055,535,960 | 600,322,274,114 |

These are whole-process counters from `/usr/bin/time -l`, not counters isolated to the cold matching phase.
All four runs preserve findings and edits. Raw reports and usage counters are under `/tmp/rerail-instructions-*`.
Scope sharing reduces instructions by 1.49% and cycles by 3.64% relative to build 32.
The list split increases both measures relative to scope sharing, despite removing the heavy list call frame.
The split is rejected and removed. Its assembly and source remain in the temporary spike directory.

The cycle gap exceeds the instruction gap. This suggests memory access and execution latency need direct attention, not just instruction counts.
The next measurement change adds hardware counters at the existing cold-job and edit boundaries.
The next execution spike tests one contiguous frame arena instead of a separate growable frame vector for each resident lane.
Parent frame indices preserve the same continuation order. Free slots can be reused across lanes without copying suspended parent frames.
This targets pointer chasing, unused per-lane capacity, and allocation layout while preserving the operation scheduler.

The qualification harness now samples macOS process instructions and cycles around each apply and its result publication.
It uses the system's V4 process-usage API and records valid nonzero deltas as optional `hardwareWork` values.
Unsupported systems, failed reads, and invalid deltas omit those values. They do not report fabricated zeros.
Cold-job and edit CPU, wall-time, RSS, and acceptance ceilings remain unchanged.
These counters are diagnostic evidence; they do not replace the CPU gate.

The first phase-counter slice reports valid nonzero values for both engines.
Native cold work retires 90,022,660,335 instructions; SQL retires 115,834,457,588, a ratio of about 1.287.
The corresponding cycle ratio is about 1.733. Core assignment can also affect cycles per instruction; these counters alone do not isolate cache misses.
The arena spike therefore remains a testable memory-layout hypothesis, not a claimed explanation of the entire cycle gap.
Vacant arena slots reuse their parent-index field as a free-list link, avoiding an extra allocation for small nested submissions.

The full phase-counter baseline (build 36) retires 1,569,218,661,305 cold-job instructions for SQL plus native cohorts.
Native retires 1,212,125,529,119, a ratio of 1.295.
SQL consumes 595,040,157,611 cycles versus 363,374,775,863 for native.
Cold CPU is 188.482 versus 114.289 seconds, a ratio of 1.649; the qualification ceiling remains unmet.
These counters exclude compilation and the three edits, unlike the preceding whole-process measurements.

The arena spike is installed for build 37 after source-hash verification and two independent reviews.
Every former stack access now uses a stable frame index. All three completion paths release their frame.
A growth-and-reuse test checks live parent chains, unrelated lanes, capture drops, and free-list integrity.
Metrics report active slots, allocated slots, capacity, slot width, and reuse instead of assuming a memory benefit.
Acceptance requires fewer measured instructions or cycles, native parity, and the unchanged whole-job gates.

Build 37 passes all 56 focused tests, including arena relocation and slot reuse.
The 500-file slice reuses 20,651,271 frame slots. Its largest arena holds 6,276 live slots, with capacity 8,192 and 120-byte slots.
However, cold instructions increase from 115,834,457,588 (build 36) to 119,826,792,403, or 3.45%.
Cycles increase from 44,887,296,352 to 49,422,552,713. CPU rises from 13.790 to 14.372 seconds.
Native instructions decrease slightly in the paired controls, so normalization does not explain the SQL regression.
The arena is rejected and removed before a full run. Its source, binary, and slice evidence remain under `/tmp`.

The next spike targets frame work rather than storage layout.
A statically incompatible child currently receives a frame, waits for the next frontier, then immediately returns false.
There are 49,364,276 guard rejections in the full baseline, though only a subset can use this shortcut.
Early parent execution is unsafe: the next frontier must still observe owner failure before consuming the rejection.
The proposed shortcut retains the zero-work child result until that same frontier, without allocating a child frame.
It must preserve failure priority, singleton transaction counts, parent continuation order, and the existing control-chain bound.

The deferred-child spike (build 38) passes all 57 focused tests and preserves slice findings, edits, control work, and cache counts.
It avoids 1,131,566 child frames, but introduces 3,744,465 static kind checks.
Instructions rise from 115,834,457,588 to 117,600,927,301, or 1.53%.
Cycles fall by 3.33%, while CPU is effectively unchanged: 13.820 versus 13.790 seconds.
The evidence does not establish a benefit. The spike is removed; its source and measurements remain under `/tmp`.

The next dispatch experiment uses the operation family already computed by the caller.
The existing handler ignores that classification for dispatch, then decodes state and opcode again.
Unlike the rejected list split, this experiment retains one non-inlined handler and its existing call boundary.
Only dispatch topology may change. Operation bodies, scheduler order, resource accounting, and frame representation must remain identical.

The single-family dispatch rewrite (build 39) passes all 55 focused tests and preserves every slice operation and cache counter.
It retires 116,849,493,806 instructions versus 115,834,457,588, an increase of 0.88%.
Cycles increase by 2.44%; CPU rises from 13.790 to 14.431 seconds.
The rewrite is rejected. Removing source-level dispatch duplication did not reduce executed work in this build configuration.

A separate measured hotspot remains in memo-key hashing.
Sample 32 attributes 65 direct leaf samples to hashing `Arc<CaptureFingerprint>` and 74 to hashing the complete memo key.
The fingerprint derives structural hashing over capture names, document/node identities, lists, and transformed bytes.
Its construction cache avoids rebuilding these values, but each memo-key hash still traverses them.
The next bounded experiment stores an immutable precomputed hash beside the full fingerprint.
Equality must retain full-value comparison, including under forced hash collisions. No native-baseline code changes.
The 37,101,502 fingerprint cache hits indicate reuse opportunities, not an exact count of expensive hashes.
Empty fingerprints, empty maps, build-time hashing, and the extra stored word all affect the net result.

The prehashed-fingerprint spike (build 40) passes all 56 focused tests and preserves slice findings, edits, and every operation counter.
Against the earlier baseline it appears to reduce CPU and cycles, while increasing instructions.
An adjacent unchanged-baseline run resolves that ambiguity: baseline instructions are 115,745,712,405 versus 116,537,747,543 for the spike.
Baseline cycles are 42,122,925,323 versus 42,492,916,409; CPU is 12.988 versus 13.121 seconds.
The spike increases instructions by 0.68% and cycles by 0.88%. It is rejected and removed.

The retained checkpoint contains exact-scope fingerprint sharing and phase hardware counters.
Its full build-36 evidence preserves 97,798 findings, three edits, and all 87,162 catalog comparisons.
Only the cold CPU ceiling fails: 1.649 versus the allowed 1.30. Median edit CPU ratio is 1.135.
It uses 23 matching SQL statements and 25,216,565 VM instructions, with both scalar-call audits at zero.
No rejected storage, dispatch, guard, or hashing path remains enabled.

The next ownership review targets immutable semantic and comparison descriptors in native request batches.
Each lane currently clones an Arc to its compiled descriptor, although execution retains the program for the whole batch.
Borrowing those descriptors could remove reference-count traffic without changing captures, scheduling, or cache semantics.
Any effect from contention across the four workspace workers remains a hypothesis until measured.
