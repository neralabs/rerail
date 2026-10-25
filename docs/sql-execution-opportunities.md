# SQL execution reduction opportunities

Each implementation delivery must remove its chunk's **full expected number of engine submissions** on the complete fixed guard workload.
Small TDD commits can support that delivery. They do not complete a performance phase on their own.
This plan supersedes the earlier practice of stopping after a small optimization slice.

The baseline is `188fb8f4`: **7,733 engine submissions**, **994 SQLite internal executions**, and zero unknown origins.
The workload includes positive and clean state-tripwire observations through snapshot and workspace execution.
It is the complete small guard, not the full data-layer repository. These counts must not be extrapolated linearly to the repository.
The [count ledger](../fixtures/host/data-layer-port/profiling/sql-execution-opportunities-20261024.json) records source hashes, witnesses, formulas, dependencies, and overlap.

## Post-C survey: current 975 submissions

Production remains at `82d82d89`, with 975 engine submissions and 106 separately counted SQLite internal executions.
The [survey evidence](../fixtures/host/data-layer-port/profiling/post-c-sql-survey-20261024.json) reconciles every engine submission to one bucket.
These counts describe the complete fixed guard, including snapshot and workspace execution for two observations.

| Disjoint work | Submissions | Recommended treatment |
| --- | ---: | --- |
| Candidate preparation and retirement | 312 | Reduce physical cuts and repeated demand preparation through bounded SQL pipelines and shared producer owners. |
| Simple event clauses | 202 | Apply the existing complete-input precondition. An isolated policy copy measures 202 → 12. |
| State-flow SQL | 184 | Plan private pipelines around necessary indexed, shared, and native-producer boundaries. |
| Index initialization and reader setup | 176 | Reuse physical SQL backing through an execution owner, with separate logical domains. |
| Compiler affinity inference | 53 | Retain SQLite's type semantics. Reduce inference boundaries only with a proven replacement. |
| File and language-catalog publication | 48 | Maintain typed bulk publication within the shared owner. Include maintenance costs. |
| **Total** | **975** | **The final limit remains 100.** |

### Proven admission opportunity

The four C frontends use `requireNonemptyInput`, but the other 28 state clauses still query a single `events` input without that precondition.
The trace contains 55 complete empty event evaluations and two nonempty evaluations.
An isolated policy copy adds `requireNonemptyInput: events` to those 28 clauses, using the existing addon.
The unchanged complete guard decreases from 975 to 785 submissions: **190 actual removals**, including all replacement and cleanup work.
All 77 state observations retain parity: 64 positive and 13 clean, with zero engine failures or execution differences.
Five fresh matched pairs measure median CPU of 575.665 → 587.471 ms, with overlapping ranges.
Median peak RSS is 300,928 → 297,328 KiB. All controls retain parity.
This experiment demonstrates a count reduction and no CPU improvement. It changes no production policy.
Admission can supply a count component of the next delivery, but that complete delivery must pass CPU and memory acceptance.

### Candidate boundaries and completion

The 312 candidate statements include 110 fills of `candidate_keys`, 66 retirement DELETEs, 20 span fills, and 28 child fills.
Seventy-four candidate fills, 26 retirements, 16 span fills, and 22 child fills change no rows in this recording.
Zero changed rows do not prove that a statement can safely be skipped.
The planner already batches independent roots and decoder requests. Additional savings require fewer physical cuts or fewer genuine dependency barriers.

The next candidate experiment must compile the captured typed graph into bounded SQL pipelines.
Cheap private work stays inside one SQL scope. Selected shared materializations retain one named owner and complete binding identity.
Decoder boundaries retain their real dependencies. Integer key affinity and bounded memory remain required.
Changing the preparation limit or concatenating rendered query trees does not establish a valid improvement.
No net removal target is assigned until the complete replacement schedule passes replay and CPU/memory controls.

### Physical owner lifetime

Index initialization costs 168 statements: four repetitions of 42.
Reader configuration adds eight statements. The four index lifetimes contain two distinct source hashes.
The guard deliberately runs both snapshot and workspace paths. Both paths must continue to execute.
A shared physical SQL owner can retain schema while logical domains retain their own versions, membership, completion, budgets, and failure attribution.
The positional FTS index remains the starting point.
One physical schema identifies a **gross ceiling of 126 avoided setup statements**.
Domain publication, retirement, cleanup, and maintenance must be subtracted before this becomes a net target.
A full-repository workspace already amortizes initialization, so this guard opportunity does not predict repository CPU savings.

### Relational pipelines and compiler semantics

The remaining state-flow work comprises two executions of 92 statements each.
Each execution materializes 24 SQL-derived relations and crosses one native reachability boundary.
Together they create 22 indexes and execute 14 ANALYZE statements.
Private single-use stages can form SQL pipelines, while indexed or multiply consumed relations retain explicit owners.
Recursive stages require separate cost and memory evidence before fusion.
Eponymous native relations identify another 16 CREATE/DROP statements for investigation. Runtime rebinding and isolation still need an experiment.
These are gross opportunities, not measured net removals.

The compiler's 53 statements infer SQLite CTAS affinity on cold schema-cache misses.
A larger warm cache does not remove first-use inference. Plain prepared SELECT metadata does not preserve all CTAS expression affinities.
Any replacement must retain SQLite validation, column names, CAST/NULL/BLOB behavior, and authorization even when runtime admission is empty.

### Capture and next delivery

Four candidate statements come from capture provenance: `SELECT fid,lang,hex(digest) FROM file ORDER BY fid`.
The profiler can obtain those versions from existing corpus metadata instead of querying SQL.
All four remain counted until the queries are actually removed. Moving work into triggers does not count as a reduction.

The next delivery must combine a priced set of these changes and remove its full promised count.
The admission experiment supplies 190 measured removals, without a demonstrated CPU gain.
Candidate pipelines and physical owner lifetime require compiled-plan experiments before pricing.
Submission savings alone do not establish CPU recovery. SQL execution spans omit preparation CPU, and matched controls remain mandatory.
The [survey journal](data-layer-performance-journal.md#remaining-sql-submission-survey-2026-10-24) records the experiment and qualifications.

## R5-A implementation result

The complete guard now submits **3,685 statements**, down from **7,733**: **4,048 removed**.
This exceeds the full 3,474 target by 574. The final 100 limit remains enforced and still fails.
The measured candidate lifecycle has four store creates, four indexes, 380 population statements, 324 deletes, and four final drops.
Its total is 716, compared with 4,764 before this chunk.
The population count exceeds the original 250 planning cap. Retirement falls much further than the modeled 1,028 boundaries.
The complete replacement cost, including every setup and cleanup statement, is below the 1,290 budget.

Logical bindings share one nonunique `(producer, fid, pre)` index within each execution domain.
The compiler batches independent roots before rendering their shared WITH scope.
A batch completes every member, including empty members. Each member retains its own last-use release.
The existing dependency scheduler supplies retained read/write barriers. It does not acquire a general graph runtime.
The fixed expansion budget limits preparation growth. Different schemas retain separate storage.
Captures include the store setup, logical slots, batch members, full logical graph, and executable schedule.
The standalone `candidate_owner` command recompiles frozen plans through the production ownership compiler.
The `cache_budget` command replays the captured schedule, including shared-owner setup and retirement.

R5-B below is repriced from 3,685 submissions. It cannot claim the retirement savings already achieved here.
The [bounded C specification](sql-execution-chunk-c.md) now selects four state syntax frontends and empty-domain admission.

## Measured inventory

The inventory in this section preserves the original 7,733-submission baseline.
After bounded C, the complete guard measures 975 submissions, with 875 remaining above the final limit.
The [C journal](data-layer-performance-journal.md#state-syntax-domain-delivery-r5-c-2026-10-24) records the current phase inventory.

| Execution family | Engine submissions |
| --- | ---: |
| Candidate selector table construction | 1,588 |
| Candidate selector index construction | 1,588 |
| Candidate selector table retirement | 1,588 |
| Relational table, view, and index construction | 1,192 |
| Relational ANALYZE | 310 |
| Relational object retirement | 628 |
| All other setup, publication, preparation, control, and result statements | 839 |
| **Total** | **7,733** |

The four captured candidate frontiers contain 393, 393, 401, and 401 indexed `(fid, pre)` tables.
Their existing schedules contain 255, 255, 259, and 259 adjacent retirement groups: **1,028 groups** in total.
A host `execute_batch` around several DROP statements still counts every DROP. It does not reduce submissions.
The SQL report assigns global query IDs. Inventory rows resolve through the report's query catalog, rather than a session's local IDs.

## Ranked opportunities

The A and B rows preserve their original component budgets. Both measured deliveries appear in this document.
C delivers 1,275 removals against its fixed 1,261-removal target, reaching 975 complete-guard submissions.
The frontend experiment and empty-session audit preserve the original price for that delivery.
Current execution counts are measured. Replacement budgets must pass a compiled-plan audit and complete runtime accounting.

| Rank | Opportunity | Before | Replacement budget | Net removal target | Delivery chunk |
| --- | --- | ---: | ---: | ---: | --- |
| 1 | Maintain candidate key indexes across producer fills | 1,588 | 4 indexes | **1,584** | R5-A |
| 2 | Batch typed candidate population, including initial store creation | 1,588 | 250 fills + 4 store creates | **1,334** | R5-A |
| 3 | Share relational backing, indexes, statistics, and retirement | 2,130 | At most 1,130 | **At least 1,000; provisional** | R5-B |
| 4 | Coalesce candidate retirement after operator batching | 324 after R5-A | At most 200 | **124** | R5-B |
| 5 | Retire candidate rows at existing boundaries instead of dropping each table | 1,588 | 1,028 deletes + 4 final store drops | **556** | R5-A |
| 6 | Bound clause-reader configuration to four readers per observed execution context | 136 PRAGMAs | At most 32 | **104** | R5-B |
| 7 | Replace executed empty syntax-schema CTAS with validated prepare-only metadata | 68 | 0 | **68** | R5-B |

The 68 schema probes end in `LIMIT 0`. Their removal must preserve column names, affinities, and full statement validation.
The 136 PRAGMAs belong to 68 clause connections. The proposed pool allows 16 readers across the four observed execution contexts.
New attachment, isolation, publication, or maintenance statements count against the net delivery target.

## R5-A: original candidate ownership and batching budget

**Target: remove 3,474 submissions, reducing the complete job from 7,733 to at most 4,259.**

This original component budget remains below for comparison. The implementation result above exceeds its full net-removal target.

The current selector lifecycle costs 4,764 submissions: one table creation, one index creation, and one drop for each of 1,588 producers.
The replacement budget is:

| Operation | Maximum submissions |
| --- | ---: |
| Create four scoped SQL stores | 4 |
| Create their persistent composite indexes | 4 |
| Populate ready producer batches | 250 |
| Retire rows at the current release boundaries | 1,028 |
| Retire the four stores | 4 |
| **Replacement total** | **1,290** |
| **Net removal: 4,764 − 1,290** | **3,474** |

Use typed SQL rows qualified by producer binding and execution domain.
Keep incompatible snapshots and generations separate. Four observed contexts do not become one semantic domain.
Use a nonunique index with the producer key before `(fid, pre)`. Preserve integer affinity, bags, NULLs, and empty completion.
Maintain that index from the start. Do not reconstruct it for each selector or file partition.

Compile operators and their dependencies before rendering SQL.
Batch requests for compatible operations, rather than concatenating complete rendered SELECT trees.
The 250-fill cap includes request publication and maintenance. It is a target that needs a plan-level feasibility check.
Use the current specialized dependency waves. A generic graph scheduler is not a prerequisite.
Fold the required named-owner and dependency metadata into this delivery, rather than reporting a separate zero-saving milestone.

Keep the existing last-use boundaries for this first chunk.
One DELETE can retire all producer keys in an existing adjacent release group.
This accounts for 1,028 replacements without assuming that extra rows remain live until the job ends.
Preserve completion, rollback, retry, source replacement, cancellation, and logical rule budgets.

The relevant code is [candidate lowering](../native/src/code_rule_dsl/matcher/anchors/sql/lower.rs),
[execution and retirement](../native/src/code_rule_dsl/matcher/anchors/sql/execute.rs),
and the [existing lifetime scheduler](../native/src/code_rule_dsl/matcher/anchors/sql/lower/lifetime.rs).

## R5-B: shared relational backing and complete lifetimes

**Repriced target: remove 1,296 additional submissions from 3,685, reaching at most 2,389.**

**Delivered: 1,435 removals; the complete guard now submits 2,250 statements.**
Runtime relational work removes 1,157, reader configuration removes 128, and empty candidate retirement removes 150.
Replacement writes and cleanup remain counted. The 68 compiler empty-schema probes remain unchanged.
The target below is the original component budget; measured components meet the full net target through a different balance.
Median CPU is roughly flat at 1,546 → 1,541 ms; median peak RSS is 335,536 → 332,960 KiB.
See the [journal](data-layer-performance-journal.md#relational-storage-delivery-r5-b-2026-10-24) and
[checked evidence](../fixtures/host/data-layer-port/profiling/relational-storage-recovery-20261024.json).

| Component | Current or modeled baseline | Replacement cap | Net target |
| --- | ---: | ---: | ---: |
| Relational object, index, statistics, and retirement lifecycle | 2,130 | 1,130 | 1,000 |
| Candidate row retirement after R5-A | 324 | 200 | 124 |
| Clause connection PRAGMAs | 136 | 32 | 104 |
| Empty syntax-schema CTAS | 68 | 0 | 68 |
| **Net target** | | | **1,296** |

First audit schemas, input bindings, typed options, source generations, and closed domains.
Equal SQL text alone does not prove shared data. Do not count an unverified producer reuse as a saving.
The trace contains 366 input-table creates over 18 declared shapes, 428 index creates, and 310 ANALYZE executions.
Those counts identify an ownership and session problem; they do not prove that all corresponding rows can share one binding.
Keep physical backing reusable where schemas match, while different producer slots retain their own data.

Publish compatible derived relations once for their proven binding.
Scoped readers must retain authorizer isolation, private names, snapshot barriers, per-rule budgets, and failure attribution.
Reuse schema and index lifetimes without recreating empty tables for every clause.
Maintain useful statistics on published owners. Do not remove ANALYZE merely to improve the execution count.

Coalesce retirement at broad ready frontiers only after memory admission proves that delaying release is safe.
The 200-retirement cap must not increase resident memory or retain stale generations.
Pool readers within the current worker bound. Preserve atomic publication without adding an uncounted transaction for every consumer.

The implementation sites are [relational sessions and stages](../native/src/code_rule_dsl/relational.rs),
[capacity admission](../native/src/code_rule_dsl/relational/batch.rs),
[scoped authorization](../native/src/code_rule_dsl/relational/authorizer.rs),
and [schema metadata](../native/src/code_rule_dsl/relational/schema.rs).
The existing [shared-storage contract test](../scripts/profiling/preparation/src/shared_storage.rs) supplies an initial connection constraint.

## R5-C: original planning price for the delivered state frontends

**Target: remove at least 1,261 submissions from 2,250, reaching at most 989.**

C changes only `state-signatures-1`, `state-flow-1`, `state-projection-recovery-2`, and `state-flow-project-1`.
Each replaces 25 syntax input partitions and five SQL reconstruction stages with the existing tape-backed syntax path.
A generic `requireNonemptyInput` precondition skips SQL admission for complete empty evidence.
The detector SQL and file/repository partition semantics remain intact.
The [specification](sql-execution-chunk-c.md) fixes the transformations, runtime integration, files, tests, and exclusions.

The frontend-only experiment measures 2,250 → 1,489, a net removal of 761.
It passes all 77 recorded state observations.
Five matched pairs record median CPU of 1,060.459 → 427.484 ms and median peak RSS of 325,600 → 285,312 KiB.
These are frontend-prototype results on the small guard, not completed C or repository measurements.

The nine remaining state sessions with empty syntax gates submit 500 statements.
Early admission removes these sessions before SQLite, with no SQL probe or fresh index construction.
That component remains modeled until runtime implementation.
The [scope ledger](../fixtures/host/data-layer-port/profiling/sql-execution-chunk-c-20261024.json) records the complete trace, empty sessions, controls, and hashes.

The required combined removal is 761 + 500 = 1,261. A 1,000-removal result does not complete this chunk.
C leaves a projected 989 submissions, still 889 above the unchanged final limit.
Job-wide storage, general derived-producer sharing, compiler-affinity redesign, and a replacement scheduler remain outside this delivery.
The earlier architecture-wide C proposal in `15930629` is superseded, not implemented.

## Accounting and delivery gates

R5-A receives credit for replacing 1,588 candidate drops with 324 deletes and four final drops.
R5-B receives credit only for its measured subsequent reduction from 324 deletes to 174.
The second chunk does not claim the original drop savings again.
Relational lifecycle counts exclude the PRAGMAs and empty schema probes priced separately.
After each accepted chunk, reprice the next chunk from the new complete trace and subtract work already removed.

A chunk is complete only when the complete job removes its full stated target against the preceding accepted checkpoint.
R5-A must remove at least 3,474 submissions; a 1,000-submission reduction does not complete this chunk. Do not stop after a supporting micro-optimization or a small green test.
Require complete parity, execution coverage, matched CPU controls, and memory measurements.
Fewer submissions do not permit a CPU or memory regression. The rejected family-packing implementation remains rejected.
Count replacement INSERTs, DELETEs, schema changes, transactions, publication, and cleanup.
Retain the fixed 100 limit without resets, reclassification, or truncated findings.

The accepted R5-A and R5-B results leave 2,250 submissions. Delivered C removes another 1,275 and leaves 975.
The current post-C survey records the remaining inventory and the isolated 190-removal admission experiment.
Production remains 875 submissions above the final limit. Unmeasured architectural opportunities have no assigned net savings.
CPU-only improvements, cheaper private CTEs, and speculative cache hits do not contribute to the submission targets.
SQLite internal executions remain separately reported and do not inflate the projected engine savings.
This plan creates no Linear issues and adds no optional production execution path.
