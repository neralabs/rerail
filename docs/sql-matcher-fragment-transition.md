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
