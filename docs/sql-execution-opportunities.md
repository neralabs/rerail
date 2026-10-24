# SQL execution reduction opportunities

Each implementation delivery must remove its chunk's **full expected number of engine submissions** on the complete fixed guard workload.
Small TDD commits can support that delivery. They do not complete a performance phase on their own.
This plan supersedes the earlier practice of stopping after a small optimization slice.

The baseline is `188fb8f4`: **7,733 engine submissions**, **994 SQLite internal executions**, and zero unknown origins.
The workload includes positive and clean state-tripwire observations through snapshot and workspace execution.
It is the complete small guard, not the full data-layer repository. These counts must not be extrapolated linearly to the repository.
The [count ledger](../fixtures/host/data-layer-port/profiling/sql-execution-opportunities-20261024.json) records source hashes, witnesses, formulas, dependencies, and overlap.

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
The [C specification](sql-execution-chunk-c.md) now owns the complete remaining gap after A and B.

## Measured inventory

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
C has a fixed complete-job target. Its replacement schedule remains unproven.
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

## R5-C: complete shared execution and the cold budget

**Target: remove at least 2,150 further submissions from 2,250, reaching at most 100.**

C owns the entire remaining gap. It cannot finish with an unassigned remainder.
The [specification](sql-execution-chunk-c.md) defines shared populated owners, typed operation batches, complete lifetimes, and scoped consumers.
It also includes cold compilation, publication, result emission, and cleanup.
The [C ledger](../fixtures/host/data-layer-port/profiling/sql-execution-chunk-c-20261024.json) reconciles the complete B trace by phase.

The remaining phase counts are 244 setup, 36 file publication, 12 catalog publication, 702 candidate preparation, 1,091 relational evaluation, and 165 cleanup.
They sum to 2,250. Component savings cannot overlap or omit replacement work.
The first implementation step must produce an executable replacement schedule within 100, including every infrastructure statement.
No invented component caps or warm-cache assumptions establish feasibility.

Completion requires at least 2,150 actual removals, complete parity, genuine populated-owner sharing, lower CPU, and bounded memory.
TDD checkpoints support one delivery. Schema reuse or a smaller numerical reduction does not complete it.

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

The accepted R5-A and R5-B results leave 2,250 submissions, still 2,150 above the final limit.
The C specification assigns that entire measured gap to one delivery. Its complete replacement schedule still needs proof.
CPU-only improvements, cheaper private CTEs, and speculative cache hits do not contribute to the submission targets.
SQLite internal executions remain separately reported and do not inflate the projected engine savings.
This plan creates no Linear issues and adds no optional production execution path.
