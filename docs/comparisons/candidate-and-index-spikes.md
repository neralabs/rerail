# Candidate selection and bulk index experiments

## Question and baseline

Composition currently exposes broad structural relations to SQL before semantic producers screen their demands.
The corrected compiler still performs about 162,000 call-site, attribute, and text cursor openings on the full corpus.
An attribute membership rewrite reduces cursor openings but doubles warm query CPU.
The batch lookup still performs two binary searches for each key.

These experiments test candidate selection and index traversal separately. Production behavior remains unchanged during the experiments.
The baseline is commit `a31964bac` on `jeremy/async-family-integration-comparison`.

## Candidate contract

Inputs specify the document/world scope, execution context, possible qualified targets, and possible member names.
Outputs contain the call, report node, reference, receiver, and candidate classification.
Node identifiers retain document/world identity. The bound immutable batch supplies the experiment's scope.
A production compiler must preserve narrower rule scopes explicitly before it shares a producer.

Direct candidates pass the existing conservative import screen. Exact semantic qualification still determines whether a target matches.
Receiver candidates pass a structural member-name test independently of direct qualification.
Unsupported qualification shapes remain admitted. Import aliases, rebinding, and shadowing require exact semantic confirmation.
Only confirmed API calls can create argument demands.

The experiment replaces three CTE bodies: `async_site`, `async_reference`, and `async_open_receiver`.
One materialized native candidate relation supplies these projections.
The spike specializes execution context `async` and member name `open`; these become explicit inputs in a general operator. Existing semantic producers and diagnostic SQL remain intact.
The target relation supplies the union of direct targets across the four rules.
Typed producer metadata identifies these relations. Their dependencies must not lead back to the candidate relation.
This replacement tests a physical contract; it is not a general compiler rewrite.

Compare these producer shapes:

1. Existing structural extraction followed by context and target screening.
2. A traversal of call postings with context selection before target screening.
3. A traversal of call postings with target screening before context selection.
4. Async-function body ranges that restrict the call postings before exact context selection.

Count visited calls, context checks, qualification screens, emitted candidates, direct candidates, and receiver candidates.
The range variant merges overlapping async bodies and retains exact context checks for nested execution boundaries.
Measure producer CPU inside the query. Charge structural extraction and index construction where they occur.

## Refined qualification contract

The first screen admits 3,083 non-name-headed expressions on this corpus.
The existing core helper returns `None` for both those expressions and unknown syntax.
The reference distinguishes definite non-qualification from unknown input.

A second spike ports that distinction over core facts and checks it against the reference on every node of targeted fixtures.
It drops well-formed call/subscript heads from direct qualification demands. Receiver demands remain independent.
Malformed expressions remain unknown and admitted.
A separate SQL experiment checks whether a demand relation is empty before it invokes its semantic producer.

## Index contract

Compare point probes, a bounded merge, galloping searches, and an adaptive choice.
Inputs are integer keys and a sorted posting index. Results retain every matching row.
Unsorted or duplicate request keys preserve the existing lookup contract through point probes.

The merge starts at the first requested key's lower bound and advances through keys and postings.
Galloping searches skip gaps from the previous index position. The adaptive choice uses demand size and covered index range.
Sparse requests must not force a scan across the complete index.

Measure comparisons and visited entries separately from timing. Include empty, duplicate, sparse, dense, clustered, and absent-key cases.
Also measure the attribute membership query over actual corpus inputs.
Separate SQL key-list decoding from native index traversal in the profile.
Keep the original lookup as a control alongside the instrumented point-probe implementation. Fewer virtual-table calls alone do not establish success.

## Preparation lifetime experiment

A sampled corpus run found 373 of 429 composed-executor samples under `sqlite3Reprepare`.
The executor calls `prepare_cached`, but batch setup repeatedly installs native, symbol, string, and aggregate functions.
These registrations can expire cached SQLite statements. Recompilation then occurs inside `sqlite3_step`.
Explicit preparation timing does not include this hidden cost.

A separate spike installs those functions once per connection.
SQLite client data owns the registration marker, including cleanup on close and protection against connection-address reuse.
A focused test compares repeated registration against stable setup through `SQLITE_STMTSTATUS_REPREPARE`.
When recompilation stopped, a separate input-version gate caught stale adopted bindings.
Their local owner disappeared after each run, while cached SQLite statements still referenced its old binding cell.
The revised spike retains those owners for the connection lifetime and replaces their sources in place.
The complete job records that counter for each composed execution.

Compare stable setup alone, typed candidate selection with stable setup, and a nonempty demand-frame gate.
The frame gate emits zero rows for an empty demand and drives the native producer through a `CROSS JOIN`.
This avoids rebuilding demand expressions for separate `EXISTS` guards.

## Gates

All modes must preserve full-corpus findings and all 178 recorded Ruff edge findings.
Candidate modes must preserve the 100-execution budget. Use the nonempty 24-file workload to check all four rule labels.
Record complete-job CPU, local-phase CPU, SQL VM instructions, query plans, and producer counters.
Use alternating unprofiled runs for CPU comparisons. Preparation and warm replay measurements remain separate.
The corpus timer includes input processing, program preparation, execution, and output writing.
Rule-file flattening and initial path enumeration occur before that timer, as in the existing comparison harness.
The tape/prepared-program store is warm; corpus and cross-file result memoization are disabled.

Acceptance requires less relevant work, lower measured CPU, and unchanged results.
Retain rejected shapes and their measurements. Do not infer a general compiler policy from one workload.

## Initial results and rejected interpretations

The first producer variants preserve findings and reduce SQL VM instructions from 3.39 million to about 0.92 million.
That reduction does not establish a CPU improvement.
Alternating unprofiled runs increase median local CPU from 0.990 to 1.145 seconds with the initial range producer.
Complete-job CPU changes from 11.695 to 11.828 seconds.
These results reject the initial range producer as a performance winner on its own.

| Producer strategy | Calls visited | Context checks | Qualification screens | Published candidates | Profiled producer CPU |
|---|---:|---:|---:|---:|---:|
| Context before screening | 691,328 | 691,328 | 65,292 | 3,093 | 84 ms |
| Screen before context | 691,328 | 38,389 | 690,248 | 3,093 | 434 ms |
| Context ranges before exact checks | 67,385 | 67,385 | 65,292 | 3,093 | 70 ms |

Ranges remove most call visits, but the first screen still admits 3,083 unnecessary direct candidates.
Preserving typed qualification eligibility reduces published candidates to ten receivers on this corpus.
Qualification demands fall from 261,170 to two. Exact origin resolution still receives ten requests.
The two qualification requests come from the downstream constructor path; none require exact qualification work here.
This is a corpus-specific result, not a promise that other projects need only twelve semantic requests.

A separate `EXISTS` guard duplicates the demand dependency in each producer query.
It preserves findings but increases profiled local CPU from 1.325 to 3.366 seconds before the preparation fix.
This shape is not accepted on the strength of its small VM count.

## Bulk lookup results

The lookup matrix uses 100,000 rows with two rows per indexed key.
Each timing is the median of seven samples with twenty calls per sample.
Index construction is reported separately. Timing runs disable per-comparison instrumentation.

| Demand | Point probes | Bounded merge | Galloping | Adaptive |
|---|---:|---:|---:|---:|
| 32 sparse keys | 1.95 µs | 38.30 µs | 1.95 µs | 2.15 µs |
| 4,000 clustered keys | 217.30 µs | 9.45 µs | 25.00 µs | 9.95 µs |
| 50,000 dense keys | 2,905.30 µs | 141.30 µs | 346.35 µs | 157.35 µs |
| 50,000 absent keys | 2,850.95 µs | 119.20 µs | 319.60 µs | 117.70 µs |

All strategies preserve rows, including duplicate and unsorted demands through the point-probe fallback.
Dense requests justify a merge. Sparse requests require point probes or galloping across gaps.
The adaptive choice uses demand cardinality and the covered posting range, rather than cursor count.

On the actual job, original attribute lookup takes 4.91 ms; adaptive traversal takes 1.30 ms.
SQL key-list decoding takes about 2.6 ms. Saving 3.6 ms cannot explain or repair the remaining local CPU cost.
The complete-job lookup profiles show no clear CPU winner.
The earlier doubled warm replay cost therefore must not be reported as a doubled complete-job runtime.

## Concrete target shape

The physical operator needs explicit context, target, member, and scope inputs.
The following SQL shows the intended interfaces. The experiment currently specializes the context and member inputs inside the producer.

```sql
WITH
candidate AS MATERIALIZED (
  SELECT call, report, reference, receiver, flags
  FROM python_candidates(
    :bound_world, :document_scope, :async_context,
    :qualified_targets, :receiver_members
  )
),
direct_demand AS MATERIALIZED (
  SELECT rr_demand_frame(reference) AS handle
  FROM candidate
  WHERE (flags & 1) != 0
  HAVING count(*) > 0
),
qualified AS MATERIALIZED (
  SELECT q.*
  FROM direct_demand AS d
  CROSS JOIN python_qualified(d.handle, :qualified_targets) AS q
),
receiver_demand AS MATERIALIZED (
  SELECT rr_demand_frame(receiver) AS handle
  FROM candidate
  WHERE (flags & 2) != 0
  HAVING count(*) > 0
),
origins AS MATERIALIZED (
  SELECT o.*
  FROM receiver_demand AS d
  CROSS JOIN python_origins(d.handle) AS o
)
-- Rule-specific joins consume candidates, qualified targets, and origins.
-- Confirmed calls create argument demands only where the rule needs arguments.
SELECT ...;
```

The native candidate operation performs one traversal over relevant posting ranges for the bound batch.
It checks exact execution context before qualification screening. It does not emit a global call-site relation for SQL to filter later.
Typed eligibility keeps malformed input conservative without admitting definite non-qualification as unknown.
Receiver candidates stay independent because constructor calls such as `Path('x').open()` need origin analysis.
SQL composes these facts through joins and demand-frame aggregates. Native code does not decide the final rule result.

The empty aggregate produces no frame row. Its `CROSS JOIN` therefore does not invoke the semantic virtual table.
This differs from a scalar frame subquery that produces a null argument and still opens a cursor.
It also avoids a second demand dependency through an independent `EXISTS` expression.

Index lookup receives a sorted demand vector, not a loop of scalar requests hidden behind one cursor.
The range-density estimate chooses merge or galloping traversal. Duplicate or unsorted demands preserve the existing point-lookup behavior.
Index construction, key decoding, and result copying remain separately charged costs.

Function and module installation belongs to the connection lifetime.
Batch bindings, pending symbols, and native demand frames still refresh for each execution.
When schemas or operator capabilities change, explicitly invalidate the affected plan. Ordinary batch replacement must preserve the compiled statement.
The compiler cache and SQLite statement cache cannot provide reuse while setup repeatedly expires those statements.

## Preparation and candidate composition

The final complete-job comparison records 71 automatic recompilations in the baseline and zero with stable setup.
These profiles use the final binary, including the binding-lifetime correction, and the same corpus. Alternating unprofiled timings provide the performance acceptance check separately.

| Shape | Local CPU | Composed query CPU | Automatic recompilations | Job SQL VM instructions |
|---|---:|---:|---:|---:|
| Baseline | 1.025 s | 0.774 s | 71 | 3,386,852 |
| Stable setup | 0.776 s | 0.508 s | 0 | 3,379,422 |
| Stable setup and typed range candidates | 0.536 s | 0.266 s | 0 | 829,973 |
| Stable setup, typed ranges, and independent `EXISTS` guards | 0.583 s | 0.292 s | 0 | 830,110 |
| Stable setup, typed ranges, and nonempty demand frames | 0.503 s | 0.250 s | 0 | 732,887 |

The query span includes native leaves and initial SQLite preparation. It excludes surrounding source processing and local-phase bookkeeping.
Baseline compiler time is 7.3 ms, and frame-driven compiler time is 12.1 ms.
Their explicit SQLite preparation costs are 4.0 ms and 8.1 ms.
The independent `EXISTS` guard costs 36.7 ms in compilation and 31.2 ms in explicit preparation.
Stable setup prevents that cost from recurring, but the frame-driven shape still avoids the duplicated demand dependency.
These one-time costs are acceptable. The baseline's implicit repeated compilation was not a one-time cost.
Cumulative query allocation falls from 1.02 GB to 247 MB. This measures allocated bytes across executions, not peak memory.

The 5,725-file job uses 80–81 executions, including setup and cross-file work, across 72–73 chunks.
Chunk boundaries vary with the runtime chunk budget. There is one composed local query per chunk and one cross-file query.
The local SQL text remains about 22 KB. Its structure changes by replacing broad structural CTEs with a narrow typed producer.

After stable setup and narrowed demands, the query takes about 250 ms of the 503 ms local phase in this profile.
Candidate production takes 90 ms inside that query.
Local page release takes 83 ms, and export/cleanup takes 33 ms.
The complete job still includes tape loading, projection, source construction, and other phases outside the local query.
Bulk attribute traversal cannot improve the winning candidate path: that path removes the broad attribute lookup entirely.

## Final timing and acceptance

Three alternating unprofiled pairs compare the baseline with stable setup, typed range candidates, and nonempty demand frames.
Both variants use the final binary. Each variant receives an excluded warmup run.

| Median | Baseline | Combined spike | Change |
|---|---:|---:|---:|
| Local user-plus-system CPU | 1.090 s | 0.503 s | −53.9% |
| Complete corpus-run CPU | 12.387 s | 11.765 s | −5.0% |
| Observed wall time | 17.106 s | 15.398 s | −10.0% |

The shared host also runs other builds. The wall-time difference is descriptive, not a causal speed estimate.
All three local CPU samples improve: baseline 1.059–1.091 seconds, combined spike 0.501–0.512 seconds.
The smaller whole-job improvement agrees with the measured worker costs outside SQL.

The [acceptance report](../../benchmark/async-family-integration/candidate-spikes/gates.json) passes every gate.
It records the baseline failing the new VM and automatic-reprepare ceilings.
The winning job uses 80 executions and 732,887 VM instructions, with zero automatic recompilations and ten published candidates.
The positive fixture retains all 120 expected findings within the previous 80,372-VM ceiling.
All 178 recorded Ruff edge findings remain unchanged. All nine composition tests pass with stable setup.
The qualification-eligibility comparison and seven-case bulk-lookup matrix also pass.

The [evidence bundle](../../benchmark/async-family-integration/candidate-spikes/README.md) contains commands, environments, profiles, SQL, EXPLAIN, timings, failures, and source/binary hashes.

## Remaining whole-job cost

The winning profile spends 6.80 CPU seconds in the batch worker and 2.96 seconds in the parse/tape worker.
Within batch construction, projection and interning consume 5.40 seconds of exclusive CPU. Text tests consume another 0.86 seconds.
The profile charges atom interning separately at 0.48 seconds inside the projection span.
These costs dominate the 250 ms query.

The spike rewrites SQL after planning. It therefore does not change the earlier `Projection::of(&prog, read)` calculation in `corpus.rs`.
`syntax::with_nearest` still builds the batch representation and its text-test inputs before the composed query runs.
This explains why a large local-query improvement gives a much smaller whole-job improvement.

Production integration must lower the typed candidate operator before projection and source-demand analysis.
Candidate selection can use core facts and document identity directly.
Only demanded semantic nodes and final report nodes then need relational IDs, interned text, and diagnostic fields.
That requires a demand-aware projection contract; merely replacing the generated SQL text cannot remove the worker cost.
This experiment measures that remaining cost but does not benchmark deferred projection itself.

## Integration decision

1. Install functions and modules once per connection. Keep binding owners alive and replace their sources between executions.
2. Add automatic-reprepare counts to normal profiling. Gate reused statements against unexpected recompilation and stale input versions.
3. Add a typed candidate operator with explicit scope, context, target, and member demands.
4. Lower exact qualification and origin demands from its narrow output. Keep unknown syntax conservative and receiver demands independent.
5. Drive semantic virtual tables from nonempty aggregate frames. Retain ordinary SQL joins for rule composition.
6. Use range density to select bulk index traversal where broad demanded lookups remain. Do not add it to work removed by candidate selection.

Production needs a dependency check before it shares target-derived producers.
The first experimental rewrite matched target relations by suffix and introduced a candidate dependency cycle.
Typed producer metadata fixed that experiment. General lowering must reject cyclic demand dependencies before it emits SQL.

These are test-only spikes. Release behavior and its single execution path remain unchanged.
The eventual implementation must replace the existing producer shape, without preserving experiment flags as user-facing modes.
