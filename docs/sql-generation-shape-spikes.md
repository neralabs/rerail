# SQL generation and recursive CTE spikes

These test-only spikes extend [the whole-call experiments](sql-whole-call-compiler-spikes.md).
They run on `jeremy/sql-capture-demand-spikes` and preserve Tim's branch.
The production addon still uses the existing scheduler.

## Execution contract

A call normally executes one SQL statement across its files and rules.
A few statements are acceptable when preparation limits or execution cost justify them.
Statement count must not grow with files, rules, candidate rows, or recursion iterations.
Physical planning can happen inside SQL generation.
It does not require a separate optimizer service or another user-facing execution path.

The new physical generator combines SQLite CTEs with a bounded number of native buffer boundaries.
SQLite executes matching operators and composition.
Native aggregates publish typed sets.
They do not evaluate rule control flow or execute nested SQL.
A finite recursive dispatcher sequences these publication boundaries inside each statement.
This dispatcher handles acyclic dependencies, separately from recursive rule semantics.

## Applicable ast-index-spike prior art

The review used main commit `9449260fea46fa81358fc638bf95a1566d475e27` and seadragon commit `de516fed81a6832e33340db899eea6b24b2a2076`.
These sources provide useful SQL patterns, rather than transferable benchmark results.

| Source | Applicable lesson | Current spike |
|---|---|---|
| [`sql_rec.py`, `_subtree_ctes`](https://github.com/neralabs/ast-index-spike/blob/9449260fea46fa81358fc638bf95a1566d475e27/bench/sql_rec.py#L4930) | Collapse each branch to its required grain before joining upward. A large final `DISTINCT` does not prevent intermediate products. | Stage consumers use keyed membership when they retain only their input columns. General capture-bearing branch collapse remains necessary. |
| [`_mat` and related helpers](https://github.com/neralabs/ast-index-spike/blob/9449260fea46fa81358fc638bf95a1566d475e27/bench/sql_rec.py#L1548) | One textual reference can execute many times when correlated. Preserve materialization for batch leaves and indexed intermediates. | Only simple single-use pipelines qualify for inlining. |
| [`_probe_semi`](https://github.com/neralabs/ast-index-spike/blob/9449260fea46fa81358fc638bf95a1566d475e27/bench/sql_rec.py#L1599) | Compare membership, existence, and join shapes on the actual backend. | Both row-value `IN` and `EXISTS` have executable comparisons. |
| [`plan_stmt.py`](https://github.com/neralabs/ast-index-spike/blob/9449260fea46fa81358fc638bf95a1566d475e27/bench/plan_stmt.py) | Use corpus cardinalities to choose anchors. Represent batch metadata as relations. | Root domains share narrow integer identities. Ancestor seeds retain file and candidate-kind restrictions. Cardinality-based anchor selection remains future work. |
| [`build_match_batch_sql`](https://github.com/neralabs/ast-index-spike/blob/9449260fea46fa81358fc638bf95a1566d475e27/bench/sql_rec.py#L8070) | Return tagged outputs for many queries in one execution. | Output tags identify each clause, file, and root across the complete admitted batch. |
| [Seadragon design](https://github.com/neralabs/ast-index-spike/blob/de516fed81a6832e33340db899eea6b24b2a2076/DESIGN.md) | Share exact demand closures. Preserve rare-anchor restrictions through later walks. Select recursive shapes empirically. | Shared root scopes and recursive SQL tests apply these contracts. |

The earlier “No recursive CTEs” interpretation was too broad.
Current `sql_rec.py` uses recursive CTEs for suitable walks.
Seadragon rejected a generic cursor implementation as its second complete lowering path.
That result does not establish a prohibition on recursive SQL.
Its backend-specific recursion and preparation results also do not transfer automatically to SQLite.

## Physical SQL generation

`physical.rs` estimates dependency expansion before SQLite prepares the statement.
It inserts native boundaries where an inline producer exceeds the expansion budget.
The remaining relations stay inside SQLite.
This bounds the shared diamonds that otherwise exceed SQLite's 65,535 table-reference limit.

For multiple statements, the generator identifies boundaries that cross statement phases.
It publishes shared identities, terminal answers, and demand handles before their later readers.
Pure root domains can be recomputed because their identities are deterministic.
It does not publish every logical relation at each statement boundary.
Liveness includes both output readers and required fallible terminal barriers.

Root domains now share `(k, file, node)` across capture layouts.
Their key is `file * 4294967296 + node`.
Each consumer initializes its own capture columns after the shared domain.
Walk candidates still use separate occurrence identities.
File input SQL reads the current batch through `node WHERE id=0`, so file counts do not change the SQL template.

A proved stage bridge returns at most one verdict for each stage root.
When its consumer retains only input columns, a membership probe preserves the result without multiplying rows.
The generator compares row-value membership with correlated existence.
It does not rewrite arbitrary capture-producing joins.

Necessary positive ancestor guards can seed candidate ranges from matching containers.
The seed retains the original file scope and pushes candidate kinds into the range scan.
`DISTINCT` removes overlap from nested containers.
Selected-root guards retain their original semantics.
The pass refuses disjunctions and negated guards.

Two failed experiments explain these restrictions:

- Broad single-use inlining removed useful indexed intermediates and increased one plan from about 51 million to 119 million VM steps.
- Ancestor seeding without candidate restrictions enumerated broad subtrees and increased execution from about one second to two seconds.

After candidate restrictions and exact file lookups, the ancestor rewrite reduces the shaped plan from 51.1 million to 46.6 million VM steps.
These figures describe the instrumented 32-file matching subset.
They do not measure a complete Rerail call.

## Intended SQL shape

For several rules with the same pure predicate, the desired shape resembles this template:

```sql
WITH
consumers(rule) AS (VALUES (0), (1)),
roots(k,file,node) AS MATERIALIZED (
  SELECT file*4294967296+id,file,id
  FROM node WHERE kind=:root_kind
),
excluded(k) AS MATERIALIZED (
  SELECT ek FROM text_match
  WHERE term=:regex_term
    AND ek IN (SELECT k FROM roots)
),
needed(k,file,node) AS MATERIALIZED (
  SELECT k,file,node FROM roots
  WHERE k NOT IN (SELECT k FROM excluded)
),
bound(file,node,capture) AS MATERIALIZED (
  SELECT file,node,v0 FROM pattern
  WHERE term=:pattern_term
    AND nk IN (SELECT k FROM needed)
)
SELECT c.rule,b.file,b.node,json_array(b.capture)
FROM consumers c CROSS JOIN bound b
ORDER BY c.rule,b.file,b.node;
```

This template illustrates a total regex filter and a fresh capture pattern.
It is not the complete policy compiler or a benchmark query.
Other independent predicate groups contribute additional shared relations in the same statement.
A fallible or capture-dependent predicate cannot move earlier without a semantic proof.
First-match walks need a witness relation at parent-key grain, ordered before capture projection.
Unknowns and recursive utilities require their own typed state relations.

The current adapter still inherits wider SQL from the frame compiler.
For example, `unused-annotation` initializes ten capture slots over 59,336 identifier rows before later filters and binding checks.
SQLite then copies these rows and creates wide automatic indexes.
CTE fusion reduces native publication but does not remove these intermediate SQLite rows.
The next logical lowering must restrict demands and delay capture allocation, rather than only changing storage boundaries.

## Measurement scope

The corpus configuration contains 135 policies and 141 applicable Python AST matchers.
The acyclic migration spike admits 55 matchers.
All completed alternatives compare every root and capture against the existing SQL engine.
They return 26 matches on 32 files and 449 matches on 138 files.
This is not Tim's original 818-clause benchmark.
Selectors, rendering, policy composition, and unsupported matchers remain outside these corpus timings.

Preparation, execution, SQL generation, and diagnostic work have separate timers.
`wholeKernelCpuNs` includes connection setup, preparation, execution, and result decoding.
It excludes recorded diagnostic CPU and excludes SQL generation.
Its wall timer includes diagnostic work, so it must not be presented as execution latency.

Legacy timing starts after session creation.
The candidate kernel includes connection setup.
The current runner installs functions and prepares new statements for each execution.
It does not measure a persistent prepared-program cache.

The following values are median CPU milliseconds from three ordinary-build runs.
All cases have complete root and capture parity.
Generation is reported separately from the matching kernel.

| Files | Plan | Statements | Prepare | Execute | Kernel | Generate | VM steps | Published rows |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 32 | All native boundaries | 1 | 31 | 978 | 1,027 | 5 | 50,543,533 | 1,227,253 |
| 32 | Budget 8 | 1 | 55 | 820 | 885 | 43 | 47,535,074 | 96,923 |
| 32 | Budget 16 | 1 | 90 | 817 | 917 | 51 | 44,302,447 | 100,544 |
| 32 | Budget 64, four phases | 4 | 100 | 796 | 907 | 70 | 46,061,465 | 40,086 |
| 138 | All native boundaries | 1 | 30 | 5,519 | 5,644 | 5 | 269,066,085 | 6,558,651 |
| 138 | Budget 8 | 1 | 64 | 5,306 | 5,433 | 48 | 255,834,634 | 500,270 |
| 138 | Budget 16 | 1 | 89 | 4,970 | 5,131 | 56 | 238,331,054 | 527,024 |
| 138 | Budget 64, four phases | 4 | 106 | 4,886 | 5,070 | 77 | 246,620,253 | 210,626 |

The existing scheduled engine measures 987 ms on 32 files and 5,217 ms on 138 files in these runs.
It records 487 and 525 statements respectively for this subset.
The one-statement budget-8 plan improves the 32-file kernel median by about 10 percent against that engine.
The larger one-statement budget-16 plan is roughly comparable to the scheduled engine.
These are modest kernel improvements, despite the large reduction in native publication.

Timing variance is substantial: budget-8 kernel CPU ranges from 796 to 964 ms on 32 files.
On 138 files, budget-16 ranges from 4,778 to 5,313 ms.
The initial sweep uses a fixed case order.
Small differences between neighboring budgets do not establish a statistically reliable winner.
VM counts, row counts, parity, and statement counts are the stronger evidence here.

Additional single-case processes reverse case order between runs.
Against each process's scheduled-engine control, the shaped plan improves CPU by 4–8 percent on 32 files and 2–6 percent on 138 files.
Those controls support a modest improvement, rather than a large speedup.

Budget 8 is a useful initial preparation bound for small calls.
Budget 16 reduces more VM work on larger calls while retaining one execution.
Four phases reduce execution CPU further in this sample, but add generation, preparation, and boundary work.
A final planner chooses these boundaries from expected work and preparation cost, without exposing compiler modes to users.
The generated SQL is byte-identical between the 32-file and 138-file configurations for every compared plan.
Prepared-program reuse can therefore amortize generation and preparation without specializing SQL to the file count.

## Profiling and actionable output

`RERAIL_WHOLE_CALL_PROFILE=1` enables diagnostics in the test harness.
It is not a production compiler switch.
Every generated statement receives an execution trace, counters, `EXPLAIN QUERY PLAN`, and opcode counts.
EXPLAIN runs after the timed execution and does not contribute to runtime statement counts.
SQLite tracing detects nested SQL executions in callbacks.
Each phase must execute exactly one actual statement.

Stage timers surround producer SQL, native leaves, publication, and release.
Publication counters report rows, cells, and final typed-copy CPU.
They do not isolate aggregate row boxing.
A progress callback samples stage VM work at 10,000-step intervals.
Stage VM counts are approximate.
Statement VM counts are exact SQLite counters.

An optional SQLite build enables native loop counters.
Each loop records starts, rows visited, estimated cardinality, actual cardinality, and hardware cycles.
Plan identifiers connect counters to EXPLAIN nodes.
Hardware cycles are inclusive and must not be summed into a CPU total.
Instrumentation changes cost, so ordinary builds provide the timing comparison.

The actionable section ranks hot stages, large publications, repeated scans, unsuccessful probes, and cardinality errors.
Hot stages include clause ownership when available.
Suggested experiments are hypotheses, separate from measured observations.
Raw plans and counters remain in the report.
Interrupted or failed runs retain partial metrics and plans, with incomplete stages marked explicitly.
A rejected candidate has no parity result.

## Recursive rule compilation

`recursive_machine.rs` compiles real logical operators into one recursive CTE.
Each recursive member has one reference to the recursive relation.
Its row carries the rule, file, root, instruction, sparse captures, continuation stack, memo, verdict, and fault.
Rules and files share one statement and batched leaf relations.
A JSON instruction relation describes operators.
Recursive SQL performs control flow.

Rule seeds use a `VALUES` relation with keyed kind scans, rather than one compound-select arm per rule.
An 818-rule generated test executes once and preserves every rule tag without hitting SQLite's compound-select limit.
That test checks seed scaling.
It is not the original 818-clause benchmark.

The SQL handles ordered alternatives, conjunction, negation, first-match walks, count intervals, and recursive utility calls.
It also handles unknown verdicts and withheld captures.
Cycle checks use the reached utility, node, and capture key.
An unread cyclic branch does not cause a refusal.

Exact single-node unification uses a nested structural recursive CTE.
The source-text function only decodes text.
It does not evaluate rule operators.

Differential fixtures compare complete captures against both native evaluation and the existing SQL engine.
They cover multiple rules and files, recursive witnesses, negation, counts, unknowns, withholding, and actual read cycles.
Factoring repeated candidate expressions through table-valued expressions reduces duplicated SQL and VM work.
The later unknown-handling extension adds work, so its final figures must not reuse the earlier 25-percent reduction as a current benchmark.

This is a scheduling feasibility baseline, not the selected performance implementation.
It rejects unsupported contracts explicitly.
Those contracts include parameterized scopes, following context, stop-set walks, list/text captures, and general capture-dependent facts.

It does not yet reproduce the native 256-level recursive dependency limit.
A 200,000-transition limit per root reports a fault instead of silently returning no match.
This compiler currently uses all eligible roots in the batch.
Selected-root integration remains incomplete.

The corpus census admits 32 matchers, including 22 with genuinely cyclic utilities.
On one 4,224-byte corpus file, all 32 matchers have complete root and capture parity.
That run returns 2,850 terminal root verdicts and uses 4,753,369 VM steps in one statement.
Its instrumented execution takes about 107 ms, with 2.5 ms preparation.

Its combined 32-file execution exceeds the experiment's 500-million-VM limit.
The partial native profile shows 1,822,314 rows in the combined leaf relation's automatic index.
It also visits 1,389,753 recursive machine states before interruption.
The generated SQL is only 68,465 bytes, so large SQL text is not the main execution problem here.

This result rejects the generic machine as a corpus performance winner.
Specialized relational recursive units and restricted demand sets remain necessary.
Per-predicate leaf relations can also avoid one large heterogeneous leaf index.
General recursion is feasible in SQL.
A generic operator interpreter still amplifies work.

## Next compiler work

1. Derive exact demanded nodes before broad pattern and regex batches.
2. Collapse capture-bearing branches to the parent key and required captures before joining them.
3. Lower common recursive utility components into typed relational recurrences.
4. Keep the continuation CTE for semantic experiments while proving specialized replacements.
5. Use cardinality metadata during generation to choose anchors, join order, and materialization.
6. Install and cache complete physical programs across calls, including input and profiler lifecycle.
7. Extend complete-call parity and statement-count checks before addon integration.

The hottest remaining clauses in the current profile include `ruff-f842-unused-annotation` and `ruff-f823-undefined-local`.
One intermediate has 59,336 rows and many capture columns.
Automatic indexes and unsuccessful probes consume work even when the final clause returns no matches.
Reducing statement count and publication alone does not remove that logical work.

## Reproduction

The ignored workload test uses the existing `corpus`, `capabilities`, `policies`, and `files` configuration.

```sh
RERAIL_WHOLE_CALL_CONFIG=/absolute/path/config.json \
RERAIL_WHOLE_CALL_OUTPUT=/absolute/path/artifacts \
RERAIL_WHOLE_CALL_PROFILE=1 \
RERAIL_WHOLE_CALL_CASES=shaped-8-1-All,shaped-64-4-All \
cargo test --profile loop --features unbounded-work \
  --manifest-path native/Cargo.toml whole_call_corpus -- --ignored --nocapture
```

For native loop counters, add `LIBSQLITE3_FLAGS=-DSQLITE_ENABLE_STMT_SCANSTATUS` to the build environment.
For the combined recursive experiment, select `RERAIL_WHOLE_CALL_CASES=recursive-machine`.
The harness saves coverage, generated SQL, metrics, plans, and rejected-candidate status.
These case selectors exist only in ignored tests.

Local artifacts for this investigation are under `/tmp/rerail-sql-generation-20261021`.
They include frozen binaries, configuration hashes, source hashes, raw plans, and repeated timing results.
The existing macOS environment also requires its `JEMALLOC_OVERRIDE` shim.

## Validation

The final frozen profiling binary passes all 185 SQL tests.
The focused whole-call group passes 26 tests, including the expanded composite-capture and 818-rule seed cases.
The existing profiling and query-shape script suites pass all 12 tests.
Clippy passes for the profiling test build and the ordinary production library.
Formatting, boundary checks, Windows path checks, and whitespace checks pass.

The repository structure check still reports its two existing failure categories.
They concern benchmark fixture locations and fixture includes from `policy_guards.rs` into `packs/`.
This work adds no new structure findings.
