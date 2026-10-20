# SQL composition performance investigation

The follow-up [profiling and tuning report](sql-set-composition-tuning.md) measures SQL shapes, candidate filters, and parameter changes.
The [implementation report](sql-set-composition-implementation.md) identifies the promoted defaults and the subsequent compiler spike.

Tim's branch and NER-1562 share the goal of SQL composition over batched native terminals.
Their recorded timings do not establish equivalent workloads or equivalent execution paths.
The 2.5× claim requires Tim's external policy catalog and raw results before a direct reproduction is possible.

A separate probe uses 135 supported Python policies with 146 clauses over 32 pinned files.
Three alternating runs preserve the same 33 full finding objects and zero failures on both engines.
SQL uses 13.79× the native apply CPU on that workload.
B020 accounts for about 79% of summed SQL clause time.
Omitting that rule reduces SQL apply CPU by 76.3% while preserving this subset's output.
That omission is a diagnostic, not an implementation improvement.

The inspected Tim revision is `c575c9e4` from `origin/Moti-Velz/sql-set-composition`.
The Rerail NER-1562 comparison uses `e2674b52`.
The separate Landing NER-1562 checkout was at `173cc0145` during inspection.
The data-layer worktree contains uncommitted relational-query work based on `6cb8dc06`.
The investigation preserves those other worktrees.

The checked-in loop manifest identifies 818 clauses and twelve policy hashes.
It selects 138 files with 7,336,001 source bytes from data-layer revision `fce4dc7208c0b892c11a549c8be43fc285580d6b`.
Its calibration field is null.
Neither that manifest nor the branch contains the actual twelve policies.
The expected external directory, `~/rerail-bench/data-layer/policies`, is absent on this machine.

The full benchmark requires the exact policies and all 5,725 files with 95,697,853 source bytes.
The loop's ratio depends on its workspace grouping, which repeats parsing across units.
A ratio from those units is not automatically the full-tree ratio.
The branch's [benchmark instructions](../benchmark/README.md) distinguish the loop, calibration, and whole-tree runs.

The current whole-tree harness hashes sorted finding and failure lines.
Those lines include the rule, clause, code, path, span, and message.
They omit full capture records, fixes, and original result ordering.
Thus, identical line digests establish a narrower claim than complete response equality.
The manifest also marks 717 of 818 clauses as producing no finding on the complete clean corpus.
The mutation stage provides necessary positive cases for those clauses.
The benchmark instructions describe 159 selected mutations from 437 recorded mutations.

The NER-1562 timings have these distinct meanings:

| Recorded workload | Result | Interpretation |
| --- | ---: | --- |
| Historical callback SQL, twelve policies and 829 clauses | 1,404.741 s | Complete full scan with 23 failures |
| Rerail NER-1562, same 829-clause catalog | 17.189 s | SQLite demands the existing native matcher through a batch callback |
| Earlier production-budget SQLite diagnostic | 14.833 s | 7,953 failures, so substantial work stopped early |
| Native control for that production-budget diagnostic | 8.983 s | Different violation count from SQLite |
| Landing resident benchmark, sixteen bundled rules | 5.406 s | Different rules and restored tapes |
| Landing transducer comparison, eight literal terminals | 359.220 ms | Matching only, with preparation excluded |

The [component compiler report](https://github.com/neralabs/rerail/blob/e2674b52/benchmark/history/ner-1562-component-compiler-and-real-catalog-2026-10-18.md) records the first four relevant measurements.
The sixteen-rule and eight-terminal results belong to the separate Landing repository's benchmark history.
They do not measure the twelve translated data-layer policies.

Another possible source of the near-native claim is the separate `ast-index-spike` result quoted in Tim's design.
That result reports 1.22× native for SQL pattern matching through a sibling NFA.
It uses set semantics and lacks native first-match behavior.
It therefore does not establish equivalent full-policy performance.

The 17.189-second result needs particular care.
At measurement commit `a4339124`, `workspace/evaluate.rs` supplies a closure to `sql.drive`.
That closure calls the existing `evaluate_file` implementation across the file batch.
SQLite reads completion and match indexes after native evaluation constructs the witnesses.
This result measures native composition behind a SQLite demand boundary.

NER-1562 later deleted that path and introduced the relational compiler.
Its [current benchmark status](https://github.com/neralabs/rerail/blob/e2674b52/docs/sql-relational-compiler-design.md#current-benchmark-status) explicitly rejects the old timing as evidence for the replacement.
The current compiler also rejects clause families outside `Forbid(Query::Ast)`.
Its recursive utility lowering supports restricted forms and can emit recursive CTEs.
A current full-catalog timing requires successful lowering and new parity evidence first.

The implementations differ at several important boundaries:

| Boundary | Tim at `c575c9e4` | Rerail NER-1562 at `e2674b52` |
| --- | --- | --- |
| SQL execution | Multiple statements over frames, with host loops | One catalog statement over a materialized packed relation |
| Composition state | Typed capture columns with backward liveness | JSON capture environments and ordered choice paths |
| Structural leaves | Native ast-grep terminals over demanded input keys | Verified packed programs with Metal or CPU JIT backends |
| Tree access | Tape-backed node virtual table and kind postings | Explicit navigation producer rows |
| Recursion | Batched demand loops with needed-row and cycle checks | Restricted recursive utility lowering through recursive SQL |
| Unsupported clauses | Explicit SQL refusals, with native available as the oracle | Catalog lowering fails without native fallback |
| Result status | Rendered corpus parity is the stated gate | Typed producer completion and failure propagation are explicit contracts |

Both designs batch work, retain stable identities, separate terminals from composition, and preserve deterministic choices.
Tim's demand keys also include the capture inputs that affect native first-match behavior.
The NER-1562 packed ABI, verifier, status rows, and bounded result pages provide useful terminal boundaries.
Those components can attach to Tim's composer without adopting the whole catalog statement or JSON capture representation.
The lower-risk integration direction is Tim's composer with selected NER-1562 terminal components.
That direction retains Tim's capture-sensitive first-match lowering and existing differential gate.
It still requires a shared full-catalog benchmark before a performance decision.

Tim's earlier profile assigned about 48% of execution time to frame B-tree writes and index lookups.
That profile predates the current columnar frame implementation.
The current [frames implementation](../native/src/code_rule_dsl/sql/frames.rs) replaces append-and-clear tables with Rust buffers and lazy indexes.
Tables that require conditional updates or deletes still use B-trees.
Current timing must distinguish virtual-table transitions, index construction, remaining B-trees, and statement preparation.

The fresh Python-pack diagnostic exposes a specific contention point.
A three-second sample contains 1,072 worker samples, including 300 waits inside SQLite's page-cache mutex.
Those waits represent 28.0% of sampled worker wall states in that window.
Other hot stacks contain B-tree inserts, record comparisons, and frame lookups.
These sample counts do not measure the complete run's CPU shares.

The bundled `libsqlite3-sys` version is `0.38.2`.
Its build enables `SQLITE_ENABLE_MEMORY_MANAGEMENT`.
In SQLite's `pcache1Init`, that option forces all page caches into one shared group.
The page-cache access macros then use the shared LRU mutex.
Disabling memory statistics through `SQLITE_DEFAULT_MEMSTATUS=0` does not remove this separate mutex.

An isolated build experiment can compare `-USQLITE_ENABLE_MEMORY_MANAGEMENT` while retaining SQLite thread safety and the existing flags.
That experiment must also preserve eviction behavior.
The engine currently calls `sqlite3_release_memory`, whose global reclamation depends on this feature.
Connection-local cache reclamation or connection closure needs a separate memory check before a production change.

Several additional optimizations already exist at the inspected revision:

- Backward liveness narrows the capture columns of each frame.
- Distinct demand keys prevent repeated utility work on equivalent environments.
- Empty-input rules skip statements, clears, and recursive blocks.
- Sessions retain prepared statements across batches.
- Frame registers and SQL skeletons reduce duplicate program storage.
- Single-consumer stages run only after their consumer has roots.

The design records 2.85 million program steps, 411 shapes, and about 15 MiB of stored SQL text.
These figures describe that design's data-layer program, rather than fresh measurements from this investigation.
They identify compilation, dispatch, and SQLite transitions as candidates for measurement.
They do not prove which candidate dominates the current full scan.

Two profile details affect attribution.
First, `profile.clauseFiles` divides a clause's batch wall time evenly among the files in that batch.
It does not measure each file's own cost.
Second, terminal time sits inside statement time, and stage time includes its statements.
Adding those fields double-counts work.
The statement ranking also uses wall time, which can change under concurrent system load.

The separate data-layer worktree uses user-authored relational queries over extracted facts.
Its five-file workload contains 1,026,452 source bytes and takes 14.810 seconds without sampling.
That worktree's report attributes 44.7% of cold CPU to table materialization and 20.6% to index construction.
Its descendant closure alone accounts for 27.1% of cold CPU.
That engine constructs recursive closure tables and repeats syntax and ownership relations across three scopes.
These costs differ from Tim's tape-backed node scans and columnar frames.

Useful next experiments follow from these differences:

1. Freeze both policy catalogs and compare their clause IDs and source hashes.
   The current evidence does not explain the change from 829 clauses to 818.
2. Run three alternating full-tree comparisons with four workers and lifted work allowances.
   Compare complete outputs, refusals, failure codes, CPU time, wall time, and peak memory.
3. Measure policy compilation, rule registration, and apply time separately.
   Tim's SQL program compiles during rule registration, before the whole-tree apply timer.
4. Profile the costliest current statements and their rows per input key.
   A representative single-file rerun can identify file costs without dividing batch time equally.
5. Compile selected capture-free subrules into batched transducer terminals.
   Preserve the SQL lowering as the definition and differential oracle.
6. Port NER-1562 terminal verification, bounded output, and completion records independently.
   Compare a Rust interpreter before Metal or JIT execution on those same terminals.
7. Measure utility-component compilation reuse from NER-1562.
   Its report reduced catalog compilation from 32.72 seconds to 2.244 seconds with one thread.
8. Tune batch size after attribution, while measuring session reuse and peak memory.

Memory limits also need a byte measurement.
The frame store caps live rows at `1 << 26`, while row widths vary with capture liveness.
That row limit alone does not enforce the design's 1 GiB memory target.

The [existing emission experiments](sql-set-composition-design.md#decisions-from-the-ast-index-spike-review) constrain further SQL changes.
Inlining materialized terminal reads caused per-row terminal execution and increased loop CPU by 93%.
Replacing the current semijoins with correlated `EXISTS` also caused regressions.
Those transformations require new measurements on a specific remaining hotspot.

The added [comparison runner](../benchmark/sql-set-investigation.mjs) uses fresh child processes and alternates engine order.
It records compilation, rule registration, apply CPU, apply wall time, output digests, and the complete SQL engine report.
The setup timer also includes the first engine report, which finalizes the SQL program before apply.
The total CPU comparison includes compilation, setup, and apply regardless of this boundary.
It hashes full violation and failure objects after sorting them, and preserves each object's internal array order.
It also records the addon hash, policy hashes, capability hash, corpus hash, failure codes, and system load.
It retains failures in comparisons and returns a failing exit code for different outcomes.
It requires an `unbounded-work` addon and checks the actual worker count and selected engine.

The runner accepts an external JSON configuration with `addon`, `capabilities`, `corpus`, `files`, `policies`, and `output` paths.
The optional fields are `rounds`, `threads`, `batchSize`, and `timeoutMs`.
`files` contains corpus-relative paths, and `policies` contains absolute policy file paths.

```sh
node benchmark/sql-set-investigation.mjs /absolute/path/config.json
```

The local diagnostic configuration and raw outputs belong under `/tmp/sql-set-investigation-20261020/`.
The diagnostic uses 138 shipped Python policies with 151 clauses.
Its first run uses the manifest's 138 source files.
Native apply takes 11.309 seconds of wall time and 15.903 CPU-seconds, with 580 findings and zero failures.
SQL reaches the 300-second process deadline without a completed output.
That SQL attempt includes the three-second sample described earlier.
The remaining large-probe repetitions stop after the timeout.
A smaller probe uses the first 32 manifest files and 1,318,099 source bytes.
Three alternating runs of the complete Python pack produce identical full finding objects on both engines.
Native returns 33 findings and no failures.
SQL returns the same findings plus 160 `sql-unsupported` failures.
The refusals affect five clauses across F405, F632, and F722 policies.
Each refusal concerns `violation.each` over a parsed tree's capture.
These runs therefore fail complete outcome parity.

The supported-catalog comparison removes those three policies from both engines.
It retains 135 policies and 146 clauses.
The B020 omission probe removes one additional policy and clause.
Each probe uses three alternating comparisons with the same addon, four workers, and lifted work allowances.
All twelve runs return identical full finding objects, zero failures, and zero blind files.

| Catalog | Native apply CPU | SQL apply CPU | SQL CPU ratio | Native apply wall | SQL apply wall |
| --- | ---: | ---: | ---: | ---: | ---: |
| Supported catalog | 2.691 s | 37.106 s | 13.79× | 2.078 s | 52.794 s |
| Supported catalog without B020 | 1.728 s | 8.782 s | 5.08× | 1.287 s | 10.223 s |

These values are medians of the three runs for each engine.
The supported SQL runs use 36.839–37.300 CPU-seconds despite substantial wall-time variation.
Their compile and setup CPU together add about 0.2 seconds.
The supported comparison's total CPU ratio, including compilation and setup, is 13.32×.

The [supported-catalog results](/tmp/sql-set-investigation-20261020/supported-python-pack-32-files.json) contain complete fingerprints and counters.
The [B020 omission results](/tmp/sql-set-investigation-20261020/supported-minus-b020-32-files.json) record the second catalog explicitly.
The [three-second sample](/tmp/sql-set-investigation-20261020/python-pack-sql-round0.sample.txt) belongs to the separate 138-file attempt.

The completed profile identifies these costs:

| Measurement | Diagnostic result |
| --- | ---: |
| Executed statements | 24,204 |
| Rows affected or returned, as counted by the driver | 21,312,453 |
| Prepared statements across sessions | 7,625 |
| SQL statement CPU as a share of apply CPU | About 94% |
| Semantic fact terminal CPU as a share of apply CPU | Less than 1% |
| B020 share of summed clause wall times | About 79% |

The dominant rule is `ruff-b020-loop-variable-overrides-iterator`.
Its capture-sensitive recursive utilities dominate dependency-edge insertion, answer joins, and dirty-key propagation.
One dependency-edge insertion statement runs 233 times and contributes 15.753 seconds of summed wall time in the first supported run.
Another answer join runs 233 times and contributes 10.525 seconds.
The next-largest clause is B023, `function-uses-loop-variable`.
These measurements support a focused experiment on recursive demand propagation before broad terminal acceleration.

The recursion loop already evaluates only dirty keys and records answer changes.
Adding a generic semi-naive algorithm would repeat an existing optimization.
The mutable demand, answer, and dependency tables remain B-trees because the frame replacement excludes conditional updates and deletes.
Useful experiments include compact mutable storage for those tables and conservative filters for possible loop-target roots.
Those experiments must preserve first-match bindings, negative dependencies, and the recursion cycle checks.
They are proposed experiments rather than demonstrated optimizations.

This diagnostic does not substitute for the missing twelve-policy benchmark.
The machine's load exceeded 60 during preparation.
The build uses the existing jemalloc compatibility shim because the local command guard prevents jemalloc's configure cleanup.
Any diagnostic timing therefore requires these machine and allocator qualifications.
