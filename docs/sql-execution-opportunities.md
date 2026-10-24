# SQL execution reduction opportunities

Each implementation delivery must remove its chunk's **full expected number of engine submissions** on the complete fixed guard workload.
Small TDD commits can support that delivery. They do not complete a performance phase on their own.
This plan supersedes the earlier practice of stopping after a small optimization slice.

The baseline is `188fb8f4`: **7,733 engine submissions**, **994 SQLite internal executions**, and zero unknown origins.
The workload includes positive and clean state-tripwire observations through snapshot and workspace execution.
It is the complete small guard, not the full data-layer repository. These counts must not be extrapolated linearly to the repository.
The [count ledger](../fixtures/host/data-layer-port/profiling/sql-execution-opportunities-20261024.json) records source hashes, witnesses, formulas, dependencies, and overlap.

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

These are **net planning targets**, not achieved reductions or guarantees about an unimplemented design.
Current execution counts are measured. Replacement budgets must pass a compiled-plan audit and complete runtime accounting.

| Rank | Opportunity | Before | Replacement budget | Net removal target | Delivery chunk |
| --- | --- | ---: | ---: | ---: | --- |
| 1 | Maintain candidate key indexes across producer fills | 1,588 | 4 indexes | **1,584** | R5-A |
| 2 | Batch typed candidate population, including initial store creation | 1,588 | 250 fills + 4 store creates | **1,334** | R5-A |
| 3 | Share relational backing, indexes, statistics, and retirement | 2,130 | At most 1,130 | **At least 1,000; provisional** | R5-B |
| 4 | Coalesce candidate retirement after operator batching | 1,028 after R5-A | At most 200 | **828** | R5-B |
| 5 | Retire candidate rows at existing boundaries instead of dropping each table | 1,588 | 1,028 deletes + 4 final store drops | **556** | R5-A |
| 6 | Bound clause-reader configuration to four readers per observed execution context | 136 PRAGMAs | At most 32 | **104** | R5-B |
| 7 | Replace executed empty syntax-schema CTAS with validated prepare-only metadata | 68 | 0 | **68** | R5-B |

The 68 schema probes end in `LIMIT 0`. Their removal must preserve column names, affinities, and full statement validation.
The 136 PRAGMAs belong to 68 clause connections. The proposed pool allows 16 readers across the four observed execution contexts.
New attachment, isolation, publication, or maintenance statements count against the net delivery target.

## R5-A: candidate ownership and typed operation batching

**Target: remove 3,474 submissions, reducing the complete job from 7,733 to at most 4,259.**

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

**Provisional target: remove at least 2,000 additional submissions, reaching at most 2,259 after R5-A.**

| Component | Current or modeled baseline | Replacement cap | Net target |
| --- | ---: | ---: | ---: |
| Relational object, index, statistics, and retirement lifecycle | 2,130 | 1,130 | 1,000 |
| Candidate row retirement after R5-A | 1,028 | 200 | 828 |
| Clause connection PRAGMAs | 136 | 32 | 104 |
| Empty syntax-schema CTAS | 68 | 0 | 68 |
| **Net target** | | | **2,000** |

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

## Accounting and delivery gates

R5-A receives credit for replacing 1,588 candidate drops with 1,028 deletes and four final drops.
R5-B receives credit only for the subsequent reduction from 1,028 deletes to 200.
The second chunk does not claim the original drop savings again.
Relational lifecycle counts exclude the PRAGMAs and empty schema probes priced separately.
After each accepted chunk, reprice the next chunk from the new complete trace and subtract work already removed.

A chunk is complete only when the complete job removes its full stated target against the preceding accepted checkpoint.
R5-A must remove at least 3,474 submissions; a 1,000-submission reduction does not complete this chunk. Do not stop after a supporting micro-optimization or a small green test.
Require complete parity, execution coverage, matched CPU controls, and memory measurements.
Fewer submissions do not permit a CPU or memory regression. The rejected family-packing implementation remains rejected.
Count replacement INSERTs, DELETEs, schema changes, transactions, publication, and cleanup.
Retain the fixed 100 limit without resets, reclassification, or truncated findings.

The two targets leave at most 2,259 submissions, still 2,159 above the final limit.
That remainder needs another measured inventory. No invented saving is assigned to it now.
CPU-only improvements, cheaper private CTEs, and speculative cache hits do not contribute to the submission targets.
SQLite internal executions remain separately reported and do not inflate the projected engine savings.
This plan creates no Linear issues and adds no optional production execution path.
