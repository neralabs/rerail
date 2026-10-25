# SQL performance recovery workplan

## Active plan — 2026-10-25

Status: A1 and further A2/A3 implementation are committed; aggregate performance acceptance remains open.

The complete 50-rule measurement has matching findings and two remaining engine failures.
It takes 899.070 cold CPU seconds. This predates bounded template ownership and reusable plan fragments.
The corresponding 60-rule run is in progress. The [journal](data-layer-performance-journal.md) records artifacts and limits.
The first matched controls regress CPU by 16–18%. The journal records that result.
Parent membership, scoped kind refinement, fused alternatives, and guarded projection streaming now pass their regression tests.
The guarded-stream 128-file controls reduce CPU by 19.4% for 50 rules and 7.1% for 60 rules versus the failed before version.
The 60-rule memory control still regresses. These ingestion controls do not establish full-repository acceptance.
Planning attribution now separates lowering, composition, rendering, and scheduling.
Finite exact kind alternatives now fold to one scoped predicate; their regression and full DSL tests pass.
The 128-file, 60-rule three-run median improves from 25.675 to 21.827 ingestion CPU seconds versus guarded streaming.
RSS remains effectively unchanged. The journal records all six controls and the separate compile measurements.
All 2,350 executable original observations and 120 supplemental controls pass through both evaluation paths. Full-repository acceptance remains open.
The older R0–R7 plan is historical. This plan supersedes its statement-count acceptance gates.
Statement counts remain diagnostic. Each delivery includes failing tests, implementation, passing tests, and measured evidence.
No production switches select competing implementations. Linear issues require prior user approval.

### Starting evidence

The completed 50-rule run covers 9,021 files and 137,924,324 source bytes.
Its workload and policy digests match the passing pre-SQL reference.

| Metric | Current | Passing pre-SQL reference |
| --- | ---: | ---: |
| Cold CPU | 958.590 s | 125.804 s |
| Ingestion CPU | 875.286 s | 73.350 s |
| Cold global CPU | 82.953 s | 52.144 s |
| Cold wall | 1,978.154 s | 196.707 s |
| Full-process OS peak RSS | 5.832 GiB | Not captured by the same method |
| Findings | 24,820 | 24,820 |
| Execution failures | 3 | 0 |

Compilation used 1.585 CPU seconds and 3.688 wall seconds, separately from cold execution.
The current host is heavily contended. Wall comparisons require matched conditions.

Exclusive counters expose 537.411 CPU seconds in anchor selection and 136.765 seconds in index preparation.
These are measured costs to investigate, not promised savings from a delivery.
The default worker phase contains another 191.489 seconds. Its old label incorrectly implies candidate lookup alone.

The user cancelled the 60-rule run after 3,149 files and at least 1,119.65 sampled CPU seconds.
The benchmark controller and completion recorder stopped. Partial results do not establish full-repository timing or parity.
Evidence remains in `.rerail/profiles/full-scale-direction-20261025/`.
The [measurement journal](data-layer-performance-journal.md#full-repository-measurement-reset-2026-10-25) records hashes and build settings.

### Target execution shape

1. Publish immutable tape versions, query-independent indexes, and SQL statistics.
2. Gather eligible demand across rules and files without executing candidate arms.
3. Read the requested statistics in one batch and choose physical anchors.
4. Compile only the chosen anchor paths and bounded refinements.
5. Execute narrow family relations against their exact demand scopes.
6. Evaluate candidates with the existing matcher and consolidate findings.

Logical rule compilation can precede publication. Physical anchor planning requires the published statistics.
Cold execution includes publication, statistics maintenance, physical planning, and evaluation.
Record logical compilation and physical planning separately to prevent cost reclassification.

The existing compiler and family dependency order are sufficient for this work.
This plan adds no generic graph scheduler or alternate interpreter.
Native producers use the same relation contracts as SQL bindings.
Typed demand batches and typed output rows preserve the CPU, JIT, and GPU integration boundary.

### Delivery order

| Delivery | Complete scope | Evidence required before completion |
| --- | --- | --- |
| A1 | SQL statistics, selected-anchor lowering, decision-aware template reuse, both evaluation paths | Adaptive-anchor tests, unchanged findings, bounded controls, full-corpus measurement before acceptance |
| A2 | Exact demand pushdown and fused candidate-family statements | No eager work for disabled demand, no scope inflation, measured CPU and memory improvement |
| A3 | Batched native relation bindings and SQL-backed estimates | Whole-demand xFilter tests, SQL/native differential results, measured callback and runtime reduction |
| A4 | Full recovery acceptance and remaining preparation/memory costs | Clean full 50/60 runs, separate compile timings, per-edit and integration measurements |

The three execution regressions form a correctness track across A1–A3.
Capture their failing tests at the start of A1. Close them before aggregate acceptance.
A delivery does not complete through a statement-count reduction alone.

### A1: choose anchors from SQL statistics

**Problem:** `Lower::pattern` creates every posting, prices each with `count(*)`, and emits every anchor path.
`Corpus::template` caches SQL by language and plan IDs, without a statistics decision identity.
Replacing the pricing query alone leaves both eager posting construction and stale template choices.

**Complete scope:**

- Use `kfreq` and `file_kind_counts` as existing SQL-owned kind statistics.
- Add exact posting cardinalities alongside existing value segments during publication.
- Retain the current meaning of `vlex.n`: segment count is not node count.
- Include uncovered value demand in estimates. Missing coverage never proves that a literal is absent.
- Maintain the new counts on publication, replacement, retirement, and coexistence of pinned source versions.
- Gather unique language, kind, and exact-literal requests across the admitted plan batch.
- Read summaries with one bounded SQL batch. Chunk only at SQLite parameter limits.
- Do not decode posting blobs or scan the full vocabulary to price candidates.
- Select a mandatory anchor with deterministic tie-breaking by pattern ordinal.
- Emit only the selected anchor path. Do not create whole-scope alternative postings for pricing or refinement.
- Bound necessary refinements to selected roots. The existing matcher confirms the complete pattern.
- Record the chosen anchor, statistics dependencies, estimates, and coverage fallback in plan diagnostics.
- Cache physical templates by compiler/rule identity and decision fingerprint, with grammar and scope dependencies.
- Refresh dependency values through batched reads. Recompile only when the decision changes.
- Retain valid completed roots for immutable versions and identical scopes. A new plan does not require index republication.
- Route workspace and stateless selection through this path. Do not leave an optimized helper unused by production.

**TDD ladder:**

1. Swap common and rare kind frequencies. Require the selected arm to flip and confirmed findings to remain equal.
2. Make an exact literal rarer than its kind. Require the value anchor without enumerating the common kind for pricing.
3. Cover absent kinds, equal counts, missing statistics, incomplete value coverage, and an initially empty store.
4. Publish an edit that flips the decision. Require a new template and correct old/new consumer isolation.
5. Repeat unchanged demand. Require template reuse without index rebuilds or posting recounts.
6. Instrument an unchosen arm that fails on entry. Require zero entries, not merely zero output rows.
7. Exercise file subsets, languages, and both evaluation paths against the existing matcher and recordings.

**Primary sites:** `matcher/anchors/sql/{corpus.rs,lower.rs,access.rs,schema.sql,store.rs,values.rs}`.
Tests belong in the existing SQL lowerer, corpus ownership, candidate-admission, and candidate-publication suites.
New statistics and decision types stay separate from SQL rendering and storage ownership.

**Boundary:** kind and exact-literal anchor choice, bounded refinement, and its cache lifecycle.
New graph algorithms, regex cost models, word indexes, and new rule ports are outside A1.
SQL aggregates required by rule semantics remain valid. The prohibition concerns execution-time anchor pricing.

### A2: restrict demand before family execution

The A1 plan supplies the demand frontier. A2 carries that frontier into fact preparation and SQL execution.

The first measured family is direct-parent membership.
Four statements each execute roughly 23 million scan steps on the 32-file control.
Their materialized child projections lack a usable parent index, so selected parents repeatedly scan every child candidate.
Red commit `9621e074` reproduces the quadratic growth without wall-time assertions.
Doubling candidate demand increases measured SQL work from 1.29 million to 4.94 million VM instructions.

Use selected parent keys as the outer relation and seek `decoded_child_lookup(fid,parent,field,pre)`.
The final intersection must retain the original consumer scope and source version.
Keep field and first-child predicates. This change adds no new materialized relation or native callback.
Compare the same 32- and 128-file controls before selecting the next family.
Commit `b07701d1` implements the indexed-parent correction.
Commit `81ea9328` adds scoped kind refinement and fused alternatives.
Guarded projection streaming retains predicate evaluation barriers; broad single-reader inlining is rejected by a fan-out test.
The guarded-stream diagnostic attributes 6.244 CPU seconds to materialization and 2.020 seconds to physical planning.
Its largest real-file statement repeatedly partitions and reunites one scope by kind.
Red commit `f2b52294` reproduces that shape. Exact finite alternatives now fold before SQL emission.
The matched one-file capture reduces SQL CPU from 6.369 to 4.977 seconds.
The next remaining family costs are candidate result reads, other materialized set operations, and span/predicate preparation.
Native whole-batch bindings and exact structural demand pushdown remain open; do not declare A2 or A3 complete.
These corrections do not complete the broader A2 scope.

- Preserve exact `(consumer, source version, file, range)` membership in demand relations.
- Batch distinct scopes with explicit scope IDs. Do not replace them with their union.
- Put conditional gates before producer entry, on the outer join side or in a recursive seed.
- Compile fused, narrow statements per candidate family instead of CTAS, indexes, and ANALYZE per intermediate.
- Default single-consumer relations to inline execution. Repeated syntax alone does not justify materialization.
- Materialize only after a measured second consumer, with identical definition, version, and candidate scope.
- Require compatible typed columns. Do not widen a family to every column from every consumer.
- Include producer construction and every consumer read in the reuse comparison.
- Keep tape values behind handles. Candidate relations carry keys and demanded scalar columns.
- Decode only properties required by chosen demands. Preserve any complete sidecar only after a measured workload justifies it.
- Apply base-table statistics maintenance at a publication boundary. Do not analyze every temporary stage.

Tests require no producer entry for false gates and complete empty demand.
They also require no decoded rows outside demanded files or ranges.
Different scopes must remain separate. Identical scopes must produce identical answers with one reusable producer.
A correlated second read must not accidentally rerun an expensive producer.
Memory evidence records peak RSS, retained bytes, and decoded rows/bytes per source node.

Primary sites are `workspace/evaluate.rs::prepare_candidates`, SQL plan gathering, lowerer access, decoding, and rendering.
The legacy authored-SQL stage executor is not a new optimization platform for this delivery.

### A3: expose native work as whole-demand relations

- Route native terminal, syntax/owner, and graph producers through their declared narrow relations.
- Supply the whole operator batch through `sqlite3_vtab_in` or a typed demand-frame relation.
- Preserve exact tuples in the demand frame. Separate IN lists must not create a Cartesian product of scopes.
- Require one `xFilter` per operator batch and `xNext` calls for output rows only.
- Remove per-node scalar callbacks from the selected family path.
- Replace the fixed 32-row range estimate with SQL-backed statistics for the actual relation and constraint shape.
- Push producer, file, and range constraints into `xBestIndex` before native execution.
- Claim ordering only for prefixes that the producer actually guarantees.
- Preserve fault isolation per file. One exhausted file cannot invalidate an unrelated file in the batch.

Start with the family that remains expensive after A1 and A2 measurements.
Do not rebuild graph algorithms that already satisfy their contract.
Differential tests compare SQL and native bindings through the same caller query.
Counters must distinguish demand batches, xFilter calls, input rows, output rows, and scalar callbacks.
The deployment path has one implementation choice, without an experimental production switch.

### Correctness track

| Rule / clause | Observed failure | Required next evidence |
| --- | --- | --- |
| `sync-async-signatures` / `transaction-connection` | Work budget on `generated_facades.py` | A recorded reproducer, candidate broadening trace, and the selected-root work responsible |
| `grammar-callable-constructors` / `constructor-arg` | Name closure over incomplete evidence | A dependency trace that establishes whether this is independent or propagated |
| `public-api-triples` / `exported-families` | Relational query interrupted | The precise producer/query, consumed allowance, plan, and a bounded reproducer |

Start with the frozen failing addon. Reduce the input only while the failure persists and the repository context remains valid.
Do not filter repository-wide expected findings into a subset and call that an oracle.
Use upstream observations or a closed fixture with explicit expected findings.
Keep existing allowances during diagnosis. Do not accept results by increasing budgets or suppressing failures.
Any later findings-contract migration must preserve explicit incomplete/Unknown outcomes.

### A4: measurement and acceptance

Apply three levels after each delivery. A bounded control is a development screen, not a substitute for full acceptance.
Do not defer all full-repository measurements until A1–A3 are complete.

1. Run the relevant TDD ladder and the original positive/clean recordings.
2. Run closed selections at increasing sizes, with fixed queries and comparable rule mixes.
3. After the controls show improvement, run the full 9,021-file 50- and 60-rule sets before accepting the performance change.

If a bounded control remains predictably slow, continue diagnosis before another expensive full run.
Mark the implementation as unaccepted while full measurements or correctness fixes remain outstanding.
A1 receives its own measurement review before A2 starts. Re-rank A2 and A3 from that evidence.

Bound development commands and diagnostic captures at 115 seconds.
An incomplete control remains incomplete. Do not reduce its scope silently or infer a full-corpus result.
Use an external bottom-up profiler on separate diagnostic controls, outside the timing runs.
Separate residual worker CPU from the old misleading candidate-lookup label before assigning its cause.

Record cold CPU, cold wall, full-process OS peak RSS, compilation, physical planning, findings, and execution outcomes.
Keep the same corpus, policies, workers, allocator, and build configuration for each comparison.
Preserve captures, hashes, failing observations, and a keep/reject decision in the journal.

Recovery requires the first 50 rules to reach the passing 125.804-second CPU reference with complete findings and no execution failures.
Compare wall and RSS under matched conditions. Do not compare OS peak RSS directly with a historical driver-only peak.
The first clean 60-rule run establishes its reference. The failed historical runs cannot establish a clean speed ratio.
The unified integration also requires the 335-pattern CPython workload and per-edit latency checks.
Record missing integration coverage explicitly. Full-repository CPU evidence is mandatory before a recovery claim.

### Prior work and integration boundary

This local plan agrees with the central direction and the prior work below.
The source excerpts were read at ast-index-spike commit `9449260`.

- Design index topic 5, `DESIGN.md` S050 L3081–3112: rare mandatory anchors bound the tail.
- Design index topics 5/12, `DESIGN.md` S241 L6385–6437: batch planning joins stored frequencies and preserves ordinal tie-breaking.
- Design index topic 11, `DESIGN.md` S155 L6927–6951: temporary tables plus ANALYZE materialize unnecessary work.
- Design index topic 13, `README.md` L1116–1223: scope-identical sharing corrects the earlier S149 conclusion.
- History index D35, `DESIGN_HISTORY.md` L1320–1331: `fts5vocab` does not push down arbitrary per-root doc/offset constraints.
- History index D50, `DESIGN_HISTORY.md` L2681–2697: temporary-stage statistics do not repair the materialization cost.
- Lessons ledger 2, 5, 9, 13, 17, 28: anchors, fusion, exact scope, narrow demand, callbacks, and end-to-end measurement.

[Central direction](../../Pulling-it-all-together/docs/rule-engine-direction/README.md),
[architecture](../../Pulling-it-all-together/docs/rule-engine-direction/architecture.md), and
[prior-work protocol](../../Pulling-it-all-together/docs/rule-engine-direction/prior-work/README.md) define the integration target.

The sibling architecture now proposes set-based findings and freezing authored SQL.
Those changes need their own coordinated integration decision and harness migration.
This recovery plan adds no authored SQL rules and does not relax corpus checks to hide the current failures.
The recorded observations, relation contracts, and source-version isolation remain durable assets.
The integration decision record must cite this evidence before implementation moves into the unified engine.

## Historical plan through 2026-10-24

The material below records previous requirements and experiments. Its count targets do not govern the active plan.

---

Status: R0 is enforced. R1 has an exact attributed inventory. R2 and R3 have tested implementation slices.
General conditional candidate admission, whole-job publication, and R4–R7 remain open.
Bounded C adds explicit admission for complete empty input domains.
The latest fixed guard has 975 engine submissions and 106 SQLite internal executions. The 100-submission gate still fails by 875.
The historical 7,733 checkpoint remains below; full-repository CPU recovery is unresolved.
Compiler input/syntax validation uses prepare-only schemas. Compiler stages share bounded SQLite-inferred metadata across documents.
Runtime producer execution and whole-job lifetimes remain open. Aggregate CPU recovery is unresolved.
All work uses the current worktree. Named span/child demand and deterministic frontier composition now have tested implementation slices.
The [aggregate journal](data-layer-performance-journal.md#aggregate-optimization-checkpoint-2026-10-23) preserves the failing starting point.

The [ranked execution opportunities](sql-execution-opportunities.md) now define the delivery size.
R5-A removes 4,048 submissions against its full 3,474 target. R5-B removes 1,435 against its full 1,296 target.
Each implementation delivery must remove its full expected count, with CPU, memory, and parity gates intact.
The [R5-B journal](data-layer-performance-journal.md#relational-storage-delivery-r5-b-2026-10-24) records the complete accounting and matched controls.
Small TDD commits support these chunks. They do not complete a phase by themselves.
The [bounded C specification](sql-execution-chunk-c.md) selects four state syntax frontends and explicit empty-domain admission.
C delivers 1,275 actual removals against its unchanged 1,261 target and 989 ceiling.
The [C journal](data-layer-performance-journal.md#state-syntax-domain-delivery-r5-c-2026-10-24) records the implementation and checks.
The final 100 limit remains unchanged. General shared execution remains later work.

## Required outcome

One complete cold job submits at most **100 SQL statements from the engine**, across all workers and connections.
The user clarified this boundary after the FTS bulk-publication audit.
SQLite internal executions remain separately counted and reconciled. They do not consume the engine allowance.
This limit comes from `jeremy/sql-capture-demand-spikes`, commit `c109a915`.
The profiler import omitted that gate and its execution-coverage checks.
The producer-shape tests counted selected native passes. They did not enforce the SQL execution limit.

Each engine submission counts, including cached statements, publication, schema setup, indexes, host helper statements, transactions, matching, and cleanup.
An `execute_batch` call counts every SQL statement inside it.
A file, rule, input batch, result packet, connection, or worker never resets the allowance.
Diagnostic EXPLAIN and enclosing native timing spans do not consume the allowance.
The gate runs after complete execution. It never truncates findings at statement 101.

Counts alone cannot establish recovery.
Acceptance also requires complete findings, no execution failures, bounded memory, and CPU without a regression against matched references.
One large statement that repeats the same expensive work does not satisfy the CPU requirement.

## Measured starting point

| Metric | Earlier reference | Current checkpoint |
| --- | ---: | ---: |
| First-50 cold CPU | 125.804 s | 2,507.496 s |
| First-50 ingestion CPU | 73.350 s | 2,456.015 s |
| First-50 global apply CPU | 52.144 s | 51.145 s |
| First-50 reported peak RSS | 6.441 GiB | 6.143 GiB |
| All-60 cold CPU | 709.151 s, failed execution | 7,211.657 s, failed execution |
| All-60 candidate materialization/fill actions | Not comparable | 1,359,072 |

The current first-50 run passes repository parity.
The current all-60 run matches 24,993 findings but reports 34 state-tripwire relational failures.
The earlier all-60 reference also fails. Its timing cannot establish a clean performance ratio.
Candidate counters omit some publication, index, transaction, and cleanup statements. They are lower bounds on complete execution counts.

The implementation introduces three concrete sources of excess work:

- `workspace/evaluate.rs::prepare_candidates` collects every rule's plans before the later file filters.
- `anchors/sql/plans.rs` collects both branches of conditional queries before their runtime gates.
- SQL lowering repeats selectors and intermediate materializations across statements and input batches.

Index publication also executes SQL in loops over files, kinds, vocabulary terms, and overflow records.
Those calls participate in the same limit.
Persistent indexes and decoded rows remain useful, but their presence does not establish shared selector execution.

The restored gate records one positive and one clean observation through snapshot and workspace paths.
Their complete replay passes parity but executes 24,159 SQL statements on 136 connections.
The trace contains 11,544 CREATEs and 5,508 DROPs.
Execution 101 creates `syntax_21` during compilation schema validation.
This small case proves that session setup and relation lifetimes also need structural changes.

## R0: restore the acceptance boundary

Deliverables:

- Restore the fixed `JOB_SQL_EXECUTION_LIMIT = 100` acceptance gate.
- Record started, completed, and running statements from connection creation.
- Record connection-open and connection-close receipts outside individual trace files.
- Reject missing entire sessions, partial streams, mixed jobs, incomplete operations, and failed parity.
- Preserve the first statement that exceeds the allowance, plus setup, transaction, and matching totals.
- Make ordinary SQL capture and report commands return nonzero when acceptance fails.
- Add a mandatory CI workload with original positive and clean state-tripwire observations.

The initial CI workload is a small regression guard. It does not establish whole-repository acceptance.
Its findings must complete before the gate reports the excess engine submissions.
The current engine remains an expected acceptance failure until recovery removes the excess work.
No skipped test, increased allowance, or optional production path converts that failure into a pass.

TDD ladder:

1. Accept 100 executions and reject 101.
2. Reject 101 executions distributed across workers and connections.
3. Count repeated cached executions and each statement inside `execute_batch`.
4. Reject low counts with missing traces, missing session receipts, or failed findings.
5. Preserve evidence and return nonzero from the ordinary report command.
6. Run the actual CI workload and record its count, parity, coverage, and first excess statement.

## R1: reconcile the complete execution inventory

This step precedes another optimization or full repository run.
Use one positive and one clean case, then the existing state and ten-rule selections.
Separate instrumentation runs from CPU controls.

Classify every execution as connection setup, catalog publication, file publication, candidate preparation, relational evaluation, or cleanup.
Attribute each execution to its producer, binding, source generation, file domain, and consumer when those identities exist.
Record the counts of input batches, rules, plans, and selected roots beside these totals.
Count actual executions rather than SQL strings, API calls, or static placements.

TDD ladder:

1. Reconcile trace totals against native started/completed counters and session receipts.
2. Preserve the total when one prepared statement runs repeatedly.
3. Preserve the total when identical work moves between connections or worker threads.
4. Add files and rules separately to expose each scaling dimension.
5. Confirm that an unrelated file does not increase the requested file's VM work.

Completion evidence contains an exact statement inventory and explicit unknown attribution.
It also identifies every source loop that cannot meet the complete-job allowance.
The inventory must reconcile before an improvement claim.

## R2: restore eligibility before candidate preparation

Retain the existing path, language, source-size, syntax, text, and rule-capability decisions.
Compute eligible `(plan, file, generation)` pairs before expensive SQL preparation.
Use the same admission logic for preparation and final evaluation.
Gather ready pairs in typed batch inputs. Do not issue one query per pair.
The primary sites are `workspace/evaluate.rs`, `anchors/sql/plans.rs`, and `anchors/sql/corpus.rs`.

Conditional consumers contribute demand only after their gate enables them.
The compiler can retain the complete plan inventory without executing every plan for every file.
Source publication must retain files required by cross-file analysis, even when local rules do not consume them.

TDD ladder:

1. Preserve findings for one admitted rule and file.
2. Add an excluded rule and prove that candidate execution does not increase.
3. Add excluded files and prove that the admitted rule's preparation does not increase.
4. Disable a conditional consumer and prove that it contributes no candidate demand.
5. Enable that consumer and preserve positive, clean, and cross-file results.
6. Preserve failure propagation, cancellation, edits, and source-generation isolation.

Compare the same compiled policy set and input corpus before and after the change.
Report removed plan/file execution and measured CPU separately.
Zero selected roots alone do not prove that preceding work was unnecessary.

## R3: batch publication across the cold job

The complete-job limit permits no SQL statement for each of 288 ingestion requests.
This constraint requires a real job boundary before SQL producer execution.
Use the [execution stages](sql-execution-stages.md) to separate source admission, publication, preparation, and consumption.
Preserve observable results and failure behavior for existing workspace requests.
Any required API boundary change needs an explicit compatibility test.

Publish language catalogs, file records, kind postings, value postings, and overflow data through typed batch relations.
Use SQL `INSERT ... SELECT` over those relations where it preserves the existing index layout.
Keep the ast-index-spike FTS layout and disk-backed SQL ownership.
Avoid JSON row payloads and unbounded native copies of decoded SQL facts.

Admit relational sessions only when their consumers can run.
Remove repeated schema construction on independent clause connections.
Include compiler validation connections in this change. Prepare-only validation must retain its authorizer and schema checks.
The primary sites are `compile/relational.rs`, `relational/database.rs`, and `relational/syntax.rs`.
Prove whether schema-only relation adapters can retain SQLite validation without repeated executed DDL.
Reject that approach if names, affinity, authorizer decisions, or validation failures change.
Use shared backing with consumer-qualified rows and bounded private scratch where semantics require it.
The 136-connection guard must demonstrate a smaller actual setup count, without omitted validation or weakened isolation.

TDD ladder:

1. Publish one file with the existing index contents and lookup results.
2. Publish many files with the same fixed statement families.
3. Preserve exact values, duplicate postings, missing values, overflow records, edits, and removals.
4. Preserve transaction rollback after interrupted publication.
5. Increase input batches without increasing the complete-job execution allowance.
6. Measure temporary bytes, retained bytes, peak memory, and publication CPU together.

Completion requires actual engine-submission counts under the intended workload.
Reconcile and report SQLite internal execution separately. Every engine helper statement still consumes the fixed allowance.
The FTS audit proves one bulk insert of 101 documents executes 109 SQLite statements internally and externally.
The user resolves that conflict by applying the limit to engine submissions; the required FTS layout stays intact.
The tracer uses SQLite callback origin evidence. It does not classify SQL by table names or omit internal measurements.

### R3 compiler checkpoint

Compiler input and syntax declarations now use prepare-only SQLite modules.
Twelve independent clauses without stages execute zero SQL during compilation.
SQLite still checks column declarations, names, query fields, authorizer decisions, and stage CTAS affinity.
Compiler initialization omits runtime PRAGMAs. Runtime connections retain those settings.
Empty runtime syntax sources use executable SQL tables over the same column declarations.

The selected small capture removes 406 connection-setup submissions, from 904 to 498.
Its complete job still submits 9,695 statements. Its 194 compiler stage CTAS executions already exceed the job allowance.
The 136-connection lifetime problem remains open. This slice does not establish whole-job schema reuse or complete R3.
Authored-order and work-estimate replacements regress CPU controls and are removed.
Allocation-dependent lowering remains a measured R4 risk. Named producer discovery must remove incidental address ordering.
The [journal](data-layer-performance-journal.md#selected-compiler-recovery-checkpoint) records the accepted changes and rejected controls.
The [checked evidence](../fixtures/host/data-layer-port/profiling/sql-recovery-compiler-20261023.json) preserves all matched CPU trials and correctness results.

The next TDD slice shares SQLite-inferred stage schemas across document compilations.
The cache key contains exact SQL, complete ordered schemas of read relations, and deferred column names.
Every clause still receives SQL, authorizer, and deferred-expression validation. Cache hits never bypass those checks.
The process owns immutable metadata only, with fixed entry and charged-payload bounds. No runtime rows or database handles are retained.
Deferred-expression validation uses a prepare-only backing schema instead of executed CREATE/DROP statements.

The final small capture records 9,095 engine submissions, including 68 compiler stage CREATEs and 372 connection-setup submissions.
All 136 connections remain. Runtime candidate preparation, cleanup, and relational evaluation still submit 8,675 statements.
The 100-submission gate fails with complete findings and coverage. This remains a partial R3 checkpoint.
Matched median CPU increases from 2,479.402 ms to 2,595.351 ms. The count reduction is not a speedup claim.
The [stage-schema evidence](../fixtures/host/data-layer-port/profiling/sql-stage-schema-recovery-20261023.json) records both control groups and the unresolved CPU requirement.

## R4: establish shared producers and remove repeated construction

The 2026-10-24 checkpoint implements trusted structural contracts and live source-pinned candidate exports.
Registered binding IDs drive same-selector decoder reuse. Definitions and complete typed bindings establish sharing without digest equality.
Exports preserve rule, clause, policy digest, result index, and selector provenance.
The early writer/reader check selects physical public owners. The legacy generated-view exception cannot provide general reader isolation.
Its count shortcut and CTE accessor problems remain explicit follow-up work.

The state guard has one span binding with 117 consumer/selector references, but still submits 22 separate span fills per normal frontier.
The guard remains failing at 9,113 engine submissions. No physical coalescing follows from the catalog alone.
The next slice must turn ready demand under that binding into one broad fill per necessary dependency wave.
The [foundation evidence](../fixtures/host/data-layer-port/profiling/shared-producer-foundation-20261024.json) records correctness, complete counts, and matched CPU controls.

The next slice is implemented for spans. Independent live selectors share one SQL fill per necessary dependency wave.
The two-rule, two-packet test preserves all 130 findings with one physical span-fill execution.
The state guard has two fills per normal frontier. Existing selector tables and the retained span owner remain the storage boundary.
The planner shares generated-reference analysis with its existing pass. It adds no algorithm flag or general scheduler.

The complete count nevertheless increases to 9,481 because upstream lowered frontiers differ under allocation-address ordering.
The selected CPU control has overlapping ranges and establishes no aggregate recovery.
The [wave evidence](../fixtures/host/data-layer-port/profiling/shared-span-waves-20261024.json) records the rejected extra scan, the selected controls, and a frozen-frontier comparison.

Before another property extraction, make frontier construction reproducible:

1. Capture identical compiled plans in explicit permutations, preserving logical consumer identities and input generations.
2. Record full lowered producer definitions, broadening decisions, physical builds, and complete engine submissions for each permutation.
3. Prove the exact source of the observed work difference. Separate bounded lowering from utility reuse and later physical placement.
4. Define producer identities and per-plan admission before composing the shared physical graph. Heap addresses must remain lookup details.
5. Preserve the existing lowering bound and the fixed 100-submission limit. Do not replace them with another ordering heuristic.
6. Require equal logical results and stable shared build counts before matched CPU and memory controls.

The earlier cache-pruning control was invalid and remains rejected. It is not evidence for this new root cause.
Conditional admission, whole-job publication, general completion, and cross-clause readers remain open.

### R4 deterministic frontier checkpoint

The fixed-plan audit confirms the cause: forward order admits 54 broadened plans, while reverse order broadens 64 of the same 139 plans.
The existing bound charges newly generated work after earlier plans populate reuse state.
Independent bounded fragments now separate admission from shared composition.
Full typed producer definitions establish sharing. Canonical topological numbering establishes physical placement and output-packet grouping.
All four permutations now produce 1,503 stages and 65 broadened plans, with matching producer and stage definitions.
The 256-node allowance remains unchanged. No cost-order heuristic or alternate executor is introduced.

The complete small guard now repeats at 8,669 engine submissions, down from 9,481.
Matched CPU decreases from 2,318.369 ms to 1,617.488 ms on this small workload.
Correctness covers 309 native tests, 44 standalone SQL tests, and 120 original positive/clean observations.
The [canonical-frontier evidence](../fixtures/host/data-layer-port/profiling/canonical-frontier-recovery-20261024.json) preserves all results and qualifications.

Next, coalesce remaining node/child demand and repeated selector construction on this stable frontier.
Preparation and cleanup still account for 6,468 submissions. Relational evaluation adds another 1,781.
Whole-job publication, general completion, cross-clause readers, and repository acceptance remain open.
The fixed 100-submission gate continues to fail and must remain mandatory.

This step implements WP1 and the first useful execution slice from WP2.
Use the [named contracts](shared-producer-contracts.md) and the existing discovery and clash tools.
Names identify declared work. Complete bindings include inputs, options, generations, schema, and semantic domains.
Private stages retain rule-qualified identities.

Start with repeated source-span demand and selector work demonstrated in the captures.
Union ready eligible keys before construction.
Compute missing coverage once, including empty output, and publish rows with completion atomically.
Consumers read one physical owner across statements, clauses, and result packets.
They do not embed another materialized copy of the owner's definition.

TDD ladder:

1. Execute a two-rule, two-packet diamond over overlapping demand.
2. Prove one construction for shared selectors and admitted keys.
3. Preserve empty results, retries, cancellation, and different source generations.
4. Diagnose a public-name clash with both origins and the differing fields.
5. Exercise the same path with actual ported rules.
6. Repeat the captured producer with 53 host placements and prove one physical owner for compatible demand.

Keep the specialized dependency phases and current scheduler where they suffice.
A new generic graph scheduler is not a prerequisite.
Extend the same mechanism to properties and child fields only after the first slice passes.

## R5: bound SQL execution and materialization together

The selector-family prototype does not satisfy this phase.
It reduces 7,733 engine submissions to 4,109, but regresses CPU even after its heap peak is bounded.
The engine retains only the proven private-owner projection cleanup and relevant-identifier filtering from that investigation.
The measured guard remains at 7,733 submissions. No aggregate recovery follows from that cleanup.

The next implementation must lower ready typed operators directly into SQL.
It must avoid rendering each selector tree first and then processing those trees again to construct a batch.
The first operator should have two real consumers, a shared immutable input, and request-qualified scalar options.
Keep producer identities separate from transport tags. Preserve typed parameters, bag multiplicity, NULLs, and source-predicate fences.
Use existing dependency waves with explicit producer edges. A generic graph scheduler remains unnecessary for this slice.
Measure compiler CPU and SQLite preparation CPU separately before promoting broader batching.

Lower compatible ready consumers into broad SQL operator families over binding-qualified inputs.
Group result emission without losing rule, clause, file, or witness identity.
Do not implement a new native rule interpreter.

Materialize a shared producer exactly once for its admitted execution domain.
Keep cheap single-use transformations inside their consumer.
Remove repeated temporary-table creation, index construction, and deletion around the same work.
Preserve type affinity, NULL behavior, multiplicity, recursive termination, and demanded-key access paths.

TDD ladder:

1. Preserve complete row multisets for each changed operator family.
2. Count setup, publication, preparation, matching, and cleanup in one cold job.
3. Increase files, roots, rules, and output packets independently without another call for each item.
4. Keep genuine dependency waves explicit and bounded.
5. Reject SQL that meets the count limit but increases VM work, CPU, or peak memory.

Completion requires at most 100 engine submissions on the state and ten-rule workloads.
Static operator counts and three native producer passes are supporting diagnostics, not substitutes for this gate.

## R6: remove execution failures and attribute memory

Reduce the 34 full-repository state-tripwire failures to bounded reproductions.
The failures contain 12 regex-budget errors and 22 SQL interruptions.
Identify the work that consumes the shared allowance before changing its limit.
Preserve the existing allowance and complete diagnostic multisets during query-shape corrections.

Add ownership estimates for retained tape, scope, and preparation storage that the current census omits.
Reconcile ownership estimates with external allocator, footprint, and residency observations without adding overlapping totals.
Prove release after workspace closure and after repeated edits.
Do not infer a leak from RSS or allocation counts alone.

Completion requires no engine failures in the positive/clean corpus or the all-60 repository workload.
The unchanged and edit checks must also contain no retained failures.

## R7: accept the aggregate recovery

Run each small acceptance workload in a fresh process and capture directory.
Require complete parity, complete connection coverage, and at most 100 engine-submitted SQL statements.
Run matched CPU controls without tracing, census, SQL capture, or external sampling.
Use identical inputs, build settings, allocator, workers, and ingestion requests within each comparison.

The acceptance matrix covers one file, unrelated retained files, larger input batches, state, ten rules, first 50, and all 60.
The final requirement applies to the complete 9,021-file cold job, not each ingestion batch.
Run repository measurements only after the smaller gates pass.
Use the existing approval for full repository runs and addon builds. Other commands retain the 115-second deadline.

Require all 3,210 native corpus observations, all 40 edit transitions, and the appropriate native suite.
Resolve or explicitly retain the public performance-guard failure and six incomplete pack checks.
No full-suite pass follows from smaller passing groups.

Compare aggregate CPU and memory against fresh matched references.
The historical first-50 target is 125.804 CPU seconds, with the recorded memory-method qualifications.
The failed earlier all-60 run remains diagnostic evidence rather than a clean speed reference.
Record every experiment, rejected approach, source identity, and acceptance result in the journal.

## Completion order

R0 → R1 → R2 → R3 → R4 → R5 → R6 → R7.
Each step ends with a reviewable change, regression evidence, execution counts, CPU, memory, and a journal entry.
WP1 starts within R4 in this same worktree.
This plan creates no Linear tickets and adds no optional production execution path.


## Recovery evidence from this iteration

The child-demand slice reduces the reproducible 8,669-submission guard to 7,733 submissions in two complete captures.
Field-qualified requests share one SQL fill per dependency wave, while distinct field bindings remain separate.
Completed absence, source changes, rollback, and retry retain tested semantics under the existing SQL owner.
The first frontier reduces child fills from 246 to 11 and retains 393 temporary selector tables.
Across nine matched pairs, median CPU changes from 1.740 to 1.708 seconds, with overlapping ranges.
The SQLite stepping heap peak remains unchanged. CPU and process memory recovery remain open.
Candidate preparation and cleanup still require 5,532 submissions and remain the next measured target.
The [journal](data-layer-performance-journal.md#shared-child-demand-2026-10-24) and checked evidence retain all nine pairs and scope limits.

The [journal](data-layer-performance-journal.md#recovery-tdd-2026-10-23) records each red test, implementation, green run, and remaining requirement.
Local candidate preparation now respects admitted plan/file domains and keeps compatible files batched.
Snapshot and later relational preparation preserve that admission. Excluded text does not become a fallback root walk.
The fixed guard workload decreases from 24,159 to 22,181 total SQLite executions after the local slice alone.
Complete findings and trace coverage pass. These counts precede the clarified origin boundary and the later runtime slices.

Typed publication reduces six insert families from 1,199 executions to 18 for four files and preserves all 800 findings.
The production helper tests cover variable limits, typed bindings, empty batches, failures, and rollback.
The change retains the ast-index-spike FTS layout and SQL ownership.
Whole-job publication still requires a source-admission boundary across the 288 ingestion requests.
It cannot be implemented by repeating these batches independently for each request.

R2 remains open for conditional cross-file activation through dependency phases.
R3 remains open for whole-job publication and session lifetimes. Compiler stage metadata now shares across documents within fixed retention bounds.
R4 must then establish named physical owners and shared missing coverage, including empty results.
R5 must coalesce compatible consumers while preserving the 100-submission guard.
R6 and R7 retain their failure, memory, and matched aggregate CPU requirements.
No aggregate recovery claim follows from these small cases.
