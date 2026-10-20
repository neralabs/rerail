# SQL composition profiling and tuning

This report records the experiments before promotion.
The [implementation and compiler spike](sql-set-composition-implementation.md) describes the current defaults and subsequent measurements.

The largest measured costs come from candidate expansion and SQL join order.
Batch size and cache size do not explain most of the gap.
This report extends the [initial investigation](sql-set-composition-investigation.md).

Three interleaved repetitions give these median apply CPU times:

| Rules | Engine | CPU seconds | SQL/native ratio |
| --- | --- | ---: | ---: |
| Original supported catalog | Native | 2.733 | 1.00× |
| Original supported catalog | Default SQL | 33.076 | 12.10× |
| Original supported catalog | Physical SQL changes | 17.823 | 6.52× |
| Original supported catalog | Physical changes plus answer-difference rewrite | 17.926 | 6.56× |
| Two root filters | Native | 1.276 | 1.00× |
| Two root filters | Physical SQL changes, batch 2 | 7.731 | 6.06× |

Physical SQL tuning reduces CPU by 46.1% without changing policies.
The combined policy and physical changes reduce SQL CPU by 76.6%.
Native benefits from the filters too, so the remaining comparable gap is about sixfold.
All eighteen runs preserve the same complete output with zero failures.
The repeated comparison does not support the answer-difference rewrite.
The raw results are in `confirmation.json` under the artifact directory below.

A larger scan checks all 138 manifest files, including 106 files outside the tuning subset.
These are single-run validation measurements using the same final binary:

| Rules | Native CPU seconds | Physical SQL CPU seconds | SQL/native ratio |
| --- | ---: | ---: | ---: |
| Original supported catalog | 15.788 | 113.508 | 7.19× |
| Two root filters | 7.374 | 41.142 | 5.58× |

All four runs return the same 580 full finding objects with zero failures and zero blind files.
The larger scan confirms output parity and a substantial remaining performance gap.
It does not provide a completed default-SQL baseline for the supported catalog.

## Workload and measurement

The tuning workload contains 135 supported Python policies, 146 clauses, and 32 pinned data-layer files.
The files contain 1,318,099 source bytes.
The native oracle returns 33 findings, zero failures, and zero blind files.
Its full-object digest is `16284c2e6cf80a95761f5b3e6510f30d3df32f69de72fc4a1685556b130a736c`.
Comparisons include captures, fixes, suppression state, status, and failure objects.
Only the outer finding and failure arrays are sorted.

Each measurement uses a fresh process and one cold workspace apply.
Compilation and registration are timed separately.
CPU time is process CPU, summed across worker threads.
Wall time is unreliable on this heavily shared M5 Max host.
Traced runs collect statement counters and query plans; they are excluded from latency comparisons.

The release build enables `unbounded-work` and uses thin LTO with sixteen codegen units for practical iteration.
Both engines use the same allocator compatibility shim, described in the initial investigation.
These diagnostic builds do not establish production throughput.
Tim's external twelve-policy catalog remains unavailable, so the original 818-clause claim remains unverified.

## Expensive SQL shapes

The B020 trace records about 945 million VM steps and 92 million full-scan steps.
Dependency insertion and answer lookup dominate its statement CPU.
The answer-difference expression accounts for only about one percent.

| Statement family | Traced CPU seconds | VM steps, millions | Full-scan steps, millions |
| --- | ---: | ---: | ---: |
| Dependency insertion | 8.665 | 325.0 | 41.9 |
| Answer lookup | 6.729 | 156.7 | 48.8 |
| Dirty propagation | 3.548 | 147.9 | 0.0 |
| Demand insertion | 2.292 | 56.2 | 0.0 |
| Dependency deletion | 0.722 | 186.2 | 0.0 |
| Dirty reset | 0.690 | 34.5 | 0.0 |
| Answer difference | 0.267 | 12.6 | 0.0 |

Zero full-scan steps does not mean an efficient operation.
An indexed scan over a broad prefix can still execute millions of VM instructions.
Returned and affected row counts also exclude scanned candidates.

The combined physical changes preserve the trace's 17,089 statement executions and 4,566,001 affected rows.
VM steps fall from 945,294,460 to 174,140,945, an 81.6% reduction.
Full-scan steps fall from 92,344,359 to 1,186,395, a 98.7% reduction.
These counters cover B020 statements, not the complete policy catalog.

### Demand joins choose the large outer relation

The generated dependency statement has this shape:

```sql
INSERT INTO edges
SELECT DISTINCT source_unit, f.source_id, target_unit, d.id
FROM input_frame f
JOIN demand d
  ON d.file = f.file AND d.node = f.node AND d.c0 IS f.s0;
```

SQLite chooses `SCAN d`, followed by a constrained virtual-table lookup into `f`.
One traced statement executes 514 times and performs 30,370,640 full-scan steps for 63,904 inserted rows.
The equivalent answer lookup performs the same scans for only 1,078 output rows.
Peak demand cardinality reaches 422,424 rows while the input frame reaches 3,000 rows.

Forcing the frame first with `CROSS JOIN` lets SQLite probe the demand key index.
This reduces the complete workload from 33.23 to 21.69 CPU seconds in a paired screening run.
The current frame scan estimate is a fixed 1,048,576 rows.
That estimate helps explain the bad choice, but it cannot describe every runtime frame.
The durable fix is cardinality-aware planning or a specific join-order constraint where the input is known to be smaller.
Cached statements and empty frames during preparation require care when exposing runtime cardinalities.

### Dependency tables duplicate set membership work

The default table has four untyped columns and two separate indexes.
Each insertion also builds a temporary B-tree for `DISTINCT`.
Repeated recursive evaluations can insert an edge that already exists from another insertion.

The experimental layout makes set membership explicit:

```sql
CREATE TEMP TABLE edges (
  su INTEGER, s INTEGER, du INTEGER, d INTEGER,
  PRIMARY KEY (su, s, du, d)
) WITHOUT ROWID;
CREATE INDEX edges_to ON edges (du, d);
```

`INSERT OR IGNORE ... SELECT` then replaces insertion-local `SELECT DISTINCT`.
The primary key serves source lookups; the destination index carries the primary-key columns implicitly.
The untyped unique layout reduces CPU from 36.65 to 26.57 seconds in the first screening run.
Widening both original indexes yields a smaller improvement, to 33.29 seconds.

The IDs also need declared integer affinity.
For `s IN (SELECT k FROM keys)`, `keys.k` is an integer primary key.
The untyped source index is searched only by `su`, then filtered by `s`.
A minimal SQLite reproduction shows that integer edge columns enable an index search on both `(su, s)`.
One traced deletion executes 81 million VM instructions without deleting a row.
The engine trace confirms `SEARCH ... USING PRIMARY KEY (su=? AND s=?)` after the combined changes.
Across dependency deletions, VM steps fall from 186.2 million to 5.7 million.

### Dirty propagation starts from too many edges

The default statement searches edges by `su`, then probes the changed-key frame:

```sql
UPDATE demand SET dirty = 1 WHERE id IN (
  SELECT e.s FROM edges e JOIN changed c
    ON c.u = e.du AND c.id = e.d
  WHERE e.su = :unit
);
```

One batch has 246,625 edges and only 202 changed keys.
Starting from `changed` and probing the destination index avoids the broad source-prefix scan.
The experiment uses `changed c CROSS JOIN edges e` with the same predicates.
This requires a suitable destination index and should be measured with the chosen edge layout.
The combined trace uses `SEARCH e USING COVERING INDEX ... (du=? AND d=? AND su=?)`.
Dirty-propagation VM steps fall from 147.9 million to 0.46 million.

### Answer difference is a low priority

The compiler computes symmetric difference with `EXCEPT`, `UNION`, and another `EXCEPT`.
A pair of null-safe left joins with `UNION ALL` preserves the tested outputs.
It provides no clear standalone improvement: 34.21 versus 33.23 CPU seconds in one screen.
Its small profile share makes further work less valuable than candidate and join fixes.

## Candidate filtering changes the amount of work

B020 binds an identifier before recursively checking whether it belongs to a loop target.
This creates many demand keys containing captures from irrelevant identifiers.
The experiment adds a capture-free necessary condition before the original rule:

```json
{
  "all": [
    {"inside": {"kind": "for_statement", "field": "left", "stopBy": "end"}},
    "original root rule, unchanged"
  ]
}
```

The JSON above illustrates the wrapper; the second element represents the original rule object.
The existing `in-target` utility requires this condition.
The wrapper therefore filters candidates before the expensive capture-bearing recursion.
It preserves policy identity, revision, and the complete observed output.

On four upstream positive fixture files, both versions return the same 31 full finding objects.
Default SQL drops from 2.890 CPU seconds to 0.104 seconds.
The original native matcher and the rewritten native matcher also agree on all 138 manifest files: 580 findings and zero failures.
The filter also speeds up native execution, from 16.99 to 10.35 CPU seconds in that larger scan.
This is a rule-search improvement, separate from SQL physical tuning.

On the 32-file workload, default SQL drops to 9.793 CPU seconds after this filter.
Additional join and layout changes provide little extra benefit there: 9.573 seconds in the first screen.
The remaining policies now dominate.

B023 then creates 518,921 demand rows carrying captured identifiers.
Its demand-insertion query already uses the complete covering key index.
The next problem is candidate volume, rather than another missing index.
A second filter requires a surrounding loop and a surrounding function or lambda before the original B023 rule.
Both filters preserve the 31 positive fixture findings and all 33 findings on the 32-file workload.
Default SQL falls to 8.010 CPU seconds; native falls to 1.363 seconds.
The resulting SQL/native ratio is still 5.88×.
Comparing improved SQL only with the original native time would overstate progress toward parity.

The compiler should derive conservative capture-free root conditions where it can prove necessity.
It must preserve capture order, first-match behavior, negation, unknown results, and recursive utility semantics.
A general optimizer needs broader differential coverage than this single-policy experiment.

## Parameter screens

These initial screens contain one run per setting and support direction, not precise confidence intervals.
Comparisons use their own same-build controls.

| Setting | Measured result | Decision |
| --- | --- | --- |
| Requested batch size 1, 2, 4, 8 | 36.68, 37.88, 39.76, 36.27 CPU seconds | Use 1 for this workload's lower memory use |
| Worker count 1, 2, 4 | 30.56, 31.40, 36.65 CPU seconds | Screen throughput separately on an idle machine |
| Frame scan estimate 64, 1,000, 10,000 | 21.53, 20.61, 21.72 CPU seconds | Correct join choice matters more than the precise estimate |
| Estimate 1,000 plus explicit join/layout changes | 18.46 CPU seconds | No additive benefit over 18.39-second combined control |
| TEMP page cache 16 MiB | 33.53 CPU seconds | No compelling benefit |
| Automatic indexes disabled | 54.56 CPU seconds | Keep enabled |
| Sort threads 4 | 32.46 versus 36.65 CPU seconds | Insufficient repeat evidence; keep default |

After both candidate filters, batches 2 and 8 take 7.680 and 7.708 CPU seconds, versus 8.010 for batch 1.
Workers 1 and 2 take 6.996 and 7.260 CPU seconds, but wall times increase to 12.33 and 7.39 seconds.
The four-worker batch-2 screen finishes in 5.45 seconds on this host.
Choose worker count for the actual latency and memory objective; CPU efficiency alone does not select the fastest configuration.
Changing the scan estimate to 1,000 takes 7.940 CPU seconds after the filters, providing little remaining benefit.

Batch size 1 peaks around 559 MiB versus 706 MiB for batch size 8 in the original binary.
The engine caps effective batch size at `ceil(file_jobs / 4)`.
For these 32 files, requesting more than eight does not test a larger batch.
The worker override changes physical concurrency; the job-partition constant remains four.

The mutable relations live in TEMP, so MAIN `cache_size` does not target their page cache.
In-memory TEMP storage also limits what a page-cache budget can accomplish.
The benchmark records both schemas' actual cache settings.

Disabling `SQLITE_ENABLE_MEMORY_MANAGEMENT` reduces one baseline from 33.99 to 30.42 CPU seconds.
The combined variant drops from 19.98 to 18.36 seconds.
This removes a global page-cache mutex, separate from the already disabled memory-status counter.
These are single-run comparisons.
The application calls `sqlite3_release_memory`; disabling memory management changes its reclamation behavior.
Keep the existing build option until repeated throughput and eviction tests justify a change.

## Closing the remaining gap

Apply root restrictions before binding captures, then repair the demonstrated demand and dependency access paths.
Those changes have the strongest evidence.
Keep the answer-difference rewrite separate because its profile share is small.

After the access-path fixes, large demand insertion becomes the next visible cost.
For rules such as B023, consider relations keyed by `(loop, name)` and `(function, name)`.
Join those relations to candidate identifiers before constructing full capture environments.
This could avoid repeated name checks and large identifier/function cross-products.
It requires a semantics-preserving compiler transformation, rather than another global SQLite parameter.

Bound pattern terminals also construct text keys in materialized SQL:

```sql
WITH e AS MATERIALIZED (
  SELECT k, file || ',' || node || ',' || COALESCE(s0, '') AS ek
  FROM input_frame
), p AS MATERIALIZED (
  SELECT ek, v0 FROM pattern
  WHERE term = :term AND ek IN (SELECT ek FROM e)
)
INSERT INTO output_frame
SELECT f.k, f.file, f.node, p.v0
FROM input_frame f JOIN e ON e.k = f.k JOIN p ON p.ek = e.ek;
```

The pattern and fact terminals split these keys back into fields in Rust.
After the access-path fixes, one such pattern statement appears among the largest remaining statement costs.
An interned integer key for the typed input tuple could remove text formatting, parsing, and text-key joins.
This opportunity is inferred from the shape and source; it has not been benchmarked here.
Preserve batching and key equality, including absent captures and cross-file values.

Next, measure intermediate frame construction and fuse cheap adjacent relational operations where profitable.
The existing engine already reuses prepared statements and grows the cache with active SQL texts.
Its initial four-entry cache is not a permanent four-statement limit.
Increasing a fixed cache setting therefore does not address the measured problem.
Preserve batched terminal materialization; earlier correlated terminal experiments regressed.

NER-1562's packed relational blocks offer a possible implementation for reducing intermediate frame traffic.
Tim's SQL lowering can remain the differential oracle for such execution changes.
The historical 17.189-second NER-1562 run still measures a native-matcher callback, as the initial report explains.
It cannot validate the throughput of that later relational compiler.

## Reproducing the experiments

The [tuning harness](../benchmark/sql-set-tuning.mjs) alternates variant order and checks a full-output oracle.
The [shape aggregator](../benchmark/sql-set-shapes.mjs) groups exact statements and preserves their query plans.
All experimental controls require the `unbounded-work` build.
During these experiments, production SQL and build options remained unchanged.

```sh
node benchmark/sql-set-tuning.mjs /absolute/path/sweep.json
node benchmark/sql-set-shapes.mjs /absolute/path/traces /absolute/path/shapes.json
```

The sweep file specifies `baseConfig`, `output`, `rounds`, `expectedDigest`, and `variants`.
Each variant can override `addon`, `engine`, `batchSize`, `threads`, `policies`, and `environment`.
The base configuration follows the initial investigation's format.
The harness records addon hashes, policy hashes, source hashes, CPU, wall time, memory, and the engine report.

The physical variant uses these environment settings:

```json
{
  "RERAIL_SQL_DEMAND_ORDER": "input",
  "RERAIL_SQL_EDGE_LAYOUT": "unique",
  "RERAIL_SQL_EDGE_TYPES": "integer",
  "RERAIL_SQL_DIRTY_ORDER": "changed"
}
```

Use `batchSize: 1` for the original-policy comparison and `batchSize: 2` for the filtered-policy candidate.
Both use four physical workers.
Leave automatic indexes enabled, and retain default cache sizes, sort threads, and memory-management build flags.
The fixed frame scan-cost override is unnecessary with the explicit demand join order.

The [candidate-filter generator](../benchmark/sql-set-candidate-filter.mjs) writes separate B020 and B023 diagnostic policy variants.
It preserves identity and revision so full-output comparisons remain meaningful.
Do not register an original policy and its variant in the same workspace.

```sh
node benchmark/sql-set-candidate-filter.mjs input.policy.json /tmp/filtered.policy.json
```

Raw configurations, binaries, traces, and outputs are in `/tmp/sql-set-tuning-20261020` on the investigation host.
The baseline statement trace is `shapes-base.json`.
The initial screens are `settings-screen.json`, `shape-screen.json`, `cost-screen.json`, and `memory-screen.json`.
The positive rule-filter checks are `positive-screen.json` and `positive-pruned.json`.
The 32-file filter comparison is `pruned-screen.json`.
The repeated finalists are in `confirmation.json`.
The larger validation is in `heldout-final.json` and `heldout-native-control.json`.
The improved B020 statement counters and plans are in `shapes-physical.json`.

## Validation

The final physical variant passes 107 existing SQL tests, with nine ignored tests.
This includes 1,239 generated differential cases, replay cases, query-plan checks, and three-valued logic cases.
The optional join-based answer difference passes the same suite with the physical settings enabled.
The larger 138-file runs preserve 580 full finding objects, zero failures, and zero blind files.
These experiments used separate policy variants.
The later implementation promotes both guards into the policy pack.
The production build passes `cargo check --release --lib` without `unbounded-work`.
Rust formatting, JavaScript syntax checks, and whitespace checks also pass.
