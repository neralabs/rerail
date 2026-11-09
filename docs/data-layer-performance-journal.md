# Data-layer performance journal

The [incremental SQL preparation design](sql-preparation-architecture.md) and its
[implementation spikes](sql-preparation-spikes.md) distinguish accepted changes from rejected retention experiments.

This journal records measurements, explanations, and implementation decisions for the data-layer ports.
Each entry identifies its implementation and evidence. Historical results remain intact after a new baseline.
Experiments stay in isolated worktrees or standalone harnesses until a keep-or-reject decision.
The engine retains one chosen execution path. Profiling adds observation and does not select another algorithm.

## Spikes across the remaining inventory: 2026-10-24

Every area in the 975-submission survey now has an executable prototype.
Production retains C. Runtime changes live in an isolated worktree and a checked patch, with no production algorithm flag.
The [design and decisions](sql-post-c-spikes.md) describe the selected shapes and remaining integration.
The [checked evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/post-c-spikes-20261024.json) retains complete trials, hashes, accounting, and validation.

### Combined runtime result

The complete fixed guard decreases from **975 to 718 engine submissions: 257 removed**.
SQLite internal executions decrease from 106 to 100. Unknown origins remain zero, and complete guard parity passes.
The unchanged final 100 gate correctly fails by 618 submissions after complete execution.
The combined count includes publication, setup, transactions, preparation, evaluation, and retirement.

The isolated policy copy adds complete-empty admission to the remaining 28 event clauses.
It also places four nonrecursive single-consumer flow stages inside their consumer SQL scope.
These policy changes measure 975 → 766 on the existing C addon.
The runtime prototype adds eponymous native bindings, metadata-based capture provenance, and fewer incidental candidate cuts.
These changes reduce the combined guard further to 718.
Required posting and interval indexes remain physical boundaries. Separate output packets retain separate reader identities.

| Complete engine work | C | Selected combined prototype |
| --- | ---: | ---: |
| Connection and compiler setup | 229 | 232 |
| File publication | 36 | 36 |
| Catalog publication | 12 | 12 |
| Candidate preparation | 308 | 276 |
| Cleanup | 24 | 4 |
| Relational evaluation | 366 | 158 |
| **Total** | **975** | **718** |

Five alternating fresh-process pairs compare C with the corrected prototype.
Both artifacts use matching build settings, allocator, shim, workers, policies appropriate to each artifact, and the unchanged recording.
Tracing, SQL capture, census, and sampling are off. No other local build or test runs during these controls.
Other host workloads remain uncontrolled.

| Fixed-guard metric | C | Selected combined prototype |
| --- | ---: | ---: |
| Median CPU | 585.845 ms | 579.437 ms |
| CPU range | 573.313–597.824 ms | 554.644–606.254 ms |
| Median process peak RSS | 299,696 KiB | 297,616 KiB |
| RSS range | 292,832–300,528 KiB | 287,232–299,824 KiB |

CPU and memory ranges overlap. This demonstrates fewer submissions, with no established aggregate CPU or memory gain.
These small-guard results do not predict full-repository performance.

### Component choices

The storage prototype compares fresh databases, cleared-store reuse, and private clones of an immutable SQLite schema/catalog seed.
It retains positional FTS and typed bulk publication. Complete logical index fingerprints match across all modes and domains.
Every setup, publication, retirement, and verification statement counts. Backup API calls remain visible and count in CPU.
Four common verification queries appear in each component count.

| Storage component | Fresh | Clear and reuse | Private seed clones |
| --- | ---: | ---: | ---: |
| Engine submissions | 220 | 100 | 127 |
| Internal executions, small input | 236 | 434 | 216 |
| Median CPU, small input | 20.509 ms | 7.803 ms | 18.043 ms |
| Median CPU, 128 copies per source | 63.542 ms | 71.157 ms | 63.242 ms |
| Internal executions, replicated input | 10,912 | 29,791 | 10,892 |

Select private seed clones. Fresh/seed stress CPU ranges overlap, and clones require fewer submissions.
Cleared-store reuse needs a reset protocol and generates substantially more FTS deletion work.
The seed contains no file facts or rule outputs; all mutable domain state remains private.
This addresses schema/catalog repetition. Shared source publication across separate domains remains open.
The component uses Python SQLite, unlike the Rust runtime. Cache limits are explicit; comparable per-mode process RSS is not measured.

The compiler prototype batches independent cold stages into bounded SQLite CTAS inference statements.
It uses the production schema adapter, authorizer, and typed cache. Authored functions never execute during inference.
All declared columns and types match serial inference, including CAST, NULL, BLOB, compound, and window expressions.
Original programs decrease **53 → 31 statements**. The selected flow pipelines change cache reuse; their programs decrease **56 → 35**.
For the latter, median CPU is 22.015 → 22.487 ms, with overlapping ranges.
SQLite heap peaks are 827,872 → 830,640 bytes. Select bounded compiler batching for its lower count and retained SQLite semantics.

Frozen candidate experiments compare the complete output multisets from all four captured domains.
Set folding decreases component submissions **272 → 252**. Unrestricted private cuts reach 226.
CPU ranges overlap, but stepping heap peaks increase from 10,845,456 to 11,833,184 and 11,881,120 bytes respectively.
Do not promote the generated-SQL string rewriter or its unrestricted cuts.
Keep the narrower typed runtime change, which preserves required indexes and passes the broader regressions.
Any later set simplification belongs in the existing typed graph and needs another memory check.

Component counts cannot be added to 718 without integration and another complete-job capture.
The storage and compiler prototypes remain separate from the combined runtime artifact.
No full-repository recovery or final 100-limit acceptance follows from these experiments.

### Regression ladder and validation

The first native-binding prototype omitted empty runtime syntax relations.
A new empty/repeated-empty/populated-domain test caught the omission.
The compiler's prepare-only schema cannot serve as an executable empty source. The corrected adapter provides a real empty cursor.

The first private-cut heuristic reached 710 submissions but failed three Rust tests.
It removed explicitly required owners and interval indexes, losing both range bounds and linear nesting behavior.
Preserving required indexes raises the count to 718 and passes the original tests without weakening them.
The selector fixture now crosses three packets with both selectors; it passes on C and the corrected prototype.

The corrected artifact passes all 1,604 Rust tests, with 23 existing ignored tests.
It passes 324 native-facing tests across 19 files, all 77 state observations, and 120 positive/clean observations across 60 rules.
Both corpora report zero false positives, false negatives, engine failures, or execution-path differences.
The preparation host passes 60 tests. Six new Python semantic tests cover the standalone rewrites and schema fixtures.
Profiling and budget tooling, TypeScript, boundaries, structure, Windows paths, formatting, and profiling-enabled all-target Clippy pass.
Initial all-target Clippy compilation exceeded the deadline; splitting out the library check allowed the all-target retry to finish within it.
The nonprofiling runtime library also passes Clippy.

Raw builds, rejected attempts, controls, and captures reside under `.rerail/profiles/post-c-spikes-20261024/`.
Builds use the standing permission to finish. Runtime experiments and checks retain the 115-second deadline.
No Linear tickets are created.

## Remaining SQL submission survey: 2026-10-24

Production remains at `82d82d89`: 975 engine submissions, 106 SQLite internal executions, and zero unknown origins.
The [checked survey](../gate/data-layer/fixtures/host/data-layer-port/profiling/post-c-sql-survey-20261024.json) assigns every engine statement to one disjoint bucket.
Candidate execution costs 312, event clauses cost 202, flow execution costs 184, index/reader setup costs 176, compiler inference costs 53, and publication costs 48.
The guard includes two observations through snapshot and workspace paths. Four physical index lifetimes cover two source hashes.
These lifecycle counts do not predict full-repository CPU savings.

### Isolated admission experiment

The other 28 state clauses have one `events` input. They do not yet use C's explicit input precondition.
The trace contains 55 complete empty event evaluations and two nonempty evaluations.
An isolated policy copy adds `requireNonemptyInput: events`, using the unchanged C addon.
The complete guard decreases from 975 to 785 submissions: 190 actual removals, including replacement and cleanup work.
Both guard observations retain complete parity and trace coverage. The final 100 gate still correctly fails.
All 77 state observations pass: 64 positive and 13 clean, with zero engine failures or execution differences.

Five fresh alternating pairs compare the two policy versions under the same C addon.
Tracing, phase profiling, SQL capture, census, and sampling are off in every control child.
Median CPU is 575.665 → 587.471 ms, an observed increase of 2.05%.
CPU ranges overlap: 574.271–599.989 ms before and 555.689–596.872 ms after.
Median peak RSS is 300,928 → 297,328 KiB. Every trial retains parity.
This series demonstrates no CPU improvement. It is separate from the earlier B/C delivery controls.
The policy remains isolated. The 190 removals can contribute to a broader delivery that also passes CPU and memory acceptance.

### Remaining architecture work

Candidate execution includes 110 key fills, 66 retirement DELETEs, 20 span fills, and 28 child fills.
Their zero-change counts are 74, 26, 16, and 22 respectively. Zero changed rows do not prove safe omission.
The planner already batches roots and decoder demand. Remaining savings require fewer physical cuts or genuine dependency barriers.
The next experiment must compile captured typed plans into bounded private SQL pipelines and selected shared owners.
It must preserve dependency order, integer affinity, complete empty publication, and bounded retention.

Index setup costs 168 statements, from four repetitions of 42. Reader setup adds eight.
One physical schema identifies at most 126 gross avoided initialization statements.
A shared execution owner must preserve separate logical domains, source versions, permissions, completion, and both parity paths.
New domain publication, cleanup, and maintenance reduce that gross saving. No net target is assigned yet.
The positional FTS index remains unchanged.

Flow execution costs 92 statements twice. Each execution has 24 SQL-derived relations and one native reachability boundary.
Private single-use relations can form pipelines. Indexed and multiply consumed relations need explicit owners.
Eponymous native relations identify 16 CREATE/DROP statements for investigation, with runtime rebinding and isolation still unproven.
The compiler's 53 cold inference statements require a replacement that preserves SQLite CTAS affinity and unconditional SQL validation.
Publication remains typed and batched. Moving writes into triggers does not establish a reduction.

Four counted candidate statements come from capture provenance reads of file versions.
The profiler can use existing corpus metadata instead. Those statements remain counted until their queries are actually removed.
The current capture therefore includes a small observer cost, which this survey does not reclassify or subtract.

The [opportunity plan](sql-execution-opportunities.md#post-c-survey-current-975-submissions) records the recommended order and pricing requirements.
No production policy, engine implementation, acceptance limit, or Linear ticket changes during this survey.
Raw audits, the policy copy, SQL captures, corpus replay, and all control trials reside under `.rerail/profiles/post-c-survey-20261024/`.
Every experiment completes within the 115-second deadline.

## State syntax domain delivery R5-C: 2026-10-24

**Result: 1,275 engine submissions removed against the unchanged 1,261 target.**
The complete fixed guard decreases from B's 2,250 submissions to 975, within C's 989 ceiling.
SQLite internal executions decrease from 181 to 106. Unknown origins remain zero, with complete connection coverage and findings.
The guard recording matches B byte for byte.
The mandatory final 100 limit still fails by 875 submissions. The launcher correctly returns failure after complete execution.
These measurements cover the small guard, including snapshot and workspace paths. They do not establish full-repository recovery.

### Implementation and accounting

The production generator migrates exactly four state frontends to the existing tape-backed syntax relations.
It removes their 25 AST partitions and five reconstruction stages. Detector decisions remain SQL.
The other 28 state clauses retain their definitions. Full pinned-source regeneration matches all ten generated policies byte for byte.
The migration preserves argument text, field identity, owner selection, ordinals, and descendant expansion.
Two clauses retain repository partitioning for `scan_project_sources`; their default partition remains file.

The new `relational.requireNonemptyInput` field names one declared input.
Complete empty evidence skips the SQL reader and body preparation. Nonempty evidence transfers into input import without another extraction.
Incomplete evidence, cancellation, budgets, and invalid SQL remain failures.
Gate/body preparation uses the existing scheduler and candidate frontier.
Repository syntax restores only eligible current documents, including evicted documents, through bounded metadata, source, and tape batches.
The loader preserves hash checks and charges repeated contents separately for each logical file.
A regression test reads 257 cold sources with three SQL statements. Warm source reads need no SQL.

The frontend experiment priced 761 removals. Empty-session SQL accounts for another 500 removals in the final run.
Seven avoided reader configurations remove two PRAGMAs each, adding 14 removals beyond the original model.
All replacement statements, transactions, publication, and cleanup remain counted.

| Complete engine work | B | C | Removal |
| --- | ---: | ---: | ---: |
| Connection and compiler setup | 244 | 229 | 15 |
| File publication | 36 | 36 | 0 |
| Catalog publication | 12 | 12 | 0 |
| Candidate preparation | 702 | 308 | 394 |
| Cleanup | 165 | 24 | 141 |
| Relational evaluation | 1,091 | 366 | 725 |
| **Total** | **2,250** | **975** | **1,275** |

### TDD ladder and validation

Red commit `1459c56c` records the missing frontend shape, input precondition, and C progress receipt.
The original generator exposes 25 inputs where the test requires one. The original engine rejects `requireNonemptyInput`.
The implementation adds the generic lowering helper, compiler field, admission check, and preparation phases.
Further tests cover nonempty row transfer, incomplete inputs, cancellation/retry, edits, removals, repeated result pages, and evicted repository documents.
Direct relation comparisons cover Unicode, optional fields, nested owners, ordinals, and both descendant expansion modes across two files.
An explicit empty aggregate test preserves unguarded SQL behavior. Profile assertions require zero relational SQL or syntax preparation after empty admission.

The final addon passes 323 native-facing tests across 18 files.
The full Rust suite passes 1,604 tests, with 23 existing ignored tests and zero failures.
All 77 recorded state observations pass: 64 positive and 13 clean.
The selected corpus passes 120 observations across 60 rules: one positive and one clean observation per rule.
Both corpora report zero false positives, false negatives, execution differences, or engine failures.
Twelve generator tests and 14 budget regression tests pass.
TypeScript, boundaries, structure, Windows paths, formatting, and all-target Clippy pass with and without profiling.
The language reference retains 38 matching grammar productions and 98 compilable examples, with eight fragments and zero failures.
The budget launcher records a successful C receipt and preserves the failing final 100 gate.

### Matched CPU, memory, and remaining work

Five alternating fresh-process pairs compare the accepted B addon and policy with the final C addon and policy.
Both builds use the same release optimization settings, allocator archive, shim, worker count, and recording.
No local build or test runs during these controls. Other host workloads remain outside this experiment's control.
Tracing, phase profiling, SQL capture, census, and external sampling are off in every control child.
CPU includes compilation, snapshot/workspace evaluation, and cleanup after module imports.
Peak RSS covers the whole process, including imports. It is a high-water measurement, not exclusive live allocation.

| Matched fixed-guard metric | B | C |
| --- | ---: | ---: |
| Median CPU | 1,586.221 ms | 550.052 ms |
| CPU trial range | 1,534.903–1,612.057 ms | 527.931–569.859 ms |
| Median peak RSS | 337,616 KiB | 298,784 KiB |
| Peak RSS trial range | 328,864–364,464 KiB | 292,400–303,120 KiB |

Median CPU decreases 65.3%. Median peak RSS decreases 11.5%.
Every control retains parity. This timing series is separate from the earlier frontend prototype and A/B controls.
A separate broader-state diagnostic captures two external stack samples and one `vmmap` snapshot, with parity intact.
The samples show regex search, SQL preparation/parsing, hashing, allocation, and copies among the remaining work.
They also contain scheduling and I/O frames. Their wall-stack percentages are not CPU shares.

The next inventory starts at 975 actual submissions, with 875 remaining above the final limit.
Candidate preparation contributes 308, relational evaluation contributes 366, and connection/compiler setup contributes 229.
These counts identify remaining work. They do not price a new implementation chunk or establish full-repository performance.
The final 100 gate and broader recovery requirements remain open.

The [checked delivery evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/state-domain-admission-recovery-20261024.json) records all trials, source and artifact hashes, validation, and accounting.
Raw builds, captures, controls, external profiles, and parity reports reside under `.rerail/profiles/chunk-c-20261024/`.
Builds use the standing permission to finish. Runtime checks and experiments remain within 115 seconds.

## Bounded C scope and frontend experiment: 2026-10-24

The user rejected the architecture-wide C proposal because it specified an outcome without a bounded implementation scope.
The [replacement specification](sql-execution-chunk-c.md) selects exactly four state-analysis frontends and one generic nonempty-input precondition.
It excludes job-wide storage, general derived-producer sharing, compiler-affinity redesign, and a replacement scheduler.
The fixed delivery target is 1,261 actual removals, from 2,250 to at most 989 complete-guard engine submissions.
The final 100 limit remains unchanged. The projected remainder is 889 above that limit.

The concrete omission is the state frontend's 25 typed AST input partitions.
The quality rules already use the tape-backed syntax path, but these four state consumers still reconstruct nodes, edges, ownership, and staging unions.
An isolated generated-policy transformation uses the existing B addon to replace those frontends.
It preserves the other 28 detector clauses and the file/repository partitions.
The prototype is not the production generator implementation.

The unchanged guard measures 1,489 engine submissions, 115 SQLite internal executions, and zero unknown origins.
That is 761 measured engine removals.
All 77 recorded state observations pass: 64 positive and 13 clean, with zero parity differences or failures.
A separate output-gated control also passes all 77 observations.
That control suppresses output for an empty syntax gate but still executes its SQL. It does not demonstrate early-admission savings.

The remaining trace contains nine complete empty-gate state sessions with 500 engine statements, plus two nonempty state-flow sessions.
A gate before connection admission models another 500 removals without extra SQL.
The trace audit finds no deleted empty-session schema creation that supplies either surviving flow session.
The combined 1,261 target remains subject to complete production accounting, rather than subtraction alone.
No implementation checkpoint is accepted yet.

Five alternating fresh-process pairs compare the original and transformed policy under the same addon, workers, allocator, and recording.
Median CPU is 1,060.459 → 427.484 ms, a 59.7% decrease.
Median peak RSS is 325,600 → 285,312 KiB.
These controls cover only the frontend experiment and small guard. They are independent of the earlier B timing series.
They do not measure the unimplemented input precondition or the complete repository.

The [checked scope evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-execution-chunk-c-20261024.json) retains the exact consumers, transformed schemas, sessions, trials, and artifact hashes.
Raw policies, transformation code, controls, and reports reside under `.rerail/profiles/chunk-c-scope-20261024/`.
All experiment commands complete within the 115-second deadline. No production code or Linear tickets change during this scoping work.

## Complete shared execution specification R5-C: 2026-10-24

Status: superseded by the bounded C specification. This architecture-wide proposal did not deliver an implementation.
A and B remove 5,483 submissions but leave 2,250. Their original combined budget also leaves approximately that remainder.
The proposal in `15930629` assigns the entire remaining 2,150-execution gap to one delivery.
That proposal requires 100 or fewer engine submissions. The replacement bounded scope does not retain that delivery target.

C requires shared populated owners and completion across compatible consumers, rather than only empty schema and connection reuse.
It includes typed operation batches, private pipelines, version membership, scoped readers, and complete producer lifetimes.
Cold compilation, publication, result emission, and cleanup remain inside the same budget.
The first implementation step must price the complete executable schedule. Replacement counts and CPU improvement remain unproven.

The [frozen C ledger](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-execution-chunk-c-20261024.json) records all six remaining phases and their source hashes.
Its counts sum to 2,250, with 181 separately reported SQLite internal executions.
Acceptance also requires complete parity, genuine nonempty shared execution, repeatable CPU improvement, and no memory regression.
Smaller TDD commits support C but cannot complete the delivery by themselves.

## Relational storage delivery R5-B: 2026-10-24

**Result: 1,435 engine submissions removed, exceeding the full 1,296 target.**
The complete fixed guard falls from the accepted A checkpoint of 3,685 submissions to 2,250.
SQLite internal executions fall from 994 to 181. Unknown origins remain zero, with complete connection coverage and findings.
Input recording bytes match A exactly. The unchanged 100-submission gate still fails, by 2,150 submissions.
This result covers the fixed state-tripwire guard, not the full data-layer repository.

### Selected execution shape and accounting

Readers reuse empty typed SQLite tables, indexes, and views when their complete declarations match.
Input bindings retain separate row lifetimes. This change does not infer shared data from equal SQL text.
Actual insert counts track occupancy. Retirement deletes populated backing at its original logical last use.
Empty candidate slots complete normally and need no DELETE.
Statistics refresh for populated tables and transitions to empty; fresh empty tables avoid redundant ANALYZE.
Generic graph construction stops early only when normalized roots and edges contain no vertices.
Edges still undergo validation when roots are empty.

A pool retains at most four idle readers. Existing resident admission still governs active consumers and broad producer waves.
Each idle reader retains at most 128 reusable objects, 4 MiB of metadata, and 16 MiB of SQLite plus metadata storage.
Admission includes idle storage. Failed or unfinished sessions discard their readers.
Successful reuse clears rows and source/regex callbacks, so snapshots and rule budgets do not leak across consumers.

| Complete engine work | A | B | Net removal |
| --- | ---: | ---: | ---: |
| Runtime relational statements, including replacement writes | 2,409 | 1,252 | 1,157 |
| Reader configuration PRAGMAs | 136 | 8 | 128 |
| Candidate retirement DELETEs | 324 | 174 | 150 |
| All other engine work | 816 | 816 | 0 |
| **Total** | **3,685** | **2,250** | **1,435** |

Runtime relational CREATE statements fall from 1,192 to 489; DROP falls from 628 to 161; ANALYZE falls from 310 to 39.
Replacement INSERTs rise from 42 to 310, and DELETEs rise from zero to 43. All remain counted.
The observed 68 runtime reader connections become four, one per observed execution context.
The planned removal of 68 compiler empty-schema probes is **not implemented**. Those probes remain counted.
Other measured components exceed the complete net target by 139, without lowering that target.

### TDD ladder and validation

Red commit `92879cd5` requires reusable relational backing and the full 1,296-removal delivery target.
The accepted A implementation configures 18 reader functions where the test requires three.
The first implementation removes only 1,040 submissions. It remains an unsuccessful experiment.
The next implementation reaches 2,250 and preserves the selected data-layer observations, but fails a new generic deferred-CAST test.
Red commit `283b0fbf` records that failure: a typed comparison loses its expected finding after storage reuse.

The correction makes compiler syntax declarations match runtime affinity.
SQLite infers public deferred projection affinity once per typed schema and query; physical backing keeps its own declaration.
No custom SQL type parser is added. Cold inference statements remain visible in execution accounting.
The compiler-schema test now counts all six cold statements, including the public-affinity probe and cleanup.
Cold and repeated compilation retain the CAST finding.

The final addon passes 314 native-facing tests across 16 files, 32 relational Rust tests, and 79 candidate SQL tests.
One existing candidate test remains ignored. The standalone host passes 57 tests; 13 budget regression tests pass.
The corpus selection retains 60 positive and 60 clean observations across 60 rules, with zero parity differences or failures.
Clippy passes for native targets with and without profiling, and for the standalone host.
TypeScript, boundaries, structure, Windows paths, and formatting checks pass.
These selected checks do not replace the still-open full-repository recovery gate.

### CPU, memory, and the next frontier

Five alternating fresh-process pairs compare the A addon with the selected B addon under matched build and worker settings.
Tracing and allocation accounting are off. CPU includes compilation, snapshot/workspace evaluation, and cleanup after module imports.
Median CPU is **1,546.054 → 1,541.322 ms**, a 0.3% decrease: roughly flat.
Median peak RSS is **335,536 → 332,960 KiB**. Trial ranges overlap; this is a small measured difference.
The frozen candidate replay retains the exact 161-row bag and its 19,269,664-byte SQLite heap peak.
That isolated candidate result does not measure complete relational memory.

The engine submission reduction is 38.9%, but it does not deliver a substantial CPU speedup on this guard.
An external bottom-up sample with tracing off shows allocation/free, copying, regex search, and regex determinization among self frames.
Only one stack capture completes on this short workload. Its wall observations do not establish CPU shares.
A separate traced capture spends much of its sampled time writing instrumentation; it is unsuitable for normal-workload CPU attribution.
The next investigation should measure compilation, lowering, allocation, and regex costs with a broader bounded sample.
The remaining 2,150 excess submissions also need a new complete inventory before the next delivery receives a target.

The [checked evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/relational-storage-recovery-20261024.json) retains all timing trials, exact accounting, artifact hashes, and experiment decisions.
Raw reports and immutable addons reside under `.rerail/profiles/chunk-b-20261024/`.

## Candidate ownership delivery R5-A: 2026-10-24

**Result: 4,048 submissions removed, exceeding the full 3,474 target.**
The complete fixed guard falls from 7,733 to 3,685 engine submissions.
It retains 994 SQLite internal executions, zero unknown origins, complete connection coverage, and full findings.
The workload bytes match the frozen `188fb8f4` baseline exactly.
The final 100-submission gate still fails. Its allowance and enforcement remain unchanged.

The owner stores typed `(producer, fid, pre)` rows under one nonunique composite index per execution domain.
Independent dependency frontiers share a bounded SQL population statement and one WITH scope.
The compiler preserves upstream materialization fences and logical rule admission.
Batch membership records completion even when a producer emits no rows.
Each logical producer retains its own last-use boundary; adjacent releases share one DELETE.
Other schemas retain separate tables. Retained read/write barriers still come from the existing specialized scheduler.
No engine algorithm switch or native rule evaluator is added.

The original candidate lifecycle costs 4,764 submissions. Its replacement costs 716:

| Operation | Measured submissions |
| --- | ---: |
| Store creation | 4 |
| Index creation | 4 |
| Population | 380 |
| Row retirement | 324 |
| Final store drops | 4 |
| **Total** | **716** |

Population exceeds its original 250 planning cap by 130 statements.
Retirement beats its 1,028-statement model by 704. The full net target is exceeded by 574.
All other 2,969 engine submissions remain in the complete accounting.
This delivery therefore meets its full target through a different measured fill and retirement balance.
The check does not substitute a 1,000-removal floor for the expected 3,474 removals.
The launcher records `chunk-a-budget.json`, while the independent 100-submission gate still exits nonzero.

### TDD and root causes

Red commit `321f6ec9` requires two independent selectors to share one indexed owner.
The frozen baseline fails with two creates, two indexes, and two drops instead of one each.
The selected implementation passes that test and preserves all 130 findings across two packets.
SQL tests cover duplicate rows, NULLs, integer affinity, empty completion, individual retirement, and repeated execution domains.
An index-plan test retains both interval bounds after the producer prefix.
A native execution test verifies cleanup after a later query error, then successful retries.

A further red replay test exposes invalid alias placement for a different temporary schema: WITH precedes CREATE.
The fix puts that WITH inside the table's SELECT body. The replay then preserves the different schema and duplicate rows.
The final SQL unit run covers 78 passing tests and one existing ignored test.
The selected addon also passes 312 native-facing tests, 54 standalone SQL tests, and 12 budget regression tests.
The original corpus selection retains 60 positive and 60 clean observations across 60 rules, with zero differences or failures.
Native Clippy passes for all targets with and without profiling. Standalone Clippy and the static checks pass.

The first ownership implementation reduces submissions but increases median CPU by 4.3%.
It builds two schedules before the final schedule and scans alias SQL that it discards.
Removing that work resolves the regression. Owner-name metadata is also built once rather than at every retirement boundary.
The checked evidence retains every timing trial:

| Experiment | Pairs | Before CPU median, ms | After CPU median, ms | Decision |
| --- | ---: | ---: | ---: | --- |
| Initial ownership implementation | 5 | 1,681.999 | 1,754.292 | Reject this version |
| Single final schedule and no discarded alias rendering | 3 | 1,678.267 | 1,610.250 | Provisional validation |
| Complete validation before owner-name metadata hoist | 5 | 1,701.422 | 1,657.521 | Retain measurements |
| Delivery build | 5 | 1,695.593 | 1,653.416 | Select |

The first pair in the initial series overlaps a short diagnostic memory replay. It is excluded from final acceptance.
The delivery series runs after our builds, tests, and diagnostic captures finish, with no overlapping work from this agent.
It alternates fresh processes against the frozen baseline addon, with identical inputs, build settings, allocator shim, and four workers.
CPU covers the full replay interval after the driver imports its modules. Tracing and allocation accounting are disabled.
The delivery CPU median falls 2.49%; ranges overlap: 1,613.386–1,716.013 ms before and 1,569.702–1,683.505 ms after.
The shared host and modest change do not support a repository-wide speedup claim.
Reported peak RSS medians are 378,784 and 335,520 KiB, with variable, overlapping ranges.
The preceding five-pair series has nearly flat RSS medians. Do not assign the delivery series' RSS decrease entirely to this change.

### Memory, replay, and remaining work

The new `candidate_owner` binary applies the production ownership compiler to a frozen logical plan.
It preserves the original consumer ordinals and rejects existing outputs or already shared programs.
The replay uses the production SQLite adapters. It does not evaluate rules in Rust.
The exact 161-row candidate multiset matches, including multiplicity.
With identical 1,024/512 KiB main/temp cache budgets, SQLite stepping heap falls from 19,793,328 to 19,269,664 bytes.
Compilation peak falls from 3,397,936 to 3,040,608 bytes.
Stage/result VM steps fall from 73,349 to 62,073. Sorts change from 1,815 to 1,812.
Peak live logical bindings increase from 93 to 110 because independent producers now complete together.
Each binding still retires after its last consumer. The measured heap peak remains lower despite that scheduling change.
Diagnostic replay timings are separate from the untraced CPU controls.

Format-v3 captures include owner setup, slots, batch members, the logical graph, source generations, and the executable schedule.
The cache-budget replay follows that schedule instead of reconstructing dependencies from the shared table's writes.
SQL batch duration belongs to the batch. Use the captured membership when analyzing its logical producers.
Two external sample captures still show regex construction, allocation/memory movement, and SQLite preparation.
These are wall-time stack observations, not CPU-time shares.

R5-B is repriced to 1,296 additional removals from the measured 3,685 baseline.
Its retirement component is now 324 to 200, rather than 1,028 to 200. It cannot claim savings already delivered here.
The updated plan leaves 2,389 submissions after that target, still above the final 100 limit.
This chunk does not run the full repository or the complete 3,210-observation corpus.

Evidence: [checked measurements and all timing trials](../gate/data-layer/fixtures/host/data-layer-port/profiling/candidate-ownership-recovery-20261024.json).
Raw captures and commands remain in `.rerail/profiles/chunk-a-20261024/`.

## Original execution reduction planning: 2026-10-24

Each implementation delivery must remove its full expected number of complete-job engine submissions.
Small TDD commits remain useful, but they do not end a performance phase.
The [ranked opportunity plan](sql-execution-opportunities.md) replaces micro-optimization milestones with two coherent delivery chunks.

The current 7,733 submissions include 1,588 candidate table creates, 1,588 candidate index creates, and 1,588 candidate drops.
The existing candidate schedules contain 1,028 adjacent retirement groups across four execution contexts.
R5-A targets 3,474 net removals through stable indexed stores, typed operation batching, and bulk retirement at those existing boundaries.
Its complete-job target is at most 4,259 submissions. Replacement publication and maintenance count against that budget.

R5-B provisionally targets at least 2,000 additional removals through shared relational backing and complete session lifetimes.
Its measured inputs include 2,130 relational lifecycle executions, 136 clause-configuration PRAGMAs, and 68 empty syntax-schema CTAS executions.
Its retirement target starts from R5-A's replacement deletes, so the same drop removals are not counted twice.
Schema and binding compatibility must be audited before producer data reuse receives credit.
These are planning targets. No corresponding runtime saving has been achieved in this planning turn.

The [checked ledger](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-execution-opportunities-20261024.json) reconciles every engine submission and records assumptions and source hashes.
CPU, memory, complete parity, and the fixed 100-submission requirement remain mandatory.
After those targets, at most 2,259 submissions remain. Their next reductions require a fresh inventory rather than fabricated estimates.

## Selector SQL owners: 2026-10-24

### Physical owner projection: green

The test from `97aad726` now passes. Different selectors retain their existing owners and all 130 findings across two result packets.
The final `(fid, pre)` CTE streams into its CTAS owner instead of constructing another ephemeral copy first.
The explicit integer casts, nonunique indexes, duplicate rows, NULL behavior, and upstream materialization boundaries remain unchanged.
Source-span fences still precede expensive source predicates.
Dependency analysis also filters identifiers to actual physical tables and retained resources before it sorts them.
The existing scheduler and its read/write ordering remain authoritative.

Bytecode tests prove one fewer `OpenEphemeral` operation for a physical owner.
Another test proves that two reads still construct a shared source once.
The selected addon passes **312 native-facing tests**, **50 standalone SQL tests**, and **120 original observations across 60 rules**.
The observations include 60 positive and 60 clean cases, with no diagnostic differences or engine failures.
All eleven fixed-budget regression tests and the producer-manifest check pass.
Native Clippy passes with and without profiling. Standalone Clippy, static checks, formatting, and patch checks pass.
An initial native run also discovers an archived test under the local experiment directory.
Renaming archived sources as snapshots removes that unintended test. The complete selected group then passes.

Five alternating fresh-process pairs compare the frozen `66f6a522` addon with the selected implementation.
Median process CPU changes from **1,650.576 to 1,638.666 ms**, a **0.72% decrease**.
The before range is 1,543.508–1,699.889 ms. The after range is 1,562.985–1,675.427 ms.
The ranges overlap. This result establishes no substantial speedup.
Median reported peak RSS changes from 337,696 to 332,320 KiB, also with overlapping ranges.
The timing series uses identical build settings, allocator shim, frozen inputs, and four workers, with tracing disabled.
Own builds, tests, and diagnostic captures finish before timing. The host also runs unrelated worktree builds.

The complete small guard repeats at **7,733 engine submissions**, **994 SQLite internal executions**, and **8,727 total executions**.
The fixed 100-submission gate correctly still fails. Findings are complete and connection coverage reconciles.
Candidate preparation remains at 3,316 submissions and cleanup at 2,216.
The two frontier shapes retain 393 and 401 physical tables, with 93 and 91 peak live tables.
Removing a private ephemeral copy changes VM work rather than engine submission counts.

The new `owner_projection` replay command applies the production rewrite to a copied SQL capture.
It preserves the original consumer ordinals for an exact intermediate comparison and refuses to overwrite an existing output.
The exact **161-row multiset** matches, including multiplicity.
Stage/result VM steps decrease from **75,068 to 73,183**. Sorts remain at 1,815.
The SQLite stepping heap peak remains **19,793,328 bytes**. This small workload establishes no memory recovery.
Five paired diagnostic replays record median SQL API CPU of 83.155 ms before and 81.758 ms after.
Those replays enable allocation accounting. They remain separate from the complete-replay CPU measurements above.
Native consumer ordinals can differ across processes, so native acceptance compares final diagnostics and same-object permutation audits.

**Decision:** retain the owner-projection rewrite and relevant-identifier filtering. Remove generic family packing and scalar-fusion packing from the engine.
The rejected paths reduce submissions but regress CPU. Their raw captures, all timing trials, and source snapshot remain available.
The checked [recovery evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/selector-materialization-recovery-20261024.json) records both decisions.
The [rejected source artifact](../gate/data-layer/fixtures/host/data-layer-port/profiling/rejected-selector-family-spike-20261024.patch) preserves the abandoned design for review.
Neither artifact selects an alternate production algorithm.
Raw evidence remains in `.rerail/profiles/selector-family-recovery-20261024/`.

The next design step must batch typed producer operations before SQL rendering.
Packing complete rendered selector trees repeats compiler processing and increases SQLite preparation expansion.
Named owners, closed domains, and explicit dependencies must guide compatible operator batches.
This result does not complete WP3, the 100-submission requirement, the full corpus, or repository acceptance.

### Family packing is rejected; the next TDD rung isolates duplicate ownership

The target from `c896b721` passes parity but fails the performance acceptance criteria.
Generic family packing reduces engine submissions from 7,733 to 4,109 after applying a preparation bound.
Its first bounded implementation increases matched median CPU from 1,679.669 to 1,855.333 ms.
Metadata reuse and root pipelining reduce that regression, but a fresh series still records 1,789.405 versus 1,682.382 ms.
All five selected trials in that second series use more CPU than all five controls.
These runtime changes are rejected. Their code is removed instead of remaining behind a switch.

Earlier nested member scopes reach a 53,478,480-byte SQLite heap peak, compared with 19,793,328 bytes before.
One shared scope still reaches 37,930,224 bytes. A 128-unit preparation bound returns the peak below the baseline.
Standalone replays compare 128, 64, 32, and 16 preparation units, with and without compatible scalar SELECT fusion.
They preserve the exact 161-row candidate multiset. Neither family counts nor row parity establish CPU recovery.
Bottom-up samples and phase CPU identify additional lowering work and SQLite SELECT preparation as the costs.
The raw experiment directory retains every valid trial and the abandoned source snapshot.
The first `repack-fused` result uses an older executable after a failed build; it is invalid and excluded.

The next red test targets a narrower defect proven in the same capture.
A private `(fid, pre)` selector is materialized inside its CTE and then copied into its physical CTAS owner.
The physical table already supplies the required boundary.
The test retains both distinct selector owners, all 130 findings, and both result packets.
It requires the owner's final projection to be a pipeline, while shared intermediate CTEs retain their boundaries.
The frozen baseline fails on its own `AS MATERIALIZED` declaration.
Evidence: `.rerail/profiles/selector-family-recovery-20261024/owned-red`.
The unchanged 100-submission gate remains mandatory.

### Rejected target shape and two-rule regression: red

This phase starts from `66f6a522` and its frozen child-demand addon.
The complete small job submits 7,733 engine statements. The first frontier constructs 393 indexed `(fid, pre)` tables.
These tables mostly represent different logical selectors. Combining their semantic identities would be incorrect.
The target shares physical construction and storage among independent selectors, while preserving each producer's identity.

Each compatible ready group uses one SQL owner with `(producer, fid, pre)` rows and one nonunique index on those columns.
A selector reads its own producer tag through an inline SQL projection.
UNION ALL preserves duplicate rows and empty members. Tags preserve different selectors with overlapping rows.
The owner builds once and remains until its last consumer across all result packets.
Dependent selectors remain in later groups. Retained read/write hazards also constrain group placement.
Existing dependency analysis supplies these barriers; this change requires no new generic scheduler or native evaluator.
Other schemas retain their current physical representation in this slice.

The red test uses two rules, two independent kind selectors, 130 clauses, and two result packets.
It requires one executed table construction, one index construction, and one retirement while preserving all 130 findings.
Captures must identify the physical owner and its logical members.
Tests must also cover bags, empty members, typed keys, dependency chains, retained-write hazards, failure, retry, and wide groups.
Acceptance compares complete findings, execution counts, CPU, and SQLite memory against the frozen baseline.
The fixed 100-submission gate remains unchanged.

## Shared child demand: 2026-10-24

### Field-qualified dependency waves: green

The test from `2bfefe52` now preserves 130 findings across two rules and two result packets with one child-fill execution.
Different field options retain separate `syntax.child_fields.v1` bindings.
The transport unions `(fid, pre, field)` requests from existing selectors and deduplicates missing keys before decoding.
The existing `decoded_children` SQL table owns both child answers and completion.
A NULL child completes its requested key. The missing-coverage join tests `d.fid IS NULL`, rather than `d.child IS NULL`.
Publication retains the existing transaction and source-version invalidation.

Child and span lookups now both introduce dependency barriers.
A later lookup waits for selectors that depend on earlier child or span answers.
The specialized wave planner remains sufficient. This change adds no generic scheduler, execution switch, or native fact cache.
The existing `nera_child` scalar still performs each missing-key lookup. This slice does not change its traversal algorithm.

Two fresh complete-job captures each record **7,733 engine submissions**, **994 SQLite internal executions**, and **8,727 total executions**.
The baseline records 8,669 engine submissions. The decrease is **936 submissions, or 10.80%**.
Complete findings and connection coverage reconcile. The fixed 100-submission gate correctly still fails.
Candidate preparation decreases from 4,252 to 3,316 submissions. Cleanup remains at 2,216.
The first captured frontier decreases from 647 to 415 stages and from 246 to 11 child fills.
Span fills increase from two to five because the planner now represents their child dependencies explicitly.
Temporary selector tables remain at 393. Peak live relations decrease from 100 to 93.
The second frontier retains 401 temporary tables and decreases from 659 to 423 stages, with 11 child fills.
Its peak live relation count decreases from 100 to 91.

The first three alternating CPU pairs show a 3.08% median regression, from 1,711.444 to 1,764.221 ms.
Six follow-up pairs reverse that direction. All trials remain in the evidence; no trial is discarded.
Across all nine fresh-process pairs, median process CPU decreases from **1,739.994 to 1,708.247 ms**, or **1.82%**.
The before range is 1,628.437–1,776.261 ms. The after range is 1,610.341–1,790.968 ms.
These ranges overlap. This result does not establish a substantial CPU gain.
Median reported peak RSS is 337,952 KiB before and 338,928 KiB after, also with overlapping ranges.
Both archives use the same build settings, allocator shim, frozen observations, and four workers.
Tracing stays disabled. Own builds, tests, and diagnostic captures finish before each timing series.

A separate SQLite replay uses the first captured observation and unchanged 1 MiB main and 512 KiB temporary cache targets.
Both programs produce 161 candidate rows and the same 19,793,328-byte stepping heap peak.
Compilation peaks also remain at 3,397,936 bytes.
Stage/result VM steps decrease from 81,781 to 75,068; sorts remain at 1,815.
Cache misses decrease from 259 to 180, writes from 401 to 322, and spills from 282 to 207.
These replay counters explain SQL behavior. They do not measure total retained ownership or establish process memory recovery.

Validation passes **311 native-facing tests**, **48 standalone SQL tests**, and **120 original observations across 60 rules**.
The source observations include 60 positive and 60 clean cases, with no diagnostic differences or engine failures.
Tests cover overlapping demand, different fields, completed absence, empty demand, source replacement, rollback, retry, and 700 tagged requests.
The production edit test preserves findings `2 → 1 → 2` as a source changes and changes back.
All eleven fixed-budget regression tests and the exported producer-manifest check pass.
Native Clippy passes with and without profiling. Standalone Clippy, static checks, formatting, and patch checks pass.
Static checks first find 26 files in `test/engine/native/`, above its limit of 25.
The child-demand test moves into its `recovery/` subdirectory; its explicit CI and producer-gate references move with it.

The decision retains batched child transport because it reduces submissions and preserves shared SQL ownership.
The remaining candidate preparation and cleanup total **5,532 submissions**, about 72% of the engine count.
Repeated temporary selector construction, indexes, and cleanup remain the next measured target.
The universal node-property fill already executes once in these frontiers; scoped node demand and empty-relation simplification remain open.
WP3 and aggregate recovery remain incomplete. This slice does not establish full-corpus or repository acceptance.
Checked evidence: [shared child demand](../gate/data-layer/fixtures/host/data-layer-port/profiling/shared-child-demand-recovery-20261024.json).
Raw captures, commands, and all nine CPU pairs remain in `.rerail/profiles/child-demand-recovery-20261024/`.

### Different fields in one ready wave: red

The next phase starts from `a63782d7` and its stable 8,669-submission guard.
The first captured frontier contains 246 child-field fills, compared with two span fills and one covering node-property fill.
The new test uses two rules, distinct child fields, 130 clauses, and two result packets.
The baseline preserves all 130 findings and both field bindings but executes two child-fill statements where one is required.
Evidence: `.rerail/profiles/child-demand-recovery-20261024/child-red`.
The target carries field-qualified request rows through one SQL fill per ready dependency wave.
Field options remain separate semantic bindings. The existing SQL owner retains completion by `(fid, parent, field)`, including NULL child results.
The change adds no algorithm flag or native fact cache. The fixed 100-submission gate remains mandatory.

## Frontier determinism: 2026-10-24

### Independent admission and canonical composition: green

The permutation test from `bc3417cd` now passes on all four orders of the same 139 compiled plans.
Each order produces **1,503 stages and 65 broadened plans**.
Producer definitions, physical stage definitions, parameter layouts, per-plan admission, result-packet counts, and peak live relations match.
The audit compares hashes as diagnostic labels. Sharing itself uses full typed definition equality.

The compiler now lowers each plan under its own existing 256-node allowance.
It discards a failed fragment completely, then constructs the conservative fallback in fresh state.
Live fragments enter one producer program. Equivalent SQL templates share through their complete schemas, bound values, and dependency identities.
Canonical topological numbering determines names and placement before physical SQL lowering.
Result packets group canonical producers while retaining the caller's consumer IDs.
Allocation addresses remain local handle and cache keys. They no longer control admission or physical producer placement.
The legacy alias-compaction path and its extra uncompacted capture are removed. Historical captures remain readable.
This implementation compiles SQL. SQLite still executes the rules and owns the fact rows.

The final addon passes **309 native-facing tests**, **44 standalone SQL tests**, and **120 original observations across 60 rules**.
The original observations include 60 positive and 60 clean cases, with zero diagnostic differences or engine failures.
The standalone tests cover canonical aliases, typed parameters, bag multiplicity, domain separation, and recursion inside a SQL operation.
All eleven fixed-budget regression tests pass. The 100-submission limit remains mandatory.
Native Clippy passes with and without profiling. Standalone Clippy, static checks, formatting, and patch checks pass.
Static validation first finds a directory-size violation. Moving the audit beside the lowering helpers resolves it.
Production Clippy finds an unread graph field. The diagnostic audit now includes the logical producer graph in its equality checks.

Fresh complete-job captures consistently record **8,669 engine submissions**, **994 SQLite internal executions**, and **9,663 total executions**.
The previous checkpoint records 9,481 engine submissions. The decrease is **812 submissions, or 8.56%**.
Complete findings and all connection coverage reconcile. The unchanged 100-submission gate correctly still fails.
Candidate preparation decreases from 4,836 to 4,252 submissions. Cleanup decreases from 2,444 to 2,216.
The two captured frontier shapes have 647 and 659 stages, compared with 761 and 721 before this change.
Both now peak at 100 live relations and retain two shared span fills.

Three alternating fresh-process pairs compare the frozen `e237b5a0` addon with the selected implementation.
Median process CPU decreases from **2,318.369 ms to 1,617.488 ms**, or **30.23%**.
The before range is 2,297.111–2,511.919 ms. The after range is 1,610.712–1,683.899 ms.
All three selected runs use less CPU than all three baseline runs.
Reported median peak RSS is 360,800 KiB before and 343,728 KiB after. Those memory ranges overlap.
Both binaries use identical build settings, allocator shim, input observations, and four workers.
Detailed tracing remains disabled, and own builds, tests, and diagnostic runs finish before timing.
The host remains shared. This small workload does not establish repository-wide recovery.

A separate SQLite diagnostic replay uses the same first source observation and production cache budgets.
Peak live relations decrease from 114 to 100. The stepping heap peak decreases from 22,828,464 to 19,793,328 bytes.
These figures describe SQLite replay allocation, rather than process RSS or retained ownership.
The candidate row counts differ: 135 before and 161 after. Independent bounded admission can produce broader candidate supersets.
Final diagnostic parity passes. The full corpus and repository checks remain necessary before broad acceptance.

The remaining preparation and cleanup submissions total 6,468, about 75% of engine submissions in this case.
The next slice can now measure their repeated materialization against a reproducible frontier.
Node/child demand coalescing, selector ownership across frontiers, broad SQL consumers, whole-job publication, and session lifetimes remain open.
Checked evidence: [canonical frontier recovery](../gate/data-layer/fixtures/host/data-layer-port/profiling/canonical-frontier-recovery-20261024.json).
Raw captures, all timing trials, the red audit, and commands remain in `.rerail/profiles/frontier-recovery-20261024/`.

### Same-object plan permutations: red

The next phase starts from `e237b5a0`.
The new candidate-frontier audit calls the normal compiler on four explicit permutations of the same compiled plan objects.
It submits no candidate SQL and selects no alternate execution algorithm.
The state-tripwire policy contains 139 plans in this audit.
Forward order produces 1,609 stages and 54 broadened plans. Reverse order produces 1,532 stages and 64 broadened plans.
The test also finds changed per-plan generated counts, parameter layouts, and physical producer definitions.
This proves that earlier producer-cache warmth changes admission under the existing 256-node allowance.
Evidence: `.rerail/profiles/frontier-recovery-20261024/state-baseline-audit.json` and `permutation-red`.

The selected target bounds each plan independently before composing shared producers.
Complete typed definitions establish sharing. Canonical topological numbering establishes placement, independent of caller order and heap addresses.
The 256-node lowering allowance and the complete-job 100-submission gate remain fixed.
The audit tool runs with `node --import tsx scripts/profiling/frontier-audit.mjs POLICY.json OUTPUT.json`.

## Shared producer foundation: 2026-10-24

### Shared span dependency waves: green

The two-rule, two-packet test from `5a0bdf6a` now executes one physical span fill and preserves all 130 findings.
Live span selectors contribute keys to one SQL fill per necessary dependency wave.
The fill reads existing selector tables, deduplicates missing keys, and publishes into the existing `decoded_spans` owner.
It does not embed those selector definitions again or create another fact owner.
Later span-dependent predicates retain later waves. Consumer materialization keeps expensive source predicates after the demanded-key fence.
The controller remains specialized. Recursive SQL stays inside its declared operation.

Each valid span key produces one row. That row records the fact and completed coverage atomically under the existing transaction.
Empty selector demand contains no keys. General producers with empty output still need explicit completion records in later work.
The SQL helper tests cover overlap, repeated consumption, empty demand, distinct source IDs, invalidation, rollback, and retry.
Balanced compound SELECTs handle 700 demand sources within one submitted statement.
All **308 focused native tests**, **37 standalone SQL tests**, and **120 original positive/clean corpus observations** pass.
Native and standalone Clippy pass, including native builds with and without profiling. Static checks and formatting pass.

The first wave implementation repeats generated-SQL token scanning for wave analysis.
Its matched CPU median regresses from 2,234.975 ms to 2,472.817 ms, or 10.64%.
That implementation is replaced. Wave levels now share the existing reference-count pass, and stage sorting caches its keys.
The earlier trials remain recorded. The separate control groups cannot establish a causal timing estimate for that scan alone.

The selected version records **9,481 engine submissions**, **994 SQLite internal executions**, and **10,475 total executions** across 136 connections.
Complete findings and trace coverage pass. The unchanged 100-submission gate still fails.
Each normal captured frontier now has **two span fills**, compared with 22 in the catalog checkpoint.
However, the complete-job count increases from 9,113 to 9,481. There is no net submission improvement.
Candidate preparation submits 4,836 statements, cleanup submits 2,444, and relational evaluation submits 1,781.

The new provenance export identifies an existing source of unstable physical work.
The catalog capture first attributes result zero to `state-signatures-1`.
The initial wave capture first attributes it to `production-ir-construction` under the same policy digest and original observation.
That snapshot frontier has 1,333 stages, compared with 703 in the catalog capture.
The selected version has 761 stages there, while another frontier has 721 stages versus the catalog's 715.
The frontiers differ before span grouping. Raw allocation-address ordering in `Corpus::prepare_files` remains a concrete cause of changing input order.
The exact interaction with bounded lowering and producer reuse needs a fixed-plan permutation test.
This finding does not revive the earlier rejected cache-pruning claim or justify another unproved ordering heuristic.

Three alternating fresh-process pairs compare the catalog addon with the selected one-pass wave planner.
Median process CPU is **2,516.352 ms before** and **2,444.173 ms after**, a 2.87% decrease.
The ranges overlap: 2,290.043–2,657.838 ms before and 2,374.218–2,779.836 ms after.
Reported median peak RSS is 375,024 KiB before and 367,312 KiB after.
These small shared-host controls establish no aggregate speed or memory recovery.
Own builds, tests, and diagnostic captures finish before the timing controls. Detailed tracing stays disabled.

A separate frozen-frontier control holds the database, parameters, selectors, consumers, and scheduler fixed.
Its split variant replaces each broad fill with one fill per existing scope, without changing the other stage definitions.
The three-process median SQL CPU is **108.291 ms for 22 split fills** and **104.903 ms for two broad fills**.
Both variants emit 48 candidate rows and retain a peak of 97 live relations.
Their SQLite stepping heap peaks are effectively equal, about 20.98 MB. Their compilation heap peaks are about 3.62 MB.
The small timing ranges overlap. This diagnostic supports the batching shape without a large speedup claim.

Decision: keep one-pass batched dependency waves for spans, without an algorithm flag.
The next recovery slice must make frontier construction independent of allocation addresses and earlier producer-cache warmth.
It must prove this on fixed plan permutations before another aggregate optimization claim.
Other property fills, repeated selector construction, whole-job publication, session lifetimes, and complete repository acceptance remain open.
Checked evidence: [shared span waves](../gate/data-layer/fixtures/host/data-layer-port/profiling/shared-span-waves-20261024.json).
All raw controls and rejected trials remain in `.rerail/profiles/shared-producers-20261024/`.

### Shared span dependency waves: red

The next test uses two rules with 65 clauses each, so candidate emission requires two result packets.
Both rules request source spans through independent kind selectors.
The catalog checkpoint preserves all 130 findings but executes two span-fill statements where one is required.
Evidence: `.rerail/profiles/shared-producers-20261024/span-wave-red`.
The target groups live span demand by its registered owner and necessary dependency wave.
Selectors retain their existing physical tables. The broad fill reads those tables without embedding their definitions again.
Dependent span predicates retain later waves, and consumers retain the demanded-key fence before source predicates.

### Structural catalog and real exports: green

The public export test from `9db86048` now passes.
Trusted declarations name node properties, source spans, and child fields beside the production decoder.
The catalog compares complete ordered binding values. Child fields use typed integer options.
Different selectors and consumers share a binding. Different input generations, options, names, and sealed domains remain distinct.
Conflicting definitions report both origins and all differing fields.
The decoder uses registered binding IDs for same-selector reuse. This change does not coalesce separate physical fills.

Normal SQL captures now contain `producer-plan-*.json` manifests accepted by the existing declaration checker.
Only live decoder demands enter the export. Compaction preserves the combined provenance of merged demands.
Consumers retain rule ID, compiled clause ID, policy digest, result index, and selector name.
The API does not supply authored file paths or lines. The export does not invent those locations.
Actual database membership and source digests resolve the lowering-local input slots at capture time.
Templates retain no resolved generation across edits. JSON remains diagnostic metadata, while runtime fact rows remain in SQL.

The early storage experiment uses the production SQLite settings and the production relational authorizer.
One writer and two readers preserve publication barriers, pinned WAL snapshots, rollback, and independent private scratch.
The selected public physical owner admits ordinary reads and aggregates. Private-table reads, writes, and unrelated private-table aliases remain denied.

The protected-view alternative fails two checks.
SQLite reports a backing read without an accessor during its `count(*)` shortcut, so the existing authorizer rejects that aggregate.
A CTE can also impersonate an accessor label used by the existing generated-view exception.
Evidence: `storage-unit-labeled/stderr.log` and `storage-unit-projection/stderr.log`.
The selected physical-owner check passes in `storage-direct-owner`.
The legacy exception remains unchanged. WP5 must not extend it or treat it as a general isolation mechanism.
Its broader correction remains explicit follow-up work. No Linear ticket was created.

All **307 focused native tests**, **33 standalone SQL tests**, **11 fixed-budget Node tests**, and **25 discovery/checker Python tests** pass.
The final addon also passes **120 original observations across 60 rules**, including 60 positive and 60 clean observations.
It reports zero false positives, false negatives, engine failures, or execution-path differences.
Native Clippy passes with and without profiling. Standalone Clippy, `npm run check`, formatting, and patch checks pass.
Two initial builds expose implementation errors, which are corrected before the final addon and validation runs.

The complete small guard records **9,113 engine submissions**, **994 SQLite internal executions**, and **10,107 total executions** across 136 connections.
Findings complete and all execution coverage reconciles. The fixed 100-submission gate correctly returns nonzero.
This count is 18 higher than the previous capture. Allocation-dependent physical ordering remains, and no statement reduction is claimed.
The eight real manifests pass declaration checks without errors or clashes. They contain 56 resolved binding groups.
One span binding has 117 consumer/selector references. Each normal frontier still executes 22 separate span-fill statements.
That gap identifies the next physical coalescing requirement.

Three alternating fresh-process CPU pairs compare the frozen `e847fd7b` addon with the catalog implementation.
Median process CPU is **2,390.952 ms before** and **2,299.499 ms after**, a 3.82% decrease on this small workload.
The respective ranges are 2,369.510–2,447.855 ms and 2,223.825–2,337.324 ms.
Reported median peak RSS is 412,064 KiB before and 334,976 KiB after.
The binaries use identical build settings, allocator shim, workload, and four workers. Detailed tracing stays disabled.
Own builds, tests, and diagnostic captures finish before timing. The host remains shared.
These observations do not establish aggregate recovery or attribute a speed or memory gain to catalog registration.

WP1 has its first catalog/export checkpoint. Private authored-stage exports and authored file/line provenance remain outside this slice.
WP2 must union ready selector demand by binding and dependency wave, then publish rows and completion atomically.
It must remove separate fills while preserving admission, empty results, edits, cancellation, and packet lifetimes.
Whole-job publication, session lifetimes, and full repository acceptance remain open.
Checked evidence: [shared producer foundation](../gate/data-layer/fixtures/host/data-layer-port/profiling/shared-producer-foundation-20261024.json).
Raw runs and commands remain in `.rerail/profiles/shared-producers-20261024/`.

### Real compiler export: red

The next phase starts from `e847fd7b` in the same worktree.
The retained state capture contains 22 separate span-fill statements in one candidate frontier.
Retained decoded rows do not prevent repeated demand preparation around those rows.
WP1 establishes trusted contracts and complete typed binding identity before changing physical sharing.
Its first public test compiles two actual rule consumers and requires a source-pinned producer manifest accepted by the existing declaration checker.
The selected addon preserves both findings but exports no producer manifest, so the test fails.
Evidence: `.rerail/profiles/shared-producers-20261024/export-red`.
This step does not claim a runtime count or CPU improvement.

## Recovery TDD: 2026-10-23

### Compiler schema reuse across documents: green

The process-owned schema cache passes the stronger test from `a2631fb0`.
Two documents with twelve repeated pipelines each execute three inference statements in total: two CTAS statements and one deferred-column ALTER.
Deferred-expression validation executes no CREATE or DROP.
Eight concurrent schema-inference requests perform one inference while compatible metadata remains resident.

SQLite supplies the schema, including expression affinity. Every clause retains SQL, authorizer, and deferred-expression validation.
The key contains exact SQL, complete ordered schemas of read relations, and deferred column names.
Immutable shared column descriptors avoid repeated deep copies. No connection, prepared statement, runtime row, or source generation enters the cache.
Retention has fixed limits of 128 entries and 4 MiB of charged payload. Allocator overhead is additional.
Eviction permits safe re-inference. Runtime materialization remains unchanged.

The final guard records **9,095 engine submissions**, **994 SQLite internal executions**, and **10,089 total executions** across 136 connections.
Complete findings and execution coverage pass. The fixed 100-submission gate correctly fails after execution finishes.
Compiler stage CREATEs fall from 194 to 68. Connection-setup submissions fall from 498 to 372.
Candidate preparation still submits 4,578 statements, cleanup submits 2,316, and relational evaluation submits 1,781.
Allocation-dependent physical ordering remains. The complete count difference cannot be attributed solely to compiler schema reuse.

Three alternating fresh-process CPU pairs compare `ac3aea34` with the final shared-metadata implementation.
Median CPU rises from **2,479.402 ms to 2,595.351 ms**, a **4.68% regression**.
The respective ranges are 2,374.845–2,576.086 ms and 2,565.284–2,627.324 ms.
Reported median peak RSS falls from 393,040 KiB to 357,152 KiB, or about 384 MiB to 349 MiB.
The host is shared. These measurements do not establish a reliable memory gain or explain the CPU regression.
Both binaries use the same workload, build settings, allocator shim, and four workers. Detailed tracing stays disabled.

The intermediate process cache used owned column vectors and recorded 9,113 submissions.
Its separate CPU comparison also regressed: 2,316.678 ms to 2,369.203 ms, or 2.27%.
That result motivated shared descriptors and prepare-only binding without recopying metadata.
The final comparison still provides no CPU improvement. Keep the schema reuse for its execution shape, with CPU acceptance unresolved.
Do not compare the two control groups as a causal measure of descriptor sharing.

The final addon passes **306 focused native tests** across eleven files and **120 original observations across all 60 rules**.
The corpus selection includes 60 positive and 60 clean observations, with zero false positives, false negatives, failures, or path differences.
All 28 standalone SQL tests pass, including schema affinity, downstream CTAS, cache bounds, and concurrent inference checks.
Native and standalone Clippy, `npm run check`, Rust formatting, and profiling-tool tests pass.
The first native Clippy attempt reaches the command deadline after package-cache lock contention. The offline retry passes.
This scope does not replace the full 3,210-observation corpus or whole-repository acceptance.

R3 remains open for whole-job publication and session lifetimes. R4 must share physical runtime producers and coalesce admitted source-span demand.
Those runtime statements dominate the remaining count. Compiler metadata sharing alone cannot meet the allowance or establish CPU recovery.
Checked evidence: [stage schema recovery](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-stage-schema-recovery-20261023.json).
Raw captures, commands, and all timing trials remain in `.rerail/profiles/sql-stage-recovery-20261023/`.

### Compiler schema reuse across documents: red

The complete cold job compiles more than one document. Its execution allowance never resets at a document boundary.
The stronger test permits one prototype inference across both documents, rather than one inference per document.
The document-owned implementation fails on two seed CTAS executions where one is required.
Evidence: `.rerail/profiles/sql-stage-recovery-20261023/job-scope-red`.
The target shares only immutable schema metadata across compilations in the process, with the same fixed retention bounds.
Complete typed keys and per-clause validation remain mandatory. SQLite connections and runtime rows never enter this cache.
A cache lock covers prototype inference so simultaneous compilations cannot duplicate the same admitted inference.

### Repeated compiler stage schemas: green

The document-owned cache passes all 306 focused native tests in `stage-green` (34.888 seconds).
The two-document test executes six statements instead of repeating inference for all 24 clauses.
SQLite infers each stage once per document. Deferred-expression validation executes no DDL.
Six production schema tests pass, including affinity, changed input types, column order, and metadata bounds.
The cache preserves each clause's SQL and authorizer validation. Invalid deferred expressions still fail after a cache hit.
Native Clippy, standalone Clippy, and the static repository checks pass.

The small guard completes findings and reconciles all coverage, but fails the fixed gate at 9,293 submissions.
Compiler stage CREATEs fall from 194 to 134. Additional input-schema differences prevent unsafe sharing of identical SQL strings.
Evidence: `.rerail/profiles/sql-stage-recovery-20261023/stage-acceptance/capture`.
These counts do not establish a CPU gain. Matched CPU controls remain pending.
The next slice extends immutable schema reuse across document compilations in the same cold process.
That scope matches the complete-job budget and requires the same typed keys and bounded retention.

### Repeated compiler stage schemas: red

The next R3 test compiles two documents, each with twelve clauses that repeat the same two-stage pipeline.
The first stage adds a deferred column. Every clause retains ordinary SQLite validation.
The target permits one inference of each stage schema per document and no deferred-validation DDL.
The selected prior addon fails: it executes twelve seed CTAS statements per document instead of one.
Evidence: `.rerail/profiles/sql-stage-recovery-20261023/stage-red`.

The 194 compiler stage executions in the prior guard contain only 53 distinct SQL strings.
SQLite rejects CTAS through its virtual-table declaration API, so native affinity inference remains necessary on a cache miss.
The target caches the schema that SQLite infers, keyed by query, complete schemas of read relations, and deferred column names.
Every cache hit still prepares the authored query under the existing authorizer.
The cache belongs to one document compilation and has fixed entry and byte bounds.
It contains schema metadata only. Runtime rows and materialized producers remain outside this cache.


### Selected compiler recovery checkpoint

The selected implementation removes compiler input/syntax DDL and runtime PRAGMAs from compiler initialization.
It retains SQLite validation, the existing stage CTAS boundaries, and executable empty runtime syntax tables.
The empty-runtime regression now passes its stronger failure assertion.
The original physical candidate order remains. Both ordering experiments and cache pruning are absent from production.

The complete small capture records **9,695 engine submissions**, **994 SQLite internal executions**, and **10,689 total executions**.
All native counters, raw events, and session receipts reconcile. Findings complete with no failures or parity differences.
The original 100-submission gate correctly returns nonzero after the complete replay.
Connection setup falls from 904 to 498 submissions: 406 fewer setup statements.
The captured previous checkpoint has 10,443 engine submissions. Its physical lowering order depends on allocation addresses.
The remaining count difference cannot establish a stable compiler-only saving. R4 must remove that incidental ordering.

Three alternating fresh-process pairs compare the previous checkpoint with the selected implementation.
Median CPU falls from **2,374.190 ms to 2,266.036 ms**, a **4.56%** reduction for this small workload.
The respective ranges are 2,216.678–2,434.455 ms and 2,168.529–2,326.142 ms.
The ranges overlap. These shared-host measurements do not establish a statistically reliable gain or repository-wide recovery.
Reported median peak RSS is 357,360 KiB before and 346,496 KiB after. Aggregate memory acceptance remains open.
Both binaries use the same opt1 engine, dependency optimization, profiling feature, system allocator shim, and four workers.
Detailed SQL and span capture stay disabled for every CPU control.

The selected addon passes **304 native-facing tests** across eleven files in `selected-functional` (12.832 seconds).
One combined replay passes **120 original observations across 60 rules**, with 60 positive and 60 clean observations.
It reports zero false positives, false negatives, execution failures, or path differences (`selected-parity`, 13.490 seconds).
This selection does not replace the complete 3,210-observation corpus or the full repository workload.
The 24 standalone SQL tests, native and standalone Clippy checks, formatting, and `npm run check` pass.
The profiling tools retain their passing 26 Node tests and 75 Python tests.

The selected capture still contains 194 compiler stage CTAS executions, 4,924 candidate submissions, and 2,444 cleanup submissions.
Compiler stage boundaries alone exceed 100 before the complete runtime workload participates.
The next recovery slices need compiler stage schemas, whole-job publication, named shared physical producers, and broad SQL consumers.
No unsupported engine-feature ticket follows from the rejected experiments.

Checked evidence: [compiler recovery checkpoint](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-recovery-compiler-20261023.json).
Detailed evidence stays in `.rerail/profiles/sql-recovery-compiler-20261023/`.

### Work-estimate physical order: rejected

The bounded work-estimate candidate passes 304 native tests and all 120 selected source observations.
Its complete-job count remains 14,699 engine submissions, with 994 SQLite internal executions.
Its median CPU is 2,461.281 ms versus the matched checkpoint median of 2,381.320 ms: a 3.36% regression.
Evidence: `work-order-functional`, `work-order-parity`, `work-order-acceptance`, and `cpu-{baseline,work-order}-{0,1,2}`.
This candidate does not justify a new ordering heuristic.

Both ordering replacements are removed. The production corpus retains its earlier physical order.
The two first-seen-order unit tests describe a rejected implementation choice, not a required rule semantic.
The compiler adapter and connection changes remain for a separate matched control.
Allocation-dependent broadening remains an observed risk for R4. Named shared producer identities must replace this incidental ordering during that work.
The cache-pruning candidate also remains removed. No alternate algorithm or new configuration flag remains.

### Authored physical order: rejected CPU control

Three alternating fresh-process controls compare the previous checkpoint with the compiler adapters and authored physical order.
The previous checkpoint median is 2,368.800 ms CPU. Its range is 2,267.492–2,707.701 ms.
The candidate median is 2,475.716 ms CPU. Its range is 2,456.084–2,520.751 ms.
All six small replays preserve findings. Detailed SQL and span capture stay disabled.
The candidate regresses median CPU by 4.51% and increases engine submissions from 10,443 to 14,699.
The earlier checkpoint count comes from its preserved capture, not a new claim of count stability under address sorting.
These measurements reject authored order as the physical lowering strategy.
Evidence: `cpu-before-{0,1,2}` and `cpu-after-{0,1,2}` in the compiler recovery directory.

The next candidate retains authored consumer identities but lowers smaller producers first.
A bounded structural work estimate orders lowering and never uses allocation addresses.
Consumer result arms remain in authored order. SQLite and the authoritative matcher retain execution semantics.
The candidate has one production path and needs the same count, CPU, and corpus checks before selection.

### Rollback control correction and empty runtime schemas: red

The seven-to-eight materialization assertion used an incomplete control.
The broadened selector legitimately consumes a kind relation that the original control never uses.
The corrected control explicitly consumes that relation in both variants.
It passes on the unchanged earlier addon, including consecutive failures and reused parameter values.
Evidence: `rollback-control-correction` (9.512 seconds).
The uncommitted cache-pruning implementation changes neither complete-job statement counts nor this valid control. It is removed.
The previous entry records a rejected interpretation, not proof of duplicated materialization.

The combined 120-observation replay catches six failures from empty runtime syntax sources.
The existing syntax test checks only empty findings and misses the execution failure.
Its stronger assertion now rejects `compiler schemas cannot be executed` in `empty-schema-red` (5.020 seconds).
The runtime empty-source loader calls the same schema helper as compilation.
The target gives compiler adapters and executable empty runtime tables separate entry points over shared declarations.

### Lowering rollback loses shared work: red

Stable authored order alone records 14,699 engine submissions, with complete findings and reconciled coverage.
Evidence: `.rerail/profiles/sql-recovery-compiler-20261023/ordered-acceptance/capture`.
This result rejects an aggregate improvement claim from ordering alone.

The next public-API test places an over-budget selector between two identical selectors.
All three rules retain their findings. The complex selector broadens exactly once, as intended.
However, its rollback clears earlier successful reuse caches and increases physical materializations from seven to eight.
Evidence: `rollback-red-2` (5.710 seconds). The earlier `rollback-red` invocation contains a test-program syntax error, not behavioral evidence.
The required fix retains cache entries whose dependencies survive the checkpoint and removes entries from the failed branch.
The implementation must avoid copying the growing complete cache before every plan.

### Compiler connection setup: green

Compiler connections now use the shared limits and SQL function registration without runtime PRAGMAs.
Runtime connections retain both storage PRAGMAs.
The public twelve-clause compiler test records zero executed SQL and passes.
All 303 focused native-facing tests pass in `setup-green` (20.326 seconds).
The standalone host Clippy check passes in `host-clippy-final`.
The corpus helper moves into `sql/corpus/` to retain the repository limit of 25 files per code directory.
This source organization change does not select another execution path.

### Compiler connection setup: red

The stronger public compiler test requires zero executed SQL when a clause declares no stages.
The adapter addon fails on 24 runtime PRAGMAs across twelve prepare-only compiler sessions.
Evidence: `.rerail/profiles/sql-recovery-compiler-20261023/setup-red`.
The target separates compiler connection initialization from runtime storage configuration.
Both purposes retain the same limits, function registration, and authorizer. Runtime PRAGMAs remain mandatory for runtime sessions.

### Candidate frontier allocation order: green

The production helper now retains first-seen consumer order and removes duplicate Arc identities.
Only the separate demand-set key sorts addresses. Equal frontiers still share one batch.
All 26 standalone SQL tests pass in `frontier-green`, including both former failures and three schema-adapter tests.
The final addon capture and CPU controls remain necessary before an aggregate improvement claim.

### Candidate frontier allocation order: red

Two deterministic tests force allocation order to oppose authored consumer order.
They exercise the production frontier helper, which retains the previous address sort unchanged.
Both tests fail in `.rerail/profiles/sql-recovery-compiler-20261023/frontier-red`.
A demand set needs an unordered identity for coalescing, but execution must preserve the first occurrence of each consumer.
Pointer equality remains suitable for shared-plan identity. Pointer ordering must not select a physical lowering order.

### Compiler schema recovery: adapter verification

The public schema test now passes. Registration executes no relation DDL.
Three direct production-adapter tests pass, including SQLite CTAS affinity equivalence and expression non-execution.
The original SQLite authorizer rejects metadata reads, writes, unknown columns, unknown tables, parameters, and unsafe functions.
Case-insensitive input collisions and reserved SQLite names remain invalid, including unused inputs.
Native Clippy and `npm run check` pass.
The focused native-facing suites pass 303 tests across eleven files (`schema-green` and `runtime-green`).

The complete small replay preserves findings but records 11,641 engine submissions and 994 SQLite internal executions.
Evidence: `.rerail/profiles/sql-recovery-compiler-20261023/schema-acceptance/capture`.
Connection setup falls by 278 statements. Candidate preparation increases by 1,088 and cleanup increases by 388.
This capture rejects a net performance claim for the adapter slice alone.
The investigation finds that `Corpus::prepare_files` sorts execution plans by Arc allocation address.
Changing compiler allocation can therefore change lowering order, reuse, and per-plan broadening.
The next TDD ladder separates unordered demand identity from authored execution order before another CPU comparison.

### Compiler schema recovery: red

The next R3 test compiles twelve independent relational clauses through the public API.
Each clause declares an input, syntax nodes, edges, and ownership.
The required shape has no executed input or syntax DDL. SQLite still validates every authored query.
The prior addon executes 48 CREATE TABLE statements and fails the new assertion.
Evidence: `.rerail/profiles/sql-recovery-compiler-20261023/schema-red` (5.593 seconds).
The mandatory producer-shape command includes this test.

The target uses eponymous-only SQLite modules with the existing column declarations.
Registration establishes opaque schema boundaries without executing CREATE statements or expanding SQL views.
Stage CTAS validation remains unchanged in this slice. It preserves SQLite affinity inference and deferred-column validation.
Runtime materialization and the fixed 100-statement allowance remain unchanged.


The first guard test exposes a real acceptance bypass.
A successful profiling child without an execution report makes the CI launcher return zero.
The new test runs the actual launcher against a temporary controller that omits or bypasses enforcement.
It also covers a forged passing summary with 101 actual executions in the full report.

The red invocation is `.rerail/profiles/sql-recovery-tdd-20261023/guard-red`.
It fails because the launcher returns zero for the missing-report case, where failure is required.
This is a behavioral failure, before the launcher implementation changes.

Red commit: `640b74a2`. The launcher now requires the complete report and independently recomputes acceptance.
It rejects missing evidence and forged passing summaries. The allowance remains fixed at 100.
The green invocation is `.rerail/profiles/sql-recovery-tdd-20261023/guard-green`.
All 22 Node profiling tests pass, including the 100/101 boundary and cross-worker accounting tests.
The real workload remains a budget failure at 24,159 executions; this fix does not claim runtime recovery.

The next red test requires an exact execution inventory before optimization.
It places identical SQL in compiler, publication, candidate, relational, and unknown contexts across three workers.
Enclosing operator CPU must not inflate SQL counts or SQL CPU.
`.rerail/profiles/sql-recovery-tdd-20261023/inventory-red` fails because the report has no execution inventory.

The green inventory counts each raw SQL event before query-text aggregation.
It retains session, worker, producer, binding, generation, file domain, consumer, stage, SQL CPU, and VM steps.
Missing identities remain null. Unknown phases remain explicit. Enclosing native spans do not count twice.
All 23 Node profiling tests pass in `.rerail/profiles/sql-recovery-tdd-20261023/inventory-green`.
The native library Clippy check passes with profiling enabled in `inventory-clippy` (26.394 seconds).

The original capture reconciles to 24,159 statements in `baseline-inventory.json`.
Its phases contain 464 setup, 1,232 catalog publication, 3,339 relational evaluation, and 19,124 unknown executions.
The older index context remains active during later candidate queries, so that attribution cannot be trusted.
New captures set explicit compiler, publication, structural-preparation, and candidate-frontier origins.
Compilation still validates through SQLite. This step changes observation, not the rule execution algorithm.
The next runtime step removes eager preparation for excluded local rule/file pairs.

The R2 red tests exercise the real public workspace API with the previous profiling addon.
Adding a path-excluded rule increases the prepared template from one plan to two, although findings remain unchanged.
Twenty files still share one batch, but both eligible and excluded plans enter that batch.
`.rerail/profiles/sql-recovery-tdd-20261023/admission-red` records both behavioral failures (6.146 seconds).
The mandatory producer-shape command now includes these preparation-admission checks.

A further red case uses twenty files split between two disjoint rule scopes.
The old path runs both plans over all twenty files in one unrestricted frontier.
The required shape is two batches, each with one admitted plan and ten files.
`.rerail/profiles/sql-recovery-tdd-20261023/domains-red` records this failure and the earlier two failures.

The R2 local-preparation implementation passes all three green public-API tests in `admission-green` (9.162 seconds).
Path, text, syntax, and language exclusions leave only the admitted plan.
Twenty compatible files share one SQL batch. Two disjoint scopes produce two ten-file batches with one plan each.
The preparation and final matching paths share syntax and text admission.
Source generation remains part of the corpus file identity; this change does not merge generations.
The candidate corpus restricts both structural preparation and selection to the admitted file set.
The existing production SQL adapter remains authoritative for selection.

Eight native-facing test files pass 295 tests in `admission-functional` (11.672 seconds).
They cover relational evidence, syntax, deferred stages, completed reachability, broad producer waves, query parity, and matcher/runtime behavior.
The native profiling library Clippy check passes (28.411 seconds), and all profiling tool checks pass in `tool-checks` (16.029 seconds).
The optimized addon build finishes under the existing build approval (2 minutes 24 seconds).
Snapshot preparation, relational re-preparation, and conditional activation still need admission changes; R2 remains open.

The fresh admission acceptance run completes parity but exposes an inventory storage error.
Repeating full SQL text in every inventory entry pushes report serialization past Node's string-size limit.
The engine target succeeds; the outer gate correctly rejects the missing complete report.
`admission-acceptance/capture` retains the raw streams for offline repair, without rerunning the workload.
The storage regression test requires inventory entries to reference the shared query catalogue.
`inventory-storage-red` fails on duplicated SQL text before the reporter fix.

The reporter repair removes duplicated inventory SQL and the extra top-level copies of query plans and bytecode.
The query catalogue retains every observed plan. Compact JSON avoids whitespace inflation in large opcode arrays.
All 23 Node profiling tests pass in `catalog-storage-green` (2.419 seconds).
Offline regeneration completes in `admission-report-repair-2` (6.321 seconds).
Its exit status remains one because the actual workload exceeds the unchanged allowance.

The repaired complete capture contains 22,181 actual executions, down from 24,159 (1,978 fewer; 8.19%).
Coverage reconciles with no errors, and both original observations pass snapshot/workspace parity.
The inventory attributes 936 setup, 1,232 catalog publication, 272 file publication, 16,402 candidate preparation, and 3,339 relational executions.
No execution has an unknown phase. Undeclared producer bindings and generations remain explicit null identities.
This is a count comparison, not an unprofiled CPU improvement claim.
Snapshot and global relational preparation still account for eager work that the local change cannot remove.

The next R2 red tests cover snapshot preparation and the later relational preparation pass.
The first still prepares a text-excluded clause. The second reactivates an excluded local rule and executes another candidate batch.
`remaining-admission-red` records both behavioral failures against the local-admission addon (10.970 seconds).
These failures explain why local admission alone removes only part of the eager frontier.

The R3 publication red test counts actual SQL executions for four files with 200 identifier values each.
It observes 275 kind inserts, 32 field inserts, 28 kind-frequency inserts, 28 file-kind inserts, 808 value-posting inserts, and 28 coverage inserts.
The required shape uses one language-catalog batch per table and one publication batch per file and table.
Findings remain complete: 800 expected identifier violations and no failures.
`publication-red` records the statement-count failure against the local-admission addon.
This first publication slice does not satisfy the whole-job boundary required by R3.

The snapshot and relational admission tests now pass, including the zero-cache-miss check.
An omitted text-excluded plan also bypasses final matching, so it cannot become a fallback tree walk.
Relational preparation no longer expands every workspace rule over the complete corpus.
Restored missing sources retain publication and admitted demand for their active relational rules.
The old unrestricted corpus preparation helper is test-only.
The combined `admission-publication-green` run passes 300 tests and exposes one existing profiling-stream interleaving failure in the producer-shape test.
The behavioral admission checks all pass; the interleaved JSON failure requires a separate instrumentation fix.
Conditional cross-file activation still needs the staged producer work. R2 remains open for that part.

The publication green test reduces those same six insert families from 1,199 executions to 18.
The 800 findings remain complete. Both language catalogs use one statement each.
Each file publishes one batch for frequency rows, kind postings, value postings, and coverage rows.
Overflow records use the same bounded typed batching helper.
Bindings stay below SQLite's variable limit and a 4,096-value cap. Empty batches execute no SQL.
The helper transfers owned posting blobs without another copy where the caller already owns them.
SQLite retains the same tables, FTS layout, vocabulary triggers, and rollback ownership.
The production helper's isolated Rust test verifies type preservation, forced small batches, empty inputs, duplicate-key failure, and transaction rollback.
`publication-unit` passes in 27.694 seconds. The profiling native library Clippy check passes in `recovery-clippy` (40.529 seconds).
This is a per-publication improvement. Whole-job publication across ingestion packets remains pending.

The FTS audit uses the exact production declaration and one bulk `INSERT ... SELECT`.
One, ten, and 101 documents execute nine, 18, and 109 SQLite statements respectively.
The internal document-size writes occur once per document. The production tracer test records each write.
`fts-budget-audit` passes in 5.490 seconds and retains the exact results.
The user then fixes the acceptance boundary at **100 engine-submitted statements**, with SQLite internal executions reported separately.
The next tracer TDD step must distinguish those origins mechanically; historical traces lack the necessary callback annotation.
The numeric limit stays 100. Engine DDL, publication, helper calls, transactions, cached executions, and cleanup all remain included.

SQLite source confirms cached internal document-size statements and transaction-level posting-list buffering.
The contentless-delete layout rejects `columnsize=0`; suspending user triggers does not suspend FTS5's update method.
The next FTS experiment compares batched publication with transaction boundaries across packets and the complete job.

The user confirms the limit applies to engine-submitted statements and requires separate internal counts.
The new red Node cases accept 100 submissions plus 500 observed SQLite internal executions, reject submission 101, and reject missing origin evidence.
The native red case requires the real FTS document helpers to carry SQLite-internal attribution.
`origin-budget-red` and `origin-tracer-red` both fail before the tracer and gate changes.
Origin must come from SQLite's trace callback decoration, not table names or the number of open cursors.

The origin green tests pass: 25 Node profiling cases and eight production-tracer tests.
`origin-budget-green` completes in 2.698 seconds; `origin-tracer-green` completes in 4.236 seconds.
SQLite's bundled trace callback marks nested VDBE execution with an extra comment prefix.
The tracer compares that callback text against the canonical statement, so authored leading comments remain engine submissions.
A second host query while another result cursor is open remains an engine submission.
Native counters, raw events, query aggregates, and per-origin session totals must all reconcile.
Unknown or missing origins fail acceptance. Older captures cannot prove the newly clarified boundary.
The fixed 100 limit covers every engine submission across all connections and workers.
A complete capture with 100 submissions plus 500 internal executions passes; submission 101 fails.

The profiling-stream failure has a concrete cause: one trace record reaches the output writer as 38 formatting fragments.
The new writer test requires one preformatted record per write and fails before the implementation changes.
`trace-output-red` records that behavioral failure (5.536 seconds).
The same production trace source compiles directly in the small SQL host; this avoids another engine test build for the unit ladder.

The trace writer green test reduces one record from 38 formatting writes to one preformatted write.
All 21 standalone SQL-host tests pass in `trace-output-green` (3.229 seconds).
The native profiling library Clippy check passes in `final-clippy` (25.935 seconds).
The producer-shape retry before this fix passes both tests, confirming the earlier malformed stream is intermittent rather than a deterministic rule mismatch.
The final addon build includes both the origin annotation and the trace-output repair.

The optimized FTS experiment now has three rotated trials, production storage settings, and the exact production positional declaration.
It stages 1,024 files with 256 tokens each, then measures only index publication and commit CPU.
Packet transactions use 39.275 ms median CPU. Packet inserts within one transaction use 23.189 ms.
One complete bulk insert uses 11.545 ms, 70.6% below packet transactions and 50.2% below packet statements in one transaction.
The packet forms leave five segments; the complete bulk statement leaves one.
All variants preserve positional results, deletion, and replacement.
The earlier development-build medians were 64.771, 50.572, and 26.594 ms; they are separate controls.
The [optimized evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/fts-bulk-publication-20261023.json) retains every trial and the schema/binary identities.
This result supports whole-frontier `INSERT ... SELECT`, rather than suspending user triggers or changing the FTS layout.
Whole-job integration remains pending; the isolated result does not predict aggregate linter CPU.

The final inventory check separates stage release from construction.
A DROP under a candidate producer currently inherits the preparation phase, hiding cleanup in that total.
`cleanup-inventory-red` fails on that classification while preserving the exact execution total.

Cleanup now retains its producer identity but has a separate execution phase.
All 25 Node profiling tests pass in `cleanup-inventory-green` (2.214 seconds).
Offline regeneration preserves the 10,443 engine submissions and 994 SQLite internal executions.
The rebuilt inventory contains 5,176 candidate construction, 2,534 cleanup, 1,781 relational evaluation, 904 setup, and 48 publication submissions.
`final-inventory-rebuild` exits one solely because the unchanged 100-submission allowance fails.

## Recovery checkpoint: 2026-10-23

The [checkpoint evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-recovery-checkpoint-20261023.json) records the final measured addon and exact controls.
All original positive and clean state-tripwire observations pass snapshot and workspace parity.
Every connection receipt and origin counter reconciles. No SQL execution has an unknown origin or phase.
The complete job submits **10,443 engine statements**, plus **994 SQLite internal statements**.
The 100-submission gate correctly returns nonzero after preserving all findings.
Execution 101 still creates `syntax_21` during compiler schema validation.
The gate is mandatory in CI, independent of the smaller producer-shape assertions.

| Small state-tripwire workload | Frozen SQL checkpoint | Recovery checkpoint |
| --- | ---: | ---: |
| Total SQLite executions | 24,159 | 11,437 |
| Median complete replay CPU, three fresh processes | 3,441.002 ms | 2,567.392 ms |
| CPU range | 3,379.856–3,462.062 ms | 2,489.278–2,581.697 ms |
| Reported process maxRSS, median raw KiB | 476,144 | 370,304 |

Total executions decrease 52.66%. Complete replay CPU decreases 25.39% on this small case.
The controls include policy compilation, both snapshot evaluations, and both workspace evaluations.
Detailed SQL and span tracing remain off. Both addons retain aggregate profiling timers and the same optimization/allocator/worker settings.
The shared host has competing external workloads. The raw ranges remain available.
These results do not replace the whole-repository baseline or establish large-corpus memory recovery.
The pre-SQL Rust reference rejects current reachability root column names and is excluded from this comparison.

The final native-facing run passes all 301 focused tests in `final-functional` (23.504 seconds).
It includes admission, publication, broad producer waves, SQL parity, runtime edits, and matcher behavior.
Six additional bounded jobs pass 120 unchanged native source observations across all 60 ports: one positive and one clean per rule.
All false-positive, false-negative, failure, and snapshot/workspace difference counts are zero.
This is selected source coverage, not a rerun of all 3,210 original observations.
The repository typecheck and all structure/boundary/path checks pass in `final-static` (55.349 seconds).
All profiling tools pass 25 Node tests and 75 Python tests in `final-tools`.
Eight production-tracer tests and 21 standalone SQL-host tests pass separately.
The standalone host passes Clippy across all targets in `lab-clippy-2` (3.509 seconds).
Native and standalone formatting checks pass.

R2 still needs conditional cross-file activation.
R3 still needs a complete-job admission/publication boundary and compiler schema reuse.
R4–R7 remain open: named shared physical producers, bounded coalesced consumers, state-rule failure/memory correction, and full aggregate acceptance.
The remaining 5,176 construction and 2,534 cleanup submissions motivate physical ownership across rules and packets.
No acceptance requirement is marked complete because the numeric gate still fails.

The last CI regression test proves the real workload remains mandatory and checks the changed-file triggers.
It exposes missing triggers for the new native admission/publication guard files.
`ci-budget-red` records that failure. The numeric allowance and ordinary launcher tests already pass.
A Python summary test also exposes the old total-count label after the origin-boundary clarification.
`budget-summary-red` records that failure. Historical totals must remain labeled as legacy when origin evidence is absent.

The CI and outer-report green run passes all 26 Node guard/report tests and 75 Python profiling tests.
`ci-and-summary-green` includes the mandatory-step checks, workload script identity, and regression-file triggers.
Both native admission/publication guard files now trigger the profiling addon and mandatory gates.
The Python summary separates engine submissions, internal executions, and total executions.
Historical counters without origin evidence remain explicitly labeled legacy and do not become current acceptance proof.

## SQL execution limit restored: 2026-10-23

The source branch defines a fixed 100-execution job acceptance limit in commit `c109a915`.
The earlier profiler import omitted its gate and started/completed coverage counters.
The existing producer-shape gate measures selected native passes and never enforced that SQL limit.

The restored gate counts the complete cold process across workers and connections.
It includes compilation setup, schema DDL, publication, transactions, matching, and cleanup.
Each execution inside `execute_batch` counts separately. Cached statement executions also count separately.
Diagnostic EXPLAIN and enclosing native timing spans do not count.
The gate reports failure after complete evaluation. It never truncates findings at execution 101.

Connection-open and connection-close receipts detect missing whole sessions.
Native started/completed counters detect incomplete statement streams.
Missing coverage, incomplete operations, or failed parity prevent acceptance, including captures with small counts.
Ordinary SQL capture and report commands preserve evidence and return nonzero when the gate fails.
A mandatory CI step uses original positive and clean state-tripwire observations.

The new [evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-execution-budget-20261023.json) records a complete real acceptance attempt.
Two original observations run through both snapshot and workspace paths in one cold process.
Both observations pass parity, with one positive, one clean, and no engine or execution-path failures.
The 136 connection lifecycles and their statement counts reconcile, without coverage errors.

| Complete-job SQL category | Actual executions |
| --- | ---: |
| Setup, including DDL and ANALYZE | 17,686 |
| Transactions | 168 |
| Matching, publication, and cleanup | 6,305 |
| Total | **24,159 / 100** |

The trace contains 11,544 CREATEs and 5,508 DROPs.
Execution 101 creates `syntax_21` during compilation schema validation, with an empty context on the main thread.
This result identifies repeated session setup and schema construction in addition to the earlier candidate-preparation regression.
Index publication also repeats SQL over kind catalogs, fields, terms, and files.
The count includes 1,100 executions of `INSERT INTO kind` and 128 executions of `INSERT INTO field`.
Those execution totals are not CPU attribution.

The real workload exits nonzero because the fixed budget fails. Complete findings remain available.
Two instrumented attempts complete in 62.872 and 88.896 seconds, within the 115-second bound.
Both record the same 24,159 executions and complete parity.
The second attempt also preserves separate target and acceptance exit codes, so offline reporting retains successful parity.
Instrumentation and deferred explanations make its CPU unsuitable for an aggregate speed comparison.
The measured addon has SHA-256 `bf671d4a7ade25aa484fa54994fd611d093c671129b17e043d18b6a2056b629f`.
No rule algorithm or production execution option changes in this step.

The [recovery workplan](sql-performance-recovery-workplan.md) orders accounting, eligibility, publication, sharing, bounded SQL, failure correction, and aggregate acceptance.
WP1 remains pending and starts within recovery step R4 in this same worktree.
The restored acceptance failure is not waived while that work proceeds.

Tool checks pass 21 Node tests, 75 Python tests, and six production-tracer tests through the standalone host.
The standalone tracer Clippy check and repository typecheck/static checks pass.
The broad native Clippy command reaches the 112-second controller deadline and remains incomplete.
The focused native library Clippy check passes with the profiling feature.
The existing producer-shape checks also pass, separately from the failing SQL execution gate.
The standalone host gains its missing test-only `tempfile` dependency. Existing dependency versions remain unchanged.
The initial gate-test invocation fails before implementation because its module is absent.
The first standalone native invocation exposes that missing dependency rather than a behavioral test result.

Local captures and validation reports remain under `.rerail/profiles/sql-budget-recovery-20261023/`.

## Aggregate optimization checkpoint: 2026-10-23

Status: repository comparisons are complete, with substantial CPU regressions and failed broader gates.
This is an evidence checkpoint, not a passing performance release. WP1 has not started and will use the same worktree.

The archived pre-SQL reference has SHA-256 `52692a151eea8bab2add9c1c3aeb8400bf0cf1e59e743f5ffdf176265b6f945b`.
Its archived changed-source manifest matches the corresponding files at `5616076a`.
The interrupted first attempt uses addon `4df3c64a06c079d4199e942fbf3daa46dba5405e635c6228f7deb199f57c92a6`.
The final SQL reference is `dab45422f19898d582f330c38fd402aadffc0bd4edb8d916827d85cd63c90419`.
The [aggregate evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/aggregate-checkpoint-20261023.json) records current validation progress and frozen source identities.
Both references use engine optimization level 1, dependency level 3, no cross-crate LTO, 256 codegen units, and the system-malloc shim.

Serial comparisons cover the original first-50 and combined 60-rule repository snapshots.
The source pin, file membership, workers, ingestion batches, and expected findings remain fixed within each comparison.
The baseline uses its committed policies. The new reference uses the optimized policy definitions.
This measures aggregate engine and authored-SQL changes. It does not isolate one optimization.
Tracing, SQL capture, and explicit allocation census remain off during CPU timing.

The initial complete native run passes 1,547 tests, fails ten, and ignores 23.
Nine failures come from fixture calls to `git add .`, which the environment rejects.
Explicit fixture filenames correct those calls, and all 17 Git workspace tests then pass.
The remaining allocator-specific test requires jemalloc arena controls absent from the system-malloc reference shim.
The cached genuine jemalloc library then passes the allocator-specific test.
Final native checks use that library; matched CPU references retain the same system-malloc shim.
The run archive preserves the historical shim binary and its hash; its source is unavailable in this workspace.

The broad public-suite attempt reaches its 112-second controller deadline and is not a completed suite result.
It exposes additional Git fixture calls and two stale catalog assertions.
The corrections preserve explicit fixture inputs, reject an actually unknown predicate, and compare grammar membership independently of enum declaration order.
Typechecking and all repository checks pass before those test-only corrections.

Local runs and frozen inputs are under `.rerail/profiles/aggregate-checkpoint-20261023/`.
Runtime source is committed at `5b09437c`; fixture and runner corrections are committed at `b9c4fc67`.
The final measurement archive pins the addon and all 470 native source files.

| Workload | Reference | Cold CPU | Resource high-water RSS | Parity |
| --- | --- | ---: | ---: | --- |
| First 50 rules | Pre-SQL | 125.804 s | 6.441 GiB | 24,820 findings, pass |
| First 50 rules | Final SQL | 2,507.496 s | 6.143 GiB | 24,820 findings, pass |
| Combined 60 rules | Pre-SQL | 709.151 s | 16.102 GiB | Findings match; 23 failures, two failed incremental checks |
| Combined 60 rules | Final SQL | 7,211.657 s | 14.997 GiB | Findings match; 34 failures, two failed incremental checks |

The fresh pre-SQL first-50 reference completes all 9,021 files with 24,820 findings and no parity failures.
Cold CPU is 125.804 seconds. This replaces historical CPU values as the matched comparison reference.
The corrected first-50 SQL reference completes with all 24,820 findings and no parity differences.
Cold CPU is 2,507.496 seconds, or 19.93 times the matched pre-SQL reference.
Reported resource high-water RSS falls from 6.441 to 6.143 GiB, a 4.6 percent reduction.
Separate `ps` sampling records peaks of 6.441 and 6.217 GiB.
Sampled RSS and `process.resourceUsage().maxRSS` do not agree exactly in these reports; the evidence preserves both measurements.
Some ingestion boundaries also report current RSS above the resource high-water counter.
These observations prevent a strict bound between the two APIs, without changing the large CPU regression.
This aggregate result is a substantial CPU regression, rather than an optimization success.
The prior 60-rule reference completes with all 24,993 expected findings and no finding differences.
It reports 21 file-budget failures, two incomplete-input failures, and two failed incremental checks.
It is not a successful parity reference.
The failures include serverplan-target-architecture clauses on `commit_branch_steps.py`.
The final SQL 60-rule run completes with all 24,993 expected findings and no finding differences.
It reports 34 relational failures in `serverplan-state-tripwires`: 12 regex-budget errors and 22 query interruptions.
The unchanged and trailing-newline edit checks retain those failures, with zero finding differences.
Matching findings do not establish complete execution when failures remain.
The regex and SQLite progress handlers share a deterministic work allowance; these errors are not command wall-time deadlines.
The full-repository result therefore fails, despite the passing source-corpus tests below.
The two 60-rule references fail differently, so their times do not establish a clean performance ratio.

The final 60-rule cold CPU splits into 6,782.080 seconds for ingestion and 429.207 seconds for the global apply.
The failed prior reference uses 202.550 and 506.184 seconds respectively.
The final unchanged check uses 0.517 seconds CPU; the edit uses 121.587 seconds, versus 62.576 seconds previously.
Total OS process CPU is 7,341.770 seconds, including startup, compilation, and incremental checks.
Sampled peak process RSS is approximately 14.997 GiB, close to the reported resource high-water value for this run.
The controller records 15,877.030 seconds wall time, including the recorded pause and diagnostic interference.
The OS `time` report instead records 3,516.32 seconds as `real`; this clock discrepancy remains unresolved.
Both raw reports remain preserved. Neither wall value supports a comparative performance claim.
Its 288 ingestion batches execute 1,363,392 candidate data statements and 1,359,072 materialization actions.
They prepare 197,856 plan memberships: 687 plans per batch, versus 377 in the first-50 run.
The same 28,230,245 indexed nodes feed both rule sets. Adding ten rules broadens preparation work substantially.
Native anchor-selection counters report 6,421.643 seconds; nested thread counters are not an additive process-CPU breakdown.

The controller restarts after the completed references to remove its arbitrary three-hour cap for the final full run.
That run uses the existing approval to finish and records OS process CPU and peak RSS alongside driver phase CPU.
The briefly started, interrupted 60-rule SQL trial remains excluded.
The final SQL trial is allowed to finish despite its measured CPU regression.
Intermediate process samples include startup and do not replace completed driver phase measurements.
During that trial, host free space falls below four GiB.
Cleanup removes 0.782 GiB of unopened, orphaned temporary index files after two batched open-file checks.
Transparent APFS compression saves another 4.519 GiB across 167 old text captures.
SHA-256 checks verify unchanged capture contents; their paths remain intact.
The active database, reference binaries, and source inputs remain unchanged.
This storage maintenance can add host contention during the 60-rule trial; the completed first-50 comparison precedes it.
Host free space later recovers above 550 GiB without a storage pause of the target process.

The completed first-50 reports locate the CPU regression in file ingestion.

| Cold phase | Pre-SQL CPU | Final SQL CPU |
| --- | ---: | ---: |
| File-input applies | 73.350 s | 2,456.015 s |
| Global result apply | 52.144 s | 51.145 s |

The current native counters report 2,247.966 seconds in anchor selection and 125.776 seconds in anchor index construction.
Parsing uses 11.087 seconds. These counters use thread CPU and can nest; they do not form an additive process-CPU breakdown.
Historical native counters need their original clock semantics before a direct phase comparison.
The driver apply measurements above use process CPU on both references.
The first-50 cold ingestion executes 288 candidate batches, 492,480 data statements, and 489,024 materializations.
Those batches publish 28,230,245 indexed nodes across 9,010 eligible file memberships.
Their plan memberships total 108,576: 377 plans per batch.
These are runtime counters from the complete run, rather than static SQL placement estimates.

The first-50 run exposes an ingestion CPU regression absent from the earlier ten-rule controls.
At 576 ingested files, sampled process CPU reaches about 209 seconds, while retained process RSS remains below one GiB.
The historical first-50 run completes all 9,021 files in 212.6 cold CPU seconds under different reference-build settings.
That observation signals a regression but does not establish a matched full-run ratio.
The new run's ingestion counters attribute most active work to candidate SQL preparation and execution.

### Broad-run access-path regression

The initial first-50 comparison stops after 960 ingested files and approximately 560 sampled process CPU seconds.
It remains an interrupted diagnostic, without a complete parity or CPU result.
One external sampling window contains 730 active native samples, all under candidate SQLite work.
The sample records VM execution, B-tree traversal, and file I/O, rather than mostly SQL compilation.
It does not assign whole-run CPU percentages.

The bundled `fts5VocabBestIndexMethod` accepts term constraints and ignores document constraints.
The candidate query joins requested files after the vocabulary instance stream scans corpus-wide postings.
Later ingestion batches therefore repeat work over an increasing corpus.
This access-path cost was absent from the earlier tiny-corpus timing controls.

A regression test requests two identifier nodes from one structurally completed file.
Adding an unrelated file with 2,000 statements increases its progress-counter ticks from 1 to 361, with 100 VM instructions per tick.
An initial decoded `(fid,kind,pre)` index passes the covered-file test but fails the representative sparse ingestion probe.
The 112-second diagnostic reaches approximately 256 files, so this trial is rejected.

The selected correction adds compressed positions to existing SQL `file_kind_counts` rows.
The existing `(fid,code)` primary key seeks each requested file and kind before the postings table expands positions.
The existing record pass builds these segments alongside counts, without additional SQL rows or per-node queries.
The original FTS index remains intact. Sparse files need no eager decoded property rows.
An atomic version-2 store upgrade derives positions from persisted records, without source reparsing.

All ten SQL preparation tests pass, including four new access-path and upgrade tests.
They cover sparse and completed files, unrelated-file growth, error and missing syntax, edits, removals, and failed-upgrade rollback.
The intermediate kind-only addon has SHA-256 `f78fad34aab59e19be3e202b450887341156d281279a2ee153d25fa7148ceed6`.
Its native source manifest is frozen before repository timing.
This trial remains provisional: repository ingestion still exposes corpus-growth cost.

Before the value regression test, the complete native suite passes in three disjoint groups: 1,561 pass and 23 ignore.
The allocator-specific case runs against genuine jemalloc.
Typechecking, boundaries, structure, Windows paths, and profiling-tool checks also pass.
The full repository comparisons and complete source-corpus checks remain pending.

The corrected-kind trial stops after 480 files rather than locking down an unexplained ingestion regression.
Its external sample again shows VM execution and file reads as the main active native work.
A second bounded-work test now checks exact value lookup as unrelated files increase.
The exact-value test fails: 200 unrelated files increase work from 1 to 19 progress ticks.
The selected SQL correction fences term and requested-file inputs before the existing `vidx` primary-key seek.
The missing-value fallback likewise starts with requested files.
All eleven preparation tests then pass, including the new value growth check.
The rebuilt reference has SHA-256 `e3b6541c2534a335cfeb2863b6af623bbfa0473ae1f8f0cd68070c142177bbf4`.
It is now frozen for final public checks, source replay, and repository measurements.

The final native test inventory contains 1,585 tests: 1,562 pass and 23 ignore.
A bounded group times out with three slow query checks pending; separate bounded invocations complete those checks.
The coverage manifest accounts for each test name rather than treating the interrupted group as a suite pass.
Both the execution-shape gate and all 25 structure metrics pass, including newly added files.
The profiler also gains tested text-summary support for source and incremental replay reports.
Earlier summary-format exceptions leave successful replay results intact; regenerated summaries use those preserved results.

Some public test groups exceed their bounded controller deadline and require smaller independent groups.
The node-pack scale test passes in isolation on both reference addons; it is not itself a proven regression.
One audit test fails in an initial group and passes in isolation.
Final public verification runs independent files and individually selected source-wrapper tests.
The profiler retains its 115-second command budget; the public invocation raises the default per-test timeout to 90 seconds.
Three source wrappers initially exceed the five-second default and pass with that invocation.

The serverplan-steps recording has 746 replayable observations, so one per-rule job cannot meet the command budget.
It now uses 63 disjoint, balanced jobs, with original observation identities preserved exactly once.
Absent source fields remain absent on retained observations; original recordings remain unchanged.
The combined positive and clean coverage remains mandatory for every rule.
The public pack-validation wrapper also exceeds the command budget.
Per-pack invocations retain all examples and fixture checks, but JavaScript, Node, Python, React, TypeScript, and Vue still hit the 112-second controller limit.
These remain incomplete broader checks, without a passing suite claim.
The SQL profiling Node test now stays outside Vitest collection and passes under its dedicated runner.
One unrelated TypeScript scale check exceeds its five-second wall-time assertion on the busy shared host.
That assertion remains unchanged. The paired baseline addon passes the same test.
The corrected addon still fails in isolation; an instrumented retry measures 8,428 ms wall and 3,849 ms CPU for mixed evaluation.
The source case contains 1,000 files and 2,293,760 bytes.
This is a broader performance regression, without a source-parity failure.
The test reports CPU beside wall time without relaxing its existing ceiling.
The complete corpus now passes all 133 bounded source jobs.
They cover 3,210 native observations across all 60 rules: 1,631 positive and 1,579 clean.
Three host signature guards remain separate from native execution counts.
False positives, false negatives, engine failures, and snapshot/workspace differences are all zero.
All 40 edit transitions also pass.
The evidence verifies original observation identities across shards without summing repeated collection metadata.

### Retained-property join regression

The post-value first-50 attempt still stops after 480 files and 175 sampled process CPU seconds.
It remains an interrupted diagnostic, without a complete comparison result.
A new test reads structural parents for one requested file, then retains 2,000 statements in an unrelated file.
Identical results require 806 progress ticks instead of six.
Each tick represents 100 SQLite VM instructions.

The property views use ordinary joins between demanded keys and retained decoded rows.
The selected correction makes requested keys the outer input with `CROSS JOIN` fences.
It covers universal node reads and keyed node, span, and child-property reads.
This preserves the existing indexes, row semantics, and batched execution interface.
All 60 active SQL anchor tests pass after the correction; one benchmark test remains ignored.
The rebuilt addon has SHA-256 `dab45422f19898d582f330c38fd402aadffc0bd4edb8d916827d85cd63c90419`.
All 1,586 native tests are accounted for: 1,563 pass and 23 ignore.
The forty edit transitions also pass on this final build.
All 133 bounded source jobs pass again on this final build, with the same 3,210 native observations and three host guards.
False positives, false negatives, engine failures, and snapshot/workspace differences remain zero.
All six larger pack retries still hit the command deadline and remain incomplete broader checks.
Typechecking, both Clippy configurations, execution-shape checks, and all-file structure checks pass.
The TypeScript wall-time guard still fails during these contended checks: 12,705 ms wall and 4,307 ms CPU.
Its correctness assertions pass, and the wall-time ceiling remains unchanged.

### Preparation frontier found during the full comparison

The workspace gathers anchor plans from every compiled rule before it publishes the eligible source jobs.
It filters publication when a file has no eligible AST rule, but then prepares every language-compatible plan for each published file.
The corpus API receives language and plan identities, without per-rule file eligibility.
This makes the plan/file frontier broader than the final matcher work.

One first-50 batch publishes 32 files and prepares 377 plans with 1,710 statements and 1,698 materializations.
Its profile reports 128 rule-file entries and 128 rejections.
The SQL frontier therefore performs substantial work before final filtering.
This observation does not quantify its removable CPU share; a scoped-frontier experiment remains future work.
Here, a rejection means zero selected roots. It does not prove that scope or condition analysis could skip all preceding work.
Across the completed first-50 run, 83 applies report zero roots for every measured rule-file selection.
They still execute 140,934 materializations and use 697.407 seconds of anchor-selection CPU.
That cost includes work needed to prove absence. It is not automatically removable.

`SelectWhen` planning also collects both queries before the runtime condition selects consumers.
The next execution design must preserve eligibility and conditional demand while retaining broad batched launches.
A complete static plan inventory does not require execution of every plan/file pair.
Repeated selector construction around retained facts remains another measured target.

This separates index join order from the planned cross-rule producer work; WP1 remains pending.

### Large-corpus pager pressure

Across four equal groups of 72 first-50 ingestion applies, anchor-selection CPU per indexed node rises from 50.2 to 115.3 microseconds.
The last group processes fewer nodes than the first. Source content also changes, so this comparison alone does not establish a corpus-size cause.

A one-second external sample observes the final 60-rule run near 1,600 ingested files.
Of 749 selected non-parked stack observations, 537 end in `pwrite` and 65 end in `pread`.
The write callers include `pagerStress`, `pagerWalFrames`, and `subjournalPageIfRequired` under candidate materialization.
These are wall-time stack observations, including possible I/O waits, rather than measured CPU shares.
SQL compilation appears in seven observations. The window identifies pager and statement-journal traffic as a concrete investigation target.

The production store sets its main page-cache target to 1,024 KiB and its temporary target to 512 KiB.
The main database already exceeds one GiB during this sample interval.
Cache-budget and publication-boundary experiments must preserve rollback guarantees and measure cache misses, writes, spills, CPU, and memory together.
The isolated cache experiment below tests this proposal. The native implementation stays frozen.

An APFS clone preserves the main database and WAL for later isolated replay.
The target process pauses for 72 milliseconds during the clone, then resumes immediately.
The snapshot needs SQLite recovery and validation before replay.
The 60-rule timing therefore includes external sampling, this brief snapshot pause, and the storage maintenance above.
The completed first-50 controls precede these diagnostics and remain separate.

The profiling report now labels I/O stack observations explicitly to prevent their interpretation as CPU percentages.
Its new counter test passes. The profiling suite now passes 74 Python checks and 14 Node checks.
The standalone replay host also gains a `cache_budget` command with cheap cache counters and explicit physical budgets.
It reuses scheduled SQL execution and avoids the ownership observer's full-table `dbstat` scans.
Its row-bag and file-scope test passes with the other 18 standalone Rust checks, and the command builds successfully.
This addition changes profiling tools only; production cache targets remain frozen.

### Controlled cache replay

The full process pauses for 1,022.218 seconds while isolated replay runs, then resumes with its computation intact.
This pause affects the reported 60-rule wall time. It does not add CPU to that stopped process.

The fixture combines the recovered native snapshot with a native-recorded state-rule input, without rebuilding its indexes.
It retains 1,886 files and 6,223,803 nodes, and requests `_conditional_compile.py` with its original 18,993-node index representation.
Source digests match the pinned checkout, and SQLite `quick_check` passes.
This query-scaling fixture holds the requested program and index representation constant; it does not measure publication or complete engine execution.

Twelve replays use three rotated observations for each physical cache pair.
All preserve the exact 146,136-row candidate multiset, including multiplicities, after file-ID normalization against the small native capture.
Every large-fixture replay records 59,590,722 VM steps, 3,294,894 full-scan steps, and 4,979 sorts for stage and result statements.
Those VM totals exclude index and release batches.

| Main / temporary cache target | Median SQL CPU | Median process peak RSS | Cache misses | Cache writes | Cache spills |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 MiB / 0.5 MiB | 1,531.078 ms | 112.656 MiB | 66,709 | 14,949 | 14,630 |
| 64 MiB / 0.5 MiB | 1,336.781 ms | 143.563 MiB | 10,814 | 5,427 | 1,721 |
| 1 MiB / 16 MiB | 1,402.635 ms | 109.344 MiB | 62,595 | 13,119 | 12,909 |
| 64 MiB / 16 MiB | 1,403.610 ms | 150.484 MiB | 6,700 | 3,597 | 0 |

The combined larger budgets eliminate measured spills and reduce median SQL CPU by 8.3 percent in this fixture.
CPU varies across trials; these observations do not establish a preferred full-engine budget.
They also do not explain the 19.93-fold aggregate CPU regression.
The unchanged VM work remains substantial for one requested file, so demand scope and repeated query work remain the main architectural targets.
Production settings remain unchanged for the completed checkpoint comparison.

### Full-run memory observation

An external `vmmap` inspection during 60-rule ingestion completes in 3.023 seconds.
It reports a 6.6 GiB physical footprint, with 4.5 GiB resident and 1.9 GiB reported swapped in `MALLOC_SMALL` regions.
Its allocator-zone view reports 15,992,992 allocations and 6.2 GiB allocated.
These rounded region and allocator views overlap; they are not an ownership census.

Prepared candidate payloads account for about 218 MiB near this interval.
The completed first-50 run peaks at 86.565 MiB of compact candidate payloads.
Those counters exclude map metadata, but the payloads alone cannot explain the process footprint.
The explicit workspace estimator also omits heap storage owned by retained tape, scope, and preparation fields.
Attribution of the remaining native allocations remains open; this observation does not establish a leak in any specific structure.

The profiling framework now extracts these external memory fields into structured reports while preserving the raw output.
The full 60-rule timing includes this additional external observation window.

A second external inspection runs after cold completion, during the unchanged/edit boundary, and takes 7.527 seconds.
Its rounded allocator-zone view reports 31,521,959 allocations and approximately 11.3 GiB allocated.
Physical footprint is approximately 12.7 GiB, with a reported peak of 13.9 GiB.
These views remain distinct from peak RSS and from Rust ownership estimates.
The larger retained allocation count reinforces the need for ownership attribution before a memory improvement claim.

### Final diagnostic and tooling checks

The profiling suite passes 75 Python checks and 14 Node checks, including the new external-memory parser test.
All 19 standalone SQL replay tests pass. Standalone format and Clippy checks also pass.
The frozen manifest still matches all 470 native files after the full run.
Final typechecking, boundary checks, structure checks, Windows-path checks, and execution-shape checks pass.
The structure check also passes after the new replay command enters the tracked file inventory.
The broader public-suite failure and six timeouts above remain unresolved; these checks do not supersede them.

The bounded ten-rule/two-file diagnostic preserves all 31 expected findings with no execution failures.
It records 11,594 trace events and a successful external memory inspection.
Two external sample commands exit with signal 9, so their observer wrapper fails despite target success.
The third sample succeeds; its 823 selected non-parked observations include 299 I/O syscall leaves.
These wall-stack observations include profiler and trace overhead and do not establish CPU percentages.
The separate state-rule SQL capture also passes, recording 6,095 trace events with query plans.
Neither instrumented workload replaces the untraced aggregate comparison.


## Shared producer implementation plan: 2026-10-23

The [workplan](shared-producer-workplan.md) defines eight runtime milestones after the existing offline tooling.
The first execution milestone shares span preparation across actual rules and result packets.
Later milestones cover structural facts, compiled frontiers, cross-clause SQL owners, pack definitions, and reviewed derived producers.
An early connection check establishes the storage assumptions before cross-clause integration.

Each runtime change requires parity evidence and matched CPU and allocation measurements.
The plan retains the existing installed baseline and separates static placement counts from runtime construction counters.
This entry records planning only. The runtime milestones remain pending, and no new benchmark or Linear issue enters this entry.

## Named producer discovery and declaration checks: 2026-10-23

The selected [contract design](shared-producer-contracts.md) replaces general equivalence discovery as the primary sharing mechanism.
Contracts live beside their implementations, while a central registry references them across consumers.
Pack-specific derived work belongs in a pack producer library. Private rule stages retain qualified local identities.
The runtime registry and compiler export remain implementation work.

The new [CLI](../scripts/profiling/shared_work.py) discovers repeated work in candidate captures and checks explicit contract manifests.
Clash reports contain both origins and exact field differences.
Binding diagnostics explain why different options, generations, snapshots, or domains prevent sharing.
Different demand sets can share a keyed producer. Unknown semantic fields fail declaration checks.

The [evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/shared-producer-discovery-20261023.json) retains capture hashes and discovery witnesses.
The tool processed the earlier state and ten-rule captures, without new engine execution.
Both reports cover every host statement, without skipped scopes.

| Captured workload | Host statements | Materialized placements | Definitions in multiple hosts | Extra host placements |
| --- | ---: | ---: | ---: | ---: |
| State | 1,716 | 10,320 | 157 | 1,240 |
| Ten rules | 3,216 | 19,001 | 255 | 2,048 |

These counts are lower than the earlier audit because the new tool distinguishes mutable-write epochs and physical CTAS projections.
It also preserves quoted literals and resolves local dependency context.
The earlier journal entry retains its original method and results.
These are static placement counts, without runtime build counts or CPU savings claims.

The 25 new checks cover false sharing, dependency aliases, conflicting names, typed options, snapshots, domains, cycles, and digest collisions.
The complete profiling-tool suite passes, and CI includes the new checks.
Production engine source and the installed addon remain unchanged.

## Compiled stage execution design: 2026-10-23

The [execution design](sql-execution-stages.md) selects a general planning graph with a specialized runtime controller.
The compiler fixes dependencies, operation families, storage bindings, and potential consumer lifetimes.
Runtime frontiers resolve actual demand, conditional gates, coverage, and capacity admission.
Recursive algorithms remain inside declared stages. SQL retains relational execution.

The design combines candidate lifetimes and clause producer waves under one evaluation plan.
Demand-source leases prevent retirement before later consumers discover their inputs.
Shared SQL backing uses a workspace writer and scoped clause readers without fact imports.
The connection contract requires an explicit isolation and cancellation proof before cross-clause promotion.

This entry records design work only. Production source and addon binaries remain unchanged.
The existing CPU and allocation measurements remain the implementation baseline.

## Materialization ownership design: 2026-10-23

The [target design](sql-materialization-shape.md) assigns one owner to every selected producer version and coverage partition across rule consumers.
Statements and result packets read those owners without embedding another materialized copy.
The design covers demand waves, empty completion, semantic domains, clause sharing, and retirement.
It retains indexed base access and single-use pipelines without blanket materialization.

The [static placement audit](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-materialization-design-20261023.json) resolves local dependencies and typed parameters before comparing definitions.
It identifies 248 state definitions and 420 ten-rule definitions present in multiple generated statements.
One demanded-span definition appears in 61 state statements and 90 ten-rule statements.
These are SQL placement counts, not measured execution counts or CPU savings.
The audit also identifies repeated empty-set materializations, which require simplification rather than another retained cache.

This entry records design work only. Production code, addon binaries, and execution remain unchanged.
Earlier benchmark results remain the baseline for implementation.

## Candidate SQL relation lifetimes: 2026-10-23

**Decision:** retain one dependency schedule with producer construction before demand and release after the last SQL consumer.
This bounds temporary storage. CPU and process RSS remain mostly flat on the measured snapshots.

The [implementation](sql-composition-implementation.md#candidate-relation-lifetimes) describes the execution contract.
The [evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-relation-lifetimes-20261023.json) retains trials, source hashes, storage measurements, allocation peaks, and corpus results.
The reference addon has SHA-256 `7a193b2a060803b6e5f73e6760066e72c43d962aa673c2fd81f51f38405f6c3b`.
The final addon has SHA-256 `86846fe45e731c10b44beecb54cbfb35b7610c2eafb71f199ce3840969c07b6d`.
The data-layer source remains at `2b1504adc4291daaf080b2e932962b134da1a69a`.
No policy changes enter this comparison.

### Implementation

Previously, the executor constructed every temporary candidate relation before the first result packet.
It retained every relation until all packets finished.
The new compiler derives dependencies from generated SQL and retains one schedule in each cached template.
It preserves ordering between retained-property reads and writes, including vocabulary maintenance.
Physical table reads determine storage lifetime separately from execution ordering.
Shared producers run once across output packets. Output order remains unchanged.
Adjacent releases share one SQL call. Error cleanup releases remaining tables and preserves the original failure.
The streaming adapter and retained decoded properties remain in use.
Production has one execution path and no new algorithm switch.

Two TDD ladders cover the schedule and its ordering constraints.
The second ladder prevents write ordering from extending an unused snapshot's storage lifetime.
Additional checks cover shared inputs across packets, delayed construction, maintenance writes, and retry after a later packet fails.
SQL profiling explains new statements before their inputs disappear, without repeated report serialization.

### Storage and paired process measurements

The replay host uses captured production SQL, codecs, database settings, and the production schedule.
Its eager comparison exists only in the diagnostic binary.
Both layouts emit identical candidate rows in identical order.
The state replay emits 146,136 rows. The ten-rule replay emits 726,444 rows.

| Snapshot | Peak live tables, before → after | Peak temporary storage, before → after |
| --- | ---: | ---: |
| State, one file | 1,029 → 206 | 14.00 → 2.66 MiB |
| Ten rules, two files | 1,905 → 437 | 48.10 → 8.99 MiB |

The following numbers are medians from three observations per side.
Cold CPU includes process user and system time. Compilation CPU remains separate.
Peak RSS includes setup and compilation.

| Snapshot | Cold CPU, before → after | CPU change | Peak RSS, before → after | RSS change |
| --- | ---: | ---: | ---: | ---: |
| State, one file | 2.582 → 2.597 s | +0.6% | 453.2 → 452.5 MiB | −0.2% |
| Ten rules, two files | 13.317 → 13.106 s | −1.6% | 563.1 → 553.3 MiB | −1.7% |

State CPU observations range from 2.534–2.729 seconds before and 2.577–2.647 seconds after.
Ten-rule CPU observations range from 13.005–14.127 seconds before and 12.994–13.388 seconds after.
Compilation CPU medians are 0.301 → 0.307 seconds for state and 0.759 → 0.747 seconds for ten rules.
These measurements establish bounded temporary storage, without a substantial CPU or RSS improvement.

Each workload uses the same selection, source hashes, policy digests, allocator, four workers, and file batch size of 32.
The serial order is before-0, after-0, after-1, before-1, before-2, after-2.
The benchmark runs without concurrent builds or checks from this task.
Tracing, SQL capture, allocation accounting, and explicit ownership census remain off during timing.
Each workload command uses the 115-second controller deadline.
Long compilation commands use the earlier build exception.
Shared-host load varies. Three trials per side do not establish statistical significance.
These snapshots do not establish full-repository performance.

### Why process memory remains high

New trace boundaries distinguish live objects, table bytes, index bytes, reserved pages, free pages, and SQLite resident memory.
They also record current RSS, peak RSS, and available allocator statistics.
Each snapshot retains only the largest 20 objects and totals for all objects.
Memory gauges retain maxima rather than sums.

The trace-only state run ends candidate preparation with zero live temporary candidate tables.
Its AST-index connection retains about 1.54 MiB of SQLite cache and about 2.65 MiB of free temporary pages.
Allocator usage changes from about 65.9 to 66.6 MiB across candidate execution.
Managed allocator space grows from about 138.4 to 210.4 MiB.
Current RSS grows by about 71.7 MiB across the same boundary.
Managed allocator space describes address space, not resident physical memory.
These observations indicate transient allocation pressure and retained allocator space rather than large remaining physical candidate tables.
Later rule fact construction raises the process peak further.

The isolated replay enables SQLite allocation accounting before initialization.
It separates SQL preparation peaks from execution peaks and compiled statement storage.
This accounting does not change production settings or enter the paired timings.

| Snapshot | Largest SQL preparation peak | Largest SQL execution peak |
| --- | ---: | ---: |
| State | 10.1 MiB | 68.5 MiB |
| Ten rules | 11.8 MiB | 98.6 MiB |

The state execution peak occurs in the first result packet and returns to about 2.1 MiB afterward.
Its compiled statement occupies about 3.3 MiB.
The query contains 1,101 inline materialized CTEs.
Its captured plan reports 1,101 materializations and 983 temporary B-trees.
These inline working sets remain outside the physical-table schedule.
The allocation peak belongs to execution, not preparation alone.
The measurements do not assign bytes to each individual CTE.

The next planning change must include inline materializations in the working-set budget.
Candidate approaches include scheduled physical producers and result packets bounded by their actual producer footprint.
Any packet change must retain sharing and measure repeated work across packets.
The next comparison must measure statement execution peaks, CPU, and process RSS together.
Physical table counts alone cannot establish a memory improvement.

SQL profiling retains large plans and increases diagnostic RSS.
The ownership and allocator discussion therefore uses the separate trace-only run.
An earlier replay reported zero global SQLite counters because memory accounting was disabled.
The corrected replay enables accounting before SQLite initialization and records nonzero preparation and execution peaks.
The evidence excludes intermediate addons and superseded diagnostic runs from the final comparison.

### Checks

The final addon passes all 269 affected original corpus calls: 194 positive calls and 75 clean calls.
There are no false positives, false negatives, or engine failures. Workspace and stateless results agree.
The ten-rule smoke replay passes 20 calls, including 10 positive and 10 clean calls.
The affected public suite passes 324 tests, with two existing skips.
Focused native checks pass 82 matcher tests, 10 profiling tests, and 20 relational tests.
The standalone harness passes 18 tests.
Typechecking, profiling-tool tests, formatting, boundaries, structure checks, and Clippy pass.
Both production and profiling Clippy checks use the pinned Rust 1.95.0 toolchain.
The installed addon also passes the multi-packet and guarded-root smoke checks.
The real Git index remains unchanged.

## SQL composition promotion: 2026-10-22

**Decision:** retain selected-forest intervals, explicit span demand boundaries, and one shared candidate catalog per grammar frontier.
Keep the streaming packed-node adapter. These choices add no production algorithm switch.

The [implementation](sql-composition-implementation.md) describes the interfaces and SQL execution boundaries.
The [evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-composition-promotion-20261022.json) records artifacts, source hashes, trials, diagnostics, and corpus results.
The reference checkout remains `5616076ad60127d6ac3df0fbf60eff9f7a523ca0` with the existing uncommitted typed-fact changes.
The saved reference addon has SHA-256 `efa0dcf04b170a10588e4661c7843a846989ce98262e68b6bc6abbb6a7dca72f`.
The measured addon has SHA-256 `7a193b2a060803b6e5f73e6760066e72c43d962aa673c2fd81f51f38405f6c3b`.

### Implementation and integration failures

The forest adapter retains indexed positions and requested root intervals in SQLite.
It validates the selected graph before publication, including cycles, distinct parents, and disconnected components.
It preserves external numeric and text identities.
A 1,000-node chain retains 1,000 positions and 1,000 root ranges while exposing the same 500,500 logical pairs.
Consumers still pay for the rows they enumerate.
General directed graphs retain cycle-safe recursive SQL.

Root queries can select `__path,id,expand`.
False or null expansion keeps the reflexive pair.
If any duplicate request permits expansion, the root expands.
The four authored state clauses now use this representation and preserve their original root guards.
Syntax also exposes exclusive raw preorder bounds for SQL range predicates.
Raw bounds do not replace selected-edge graph validation.

Span selection remains materialized before costly source predicates.
All plans share one candidate definition catalog per grammar.
Output packets contain at most 128 arms and consume the same prepared relations.
The 535-output regression uses five packets with 128, 128, 128, 128, and 23 rows.
The v2 diagnostic capture and standalone replay preserve all 535 rows.

The initial integration exposed two regressions. Both required corrections before timing the selected implementation.
Sharing the full catalog amplified repeated copies and scans during SQL generation.
Indexed dependency traversal reduces template generation from 1,116.586 to 604.187 CPU ms in the diagnostic runs.
The old implementation used 600.219 CPU ms across its five smaller catalogs.
This correction removes the new regression. It does not establish faster compilation than the reference.

SQLite also reordered a correlated subtree probe to scan all identifiers before interval membership.
The affected `direct_events` stage increased from 774,666 to 16,328,922 VM steps.
Explicit `CROSS JOIN` order keeps each demanded subtree before its identifier lookup.
The corrected stage uses 985,870 VM steps.
This is an authored SQL change, and the generator emits it identically.

On the state diagnostic, candidate catalogs decrease from five to one.
Temporary candidate tables decrease from 1,784 to 1,029.
Candidate materialization and fill stages decrease from 3,044 to 1,712.
Result packets decrease from five to two for the 139-plan frontier.
These counts exclude other relational consumer tables.

### Measurements

Both versions use engine optimization level 1, dependency level 3, 256 code generation units, and no LTO.
Both addons use the same Rust 1.97.1 compiler, four native workers, and the system-malloc shim.
The repository checks use its pinned Rust 1.95 toolchain.
Timing controls disable tracing, SQL capture, external sampling, and memory census.
They run serially without our builds or tests.
CPU includes process user and system time during cold evaluation. Policy compilation remains separate.
RSS measures the whole process high-water mark, including setup and policy compilation.

Each result below is the median of three observations per version, with rotated execution order.
The state snapshot reads one 108,562-byte file and expects zero findings.
The smaller ten-rule snapshot reads two files, including required linter metadata: 178,826 bytes and 31 findings.
Fresh Python recording supplies its expectations.
Both versions preserve every expected finding without failures.

| Snapshot | Before CPU | After CPU | CPU change | Before peak RSS | After peak RSS | RSS change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| State, one file | 2.424 s | 2.120 s | -12.5% | 397.2 MiB | 451.5 MiB | +13.6% |
| Ten rules, two files | 13.338 s | 9.903 s | -25.8% | 497.1 MiB | 540.9 MiB | +8.8% |

The shared host causes substantial variation.
State CPU ranges from 2.375 to 5.619 seconds before and from 1.952 to 5.630 seconds after.
Ten-rule CPU ranges from 11.841 to 21.036 seconds before and from 9.588 to 12.019 seconds after.
The medians show a CPU reduction on these snapshots. They do not establish a stable full-repository speedup.

The original five-file ten-rule snapshot exceeds the deadline for both versions.
The first smaller after run also exceeds the deadline. Its completed retry remains identified in the evidence.
Censored runs and diagnostic runs do not enter the medians.
An external sample attempt also expires during symbol processing and produces no usable bottom-up attribution.
It supplies no evidence for a CPU attribution claim.

Policy compilation uses median CPU of 0.235 versus 0.344 seconds on the state snapshot.
It uses 0.658 versus 0.585 seconds on the smaller ten-rule snapshot.
These values do not enter the cold CPU table.

Peak memory increases on both snapshots.
State RSS after global evaluation increases from 221.5 to 400.5 MiB.
The smaller ten-rule snapshot increases from 446.1 to 489.4 MiB at that boundary.
Fewer SQL constructions do not guarantee lower allocator residency or a smaller live relation frontier.
The comparison does not yet separate those causes.
The next memory investigation must measure overlapping relations, indexes, and allocator residency across the shared catalog.
The system-malloc shim does not establish production jemalloc memory behavior.

These selections do not compare all 50 rules against the historical full-repository run.


### Validation and limits

The complete affected source corpora pass: 77 state calls and 192 quality calls.
They include 194 positive and 75 clean calls, with no false positives, false negatives, or engine failures.
Workspace and stateless execution agree.
Each bounded replay keeps the complete input of each original call.
The split does not partition repository-wide analysis by file.
A separate smoke replay covers one positive and one clean call for each of the ten rules.

The corpus replay started on addon `93733caa969fbf178f55e76870736c7a5ef6c84839bdfbe3848b6ae57faa11d7`.
The final addon adds an aggregate counter correction for extra result packets.
It passes the remaining corpus groups, the complete smoke replay, and all 324 public regression tests.
Per-group binary hashes remain in the evidence.
The native checks pass 76 anchor tests and 20 relational tests.
The standalone checks pass 12 Rust tests, including dependency traversal and v1/v2 replay.
All ten generated policies reproduce the checked files.
Typechecking, profiling checks, source boundaries, Windows paths, formatting, and all 25 structure metrics pass.
Clippy passes with and without profiling on the pinned Rust 1.95 toolchain.
A Rust 1.97 check reports a newer lint in an existing Rust-language test.
It does not change the implementation.

Workloads retain a 115-second controller deadline.
The user approved longer compilation after bounded build attempts expired.
Timed-out commands and diagnostic runs remain recorded and do not enter the performance medians.


## SQL composition opportunities: 2026-10-22

**Decision:** pursue proven tree intervals, explicit demand boundaries, and shared definitions with physical cuts.
Keep the current streaming node table for CPU execution.
Extra copied column buffers do not establish a CPU improvement.
These are isolated experiments. They add no production execution switch or engine cutover.

The [design](sql-composition-opportunities.md) records the sibling survey and integration requirements.
The [evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-composition-20261022.json) retains every measured trial, query, plan, source hash, and scope limit.
The baseline is `5616076ad60127d6ac3df0fbf60eff9f7a523ca0` with the existing typed-fact changes.
The engine addon remains `efa0dcf04b170a10588e4661c7843a846989ce98262e68b6bc6abbb6a7dca72f`.

The standalone host imports production packed decoders and SQLite adapters.
Each alternative has six fresh serial trials with rotated order.
CPU means process user plus system CPU.
The main source has 18,993 syntax nodes from the pinned `_conditional_compile.py` capture.
The terminal comparison uses three files and 68,035 nodes.

| Kernel and shape | Kernel CPU, ms | CPU with common setup, ms | Peak process RSS, MiB |
| --- | ---: | ---: | ---: |
| Tree: all-root closure | 267.068 | 282.436 | 9.875 |
| Tree: demanded-root closure | 73.356 | 89.109 | 8.062 |
| Tree: interval join | 0.994 | 17.677 | 6.359 |
| Demand: eager identifier text | 6.120 | 22.110 | 7.477 |
| Demand: selective SQL syntax | 2.943 | 18.458 | 7.438 |
| Demand: materialized keys | 2.267 | 18.768 | 7.539 |
| Utility depth eight: inline tree | 162.835 | 179.339 | 93.078 |
| Utility depth eight: named CTEs | 10.908 | 26.481 | 14.523 |
| Utility depth eight: physical cuts | 5.353 | 20.710 | 6.242 |
| Terminal: scalar functions | 24.780 | 88.933 | 14.234 |
| Terminal: current streaming table | 19.096 | 82.209 | 14.227 |
| Terminal: copied column waves | 20.394 | 84.406 | 17.547 |

Common setup copies captured typed facts and builds the same two indexes for every alternative.
The terminal query does not consume that copy. Its setup-inclusive total is not a production setup estimate.
Database copying, input shape checks, output serialization, and diagnostic EXPLAIN remain separate.
Peak RSS includes the complete parity result and diagnostic preparation.
These values do not compare engine memory with Python linter memory.

### Explanations

The demanded-root control separates root pruning from the interval representation.
Intervals avoid closure rows, deduplication, and two closure indexes.
VM steps decrease from 9,529,897 to 72,996.
The real consumer returns the same 694 root, count, and first-witness rows.
An exhaustive fixture also compares all containment pairs, including root self-membership and root stops.
The input check rejects a graph without contiguous preorder tree semantics.

Selective SQL syntax alone still decodes all 4,612 identifiers.
SQLite evaluates the native predicate before the selective edge joins.
A materialized demand relation reduces text and span decoding to 244 identifiers.
VM steps increase from 41,523 to 45,003, while kernel CPU decreases.
Setup-inclusive totals overlap and do not establish a further complete-pipeline gain over selective SQL.
The real source is clean for this predicate. Fixtures cover a positive, a clean file, and incorrect fields.

The utility stress uses distinct consumers of shared syntax keys.
It is a controlled compiler experiment, not a complete data-layer rule.
From depth four to eight, named SQL grows from 709 to 1,333 bytes.
Preparation CPU still grows from 0.392 to 6.075 ms.
Physical cuts keep preparation at 0.274 and 0.431 ms.
Naming alone does not bound SQLite preparation work.
The existing candidate compiler already supports common expressions and physical cuts.
The proposed extension shares semantic definitions across the whole policy.

The column table uses one cursor and two bounded CPU chunks for three files.
All terminal alternatives read the same 408,210 record bytes and 68,035 flag bytes.
The column table retains per-row SQLite iteration and adds column allocation and writes.
It therefore does not improve the current CPU path.
Future GPU producers still need immutable descriptors, bounded waves, and shared output columns.
CPU chunks do not define GPU launches. Broad count, scan, and scatter passes cover the complete task set at each dependency wave.
This experiment measures no GPU launches or device transfer costs.

Several small timings retain more than 5% variation after matched trials.
The evidence records the full range and coefficient of variation.
Small differences require caution. The closure and utility results have much larger margins.
No whole-rule or full-repository speedup follows from these kernels.

### Checks and next integration

Ten Rust checks cover tree shape, complete pair parity, positive demand, clean demand, utility keys, decoder parity, and batch lifetime.
Two Python checks reject lost positive multiplicity and incomplete execution.
Release Clippy passes with warnings denied.
The profiling suite passes all 59 checks, and all 25 repository structure checks pass.
The complete engine suite was not rerun because the spikes leave production execution unchanged.

The first production change is a typed subtree boundary for eligible authored descendant stages.
That change needs selected-edge semantics, capture identity parity, and positive corpus checks before broader timing.
Demand boundaries and shared utility definitions follow that change.
The copied column prototype remains a rejected CPU alternative in the isolated lab.

## Typed projected facts and SQL input streaming: 2026-10-22

**Decision:** keep typed column storage, SQL column demand, deferred preparation, and bounded SQL import.
These changes remove per-row JSON maps from normal AST projections. They do not establish an end-to-end performance improvement.
SQLite still executes the rules. The engine has one execution path and no new algorithm switches.
The [evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/typed-facts-20261022.json) records individual trials, hashes, counters, failures, and reproduction commands.

### Implementation

- [Projected facts](../native/src/code_rule_dsl/violations/projection.rs) share schemas, source slices, and immutable typed column batches.
- SQLite validation records actual column reads, including nested predicates and wildcards. Consumers omit unused projected columns without cloning matcher trees.
- SQL import binds column positions once per schema. It borrows scalar values instead of rebuilding JSON objects or copying each text value.
- The scheduler queues consumer descriptions. It restores file-local inputs during session preparation, rather than preparing every consumer before admission.
- [The input sink](../native/src/code_rule_dsl/relational/inputs.rs) inserts completed column batches and releases them during matching.
  It retains anchor metadata. Cached cross-file evidence also uses typed storage and borrowed SQL bindings.
- SQL input witnesses retain only the capture needed by their anchor. Messages, fixes, and unrelated capture spans are not constructed.

JSON remains the public wire format. Existing JSON-semantic cross-file operations still convert values at their boundary.
Large regex-group enumeration can still construct a full regrouping environment before output.
The new bound applies to projected SQL columns, not every matcher allocation.
Global source-reading clauses still retain source snapshots; this change does not bound every source allocation.
Producer waves and the SQL consumer scheduler retain their existing batched structure.

### Measurements

Both matched binaries use release dependencies, engine optimization level 1, 256 code generation units, no LTO, and profiling support.
Both use the same system-malloc shim, one worker, and 32-file ingestion batches.
Tracing, census, external sampling, and SQL capture are disabled in timing controls.
These RSS results do not establish production jemalloc behavior.
CPU excludes policy compilation; the evidence records compilation CPU separately.
Every command remains below two minutes.

The small state workload reads one actual data-layer file: 108,562 bytes, with zero expected findings.
Positive source tests provide separate rule coverage. Values below are medians of three alternating before/after pairs.

| Measurement | Before | Typed facts and streaming | Change |
| --- | ---: | ---: | ---: |
| Cold process CPU | 3.202 s | 3.228 s | +0.8% |
| Process peak RSS | 395.8 MiB | 461.6 MiB | +16.6% |
| RSS after global evaluation | 288.3 MiB | 222.2 MiB | -66.1 MiB |

CPU is effectively unchanged. Retained resident memory falls, but the process high-water mark increases.
The evidence does not support a memory or speed success claim for the full execution.

A separate ten-rule snapshot reads five files, including required linter metadata: 257,073 bytes and 44 findings.
One observation per version measures 24.695 s before and 23.529 s after in cold CPU.
Peak RSS measures 535.3 MiB before and 544.5 MiB after.
This single pair does not establish a speedup. Both versions match all 44 findings without engine failures.
Missing-file contracts contribute to those findings; the source replay also checks positive cases within actual test inputs.

The matched single-worker Python observations measure 0.264 s CPU for the state file and 1.429 s for the ten-rule snapshot.
Native execution remains approximately 12 and 16 times slower on these selections.
These are small selections, not the full 50-rule repository run or a comparison against its historical 1,400-second result.
The checked [state selection](../gate/data-layer/fixtures/host/data-layer-port/profiling/typed-facts-state-one.json) and
[ten-rule selection](../gate/data-layer/fixtures/host/data-layer-port/profiling/typed-facts-ten-four.json) preserve source hashes and snapshot expectations.

### Storage proof and remaining costs

The baseline queues 11,272 input witnesses with 198,758 JSON fields and an estimated 19,624,497 bytes of projection heap.
The final queue holds two consumer descriptions and zero materialized input rows.
The active consumer streams 11,272 rows with at most 256 buffered rows and zero retained projected rows after SQL import.
An intermediate typed census measured 4,208,391 bytes of column storage before the streaming handoff was added.
These counters describe logical storage. They are not exclusive RSS estimates.

Three external sample windows attribute 98–100% of selected native samples to SQLite.
Bottom-up stacks show VM execution, B-tree cursor movement, record comparison, and file reads and writes.
Sample shares are not exact process CPU fractions. External diagnostics are excluded from timing controls.
The vmmap snapshot also shows substantial allocator residency and slack.

New native memory probes record process high-water marks and macOS allocator statistics around selected stages.
They identify large absolute growth during `descendants` materialization and its two indexes.
In that diagnostic, the allocator holds about 266 MiB with about 133 MiB in use after the later index stages.
The large remaining CPU frontier is candidate SQL construction and execution.
The memory frontier includes overlapping SQL tables, indexes, and allocator residency during global evaluation.

The probe identifies where the new execution grows. It does not establish why its peak differs from baseline.
That differential remains unresolved. Replacing the database or adding another execution backend is not justified by these results.

### Validation and exclusions

Validation passes 1,234 native DSL tests, 67 workspace context tests, and 127 other workspace tests.
The runtime suite passes 320 tests. Existing skips remain: 23 native tests and two runtime tests.
Three large Python shortcut corpus tests were excluded from this bounded native run.
The source replay checks ten positive and ten clean cases across all ten rules, through stateless and workspace execution.
There are no false positives, false negatives, engine failures, or execution-path differences.
The instrumented addon also passes that source replay.
Profiling report tests, TypeScript checking, repository checks, formatting, and Clippy checks for production and profiling builds pass.

The original ten-rule twelve-file attempt and two smaller alternatives exceed the 112-second workload deadline.
They cannot establish performance or parity. One state control cannot write its report because the filesystem fills.
That run is excluded. The evidence retains these failures instead of treating incomplete output as a benchmark result.
The first memory-instrumentation build also reaches the deadline; its isolated retry succeeds within the limit.

## SQL preparation and retained-layout experiments: 2026-10-22

**Decision:** keep shared preparation, duplicate-expression removal, and incremental vocabulary matches.
Reject retained expanded postings and generic utility sets for the tested workloads.
SQLite still executes all candidate relations. There are no runtime backend switches.

The base commit is `5616076ad60127d6ac3df0fbf60eff9f7a523ca0`, with the existing value-index work still uncommitted.
The baseline archive captures that complete pre-experiment SQL source and its native binary.
Both rebuilt comparison binaries use release dependencies, engine optimization level 1, 256 code generation units, and no LTO.
Both use the same system-malloc shim, bundled SQLite settings, one worker, and 32-file ingestion requests.
Parser generation now builds at optimization level 3 to fit the bounded build deadline; this changes build-tool execution only.
The [evidence file](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-preparation-20261022.json) records binary and source hashes.

Each cell below is the median of three fresh serial runs, with before/after order alternated.
Detailed tracing, SQL capture, census, and external sampling are off during timing runs.
CPU includes ingestion, new preparation, selection, and final matching. It does not move preparation outside the cold timer.

| Workload | Phase | Before CPU | After CPU | Change |
| --- | --- | ---: | ---: | ---: |
| Two rules, twelve files, 61 findings | Cold | 2.476 s | 1.936 s | -21.8% |
| Two rules, twelve files | Edit | 0.543 s | 0.418 s | -23.9% |
| Quality rule, five files | Cold | 3.458 s | 3.268 s | -5.5% |
| Quality rule, five files | Edit | 1.007 s | 0.933 s | -7.3% |

Compile-plus-cold CPU decreases from 2.511 to 1.960 seconds on positive and from 3.707 to 3.513 seconds on quality.
Median peak RSS is 366.6 to 326.0 MiB on positive and 517.0 to 519.3 MiB on quality.
Unchanged requests remain approximately 3 ms and perform no candidate SQL or fact preparation.
All trials have exact findings and zero incremental differences.
Cold CPU variation stays below 5% within each three-trial group.
Peak RSS varies more on this shared host. Individual RSS measurements remain in the evidence; they do not prove a stable memory reduction.
The matched baseline is a fresh rebuild, so its quality CPU differs from the historical 3.192-second run.
The changes miss both proposed cold-CPU targets: 30% on positive and 10% on quality.

### Accepted execution changes

Preparation collects universal structural demands across all 32-plan groups before running them.
Fresh files stream typed tape columns into the retained SQL properties table.
Partly prepared files decode missing keys only. Changed files invalidate their own facts.
Sparse span and child demands retain their dependency order and sparse coverage.
No per-node packed row allocation or five-call scalar extraction remains in universal structural preparation.

The compiler removes equivalent generated expressions before it creates physical temporary tables.
Canonical definitions include typed parameters, child identities, language, column shape, and request dependencies.
Complete definition equality resolves hash collisions. Exactness metadata remains separate from row sharing.
Fixed SQL request tables receive file IDs in batches of 128 rather than altering every generated expression.
Compiled preparation templates survive file edits.
Final candidate SELECTs must be read-only; tests reject writes and check rollback.

Regex matches now persist by language, expression, and vocabulary term.
New predicates check current vocabulary; existing predicates check new terms.
Foreign keys remove matches when the last term occurrence disappears.
Empty completed predicates do not repeat the full vocabulary scan.
A rejected first implementation compiled a dynamic regex per term and increased positive cold CPU to 14.336 seconds.
The corrected callback caches one current predicate per SQL expression, and a regression requires two compilations for 200 evaluations.

### Rejected retention layouts

The standalone replay host imports production SQLite adapters, codecs, connection settings, and the same layout experiment used by native tests.
It compares normal production SQL after duplicate-expression removal, rather than the older uncompacted descriptions.
Each trial rotates layout order and requires exact candidate rows, including multiplicity.
Captures with equal file domains and revisions form one sequence, preserving reuse across real plan batches.

| Workload | Temporary stages | Retained postings | Retained shared sets |
| --- | ---: | ---: | ---: |
| Two rules, twelve files | 1.146 s | 1.501 s (+31.0%) | 1.164 s (+1.6%) |
| Three rules, twelve files | 1.948 s | 2.427 s (+24.6%) | 2.252 s (+15.6%) |

These are median replay thread CPU totals, not complete execution CPU.
They include setup, retained construction, writes, indexes, and consumers.
They exclude parsing, file publication, earlier structural preparation, and final matching.
The three-rule selection adds the quality rule to the positive control and preserves all 61 findings.
Its two file domains each execute two real plan batches.

Retained postings store 303,285 rows on the two-rule control with no reuse.
They store 410,038 rows and reuse 32 constructions across the three-rule batches.
Retained shared sets store 163,552 rows and reuse 38 constructions on that workload.
The reuse does not offset retained writes, wider keys, completion joins, and subsequent reads.
The engine keeps temporary physical boundaries for these sets.
The isolated host retains the experiments for reproduction; the engine has no selectable retention path.

An initial retained key placed revision between file ID and preorder.
SQLite then failed to use the ordinal suffix unless the join supplied revision at the right point.
The corrected primary key is `(relation_id,fid,pre)`, with revision in the payload and a cascading foreign key.
A 2,048-interval regression decreases range work from about 39.9 million to 79,000 VM steps.
Membership work decreases from about 16.9 million to 92,000 VM steps.
Both ordinal bounds and indexed membership are explicit test requirements.
The corrected layouts above still lose; the key correction does not justify promotion.

### Remaining frontier and reproduction

The final positive diagnostic executes 500 candidate data statements, including 480 physical materializations, versus 550 statements before this work.
Structural preparation decodes 340,517 nodes once, costs about 155 ms of diagnostic thread CPU, and performs no source-text extraction.
Selection performs no structural decoding.
Other candidate materializations still cost about 734 ms, temporary index construction 116 ms, and final candidate queries 250 ms.
These instrumented API timings must not be subtracted directly from untraced process CPU.
Separate matched SQL captures reduce total observed VM instructions from 141.6 million to 98.9 million.
Those captures include the new structural preparation statements.
The final ownership diagnostic reserves 16.97 MB in the main database, 12.13 MB in temporary pages, and 9.36 MB in WAL.
SQLite reports approximately 1.77 MB resident for that connection. These bytes do not represent total process memory.
The separate authored relational executor has not acquired a shared relation cache.

The external bottom-up sample still finds SQLite VM execution, B-tree comparison, and file I/O.
It does not identify the Rust structural decoder as the dominant remaining cost.
The next experiments should reduce demanded/intermediate row volume and consumer work before expanding another index or cache.
Their oracle must include full preparation and maintenance CPU.

Commands use `python3 scripts/profiling/run.py --deadline 115`; no command needs the earlier full-run exception.
Timing artifacts are `.rerail/profiles/sql-preparation/matched-{before,after}-{positive,quality}-{1,2,3}`.
Use `--recording` with the pinned selections in the evidence, `--trace off`, and `--allocator system-shim`.
Set `RERAIL_ENGINE_PATH` to the corresponding archived binary.
The [replay host instructions](../scripts/profiling/preparation/README.md) describe the three rotated storage trials.
Diagnostic and external sample artifacts are separate from all timing medians.
Discarded overlapping trials and build timeouts are not timing evidence.

The public workspace still requires rules before evaluation. New-plan reuse tests use the internal corpus API.
No public policy reload, durable cross-process cache, GPU implementation, or full-repository speed claim forms part of these results.

### Validation scope

The final native binary passes 1,223 focused DSL tests, including the new preparation and retention regressions.
The broader native run passes 1,520 tests and fails ten environment-dependent tests; 23 remain ignored.
Nine Git fixture tests use `git add .`, which the local staging guard rejects.
The allocator purge test requires jemalloc controls that the matched benchmark's system-malloc shim does not implement.
No failing SQL, DSL, or workspace assertion appears in that run.
Three separate bounded commands then pass the complete F811, F821, and F841 shortcut-parity fixture corpus.
The complete native run is therefore not all green.

The final addon also passes 320 public API tests, with two skipped tests.
Recorded source replay covers ten positive and ten clean calls, with zero diagnostic or execution-path differences.
The replay host's real-FTS smoke test checks empty sets, bag equality, and reuse.
Profiler tests, typechecking, repository structure checks, Windows path checks, formatting, and Clippy pass.
Clippy covers ordinary and profiling builds, plus the standalone replay host.
The evidence records each command, result, log digest, and parity scope.

## Value postings, vocabulary access, and shared SQL properties: 2026-10-21

The corpus now stores case-exact value postings and a persistent distinct-term vocabulary.
File publication creates the value index beside the positional kind index.
Replacement and deletion update both indexes in the same transaction.
Partition evaluation consumes prepared selections and does not rebuild either index.

The value layout follows the reference's term, posting, vocabulary, and coverage separation.
SQLite B-tree keys retain complete terms instead of using the reference's linear-hash buckets.
Each term, file, and kind has a delta-varint posting segment.
Full term keys avoid hash collisions and their source-confirmation path.
A small virtual table streams these packed ordinals. SQL owns filtering, joins, and matching.

Coverage includes every clean token, including keywords, numbers, and anonymous grammar tokens.
The clean-token limit remains 96 Unicode characters.
Non-clean leaf residue has a corpus-wide limit of 4,096 occurrences per language and kind.
A publication also limits residue text to 384 KiB per kind.
Exact uncovered ordinals retain confirmation for interiors and overflow without scanning every indexed leaf.

Literal predicates seek their terms. Regex predicates test distinct vocabulary terms once per batch.
The compiler shares vocabulary hits and restricts posting segments by known kind domains.
Bare kind domains use value postings directly. Other domains use indexed membership checks.
The kind index remains the reference's contentless positional FTS5 layout.

Shared decoded properties live in ordinary SQL columns, keyed by file and preorder.
Separate tables retain structural properties, span offsets, and first-child field lookups.
Batched fills decode missing keys and reuse the rows across later statements and batches.
File replacement invalidates these rows through foreign keys.
Source text is not retained for interior nodes.

The first implementation passed parity but regressed the positive control from 1.854 to 9.112 cold CPU seconds.
Instrumentation identified an affinity error in the new access path.
INTEGER bounds from decoded properties met temporary ordinal columns with empty affinity.
SQLite consequently omitted ordinal range constraints and repeatedly scanned complete file postings.
Two statements alone executed more than 660 million VM steps.

The compiler now gives materialized numeric columns explicit INTEGER affinity.
An isolated 2,048-interval case decreased from approximately 35,664,900 VM steps to 32,700.
The regression requires both ordinal bounds in the query plan and limits total VM work.
The affinity correction preserves the candidate and finding sets.

Further profiling identified repeated base-posting scans and tiny incremental BLOB reads.
The compiler materializes shared kind postings and enumerates unrestricted ordinals with the standard series virtual table.
Small record and span buffers amortize reads inside batched SQL decoding.
Decoded facts remain in SQL, and these transport buffers do not form another in-memory index.
The engine retains one execution path and introduces no runtime backend switches.

A live all-node property demand covers every subset demand for the same batch.
The compiler hoists that fill before its consumers and records file completeness in SQL.
Later batches skip completed files. Sparse demands retain sparse preparation.
We rejected an experiment that removed source materialization without this coverage proof after execution became substantially slower.

Three serial fresh-process trials per version give these median cold CPU totals:

| Workload | Before | After | Change |
| --- | ---: | ---: | ---: |
| Two rules, twelve files, 61 findings | 1.874 s | 2.504 s | +33.6% |
| Quality rule, five files | 3.004 s | 3.192 s | +6.2% |

Both controls preserve findings, multiplicity, and diagnostics.
The evidence records the matching workload and policy digests.
These complete-pipeline measurements include index publication and property construction.
They establish a remaining CPU regression, not a completed performance improvement.

The positive-control diagnostic retains all 11,763 candidate rows.
Text extractions decrease from 517,994 to 13,739.
Incremental BLOB reads decrease from 2,791,126 to 51,444.
Candidate VM steps increase from 111,766,482 to 130,976,638, despite fewer sorts.
The pipeline executes 550 SQL data statements instead of 72.

Shared property construction, temporary relation copying, and index construction now form the next performance frontier.
The external profiler runs separately with detailed tracing disabled.
Its two-second window contains substantial SQLite VM, comparison, and file-write activity.
Those samples identify execution paths. They are not an exact allocation of process CPU time.
Median recorded peak RSS decreases from 360.7 to 341.0 MiB on the positive control.
Host memory pressure can affect resident measurements, so this comparison does not establish an allocator-memory reduction.

Validation passed 1,404 native DSL/workspace tests, with 22 existing ignored tests.
The public native suites passed 320 tests, with two existing skips.
The source smoke replay passed ten positive and ten clean calls without differences.
Both profiling and production configurations passed Clippy. Repository checks and profiling-tool tests also passed.

Evidence: [value access and decoded properties](../gate/data-layer/fixtures/host/data-layer-port/profiling/value-access-20261021.json).

## Candidate SQL CPU root cause: 2026-10-21

The expensive path performs much more work than an index lookup.
The kind postings match `ast-index-spike`'s encoding, but the value access path is missing.
Rust retains per-kind value dictionaries and cached text selections.
The reference spike persists value postings in `vidx` and a distinct-term vocabulary in `vlex`.
It evaluates text predicates against vocabulary terms and seeks their postings.
Our SQL predicates instead decode and test individual occurrences.

The reference implements coverage checks and bounded residue handling for indexed values.
Those checks keep incomplete vocabulary coverage from producing false absence results.
The present investigation identifies this missing performance functionality; it does not implement the value index.
The comparison measures complete implementations with different available access paths.
It establishes no inherent database-engine disadvantage.

The positive control runs two data-layer rules over twelve files and retains all 61 findings.
Its candidate pipeline originally executes 164,171,085 SQLite VM steps and 2,174 sorts.
One final statement contains 710 CTEs and executes 536 sorts.
Most generated relations explicitly request materialization.
Repeated kind restrictions create additional set intersections and sorting.
The same low-cost base CTEs also occur in multiple physical statements.

For example, one generated chain computes `s162 = s160 INTERSECT s5`, then intersects that result with `s5` twice more.
The last two intersections add no restriction.
The compiler now reuses identical generated relations and canonical intersections.
It preserves conservative exactness metadata separately from shared row sets.
A regression also corrects subtraction of a set from itself under nested negation.
Another regression preserves deduplication when a union receives projected duplicate rows.

| Candidate work | Before | After SQL cleanup |
| --- | ---: | ---: |
| SQL data statements | 82 | 72 |
| VM steps | 164,171,085 | 111,766,482 |
| Sorts | 2,174 | 1,292 |
| Candidate rows returned | 11,763 | 11,763 |
| BLOB reads | 2,858,925 | 2,791,126 |
| Text extractions | 519,033 | 517,994 |

The cleanup removes approximately 32% of VM steps and 41% of sorts.
Most text decoding remains.
The original reads transfer 102,258,042 bytes through SQLite's BLOB API.
Repeated packed-span blocks account for 82,960,502 bytes.
Source text accounts for 8,637,096 bytes.
The pipeline indexes 340,517 AST nodes once.

New API-boundary timers separate lowering, SQL preparation, materialization, indexing, result execution, and cleanup.
A diagnostic before cleanup attributes 1.919 CPU seconds to materialization and 0.917 seconds to result execution.
SQL preparation takes 0.035 seconds; temporary-index construction takes 0.024 seconds.
Lowering takes another 0.054 seconds.
These diagnostic figures use an engine build at optimization level zero, with unchanged SQLite dependencies.
They describe attribution; production comparisons use optimization level one on both sides.

An external two-second sample captures 1,569 active native samples.
FTS-related frames occur in 29 samples; packed-BLOB decoding occurs in 219.
Sorting and record-comparison paths occur in 278.
The remaining samples include VM dispatch and generic B-tree work.
These mutually exclusive categories describe one sampling window, not an exact process CPU partition.

The existing SQL callback clocks also contain a measurement error.
An internal FTS cursor remains open while other statements execute.
Its accumulated lifetime reports 14.5 CPU seconds during a cold phase that uses only 2.547 CPU seconds.
That lifetime is not an exclusive index-execution cost.
The report now withholds CPU priorities when statement lifetimes cannot reconcile with the apply CPU budget.
Direct API timers provide the execution breakdown instead.

Three fresh production trials compare each implementation on each workload.
These serial runs alternate version order and pass identical workload and policy fingerprint checks.
All trials retain exact parity.

| Workload | Before cold CPU | After cold CPU | CPU change | Before peak RSS | After peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Two rules, twelve positive files | 2.216 s | 1.783 s | -19.5% | 392.44 MiB | 410.00 MiB |
| Quality rule, five files | 2.948 s | 2.984 s | +1.2% | 602.62 MiB | 611.89 MiB |

The quality control does not show a meaningful CPU change.
Neither control establishes a memory improvement.
The positive control confirms a useful SQL cleanup, while its remaining text-decoding work remains substantial.

The `serverplan_target_architecture` port provides a concrete value-index case.
Its AST branches repeatedly match identifier text such as `^(?:step_tags)$`.
The current path reads kind postings and decodes text to evaluate those predicates.
The reference value index can select matching vocabulary terms before locating occurrences.
Porting that access path is the next priority.
Sharing demanded decoded properties across SQL statements is the next remedy for remaining scalar-read amplification.

The final code retains batched preparation and one execution path.
It adds no optional execution backend.
The [root-cause evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/index-root-cause-20261021.json) records source identities, reference files, traces, counters, experiments, and validation.

Validation passes 1,202 DSL tests and 194 workspace tests, with 22 existing native skips.
The public suites pass 320 tests, with two existing skips.
The recorded linter source corpus passes ten positive calls and ten clean calls.
Profiling tests, typechecking, repository checks, and Clippy pass.
Several bounded build attempts reach the deadline before a later attempt completes.
The evidence records those attempts separately from completed checks.

## Repeated index comparison: 2026-10-21

Three fresh trials compare each implementation on both bounded workloads.
Each trial runs alone, with identical source bytes, policies, ingestion batches, and allocator settings.
The second round reverses the order; the third rotates it.
The figures report median cold process CPU and median process peak RSS.
Compilation CPU is excluded.
All 24 trials pass parity.

| Implementation | Positive control CPU | Positive peak RSS | Quality control CPU | Quality peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Rust index | 0.462 s | 305.12 MiB | 2.648 s | 497.36 MiB |
| SQL, index rebuilt per partition | 2.339 s | 303.06 MiB | 3.196 s | 496.27 MiB |
| SQL, shared corpus with individual queries | 2.633 s | 301.22 MiB | 3.025 s | 492.66 MiB |
| SQL, current batched candidates | 2.136 s | 399.39 MiB | 2.956 s | 614.05 MiB |

The positive control runs two rules over twelve files and retains all 61 findings.
The quality control runs one rule over five files and expects no findings.
The percentages compare current batched SQL with each reference.
Positive percentages mean more resource use; negative percentages mean less.

| Reference | Positive CPU over/under | Positive RSS over/under | Quality CPU over/under | Quality RSS over/under |
| --- | ---: | ---: | ---: | ---: |
| Rust index | +362.9% | +30.9% | +11.6% | +23.5% |
| SQL, index rebuilt per partition | -8.6% | +31.8% | -7.5% | +23.7% |
| SQL, shared corpus with individual queries | -18.9% | +32.6% | -2.3% | +24.6% |

Current SQL remains substantially more expensive than Rust on the positive control.
Batching reduces CPU against the earlier SQL implementations, but increases memory.
The quality difference against shared-corpus SQL is too small for a strong speedup claim.
These repeated comparisons replace the earlier single-run comparison for these small workloads.

The Rust binary retains its earlier SQLite page-cache compile option.
The comparison covers complete implementations and does not isolate the candidate index alone.
The ten-rule batched control has no completed result within the two-minute cap.
It therefore has no valid total CPU over/under result.

The [comparison evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/index-comparison-20261021.json) records every trial, binary identity, fingerprint, metric, and percentage.

## Batch candidate preparation: 2026-10-21

The corpus index previously retained postings but queried them separately for each file and rule.
That query boundary defeated batching.
The engine now prepares candidate sets before matching starts.
Matching and relational partitions consume those sets without issuing candidate SQL.

The preparation API accepts all rule plans together.
Each SQL batch covers up to 32 plans and 16,384 files of one language.
The existing ingestion limits determine each incoming file group.
One workspace corpus retains the prepared sets across partitions and revisions.
A source or grammar change invalidates that file's sets.
An unchanged request issues no candidate SQL.

The index still uses bare positional FTS5 kind tokens and the tape columns from `ast-index-spike`.
Publication uses existing corpus metadata without per-file identity SELECTs.
SQL executes the candidate predicates.
The transport stores empty, complete, sparse, or bitmap selections.
These representations compress results without implementing another rule executor.
The preparation barrier carries parsed documents, line tables, metrics, and work budgets into matching.

Compiler cleanup removes dependencies that discarded rule branches no longer use.
Each materialization statement also contains only its required CTEs.
An interrupted batch publishes no partial absence result and can retry.
Missing prepared results retain authoritative matching and increment an explicit counter.

`anchorBatches` counts preparation batches.
`anchorSqlDataStatements` counts their final SELECTs and temporary-table materializations.
`anchorSqlMaterializations` reports the latter separately.
These counts exclude schema operations and SQLite's internal FTS statements.
`anchorCacheMisses` and `anchorBatchFailures` expose incomplete preparation.
`peakPreparedCandidateBytes` estimates compact result payloads; it excludes maps, documents, and SQLite memory.

A regression compares one file with 64 files under the same rule plans.
Both workloads issue the same number of SQL data statements.
Three repeated consumption passes issue none.
Another regression prepares 65 plans across 64 files in three batches and reuses every result.
Budget, retry, and discarded-dependency tests also pass.

The final measurements and binary identity appear in the [batch evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-batch-candidates-20261021.json).
The two-rule positive control contains all twelve source files and retains all 61 expected findings.
It has no false positives, false negatives, or engine failures.
The five-file quality control also checks unchanged and edited revisions.

| Workload | Previous cold CPU | Batched cold CPU | Previous peak RSS | Batched peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Two rules, twelve positive files | 2.620 s | 2.198 s | 298.39 MiB | 376.33 MiB |
| Quality rule, five files | 2.606 s | 3.023 s | 488.83 MiB | 614.84 MiB |

The positive control uses two batches containing 82 SQL data statements, including 80 materializations.
Its relational consumption pass issues zero candidate SQL statements.
Selection uses 1.783 CPU seconds, compared with 2.212 seconds previously.
The quality control uses one batch containing nine data statements.
Both controls report zero failed batches and zero missing prepared selections.

The quality control slows down, and both controls use more peak memory.
Batching therefore does not resolve the overall CPU and memory problem.
An incremental quality run publishes files in counts of five, zero, zero, and one across its four applies.
Its batch counts are one, zero, zero, and one.
The unchanged revision executes no candidate data statements; the edited revision executes nine.
The compact result payload peaks at 47,052 bytes for the positive control and 381,152 bytes for the quality control.

The measured final binary SHA-256 is `03125d5d0097443a55f2825f65074439951f2f295b12f5d07dde020e44ce6646`.

These are single runs on a shared host, with matching workload and policy digests.
Both controls use the same system allocator shim and native build optimization.
The larger ten-rule control reaches the 112-second deadline in the recorded intermediate implementations.
Those timeouts provide no completed CPU result and support no full-workload speedup claim.
The final source differs from the last timeout only in counters and error handling.

The external sample covers an initial batched implementation before dependency pruning.
SQLite accounts for 98.9% of its active native samples.
Sorting, record comparisons, memory copies, and temporary-file writes appear among its expensive operations.
This sample motivates the compiler cleanup; it does not establish the final distribution.

Large SQL batches and broad preparation remain performance problems.
The current inventory prepares the full language-specific plan group across published files.
More selective rule/file demand and intermediate SQL lifetime require further measurement.

Validation passes 1,198 DSL tests, 194 workspace tests, and 320 public tests.
The suites retain 22 native skips and two public skips.
The recorded linter source corpus passes ten positive calls and ten clean calls.
Typechecking, repository checks, profiling tests, and Clippy also pass.
The previously documented Git-fixture and allocator-probe environment limits remain outside these checks.

## Corpus index lifetime correction: 2026-10-21

The previous adapter rebuilt a temporary SQL index for each file partition.
That lifetime defeated index reuse and invalidated the architectural comparison with Rust.
The measurements below remain historical evidence of that rejected adapter.
They do not compare equivalent index lifetimes.

The workspace now owns one file-backed corpus database.
A file's source digest and grammar determine whether its postings need replacement.
Unchanged documents reuse their existing rows, including after tape restoration and document-cache eviction.
Relational preparation publishes missing files before processing partitions.
File removal deletes the corresponding rows.

Workers lease separate connections to the same database.
Each connection has independent temporary SQL tables and a bounded page cache.
WAL permits publication while other workers read coherent snapshots.
The workspace owns the database through the final connection lease.

Kind counts stored by file support replacement and deletion without scanning the corpus vocabulary.
The index reuses the validated tape's source digest.
Its identity check does not export the tape or hash the source.
Unchanged sources also retain their postings through context changes and retries after interrupted rules.
The database survives workspace revisions.
It does not survive process restarts.

`indexedFiles` and `indexedNodes` now count committed publications.
`anchorIndexViews` and `anchorIndexViewNodes` count logical readers, including reused entries.
`sqlIndexWrites`, `sqlIndexWrittenNodes`, and `sqlIndexReuses` make the distinction explicit.
Preparation and selection retain separate CPU timers.

The twelve-file control publishes 12 files and 379,809 nodes once.
Its 76 later partition lookups publish nothing.
The rejected adapter constructed 88 indexes containing 3,846,259 nodes across those same requests.
Preparation falls from 0.677 to 0.062 CPU seconds.

| Workload | Rust cold CPU | Corpus SQL cold CPU | Rust peak RSS | Corpus SQL peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Ten rules, 12 positive files | 32.436 s | 122.420 s | 2.474 GiB | 2.514 GiB |
| Quality rule, five files | 2.631 s | 2.606 s | 497.80 MiB | 488.83 MiB |

The positive control retains all 61 findings without false positives, false negatives, or engine failures.
These are matched single runs on a shared host, with identical policy and workload digests.
Both binaries use the same system allocator shim and build optimization.
The saved Rust binary retains its earlier SQLite page-cache compile option.
The comparison covers the complete branch change.
The five-file CPU difference is too small to support a speedup claim.

A separate five-file incremental probe publishes five files initially, zero for an unchanged request, and one for an edit.
The unchanged request uses 0.002788 CPU seconds.
The edit uses 0.816207 CPU seconds.
Parity remains clean through both requests.

The corrected index lifetime does not resolve expensive SQL selection.
Selection uses 93.322 CPU seconds, against 3.722 seconds in the saved Rust control.
The same run records 1,258 compiler broadenings and 8,990,941 candidate root pairs.
File filtering also has an access-path problem: SQLite's vocabulary table seeks by term, then tests the file identifier.
The bundled SQLite implementation exposes no document constraint in `fts5VocabBestIndexMethod`.

A standalone diagnostic keeps 100 requested postings constant and adds 32 unrelated files.
The query grows from approximately 700 to 160,700 SQLite VM steps while returning the same 100 rows.
That diagnostic uses SQLite 3.54.1.
The engine bundles 3.53.2 and has the same constraint limitation.
It establishes an access-path limitation, rather than attributing the full timing difference to one cause.
Corpus batching and efficient file-constrained posting access remain the next query issues.

Validation passes 194 workspace tests, 1,194 other DSL tests, and 320 public tests.
The native DSL suite retains 22 existing skips.
Public tests retain two.
The recorded source corpus passes ten positive and ten clean calls.
Clippy passes with profiling enabled and disabled.
Typechecking, repository checks, and profiling tests also pass.

Broader native checks pass another 103 tests and encounter ten environment limits.
The local Git safety wrapper blocks nine fixtures that invoke `git add .`.
Its stated reason is that mass staging can include unrelated files.
The allocator shim cannot run the jemalloc arena probe.
These unavailable checks are recorded separately from passing tests.

Workspace tests cover unchanged requests, retries, edits, deletion, and independent workspaces.
SQL tests cover shared connections, snapshot consistency, and cleanup after the final reader.
Broader validation exposed a missing budget poll on empty candidate results.
Selection now observes exhaustion before and after SQL, preserving clause failure and eviction behavior.
A budget fixture contains enough roots to exhaust its allowance after shared selection.

The saved Rust baseline also reproduces an existing Python NFKC false negative.
The semantic name prefilter now retains non-ASCII names for authoritative resolution.
The existing identifier test passes after that correction.

The [corpus evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-corpus-index-20261021.json) records source hashes, binary identities, controls, lifecycle counters, validation, and diagnostic bytecode.
The measured binary SHA-256 is `b8347b9f50101a7a9c2a089840c9eb806365d020e5509405c8193734bbea7f46`.
Every command remains below the two-minute limit.
Bounded runs stop after 112 seconds.

## SQLite positional kind anchors: 2026-10-21, after `5616076a`

**Use the positional FTS5 kind index from `ast-index-spike`.**
The implementation follows `bench/tape_store.py::_build_kind_fts` and its thin file-tape columns.
Each file has one contentless FTS5 row.
Each preorder node contributes one bare `k<base36(language * 1024 + kind)>` token, including anonymous nodes.
The FTS5 instance vocabulary returns file identifiers and preorder positions.

The schema enables contentless deletion for file replacement and removal.
It adds no field tokens or persistent row per AST node.

The database uses a real temporary file and a 1 MiB main page-cache target.
Temporary SQL storage uses files, the temporary page-cache target is 512 KiB, and mmap is disabled.
A storage test writes a 4,816,896-byte database.
SQLite reports 1,114,704 bytes for its page cache, schema, and prepared statements after those writes.
These counters exclude source tapes, allocator metadata, operating-system file caches, and query result arrays.
The cache setting is a target, not a limit on total process memory.

SQL selects candidates through kind postings, text predicates, field constraints, and parent relationships.
The existing matcher constructs captures and checks complete rule semantics.
Sibling constraints currently retain conservative candidate sets.

SQL combines language and path filters with candidate selection across indexed files.
An authorization test forbids tape-column reads during kind and path filtering.
Existing repository scope matching retains its current glob semantics.

This rejected adapter created an index for each file partition.
The storage API supported multiple files, replacement, deletion, and reopen.
The correction above replaces that ownership model.

The previous Rust candidate index remains a test oracle and saved benchmark binary.
The production path has no backend selection flag.
SQLite connections use the existing SQL profiler.

### Rejected trials and corrections

The first SQL trial passed the five-file parity probe but repeatedly loaded complete tape BLOBs.
It used 16.005 cold CPU seconds, against 2.631–2.848 seconds for Rust.
An external sample attributed 76.33% of active samples to `pread` and 12.23% to memory copying.
Incremental BLOB reads reduced this trial to 3.867 cold CPU seconds.
That trial used 468.73 MiB peak RSS, against 497.16–497.80 MiB for Rust.

The positive workload then exposed SQLite's CTE reference expansion during preparation.
The trial logged 178 table-reference-limit errors and reached the 112-second harness deadline.
It is not a passing performance result.
The compiler now places SQL temporary tables at shared dependencies with large expansion estimates.
These physical boundaries prevent repeated dependency expansion.

Temporary indexes support file and preorder lookups within those candidate sets.
Small dependencies remain in one SQL statement.
Ancestor selection also shares ancestor edges across matching descendants.

Incremental readers close before SQL schema changes and file updates.
Tests cover overflow records, span checkpoints, stale cursors, and cancellation.
A node-text lookup reads fewer than 256 bytes from a file with more than 65,535 nodes.
A dependency regression executes a 20-level shared graph twice and checks temporary-table cleanup.

A second positive trial avoided the CTE reference errors but still reached the harness deadline.
An external sample attributed 44.84% of active samples to mutex wakeups during SQLite page-cache access.
The bundled `SQLITE_ENABLE_MEMORY_MANAGEMENT` option forces all connections into one shared page-cache group.
The build now disables that option and retains separate bounded page caches.
Workspace eviction already closes idle content-store connections and releases their pages.
The obsolete global page-cache release call is removed.

The BLOB-handle map also uses the engine's existing fast hasher.

Direct field relations now select their structural context before inner text and pattern predicates.
The child accessor skips subtrees in the stored tape and returns the first requested field edge.
It retains no Rust posting index.
This follows the spike's child-stepping approach and the prior Rust selector's anchor discipline.
A node-budget regression also preserves exhaustion when the index refuses an oversized file.
The two public exhaustion tests pass after that correction.

The next positive trial still exceeded the deadline during candidate selection.
Pattern plans repeatedly scanned postings for each candidate root.
Their anchor estimates also counted kinds instead of the selected literal postings.
Pattern plans now use actual posting counts and indexed SQL temporary tables for subtree probes.
A materialized span relation computes each candidate's subtree end once.
A regression bounds SQLite VM steps for 2,001 similar calls with one rare target.

The remaining ancestor walks still repeated work for deeply nested AST nodes.
AST subtree ordinals form intervals, so descendant predicates now use indexed SQL range queries.
Inside predicates remove nested intervals with a SQL window before probing candidate positions.
Field predicates use the first field child's interval.
Direct parent predicates retain one parent lookup.
General graph reachability retains its existing recursive SQL.

A second VM-step regression covers 200 nested calls, including field relations.

### Rejected lifetime comparison and query observations

The first completed positive control retains all 61 expected findings and reports no engine failures.
Its cold CPU time is 113.949 seconds, against 32.436 seconds for the saved Rust build.
Peak RSS is 2.507 GiB, against 2.474 GiB for Rust.
These controls use identical source bytes, policies, ingestion batches, build optimization, and allocator settings.
The source selection contains 12 files and ten rules.
They are single runs on a shared host.

The final rebuild retains the same algorithm and adds profiling counters and configuration guards.
Its matched controls are:

| Workload | Rust cold CPU | SQL cold CPU | Rust peak RSS | SQL peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Ten rules, 12 positive files | 32.436 s | 105.663 s | 2.474 GiB | 2.546 GiB |
| Quality rule, five files | 2.631 s | 3.113 s | 497.80 MiB | 486.13 MiB |

The positive control retains all 61 findings without false positives, false negatives, or engine failures.
An earlier Rust five-file control used 2.848 CPU seconds.
Both controls finish within the two-minute command limit.
The SQL build also disables SQLite's global page-cache group, while the saved Rust control retains it.
The comparison covers the complete branch change and does not isolate index storage alone.
The [evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-ast-index-20261021.json) records binary identities, source hashes, rejected trials, external samples, and passing checks.

This SQL implementation is a storage and comparison baseline, not a performance improvement.
Candidate selection consumes 81.422 CPU seconds, against 3.722 seconds for Rust.
Initial ingestion selects 5,176,955 root pairs, against 1,604,003 for Rust.

The next frontier is SQL candidate-plan construction, shared materialization, and lost selectivity.
The profiler now counts compiler plan broadenings separately from incomplete indexes and query fallbacks.
The positive control records 1,258 compiler broadenings.

The compiler currently broadens plans that exceed its CTE bound.
This behavior preserves findings but weakens filtering.

Validation includes 86 native tests and 320 public tests, with two existing skips.
The recorded source corpus includes ten positive cases and ten clean cases.
These cases preserve findings and execution-path parity.
Clippy passes with profiling enabled and disabled.
Typechecking and repository checks also pass.

The existing projected-row queue remains unchanged.
Moving the kind index to disk does not remove that queue.
Persistent corpus indexing, SQL scope integration, and the spike's later value-posting indexes remain separate work.

## Field-aware anchors and shared input indexes: 2026-10-21, after `a92c126a`

**Keep selective anchors and share one index across each file's relational inputs.**
The [evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/anchor-execution-20261021.json) records the controls, rejected trial, navigation counters, external samples, and tests.

The implementation uses the existing kind and value postings, plus sparse postings for requested AST fields.
Each field posting identifies the first child in that field.
Stored parent links support reverse navigation from those children.
Utility plans retain their relation constraints.
File-local fact extraction now uses these selections before the matcher constructs captures and projected rows.
Kind-only queries continue to use kind postings directly.

The tape records first-field occupancy during restoration.
This marker occupies existing padding: each navigation record remains 40 bytes.
Ancestor checks read the known child edge directly and preserve first-child behavior for repeated fields.
Nested relations, stop rules, negation, damaged trees, and capture order retain their existing semantics.

These candidate indexes remain in Rust because the existing AST matcher consumes the tape before SQL input construction.
SQL continues to use indexed syntax virtual tables for kind, parent, identity, ownership, and byte ranges.
Their existing field-access index remains available to SQL projections.
This change adds no execution backend or configuration switch.
Moving AST candidate selection into SQL requires a separate integration with that syntax source.

### Matched controls

Both builds use engine optimization level 1, dependency level 3, no cross-crate LTO, 256 codegen units, and the system-malloc shim.
The baseline build comes from a clean `a92c126a` worktree.
The source pin, source hashes, rule definitions, ingestion size, and expected findings remain identical.
These builds differ from the earlier thin-LTO reference.

| Workload | Before CPU | After CPU | Before peak RSS | After peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Ten rules, 12 files, tracing off | 38.155 s | 32.436 s | 2.564 GiB | 2.474 GiB |
| Quality rule, 64 files, tracing on | 9.184 s | 6.947 s | 656.69 MiB | 639.17 MiB |

The positive workload retains all 61 expected source findings with no engine failures.
The quality workload retains zero findings and no failures.
The CPU reductions are 15.0% and 24.4%, respectively.
The table uses the final rebuilt binary.
An earlier run of the retained algorithm measured 33.464 seconds and 7.735 seconds, respectively.
The evidence preserves both runs and their binary identities.
The baseline has one control run on the shared host.
The positive workload's elapsed times range from 48.831 to 65.662 seconds, against the 51.631-second baseline.
These comparisons use process CPU and do not establish an elapsed-time improvement.
No new full-repository or production-build result is claimed.

### Navigation and index construction

| Quality probe counter | Before | After |
| --- | ---: | ---: |
| Field lookups | 16,140,707 | 614,645 |
| Child examinations | 427,807,885 | 1,403,498 |
| Child examinations in failed lookups | 423,856,845 | 808,916 |
| Direct checks of known field edges | 0 | 2,050,091 |

Child examinations decrease by 99.67%.
The first implementation rebuilt the structural index for each input.
It performed 695 builds over 31,441,607 nodes during positive-workload preparation.
One state-flow clause then exhausted its work budget, despite the correct finding count.
That run is rejected as a passing benchmark.

The final implementation builds 76 indexes over 3,466,450 nodes for the same input partitions.
Each batch shares kind, value, and field postings across its input plans.
Selections retain compact ordinal arrays after the shared index is released.
The selection cache uses the compiled plan and current source identity.
A source change clears the cache, and the evaluation scope releases it.
The original work limits remain unchanged.
A regression test requires two distinct inputs to share exactly one index and preserve separate selections.

### Remaining costs and validation

The final diagnostic retains 80 queued jobs, 577,199 projected rows, and 9,833,801 JSON fields.
Its referenced row-storage estimate remains 1,256,398,807 bytes before SQL admission.
The representation problem from the previous entry remains.

External samples show allocation and matcher overhead during preparation.
Two later sampling windows contain 81.4% and 99.7% SQLite samples.
Their leaf functions include record comparisons, B-tree lookups, memory copies, and sorting.
Those percentages describe sampling windows, not total execution CPU.
The diagnostic attributes 22.052 seconds of CPU to the SQL batch and 5.785 seconds to input preparation.
The largest descendant materializations and their indexes remain substantial costs.

Validation covers 69 focused Rust tests, 320 public native tests, and 17 profiling tests.
Two existing public tests remain skipped by their existing gates.
The source-corpus sample covers one positive and one clean observation for each of the ten rules.
All 40 stateless and workspace evaluations pass without differences.
TypeScript, repository checks, formatting, and Clippy run separately from the timed controls.
Every command retains a deadline shorter than two minutes.

## CPU and memory attribution: 2026-10-21, after `4def200d`

**The remaining problem is large fact construction, repeated AST navigation, and large SQL intermediates.**
The earlier 1,114.637-second reference also contains substantial profiling work.
Removing that measurement flaw does not remove the engine's preparation costs.
The [evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/cpu-memory-attribution-20261021.json) records controls, ownership counts, samples, and validation.
The [profiling guide](code-rule-profiling.md#measure-cpu-and-memory) describes the new tools.

### Matched Python comparison

Both engines evaluate the same ten sixth-batch rules on 12 files containing 1,984,216 source bytes.
The source pin remains `2b1504adc4291daaf080b2e932962b134da1a69a`.
Both produce the same 61 source findings, including multiplicity.
Thirty findings report required files absent from this restricted snapshot; 31 are detections from the repository recording.

| Control | Analysis CPU | Peak process RSS |
| --- | ---: | ---: |
| Upstream shared Python fact engine, one worker | **4.320 s** | **196.70 MiB** |
| Rerail, automatic residency scans removed | **37.237 s** | **1.681 GiB** |
| Rerail / Python | **8.62×** | **8.75×** |

Native CPU excludes 0.714 seconds of policy compilation.
Python CPU includes `run_plugins`, waited-for child CPU, and the source comparison adapter.
Peak RSS includes process setup for both engines.
The native reference uses engine optimization level 1, dependency level 3, thin LTO, 256 codegen units, and the system-malloc shim.
It is not a production optimization-level-3 benchmark.
Tracing, SQL capture, external sampling, and explicit censuses are off for both timing controls.
The profiling feature remains compiled into the native reference, with aggregate CPU accounting enabled.

The native driver attributes 7.558 seconds to ingestion and 29.675 seconds to global preparation and SQL.
These are disjoint process CPU measurements from the completed control.
The host is heavily shared; CPU comparisons are more useful than elapsed-time comparisons here.
All new commands remain bounded below two minutes.
Larger captures and a later twelve-file repetition hit their deadline and remain partial evidence, not completed benchmarks.

The initial subset incorrectly filters expectations from the full repository.
That loses the 30 required-file findings and marks correct detections as extra findings.
The corrected upstream snapshot oracle retains those findings.
The cross-runtime tool verifies file hashes and reconstructs the timed run's complete finding multiset from its original expectations and differences.
The resulting comparison passes; the original timing report remains unchanged.
Generated-file freshness remains a separately recorded upstream integration contract, as established in the sixth-batch port.
The restricted state oracle uses unchanged upstream source conversion and signed allowlist functions.
It preserves the freshness reporter observation instead of counting it as a native source finding.

### The queue exists before memory admission

A diagnostic capture reaches SQL admission with 80 jobs and 577,199 projected rows already constructed.
Those rows contain 9,833,801 JSON object fields.
Their referenced heap estimate is **1,256,398,807 bytes**, excluding supplied syntax, snapshots, and allocator overhead.
This estimate is not an exclusive RSS measurement.
The native timing control peaks at 1,805,041,664 RSS bytes.

| Consumer | Queued rows |
| --- | ---: |
| State flow | 196,089 |
| Legacy execution graphs | 162,195 |
| State signatures | 118,589 |
| Remaining consumers | 100,326 |

The first three consumers account for 82.6 percent of rows.
Projected field names alone contain 103,706,252 bytes; projected string values contain another 82,247,225 bytes.
Maps, row metadata, capture spans, paths, and allocation capacity dominate the representation.
All 577,199 projection references identify different row objects within the queue.
Shared SQL stages and the packed producer arena do not eliminate these upstream objects.

The 256 MiB SQL admission target starts after this queue exists.
It therefore cannot bound this preparation peak.
The 64 MiB document cache also cannot release syntax objects retained by queued jobs.
On the separate 64-file quality capture, the queue references 57 syntax objects with a 247,446,912-byte cache estimate.
That estimate can overlap document storage and uses the existing syntax-cache heuristic.
It is not another independent RSS category.

### Field navigation performs hundreds of millions of scans

The 64-file quality capture records:

- 16,140,707 field lookups.
- 427,807,885 child examinations.
- 423,856,845 examinations inside failed lookups: **99.08 percent**.
- A maximum scan of 400 children for one lookup.

`TapeNode::child_by_field_id` searches the sibling list for every request.
Many requests name fields absent from that node.
Two external ingestion windows attribute 43.0 and 63.7 percent of self samples to this function.
These percentages describe sample windows, not the entire run.
The broader source capture also shows repeated anchor selection, sorting, projection, and allocation work during input construction.
Python's fact engine shares one AST traversal across collectors and accesses AST fields directly.

The next navigation change needs an indexed field lookup or a grammar-aware rejection before scanning children.
It must preserve first-child order, repeated fields, aliases, anonymous nodes, and existing tape parity.
The ownership postings fixed earlier are a separate interface; these measurements concern the AST matcher adapter.

### Profiling was adding quadratic work

The previous profiling feature serializes all resident witness payloads after every ingestion request, even with tracing disabled.
The large reference has 288 ingestion revisions.
Its retained-result estimates sum to approximately 316 GB across those repeated scans.
That sum is neither resident memory nor an exact serialized-byte count.
An earlier external sample places 74 of 96 active native samples inside this accounting routine.
The old 1,114.637-second result consequently cannot establish clean production execution time.

Automatic residency scans are removed.
An explicit census now counts shared projections once and reports its own CPU cost.
The normal apply path no longer performs a complete residency traversal for profiling.
On the small 64-file control, cold CPU decreases from 7.630 to 7.245 seconds.
This single shared-host comparison establishes neither a repository speedup nor the full profiling overhead.

### SQL remains a substantial second cost

The completed twelve-file control spends 29.675 seconds in global preparation and SQL together.
The separate diagnostic records 7.797 seconds of queue preparation, including its ownership census.
These measurements use different observation modes and must not be subtracted to estimate SQL CPU.
Its largest SQL stages include state and legacy descendant tables and their indexes.
One state-signature descendant materialization alone executes 23,860,081 VM instructions in an observed partition.
The detailed evidence ranks the measured stages and preserves their rule, clause, and file context.

The small forest kernel optimized earlier does not account for these broad input tables or repeated descendant materializations.
Optimizing that kernel again would leave the main allocation and preparation costs intact.

### Required implementation order

1. Queue versioned descriptors before construction; admit capture and syntax leases within the same memory boundary as SQL consumers.
2. Build shared capture columns per file version, with row-selection views and a union of demanded columns across consumers.
3. Keep source text as bounded references until SQL requests it; avoid repeated field-name strings and unused capture-span maps.
4. Index AST field access and reject impossible fields before sibling scans.
5. Reduce demanded descendant roots and repeated SQL intermediates after the input representation changes.

Admission must retain broad producer waves and the existing shared pass allowance.
SQL continues to own rule semantics.
These requirements do not justify a rule-specific Rust evaluator or another optional execution backend.
No Linear tickets are created.

### Validation

The focused native suites pass 67 distinct checks, including shared projection accounting through edits, removal, and workspace closure.
The mandatory producer-shape gate passes all five checks.
Twenty recorded source observations cover one positive and one clean case for each linter.
All twenty match on both stateless and workspace execution, with zero engine failures or finding differences.
The complete profiling checks, typecheck, repository checks, formatting, and production and profiling Clippy checks pass.

Two bounded attempts to build the full Rust unit binary stop before test execution.
The memory invariant instead has a public binding regression, which verifies actual evidence sharing and invalidation.
No Rust unit execution pass or new full-repository benchmark is claimed.

## Batch six diagnosis: 2026-10-21, after `71ea1606`

**All 60 rules now pass together over 9,021 files: 24,993 findings and zero engine failures.**
Cold, unchanged-file, and trailing-newline edit comparisons have zero missing or extra findings.
The [diagnosis evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/batch-six-root-cause-20261021.json) contains reports, source hashes, test receipts, SQL counters, and external samples.

Both combined runs use the same 137,924,324-byte pinned snapshot and reference harness.
CPU includes process user and system time, including the profiling build's residency accounting.
Peak RSS covers the complete process, including incremental checks.

| Combined 60-rule run | Cold CPU | Cold elapsed | Unchanged CPU | Edit CPU | Peak RSS | Result |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| After capacity admission, before remaining SQL fixes | 1,076.563 s | 1,054.093 s | 2.102 s | 53.083 s | 28.12 GiB | Fail: conditional budget exhaustion |
| Final | **1,114.637 s** | 1,277.989 s | 2.109 s | 63.196 s | **18.30 GiB** | **Pass: 24,993 findings** |

Final compilation uses 2.163 s CPU, separately from cold evaluation.
The final run completes analysis that the preceding run interrupts. These results do not establish a passing CPU speedup.
The observed memory peak decreases, but full input preparation remains outside admission and still needs architectural work.
The historical 951-second passing run covers only ten rules and records no CPU time.

The preceding measurements are committed and pushed in `71ea1606`.
This investigation identifies two independent causes of the 890 repository failures.
The fixes retain SQLite evaluation, existing limits, and one production execution path.
The [execution contract](gpu-producer-execution-contract.md) describes capacity admission and its memory boundaries.

### Shared producer capacity and eager SQL preparation

The previous driver creates every SQL session before it advances any consumer.
Each session retains its input tables and intermediate state while the driver prepares its peers.
That SQLite storage is outside the producer's 64 MiB packed-input arena.
Once aggregate producer input exceeds capacity, preparation rejects every ready relation.
This explains the 889 identical `quality-contracts-1` failures.
It also permits unbounded suspended SQL storage, but the preceding 21.33 GiB process peak does not isolate that storage from prepared inputs.

The driver now prepares consumers as capacity permits.
Admission counts shared packed columns once and checks SQLite cache, schema, and statement storage through its 64-bit status API.
SQLite admission measures storage at the first producer suspension; later SQL stages can grow it.
Ready consumers share broad producer waves within each capacity partition.
Finished consumers close before admission continues.
The driver preserves original result identities and keeps one 100-pass allowance across all capacity partitions.
It does not reset work limits when it releases physical storage.

The SQLite admission target is 256 MiB, with one prepared candidate retained while the preceding partition drains.
A single consumer can exceed that target under its existing work and output limits.
Projected evidence, prepared syntax facts, publication indexes, and document caches are outside this target.
It therefore does not bound process RSS.
The producer retains its 64 MiB input and 256 MiB numeric-output limits.
An individually oversized relation fails without rejecting fitting peers.

Four focused Rust tests cover capacity partitioning, SQL admission, peer isolation, and exhaustion of the shared stage allowance.
A separate layout test checks shared-column accounting and transactional admission.
The mandatory producer-shape gate preserves stage counts as independent roots, files, and policies increase.

### A file gate cannot preserve selected AST rows

The conditional rule needs declarations across the repository, plus selected nodes within particular bodies.
Its authored AST input selects individual rows and preserves selected ancestry.
The native syntax lowering replaces that input with a file gate and whole-file syntax.
Since the selection includes module declarations, every file qualifies.
The lowering therefore expands a selected relation into the entire repository's syntax and source demand.
The 32 MB repository-source guard then correctly rejects the 137 MB snapshot.

The initial experiment moves byte accounting after file selection.
A full quality-only run still fails: the file gate admits every repository file.
That experiment is removed. Source-budget behavior and limits remain unchanged.
The actual fix restricts native syntax lowering to file-partitioned contract clauses.
The global conditional clause uses its existing projected AST inputs and ordinary SQLite stages.
Its 20 typed input partitions share one AST selection utility instead of repeating the selection definition.
A generator regression test ensures global node selection cannot become a whole-file gate.
No new engine feature or Linear ticket is required for this correction.

All 1,259 sixth-batch recorded comparisons pass, including 417 positive and 842 clean observations.
These comparisons check both stateless and workspace execution.
The quality rule's 200 observations include 135 positives and pass separately.
The 12 focused Rust relational tests and native relational and conformance suites pass.

### Measurements and remaining preparation costs

The first combined 60-rule rerun matches all 24,993 findings but still fails parity.
Its only failures are interruption and work-budget exhaustion in the conditional ownership stage.
All 889 producer-capacity failures disappear, and the conditional clause passes the preceding source-budget guard.
Cold CPU is 1,076.563 s, elapsed time is 1,054.093 s, and peak RSS is 28.12 GiB.
Unchanged CPU is 2.102 s. Edit CPU is 53.083 s; both retain the conditional failures.
This is another failed measurement, not a passing speedup.

The SQL capture then identifies wide window rows and graph normalization as additional costs.
The general column-nulling pass increases the five-file conditional capture from 9,574,525 to 10,680,292 VM instructions.
That experiment and its unused helper extension are removed.
The selected ordinal window instead sorts narrow keys before joining projected node values.
The SQL test preserves every node value and requires fewer VM instructions on wide input.
Declaration selection also uses materialized key sets before joining class and function values.
Tests preserve class ownership and the latest declaration when names repeat.
Together, these changes use 8,723,163 VM instructions on the same five-file conditional capture, about nine percent fewer.
Instrumented CPU does not improve on that small selection. These counters do not establish a CPU speedup.

A focused repository capture then completes ownership but exhausts work during descendant normalization.
It uses 784,639,688 measured SQLite VM instructions before failure.
The normalizer prepares graph keys for every selected file, although only files with requested roots can contribute descendant rows.
The authored conditional plan now materializes its roots once and limits its edge query to those file domains.
CST edges cannot cross files. The test preserves closure results and isolated roots while excluding an unrelated file's edges.
Engine forest validation and graph semantics remain unchanged; the authored edge relation carries the narrower domain.
One covering forward edge index replaces two overlapping indexes.
The unused outermost-function ownership index is removed.

Those changes still exhaust the allowance during graph-key compaction on the full conditional snapshot.
The capture records 67,410,860 VM instructions for the key sort and 18,962,193 for two key indexes.
Compaction then consumes another 12,967,702 instructions before interruption.
The key table stores a dense number separately from SQLite's own row identity.
The engine now declares that number as `INTEGER PRIMARY KEY` and lets SQLite assign it during the existing deduplicated insertion.
This removes the numbering window and the redundant numeric index.
The unchanged scalar-key union preserves SQLite equality and its selected key values.
The fresh table receives one insertion per unique key, so its identities remain dense for the existing forest adapter.
Its scalar-key index also carries the numeric identity through SQLite's row index.
A small derived-forest test reproduces the preceding budget failure without a repository run and compares completed output with recursive SQL.
The preceding engine fails that test at 25,000 relational work units. The new engine passes all 28 reachability tests.
A separate SQL probe extracts the actual normalization statements from both engine versions.
It preserves decoded scalar types and values, including text, numbers, and blobs, and retains dense identities.
For 5,008 keys, normalization decreases from approximately 921,600 to 699,000 VM instructions.
This isolated probe uses Python's SQLite and does not establish a native workload CPU gain.

File-domain pruning alone still includes unrelated components within requested files.
The selected conditional plan therefore prepares the union of requested subtrees through one unlabelled SQL frontier.
Overlapping roots share that preparation. The existing producer then computes the separate closure for each root.
The regression test excludes an unrelated component within the same file while preserving all oracle closure rows and isolated roots.
This prepares demanded graph facts instead of normalizing every declaration's graph identity.

The corrected conditional clause passes over all 9,021 files, with zero findings expected and zero engine failures.
Its graph catalog contains 88,625 keys, compared with 824,441 in the preceding file-domain attempt.
The key insertion falls from 67,410,860 to 4,596,429 VM instructions.
The complete capture records 763,559,808 VM instructions, fewer than the preceding failed partial capture's 784,639,417.
Cold CPU is 199.837 s, elapsed time is 208.689 s, and peak RSS is 7.24 GiB.
This is a traced, conditional-only diagnostic with SQL capture and larger ingestion batches: 2,048 files or 16 MiB.
It is not the standard combined reference run, which retains 32-file or 1 MiB ingestion batches.
The full sixth-batch corpus also passes again on the new engine: 1,259 observations, including 417 positives and 842 clean observations.

The intermediate quality-only run eliminates all 889 producer-capacity failures.
It uses 140.094 s cold process CPU, 215.249 s elapsed time, and 2.79 GiB peak RSS over 9,010 files.
It still has the conditional source-budget failure and is not a passing performance result.
Its traced first partition contains 439 files and 950,054 roots in three producer passes.
Two capacity partitions suffice. The admission observations reach 268,454,464 SQLite bytes across 450 suspended sessions.
This counter measures admission storage, not the peak across subsequent SQL stages.
This quality-only measurement cannot establish a memory ratio against the preceding ten-rule run.

The corrected five-file quality probe passes at 3.888 s cold CPU and 1.068 s edit CPU, with 515 MiB peak RSS.
The earlier median is 1.940 s cold CPU and 0.710 s edit CPU for the same source selection.
The policy changes to preserve selected global evidence. This is a correctness cost, not an optimization gain.
Efficient native projection must preserve selected rows, ancestry, and column demand before it can replace that evidence path.

The full repository run also exposes profiling overhead during ingestion.
After every ingestion batch, `record_workspace_residency` walks all retained witnesses and serializes their projections to estimate bytes.
Repeated batches therefore rescan an increasing corpus.
A one-second external sample contains 96 active native samples; 74 include this accounting function.
This is a local ingestion observation, not a whole-run CPU percentage.
The projection captures retain spans and shared JSON projections; they do not retain complete source text for every capture variable.
Selected evidence still has substantial JSON storage and allocation overhead outside SQL admission.
Incremental residency accounting and compact demanded evidence columns remain preparation frontiers.

The workspace also prepares every dirty file-local job before it calls the SQL scheduler.
Its file-partitioned evidence index deliberately keeps eligibility witnesses, with projection and enumeration disabled.
Those gates are not complete cached input relations.
The consumer restores shared tapes, extracts the full AST inputs through `relational_rows`, and retains those rows alongside prepared facts in queued jobs.
Reusing the eligibility witnesses as complete input would lose findings.
Global projected-only producers also construct capture span maps before projection and then discard those maps through `retain_projection`.
Their next extraction path must preserve requested anchors and projected values while skipping unused capture metadata.
The new admission boundary is downstream of that preparation.
A second one-second sample contains 89 active native samples; 87 include `relational_rows` and `stateless_rows`.
Thirty-eight include witness construction, and 27 include evidence projection. These inclusive counts overlap.
This identifies full input extraction and queued input storage as another preparation frontier, separate from suspended SQL storage.
The next admission boundary must precede full projection, with queued descriptors holding document versions rather than complete input row vectors.
That change must preserve source versions, evidence completeness, cancellation, and broad producer batches.
The combined repository run exceeds 24 GiB despite SQL admission. End-to-end memory use remains unresolved.
A third one-second sample, after preparation, has 90 active native samples, all in SQLite under `Session::advance`.
The three samples distinguish ingestion accounting, repeated extraction, and SQL execution. They do not assign whole-run CPU shares.

The reference addons use engine optimization level 1, thin LTO, 256 codegen units, four workers, and the system-malloc shim.
The intermediate addon has SHA-256 `4c3f5357e27dfc054fa4e61a4139d71effb67430df7577e18890771774096631`.
The final addon has SHA-256 `26960ddfc2c0ecb06dcf7f25d639111f51c2a77d5499d261f1fa96fa39b3c1b9`.
Its build tools use optimization level 3 and 16 codegen units. These settings accelerate parser generation without changing runtime optimization settings.
Timing tracing is disabled, but profiling residency accounting remains enabled in that build.
Historical CPU timings remain unavailable. These results do not establish a CPU speedup over the historical 951-second passing run.

### Final checks and next work

The final addon passes all 1,259 sixth-batch observations, including 417 positives and 842 clean observations.
The global conditional entry contributes 28 observations, including 16 positives.
All 40 edit transitions pass. The 12 Rust relational tests, 28 reachability tests, and 40 native and signature conformance tests pass.
The mandatory producer-shape gate passes all five tests without skipping profiling assertions.
Its dependency test receives the same 110-second allowance as its independent-batch test after a shared-host five-second timeout.
Engine work limits do not change.
Clippy passes with and without profiling. Typecheck, repository checks, formatting, and profiling-script tests pass.

The next work starts before SQL session admission:

1. Queue document-version descriptors and admit full input extraction by capacity, instead of retaining all projected rows before consumers run.
2. Build demanded capture columns for global selected evidence and omit unused capture-span maps while preserving anchors and completeness.
3. Update profiling residency totals incrementally, instead of serializing the complete retained corpus after each ingestion batch.

The completed conditional forest walk takes about 56 ms CPU for 46,469 roots in 182 bounded batches.
The surrounding diagnostic uses about 200 s cold CPU. Faster graph traversal cannot remove the extraction and materialization costs around it.
The native producer gate preserves broad count, scan, and scatter passes.
Global projected AST extraction and its SQL demand frontier remain CPU preparation; these results do not measure whole-workload GPU launches.

## Full repository verification: 2026-10-21, execution `3285b48f`

**The first 50 rules pass at repository scale. Batch six fails at repository scale.**
The [complete evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/full-repository-20261021.json) preserves reports, failures, commands, source hashes, binary hashes, and corpus coverage.
This entry changes no engine implementation or limits.

### Repository results

All sources use data-layer commit `2b1504adc4291daaf080b2e932962b134da1a69a`.
The first 50 rules run together over 9,021 eligible files and 137,924,324 source bytes.
Batch six runs separately over its original 9,010-file, 137,896,473-byte snapshot for the historical comparison.
The additional 11 files are Rust grammar inputs.
There is no combined 60-rule repository result.

CPU values include process user and system time. Compilation is separate.

| Repository workload | Cold CPU | Cold elapsed | Unchanged CPU | Edit CPU | Peak RSS | Result |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| First 50 rules, current | **212.619 s** | 233.738 s | 0.801 s | 40.182 s | 6.60 GiB | Pass: 24,820 findings |
| Batch six, historical passing run | Not recorded | **951.004 s** | Not recorded | Not recorded | 11.14 GiB | Pass: 173 findings |
| Batch six, current | **538.452 s** | 645.958 s | 0.087 s | 11.344 s | 21.33 GiB | **Fail: 890 engine failures** |

The first 50 rules have zero extra findings, missing findings, or engine failures.
Their unchanged-file and trailing-newline edit comparisons also pass.
The edited file is `impl/common/l2200_sql_dialect/sql_connection.py`.

Batch six produces the expected 173 findings, but its failures prevent a parity pass.
Its unchanged and edited results also contain failures.
All repository findings belong to the direct-execution check. Matching them does not certify the failing quality check.
The shorter elapsed time therefore does not establish a passing speedup over 951 seconds.

### What the session history establishes

The remembered 1,400-second result is **1,456.276 seconds for batch six's ten rules**.
It has 84 extra findings and 46 engine failures.
The later passing run takes 951.004 seconds for those ten rules.
Neither historical result records process CPU time. Neither covers all 50 rules together.
No historical-to-current CPU speedup ratio is available.

The evidence identifies the session history file and its relevant lines.
It also preserves the historical reports, including the passing 40-rule and individual batch-five measurements.
Adding those independent durations would not reconstruct a joint 50-rule baseline.

### Full source-corpus coverage

All 60 rules pass **3,210 native observations: 1,631 positive and 1,579 clean**.
Each observation compares stateless and workspace execution.
Three additional signed-allowlist host checks pass separately.
Every rule has positive and clean coverage. There are zero diagnostic or execution-path differences and zero engine failures.
The 40 sixth-batch positive/clean edit transitions also pass.
The first-50 Vitest gate passes 25 tests, including deliberately incorrect detectors. Two signature tests also pass.

The evidence combines the complete first-50 gate with the all-batch replay.
It replaces 181 overlapping batch-five observations with that batch's full 312-observation result.
The overlap matches as a multiset of observation identities and outcomes. Both executions use the same native binary and policies.
Retained reporter contracts, synthetic graph cases, original I/O cases, and uncaptured-test metadata are not native passes.
This measurement replays recorded upstream inputs. It does not rerun upstream pytest or regenerate oracle expectations.

### Scale failures and the next frontier

There are **889 `producer capacity exceeded` failures** in `quality-contracts-1`.
The producer packs ready relations into a shared 64 MiB input arena.
It rejects every pending relation when preparation exceeds capacity.
The driver currently creates all dirty jobs and SQL sessions before the producer wave.
Automatic memory partitioning remains unimplemented, as recorded in the [execution contract](gpu-producer-execution-contract.md).
SQL session storage is outside the producer arena's limit.

One additional failure affects `quality-conditional-1`: `repository input budget exceeded`.
Global snapshot construction supplies all files in the rule's scope.
`Session::new` checks their combined source bytes against the 32 MB limit before selecting syntax-gated files.
The small profiling selections do not expose this full-snapshot failure.

A one-second external sample during the failed run contains 796 active native samples.
Of those, 789 are within SQLite stage construction, including B-tree work and memory copying.
This window identifies active materialization work. It does not identify the exact SQL statement or assign whole-run CPU percentages.

The next architectural work must partition execution by resident capacity and constrain preparation to demanded files and columns.
It must preserve repository semantics, completion outcomes, work allowances, and broad producer passes within each capacity partition.
Raising limits or suppressing failures would not establish parity.

### Measurement boundaries and reproduction

Timing runs use the preceding baseline's profiling reference addon: engine optimization level 1, thin LTO, and 256 codegen units.
The historical commands use a release build with thin LTO and 256 codegen units.
This comparison does not isolate optimization changes under identical compiler settings.
The ordinary correctness addon has no profiling feature and disables LTO. It supplies the corpus results, not performance measurements.
Both use the system-malloc shim. The evidence records all binary hashes.

Timing processes run sequentially. Host load is high; one observation is about 64 on 18 logical CPUs.
The failed batch-six timing includes the external one-second sample.
Cold means the first workspace evaluation in a fresh process, with no OS-cache flush.
Initial bounded attempts stop at about 112 seconds and do not enter the completed timings.
The user then authorizes full runs beyond the earlier two-minute limit.

Local complete reports are under `.rerail/profiles/full-repository-20261021/`.
`first-50-complete/parity.json` and `batch-6-complete/parity.json` retain the native driver output.
The first-50 oracle recording unions the pinned 40-rule and batch-five recordings.
Duplicate file paths must contain identical source. Every source is also checked against its pinned Git blob.
The union has no combined Python oracle timing. Its original recording hashes are in the evidence.

Use `conformance/data-layer/suite.ts --recording RECORDING --output RESULT --allocator system-malloc-shim --progress` through `node --import tsx`.
Select the recorded reference addon through `RERAIL_ENGINE_PATH` and preserve its allocator setup.
For fresh oracle recordings, use `scripts/dsl/data_layer_suite_record.py --batches 1,2,3,4,5` or `--batches 6` at the same source pin.
The evidence retains the actual invocation commands and ordered rule sets.

## Current baseline: 2026-10-21, execution `7da151b8`

This entry establishes the comparison point after removal of the experimental execution paths.
The [baseline evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/execution-baseline-7da151b8.json) contains all 15 timing runs, diagnostic summaries, and external samples.
The evidence includes policy, source, binary, allocator, and measurement-code hashes.

All times in this table are **process CPU milliseconds**, including user and system time.
Each value is the median of five fresh processes. Compilation is separate from cold evaluation.

| Pinned workload | Files / source bytes | Compile | Cold | Unchanged | Edit |
| --- | ---: | ---: | ---: | ---: | ---: |
| Quality | 5 / 1,026,452 | 88.2 | **1,940.1** | 1.96 | **710.1** |
| State | 7 / 1,123,712 | 298.3 | **12,512.3** | 1.61 | **1,137.8** |
| Dispatch | 1 / 136,810 | 46.8 | **982.7** | 1.43 | **972.5** |

Quality cold CPU ranges from 1,893.6 to 2,011.0 ms.
State ranges from 12,277.8 to 12,719.3 ms. Dispatch ranges from 954.9 to 1,005.1 ms.
Median process peaks are 444 MiB, 1,223 MiB, and 390 MiB, respectively.
These peaks include compilation and workspace storage. They do not identify particular heap allocations.

The quality result agrees with the promotion measurements of 1,967 ms cold and 707 ms per edit.
This entry records a new measurement series, with no implementation change or new claimed speedup.

### Method and limits

The implementation is `7da151b8e55dccbce883f100a9f941a09583b927`.
All source selections use data-layer commit `2b1504adc4291daaf080b2e932962b134da1a69a`.
The harness checks source hashes before evaluation and compares cold and incremental findings.
All 15 timing runs pass. No timing run is discarded.
Every measured command has a 115-second deadline.

The reference addon has SHA-256 `8a1f573e09b7837fabbf07073dbf7064b26ff298a7b104f0394acd2e8cac310f`.
Its source hashes match the committed implementation and promotion evidence.
It uses engine optimization level 1, dependency optimization level 3, thin LTO, and 256 codegen units.
The build includes `code-rule-profile`, with tracing, SQL capture, and external sampling disabled for timing runs.
The allocator is the existing system-malloc shim. The evidence records its library hash.
The runtime uses Node 26.8.1 and four native workers on an Apple M5 Max with 18 logical CPUs.

This is a reference build, not a benchmark of the standard optimization-level-3 production build.
The ordinary correctness build disables LTO and does not enter this timing series.
Cold means the first workspace evaluation in a fresh process. The procedure does not flush OS caches.
The workloads run sequentially, with alternating order between rounds, on a shared host.
Small differences within the observed ranges do not establish an optimization gain.

The three performance selections expect zero findings.
Positive coverage remains separate from timing: the quality policy and addon match the [promotion proof](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-shape-decisions.json).
That proof contains 200 upstream cases, including 135 positives, plus 219 production comparisons and 177 edit transitions.
This baseline reuses that proof. It does not claim another corpus replay.

### Diagnostic checkpoint

Three additional captures record SQL statements, plans, bytecode, counters, and native spans.
These captures pass parity and contain no incomplete executions.
They remain outside the timing medians.

| Cold diagnostic span CPU (ms) | Quality | State | Dispatch |
| --- | ---: | ---: | ---: |
| SQL materialization | 1,096 | 5,495 | 675 |
| Ordinary SQL indexes | 251 | 2,410 | 158 |
| Completed numeric publication | 20 | — | — |

These columns describe separate instrumented runs. They do not form percentages of the untraced timing medians.
The evidence retains the five most expensive SQL statements for each workload, including complete SQL and counters.
The local HTML reports provide the complete captures:

- Quality: `.rerail/profiles/rebaseline-7da151b8-20261021/quality/diagnostic/sql/profile.html`
- State: `.rerail/profiles/rebaseline-7da151b8-20261021/state/diagnostic/sql/profile.html`
- Dispatch: `.rerail/profiles/rebaseline-7da151b8-20261021/dispatch/diagnostic/sql/profile.html`

Quality's remaining targets are `scoped_nodes`, `scope_owners`, root collection, and ordinary SQL indexes.
Its capture records about 131 ms in `scoped_nodes` and 124 ms in `scope_owners`.
The main root query costs about 113 ms.

State is now the largest cold workload in this baseline.
Its leading SQL statements construct descendant relations, their indexes, syntax nodes, and ownership.
Those paths still reconstruct facts through the older evidence and SQL representation.
The quality implementation's shared syntax and completed numeric publication do not automatically cover those rules.

Three one-second external samples cover cold intervals in the state workload.
The first window also overlaps the compile boundary by about 14 ms.
The first two windows show matcher selection and allocation. The third shows SQLite execution, comparisons, and memory copies.
These windows expose both preparation and SQL costs. They do not estimate whole-run CPU proportions.
The evidence retains the top self costs and bottom-up caller chains.

### Reproduce this baseline

Use the implementation, source pin, build configuration, and allocator from this entry.
Select the matching addon through `RERAIL_ENGINE_PATH`.
Run each measurement in a fresh process and a new output directory:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/profiling/run.py \
  --trace off --deadline 115 \
  --source /path/to/pinned/nera-data-layer \
  --selection fixtures/host/data-layer-port/profiling/quality-five.json \
  --policies packs/data-layer/candidates/rules \
  --allocator system-malloc-shim \
  --output .rerail/profiles/new-baseline/quality/round-1
```

Use `state-seven.json` or `dispatch-one.json` for the other workloads.
Repeat each workload five times without overlap.
The evidence records the exact order, phase CPU values, ranges, and commands.
For diagnostic captures, add `--sql-profile --trace on --cold-only` and use separate output directories.
For external state samples, use `--external sample --sample-every 2 --sample-count 3 --sample-seconds 1`.
Select the native module through `--sample-module` if the addon has another filename.

## Experiment ledger

### 2026-10-20: shared SQL and reachability

**Decision:** keep SQL as the semantic engine and reuse shared facts before comparing execution backends.
The earlier specialized Rust evaluator did not provide a fair comparison against equivalently shared SQL preparation.
Its results do not establish a general native-engine speedup.

The reachability experiments compare recursive SQL, a Rust scalar function, and a Rust virtual table on the same requests.
The implementation at `d97e1239` keeps batched recursive SQL for general directed graphs.
Recursive UNION retains cycle termination and deduplicates converging paths.
UNION ALL is valid only with a proved unique-path invariant.
Scalar probes do not replace a broad batch interface.

Evidence: [shared SQL profile](shared-sql-performance.md), [reachability experiments](reachability-spikes.md), and [batched SQL implementation](batched-sql-performance.md).
The detailed reports retain the original build and workload boundaries.
Their results do not form a cumulative speedup series with later builds.

### 2026-10-20: generic forest traversal and storage

**Decision:** use native structural primitives where their graph contract is explicit, while SQL retains rule decisions.
The implementation at `f52817fc` adds strict forest traversal, deferred source text, and release after the last consumer.
General directed graphs retain cycle-safe SQL.
The IR-name propagation experiment is slower and does not become a native engine path.

The first budget model charges native visits too heavily and treats derived output as new AST input.
The corrected model retains limits and cancellation without those duplicate charges.
This result requires both positive corpus coverage and the larger pinned workload.

Evidence: [native relation experiments](native-relations-spikes.md), [implementation report](native-relations-implementation.md), and [measurements](../gate/data-layer/fixtures/host/data-layer-port/profiling/native-relations-implementation.json).

### 2026-10-20: shared syntax preparation

**Decision:** remove repeated preparation before adding more execution backends.
The implementation at `c4bd76bf` shares immutable syntax facts and uses direct parser identities and subtree boundaries.
SQL no longer reconstructs those facts for the quality groups.
This change addresses the expensive preparation that limited earlier native-access experiments.

Evidence: [requirement](shared-syntax-requirement.md), [implementation](shared-syntax-implementation.md), and [controlled measurements](../gate/data-layer/fixtures/host/data-layer-port/profiling/shared-syntax-metrics.json).

### 2026-10-21: broad producer passes

**Decision:** batch independent structural producers above their SQL consumers.
The implementation at `eec751d0` uses count, scan, and scatter passes for each dependency wave.
Resident packed input avoids repeated uploads or preparation between waves.
Tests keep the number of passes constant as roots, files, and policies increase.

The motivation includes the future GPU execution contract. This is a CPU reference, not a GPU speed measurement.
Batching alone does not establish a large CPU gain.

Evidence: [execution contract](gpu-producer-execution-contract.md) and [producer measurements](../gate/data-layer/fixtures/host/data-layer-port/profiling/batched-producer-evidence.json).

### 2026-10-21: complete SQL instrumentation

**Decision:** observe actual statement execution before another SQL-shape change.
The implementation at `e45dd638` imports statement capture, CPU counters, deferred plans, bytecode, timelines, and searchable reports.
Inclusive operation timings do not add to their child SQL timings.
VM instructions, sorts, and cursor visits supply explanations. They do not replace measured CPU.

Evidence: [profiler method and accounting](sql-workload-profiling.md).

### 2026-10-21: isolated SQL-shape experiments

The experiment implementation is `b6934ddf`.
Each row is a median of three fresh processes, with one binary and the pinned five-file quality workload.
All values are process CPU milliseconds. These are historical controlled measurements.

| Experiment | Cold | Edit | Decision |
| --- | ---: | ---: | --- |
| Baseline | 2,966 | 1,180 | Control |
| Method identity index | 2,822 | 1,011 | Keep |
| Forced owner-first joins | 2,842 | 1,048 | Reject forced order |
| Index and forced joins | 2,799 | 985 | Keep index alone |
| Return range index and bound | 2,943 | 1,120 | Keep |
| Narrow projections | 2,815 | 1,098 | Keep |
| UNION ALL root seeds | 3,027 | 1,197 | Reject |
| Materialized root CTE | 3,131 | 1,296 | Reject |
| Root unpivot | 3,080 | 1,247 | Reject |
| Virtual publication | 2,412 | 989 | Keep |
| SQL combination with UNION ALL roots | 2,548 | 835 | Retain as control |
| That combination and virtual publication | 1,996 | 718 | Retain as control |
| Accepted combination with original roots | 1,991 | 685 | Promote |

The method index lets SQLite select the better join order without a forced CROSS JOIN.
`self_stores` falls from about 8.1 million to 77,000 VM instructions.
The return index and upper start bound reduce excessive candidate scanning, while the sort count stays unchanged.
Narrow projections reduce unused column reads and retain row multiplicity and logical schemas.
They do not implement physical column removal or a shared cache of derived facts.

Virtual publication replaces output insertion and SQLite index construction with access to completed numeric arrays.
Diagnostic publication CPU falls from 671 ms to 19 ms, with some extra cost in SQL consumers.
The net workload gain is smaller than that publication saving.

All root rewrites regress in repeated CPU measurements.
UNION ALL removes sorts and lowers VM counts. The materialized CTE and unpivot reduce scans.
Those counter reductions do not offset their other execution costs.
The accepted combination retains the original root query.

Evidence: [experiment report](sql-shape-spikes.md) and [individual runs and counters](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-shape-spikes.json).

### 2026-10-21: promotion and removal of optional paths

**Decision:** one permanent execution path, implemented at `7da151b8`.
The method and return indexes, column narrowing, and completed-output virtual table become normal behavior.
The engine retains bounded 32-row insertion and always releases stages after their last consumer.
The three source domains remain separate.

Five execution switches, alternate input adapters, and older authoring controls disappear.
The profiling addon no longer depends on the experimental native-access library.
The module retains its source for SQLite reconnects and releases that ownership on DROP.
Live tables and cursors retain their own references.

Validation covers 400 native tests in each build, the producer gate, the positive corpus, production comparisons, and edit transitions.
Typechecking and Clippy pass in both build modes.
The [promotion evidence](../gate/data-layer/fixtures/host/data-layer-port/profiling/sql-shape-decisions.json) records the exact scope and hashes.

### 2026-10-24: R5-A acceptance starts with a failing ownership test

The delivery must remove all 3,474 planned submissions from baseline `188fb8f4`.
A reduction of 1,000 does not complete R5-A. The fixed guard must finish at or below 4,259 submissions.
The first regression test requires one indexed SQL store for two independent selectors.
It fails against the frozen baseline addon: two creates, two indexes, and two drops instead of one each.
The failing command uses `RERAIL_REQUIRE_PRODUCER_SHAPE=1` and the existing selector-materialization native test.
No production change is included in this red commit.

## Full-repository measurement reset (2026-10-25)

**Decision:** statement counts are diagnostic. The acceptance workloads contain all 9,021 data-layer files,
with separate runs for 50 and 60 rules. Gate on cold CPU, wall time, peak RSS, and complete findings parity.
Record rule compilation separately. This supersedes the earlier fixed-100 acceptance statements in this journal.

The previous full first-50 result was 2,507.496 s cold CPU, against 125.804 s before SQL candidate selection.
That regression remained unmeasured after the 10-23 and 10-24 changes.
Reducing the two-observation guard from 975 to 264 engine submissions did not demonstrate recovery.
I used that proxy to choose work after the repository measurement had already identified anchor selection as the dominant cost.

I read the sibling direction README, especially Statistics, Pushdown boundary, and Measurement gate,
and its lessons ledger before selecting further engine changes.
The current `Lower::pattern` still builds each possible posting, executes `count(*)` to price each arm,
and joins the posting to its runtime choice. This conflicts with plan-time selection from stored SQL statistics.
The producer adapter supports batched IN constraints, but its range path still estimates 32 rows.
Logical rule compilation currently precedes corpus publication. Physical anchor planning must follow index publication,
so it can use SQL statistics instead of reconstructing costs through candidate execution.
Logical rule validation can remain earlier. Cold timings must include fact and index publication.
This ordering does not require a generic graph scheduler.
The shared owner, source-version isolation, parity harness, relational clauses, and producer contracts remain useful.
The materialization boundaries and caches are provisional until full-scale measurements support them.

The new runner is `scripts/profiling/full_repository.py`. It freezes the addon and policies,
validates the two recording sizes, and runs fresh processes serially.
It records recording, policy, driver, and binary hashes, OS CPU/wall/RSS, phase CPU, compilation, findings, and failures.
SQL capture, detailed tracing, external sampling, and memory census are disabled in these timing runs.
Compact phase markers remain enabled, matching the historical controls.

The reference build uses engine opt-level 1, dependency opt-level 3, LTO disabled, 256 codegen units,
four native workers, and the archived dynamic system-malloc shim.
The recent small guard addon instead embedded real jemalloc. Those allocator configurations are not matched controls.
I rebuilt with `JEMALLOC_OVERRIDE=/tmp/librerail_jemalloc_check_shim.dylib` before the full comparison.
The matched addon SHA256 is `0b748ebd271647396e229918bf1af0acbbfa4286c55f9c886e4f4c54b5ee281b`.

Command:

```sh
python3 scripts/profiling/full_repository.py \
  --addon .rerail/profiles/full-scale-direction-20261025/matched-addon/engine.node \
  --output .rerail/profiles/full-scale-direction-20261025/rerun
```

Inputs are the unchanged `full-repository-20261021/first-50-recording.json` and `all-60-recording.json`.
Each has 9,021 files and 137,924,324 source bytes. Expected findings are 24,820 and 24,993 respectively.
The runner manifest and per-run receipts are under `.rerail/profiles/full-scale-direction-20261025/rerun`.
The reporting regression test first failed at 101 statements, then passed after removing the count gate.
These two isolated checks took about 0.3 seconds total. No builds or engine test workloads ran concurrently from this worktree.
The new report preserves the fixed reference, statement 101, internal counts, and complete-capture validation.
Lightweight reporting tests passed: 29 JavaScript tests (1.5 s wall) and 25 Python tests (3.5 s wall).
They ran during the 60-rule process. No engine builds or engine test workloads ran concurrently.
The actual diagnostic guard will run after the timing processes finish.

### First-50 result

The first-50 process completed with exit code 1. Workload and all 50 policy digests match the passing pre-SQL reference.

| Metric | Passing pre-SQL reference | Last SQL reference | Current run |
| --- | ---: | ---: | ---: |
| Cold process CPU | 125.804 s | 2,507.496 s | 958.590 s |
| Ingestion CPU | 73.350 s | 2,456.015 s | 875.286 s |
| Cold global evaluation CPU | 52.144 s | 51.145 s | 82.953 s |
| Cold wall | 196.707 s | Historical wall is not reliable | 1,978.154 s |
| Rule compilation CPU | 1.593 s | 1.635 s | 1.585 s |
| Rule compilation wall | 4.015 s | 3.817 s | 3.688 s |
| Driver peak RSS before close | 6.441 GiB | 6.143 GiB | 4.755 GiB |
| Findings | 24,820 | 24,820 | 24,820 |
| Execution failures | 0 | 0 | 3 |

OS resource accounting through process exit records 963.920 CPU seconds, 1,990.140 wall seconds,
and 6,261,948,416 bytes (5.832 GiB) peak RSS. This includes startup, compilation, reporting, and cleanup.
Use this full-process RSS for acceptance. Do not compare it directly with historical driver-only high-water marks.
The driver sampled its own high-water mark before workspace cleanup.

Cold CPU is 2.616x faster than the last SQL measurement, but 7.620x slower than the passing pre-SQL reference.
This run does not pass acceptance. Matching findings do not cancel execution failures:

- `sync-async-signatures`, `transaction-connection`: clause work budget exceeded in `impl/common/l2100_sql_grammar/rust_bridge/generated_facades.py`.
- `grammar-callable-constructors`, `constructor-arg`: name closure cannot decide membership over incomplete evidence.
- `public-api-triples`, `exported-families`: relational query interrupted.

Exclusive native counters record 537.411 s in anchor selection, 136.765 s in index preparation,
58.161 s in AST matching, and 11.148 s in parsing.
The counter named `manifestCandidateLookupNs` totals 191.489 s, but the profiler also uses that phase as its default worker phase.
It is residual worker CPU, not proof that candidate lookup alone caused that cost.
The first-50 run had no failed candidate batches; the final failure list still prevents acceptance.

The host had load around 73 on 18 cores and about 16.5 GiB swap in use during ingestion.
These are observations, not an explanation for every cost. Wall comparisons need that context.
Cold means a fresh process and workspace; OS filesystem caches were not flushed between serial runs.
The user cancelled the 60-rule run before completion. The cancellation entry records the partial result.
The completed 50-rule run remains an acceptance failure.

## Recovery work prepared and long run cancelled (2026-10-25)

The user requested the next work plan instead of waiting for the known-slow 60-rule run.
I stopped its Node process group, benchmark controller, and automatic completion recorder.
The run stopped after 3,149 files, with at least 1,119.65 sampled process CPU seconds.
Its last progress message reported 6,920,577,608 resident bytes. That observation is not an OS peak RSS measurement.
No completed cold timing, findings parity, or 60-rule acceptance result exists for this attempt.
The frozen manifest marks the run `cancelled-by-user`. The partial evidence remains available.
The cancellation receipt is `.rerail/profiles/full-scale-direction-20261025/rerun/all-60/cancellation.json`.
The completion recorder did not run the planned diagnostic guard.

The [active recovery plan](sql-performance-recovery-workplan.md#active-plan--2026-10-25) replaces the earlier count-driven sequence.
A1 includes SQL-owned kind/value statistics, selected-anchor lowering, decision-aware template reuse, and both evaluation paths.
It also starts the failing tests for the three full-scale execution regressions.
A2 restricts demand and fuses family statements. A3 batches native relation bindings and supplies honest estimates.
A4 establishes aggregate acceptance after bounded controls improve and the correctness regressions close.
No engine implementation, new production switch, or Linear issue forms part of this planning change.

The plan cites the central direction and both prior-work indexes.
I read the cited ast-index-spike source at `9449260`, including the correction from union sharing to identical-scope sharing.
The sibling integration also proposes a new findings contract and freezing authored SQL.
Those proposals remain separate from local recovery and cannot justify suppressing the current failures.

## A1: SQL statistics and selected anchors (2026-10-25)

Starting implementation: `983cb1db`, with the ownership checkpoint at `7222d7e8`.
The frozen before addon is `0b748ebd271647396e229918bf1af0acbbfa4286c55f9c886e4f4c54b5ee281b`.
Build settings remain engine opt-level 1, dependencies opt-level 3, LTO off, 256 codegen units, four workers, and the system-malloc shim.

Red commit `4067a17f` proves two failures: runtime posting counts and a stale physical template after an anchor-choice flip.
Red commit `e498b3a9` adds snapshot and workspace regressions for `sync-async-signatures`.
The 1.15 MB generated-facades file reproduces the observed budget failure in isolation.
A synthetic reduction with 1,000 synchronous functions also fails in both paths.
Neither source has function names with the required async suffix.
Both failures keep the original capabilities and allowances.

The first implementation adds SQL-owned value cardinalities beside existing kind frequencies.
`vlex.n` retains its segment-count meaning. Value estimates include uncovered nodes.
One batched statistics query supplies the admitted frontier. Physical template reuse depends on the selected requirements.
The compiler emits one anchor path and checks necessary root kinds after candidate selection.
Unused-arm tests poison compact postings. They pass only when the compiler avoids both unused requirements and common-kind enumeration.
SQL schema version 3 backfills older segment counts once and removes its migration callback.
A separate exclusive `anchorPlanningNs` counter exposes statistics and physical planning inside cold execution.

Initial verification passed 1,505 Rust DSL tests, including 93 SQL tests, and 120 recorded source observations.
Those observations contain 60 positive and 60 clean cases, with no finding differences or engine failures.
The generated-facades regression still failed. A profile proves that its plan exceeded the unchanged 256-node compiler allowance.
The fallback then admitted all 1,000 function roots.
This result is not a performance acceptance claim.

The next TDD rung orders candidate conjunctions by bounded structural compile cost.
It retains completed necessary filters when a later conjunct cannot fit, with explicit partial-admission metadata.
The native matcher still evaluates the complete authored rule. The compiler and runtime allowances remain unchanged.
A new test requires a cheap rejecting conjunct to survive a larger failed fragment.

Static checks also exposed two flat-directory limits. The runner now lives at `scripts/profiling/workloads/full_repository.py`.
The native regression lives under `test/engine/native/recovery/` and participates in the mandatory producer-shape suite.
The old journal commands retain their original paths as historical evidence.

Evidence and build/test logs are under `.rerail/profiles/anchor-planning-20261025/`.
No A2 or A3 implementation and no full-repository recovery claim exists at this point.

The bounded-conjunction change passes the snapshot and workspace regressions without larger allowances.
The synthetic case selects zero roots instead of 1,000. The production-shape suite passes 40 tests across 15 files.
Its old rollback check asserted a fixed total materialization count.
That count includes legitimate retained partial filters, so the check now requires one construction per logical producer.
This preserves its original sharing invariant.

The 120 source observations still match all positive and clean results.
A new positive pattern regression exposed duplicate parent roots when two rare leaves share a call.
The frozen intermediate addon returned two findings instead of one.
The fix makes each SQL parent climb distinct. A negative case also protects partial prefilters under negation.
Partial compilation cannot restore exactness through a cached utility.
Verification passed 1,506 Rust DSL tests and 42 production-shape tests.
Clippy and the repository static checks passed.
The final candidate addon hash is `ce93fc679e4ab6e3520a3bf39a0519d12a41c7862c3d8264e499c0d37dfcbfe9`.
Source batches 1–3 pass all 786 executable observations through both evaluation paths.
Batch six exceeded the 110-second command limit. Its replay remains incomplete.
Unchanged observation shards preserve the original cases and positive/clean coverage; repeated coverage cases need separate accounting.

### A1 bounded controls: performance regression, not acceptance

Hypothesis: stored statistics and one selected arm reduce candidate selection CPU.
The first controls reject that hypothesis for this implementation alone.
They include the compiler-admission correction, which retains useful predicates instead of admitting every root.

| Sample / rules | Before ingestion CPU | Candidate ingestion CPU | Change | Local failures before / candidate |
| --- | ---: | ---: | ---: | ---: |
| 32 / 50 | 9.588 s | 11.270 s | +17.5% | 1 / 0 |
| 128 / 50 | 15.395 s | 17.937 s | +16.5% | 1 / 0 |

These are single fresh-process, ingestion-only controls. They exclude repository analyses and do not establish findings parity or full-repository performance.
The workload samples include the original generated-facades file.
Both before runs retain its known signature budget failure; neither is a passing correctness reference.
Compilation remains separate: 1.529/1.571 CPU seconds for 32 files, and 1.547/1.567 for 128 files, before/candidate respectively.
Driver peak RSS changes from 1,256,488,960 to 1,224,261,632 bytes for 32 files.
For 128 files, it changes from 1,354,547,200 to 1,327,202,304 bytes.
OS resource files also retain whole-process CPU, wall, and peak RSS through cleanup.

Exact commands, resource files, and invocation receipts are in `control-{before,after}-{32,128}-50/` under the A1 evidence directory.
Each command uses `run-control.py VERSION FILES 50`, which invokes the tracked `anchor-ingestion.ts` workload under `/usr/bin/time -lp`.
The recording hash is `5ede2971ed87b1e64bd7fd125281410e3c527f5d7f756934e82f93d0828c9585`.
The selection hashes are `1ce275aa90a2b32619221296d812cc8205f581d9a72405e9a58d7fa1690508f2` and `43d012c4bff575ba4b35ecd2023e2d8931c64ce0b8a77c36f19f2305937802f2`.
Before/candidate receipts contain the matching policy digests and frozen addon identities.

For 128 files, matcher CPU falls from 2.771 to 0.637 seconds.
Selection CPU rises from 8.472 to 12.161 seconds, and physical planning uses another 3.267 seconds.
Index construction rises from 0.762 to 1.006 seconds.
This result rejects a full-scale recovery claim and does not justify another long full-repository run yet.
Keep the statistics, selected-arm correctness, and signature correction as implementation groundwork.
The next delivery must correct the remaining SQL shape and demonstrate a CPU improvement.

The separate `diagnostic-sample` run collects native spans and macOS bottom-up stacks.
Its first sampler exceeded its budget and produced no usable stack report; three later samples completed.
The workload itself completed successfully. This is diagnostic evidence, not a timing comparison.
Four candidate materialization statements each execute about 206–213 million VM instructions and 22.8 million full-scan steps.
They account for about 4.18 CPU seconds in that diagnostic run.
The next investigation captures those exact statements and their query plans before changing materialization.


## Indexed parent membership after A1 (2026-10-25)

Hypothesis: direct-parent membership creates quadratic scans when SQLite underestimates a materialized child projection.
The captured generated-facades queries confirm this shape. Queries 187–190 perform 838 million VM instructions and 91 million scan steps in total.
They use 4.277 CPU seconds in the diagnostic SQL capture.
The plan shows nested scans of `m` and `w`, without an automatic index.
The existing persistent child index already supports the required inverse lookup.

Red commit `9621e074` doubles direct-parent demand from 256 to 512 calls.
Its SQL work increases from about 1.29 million to 4.94 million VM instructions.
The test also requires correct roots and excludes a second file with identical contents.
The implementation seeks `decoded_child_lookup(fid,parent,field,pre)` from selected parents.
The final intersection retains the exact original scope. Field and first-child guards remain in place.
The upward `has` path also reads its narrow properties directly, without an intermediate wide projection.
This follows the prior-work lessons on narrow relations, indexed anchors, and avoiding temporary stage construction.

The scaling test now passes. All 1,507 Rust DSL tests pass; 23 remain ignored and 114 unrelated tests remain outside that filter.
The frozen addon hash is `a8257da224e2e6e4274ea2859873173239751cba0c40f3754e9e0cfbb4e3c2f1`.
Build settings, allocator, worker count, recording, and selection hashes match the A1 controls.

| Sample / rules | Frozen before CPU | A1 CPU | Indexed-parent CPU | Change from before |
| --- | ---: | ---: | ---: | ---: |
| 32 / 50 | 9.588 s | 11.270 s | 7.002 s | −27.0% |
| 128 / 50 | 15.395 s | 17.937 s | 13.563 s | −11.9% |

These are ingestion-only CPU measurements. Repository analyses and complete finding parity remain outside these controls.
The indexed-parent runs have no local execution failures; both frozen-before runs retain the known signature failure.
Physical planning remains inside ingestion CPU. Logical compilation uses 1.518 and 1.611 CPU seconds for the two new runs.
The new 32-file run uses 14.590 ingestion wall seconds and 1,169,195,008 bytes of full-process OS peak RSS.
The new 128-file run uses 24.591 ingestion wall seconds and 1,316,241,408 bytes of full-process OS peak RSS.
The host remains heavily contended; these single wall-time observations do not establish a latency gain.

Commands and OS resource files are in `control-parent-{32,128}-50/` under the A1 evidence directory.
The exact commands use `run-control.py parent FILES 50`; each invocation receipt records the command and host load.
Keep the indexed path. The result does not complete A2 or establish full-repository recovery.

The original generated-facades source also completes `sync-async-signatures` with no findings or failures.
Its recorded reproducer is `signature-recording.json`, and its A1 result is `signature-a1.json`.
Source batches 4–5 contain another 330 executable observations with no finding differences or engine failures.
Their overall replay commands return nonzero because six listed checks have no executable observations in those original recordings.
Do not count those checks as covered by these recordings. The separate 120-observation controls supply positive and clean cases for every rule.
Batch-six replay remains incomplete. Its timed-out commands and unchanged smaller shards remain available.

The SQL diagnostic command itself completes, but its report validation returns nonzero because the ingestion-only driver supplies no parity report.
That is not a count-limit failure. Its report cannot establish correctness acceptance.
The fixed 100 reference remains diagnostic.

The 60-rule comparison blocks aggregate acceptance of this candidate.
The frozen before version completes 128-file ingestion in 28.149 CPU seconds and 80.271 wall seconds, with the known signature failure.
The indexed-parent candidate exceeds the 115-second command limit and is terminated as a process group.
It completes only three of five ingestion batches, which account for 33.897 worker CPU seconds.
Its final cold timing, findings, and OS peak RSS remain unavailable.
The receipts are `control-{before,parent}-128-60/invocation.json`.
Do not report a 60-rule gain or extrapolate the 50-rule improvement to the full workload.

The indexed-parent candidate also passes all 42 production-shape tests and the 120 positive/clean rule controls through both evaluation paths.
The next TDD rung prohibits whole-kind enumeration during refinement of an already restricted candidate set.

To reproduce a bounded ingestion control directly, set a fresh output path and a frozen addon:

```sh
RERAIL_ENGINE_PATH="$PWD/.rerail/profiles/anchor-planning-20261025/parent-addon/engine.node" \
DYLD_INSERT_LIBRARIES=/tmp/librerail_jemalloc_check_shim.dylib RERAIL_NATIVE_WORKERS=4 \
/usr/bin/time -lp -o CONTROL_RESOURCE_PATH \
node --import tsx scripts/profiling/workloads/anchor-ingestion.ts \
  --recording .rerail/profiles/full-repository-20261021/all-60-recording.json \
  --files 128 --rules 50 --output CONTROL_REPORT_PATH
```

The recorded controller enforces a 115-second limit and terminates the whole process group on timeout.
Detailed tracing, SQL capture, and memory census remain disabled in timing controls.

## Scoped refinement and fused alternatives (2026-10-25)

Hypothesis: refine existing candidate keys without constructing another complete kind posting.
Red commit `94025468` poisons that unused posting and observes a truncated-posting failure.
The new path passes positive and clean cases without opening it.
It also uses known kind domains to remove redundant checks.
This change alone remains unaccepted: `control-scope-128-60` reaches the 115-second deadline without completing ingestion.
Its frozen addon hash is `42193cb37b85e9db850bc31ef42b03ca560aa01dbc9a8eea607310d064cd51ae`.

The separate `scope-diagnostic-one` run isolates the original generated-facades source with all 60 rules.
Its candidate batch completes in 11.050 diagnostic CPU seconds.
It decodes no new structural nodes, so sparse structural decoding does not explain that run's cost.
It decodes 60,507 spans and performs 370,224 source-text reads.
One left-deep union statement performs 89,776,219 VM instructions, 6,482,321 full-scan steps, and 110 sorts.
Two ingestion samples complete; their main leaf frames include SQLite execution, record comparison, and file writes.
The first sampler times out during compilation. The SQL report requires unavailable parity and returns nonzero.
The workload itself completes. These diagnostic observations do not establish performance or correctness acceptance.

Red commit `d4c288c4` records two observed failures.
Doubling alternative output increases work from about 243,100 to 878,400 VM instructions through repeated union materialization.
Failed literal admission also leaks its temporary kind restriction into the global input domain.
The implementation uses one deduplication boundary for each alternative family and restores the previous domain on failure.
It preserves known kind domains and duplicate-free results.

Optional capture fallbacks provide another proven source of unnecessary work.
The [public-api rule](../gate/data-layer/packs/data-layer/candidates/rules/data-layer-public-api-triples.json#L237) includes an unrestricted candidate arm beside structural capture arms.
The compiler now skips other candidate arms when one already covers the entire input scope.
The authoritative matcher retains the authored capture order and complete predicate semantics.
A test requires that gate to work before any additional producer can enter admission.
These unions compose alternatives over the same input scope; they do not merge different consumers' scopes.

The combined frozen addon is `6599af76a2980c20a1203484cf0d8dc3021de8fab01478a3277e30b9d1e56391`.
Build settings, allocator, workers, recording, and selection hashes match the preceding controls.

| 128-file control | Frozen before ingestion CPU | Combined CPU | Change |
| --- | ---: | ---: | ---: |
| 50 rules | 15.395 s | 12.457 s | −19.1% |
| 60 rules | 28.149 s | 26.246 s | −6.8% |

Both combined controls complete without local execution failures.
Both before controls retain the known signature failure. None of these controls evaluates repository-wide findings.
Logical compilation uses 1.603 and 2.528 CPU seconds for the combined 50/60 runs, separately from ingestion.
For 60 rules, physical planning uses 5.963 CPU seconds and selection uses 15.828 seconds.
Matcher CPU uses 2.262 seconds; index construction uses 1.284 seconds.
The fixed-count diagnostics report 4,445 data submissions and 3,863 materializations. They do not establish acceptance.

The combined 60-rule control uses 35.001 ingestion wall seconds; the before control uses 80.271 seconds.
Host load differs substantially, so this wall-time comparison does not establish a proportional speed gain.
Full-process OS peak RSS rises from 1,774,993,408 to 1,868,857,344 bytes.
The combined 50-rule control uses 21.013 ingestion wall seconds and 1,313,030,144 bytes of OS peak RSS.
Keep the correctness fixes and measured family improvements as recovery groundwork.
The CPU gain is modest and the 60-rule memory result regresses. Full-repository recovery remains unaccepted.

All 1,511 Rust DSL tests pass in two disjoint groups: 1,317 core tests and 194 workspace tests.
The groups retain 23 ignored tests; 114 unrelated tests remain outside the DSL filter.
An earlier combined attempt times out after 110 seconds and is incomplete evidence, not a pass.
The new SQL subset passes 99 tests with one ignored test; those tests also appear in the core group.
All 42 production-shape tests and all 120 positive/clean controls pass through both evaluation paths.
The complete original batch-six recording remains unfinished.

Commands and receipts remain under the A1 evidence directory, including `control-union-128-{50,60}` and `union-dsl-{core,workspace}.log`.
The next TDD rung removes private row copies from single-reader projection pipelines.
It preserves the source-span boundary that prevents repeated source predicates.

## Guarded projection streaming and planning attribution (2026-10-25)

Hypothesis: stream private key projections without copying every intermediate into an ephemeral table.
The red projection test in `0f3bce9e` measures approximately 45,000 versus 438,300 VM instructions across sixteen copies.
A broad single-reader inliner passes that test but fails the next TDD rung, `0cc8a7cf`.
One syntactic reader can perform many dynamic probes: the predicate runs 16,384 times instead of 1,024.
Reject that general inliner. Stream only generated key aliases with no computation, join, or renamed column.
Keep source-span barriers and computed predicates materialized before fan-out.
This policy adds no execution switch.

The rejected addon is `c987feb7fc390669a1ef06c2894909792a8fc1ddef059aea2305ed44a71781d7`.
Its 128-file, 60-rule control exceeds 115 seconds with only two ingestion batches complete.
Host load rises from 68 to 92. This timeout cannot establish which code change caused the delay.
Its one-file SQL diagnostic completes with 6.254 candidate CPU seconds and 311,536 source-text reads.
The preceding union diagnostic records 6.408 seconds and 311,530 reads.
Thus the synthetic fan-out failure does not explain that real-file timeout.
Keep this distinction explicit.

The guarded addon is `a5bf2e1dfd0d7b0ad64c735fa91b1c5e8ccb9fbfe181f03d15ce71aeb919291f`.
Build configuration, allocator shim, worker count, source hashes, and selections match the preceding controls.
The build uses release mode, LTO disabled, 256 codegen units, engine optimization level 1, and `code-rule-profile`.
Receipts and commands remain under `.rerail/profiles/anchor-planning-20261025/`.

| Ingestion control | Before CPU | Guarded CPU | Guarded wall | Guarded OS peak RSS | Logical compile CPU |
| --- | ---: | ---: | ---: | ---: | ---: |
| 32 files, 50 rules | 9.588 s | 6.751 s | 9.206 s | 1,201,455,104 B | 1.567 s |
| 128 files, 50 rules | 15.395 s | 12.404 s | 20.848 s | 1,308,999,680 B | 1.577 s |
| 128 files, 60 rules | 28.149 s | 26.152 s | 39.037 s | 1,833,091,072 B | 2.521 s |

All guarded controls complete without local execution failures. Before controls retain the known signature failure.
These controls omit repository-wide analyses and findings parity. They do not establish aggregate acceptance.
The 60-rule CPU improvement remains modest, at 7.1%. Its peak RSS still exceeds the before control by 3.3%.
Guarded streaming adds little beyond fused alternatives, so do not present it as the main recovery gain.
The receipts are `control-stream-{32-50,128-50,128-60}/invocation.json`.

The new profiler separates fragment lowering, composition, canonical ordering, rendering, and ownership scheduling.
Commit `11827b79` supplies the red reporting test. Reports retain the inclusive planning total without adding its children twice.
External sampling records sampler duration and timeout status. Its processing allowance rises while the whole command remains bounded.
The `stream-planning-diagnostic` run completes in 40.041 seconds; all three external samples complete.
Physical planning uses 2.020 CPU seconds: fragments 0.801, composition 0.278, ordering 0.071, rendering 0.389, and ownership 0.385.
SQL materialization uses 6.244 seconds; result reads use 1.017 seconds; SQL preparation uses 1.255 seconds.
The diagnostic working-set snapshots add 1.950 seconds and must stay disabled in timing controls.
The dominant SQLite sample contains record comparison, B-tree seeks, VM execution, and temporary-file reads and writes.
These are wall-time stack observations, not CPU percentages.
Planning is measurable, but execution remains the larger frontier. Target the expensive relation before further compiler caching.

The guarded version passes 1,513 Rust DSL tests in disjoint core and workspace groups.
There are 23 ignored tests; 114 tests outside the DSL filter remain untested by this run.
All 26 Python profiler tests pass. Type checking and strict Clippy pass.
All 42 production-shape tests pass with this frozen addon.
The combined canonical replay passes 1,236 native calls through stateless and workspace execution.
It contains 1,116 original executable observations from batches one through five, plus 120 supplemental controls across all 60 rules.
The supplemental controls provide 60 positive and 60 clean observations. They are not additional unique original tests.
The replay has zero false positives, false negatives, execution failures, path differences, or host-guard calls.
Six metadata-only checks have no executable observations in the original first-five recordings; supplemental coverage does not change that fact.
Original batch-six coverage remains unfinished.
The replay receipt is `stream-originals-and-controls-receipt.json`; the complete report is `stream-originals-and-controls.json`.

The separate `stream-sql-diagnostic-one` capture completes its workload, but report validation rejects unavailable parity.
It records 6.369 SQL CPU seconds without incomplete operations.
Its largest statement uses 1.247 seconds and 50,792,117 VM instructions.
That statement repeatedly partitions one candidate scope into kind alternatives, reunites them, and subtracts the union.
This identifies a concrete next hypothesis: fold finite kind alternatives before SQL emission.
Do not infer a full-workload gain from the statement cost alone.

## Finite kind alternatives: eliminate repeated partitions (2026-10-25)

The preceding captured Q224 identifies the actual relation shape for this change.
The serverplan-quality syntax exclusion at `packs/data-layer/candidates/rules/data-layer-serverplan-quality.json:9331` contains nested finite kind alternatives.
Related inputs occur in serverplan-dispatch-architecture and legacy-execution-surfaces.
See `performance-decisions/finite-kind-alternatives.md` for the hypothesis and prior-work citations.

Red commit `f2b52294` observes admission failure when finite alternatives have room for one decoder, one filter, and one subtraction.
The test retains the original restricted scope and compares selected nodes against tape kinds under positive and negative execution.
It includes repeated arms, nested alternatives, an enclosing restriction, and a shared utility.
The implementation folds exact finite alternatives to one kind set before SQL emission.
It memoizes utility definitions and retains the existing conservative path for approximate and non-kind predicates.
The arbitrary-union regression now exercises union construction directly, independently of kind folding.

All 104 SQL tests pass, with one ignored test.
All 1,516 Rust DSL tests pass: 1,322 core tests and 194 workspace tests.
There are 23 ignored DSL tests and 114 unrelated tests outside this filter.
The matched addon build completes in 98 seconds; the test build completes in 142 seconds.
Build duration remains separate from logical compilation and execution.
The frozen addon is `18371d800490b5aca2b4bf2962fead322a27ab7218fd5026ed117b867883eebd` in `kind-addon/engine.node`.
The evidence directory and build settings match the preceding guarded-stream entry.
All 42 production-shape tests and the same 1,236 canonical source replays pass with this addon.
Those replays preserve 1,116 original first-five observations and 120 supplemental controls, with zero mismatches or execution failures.
Strict Clippy passes. The complete original batch-six replay follows below.

The one-file diagnostic records SQL CPU of 6.369 seconds before folding and 4.977 seconds after folding.
VM instructions fall from 301,856,025 to 259,876,714; full-scan steps fall from 8,450,565 to 4,280,207.
Materialization spans fall from 4.625 to 3.246 CPU seconds; physical planning falls from 0.168 to 0.152 seconds.
Statement grouping changes, so do not attribute the entire difference to one matching query identifier.
Both diagnostic workloads finish with target exit zero. Report validation rejects absent parity, as expected for ingestion-only capture.
These captures do not establish performance acceptance. Unprofiled controls follow below.
Full-repository recovery remains unaccepted.

The unprofiled 128-file, 60-rule comparison uses three fresh processes per frozen addon.
All six runs use the same recording, selection hash, build configuration, allocator shim, and four workers.
Each receipt records source and native hashes. Repeat addon paths resolve to the same frozen binary.

| Metric | Guarded streaming, median | Finite kind folding, median | Change |
| --- | ---: | ---: | ---: |
| Ingestion CPU | 25.675 s | 21.827 s | −15.0% |
| Ingestion wall | 33.310 s | 24.824 s | −25.5% |
| Full-process OS peak RSS | 1,846,837,248 B | 1,838,448,640 B | −0.5% |
| Logical compile CPU, separate | 2.513 s | 2.546 s | +1.3% |
| Logical compile wall, separate | 3.288 s | 3.189 s | −3.0% |

The individual ingestion CPU values are 26.152, 25.183, and 25.675 seconds before folding.
They are 21.827, 21.948, and 21.615 seconds after folding.
No control has a local execution failure. These are passing ingestion controls, unlike the older failed before reference.
Host contention changes between runs. The CPU reduction is repeatable; the wall ratio is not an isolated scheduler effect measurement.
RSS remains effectively unchanged. This change does not resolve the memory regression.
The medians receipt is `kind-versus-stream-medians.json`.
Run receipts are `control-{stream,kind}[-repeat-{2,3}]-128-60/invocation.json`.
Each control uses the previously documented `anchor-ingestion.ts` command with a 115-second process-group deadline.

The single 128-file, 50-rule control records 11.929 CPU seconds, 20.387 wall seconds, and 1,314,832,384 bytes OS peak RSS.
Logical compilation uses 1.574 CPU seconds separately. The preceding guarded-stream control records 12.404 ingestion CPU seconds.
This small single-run difference is diagnostic; no 50-rule median claim follows from it.
The receipt is `control-kind-128-50/invocation.json`.

The next one-file diagnostic frontier remains materialization at 3.246 CPU seconds and candidate result reads at 0.831 seconds.
Span population alone uses 0.314 seconds; remaining fused set statements and source predicates account for other materialization work.
Planning uses 0.152 seconds on that file. Compiler caching alone cannot remove the larger execution cost.
The whole-batch native contracts, selective structural demand, and memory ownership work remain open.
This delivery completes the finite-kind family correction, not A2, A3, or the full recovery plan.

The complete original batch-six corpus now passes on the frozen kind-folding addon.
The manifest partitions 1,234 executable original observations into 198 bounded shards.
Each shard retains unchanged original events and includes positive/clean guards when needed.
The guards cause 376 repeated observations; they do not increase original coverage.
The aggregate verifies source and shard hashes, event equality, native identity, row identities, and exact primary-index coverage.
It records 405 positive originals and 829 clean originals, with no missing or failed shards.
All seven original retained observations keep their original status. No new skip or waiver is introduced.
The four verification groups complete in 68.197, 80.994, 63.658, and 72.126 seconds.
These verification durations are not runtime benchmark comparisons.

Together with the first-five canonical replay, this validates all 2,350 executable original observations through stateless and workspace execution.
There are 1,114 positive originals and 1,236 clean originals.
The 120 supplemental controls separately cover positive and clean execution for all 60 rules.
Six metadata-only checks still have no executable observations in the original recordings; the supplemental controls supply their native coverage.
There are zero false positives, false negatives, engine failures, execution-path differences, or host-guard substitutions.
Original observation coverage is now complete; full 9,021-file repository acceptance remains open.
The remaining constructor-argument source-incompleteness and exported-family query interruption still require full-scale root-cause evidence.

The complete manifest and aggregate are `corpus-06-kind/{manifest,aggregate}.json` under the evidence directory.
The source recording SHA256 is `a34b4a7e0c04be501912e9114f01e431044b030d2ad53a5ddc57650c9d9c070b`.
Commands are preserved in the shard runner and the four `kind-corpus-group-*.log` receipts.
A prior guarded-stream validation also passes the six original serverplan-bypass and four raw-execute observations.
Those ten observations are included again in the complete kind-folding validation, not added to unique coverage.

## Recovery continuation: ownership and whole-frame source confirmation (2026-10-25)

The preceding small-control gains do not complete the recovery plan. Full-repository acceptance remains open.
The new evidence directory is `.rerail/profiles/recovery-completion-20261025/`.

Red commit `d8938239` shows that physical ownership discards streaming decisions from the earlier pass.
Its sixteen-layer projection pipeline takes 503,800 versus 110,500 VM instructions after ownership.
Commit `8d111677` retains the selected boundaries in the graph that ownership renders again.
The regression passes. Earlier projection tests exercised the first renderer and missed this production handoff.
The earlier guarded-stream gain must not be interpreted as proof that ownership retained all those decisions.

Red commit `61fca9d0` requires whole-frame source confirmation.
The implementation stages typed `(producer, file, node)` requests, with a separate predicate dictionary.
Each dependency wave executes one native virtual-table filter. It returns typed matching keys only.
The producer decodes spans and source text internally, without per-node SQL scalar callbacks.
It reuses a point's source text within the wave while preserving each exact producer scope.
Requests are not widened to a union applied to every predicate.
SQL owns frame storage, candidate composition, and the published frame cardinality used by the native planner.
The native output loop only advances completed matches. No alternative production algorithm or configuration switch is added.
Both the registry and the tests are being updated to describe the fused contract accurately.
The old test requiring a separately materialized SQL span table no longer describes this binding.
Its replacement must preserve pinned source generations, identical-scope sharing, distinct-scope isolation, and one batch across result packets.

The first frozen source addon is `fe987c5bcb3d3aa95b40a3cea34f7e4a684dd0a4f52aceb0c7a3edcd3797dd9b`.
It passes 108 SQL tests, 1,326 core DSL tests, and 1,236 canonical replays.
The replays retain 1,116 first-five original observations and 120 supplemental controls, with no mismatches or execution failures.
Its first production-shape run passes 40 of 42 tests. The two failures assert the superseded separate-span shape, as described above.
The replacement contract metadata is not present in this frozen addon and requires a new build and validation.

The first 128-file, 60-rule source control records 21.263 ingestion CPU seconds and 50.387 wall seconds.
Its driver peak RSS is 1,841,168,384 bytes. It overlaps core verification and is diagnostic, not a clean timing comparison.
Do not infer a substantial speed gain from this control. Repeat timings after verification ends.
The matched build settings, source and selection hashes, four workers, and allocator shim remain unchanged.
The receipt is `control-source-128-60/invocation.json`.

The separate `source-diagnostic` run completes in 36.960 seconds with two completed external samples.
It reports 36 native source batches, 455,541 requested predicate/key pairs, 77,365 outputs, and 88,824 text reads.
SQL selection uses 8.178 diagnostic CPU seconds; physical planning uses 1.997 seconds.
This is diagnostic ingestion evidence, not full-repository parity or performance acceptance.

The new census checkpoints separate the recording harness, compilation, and ingestion.
The initial census records approximately 597 MiB resident before compilation, 957 MiB after compilation, and 1,427 MiB after ingestion.
The original workspace estimate accounts for only 35.8 MiB after ingestion.
The later source census records 19 physical templates containing 11,689,277 SQL bytes and 4,636,222 graph bytes.
Those retained strings do not explain the remaining RSS. The allocator counters returned by the shim are unavailable zeros.
The workspace census now exposes OS allocator accounting and retained anchor-cache sizes to separate live data from reserved pages.
The full recording buffer contributes about 145 MiB of array buffers even in the 32-file control.
Memory changes must preserve this measurement boundary and distinguish harness bytes from engine bytes.

Authored policy inspection finds 4,808 regex occurrences and 881 distinct patterns across the policy directory.
Validation and matcher construction currently compile identical default regexes separately.
The next TDD rung tests a compilation-scoped program cache, which releases its dictionary after compilation.
It must not introduce a persistent global regex cache.
A separate rung folds exact kind negation into one predicate rather than constructing and subtracting another row set.
Both changes remain unaccepted until their tests and matched CPU/RSS controls complete.

## Future entries

Each new entry must identify the hypothesis, baseline commit, workload hashes, build configuration, and exact commands.
It must record every timing run, parity scope, explanatory counters, and the keep-or-reject decision.
Diagnostic captures must remain separate from timing medians.
A different binary, allocator, workload, or build configuration needs an explicit comparison boundary.
New experiments must not add selectable execution paths to the engine.

### Compiler normalization and current acceptance run

The compiler now reuses regex programs within one logical compilation.
The cache drops at scope exit. Tests require one construction per distinct expression and fresh construction in the next scope.
Exact Boolean kind expressions now lower as finite sets or complements.
Scoped negation reads the supplied keys and does not build a global subtraction.

The frozen candidate is `ec6a287978eb3bfac6c03ce8c11925165c850a025c08af5cf15e73574043cb51`.
Evidence is under `.rerail/profiles/recovery-completion-20261025`.
The clean 128-file, 60-rule control takes 21.433 CPU seconds and 50.763 wall seconds.
Logical compilation takes 1.764 CPU seconds and 4.142 wall seconds.
The driver reports 1,480,523,776 peak RSS bytes. This is one run, not a median or an OS RSS comparison.
The previous finite-kind control median is 21.827 CPU seconds. Execution CPU has barely improved.
The previous compile measurement is 2.546 CPU seconds. Compiler reuse reduces this cost by about 31 percent.

Validation passes 1,328 Rust core tests and 194 workspace tests.
There are 23 ignored tests and 114 tests outside the DSL filter.
The SQL subset passes 109 tests with one ignored test.
Clippy with warnings denied, the repository static checks, and all 42 producer-shape tests pass.
The candidate also passes 1,236 original batch 1–5 and supplemental observation replays on both evaluation paths.
Batch 6 needs its own complete receipt. Eight jobs in the first 50-shard group exceeded the group deadline during the full-run overlap.
Those timeout receipts remain failed diagnostic evidence; they are not parity passes.

Full 9,021-file runs for 50 and 60 rules are in progress.
The first run overlaps the bounded parity group and a five-second external sample.
Its wall measurement needs this qualification. A clean acceptance run must have no such overlap.
The external sample attributes 327 of 458 active native stack observations to index publication.
It attributes 408 observations to SQLite overall. These are wall-time samples, not CPU percentages.
At 4,641 files, phase CPU totals include 230.929 seconds for selection, 91.076 for indexing, and 87.468 for physical planning.
This evidence does not establish aggregate recovery.

### Template ownership and fragment reuse ladder

Two cache tests reproduce missing eviction and admission of an oversized SQL statement.
The red build passes compilation and fails both tests in `cache-red.log`.
The production template owner now exposes its retained SQL estimate and a fixed 64 MiB SQL budget.
The red cache still ignores that budget; the next implementation enforces it.
A separate regression requires unchanged plan fragments to survive changes to the admitted frontier.
Another regression requires one kind posting with one SQL consumer to stay in the result statement.
These tests target retained memory, repeated compilation, and unnecessary physical materialization separately.

### Bounded SQL ownership and reusable fragments

The fragment and fusion red run passes 111 SQL tests and fails the two new regressions.
The cache green implementation passes eviction and oversized-entry tests.
The completed implementation passes all 113 SQL tests, with one ignored test.
It also passes 1,332 core tests and 194 workspace tests. There are 23 ignored core tests.
All 42 production-shape tests pass after removing an obsolete requirement for at least one materialized producer.
The test still requires executed result reads, unchanged findings, bounded admission, and one construction per physical producer.

The template cache admits at most 64 MiB of retained SQL and graph strings.
A separate 32 MiB fragment cache reuses per-plan lowering across admitted frontiers.
Neither budget estimates the complete process heap. SQLite keeps ownership of published facts.
Cache entries include language, logical plan identity, scope mode, path filter, and stored-statistic decisions.
A changed anchor winner replaces the fragment. Consumer IDs are rebound during physical composition.
One-reader posting relations remain inside their consumer statement; indexed membership and range probes retain required indexes.
No production flag selects these changes.

The frozen addon is `1734f8bc00d18c61cddcf4e6da84bf14007db6e4ad5371e52242f2758071533f`.
Its 128-file, 60-rule diagnostic takes 18.783 CPU seconds and 37.638 wall seconds.
Physical planning takes 3.619 CPU seconds, compared with 6.173 in the preceding control.
Selection remains almost unchanged: 11.049 versus 11.063 CPU seconds.
The driver peak RSS is 1,746,649,088 bytes. Compilation takes 1.785 CPU seconds.
This single control overlaps the earlier full-repository diagnostic and cannot establish a wall-time acceptance result.
The earlier fragment check failed on a Rust visibility error; the subsequent build corrects it.

### First complete 50-rule result after anchor recovery

All 9,021 files and 24,820 findings match, with zero false positives and zero false negatives.
Two failures remain: constructor-arg incomplete evidence and exported-families query interruption.
The earlier transaction-connection budget failure no longer occurs.
Cold execution takes 899.070 CPU seconds and 1,897.468 wall seconds.
Logical compilation takes 1.100 CPU seconds and 2.725 wall seconds.
OS peak RSS is 5,332,975,616 bytes, or 4.967 GiB.
The process-wide CPU total is 904.85 seconds, including startup and compilation.
The previous cold CPU result is 958.590 seconds. This is a 6.2 percent reduction, far short of recovery.
This artifact predates bounded SQL caches and fragment reuse.
Its wall time overlaps development tests, builds, and external samples; it is diagnostic evidence.

The full 60-rule run continues with the same frozen artifact.
A separate constructor-only run over all repository inputs reproduces incomplete evidence.
Both CPython and the generated native parser accept every one of the 9,010 Python sources.
The three largest grammar files also pass independently under the constructor rule.
The failure therefore requires evidence extraction or completion diagnosis; parser syntax is not established as its cause.

### Constructor evidence failure: reduced reproduction

A bounded bisection reduces the full constructor failure to `rust_bridge/generated_install.py`.
The generated installer contains two long functions with repeated `_facades.SomeClass` argument expressions.
An evidence-partition trace identifies exhausted utility evaluations, with no syntax failure and no completed witnesses.
The limit is the existing `maxVisitedNodes` utility allowance; this is not a name-closure traversal limit.
The port evaluates lexical-scope utilities before checking immediate constructor-argument position.
Identifiers inside an attribute therefore trigger scope scans before their parent shape rejects them.
A new 1,800-call synthetic test reproduces incomplete evidence with the ordinary capabilities.
The test requires complete, empty results. No budget is increased.
The selected correction moves argument-position guards before lexical-scope analysis while retaining capture dependencies.

The constructor ordering regression fails on both the existing addon and the partition-diagnostic addon.
Moving the immediate argument guard first makes all seven candidate-admission tests pass.
The original generated installer then completes with no failures and no findings.
The rule revision increases to 2. The lexical-scope algorithm and all limits remain unchanged.
The complete original batch 1–5 corpus and 120 supplemental controls pass all 1,236 replays on both execution paths.
This run uses diagnostic addon `fc8be34fff719f3651fb3ff7a5f9dc918d4d76203dc75ec55daa034a77c08356`.
Separately, all 198 batch 6 shards pass against the fragment addon, with 1,234 original observations and 376 guard repeats.
The batch 6 aggregate verifies 405 positive and 829 clean original observations, exact source events, and native binary hashes.
Its seven retained original cases remain explicit. These are not passes.

### Filtered SQL diagnosis

A new red test requires SQL capture to exclude unrelated publication contexts.
It also requires filtered captures to declare incomplete whole-job coverage.
The red run records two statements where the selected relational context contains only one.
The implementation will expose a diagnostic context filter, without changing execution algorithms.
The existing 100-statement validator rejects captures that lack whole-session coverage.

The filtered-profiler unit ladder passes all nine SQL profiler tests.
A separate production test reproduces unwanted candidate snapshots during a relational-only capture.
The red test observes one `.sqlite` snapshot where none is requested.
Candidate capture, origin discovery, and source-version capture will use the same context selection as the statement tracer.


The constructor-only full repository run now passes all 9,021 inputs, with no findings or engine failures.
It uses the revised rule and the diagnostic addon; cold CPU is 6.945 seconds.
The filtered capture implementation passes all nine profiler tests and all 44 production-shape tests.
Clippy with warnings denied passes. The execution-accounting tests pass.
The first static check fails the 25-file directory limit after two modules were added.
The source-confirmation module moves under `functions/`; the cache module moves under `corpus/`.
Rust module identities and behavior remain unchanged.

The full public-API diagnostic reproduces the query interruption with filtered SQL capture and tracing.
Its final query uses 419,835,138 VM steps, 18,055,384 full-scan steps, and 902,484 automatic-index rows.
Sixteen fused stages are already `AS MATERIALIZED`; twelve `NOT MATERIALIZED` definitions are aliases of retained inputs.
An initial interpretation of those aliases as missing boundaries was incorrect.
The fused stages lose their authored indexes. The next ladder must test indexed boundaries and live result plans.
Post-failure EXPLAIN currently reports released private inputs, so it cannot establish the failing result plan.
The diagnostic takes 262.806 cold CPU seconds and 452.646 wall seconds, under tracing and concurrent workloads.
It is not a performance control. Its raw SQL capture is 6.2 MiB and contains no candidate database copies.

The fresh 32-file anchor SQL capture completes without a timeout.
It intentionally fails complete-job accounting because it is filtered and has no findings oracle.
SQL totals include census overhead: repeated dbstat queries consume 1.421 CPU seconds.
One candidate-family fill consumes 22,244,658 VM steps; the largest result packet consumes 38,253,186.
Its source-confirmation transport reads 8.8 MB in the earlier comparable diagnostic.
Source-buffer copying is therefore not established as the main CPU cause; no new source cache is added.

### Indexed family boundary ladder

A closed 2,000-row regression preserves findings but observes zero authored indexes for a repeatedly read stage.
Fusion puts that stage inside a materialized CTE and discards its declared lookup index.
The correction retains one indexed owner when validated consumers read the producer more than once.
Single-use and unused stages remain eligible for fusion.
The read graph and existing conservative occurrence counter select this boundary; no runtime configuration chooses it.

### CPU tracing must not request an SQL page census

The public-API diagnostic spends 142.096 CPU seconds in 909 SQL working-set snapshots.
Those snapshots scan the growing main database through dbstat.
This is instrumentation overhead, not evidence of production fact-construction cost.
A new production test requires no page-census spans during ordinary CPU tracing and positive coverage when explicitly requested.
Its red run observes census spans without an explicit request.

### Shared producer identity ladder

The completed 60-rule diagnostic reports 24,993 matching findings but 1,694 engine failures.
Cold CPU is 2,701.305 seconds; cold wall time is 4,809.681 seconds.
OS peak RSS is 14,610,890,752 bytes. These results overlap development work and are not acceptance measurements.
Most failures report the producer stage limit. Independent capacity partitions incorrectly share this execution allowance.
An external bottom-up sample attributes 60.35 percent of active native observations to evidence-key interning.
Derived hashes repeatedly read complete policy bodies and repository version arrays.
Metadata accounting also charges the same shared allocations for every input, causing premature pool eviction.
Three new identity regressions fail: constant-cost version hashing, collision-safe cached definition hashes, and unique-owner memory accounting.
The memory test charges 3,551,197 bytes for shared owners after starting at 54,631 bytes.
Full definition equality must remain authoritative after cached hashes are introduced.

The indexed-family diagnostic now interrupts earlier, in the surfaces stage.
The repeated-reader boundary alone is therefore insufficient. Its live query plan will guide the next correction.

The shared identity regressions now pass. Repository fingerprints are computed once per immutable version set.
Evidence keys hash the existing rule digest, while full canonical equality still prevents collision-based sharing.
Memory accounting charges shared owners once and retains charges for distinct allocations.
The full-array versions and canonical definitions remain authoritative; no scope is weakened.

The 120-consumer capacity regression fails at consumer 33 with the old global producer allowance.
The correction keeps 100 native passes per logical consumer and records total launches separately.
An additional dependent-consumer test requires failure beyond 100 passes without rejecting a shallow peer.

The public-API live plan identifies a correlated scan of raw base classes inside the method loop.
Owner indexes also use only `__path`, because dynamic owner columns lack integer affinity.
A SQL regression reproduces the missing second seek column. Explicit casts of byte-offset owners make it pass.
The port adds one indexed base-fact owner for its two consumers and gates method discovery to exported classes.
The exported-class relation stays outside method expansion through an explicit SQL join order.
These changes preserve the authored rule semantics and existing budgets.
The closed SQL ladder passes both tests. Its lookup bound also passes before the change on the host SQLite planner.
That bounded test is therefore a guard, not a reproduced performance failure.

### Public API full-repository correction

The revised public-API rule completes all 9,021 inputs with zero findings and zero failures.
The frozen addon hash is `2606c4821cc149d01dbe181fb627c5fe992b71611996ffc639fbab91c4b9f377`.
The filtered capture is `.rerail/profiles/recovery-completion-20261025/public-api-fixed/`.
Cold CPU is 116.406 seconds, including tracing and filtered SQL capture. This is not an acceptance control.
The traits statement drops from 148,856,406 to 28,266,543 VM steps.
The previous surfaces stage consumed 47,398,436 scan steps before interruption. The revised rule completes.
The fifth-batch recording passes all 312 replays with the revised policy.
All 40 relational unit tests pass, including independent partition and dependent-depth limits.
The profiler tests pass 26 tests, and SQL profiling/accounting passes 29 tests.

### Empty repository gate before source hydration

A closed workspace regression sets a 32-byte source allowance and supplies three individually valid 27-byte files.
Its complete empty gate must avoid source hydration; the old executor reports a repository input-budget failure.
The correction queues source-free metadata and hydrates source only after admission.
A nonempty clause must still enforce the unchanged cumulative source allowance.

The admission addon passes all 47 production-shape tests and all 194 workspace unit tests.
The core DSL run passes 1,338 tests, with 23 explicit ignored cases.
The refreshed original batch 1–5 corpus and supplemental controls pass all 1,236 replays on both paths.
The admission addon hash is `cbcd50c7616fa3b20458b75f2b0fbe8da830a2a36caeb62b7660f1333ba4372d`.

A separate conditional-rule ingestion sample records 266 active native observations, all inside candidate SQLite execution.
Record comparison and B-tree seeks dominate that interval. These are sampled observations, not CPU percentages.
The sample is `quality-ingestion.sample.txt` beneath the recovery-completion evidence directory.
A statement-only profiler regression reproduces unwanted candidate database copies in an AST-index capture.
The new diagnostic switch disables replay copies while preserving statement counters and coverage.
This switch controls instrumentation only; production execution has one algorithm.

The candidate compiler also loses known empty scopes after contradictory kind constraints.
A new regression requires an impossible structural branch to construct no decoder or source producers.
This is exact empty-demand propagation, not a broader candidate superset or a changed matching contract.

The statement-only profiler passes all 48 production-shape tests and 27 Python profiler tests.
A bounded 32-file, 60-rule capture completes in 74.734 wall seconds, with no target timeout.
Its target uses 16.788 ingestion CPU seconds under instrumentation. This is diagnostic, not an acceptance control.
The report correctly rejects incomplete whole-job accounting because the capture filters AST-index contexts.
No candidate database snapshots are written. Plans and bytecode still produce a large diagnostic report.
One result statement performs 400 sorts, with 514 ORDER BY nodes in its captured plan.
It consumes 38,423,234 VM steps. The current transport requires `ORDER BY qid,fid,pre` for one-file buffering.
The selected correction uses bounded sparse-to-bitmap accumulators for unordered typed result rows.
Each accumulator stays bounded by its file's retained candidate representation, without repository-sized ordinal vectors.
The matcher must still receive canonical, sorted, unique ordinals. No algorithm-selection switch is added.
The contradictory-kind regression fails after constructing eleven unnecessary relations.

External vmmap evidence during conditional ingestion records 4.1 GiB of allocated native malloc storage.
It reports about 4.5 million allocations, 2.3 GiB resident malloc pages, and 1.9 GiB swapped/compressed malloc pages.
The JavaScript result array is not established as this memory's owner. Precise native ownership remains to be measured.
The 32-file capture's 400 sorts arise from candidate transport, not authored rule ordering.
Both transport regressions fail: sparse roots retain unordered duplicates, and SQL requires global packet ordering.
The empty-branch correction passes its regression without changing matcher confirmation or candidate semantics.

The SQL ladder catches a duplicate-row regression in the first empty-set shortcut.
The correction preserves DISTINCT boundaries for UNION and EXCEPT, including empty arms.
The SQL suite then passes 117 tests with one explicitly ignored replay laboratory test.
The mixed-coverage test compares complete typed row bags, since transport no longer promises SQL ordering.
The actual matcher-facing candidate representation remains sorted and unique.
A bounded accumulator test covers sparse promotion, duplicate rows, word boundaries, and complete-file selection.
The transport regression now measures SQLite sort counters over 6,000 candidate rows, rather than checking SQL text alone.

### Conditional global failure: complete reproduction

The conditional-only full run completes 9,021 input files and fails in ownership, with no reported findings.
Its empty diagnostic oracle does not establish findings parity.
Cold CPU is 996.902 seconds, including tracing and SQL capture. Cold wall time is 2,171.389 seconds.
OS peak RSS is 8,332,623,872 bytes.
The capture is `.rerail/profiles/recovery-completion-20261025/quality-conditional-diagnostic/`.
The ownership plan uses both declared lookup keys. There is no demonstrated missing ownership index.
The preceding nodes statement writes 1,419,253 wide rows and consumes 523,686,704 VM steps.
The ownership statement interrupts after another 99,226,765 steps, following other stage and index work.
The fused query materializes two wide pure projection unions, then copies the wide data into an automatic join index.
The narrow ordinal consumer cannot push its column demand through those forced materialization boundaries.
A closed production regression reproduces `MATERIALIZE helper` for a 48-column projection with a narrow window consumer.
The correction streams pure column/NULL projection unions and directs the wide scan toward the narrow ordinal lookup.
Native sources and deferred columns retain their evaluation boundaries. No rule allowance increases.

Failure-only snapshots will preserve main and temporary SQL owners separately before failed-session cleanup.
They require an explicit diagnostic directory and run only after failure, so successful benchmark runs do no snapshot work.
Native virtual-table frames and external source callbacks are not serialized by SQLite backups.
Snapshot metadata must record this limit.
The refreshed admission artifact passes all 198 batch 6 shards: 1,234 original observations and 376 guard repeats.
Seven original retained cases remain explicit. The transport artifact is receiving its own full recording rerun.

The pure-projection implementation passes all 50 production-shape tests, 1,343 core DSL tests, and 194 workspace tests.
All 1,236 original batch 1–5 and supplemental replays pass with the revised policy.
The SQL laboratory initially fails to compile because earlier source-batch imports and a diagnostic hash dependency were missing.
Its production source relation import is restored, and CST-dependent tests move into the native test owner.
The laboratory adds the same SHA-256 dependency used by production. Existing dependency versions remain pinned.
The failure replay host shares the engine's SQLite backup boundary and restores both SQL owners.
Its closed test checks Unicode, NULL, typed bag comparison, temporary-owner restoration, and write rejection.
The final addon hash is `dc6974796680385a06ecaca90e980ac339a9f36bdbb9c83a6c53fe332e54dc8e`.
Its production-shape run also passes all 50 tests, including failure capture without statement tracing.
Full recovery acceptance remains open until clean full-repository correctness and performance measurements are reviewed.

The final source passes static/type/boundary/path checks and native Clippy with warnings denied.
The SQL laboratory passes 63 tests and Clippy with warnings denied under its pinned dependency lock.
Its source-batch and diagnostic hash dependencies now match production.
The transport artifact passes all 198 original batch 6 shards, including 1,234 original observations and 376 guards.
All first-five-batch originals and supplemental controls also pass with the projection artifact.
The final addon passes all 50 production-shape tests, with no skipped shape checks.
The fixed execution-accounting suite passes all 29 tests.

### Frozen full-repository candidate run

The next measurement uses frozen addon `dc6974796680385a06ecaca90e980ac339a9f36bdbb9c83a6c53fe332e54dc8e`.
Both 9,021-file workloads retain their original recordings and expected findings.
Logical compilation, cold execution, wall time, and OS high-water RSS remain separate measurements.
The controller runs the first 50 rules, then all 60, with four native workers and 32-file ingestion requests.
CPU tracing, SQL statement tracing, page census, memory census, external sampling, and replay copying are disabled.
Failure-only SQL snapshots are enabled. They run only after an error; a failed measurement cannot pass acceptance.
The evidence directory is `.rerail/profiles/recovery-completion-20261025/full-candidate/`.
No own development workload will overlap these timing runs. Other worktrees remain outside this agent's control.

### First-50 result: recovery rejected

The frozen first-50 run completes all 9,021 files and fails acceptance.
Cold CPU is 903.546 seconds: 600.330 user seconds and 303.215 system seconds.
Cold wall time is 1,312.792 seconds. Logical compilation takes 1.129 CPU seconds and 1.492 wall seconds.
OS peak RSS is 4,822,499,328 bytes (4.491 GiB).
The complete process takes 907.50 CPU seconds and 1,318.46 wall seconds, including compilation, reporting, and cleanup.

The run reports 19,889 of 24,820 expected findings, zero false positives, and 4,931 false negatives.
All missing findings belong to `internal-dead-code`.
Its `unreachable-internal-symbols` clause interrupts in `reachable` and reports an exhausted work budget.
The constructor and public API failures from the earlier run do not recur.
Matching the other findings does not offset this new failure.

Cold CPU is 7.182 times the passing 125.804-second reference.
It is also 0.50% above the previous failed 899.070-second measurement.
These measurements do not demonstrate aggregate CPU recovery.
Failure-only snapshot copying adds work after the error, so this failed timing is diagnostic.
The [checked receipt](../gate/data-layer/fixtures/host/data-layer-port/profiling/recovery-full-candidate-20261025.json) preserves identities and measurements.

Across all phases, exclusive counters record 453.404 seconds in selection, 206.421 in index preparation, and 93.175 in physical planning.
The earlier failed run records 418.068, 183.807, and 132.476 seconds respectively.
Both runs publish 28,230,245 nodes and select 6,425,028 root pairs.
The lower planning cost does not recover the complete workload's CPU cost.

The failure capture preserves 121 MiB of main data and 385 MiB of temporary data before cleanup.
An initial host-SQLite EXPLAIN shows an automatic `reach_edges.source` index.
That check executes no producer query and uses host SQLite 3.54.1.
It does not prove a missing index or establish the production query's runtime.
Production-SQLite replay must separate edge construction, recursive traversal, and both budget boundaries.

The first-50 measurement has no overlap with this agent's builds or tests.
The all-60 process continues as a diagnostic while failure investigation proceeds.
Builds and bounded experiments can overlap that second process; its timing cannot establish acceptance.

### Dead-code failure: protocol demand arrives after method fan-out

The failure replay now installs the engine's actual regex functions.
The shared implementation preserves NULL handling, capture-group errors, regex caching, and work accounting.
Red commit `bf5f89c6` fails with `no such function: rx_replace`; both replay tests now pass.
Red commit `853be685` requires failure snapshots to preserve both budget boundaries.
The corrected addon passes all seven relational-storage tests and all 50 production-shape tests.
It also passes all 1,236 batch 1–5 original and supplemental replays, without findings or execution-path differences.
Native and replay Clippy checks pass with warnings denied. All 64 laboratory tests pass.

Production SQLite 3.53.2 replays the captured `reachable` query in 51.819 CPU seconds.
It executes 474,828,846 VM steps and returns 67,284 symbols.
The isolated class-name producer takes only 0.293 CPU seconds, disproving the initial class-name hypothesis.
Edge construction takes 34.738 CPU seconds and 388,693,756 VM steps.
An external stack sample shows B-tree seeks, page reads, and cache spills in that query.

The final edge arm expands matching method names across unrelated classes before checking the owner's protocol status.
The capture's plan reads methods, then unrelated methods, then the owning class.
The corrected SQL puts protocol classes first and uses ordered CROSS JOINs for both dependent method sets.
It preserves all predicates, duplicate elimination, recursion, and declared allowances.
Red commit `79debb52` exceeds two million VM steps with empty and nonempty protocol demand.
The correction passes both cases and checks the complete expected edge sets under skewed statistics.

| Captured component | Original CPU | Corrected CPU | Original VM steps | Corrected VM steps |
| --- | ---: | ---: | ---: | ---: |
| Edge construction | 34.738 s | 2.544 s | 388,693,756 | 68,781,444 |
| Complete reachable query | 51.819 s | 20.196 s | 474,828,846 | 154,916,534 |

Both comparisons retain identical typed-row digests and counts.
The edge set contains 790,115 rows. These measurements include result hashing and exclude ingestion and the original allowance.
Each replay command has a 110-second deadline. Other diagnostic work overlaps the replays.
The [checked experiments](../gate/data-layer/fixtures/host/data-layer-port/profiling/dead-code-demand-recovery-20261025.json) retain results and limitations.

A separate experiment changes the edge union to UNION ALL with one outer DISTINCT.
It removes all sort operations, but edge CPU increases from 34.738 to 39.815 seconds.
The complete query exceeds the 110-second deadline. The partial rewrite also exceeds that deadline for the complete query.
Both union experiments are rejected. Production retains the original set operators.
The protocol-demand correction changes policy revision 2 and its generator only.
The full 9,021-file original dead-code oracle now runs separately to check all 4,931 expected findings.
Aggregate recovery remains open. No successful full-rule-set performance result follows from this component improvement.

The corrected dead-code policy completes the full original repository oracle with 4,931 expected and actual findings.
It reports zero false positives, false negatives, or engine failures over all 9,021 files.
The [full validation receipt](../gate/data-layer/fixtures/host/data-layer-port/profiling/dead-code-full-validation-20261025.json) preserves identities and the concurrent-timing qualification.
Cold CPU is 157.516 seconds, wall time is 200.574 seconds, and OS peak RSS is 2,702,163,968 bytes.
That single-rule measurement overlaps other work and does not establish aggregate performance acceptance.

The rebuilt native DSL suite passes 1,537 tests, with 23 explicit ignores and no failures.
All 198 final-artifact batch 6 shards pass: 1,234 original observations and 376 guard repeats.
The seven retained original cases remain explicit. No shard is missing or failed.
Together with batch 1–5, this covers all 2,350 executable original observations, plus the supplemental controls.
The final corpus addon is `2e533a51a816eb47cc8ea682707cff4b88b34d2aff8d786a71f368aeb2d60cd4`.
The later diagnostic field for nested budget exhaustion also passes the rebuilt native suite.

### Next candidate-selection frontier

An external sample observes the running all-60 ingestion at 4,768 loaded files.
Its active worker stays inside `Corpus::select_group` and `execute::rows_for` for all 467 observations.
The hottest stacks compare B-tree records. This is a short sample, not a complete CPU census.
The sample is `.rerail/profiles/recovery-completion-20261025/full-60-ingestion.sample.txt`.

The regex value path places cached matching terms before the admitted file relation.
It therefore performs a vocabulary-term by requested-file product before each `vidx` lookup.
The global retained vocabulary can grow while a consumer requests only a few files.
A new production-lowerer regression isolates this scaling shape with 20,000 unrelated terms and 32 admitted postings.
This is the next experiment. No speed improvement or selected replacement follows from the source inspection alone.

### Regex candidates: bound vocabulary work to admitted files (2026-10-26)

Red commit `663df84e` reproduces 4,540,694 VM steps for 32 admitted postings and 20,000 unrelated matching terms.
The corrected regex path scans `vidx_file` within the admitted files, then seeks cached predicate membership.
Exact literals retain their direct term lookup. The positional FTS index and SQL-owned statistics remain unchanged.
The native regression now stays below 20,000 VM steps. All 118 SQL tests and 50 production-shape tests pass.
Native Clippy passes for all targets with warnings denied.
All 2,350 executable original observations pass with this addon, plus supplemental controls and guard repeats.

The 128-file, 60-rule ingestion control changes from 18.866 to 19.175 CPU seconds.
That small control does not demonstrate a CPU improvement. Both processes overlap other diagnostic work.
The retained full-repository index exposes a different scale: 294,614 cached terms for `(?s).*` during the survey.
One read-only snapshot compares both SQL access paths over the same 32 admitted files.
Both return 3,876 identical posting rows. CPU changes from 2.968 to 0.033 seconds.
Approximate VM work changes from 81,708,000 to 84,000 steps.
This probe uses host SQLite 3.54.1 and omits tape decoding. It is component evidence, not aggregate acceptance.
The production regression uses bundled SQLite 3.53.2.
The [checked experiment](../gate/data-layer/fixtures/host/data-layer-port/profiling/regex-file-scope-recovery-20261026.json) preserves these qualifications and full corpus coverage.
Keep the bounded regex path. It removes work proportional to unrelated retained vocabulary without an execution switch.
The first complete repository measurement of this replacement remains required.

### Complete all-60 diagnostic: remaining failures

The frozen older addon completes the repository with 2,714.768 cold CPU seconds and 3,914.650 cold wall seconds.
Compilation takes 1.985 CPU seconds and 3.358 wall seconds.
OS peak RSS is 12,607,602,688 bytes (11.742 GiB).
The run overlaps development, replays, and external sampling. Its timing is diagnostic.
The [checked receipt](../gate/data-layer/fixtures/host/data-layer-port/profiling/recovery-all-60-candidate-20261025.json) records the frozen identities.

It reports 20,062 of 24,993 findings, zero false positives, and seven failures.
All 4,931 missing findings belong to the older dead-code policy; its correction passes the separate full repository oracle.
Other failures remain in conditional ownership, contract call paths, and two projection-recovery result evaluations.
The producer-depth failures do not recur, but these SQL failures still reject recovery.
Conditional ownership alone replays in 4.475 CPU seconds and 103,042,098 VM steps, with no sorts or automatic indexes.
It returns 1,419,253 rows. Work before this stage remains part of the failure's cause.
No allowance changes follow from this result.

The external vmmap sample records 10.7 GiB allocated across approximately 25.45 million native allocations.
The 128-file census separately records 93,716,350 bytes of typed projections in 1,749 immutable batches.
Those batches contain 49,185 rows, averaging 28 rows per batch against a reserved capacity of 256.
The next memory regression tests whether sealed partial batches retain unused column capacity.


### Release unused projection capacity (2026-10-26)

Red commit `e3bcba1e` proves that six published rows retain storage for 256 rows in each column.
Published projection columns now use boxed slices. Builders retain their growable vectors until publication.
The change preserves typed cells, shared source storage, schemas, and row identities.
The native DSL suite passes 1,539 tests, with 23 explicit ignores. All 50 production-shape tests pass.
All-target Clippy passes with warnings denied.

The 128-file census retains the same 49,185 unique rows, 680,573 fields, and 1,749 projection batches.
Typed projection storage decreases from 93,716,350 to 11,696,974 bytes, an 87.5% reduction.
Driver peak RSS changes only from 1,655,357,440 to 1,649,115,136 bytes.
Census CPU changes from 20.336 to 18.423 seconds, but concurrent work prevents a timing acceptance claim.
This addresses retained column padding. It does not prove full-repository RSS recovery.
The [checked census](../gate/data-layer/fixtures/host/data-layer-port/profiling/projection-capacity-recovery-20261026.json) preserves scope and identities.


### Quality imports and conditional ordinal demand (2026-10-26)

The contract failure reproduces on `intent_versioning_extended.py` with 148,502 syntax nodes.
The failure location is `call_paths`, but `py_names` consumes most of the shared SQL allowance first.
Its range join compares file-wide identifier candidates against every import range.
A separate import-subtree producer now receives all import roots together through the existing reachability contract.
Identifier membership retains the composite file/node guard. It does not enlarge the general descendants domain.

The traced `py_names` work decreases from 86,986,185 to 5,588,856 VM steps.
The complete contract clause finishes without errors or findings on that file.
Its original interrupted trace records 114,708,243 VM steps; the complete corrected trace records 47,135,698.
These totals cover different completion states. No allowance increases apply.
Red commit `c830a02f` bounds import collection and checks cross-file node-ID collisions.
The [checked trace](../gate/data-layer/fixtures/host/data-layer-port/profiling/quality-import-demand-recovery-20261026.json) preserves stage counters.

Red commit `8beede3e` requires conditional ordinals only for four demanded parent kinds.
The correction numbers every child within those complete parent partitions.
A LEFT JOIN retains all other nodes with NULL ordinals. The consumer predicates still read the same ranks.
The 9,021-file conditional-clause validation runs separately; it is not yet a passing aggregate result.

### Projection recovery: stop repeated ancestor and identifier scans (2026-10-26)

Both remaining result failures reproduce on `sql_graph_query_builder.py` and `rest_router.py`.
Their SQL allowances reach zero while the outer clause allowances remain unexhausted.
The isolated assignments and string-value components finish cheaply.
The ancestor predicate alone reproduces both failures.
Its range access reads large node prefixes repeatedly before checking kind and scope name.

A narrow `projection_scopes(__path,__start,__end)` relation now selects eligible scopes once per file.
All conditional consumers reuse that exact file domain. No cross-rule union expands its scope.
The existing fused result statement owns the materialization; no new execution mode or temporary-table lifecycle is added.
Red commit `6710fbc5` reproduces 442,574 kind reads for 200 conditionals and checks function/class scope boundaries.
The corrected test remains below 10,000 reads with identical selected conditions.

The two complete result queries now finish with zero failures and findings.
Their VM work changes from interrupted lower bounds of 89,917,664 and 91,771,764 steps to 288,939 and 275,619 steps.
These are traced component results. They do not establish full-repository CPU or RSS recovery.
A separate experiment that hides the range bound inside CASE still fails both files. It is rejected.
The [checked comparison](../gate/data-layer/fixtures/host/data-layer-port/profiling/projection-scope-recovery-20261026.json) retains each file's counters.

The same investigation found repeated identifier scans in module constant propagation.
Red commit `2d296f25` measures 64,000 text reads for 80 bindings and 800 identifier uses.
A narrow module-binding relation now precedes one identifier scan and an indexed SQL membership join.
The test retains cross-file isolation and the complete propagated value set, below 2,000 text reads.
This correction alone does not resolve the real result failures; the ancestor correction is required.
All 16 shared-SQL tests pass. The original positive/clean corpus is rerun after these changes.


### Conditional preparation: later outer-budget exhaustion (2026-10-26)

The scoped-ordinal run finishes ingestion of all 9,021 files but still fails.
It passes ownership, then exhausts the 50,000-unit outer allowance at `conditional_methods`.
The error snapshot retains 9,967,090 SQL work units. This is a different budget boundary from the contract and projection failures.
Its functions table contains 161,553 rows with 53 columns. Conditional classes contain 31,841 rows with 55 columns.
The original node relation contains 562,809 identifiers and 337,575 dotted names.

The column-demand tool now supports projected-input plans as well as native syntax plans.
It removes unused stored columns while retaining their logical names through NULL expressions in compatibility views.
The existing 32-field deferred limit remains unchanged. Additional unused fields remain stored NULLs.
Functions now store 21 columns, including 11 live values. Classes store 21 columns, including five live values.
Conditional methods store 25 columns, including nine live values.
The demand analysis preserves predicates, indexes, alternatives, window inputs, DISTINCT, bags, and deferred-expression dependencies.
Tests compare projected and native input plans with the original row results.
A direct attempt to remove logical names fails SQL binding in unused nested expressions and is rejected.

The import selector also requested every descendant of every import.
Alias resolution needs import declarations and direct dotted-name or aliased-import entries.
The narrowed selector retains those entries. Other selector arms retain syntax needed by bodies and annotations.
A production-policy native regression changes 400 unused identifier observations to zero over 100 imports.
It retains all 100 import declarations, aliased imports, and direct dotted names.
The [checked preparation receipt](../gate/data-layer/fixtures/host/data-layer-port/profiling/conditional-preparation-recovery-20261026.json) records stored widths and unchanged limits.
The complete conditional repository check remains required before closing this failure.

The fixed SQL accounting diagnostic records 296 engine submissions and 45 SQLite internal executions.
It retains complete accounting and parity, but exceeds the fixed 100 reference.
The reference remains diagnostic under the agreed measurement gate. It is neither removed nor reported as passing.
All 29 accounting tests remain part of the profiling suite.


The column-only full conditional experiment still fails at `conditional_functions`.
Its trace attributes 373,032,217 VM steps and 5.485 CPU seconds to constructing the node relation.
Four node indexes cost another 4.365 CPU seconds. Ownership construction costs 4.295 CPU seconds and 108,719,114 VM steps.
These costs precede the later failure. Narrower function storage alone does not close the outer budget failure.
The [two diagnostic receipts](../gate/data-layer/fixtures/host/data-layer-port/profiling/conditional-budget-diagnosis-20261026.json) preserve both failures.
The subsequent run includes the narrowed import selector. Its result remains pending.

All 51 final production-shape tests pass. Static checks and the profiling suite pass.
The original corpus retains all 2,350 executable observations, supplemental controls, and 376 batch 6 guard repeats.
The quality and state groups are repeated after their final rule changes.
A controller gates fresh sequential full 50/60 runs on the complete conditional-clause check.
Those combined measurements disable tracing, census, replay copying, and external sampling.
No own build, test, or profiling workload will overlap their timing intervals.


The narrowed-import conditional check completes all 9,021 files with zero execution failures.
Its output matches the clause's empty repository diagnostic oracle. The original positive corpus also remains green.
The work allowances are unchanged. The [full validation receipt](../gate/data-layer/fixtures/host/data-layer-port/profiling/conditional-full-validation-20261026.json) records artifact identities and limits.
This closes the reproduced conditional execution failure; it does not establish combined-rule performance acceptance.
The frozen first-50 measurement starts next, followed by all 60. Neither run overlaps this agent's builds, tests, or profiling probes.


### Frozen first-50 result: parity restored, CPU target still missed (2026-10-26)

The latest first-50 run passes all 9,021 files and all 24,820 expected findings.
It reports zero false positives, false negatives, and engine failures.
Cold CPU is 672.782 seconds: 448.716 user seconds and 224.066 system seconds.
Cold wall time is 725.358 seconds. Compilation takes 0.830 CPU seconds and 0.836 wall seconds.
OS peak RSS is 4,960,436,224 bytes (4.620 GiB).
No own build, test, trace, census, replay copy, or external sample overlaps this timing run.

Cold CPU is 25.54% below the earlier failed 903.546-second candidate.
That earlier run omitted 4,931 findings and performed failure snapshot copying, so it is not a clean correctness-matched speed baseline.
The new complete result still uses 5.348 times the passing 125.804-second pre-SQL CPU reference.
Peak RSS is 2.86% above the earlier failed candidate. The sparse-column improvement does not establish full-workload RSS recovery.
The [checked receipt](../gate/data-layer/fixtures/host/data-layer-port/profiling/recovery-first50-full-20261026.json) retains counters and resource accounting.
Aggregate recovery remains rejected. The sequential all-60 measurement starts after this process exits.


### Frozen all-60 result: parity restored, deferred evaluation dominates (2026-10-26)

The all-60 run passes all 9,021 files and all 24,993 expected findings, with zero failures and diagnostic differences.
Cold CPU is 2,540.634 seconds: 1,855.030 user seconds and 685.604 system seconds.
Cold wall time is 2,611.045 seconds. Compilation takes 1.533 CPU seconds and 1.566 wall seconds.
OS high-water RSS through exit is 6,862,143,488 bytes (6.391 GiB).
The driver checkpoint is lower; comparisons use the OS figure for both runs.

Cold CPU is only 6.41% below the earlier failed 2,714.768-second candidate.
OS peak RSS decreases 45.57% from 12,607,602,688 bytes.
The earlier candidate reports seven failures and includes snapshot work, so the CPU comparison remains qualified.
No own builds, tests, traces, census, replay copies, or external samples overlap the new sequential measurements.
The [checked receipt](../gate/data-layer/fixtures/host/data-layer-port/profiling/recovery-all60-full-20261026.json) preserves identities, accounting, and the comparison limits.

The final deferred apply alone takes 1,392.224 CPU seconds and 1,446.935 wall seconds.
It includes deferred file analyses as well as repository-wide clauses; it is not one global SQL query.
Across the whole job, exclusive counters record 729.928 seconds in selection, 129.736 in index preparation, and 159.368 in physical planning.
The default worker phase records another 1,398.713 seconds, dominated by that deferred apply.
The next profiling slice must attribute this work to clauses and preparation stages before selecting another optimization.
Recovery remains open despite complete findings parity and the substantial all-60 memory reduction.


### Follow-up: apply import membership to both contract consumers (2026-10-26)

The checked policy still retained range-based import membership in `quality-contracts-2`.
The generator helper supports both plans, but the earlier checked-policy update covered only `quality-contracts-1`.
The bounded-work regression now executes every applicable native contract plan and fails on the second plan before the correction.
The checked policy now applies the same existing subtree producer to both consumers, with their independent file scopes.
All 16 shared-SQL tests and the complete original quality fixture group pass.
The [bounded comparison](../gate/data-layer/fixtures/host/data-layer-port/profiling/all-contract-import-recovery-20261026.json) preserves result-query CPU and external samples.
This policy correction follows the frozen full 50/60 measurements; those reports retain their original hashes.

The 128-file all-ten-rule profile completes within 14 seconds, with no engine failures.
Its 30 diagnostic differences reflect the open cross-file subset and do not establish corpus parity.
External bottom-up samples show allocator enumeration consuming a large part of traced execution.
CPU tracing currently calls `malloc_zone_statistics` at every selected SQL span boundary, even without a requested memory census.
The next instrumentation correction must make allocator enumeration explicit while retaining cheap RSS observations.
The accepted full runs use tracing off and are unaffected by this profiling overhead.


### CPU tracing must not run an allocator census (2026-10-26)

Red commit `b9117c57` proves that default CPU tracing calls the allocator probe and can inherit a census setting accidentally.
The profiler now removes inherited census configuration and enables allocator enumeration only for `--memory-census`.
Native trace spans retain cheap process RSS observations without traversing allocator zones.
The existing explicit memory request retains allocator values; no production execution alternative is added.
The same native memory implementation runs in the offline replay laboratory's regression tests.

The 128-file all-ten-rule trace previously enumerates allocator zones at 5,259 boundaries.
The corrected CPU-only trace records the same 5,259 RSS boundaries with zero allocator enumerations.
Its deferred CPU changes from 6.775 to 2.174 seconds. The new diagnostic overlaps Clippy and is not a controlled engine-speed comparison.
The explicit-census check retains allocator statistics at all 266 observed boundaries.
External samples identify allocator enumeration directly; the mocked-probe regression checks that it is never called in CPU-only mode.
The [checked receipt](../gate/data-layer/fixtures/host/data-layer-port/profiling/cpu-trace-overhead-recovery-20261026.json) records identities and qualifications.
Full repository acceptance timings use tracing off and do not contain this overhead.

The rebuilt DSL suite passes 1,540 tests, with 23 explicit ignores.
All 51 production-shape tests pass. Native and laboratory Clippy pass with warnings denied.
All 28 Python profiler tests pass, along with the complete profiling suite and static checks.
The frozen full 50/60 runs precede the final contract-2 correction and profiling-only change; their artifact identities remain explicit.
All original quality fixtures pass again after the contract-2 correction.

The next performance chunk must attribute the complete deferred apply with the corrected CPU tracer.
It must cover all ten additional rules, rather than assuming the quality/state slice represents their total cost.
The 128-file profile has no engine failures, but its open cross-file scope prevents parity acceptance.
Do not extrapolate its clause proportions to the 1,392-second full-repository apply.
Candidate selection remains a separate measured frontier: 347.871 CPU seconds for first 50 and 729.928 for all 60.


### Cold CPU architecture plan and exclusive attribution (2026-10-26)

The [new workplan](cold-cpu-recovery-plan.md) replaces execution-count targets with complete cold-resource acceptance.
It defines six bounded architectural spikes and a complete attribution pass before promotion.
Red commit `384988cc` detects double-counted nested CPU.
The corrected reporter subtracts immediate child CPU on the same thread and preserves unknown coverage.
Stages now use stable logical labels in traces. Consumer, admission accounting, release, and trimming have separate spans.
These changes add profiling only. They do not select a different production algorithm.
All 29 Python profiler tests pass. The instrumentation addon builds successfully.
The 128-file instrumentation check has no missing coordinates, partial overlaps, or CPU overdraw.
It records 9.017 observed CPU seconds, including 5.318 seconds without a complete rule/clause identity.
Its open repository scope does not establish finding parity.

A frozen full 9,021-file all-60 diagnostic is running under `.rerail/profiles/cold-cpu-spikes-20261026/attribution-full/`.
Native addon SHA-256 is `4eb3475f63a1a676d07a93e17110987fc61592d527720fdc28e10ebb8adfa518`.
It enables CPU tracing and external samples, and disables allocator enumeration and SQL snapshots on success.
Laboratory development can overlap it. Its time is diagnostic and cannot replace uninstrumented acceptance.
The new replay experiments use captured tapes and generic operators. They do not add production feature flags.


### Whole-demand operator and sampler corrections (2026-10-26)

The first architectural host and its comparisons are committed as `4208662f`.
All 68 laboratory tests pass. The native DSL passes 1,540 tests with 23 explicit ignores.
All 51 production-shape tests pass. Native and laboratory Clippy pass with warnings denied.
Static checks exposed a flat-directory limit; analysis helpers now live in `scripts/profiling/analysis/`.

The timing wrapper exposed an external sampler bug: it attaches to the launch PID instead of the workload child.
The full diagnostic's scheduled samples therefore describe `/usr/bin/time`, not engine execution.
They are invalid engine evidence and must not enter the CPU diagnosis.
Red commit `40f27d6b` reproduces this target-selection error.
The corrected sampler chooses the largest live workload process and records its PID.
All 30 Python profiler tests pass. Supplemental samples target Node PID 17694 directly.
The first supplemental capture shows SQLite schema/statistics work during deferred execution.
Its wall-time stack weights cannot be interpreted as CPU shares.

The native child prototype now fills bounded typed output columns in broad waves.
A regression covers 40,000 correlated requests and 80,000 output rows: one xFilter, two waves, at most 65,536 buffered rows.
The next measurement retains this shape; xNext serves completed columns.
No production execution alternative is added.


### Complete cold CPU attribution and architectural spikes (2026-10-26)

The [spike report](cold-cpu-spike-results.md) records decisions for every aspect of the [workplan](cold-cpu-recovery-plan.md).
The plan now contains four bounded implementation deliveries, their regression ladders, budget isolation requirements, and full-resource gates.
The [attribution receipt](../gate/data-layer/fixtures/host/data-layer-port/profiling/cold-cpu-attribution-20261026.json) and [experiment receipt](../gate/data-layer/fixtures/host/data-layer-port/profiling/cold-cpu-architectural-spikes-20261026.json) preserve identities and measured trials.
Raw evidence remains under `.rerail/profiles/cold-cpu-spikes-20261026/`.

The complete traced all-60 run passes across 9,021 files: 24,993 expected and actual findings, with zero FP, FN, or failures.
Compile CPU is 1.958 seconds. Cold CPU is 3,586.594 seconds; cold wall time is 4,308.203 seconds.
OS peak RSS is 7,530,659,840 bytes. This is diagnostic, with tracing and overlapping laboratory work.
It is not a replacement for the frozen uninstrumented 50/60 baseline or an isolated profiling-overhead measurement.

Exclusive attribution observes 3,204.752 CPU seconds. Shared or incomplete clause attribution accounts for 1,527.242 seconds.
The capture retains 103 overlap flags and seven CPU-overdraw flags among 1,953,732 events.
Analysis calls account for 679.109 measured seconds, candidate materialization 619.630, relational materialization 455.858, and relational indexes 341.870.
Trimming accounts for 190.128 seconds and release for 132.883. Span counts are not statement-submission counts.
The batch executor reaches 792 suspended SQL consumers in four partitions.
`quality-contracts-1` accounts for 776.922 measured clause seconds; state flow, authority sites, and legacy graphs follow.

Supplemental Node samples corroborate statistics reload and schema lookup overhead.
SQLite's bundled `sqlite3AnalysisLoad` reloads complete schema statistics after table analysis.
The automatically scheduled samples targeted the timing wrapper and remain invalid engine evidence.
The corrected sampler selects and records the workload child.
A full deferred vmmap snapshot shows substantial swapped allocator pages; falling current RSS is not memory recovery.
The separate 128-file ownership census records zero JSON projection fields and 6,977,240 typed projection bytes.
It does not explain every full-repository allocation.

The SQL controls preserve complete row multisets, including overlapping and empty scopes.
Stored statistics avoid the wrong anchor, while broad construction still dominates small-query total CPU.
Fusion wins over repeated inline work and the CTAS/index/ANALYZE lifecycle in the structural control.
The native first-field-child operator now fills completed typed columns in broad waves.
Its regression covers 80,000 requests and outputs, one xFilter, two waves, and a 65,536-row buffer bound.
Its complete CPU does not beat the scalar control; no native child CPU optimization is promoted.

The publication control isolates transaction shape at the production 1-MiB cache setting.
Median CPU changes from 3,458.821 ms per-file publication to 529.826 ms with 32-file packets and existing triggers.
Grouped statistics take 489.235 ms, with overlapping variation. The selected design retains current triggers and cache budgets.
Rollback, replacement, complete posting/statistic checks, and a pinned SQLite reader pass.
A separate 32-file trace records seven host submissions and 46 internal SQLite completions for packet publication.
Its 2,336 trigger entries are not additional host submissions.

The actual policy overlay first fails the existing 16-KiB authoring limit when broad fusion moves into authored SQL.
The bounded overlay retains that limit and preserves every authored option and default.
All 192 original quality observations pass, including 130 positive and 62 clean observations.
A small repository timing pair establishes no useful improvement from this policy-only cut removal.
Broad fusion therefore belongs after logical validation in the physical planner.

The real schema replay retains the existing bounded production cache. Its no-sharing control is not an unimplemented engine opportunity.
The composed model shows that removing laboratory fact reconstruction matters more than swapping the final operator.
That model is not an end-to-end production speedup and its component ratios are not multiplied.

The chosen implementation order is candidate demand/statistics and packet publication, the full quality pipeline, the next three expensive clauses, then measured ownership and reuse cleanup.
Demand gathering must precede private SQL schema creation.
No production algorithm flags, alternate rule interpreter, allowance increase, or Linear issue is added.
Full uninstrumented 50/60 resource acceptance remains required after implementation.

Native DSL validation passes 1,540 tests with 23 explicit ignores; all 51 shape tests pass.
The final laboratory suite passes 70 tests; the profiler passes 30.
The first combined native build/test command hits its 110-second deadline; the cached direct test binary then completes in 60 seconds.
Static checks and the profiling suite pass after moving analysis helpers under their own directory.
Final checks below confirm the last tooling changes.

Final static checks, the complete profiling suite, all 70 laboratory tests, and laboratory Clippy pass on the final tooling.
Native source is unchanged after the successful native DSL, shape, and all-target Clippy checks.
The final packet-trigger comparison uses the same 1-MiB cache as the per-file control.
Frozen primitive and publication-control binaries remain separate, with their one control-setting difference recorded in the receipt.


### Detailed schema lifecycle implementation plan (2026-10-26)

The [schema lifecycle plan](sql-schema-lifecycle-plan.md) expands the recovery plan's second delivery.
It defines four complete implementation deliveries: consumer/context separation, SQL family fusion, statistics maintenance, and retained-state cleanup.
Each delivery has concrete source locations, runtime responsibilities, a TDD ladder, and a complete resource gate.
This is a documentation change. It adds no production implementation or new performance result.

The detailed design distinguishes producer batching from SQL scope.
Native producers consume complete demand batches and expose completed typed columns.
Opaque authored SQL initially retains its original scope through a stable reusable context.
`partitionBy: file` does not prove that aggregates, windows, limits, or arbitrary subqueries distribute over files.
Cross-scope SQL batches therefore require an explicit partition-safe contract.
A general SQL partition rewriter and a new retry scheduler are not prerequisites for schema reuse.

Stable cut storage uses exact binding identities and dependency pins.
A suspended consumer retains its data handles and releases its SQL context.
The next execution rebinds its handles without constructing another private schema.
Necessary boundary output goes directly into its final backing or typed frame, with no second full-result copy for suspension.

Statistics maintenance follows the cuts that remain after fusion.
The initial context-reuse delivery can retain required analyses within the smaller schema.
Later changes require complete-cost evidence for each retained refresh.
Equal row counts do not establish equal distributions or fresh statistics.
The existing recovery plan and spike report now link this detailed design.

The new document passes local link checks and the writing self-check.
`git diff --check` and `npm run check` pass. No runtime tests repeat because this delivery changes documentation only.


### Schema lifecycle SC1: reusable backing and native bindings (2026-10-26)

The matching control addon and current policies are frozen under `.rerail/profiles/schema-lifecycle-20261026/control/`.
Its identity includes the final import-membership correction. No new full resource result exists yet.

Red commit `b1d36739` requires schema reuse, scope isolation, and bounded completed storage.
The laboratory red log shows 200 historical completed bindings retained, exceeding the 128-entry cache bound.
The green storage ladder now passes all three tests, including pinned complete and empty output.
The integrated relational ladder passes 45 tests.
A new test suspends 200 consumers with at most eight schema objects and one native module.
It resumes them in reverse order and checks each original input and syntax domain.

Schema templates now have identities independent of source versions.
Exact producer identity still determines whether completed SQL data can be shared.
Stable SQL backing adds an indexed execution key, and views expose only its active binding.
Input publication remains bulk and retains original SQL types and duplicate rows.
Native adapters rebind immutable sources instead of registering one module per consumer.
Open cursors retain their original frame; rebinding releases payloads after cursor closure.
Regex and source functions retain their registrations and bind the active budget and source.
Authored SQL still passes the original compiler authorizer before internal storage names exist.
Runtime metadata preparation uses only those compiled and engine-resolved queries.

This commit does not establish completion or a CPU improvement.
Statistics freshness, retained-cut inventory, cleanup, original observations, and full resource acceptance remain in progress.
The first complete native DSL invocation reached its 110-second deadline without a reported assertion failure.
The focused relational suite completes in 0.11 seconds after loading the test binary.


### Schema lifecycle: family reuse, statistics, and retained data (2026-10-26)

The implementation now separates reusable layouts from exact source-dependent data identity.
Suspended consumers share physical programs and immutable SQL text.
Data-key hashing uses a cached program fingerprint and still compares full definitions on collisions.
Native source sizes and access paths form bounded planning signatures.
An empty binding cannot freeze the prepared plan for a later populated binding.
Functions preserve their registrations while source and work bindings change.
The executor refuses to change scope while an SQL cursor is active.

Red commit `f40b4148` shows stale statistics after an equal-size distribution change.
Both populations retain the old `100 100 100` index estimate before the fix.
The selected implementation refreshes TEMP statistics at complete publication boundaries.
One `ANALYZE temp` reloads statistics for that batch.
It replaces per-consumer, per-cut analysis in managed backing.
Persistent corpus indexes retain their existing statistics and index shapes.
Repeated reads with no population change require no refresh.

Red commit `705af45b` shows four apparent reads for one table read with three column qualifiers.
The family compiler now excludes qualifiers, comments, and definite string literals from repeated-read counts.
It conservatively retains possible quoted table references.
Actual joins and correlated second reads still retain their authored index boundary.
The existing compiler continues to compose the complete validated clause.
Each capture now reports every logical stage, its retained or fused status, its reason, and its readers.
Remaining token-count uncertainty is identified explicitly.
No policy-only overlay or general SQL partition rewrite is used.

Red commit `b2e9e98a` exposes a collision between a public column and the private execution key.
Backing now chooses a distinct private column with SQLite-checked public metadata.
The public schema and stored values remain unchanged.
The storage ladder passes six tests, including final-reader cleanup and pinned empty completion.
Context layouts, completed bindings, native modules, and physical programs have bounded retention.
Input declarations remain outside input transactions, so rollback cannot remove a cached schema.
Last readers release their data pins; other suspended consumers retain theirs.
Recording retains its complete input snapshot until capture; acceptance disables recording.

The SQL executor also removed an unused metadata-only preparation before each stage execution.
Original SQL and composed SQL already pass compiler validation.
Native bindings enforce their schemas before execution.
Final result columns now come from the statement that actually executes.
This removes duplicate preparation without expanding the statement-cache allowance.

Before the final metadata cleanup, the complete native DSL passes 1,554 tests with 23 existing ignores.
The final addon passes all 192 original quality observations: 130 positive and 62 clean.
There are zero false positives, false negatives, engine failures, or execution-path differences.
The initial shape run requires two test updates for the changed backing format.
Snapshot checks now use the recorded logical alias in the TEMP snapshot.
Index checks require the private scope key before the authored lookup column.
All 77 laboratory tests and static and profiling checks pass at the preceding checkpoint.
Final source checks and matching full-repository cold measurements follow.
These checks do not establish a CPU or RSS improvement.


### Correct the statistics boundary before full acceptance (2026-10-26)

The first larger five-file quality control exposes four work-budget failures in the new executor.
The frozen control passes the same source domain without a failure.
These failures also occur without instrumentation, so capture overhead does not explain them.
The initial implementation is not accepted.

The affected `execution_calls` reader joins call paths to `calls.callee_id`.
Its plan scans call paths first and probes calls through `(__execution_key,__path)`.
The post-native call-path population has no statistics when this reader prepares.
Three failing executions consume roughly 23–33 million VM steps and exhaust regex or SQL work.
An ownership reader also exhausts its SQL allowance after roughly 125 million VM steps.
Refreshing statistics after each consumer finishes is too late.

Red commit `793b1aa4` requires a statistics boundary between a populated post-native family and its reader.
The coordinator now advances the existing ordered stage program in family frontiers.
It populates every admitted original scope for that family, trims retired data, and refreshes statistics before the next reader.
It gathers complete native demands before the existing broad count, scan, and scatter passes.
It does not add a generic graph scheduler, a domain union, or an SQL partition rewrite.

Admission includes consumer descriptor storage and shared tree footprints.
Actual native requests retain the original input and output capacity checks.
SQL families preserve each consumer's remaining work allowance across every transition.
Maintenance runs outside statement work accounting, as owner cleanup did before.
Ready-row eviction groups binding keys by backing table and deletes them in bounded batches.
Publication records existing inserted-row counts without additional counting queries.
Context observations now include prepared-statement and published-row counts.

The coherent-family addon passes the five-file quality domain with no failures or finding differences.
This is a bounded semantic and planning control, not full resource acceptance.
All 192 original quality observations also pass, including 130 positive and 62 clean observations.
The complete native DSL passes 1,555 tests with 23 existing ignores.
All 52 shape tests pass, including the new post-native statistics-readiness regression.
Final all-target native Clippy passes with warnings denied.
The cache-eviction and row-count instrumentation receive a final rebuild and verification before full timings.


### Frozen schema-lifecycle acceptance candidate (2026-10-26)

Candidate source is `2cc0a5a2ee74b1b5c6d19f8ab2058e2f5fc61b72`.
Frozen addon SHA-256 is `63bad0a538b78a8d6022123df6a100ff27311859130cbc298706f9c7bb729211`.
The final source passes 1,555 native DSL tests with 23 existing ignores.
All 52 shape tests, 78 laboratory tests, and static and profiling checks pass.
Native all-target Clippy passes with warnings denied.
All 192 original quality observations pass: 130 positive and 62 clean.
The larger five-file domain also has no failures or finding differences.

The fixed recorded-observation guard reports 349 engine submissions and 45 SQLite internal executions.
Its complete cold job retains exact positive and clean parity.
The preceding recorded count is 975. The 100 reference remains diagnostic.
These counts do not establish full-repository speed or memory acceptance.

Matching full 50- and 60-rule control and candidate runs now start sequentially.
Both use all 9,021 files, current matching policies, four native workers, and 32-file input packets.
The control includes the import-membership policy correction.
Detailed tracing, SQL capture, allocator census, external sampling, and overlapping worktree builds or tests remain off during these timings.
Compilation CPU is reported separately from cold CPU.
Process CPU and OS peak RSS also include final cleanup and remain distinct from the driver's cold phase.


### Reject the first full schema-lifecycle candidate (2026-10-26)

All four measurements use the complete 9,021-file, 137,924,324-byte repository.
The [machine-readable receipt](performance-decisions/schema-lifecycle-20261026-rejected.json) records identities, failures, compile costs, and OS resources.

| Rules | Artifact | Cold CPU, s | Cold wall, s | Peak RSS, bytes | Engine failures |
| --- | --- | ---: | ---: | ---: | ---: |
| 50 | Matching control | 743.541 | 890.458 | 4,738,318,336 | 0 |
| 50 | `2cc0a5a2` candidate | 829.704 | 1,066.183 | 4,699,488,256 | 0 |
| 60 | Matching control | 3,899.342 | 5,557.645 | 6,520,143,872 | 0 |
| 60 | `2cc0a5a2` candidate | 2,096.511 | 2,631.567 | 6,323,503,104 | 5 |

All findings match: 24,820 for 50 rules and 24,993 for 60 rules.
The 50-rule candidate regresses cold CPU by 11.6 percent.
The 60-rule candidate exhausts SQL work in five `runtime-reference-summaries` consumers.
Neither result passes acceptance. The apparent 60-rule improvement is not an accepted speedup.
Host load and wall time vary substantially; the receipt retains these observations.
They do not excuse either the CPU regression or the engine errors.

Captured failing queries expose a scope-index planning error.
A scalar scope predicate encourages SQLite to put input scans before the recursive frontier.
Two captured consumers take 818,453,316 and 1,250,315,587 VM steps after removing the original budget for laboratory replay.
An outer binding gate reduces them to 27,793 and 13,116 VM steps, with identical typed row multisets.
These are query-only diagnostic measurements, not full workload timings.
The laboratory changes only restored snapshot copies.

The statistics boundary also runs `ANALYZE temp`, revisiting unchanged relations at every family frontier.
The next correction tests outer scope gates and refreshes only dirty physical cuts.
The complete repository acceptance gate remains pending.


### Correct input scope planning and statistics maintenance (2026-10-26)

Red commit `617ddca9` exposes scalar scope gates and repeated analysis of unchanged relations.
Commit `94ca6865` makes imported inputs use an outer binding gate and refreshes only dirty physical cuts.
Authored stage cuts retain their indexed scalar binding lookup.
Applying outer gates to every cut changes a quality-rule plan and exhausts regex work in the five-file control.
That broader variant is rejected; the production change applies only to imported input relations.

A remaining interval join scans bindings through the scope-only index.
Red commit `803c075b` requires a selective imported span lookup to use fewer than 1,000 VM steps.
The original lookup uses 7,028 steps on the 1,000-row scope fixture.
Commit `1efb5768` replaces the imported scope-only index with `(binding,__path,__start,__end)`.
The scope prefix still supports cleanup. There is no second input index or per-consumer index build.
Stage indexes retain their authored columns after the binding prefix.
The [captured-query receipt](performance-decisions/schema-lifecycle-scope-plans.json) records matching complete typed multisets.

The three captured queries change as follows:

| Query | Original VM steps | Corrected VM steps | Rows |
| --- | ---: | ---: | ---: |
| Interval ownership | 20,960,663 | 6,179,714 | 1,801 |
| Recursive consumers A | 818,453,316 | 17,843 | 4 |
| Recursive consumers B | 1,250,315,587 | 11,288 | 4 |

These query-only results exclude original work allowances and report index construction as setup.
The production checks retain the original allowances and include index construction.
The 12-file failure batch, 32-file batching control, and five-file quality domain now pass without engine failures or finding differences.
All 192 original quality observations pass, including 130 positive and 62 clean observations.
All 52 execution-shape tests pass on the combined addon.

Statistics collection now targets dirty backing relations at each coherent family boundary.
The regression test leaves an observable sentinel on an unchanged relation and requires that it survive the next boundary.
Equal-size distribution invalidation still passes.
The lifecycle report adds the number of analyzed relations, separate from boundary and request counts.
Table-scoped ANALYZE still reloads SQLite statistics metadata; the bounded schema makes this a different cost from the old private-schema model.
It avoids re-sampling every unchanged relation at each family frontier.
Complete cold resource acceptance remains pending.


The combined source passes 1,558 native DSL tests, with 23 existing ignores.
All 81 laboratory tests pass; native all-target Clippy passes with warnings denied.
Static and profiling checks pass. All ten selected serverplan source batches pass.
Two short-deadline diagnostics timed out during concurrent compilation and then passed with 45-second deadlines.
Those overlapping diagnostic runs are not performance comparisons.
The fixed guard remains 349 host submissions plus 45 internal SQLite executions, with complete parity.
The 100 reference remains diagnostic rather than a resource acceptance substitute.

The combined addon is frozen at source `1efb5768` with SHA-256 `dd09f243ecc65ed798e597068c73f4eee11e79272ece9b990342c0c25b0a40df`.
It receives a paired 96-file timing diagnostic and complete corrected repository measurements next.


The [paired 96-file diagnostic](performance-decisions/schema-lifecycle-paired50-diagnostic.json) uses control/candidate/candidate/control order without overlapping builds or tests.
Mean cold CPU is 2.398 seconds for the control and 2.484 seconds for the candidate, a 3.6 percent increase.
The two control samples differ by 0.163 CPU seconds, more than the 0.086-second difference between means.
The small subset does not establish a speed improvement or isolate the complete 50-rule regression.
Both engines produce identical finding multisets and no failures.
Its global scope differs from the original full-repository recording, so its oracle-difference counts are not semantic acceptance results.
The final complete corrected 50- and 60-rule measurements now run sequentially with the same frozen artifact and policies.


### Corrected full 50-rule result: parity passes, resources do not (2026-10-26)

The corrected addon completes all 9,021 files with 24,820 expected and actual findings.
There are zero false positives, false negatives, and engine failures.
Cold CPU is 832.236 seconds; cold wall is 1,047.496 seconds.
Policy compile CPU is 1.088 seconds, reported separately.
OS peak RSS is 4,835,475,456 bytes.
The earlier matching control records 743.541 cold CPU seconds and 4,738,318,336 peak RSS bytes.
The corrected measurement fails the aggregate resource gate against that control.

Input processing uses 775.910 CPU seconds; repository-wide results use 55.967 CPU seconds.
The preceding rejected candidate uses 776.195 and 53.149 seconds for those same phases.
The new correction therefore does not establish an aggregate 50-rule improvement.
The original control uses 699.252 and 43.931 seconds.
The slowdown also affects parsing and anchor operations outside the changed relational executor.
Both artifact dependency lists and the recorded allocator SHA-256 match.
Those observations motivate one contemporary frozen-control repeat; they do not establish the cause or excuse the failed gate.
The repeat was declared before the 60-rule result and will retain both earlier control measurements.
It uses the complete original repository and 50-rule set, without sharding or reduced policies.
The measurement controller now accepts an explicit complete rule-set selection for this purpose.
The corrected 60-rule measurement continues first, without overlapping worktree builds or tests.


### Stop the next timing candidate for incomplete input statistics (2026-10-26)

The corrected 60-rule run is stopped during input processing, after about 440 process CPU seconds.
It has no complete parity or resource result and cannot serve as an acceptance measurement.
The complete 50-rule failure remains recorded unchanged.

Review identifies a concrete SC3 omission after adding imported span indexes.
`populated` marks imported data but does not mark those indexes for analysis.
Targeted stage maintenance therefore stops refreshing the input statistics that the preceding whole-TEMP analysis supplied.
SQLite prepares readers with default input estimates even after a complete input publication.
This is a planning defect, not a proven explanation for the unchanged ingestion slowdown.

Red commit `dde47582` requires input statistics after publication, removal, and an equal-size distribution replacement.
The first input publication fails because no statistics relation is created at all.
The correction marks imported indexes dirty on publication and retirement.
Incomplete-binding cleanup also invalidates its statistics.
Refresh stays at the existing coherent owner boundary and targets only changed relations.
All ten focused production storage tests pass in the lightweight laboratory host.
The original input row and query work allowances remain unchanged.
The fresh frozen-control repeat and final candidate measurements follow after broad validation of this correction.


The input-statistics correction passes 1,559 native DSL tests, with 23 existing ignores.
All 82 laboratory tests, all 52 execution-shape tests, static checks, profiling checks, and all-target native Clippy pass.
The original 192 quality observations retain exact parity: 130 positive and 62 clean.
All ten selected serverplan source batches and the five-file quality domain pass.
Four 12-second diagnostics hit their deadline during concurrent validation, then passed without engine errors at a 25-second bound after compilation finished.
The fixed guard retains exact parity and reports 357 host submissions and 69 internal SQLite executions.
The eight additional host statements publish and retire input statistics at the owner boundaries.
Counts remain diagnostic.

Final source is `2452e9611c50808ee30884abdab7310867a2fce8`.
Frozen addon SHA-256 is `7d3c667eb238ad3b4ec0ec09e989c06b2165e2fccb21bca83af76fa833518cf9`.
Frozen control and current policy identities match exactly.
A contemporary complete 50-rule control repeat now precedes the complete final 50- and 60-rule candidate runs.
They execute sequentially, without overlapping worktree builds, tests, detailed tracing, SQL capture, or allocator census.
Both earlier control results and rejected candidates remain part of the record.


### Final 50-rule measurement and unchanged control repeat (2026-10-26)

The unchanged frozen control completes its one declared repeat with exact 24,820-finding parity and zero engine failures.
It uses 814.560 cold CPU seconds, 1,013.311 cold wall seconds, and 4,861,984,768 peak RSS bytes.
Its compile CPU is 1.125 seconds, separate from cold execution.
The earlier identical control uses 743.541 CPU seconds and 4,738,318,336 peak RSS bytes.
The 9.6 percent CPU difference between unchanged controls establishes substantial measurement variation; it does not identify its cause.

The final `2452e961` candidate also has exact 24,820-finding parity and zero engine failures over all 9,021 files.
It uses 764.567 cold CPU seconds, 876.763 cold wall seconds, and 4,677,632,000 peak RSS bytes.
Its compile CPU is 1.064 seconds, separate from cold execution.
Against the adjacent control repeat, CPU decreases 6.1 percent and peak RSS decreases 3.8 percent.
Against the earlier control, CPU increases 2.8 percent and peak RSS decreases 1.3 percent.
A robust first-50 CPU improvement is therefore not established.

Final input processing uses 717.876 CPU seconds and global results use 46.373 seconds.
The adjacent control uses 763.851 and 50.398 seconds respectively.
Both phases vary; the complete difference cannot be attributed solely to SQL context reuse or statistics maintenance.
The first-50 125.804-second recovery target remains unmet.
The complete final 60-rule measurement now runs with the same frozen artifact and matching policies.


### Full input-statistics result: CPU win, later cleanup RSS failure (2026-10-26)

The [complete receipt](performance-decisions/schema-lifecycle-input-statistics-full.json) retains both controls and final full sets.
The 60-rule run has exact 24,993-finding parity and zero engine failures over all 9,021 files.
Cold CPU decreases from 3,899.342 to 1,894.330 seconds, a 51.4 percent reduction.
Cold wall decreases from 5,557.645 to 2,095.695 seconds.
Compile CPU is 1.726 seconds, separate from cold execution.
Input CPU changes from 1,515.347 to 1,416.522 seconds.
Global-result CPU changes from 2,383.574 to 477.478 seconds, an 80.0 percent reduction.
This places roughly 75 percent of the remaining cold CPU in the input phase.

The earlier driver peak sample is 6,571,507,712 bytes.
The authoritative OS receipt reaches 7,372,029,952 bytes through complete process cleanup.
The matching control OS peak is 6,520,143,872 bytes.
The full memory gate therefore fails by 13.1 percent, rather than the apparent 0.8 percent from the pre-close sample.
The earlier RSS comparison was incomplete. No memory acceptance is claimed.

Code inspection identifies a real native payload lifetime defect.
Stable module registration does not populate each lease's old private-module list.
Final-query native sources can therefore remain bound after the owning lease completes, until another lease starts or the cached owner closes.
Red commit `2c0ff6c8` proves this with a weak reference to a native row payload.
The test also requires that dropping an inactive lease preserve another active owner's source.
The correction clears active native bindings when their lease completes and preserves reusable schema metadata.
The focused test now passes without rebuilding the schema.

A four-file, 60-rule diagnostic reports about 860 MB peak RSS before the correction and 788 MB afterward.
Both variants produce the same diagnostic findings and no engine failures; the restricted domain is not full-repository parity.
Its census and overlapping validation make these diagnostic observations, not resource acceptance measurements.
The lifetime test proves retention. The late full-process peak still needs a complete corrected measurement.

The harness now writes a separate close receipt without moving the original cold/report sequence.
Both ordinary and census runs record the first actual close, including its CPU and before/after peak RSS.
The complete measurement controller now uses the OS peak as its primary RSS result and retains the earlier driver sample separately.
A regression test uses the observed 6.57/7.37-GB mismatch to prevent a future false memory gate.


The native frame retirement correction passes 1,560 native DSL tests with 23 existing ignores.
All 192 original quality observations pass, including 130 positive and 62 clean observations, with no execution-path differences.
All 52 execution-shape tests pass on an identical retry.
The preceding concurrent shape run had one missing child-profile-record assertion; its finding assertions passed. No test or production change was made for that retry.
Static checks, all-target native Clippy, and the profiling checks pass.
The profiling test now requires the observed late OS peak to override the earlier driver sample, while retaining both values.

The corrected four-file diagnostic retains identical finding multisets and zero engine failures.
Its recorded peaks are 859,832,320 bytes before the fix and 787,988,480 bytes afterward.
These are bounded, census-enabled diagnostics, not full-repository acceptance.
The complete native payload retirement artifact receives new complete 50- and 60-rule measurements next.
The cold computation and report order stay unchanged; close CPU/RSS are additional sidecar observations.


Frame-retirement source is `c2f1f2e29cdc048b6d6d865d8b8df6dea18dc0fc`.
Frozen addon SHA-256 is `fcbd28d41d16b4c9dade8b25c071d3bfd591f44c0cfb4f4ae52337657abac28e`.
The larger source batches and five-file quality domain pass without finding differences or engine errors.
The fixed guard remains 357 host and 69 internal SQLite executions, with complete parity.
A new complete 50/60-rule measurement starts sequentially on the frozen artifact, without overlapping builds or tests.
The native close stays in its original position; only its additional CPU/RSS sidecar and the controller's primary RSS source change.
Comparison uses the previously recorded authoritative OS resources, not the old driver's earlier RSS sample.
The 125.804-second first-50 recovery target and source ownership frontier remain outside this schema-lifecycle delivery.


### Final frame-retirement measurements: semantic pass, memory gate still failed (2026-10-26)

The [final machine-readable receipt](performance-decisions/schema-lifecycle-final-20261026.json) records complete artifact, driver, workload, compile, cold, close, and OS resource observations.
Both complete runs use all 9,021 files and matching unchanged policies.
The 50-rule run matches all 24,820 findings; the 60-rule run matches all 24,993 findings.
Both have zero false positives, false negatives, and engine failures.

| Workload | Artifact | Cold CPU, seconds | Cold wall, seconds | OS peak RSS, bytes |
| --- | --- | ---: | ---: | ---: |
| 50 rules | Original control | 743.541 | 890.458 | 4,738,318,336 |
| 50 rules | Unchanged control repeat | 814.560 | 1,013.311 | 4,863,492,096 |
| 50 rules | Final frame-retirement candidate | 794.814 | 935.626 | 4,673,306,624 |
| 60 rules | Matching control | 3,899.342 | 5,557.645 | 6,520,143,872 |
| 60 rules | Final frame-retirement candidate | 1948.767 | 2235.125 | 7,579,385,856 |

Policy compile CPU is 1.089 seconds for 50 rules and 1.902 seconds for 60 rules, separate from cold execution.
The final 60-rule CPU reduction is 50.0 percent.
Its OS peak RSS increases 16.2 percent against the matching control.
The strict complete resource gate therefore remains failed. This branch is not an accepted memory baseline.
The first-50 CPU result remains within the observed control variation; no robust gain is established.

The native payload lifetime defect is fixed and its regression test passes.
Its bounded diagnostic improvement does not establish a complete repository memory improvement.
The new close receipt records 7,350,648,832 peak bytes before close and 7,579,336,704 after close on the 60-rule run.
The complete OS peak is 7,579,385,856 bytes.
The pre-close computation itself also reaches a high peak; the remaining memory problem cannot be attributed only to final close.
An attempted external memory-map capture found that Node had already exited and collected no live profile.
The remaining full-repository memory cause has not been established by this experiment.

Input processing consumes 1420.769 CPU seconds and global results consume 527.666 seconds.
Input preparation, matching, and fact construction therefore form the main remaining CPU frontier.
Parser/source-graph ownership and additional memory attribution require their own evidence before changes.
The 125.804-second first-50 target remains unmet.
SC1–SC4 implementation and semantic checks are complete; resource acceptance stays explicitly open on RSS.
No new Linear issues or production optimization flags were added.


### Recover rule-port continuation after the performance work (2026-11-03)

The [restart plan](data-layer-port-restart.md) and [machine-readable inventory](../gate/data-layer/fixtures/host/data-layer-port/restart-20261103.json) recover the complete original 142-check scope.
A fresh read-only Git inventory confirms 60 candidate policies matching batches 1–6 exactly, with 82 unported checks in batches 7–15.
The associated-test survey has no parse errors. Batch seven has ten associated modules and 168 test definitions before parameter expansion.
These are survey results, not new oracle, native parity, or performance passes.

The existing corpus recorder and repository recorder accept only batches 1–6.
Batch-seven work must extend the real analyzer and test mappings, including `check_versioned_merged_identity.py`.
The restart plan links each of its ten source requirements and keeps signing, reporters, and retained integration cases visible.
Stale pack metadata claiming 48 candidates and incomplete fifth-batch parity is corrected; the activation list stays empty.

Newer integrated and final-shape engine branches have changed policy semantics and execution contracts.
The restart preserves this branch's source and evidence, identifies those successors, and requires an executable reduced-rule gate before targeting a newer engine.
No automatic merge, rule regeneration, or sibling-worktree edit was performed.
The frozen October addon no longer loads because its temporary allocator library is absent; the temporary Cargo target is also gone.
Existing full-run receipts remain historical evidence, including the failed OS RSS gate.
No new candidate or Linear issue was created during recovery.


### Restart batch seven through the integrated production API (2026-11-03)

A clean, isolated worktree at integrated revision `9bc955cc5` builds with the normal allocator in 2 minutes 11 seconds.
The frozen addon SHA-256 is `4f356c1cb4bbc8aacd35eea337adcfc543eef33f4905c48b59c6c4b31674d181`.
No sibling worktree files or in-progress logic changes enter that build.
The current production API omits the legacy execution-owner class and accepts relational clauses only in test builds.
The replay harness now bridges the old owner API and the integrated compiled-rule/workspace APIs; its exact scoring dimensions remain unchanged.
The first publication rule passes its original positive and clean observations on both production APIs after a failing missing-policy regression.
This is one working port, not batch-seven completion or cumulative 70-rule validation.

The unchanged pinned upstream batch-seven suite passes 208 tests in 8.07 seconds, including original root conftest and configuration.
Source-blob verification has no mismatches.
Its new recorder captures 150 eligible observations: 108 positive and 42 clean.
It retains four aggregate post-op/signing observations without source-bearing arguments and one empty-domain missing-constructor observation.
There are 53 uncaptured helper/integration tests.
The post-op dispatch original corpus has ten positive observations and no clean observation, so supplements are required before parity certification.


## 2026-11-03 — Restart batch seven: seven local and graph candidates

Status: partial delivery. Batch seven still requires three checks and cumulative validation.
Jeremy approved integrated-engine compatibility and capabilities as part of the remaining work.

The seven candidates pass 163 recorded observations: 109 positive and 54 clean.
Snapshot and resident-workspace APIs have zero finding differences and zero failures.
The [receipt](../gate/data-layer/fixtures/host/data-layer-port/batch-07-local-evidence.json) records policies, addon identity, and every replay result.
The integrated addon is a clean normal-allocator build of `9bc955cc5`.
These small source observations are correctness evidence, not a full-repository performance comparison.

The TDD ladder first exposed incorrect deletion targets, empty defaults with comments, keyword arguments, and chained identity comparisons.
Additional probes exposed import aliases, nested history paths, production extraction triggers, and differences between text and structured diagnostic anchors.
The seven-check regression gate now passes, including original positives and clean controls for each check.

The grammar rule initially failed with a correlated `calls` matcher.
Existing evidence relations and partitioned name closure represent the required graph without that matcher extension.
No missing feature ticket is justified by that failed formulation.
Graph probes cover cycles, constructor/runtime overlap, class ownership, nested functions, and arbitrary helper chains.
A recursive structural utility handles nested subscripts without a fixed depth.

The original recording preserves five retained integration calls and 53 uncaptured helper/integration tests.
The remaining source candidates are `post_op_side_effects`, `post_op_no_internal_dispatch`, and `versioned_merged_identity`.
The current integrated addon also rejects ten legacy candidates at the removed `relational` clause boundary.
Compilation rejection is not a parity result. The previous 60-rule baseline has not passed on that addon.
No Linear issue was created.


## 2026-11-03 — Batch seven: native contracts and cumulative rejection

Status: ten candidate rules pass local observations. Cumulative acceptance remains open.
The user included integrated-engine compatibility and capabilities in this delivery.

The source recording contains 148 eligible calls: 104 positive and 44 clean.
The 62 supplemental calls contain 38 positive and 24 clean observations.
All ten candidates pass snapshot and resident-workspace replay without finding differences or failures.
Seven additional captured calls retain their findings as legacy-helper observations.
Production code does not instantiate that helper visitor, so these calls are not detector passes.
The unchanged upstream tests pass 208 tests. There are 53 uncaptured helper, CLI, or integration tests.

Real ports required three generic contracts: production relational compatibility, configured SQL text escaping, and detached Ed25519 verification.
SQL text options become UTF-8 hexadecimal text expressions, including quotes, NUL, and Unicode.
Signature verification operates on artifact rows and returns false for malformed input.
The RFC 8032 vector, tampered input, and malformed input pass through the production N-API boundary.
A separate regression preserves declared file-level diagnostic anchors on relational inputs.
The original 60 candidates compile through that boundary.
This compatibility implementation does not lower authored relational clauses into the newer wave/GPU execution model.
No new Linear issue was created.

The first complete 70-rule run matches all 24,993 expected findings, but reports 22 engine failures.
Twenty-one legacy execution partitions report interrupted SQL queries.
One state-analysis clause exceeds its repository-input budget.
These failures reject the run; matching finding counts do not establish parity.
Compilation uses 1.418 seconds of CPU and 1.475 seconds of wall time.
Cold execution uses 646.248 seconds of CPU and 567.704 seconds of wall time.
OS peak RSS reaches 14,283,554,816 bytes. This is not a memory improvement.
The addon SHA-256 is `3723d86bb33b5c5255b4b0f142349976e4b54c11d6bec4d6fe01be472ba19461`.
The workload SHA-256 is `8dcd26660991ff4fe64021ffd17947845ba99879c2d00c909728754900e9c1b9`.
Local receipts are under `.rerail/data-layer-batch-seven/suite-70*`.
This run precedes two supplemental fixes for case-sensitive authority matching and Python-compatible signature decoding.
It is retained as a failed compatibility measurement, not a final policy benchmark.

The pinned Python oracle uses 152.746 seconds of analyzer CPU and 199.203 seconds of total wall time.
Its OS peak RSS is 1,947,811,840 bytes across all 9,021 files.

A production regression now proves that compatibility loaded source before empty-gate admission.
It also proves that syntax-only consumers requested unrelated source across the entire rule scope.
The fix queues file identities, admits the gate, and loads syntax only for its own gated files.
Explicit source-text queries retain aggregate byte limits after admission.
The reduced tests and all ten candidate gates pass on addon `550fb2943642d50f268fa420ad5a433956c9f61b777878a99e224ab3b9fe15fa`.
The edit harness passes 40 oracle edit transitions and ten removals compared with a fresh empty workspace.
A closed-domain rule can report an absence after removal; the harness does not assume an empty finding set.
Legacy interruptions remain under investigation. The cumulative gate has not passed.


## 2026-11-03 — Compatibility: identify and remove the correlated legacy scan

The complete legacy intent source reproduces an interrupted final query.
The unchanged pinned reporter returns zero findings on that same file.
The regression reuses the full source already present in `batch-06-recording.json`.
Its byte count and SHA-256 are recorded in the [engine regression receipt](../gate/data-layer/fixtures/host/data-layer-port/batch-07-engine-regressions.json).

SQL instrumentation identifies the `LEG-083` arm.
It starts from syntax nodes and asks a correlated question over eager anchors and active functions.
SQLite scans functions repeatedly, even when the eager-anchor relation is empty.
The interrupted result statement executes 72,992,521 VM instructions and 2,538,677 full-scan steps.
It consumes 660.179 milliseconds of thread CPU before interruption.
An extra index on `function_symbols` does not clear the failure. That experiment was removed.

The corrected arm joins from the existing eager anchors to functions and syntax nodes.
It retains the source condition, allowlist, diagnostic span, identity, and work budget.
The result statement completes with 657,518 VM instructions and 124,773 full-scan steps.
It uses 7.300 milliseconds of thread CPU on the same complete source file.
This is a statement-level comparison, not an end-to-end performance claim.
The production-API regression passes, and the generator reproduces the corrected policy exactly.

The state-analysis diagnostic also passes across all 9,021 files after deferred source preparation.
It reports zero findings, zero differences, and zero failures.
Cold CPU is 73.063 seconds; wall time is 75.983 seconds; reported peak RSS is 2,651,701,248 bytes.
That single-rule diagnostic preserves the complete repository domain.

The integrated frontend had also removed fields still implemented by its native compiler.
Compatibility now restores validation for source gates, line scans, capture columns, helper closures, and relational clauses.
All 70 candidates pass the selected frontend and native compiler.
Modern embedded-language schema behavior is retained.
The selected implementation still executes legacy relations through the compatibility evaluator, outside the wave/GPU lowering path.
A durable integrated-schema compile harness records all policy and addon identities.
The corrected cumulative run is in progress; no final performance or parity claim is made here.


## 2026-11-03 — Corrected 70-rule repository gate

All 70 candidates now match the complete 9,021-file oracle domain.
Expected and actual findings both equal 24,993, with zero extra findings, missing findings, or engine failures.
All ten new rules are clean on this production repository; the separate positive corpus remains required.
The source pin and workload SHA-256 match the earlier rejected run.
All final policy digests are present in the local full-run receipt.

| Measurement | First rejected run | Corrected run |
| --- | ---: | ---: |
| Compile CPU | 1.418 s | 1.723 s |
| Compile wall | 1.475 s | 1.741 s |
| Cold CPU | 646.248 s | 465.234 s |
| Cold wall | 567.704 s | 382.467 s |
| OS peak RSS through close | 14.284 GB | 15.323 GB |
| Engine failures | 22 | 0 |

The first column is an incomplete execution, not an accepted performance baseline.
The corrected run restores coverage while memory increases. It does not close the memory gate.
The pinned Python oracle uses 152.746 seconds of analyzer CPU and 1.948 GB peak RSS.
The native cold phase remains about three times that CPU and eight times that peak memory.
The timing boundaries differ: native includes ingestion and workspace execution; Python reports shared analyzer execution.
These measurements do not establish an improvement against the historical 60-rule baseline.

The corrected addon is `550fb2943642d50f268fa420ad5a433956c9f61b777878a99e224ab3b9fe15fa`.
The local receipt is `.rerail/data-layer-batch-seven/suite-70-admitted-result.json`, with a separate close receipt and OS log.
RSS is already about 12.5 GB after file ingestion, before repository clauses run.
That localizes most retained memory to preparation/workspace state rather than the final SQL query alone.
A future memory investigation must measure retained evidence and capture ownership at that boundary.
This delivery does not reopen the broader optimizer program or claim wave/GPU migration.


## 2026-11-03 — Broader restart regression gates

The first-50 conformance tests pass alongside all ten batch-seven tests and the production compatibility regressions: 41 tests pass.
The sixth-batch corpus passes all 1,259 independent observations: 417 positive and 842 clean.
Its 24 diagnostic shards preserve every observation's complete file domain.
All findings, multiplicities, and snapshot/workspace results agree, with zero engine failures.
Per-shard positive/clean gaps are aggregated across the complete corpus rather than treated as detector failures.

The integrated frontend's existing schema/options tests and new compatibility tests pass all 32 tests.
The candidate and integrated worktrees both pass typechecking.
The full native suite initially reports 2,509 passes and seven failures.
Six failures result from missing pack fixtures, including a widened-walk sentinel with an empty corpus.
After restoring Python, SQL, and Vue fixtures with matching lock files, all six affected tests pass.
A complete rerun reports 2,515 passes, one failure, and 67 ignored tests in 58.71 seconds.

The remaining failure is the inherited admission experiment's snapshot-hash guard.
Both files it checks are unchanged from `9bc955cc5`, and the base already contains the same mismatch.
The actual region hashes to `25dd37caf0584afab85c4ed54d00c5d6e34e3f456ec3de2cec916eee29cfae0f`.
The retained experiment expects `f9b8077c59090eefd5d84c92a2323114889733610a5780d65c452ff63de27b50`.
This work does not refresh an unrelated experiment snapshot to make the broad suite green.
This qualification remains separate from the passing data-layer gates.

The integrated compatibility implementation is committed as `d627273f67aaa3504e92905a677de2ea6370bc56`.
The production addon is being rebuilt from that sealed commit for the final reproducible receipt.
The earlier accepted complete run used the same execution changes before the profiling-only exhaustive-match fix.
Its source-derived build identity therefore differs; its checksum and measurements remain recorded rather than relabeled.


## 2026-11-03 — Seal batch seven against the committed engine

The final production addon is built from the committed native sources at `d627273f67aaa3504e92905a677de2ea6370bc56`.
Its SHA-256 is `fee334990d0f21843779fd305f720598b49965b1b0e3258d20596b60185a2126`.
The profiling-only match fix changes the source-derived build identity, so the earlier addon was not relabeled.
The complete final run again matches all 24,993 findings over 9,021 files, with zero engine failures.
Compile CPU is 1.356929 seconds; compile wall time is 1.350852 seconds.
Cold CPU is 395.518833 seconds; cold wall time is 321.603251 seconds.
OS peak RSS through close is 15,398,617,088 bytes.

The preceding accepted measurement used 465.234 CPU seconds and 382.467 wall seconds on the same execution changes.
There is no additional execution optimization between these accepted runs.
The spread remains unexplained; host conditions and process variation are possible causes, not established explanations.
Do not present this repeat as a further optimization gain.
Memory remains far above the Python oracle and the historical memory gate.

The final addon passes all 41 first-50, batch-seven, and production compatibility tests.
It also passes all 1,259 sixth-batch observations, all 210 seventh-batch observations, and all 50 edit/removal transitions.
The 24 sixth-batch shards retain complete observation domains and aggregate positive/clean coverage across all ten checks.
Both worktrees pass typechecking. The 32 integrated frontend/schema/options tests pass.
The broad native test qualification remains: 2,515 passes, one inherited snapshot failure, and 67 ignored tests.

The [batch receipt](../gate/data-layer/fixtures/host/data-layer-port/batch-07-evidence.json) records coverage, retained calls, source identities, and policy hashes.
The [engine pin](../gate/data-layer/fixtures/host/data-layer-port/batch-07-engine.json) records the companion branch and build contract.
The [full measurement](performance-decisions/data-layer-batch-seven-20261103.json) retains the complete execution and close receipts.
There are now 70 candidate policies out of the pinned 142 checks, leaving 72.
The pack remains inactive. Signing workflows, uncaptured integration tests, and wave/GPU migration remain separate contracts.
No Linear issue was created.


## 2026-11-03 — Require positive coverage from the native engine

The unchanged upstream suites for all 70 rules collect 4,227 tests: 4,224 pass and three retain their original skips.
The native audit matches 3,427 observations: 1,777 positive and 1,650 clean.
All 70 rules have both native positive and native clean coverage.
There are zero extra findings, missing findings, engine failures, or snapshot/workspace differences.
Three host signature observations remain separate and do not contribute to native coverage.

Two TDD ladders strengthen acceptance.
First, five newly captured original authority-helper observations fail against the previous post-op entry-point handling.
The corrected helper modes match two positive and three clean observations without adding unrelated production findings.
Ten always-clean candidate controls fail against original positives, and a fixture gate requires coverage for 35 diagnostic codes.
Two new oracle probes cover the history sentinel and stale fate override.
Batch seven now passes 217 native observations: 146 positive and 71 clean.
Its original 208 pytest tests pass; seven unused-visitor calls remain outside production replay.

Second, a host-only signature positive plus one native clean result previously passed replay certification.
The new regression fails before the fix.
Replay now requires native positive and clean coverage for each rule, separately from host-guard observations.
All 54 focused conformance tests and typechecking pass after this change.
The sixth-batch corpus again passes all 1,259 observations in bounded complete-observation shards.

Fresh upstream tests expose two setup requirements for signing/CLI tests.
The archive lacks the Git metadata needed for their committed-baseline read.
A real worktree supplies it, but inherited `GIT_PAGER` still violates the upstream guard's environment whitelist.
Removing only that variable makes all 933 sixth-batch tests pass.
No scan guard or upstream assertion was disabled or changed.

The [testsuite receipt](../gate/data-layer/fixtures/host/data-layer-port/testsuite-positive-coverage-20261103.json) records per-rule native positive and clean counts.
The [coverage report](data-layer-testsuite-coverage.md) separates oracle tests, native observations, controls, and remaining exclusions.
The resolved production post-op program has identical before/after SHA-256 `e6d5f2ff3ab1166ae96aa9718231568520f7da2bd022645e025d02a5fce732d8`.
The historical repository benchmark remains unchanged; these tests do not establish a new performance result.
No Linear issue was created.

## 2026-11-03 — Batch eight: first four native ports

The previous 70-rule checkpoint is committed and pushed at `4c6517314`.
Batch eight records the next ten unchanged source linters at the same source pin.
The original tests pass: 456 tests, with no source mismatches.
The recorder captures AST-backed source inputs without adding source reads or AST walks to the observed analyzer.
This preserves tests that count parser work and tests that supply parsed trees for virtual paths.

Four candidates pass the first native parity ladder: read-through authority, authentication plan purity,
legacy callback quarantine, and lazy adapter ownership.
They match 35 original observations: 28 positive and seven clean.
Both snapshot and workspace execution agree, with no extra findings, missed findings, or engine failures.
The initial authority test was committed red in `18572162d`.
Its missing finding exposed a real difference between direct scanner calls and production source-trigger admission.
The callback tests also require separate repository and single-file path filters.
The ports preserve those entry-point contracts.

These four candidates are a partial delivery. Batch eight is not complete.
The cached policy rule requires byte-exact CPython AST fingerprints; its implementation choice remains pending.
No performance claim or Linear issue is added.

### Batch eight: helper aliases and SQL surface

The next TDD commit (`804e00167`) adds original helper observations and 42 supplemental oracle observations.
The helper tests first fail on seven repository path exclusions.
The SQL surface tests first fail because the candidate is absent, then on a bare authority context expression.
Both corrections preserve the original entry-point semantics.
Six candidates now pass 146 native observations: 106 positive and 40 clean.
The helper resolver uses a versioned alias relation with ordered fallback to earlier successful bindings.
It does not execute one query per alias or store an environment in a JSON field.
The SQL surface tests cover each forbidden method, context attribute, bad type, and execution receiver family.
They include source-trigger and allowed-file controls.

The source pin has no dedicated lazy SQL surface test module.
Supplemental expectations come from its unchanged analyzer, with source files written to a temporary repository.
These probes supplement upstream tests; they do not count as upstream pytest cases.

### Batch eight: purity source and call-graph ladder

The purity recorder now includes facade sources read by the original export check.
It keeps the selected analysis paths separate from those auxiliary files.
It also preserves source text for cached-policy facts constructed from virtual paths.
Source hashes computed by the original linter are not supplied as native facts.

The initial purity replay misses three observations.
The failures identify export reachability, a NULL-sensitive global-name predicate, and frozen dataclass decorators.
After correction, all 42 original purity observations pass through both native APIs.
Together, the seven candidates pass 188 observations: 131 positive and 57 clean.
Further semantic probes, repository parity, and the three remaining candidates are still required.

### Batch eight: eager-resolve helper parity

The eighth candidate matches all 59 recorded original eager-resolve observations: 45 positive and 14 clean.
The first semantic ladder exposes duplicate phase identities and incorrect taint from canonical legacy helper wrappers.
Phase findings now use the linter's function/pattern identity.
Canonical wrappers suppress only the specific legacy resolver seed that the original linter removes.
The native executor still owns cycle-safe SQL propagation in both directions: phases reach callees; eager patterns reach callers.

The original AST helper wrappers call `ast.unparse` before scanning.
The recorder now preserves that actual scanner input, rather than comparing normalized diagnostic lines with pre-normalized source.
It also records the effective external-I/O allowlist used by each observation.

For lazy architecture, 84 distinct source inputs from original fact tests are evaluated by the unchanged full analyzer.
These are supplemental source probes, not replacements for the original fact assertions.
Seven additional probes bring positive coverage to all 18 LA diagnostic families.
The ninth and tenth ports, broader semantic probes, and repository acceptance remain open.

### Batch eight: lazy architecture, all diagnostic families

Nine candidates now pass 428 native observations: 263 positive and 165 clean.
The original 456 pytest tests still pass without source changes.
Lazy architecture covers all 18 diagnostic codes, including supplemental full-analyzer evaluations of original fact-test source inputs.
Snapshot and workspace results agree with zero extra findings, missing findings, or engine failures.
The focused native run completes in 13 seconds.

The first lazy-architecture ladder fails on anchor contracts and repeated alias scope searches.
Rollups now anchor to actual syntax spans.
Alias scope ranges are prepared once and indexed by binding name and position.
The original two real collect-results source files no longer exhaust the SQL work budget.
Raw deleted-surface checks use first-occurrence regular expressions, including comments and overlapping surface names.
They do not add a regex-evidence engine extension solely for this rule.

Three original fixtures provide ASTs from one text representation, then use `ast.unparse` as a fallback file reader.
Their syntax facts and raw-text findings therefore have different coordinates.
The recorder observes both unchanged producers and partitions their actual results by object identity.
It preserves both input representations and all original diagnostic locations.
Those component observations are identified explicitly; coherent physical-file probes still execute the complete rule.
No expected diagnostic is moved or discarded.

The final cached-policy port remains open.
It requires exact CPython 3.13 AST fingerprints, rather than a source-text or structural-equality hash.
No new Linear ticket or performance claim is added.

### Batch eight: cached policy and broader source ladders

The cached policy now matches its 13 original observations through both native APIs.
Supplemental ladders cover clean memoization, body and signature drift, metadata changes, unsafe cache access, aliases and stale entries.
The tests retain policy configuration as input. They do not supply oracle-computed actual fingerprints to the native detector.

The first expanded ladder exposes three concrete differences.
Policy dictionaries retain the last duplicate entry; duplicate diagnostics still inspect the original list.
A negative Python integer literal has a unary-expression AST, so it fails the verifier's constant-integer requirement.
Stale entries must remain visible when the source trigger disappears. These closed-domain clauses therefore execute on empty syntax input.

The selected generic producer uses one isolated CPython 3.13 process per nonempty demand wave.
It parses each demanded document once within that wave and publishes typed, indexed result columns.
SQL callbacks read completed columns. They do not start Python or parse nodes.
The self-contained AST serializer prototype remains outside Git and is not an optional production backend.
This choice adds a CPython 3.13 runtime dependency, documented in the batch report.

The independent oracle covers 30 source documents and 160 function fingerprints.
Six canonical-AST unit tests pass. A seventh native test checks artifact SHA-256.
The selected engine commit is `41cba695d9388ac8db05dd525c31c09d4fe0751a`.
The addon SHA-256 is `27bd5226769985775484b6b3d739cbd4ed4b2108de254c586d55db0a13e26da2`.
Its allocator remains jemalloc, with no extra build features.

Broader source probes expose the missing eager raw-line scan.
The port now checks all six original patterns, including string contents, with comment and multiline-string exclusions.
Three positive probes also require purity propagation through imported helpers across files.
A separate inherited-method probe returns no original finding through this cached entry point.
The attempted inheritance propagation produced a false positive and was removed. The clean oracle probe remains as a regression.

Invalid-source probes distinguish actual behavior from unused error branches.
The adapter scanner reports `LAO000`; authentication and read-through source scanners return no findings for their selected malformed inputs.
The ports retain those observed contracts.
No Linear ticket was created from these differences.

A fresh upstream run passes all 456 tests with zero pinned-source mismatches.
The broader TypeScript gate passes 70 tests.
The native gate passes 2,522 tests, with 67 existing ignores and the previously documented snapshot-hash test excluded.
The ten edit/removal ladders pass all 50 transitions. Typechecking and SQL-column generator tests pass.

The repository oracle for this batch completes in 174.9999 seconds wall and 168.0570 seconds CPU.
Its plans select 3,186 files and report 34 eager findings plus 235 purity findings.
The native diagnostic run uses the previously verified, fixed 9,021-file input domain.
The oracle plans and expected findings remain unchanged.
The initial native diagnostic overlaps focused tests and includes an external sample. Its timings cannot establish a performance improvement.
Node phase CPU excludes CPython child processes; the driver now states this explicitly.
An external process-family CPU receipt is required for measurements of the new producer.
Repository parity and activation remain separate gates.

### Batch eight: repository gate rejects the focused checkpoint

The first 9,021-file native run fails repository acceptance.
It reports 600 findings against 269 expected findings, with 566 extra findings and 235 missing findings.
There are 46 execution failures. These failures invalidate the focused checkpoint as repository acceptance evidence.
The initial evidence receipt remains a focused-test receipt; its repository status stays open.

The run uses 341.377 seconds cold wall time and 336.105 seconds parent-process cold CPU.
The external receipt reports 343.11 seconds wall, 338.07 seconds total CPU and 2.975 GB maximum RSS.
Compilation takes 84.6 milliseconds wall time.
The run overlaps verification and external sampling, so these numbers are diagnostic and establish no performance improvement.

Adapter ownership contributes 506 extra findings, all outside the original plugin's six production roots.
The candidate now uses those exact roots. Explicit-file tests retain their original scope override.
Eager source regressions expose generator arguments mistaken for direct calls and missing facade/server-ref output exemptions.
Seven complete source files from this failed run are now supplemental oracle regressions.
They are clean in the original analyzer; the native run must also complete without findings or failures.

Repeated declaration-range scans are replaced with indexed parent-chain links.
Eager body selection starts from calls and ownership, then joins the owning function.
Lambda and class regions are prepared once for both body consumers.
These changes keep SQL batching and preserve the original visitor boundaries.

The repository paths also treat `maxSourceBytes` as an aggregate limit, although the model defines it per file.
A reduced public-API test uses three individually admitted files whose total exceeds the cap.
This test must pass snapshot and workspace execution without raising any configured limit.
Canonical byte columns replace cache-policy source-prefix reads, so those queries no longer request a whole source snapshot.
The corrections and repository rerun remain in progress.

### Batch eight: repository recovery, remaining indexed joins

The next three fixed-domain runs also fail acceptance. Expected source findings remain 269.

| Run | Findings | Extra | Missing | Failures | Cold wall | Parent CPU | Maximum RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Per-file source caps | 37 | 3 | 235 | 2 | 468.398 s | 457.177 s | 4.022 GB |
| Instrumented, narrowed string inputs | 34 | 0 | 235 | 4 | 1,042.290 s | 931.098 s | 5.543 GB |
| Scoped node lookups | 34 | 0 | 235 | 16 | 579.057 s | 559.135 s | 6.322 GB |

The external receipts report 470.32, 1,045.46 and 580.97 seconds wall time, respectively.
Their process-family CPU totals are 459.38, 932.91 and 561.15 seconds.
The first two runs include diagnostic overlap. The instrumented run also enables detailed SQL tracing.
These failed runs establish no performance improvement or repository acceptance.

The scoped lookup engine is `fd2100081`. Its addon SHA-256 is `b6ab5e8b89f0049bbf1ab3cb5ea8a36d471f44965c8e4b58f539604c9e0594aa`.
It caches cardinality estimates and applies file and node demands together.
A 128-file regression requires fewer than 1,024 candidates for 128 parent lookups.
Both file and node `IN` constraints retain whole-batch handling.
The broader native gate passes 2,523 tests, with the same inherited exclusion and 67 ignores.

The instrumented trace exposes further query-shape faults.
Lazy call construction consumes 177.94 seconds CPU and 716 million VM steps.
Cached keyword construction reaches about one billion VM steps.
A condition on the output of an `OR` join does not prevent candidate enumeration.
The authored expression, parameter, argument and terminal relations now select their lookup key before the join.
Annotation analysis follows indexed child edges. Lazy body selection follows the same edges with explicit visitor pruning.
The rerun remains open until all expected findings appear without execution failures.

Four new oracle probes cover Unicode names and quoted annotations.
The selected correction uses canonical token facts and canonical annotation formatting.
The canonical name relation has one admitted scope and several consumers. It does not combine unrelated rule scopes.
No new Linear issues or optional backends were introduced.

The chosen-key and child-edge run reduces execution failures from sixteen to two.
Eager resolution matches all 34 expected repository findings without failures.
Lazy architecture reaches `function_events`; purity reaches `global_receivers` before interruption.
The run still omits all 235 expected purity findings and fails acceptance.
Cold wall time is 407.224 seconds, parent CPU is 409.692 seconds, and maximum RSS is 7.837 GB.
The external receipt reports 408.76 seconds wall and 411.79 seconds process-family CPU.
Its final phase overlaps a focused native test build and replay. These timings remain diagnostic.

The event joins require `owned_nodes(__path,id)` and `owned_nodes(__path,__start,__end)`.
The existing index starts with `(__path,fn,id)` and cannot serve either complete lookup key.
Purity also scans each function's node range before selecting global names.
The next query starts with global names and the indexed expression-root name relation.
The findings and source scopes remain unchanged.

### Batch eight: completed queries expose semantic failures

The event-index run completes purity analysis. It reports 275 findings against 269 expected findings.
There are ten extra findings, four missing findings, and one lazy-architecture execution failure.
Its diagnostic cold wall time is 437.063 seconds. Parent CPU is 380.254 seconds and maximum RSS is 8.593 GB.
The external receipt reports 438.65 seconds wall and 382.07 seconds process-family CPU.
The run overlaps builds and tests. It does not establish a performance improvement.

The isolated lazy trace identifies 529,844,318 full-scan steps in `function_events`.
Two correlated subqueries scan the decoded-string input without its required file/start index.
The selected shape copies that input once into an indexed SQL relation used by all its consumers.
Its scope is unchanged. There is no per-subquery materialization or optional backend.
The event stage then completes with 5,845,913 scan steps and 54,403,412 VM steps, without automatic indexes.
Its measured CPU falls from 27.908 seconds before interruption to 1.266 seconds with a complete result.
These instrumented stage figures are diagnostic, not an end-to-end performance gate.

Completion exposes 942 extra lazy findings. Final symbol selection must exclude overload stubs as call targets.
Declaration diagnostics must still inspect each declaration. A new positive oracle case detects the lost overload diagnostics.
Purity also needs final-definition selection for its record map and taint propagation from protected `getattr` assignments.
A managed cache inside parentheses loses its constructor identity in the old SQL relation.
A shared binding-value relation now removes those parentheses for cache classification and context-variable lookup.
Reduced oracle probes retain these contracts, including managed cache reads and writes.

The source-cap compatibility test now uses the documented per-file limit.
Several admitted files can exceed that limit in total. One oversized file still causes an explicit failure.
The broader TypeScript command was stopped after 125 seconds while replaying the older sixth-batch corpus.
It did not pass and is not included in the completed gate counts.

### 2026-11-04: batch-eight semantic recovery reaches the repository gate

The next ten-rule diagnostic run completes every query without engine failures.
It reports 329 findings: 63 extra lazy findings and three missing purity findings.
Cold wall time is 327.665 seconds. Parent CPU is 313.606 seconds and maximum RSS is 9.334 GB.
This remains failed parity evidence. Its final phase overlaps verification, so its timings remain diagnostic.

Five remaining lazy call-graph differences involve visible bindings and visitor boundaries.
The original analyzer ignores local module imports for attribute call targets, skips assignment targets, and resolves module aliases.
The port now preserves those behaviors. It retains every declaration for declaration diagnostics and selects the last definition for call targets.
The service-file predicate also needs parentheses around its SQL `OR` expression.
Without them, unrelated events in an exact service file become callable-payload findings.
Native cursor reads remain eager I/O. They do not count as plan orchestration for `LA-004`.
New probes cover both the clean orchestration case and the positive pure-function case.

The three missing purity findings depend on inherited methods.
The original analyzer selects candidate files before it resolves class inheritance.
The port now keeps that scope, resolves base methods in source order, respects overrides, and prevents traversal cycles.
The earlier unadmitted-parent probe stays clean. Two new admitted-parent probes produce the original positive findings.
All these operations remain authored SQL over typed relations.

The final focused replay matches 522 observations: 329 positive and 193 clean.
It reports no finding differences, engine failures, API differences, or missing native coverage.
The 50 edit/removal transitions also match.
The broader TypeScript gate passes 117 tests and skips two existing opt-in producer-shape tests.
The final native source passes 2,525 tests, with 67 existing ignores and the documented inherited snapshot exclusion.
Typechecking and all three SQL-column generator tests pass.

The two global rules then match the complete 9,021-file domain: 235 expected findings, 235 actual, and no failures.
This diagnostic uses 145.397 seconds cold wall, 151.671 seconds parent CPU, and 9.311 GB maximum RSS.
The external receipt reports 147.06 seconds wall and 153.29 seconds process-family CPU.
It overlaps frontend verification and cannot establish a performance improvement.

The selected engine is `899c3e53d2cb43a509a47d94e0bdc4fd36bb3c21`.
Its normal addon SHA-256 is `4f5a112f088dc59713231e387e1bd62b3ec6192352e36a044688481007d49c3b`.
The final normal addon build takes 36.61 seconds wall. Compilation remains separate from execution.
The engine branch is pushed. No Linear tickets were created.
The cumulative 80-rule run is now in progress without competing builds or tests.
It uses the identical verified 9,021-file domain and 25,262 expected findings from the unchanged 70-rule and batch-eight recordings.
This combined expectation set is not a newly timed Python oracle run.

### 2026-11-04: cumulative 80-rule gate passes

The reference run completes all 80 rules over the fixed 9,021-file, 137,924,324-byte domain.
All 25,262 expected findings match, including all 269 findings from the new ten rules.
There are no extra findings, missing findings or engine failures.
No instrumentation, memory census, competing builds or competing tests run during this measurement.

Cold execution uses 695.641446 seconds parent CPU and 607.831231 seconds wall time.
Compilation uses 1.836054 seconds CPU and 1.874628 seconds wall time, measured separately.
The external process-family receipt reports 701.98 seconds CPU, including Python children, and 613.94 seconds wall time through close.
Maximum RSS is 20,451,016,704 bytes. OS peak memory footprint is a separate metric at 21,238,647,208 bytes.
Workspace close uses 2.676788 seconds wall time and reduces RSS from 13,765,086,616 to 2,603,943,280 bytes.
Memory remains an open optimization problem.

The historical 70-rule result uses 395.519 seconds cold CPU and 15.399 GB maximum RSS.
The engine and rule set differ. These results do not establish a speed comparison.
The earlier roughly 1,400-second measurement is also a different workload and remains historical.

The [measurement receipt](performance-decisions/data-layer-batch-eight-20261104.json) preserves the measured phases and identities.
The [batch evidence](../gate/data-layer/fixtures/host/data-layer-port/batch-08-evidence.json) binds the final 522 observations and 50 transitions to the same addon.
All 80 compiled digests match the repository receipt and current policy files.
The compatibility tool now records both each source-file hash and its authored compiled digest.
Typechecking also passes after that instrumentation change.
The pack has 80 candidates and 62 checks remaining. It remains inactive, with signing/reporting and GPU lowering boundaries documented separately.

## 2026-11-04 — Original test cases, positive coverage and exact diagnostic owners

The user restricted future cold measurements to new kernels.
This audit ran no complete repository cold workload and made no new CPU or RSS claim.
The existing complete repository receipt remains historical.
The integrated addon remains `899c3e53d2cb43a509a47d94e0bdc4fd36bb3c21`, with SHA-256 `4f5a112f088dc59713231e387e1bd62b3ec6192352e36a044688481007d49c3b`.
No native engine code changed.

All eight unchanged upstream test batches ran again at `2b1504adc4291daaf080b2e932962b134da1a69a`.
They collected 4,683 cases: 4,680 passed and three kept their original skips.
Each recorder checked imported source and test files against Git blobs. All source checks passed.
Every upstream command had a 115-second deadline. The slowest batch completed within that deadline.
The inherited `GIT_PAGER` variable was removed for these processes, as required by the upstream scan guard.

| Batch | Passed | Existing skips |
| --- | ---: | ---: |
| 1 | 564 | 3 |
| 2 | 700 | 0 |
| 3 | 568 | 0 |
| 4 | 605 | 0 |
| 5 | 646 | 0 |
| 6 | 933 | 0 |
| 7 | 208 | 0 |
| 8 | 456 | 0 |

The audit found two recorder defects.
Recording started after fixture setup, so it missed scanner calls and AST sources created by fixtures.
The lazy-architecture adapter also used analyzer identities to locate sources from `AnalyzedFile` records.
The recorder now starts before fixture setup and resolves those source domains by file path.
It also records empty and missing-file scanner inputs as empty native snapshots.
This recovers seven lazy-architecture observations, including six positives, and five clean observations.
The original scanner corpus now contains 320 eligible observations: 201 positive and 119 clean.
One invalid-encoding file-reader call remains an explicit Python I/O contract.

An initial source-preservation change produced 41 pairs of line differences in eager AST tests.
Investigation showed that upstream `_scan_source` intentionally calls `ast.unparse(tree)` before its fact engine reparses the input.
The corrected adapter records both the original text and this effective scanner text.
It compares findings in the effective coordinate domain. The test assertions and upstream source remain unchanged.

The old comparator also ignored semantic owners that original tests inspect.
Native terminal projections now expose eager symbols, purity qualified names and plan-helper callsites.
Diagnostic rows carry these values from their existing relations, without additional per-finding source or owner lookups.
Replay compares these fields with the original findings.
A new control proves that the correct eager code at the correct location still fails on the wrong symbol.
Only diagnostic projections changed; the candidate predicates and producer implementations remain unchanged.

Cached discovery now has a direct native contract check.
It replaces only the terminal projection of the production cached-policy program.
The SQL uses the same producer demands, aliases, fingerprints, cache relations and validation errors.
Its JSON object is a test result envelope; it adds no production JSON storage or alternate linter implementation.
All 15 observations match every `DecoratedFunction` field through both native APIs: 14 positive and one clean.
Controls reject a removed declaration and a changed body hash.
The original syntax-exception assertion remains a separate Python API contract.

All 534 batch-eight scanner observations pass both native APIs: 335 positive and 199 clean.
The broader native corpus also passes: 3,961 observations across 80 candidates, including 2,112 positives and 1,849 clean observations.
All 80 candidates have both native positive and clean coverage.
There are zero extra findings, missing findings, engine failures or execution-path differences.
Three host signature observations remain outside native counts.
Batch six completes all 1,259 observations in 24 bounded shards; each observation retains its complete file domain.
No complete-repository benchmark substitutes for these positive test inputs.

The TypeScript runs pass 86 distinct assertions, including ten always-clean controls and the new discovery and owner controls.
The final focused run passes 25 tests. The Python harness passes nine regression tests.
The integrated frontend and compiler accept all 80 current policies.
All 50 batch-eight edit, restoration and removal transitions pass.
The native Rust suite was not rerun because this audit changes no engine code.

The new [case ledger](../gate/data-layer/fixtures/host/data-layer-port/batch-08-case-coverage.json) lists every batch-eight case and its actual evidence.
The audit rejects dropped calls, duplicate calls and finding differences, even when other positive coverage remains.
Its input hashes and retained-call inventory have a regression gate.
The ledger records scanner observations for 247 cases and discovery observations for five cases, with one overlap.
The remaining 205 cases pass their original assertions without a direct native scanner or discovery observation.
Thirty of those cases supply original source inputs for the supplemental corpus.
These source comparisons do not certify the helper assertions that inspect them.
Diagnostic prose remains outside comparison unless it forms the recorded diagnostic identity.
This boundary is explicit; passing Python tests does not imply complete translation of their internal assertions.

The [80-rule receipt](../gate/data-layer/fixtures/host/data-layer-port/testsuite-positive-coverage-20261104.json) records current counts, input hashes and qualifications.
The [batch evidence](../gate/data-layer/fixtures/host/data-layer-port/batch-08-evidence.json) records current policy hashes and the unchanged addon.
The historical complete-repository receipt retains its original policy digests.
It was not relabeled after the three diagnostic projections changed.

Reproduction uses `data_layer_record.py --batch N` for each unchanged upstream batch.
Native scanner replay uses `conformance/data-layer/replay.ts`.
Native discovery replay uses `conformance/data-layer/cached-discovery.ts --recording <json> --output <json>`.
The per-case audit uses `scripts/dsl/data_layer_audit/coverage.py`; its help lists the required evidence inputs.
The regression tests are `test_data_layer_recording.py`, `test_data_layer_case_coverage.py`, `cached-discovery.test.ts` and `diagnostic-metadata.test.ts`.
The Python harness tests use the standard library. The original suites exercise the real pytest lifecycle.

Typechecking, boundary checks, script tests and whitespace checks pass.
The complete `npm run check` still fails the repository structure gate on three existing directory-size findings.
The baseline commit `408f91f1a` already has 26 files in `native/src/code_rule_dsl/`, 31 in `scripts/dsl/`, and 26 in `test/engine/native/`.
New Python audit tools and tests use subdirectories. They add no directory-size or CI-registration finding.


## 2026-11-04 — Finish batch-eight native assertion translation

The previous audit did not translate semantic helper assertions. Its Python pass count overstated the native evidence.
This delivery replaces those calls and adds a guard against original semantic execution.
The [native assertion receipt](../gate/data-layer/fixtures/host/data-layer-port/batch-08-native-assertions.json) binds the tested policies, adapters and addon by hash.

The first direct run exposed eight purity failures and eight callback failures.
A subsequent lazy-architecture run exposed missing diagnostic symbols, messages and stable identities.
Shared SQL now carries those payloads. The engine requires nonempty message-column names and text results.
Canonical wrapper signatures, eager exemptions, signature validation, cache discovery and runtime hashes now execute natively.
Native fact exports retain class aliases, execution context flags, tombstones, side-queue declarations and deferred call targets.
Plugin tests batch native fact construction instead of invoking the original Python collector.

Both execution APIs pass all 456 original cases, without skips or semantic fallbacks.
Each run records 480 native calls across 336 cases: 283 diagnostic calls and 197 helper calls.
Of the diagnostic calls, 199 contain findings.
The other 120 cases exercise host constants, DTOs, file handling, signing or CLI orchestration.
Three instrumentation functions use native counters in four cases; their behavior assertions are unchanged.

The instrumentation ladder exposed two real defects.
Relational work counters were lost at snapshot and workspace reporting boundaries.
Canonical byte-column requests also parsed and walked the module unnecessarily.
The counters now reach both reports, and text-only requests perform no parse or module walk.
A 108-kilobyte unused literal stress case exposed unnecessary immutable-container classification.
That preparation now follows actual global-reference demand. The original stress observation passes without increasing its budget.

A new exact-message regression exposed alphabetical target selection beneath one shared graph hop.
The port now preserves source call order and stops at the first matching sink.
Ordered traversal serves actual diagnostic demand and guards cycles.
The new corpus also checks nested owners, compatibility imports, deferred delegates and first matching connection-owner calls.

All 552 recorded observations pass both native APIs: 347 positive and 205 clean.
This includes the earlier 534 observations and 18 new exact-message observations.
All ten native always-clean controls reject their selected positives.
Disabling the nine detectors with original pytest modules produces 173 failed cases and ten positive-fixture setup failures.
Those failures contain no engine errors or original-detector fallback.

The final focused TypeScript gate passes 35 tests. Canonical producer tests pass nine; relational tests pass 94.
The integrated compiler accepts all 80 candidates.
Typechecking, boundary checks and Windows path checks pass.
The structure gate still reports the three directory-size failures already present at the recorded baseline.
The test-binary build took 187 seconds; its nine canonical tests took 0.08 seconds.
The selected addon build took 59.91 seconds. Build times are separate from execution evidence.
No complete-repository cold benchmark ran. Historical CPU and RSS measurements remain historical.

Root TDD commits `aac633d9f` and `e364ead20` capture the assertion and ordered-target regressions.
Companion engine commit `ebcfefa0a` supplies diagnostic payloads and native work counters.


## 2026-11-04 — Correct the incomplete batch-eight translation

The preceding completion claim was premature. Original partial assertions missed complete field and ordering differences.
Some adapters rescanned source instead of consuming supplied facts. Test-only configuration also changed rule behavior.

The new comparison records complete upstream API results before installing native adapters.
It checks every returned field, list position, count and syntax-error attribute.
Complete analyzer comparison initially found differences in 25 of 85 inputs.
The fixes preserve declaration order, CPython visitation order, future imports and private-runtime events.
The shared predicates now consume supplied function and policy facts without source files.
All ten rules have no `oracleAnalyzer` option.

Actual porting exposed another engine requirement: ordered first-match traversal with memo reuse across roots.
Independent reachability chose different call sites and returned extra findings through cycles.
The generic native producer now reads whole typed SQL batches of roots, edges and sinks.
It preserves negative memo entries and keeps each file's callable map separate.
Shared sinks use one relation, without a file-by-sink cross product.
The producer returns indexed columns and contains no linter symbols.
The companion also exposes canonical syntax-error facts through the existing isolated CPython worker.

Additional fixes retain separate raw-text and AST coordinates, canonical eager expression text and exact policy comparison order.
The cached verifier checks nested immutable globals and keeps duplicate-declaration errors separate.
Policy annotation formatting now preserves Python quoting.
The final corpus caught a missing sync/async grammar lookup; the original expected finding remains in the gate.
Nine unused comparator definitions were removed. Regeneration produced identical rule and fact-policy bytes.

Final correctness evidence, through both snapshot and workspace APIs:

| Gate | Result |
| --- | --- |
| Original upstream suite | 456 passed, zero skipped, zero semantic fallbacks |
| Complete API results | 305 observations, zero differences |
| Complete analyzer facts | 85 inputs, zero differences |
| Recorded scanner corpus | 558 observations: 351 positive, 207 clean; zero differences |
| Supplied-fact batches without source | 16 inputs through each API |
| Native calls in the original suite | 523 across 336 cases: 326 diagnostic calls and 197 helper calls |

Of the 326 diagnostic calls, 201 contain findings.
The other 120 original cases test host behavior; they are not native detector coverage.
Always-clean mutations disable both source and supplied-fact diagnostic consumers.
The original positive tests reject those mutations across all nine suites that have pytest modules.
The control run has 173 assertion failures and ten positive-fixture setup errors, without semantic fallback.
The SQL-surface rule has separate positive and clean oracle probes and a native mutation control.

The selected engine commit is `29cbf425b`, on `jeremy/data-layer-integrated-compat-20261103`.
Its addon SHA-256 is `34b553e63d5b8277d488fba591816bfbb6c7a7554509c3ea63a1566dbdc6871b`.
The final addon build took 50.70 seconds. The test-binary build took 218 seconds.
Those build times are separate from test execution.
The canonical producer tests pass ten cases. The relational suite passes 38 cases, including the ordered-graph tests.
The final TypeScript gate passes 70 tests. The integrated compiler accepts all 80 candidate rules.
Typechecking, boundary checks and Windows path checks pass.
The structure check retains its three existing directory-size findings.

The native assertion receipt binds the engine, rule programs, fact programs, adapters and complete-result oracles by hash.
Root red commits `5cb371089`, `0a5f86198` and `992350ca5` capture the stronger comparisons and positive regressions.
Companion red commit `bc138befc` captures the syntax-error contract.
No full-repository cold benchmark ran. This correctness work makes no new CPU or RSS claim.


## 2026-11-04 — Audit remaining batch-eight gaps

The completion statement was broader than the evidence. Two fresh comparisons demonstrate remaining defects.
The LA-011 hydration consumer uses minimum location instead of supplied call order.
For matching calls ordered at lines 3 and 2, upstream returns line 3; Rerail returns line 2.
This reproduction uses the supplied-fact API, not a claim about ordinary source discovery.
The standalone transitive helper also ignores an existing memo entry.
For a cached root line of 99 and a target edge at line 10, upstream returns 99; Rerail returns 10.
Both native reproductions report zero semantic fallbacks.
The passing 456-case suite and recorded corpus did not cover these inputs.

The strict API and complete-fact comparisons remain manual commands outside CI.
The companion engine features are not integrated into this worktree's default native build.
Default production scopes and complete Rerail reporter/signing/ratchet workflows still need integration coverage.
The pack remains inactive. GPU lowering remains separate.
The revised batch-eight kernels have no new CPU/RSS profiling receipt.
No implementation changed during this audit.

## 2026-11-04 — Complete batch eight on the default engine

The two audit defects are fixed. Red commit `b4cec00a0` adds the hydration-order case and twelve independent graph-state probes.
LA-011 now uses supplied visitation order, not minimum location.
The transitive helper consumes initial memo and visiting state through the production ordered-graph producer.
It preserves negative entries, target precedence, memo insertion order and complete updates.

Commit `edffa2547` integrates the required capabilities into this worktree's default engine.
It preserves SQL fusion, shared binding ownership and typed producer contracts.
The native graph producer contains no data-layer symbols or per-node SQL submissions.
Canonical Python requests remain whole demand batches, with one isolated CPython process per nonempty wave.
SQL diagnostic messages and native output schemas now survive compilation and inference.
Source-byte admission is consistently per file.

A real enforcer test found another integration defect: its result parser rejected the SQL `row` object.
The enforcer and API now accept the existing recursive JSON value type.
Production tests check authentication and purity positives with shipped defaults and scopes, exact messages and source lines, delivery, retraction and exclusions.
These tests execute the real enforcer rather than the replay adapter.

The upstream cache-discovery case reads 3,095 files and 62,921,799 source bytes.
External sampling found SQL fact publication and index retirement, including WAL work, for files already rejected by their selectors.
Preparation now skips publication when there is no admitted anchor demand, in both execution paths.
Red commit `3f53dd32c` records the unused-publication regression.
Complex Unicode trigger expressions also lost admission when literal extraction exceeded its alternative limit.
The generic text requirement now retains a conservative regex in that case.
It removes zero-width assertions before searching the containing source.
Tests prove that node anchors and word boundaries cannot become file boundaries.

One Rust allowance test relied on work that the new admission could eliminate.
Its input now gives every integer a real matching preceding sibling.
The test still requires clause-local failure under the same fixed allowance; it cannot pass through an empty candidate set.
The broader Rust DSL run then passed 1,552 tests, with 22 existing ignores.
It took 50.34 seconds. Test compilation was checked again after fixture relocation.
The final addon build took about 103 seconds; builds are separate from workload timings.

Commit `5d218b3d2` gives each rule one canonical document.
The ten batch-eight documents live in `packs/data-layer/rules/`; the earlier seventy remain candidates.
The harnesses resolve both locations and retain explicit mutation-directory isolation.
The frozen profiling workload includes both locations and rejects duplicate filenames.
The private pack loads through the public catalog; core does not enable it automatically.

Final addon SHA-256: `75468a30ae1520582ed11b4a4bd12680aded5ead1de26a7bd755ca6bcb96b0db`.
It was rebuilt from the committed source after the fixture relocation.
All final native receipts use this addon, with `RERAIL_ENGINE_PATH` unset.

| Correctness gate | Final result |
| --- | --- |
| Unchanged upstream source; original behavior assertions | 456 cases passed in each API mode; no skips or semantic fallback |
| Complete API comparison | 305 results per mode; zero differences |
| Complete analyzer comparison | 85 inputs per mode; zero field differences |
| Graph-helper state | 12 inputs per mode; exact results, memo order and visiting state |
| Supplied facts without source | 17 batches through both APIs |
| Recorded scanner corpus | 558 observations: 351 positive and 207 clean; no differences or engine failures |
| Disabled-detector controls | 173 assertion failures and ten positive-fixture errors across nine original pytest suites |
| Focused TypeScript suite | 74 passed |
| Default-engine compilation | All 80 policies accepted |
| Typechecking, boundaries and Windows paths | Passed |
| Structure | Only the three existing directory-size findings |

Of the 456 original cases, 336 invoke native execution: 523 calls, including 201 positive diagnostic calls.
The other 120 cases test host constants, DTOs, discovery, signing or CLI orchestration.
Four instrumentation cases translate Python hooks and count assertions to actual native counters; their behavior assertions remain intact.
The original Python detector and analyzer implementations are forbidden by the harness.
The SQL-surface rule has no upstream pytest module at this pin; independent positive and clean probes and a disabled-detector control cover it.

CI now builds this engine and runs both upstream API modes against the pinned private checkout.
It also checks complete results, complete facts, graph state and disabled-detector rejection.
Each subprocess is bounded at 115 seconds.
The existing CI app supplies read-only upstream access on trusted jobs.
Remote CI execution and app installation access have not been verified locally.

### Focused CPU and RSS measurements

The [measurement receipt](performance-decisions/data-layer-batch-eight-default-kernels-20261104.json) contains three serial cold processes per kernel and a separate Instruments capture.
The benchmark loads only the new kernels, not the rule batch or the repository.
Compile and evaluation CPU remain separate.
External BSD `time` records process-family CPU, including waited Python children and process startup.
RSS is the maximum per-process RSS, not simultaneous family memory.
The companion and default profiles both use optimization level 3 but differ in inheritance and codegen-unit count.
These are compatibility measurements, not a controlled claim about an engine-wide speedup.

Medians from three cold processes:

| Build and kernel | Compile CPU | Evaluation CPU, Node | Evaluation wall | Process-family CPU including startup | Maximum RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Companion: 1,000-root graph | 21.831 ms | 14.002 ms | 14.005 ms | 0.85 s | 242.6 MiB |
| Default: same graph | 24.566 ms | 14.076 ms | 14.100 ms | 0.83 s | 226.5 MiB |
| Default: graph with memo and visiting state | 23.084 ms | 8.654 ms | 8.679 ms | 0.78 s | 225.1 MiB |
| Companion: 20 canonical Python documents | 0.102 ms | 1.037 ms | 41.468 ms | 0.85 s | 258.9 MiB |
| Default: same Python documents | 0.101 ms | 0.777 ms | 40.389 ms | 0.78 s | 242.3 MiB |

The Python evaluation CPU column excludes its child process; it is not total kernel CPU.
Use the external process-family measurement for that boundary.
The graph-state case emits 599 findings, while the plain case emits 1,000, so those rows do not establish an algorithm speedup.
The plain graph's evaluation CPU is effectively unchanged between builds.

Instruments Time Profiler captured 5,681 running-thread samples from the final addon.
Bottom-up self time was led by memory comparison, JSON value indexing, memory copying, SHA-256 compression and allocation.
Inclusive CPU shares were 48.86% in JSON work, 15.16% in finding-row construction, 14.43% in SQL and 3.91% in finding identity.
Graph preparation accounted for 4.03%; traversal itself accounted for 0.56%.
These inclusive categories overlap.
This positive-output kernel points toward result construction and serialization before further graph-algorithm optimization.
It does not rank bottlenecks in a mostly-clean repository run.
The initial `sample` capture was too sparse for percentages; the Instruments capture supplies the quantitative evidence.

Reproduce a bounded kernel with `node --import tsx conformance/data-layer/batch-eight-kernels.ts --kernel graph-state`.
Use `--seconds 10` for an external profiler attachment; the tool rejects durations above ten seconds.
The other kernels are `graph` and `python`.
`xcrun xctrace record --template 'Time Profiler' --attach <pid> --time-limit 5s --output <trace>` records the native stacks.
The tool prints its PID before evaluation.

No full-repository cold run was performed.
The initial interrupted discovery runs are timeout observations, not before/after benchmark measurements.
This work does not claim to resolve the historical repository CPU or 20.451 GB peak-memory problem.
There are 62 rules left to port. No Linear issues were created.

## 2026-11-04 — Batch nine: canonical columns and exact API parity

The port inventory advances from 80 to 90 of 142 checks.
The private pack now ships twenty documents; seventy earlier ports remain candidates.
The [batch report](data-layer-batch-nine.md) records the source pin, contracts and reproduction commands.

The original oracle passes 201 cases and records 245 complete API results.
Those results include 142 positive observations.
The corpus combines 133 upstream linter/framework cases, 13 adjacent codemod cases and 55 additional controls.
One native instrumentation translation measures actual parse and module-walk counters.
The other 244 API results require complete-value parity.
Detector-disabled controls must fail the original positive assertions for all ten checks.
The final [assertion receipt](../gate/data-layer/fixtures/host/data-layer-port/batch-09-native-assertions.json) records each native execution path.

The new service-rule repository test exposed concrete preparation defects:

1. Complete AST row dictionaries exceeded the 64 MiB transport limit on its 100-file input.
   The worker now sends demanded column arrays. The native adapter retains column vectors.
2. Unused annotation rendering and breadth-first ordering still ran for every document.
   Canonical demand now controls both operations. SQLite column analysis narrows generated producer fields.
3. Rejoining every canonical node to the CST repeated unnecessary work.
   Validated canonical source spans remove that join and preserve variadic-parameter locations.
4. Delegate body lookups lacked an index on their actual join key.
   The body relation now indexes path and node identity. An ownership anti-join avoids repeated delegate scans.
5. An inner alias shadowed its method owner in a descendant query.
   The resulting uncorrelated predicate falsely flagged 351 clean methods. Explicit aliases restore correlation.
6. Cached SQL results could outlive their canonical anchor proofs in a fresh workspace document.
   The SQL owner now retains source-versioned proofs under its existing binding-memory limit.

Additional controls found lost diagnostic multiplicity for repeated delete targets and keyword arguments.
The corrected joins preserve each occurrence.
Legacy structural helpers also preserve first-definition behavior and their explicit method order.
No capacity or statement limit increased.

The new canonical-column kernel uses twenty files, 44,980 source bytes and 400 findings.
Both retained addons execute the same kernel. Each row reports the median of three fresh processes.
Compilation is separate from evaluation. The command also performs one repeated evaluation.
The [kernel receipt](../gate/data-layer/fixtures/host/data-layer-port/batch-09-kernels.json) retains all six runs and addon hashes.

| Retained implementation | Compile CPU, Node | Cold evaluation CPU, Node | Cold evaluation wall | Whole-command family CPU | Maximum RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Complete row dictionaries | 1.797 ms | 108.868 ms | 414.796 ms | 1.32 s | 216.6 MiB |
| Demanded column arrays | 1.702 ms | 73.656 ms | 223.495 ms | 1.20 s | 212.2 MiB |

Node evaluation CPU excludes the CPython child. It is not total kernel CPU.
External BSD `time` includes process startup, compilation, both evaluations and waited child processes.
Its family CPU improvement is about nine percent for this small command.
Startup therefore limits this comparison. The wall-time reduction is not a CPU reduction of the same size.
RSS reports the maximum per-process resident size, not simultaneous family memory.
These retained builds identify a local kernel improvement; they do not establish an engine-wide speedup.

Reproduce the kernel with `node --import tsx conformance/data-layer/batch-nine-kernels.ts`.
All ordinary test commands remain bounded at 115 seconds.
An early combined retained-test command and a two-shard workspace attempt reached that bound under concurrent load.
The acceptance runs use smaller independent assertion shards. Each repository-wide test retains its complete file domain.
Builds used the existing authorization to finish.

No cumulative cold repository benchmark ran.
The historical full-repository CPU and 20.451 GB memory problems remain open.
No Linear issues were created.

Final validation passes 201 cases in each native API mode, with zero differences across 244 complete results per mode.
Disabled detectors produce 147 expected failed cases and 54 passing host or clean cases.
All ten checks reject their positive controls.
The native DSL suite passes 1,576 tests, with 23 existing ignores.
The TypeScript replay and receipt tests pass all 26 cases.
Retained checks pass 34 supplied-fact API cases, thirteen capability cases and four prior-receipt cases.
TypeScript typecheck passes. The ten documents regenerate byte for byte.
The structure check retains three pre-existing directory-size findings; all other metrics pass.

## 2026-11-05 — Batch-nine completion claim withdrawn

A new audit reproduces three semantic regressions: two false SBC002 findings and one missing BEI024 finding.
The original implementation passes all three reduced cases. Both native API modes fail all three.
The cause is lambda argument leakage into enclosing function signatures.

Direct shipped-policy evaluation also drops SAP000 and emits NATIVE-SYNTAX instead of BCH000.
The replay adapter supplies the BCH000 conversion, so its passing assertions concealed the production mismatch.
The plan-composition producer is only called by the test harness; the default rule requires manually supplied observations.
Four upstream MCP composition cases were omitted. They pass when explicitly selected through the adapted harness.
Neither lazy-SQL entry point has a dedicated upstream pytest module; the earlier recovered-tests statement was incorrect.

The [audit](data-layer-batch-nine-audit.md) and [evidence](../gate/data-layer/fixtures/host/data-layer-port/batch-09-audit-20261105.json) retain the reproductions.
The 201-case historical receipt remains valid evidence of those runs, rather than complete batch-nine acceptance.
The completion claim is withdrawn. No Linear issues were created.

## 2026-11-05 — Batch-nine TDD repair and production acceptance

The completion audit exposed real defects. This delivery repairs them before any further ports.
The source pin remains `2b1504adc4291daaf080b2e932962b134da1a69a`.
The historical audit evidence remains unchanged.
Red-test commits are `a07e11e45`, `e179ec9dd`, `1c695547e` and `076bf932a`.

| TDD step | Observed red result | Green implementation |
| --- | --- | --- |
| Canonical argument ownership | Function and lambda ownership facts fail the new primitive regression | Publish `arguments` containers; use function-owned parameters in signature relations |
| Batched parser errors | Invalid-source parser stage is unsupported; leading blank lines later expose an invalid anchor | Generic source demand returns bounded `syntax_error` facts; admitted files permit a file-start diagnostic |
| Shipped rule execution | Lambda signatures, SAP000 and BCH000 fail raw-policy comparisons | Correct signature joins and emit reporter diagnostics in SQL; remove BCH000 adapter fabrication |
| Production composition | Public runtime integration is absent; pre-normalized options prevent binding | Add explicit CLI and host API; bind fresh observations before schema normalization |
| Freshness | A new process reads timestamp-valid stale bytecode | Give each probe process an empty bytecode-cache path |
| Revision zero | Both public-runner tests reject the initial snapshot | Advance the fresh workspace to revision one for initial revision-zero input |
| Complete-value verifier | A changed diagnostic does not fail the aggregate verifier | Compare complete API records, input domains and call multiplicity |

The corpus now includes 137 upstream linter/framework cases, 13 adjacent host codemod cases and 62 additional cases.
The additional cases include two harness checks. All four omitted MCP cases now run.
Two independent pytest collections retain the upstream contract markers without conflicting owners.
Each repository test retains its complete file domain, including the mandatory 100-file service test.

| Acceptance gate | Result |
| --- | --- |
| Original oracle | 212 cases pass; 254 complete API records |
| Native snapshot assertions | 212 cases pass; 253 complete results match; 147 positive results |
| Native workspace assertions | 212 cases pass; 253 complete results match; 147 positive results |
| Instrumentation translation | One original walk record maps to actual native parse and module-walk counters |
| Disabled detectors | 152 expected failed cases and 60 passing cases; all ten detectors rejected |
| Native DSL suite | 1,576 pass; 23 existing ignores; 114 non-DSL tests outside this command |
| Focused TypeScript acceptance | 55 pass, including 22 shipped-policy and two production-runner regressions |
| Retained API and capability checks | 44 pass |
| Selected earlier receipt checks | Four pass; 21 unrelated cases outside this command |
| TypeScript typecheck and boundaries | Pass |
| Generated documents | All ten regenerate byte for byte |
| Structure | Three pre-existing directory-size findings; no new findings |

The public producer runs the actual application's builder and returns observations only.
SQL retains the detector predicates. Probe failures fail closed.
The runner rejects stale snapshot source. Fresh bytecode paths also cover same-size edits with unchanged modification times.
Low-level compiled composition rules require explicit observations; the supported CLI and host API supply them automatically.

The [batch report](data-layer-batch-nine.md) and [assertion receipt](../gate/data-layer/fixtures/host/data-layer-port/batch-09-native-assertions.json) record reproduction and identities.
The previous kernel receipt remains historical. It does not measure this repaired build.
No cumulative cold repository run or new performance claim forms part of this repair.
No capacity or statement limit increased. No Linear issues were created.

## 2026-11-05 — Second audit: public contracts concealed by replay adapters

The previous repair claim at `8e4cede5f` still overstated completion.
A passing DTO comparison did not establish correct public diagnostics or configured-plugin behavior.
The [second audit evidence](../gate/data-layer/fixtures/host/data-layer-port/batch-09-public-audit-20261105.json) preserves the actual failing output.

BEI035 reported line three in a SQL field, but its public span pointed to line one.
Both adapters read the SQL field, so both passed the same incorrect production result.
Leading blank lines also changed the SQL field because the captured module text omitted them.
The production tests selected one positive per rule and missed these raw-source locations.

The same audit found incorrect cache-health scope, ignored-directory differences and blocking enforcement for both report-only lazy-SQL checks.
It also found incorrect configured parser behavior and partial results after a failed bootstrap project derivation.
BCH000 belongs to the cache source API. The configured plugin does not emit it for the same malformed source.
The earlier parser repair copied the source API contract into the plugin default.

| TDD commit | Observed red result | Repair |
| --- | --- | --- |
| `8596c1b29` | Ten public tests fail; the bounded line primitive is unsupported | Add a validated line anchor, preserve complete source, widen helper-file scope and restore report-only behavior |
| `ca77a1a89` | Both scope comparisons fail original `FactEngine.plan()` output | Preserve actual ignored directories for both configured plugins |
| `92ecf48f9` | Eight configured parser comparisons fail | Separate source entry points from configured plugin defaults |
| `261fea8eb` | Both native modes retain findings after the original project extractor discards its result | Suppress the derived result for malformed coverage dependencies |

The line anchor narrows a proven witness to a selected line.
It rejects noninteger, nonpositive or out-of-witness locations. It cannot create arbitrary source ranges.
Both replay paths now compare public diagnostic fields before constructing upstream DTOs.
The original source and configured-plugin paths retain distinct acceptance cases.

The configured observation recorder executes actual upstream plugins and verifies 121 loaded source files against the pin.
Its two scope inventories and nine parser/dependency scenarios reproduce byte for byte.
The production composition CLI also matches the original clean result against the pinned application's real builder.
Positive composition controls still require the SQL detector to reject defective builder observations.

No cumulative cold repository benchmark ran. The previous performance limitations remain open.
No capacity or statement limit increased. No Linear issues were created.

The refreshed oracle passes 216 cases and records 258 API results.
Both native modes pass 216 cases and match 257 complete results, including 151 positive results.
The remaining original result maps to actual native parse and walk counters.
Disabled controls produce 156 expected failed cases and 60 passing cases. All ten detectors are rejected.
The native DSL suite passes 1,576 tests, with 23 existing ignores and 114 non-DSL tests outside the command.
Retained checks pass 44 API/capability cases and four selected earlier receipt cases.
TypeScript typecheck and boundary checks pass. All ten documents regenerate byte for byte.
The structure check retains three pre-existing directory-size findings.

These results cover recorded inputs and explicit public contracts. They do not certify untested production behavior.
The remaining 52 rules are unauthored. Repository activation remains separate from authored-rule inventory.

The final combined TypeScript acceptance run passes 86 tests, including 30 public-contract regressions.

## 2026-11-05 — Third audit: equality and presence boundaries

The audit of `e5d9cc2a7` finds two more SQL translation defects.
The [audit evidence](../gate/data-layer/fixtures/host/data-layer-port/batch-09-third-audit-20261105.json) retains six failing API evaluations across both native modes.

The BEI007 predicate casts canonical numeric text to a SQLite real value.
Overflowing Python literals produce infinity. SQLite casts their text to zero, so the port reports a false positive.
The repaired predicate recognizes canonical zero representations directly and preserves `False == 0`.

The structural query uses valid-file evidence to choose its lifecycle owner.
An existing malformed lifecycle file therefore triggers an incorrect fallback to `connection.py`.
Explicit invalid-file evidence now preserves the original existence-based choice.
Absent, empty and malformed files have separate regressions.

Red-test commit `4b349d3df` precedes both repairs.
The original reduced suite passes 23 cases. Both native modes initially fail three cases and pass twenty.
The new TypeScript shape gate initially fails ten tests, including shipped-default comparisons.
Both repairs pass the reduced suites and the expanded shape gate.

A reproducible shape recorder now observes 88 cases across all ten original check implementations.
The first 52 cases matched, before the numeric and lifecycle probes exposed these defects.
The retained matrix includes positive and clean variants and verifies 124 loaded upstream source files.
CI requires exact reproduction. The receipt binds the nested recorder modules and corpus.
These are added audit observations, not additional upstream pytest cases.

The complete oracle passes 228 cases and records 270 API results, including 156 positive results.
The twelve new pytest cases cover nine numeric variants and three lifecycle states.
No native code changed, so the prior 1,576-test native result still applies to the unchanged addon.
No cumulative cold repository run or new performance claim forms part of this repair.
No capacity or statement limit increased. No Linear issues were created.

Both native modes pass all 228 cases and match 269 complete results, including 156 positive results.
One original instrumentation record still maps to actual native parse and walk counters.
Disabled controls fail 161 cases and pass 67; all ten detectors reject their positive controls.
The separate 88-observation shape gate passes 44 TypeScript tests through both native modes and selected shipped defaults.
TypeScript typecheck and boundary checks pass. All ten documents regenerate byte for byte.
Retained API and capability checks pass 44 tests.
The structure check reports only its three pre-existing directory-size findings.

The final combined TypeScript acceptance run passes 130 tests across seven files in 35.53 seconds.

## 2026-11-05: Batch ten ports and complete-result validation

The inventory advances to 100 authored checks out of 142.
The private pack ships 30 documents. Seventy earlier ports remain candidates.
The [batch report](data-layer-batch-ten.md) lists all ten rules and reproduction commands.

The original suite contributes 265 cases. Another 61 cases run first against the original linters.
All 326 cases pass through snapshot execution and resident workspace execution.
The corpus preserves 203 complete detector results, including 169 positives.
These calls occur in 202 tests. The remaining 124 tests retain upstream utility, DTO, discovery, and CLI assertions.
Original detector functions and collector methods are forbidden during native detector replay.

The optimizer has no dedicated upstream suite. Its 32 added cases exercise every diagnostic family and include clean controls.
The other 29 added cases test aliases, annotation forms, parser errors, source readers, and forbidden artifact presence.
Disabled detectors cause 145 original assertions to fail. Every one of the ten detectors is rejected by positive controls.
Both APIs also match 61 configured-default observations from actual upstream reporters and file discovery.

The [assertion receipt](../gate/data-layer/fixtures/host/data-layer-port/batch-10-native-assertions.json) binds the complete runs and controls to their engine and implementation files.
The unchanged pytest runs each finish in about 70 seconds under concurrent verification load.
These times describe assertion execution. They are not cold performance baselines.

The porting process exposed and repaired these defects:

| Defect | Resolution |
| --- | --- |
| SQL cannot read complete documentation or distinguish empty files from absent files | Add the bounded `source_text(path)` overload |
| Relational evidence only accepts AST queries | Add typed `file.content` regex evidence |
| Text-only diagnostics cannot narrow source lines | Narrow proven text spans; retain declared missing-file anchors at line one |
| Workspace regex evidence loses projected columns | Publish the same typed projections as snapshot execution |
| An unrelated AST error makes workspace text evidence incomplete | Keep regex completeness independent from AST syntax status |
| Matching repeats canonical preparation across detector clauses | Share one canonical producer per candidate file |
| Matching static strings repeatedly scan a recursive CTE | Store indexed expression parts and aggregate them once |
| Direct scanner triggers differ from configured plugin triggers | Keep their observed entry contracts separate |
| Bare authority types and generic annotation bases are missed | Preserve the original annotation predicates and recursive alias rules |
| Optimizer documentation checks run without verifier facts | Apply the configured reporter's actual activation condition |

The bounded-path rule keeps the original text semantics across Python, RST, and Markdown.
It preserves duplicate missing-file findings and required-file checks.
The conditional rule reuses the existing shared producer rather than creating another inference implementation.
The matching repository assertion retains all 140 files selected from its complete upstream repository snapshot.

The native DSL suite passes 1,576 tests, with 23 existing ignored tests and 114 tests outside its selected namespace.
Typechecking, boundary checks, DSL reference validation, and byte-for-byte regeneration of all ten documents pass.
The adjacent catalog, pack, and new native regression tests pass 403 assertions.
Batch nine also passes its 228 unchanged cases through both APIs after the engine changes.
Its disabled-detector controls remain effective, and its receipt is refreshed from those runs.

The structure checker still reports three existing flat-directory limits: 26, 31, and 26 files where the limit is 25.
Those counts predate this batch. New port code and tests use subdirectories.
An initial adjacent-test command also selected an archived checkout under `.rerail`.
Restricting discovery to `--dir test` gives the 403 passing current-checkout assertions reported above.

No cumulative cold benchmark was run. The previous complete repository measurement still covers 80 rules.
No Linear issues were created.

## 2026-11-05 — Batch-ten completion repair

The earlier batch-ten completion claim was disproved. Treat its 326-case receipt as superseded.
Nine audit inputs exposed four ports with incorrect results through both native APIs.
The prior harness also omitted 14 shared conditional tests and filtered matching inputs with the original Python selector.
Its earlier statement about retaining the complete matching domain was therefore misleading.

The repair proceeded through failing-test commits before implementation:

- `58ffdd3a6` records the nine audit regressions through both APIs.
- `ecd327ef8` requires structural relative-import levels.
- `7711d7a9c` requires structural parents without source locations.
- `27a303d66` exposes method-event ownership and same-line control deduplication defects in the replacement analyzer.

The implementation fixes authority tokenization, application-path ownership, repeated counter targets, and relative imports.
It also fixes parenthesized rejection and the matching default source predicate.
A canonical SQL conditional analyzer replaces the earlier CST approximation.
The new analyzer preserves the original traversal's alias timing, overwritten descriptors, control-flow rules, and class-level events.
A source-inventory collector supplies files to shared tests without running the original semantic collector.

The final original run passes 744 cases at source pin `2b1504adc4291daaf080b2e932962b134da1a69a`.
These comprise 279 upstream cases and 465 supplementary cases evaluated by the original linters.
The supplements include 354 transformations across all ten rules and 50 repair, control-flow, ownership, and selection cases.
Each native API matches 621 complete detector results, including 551 positives, across 620 detector tests.
The other 124 cases retain their original utility or CLI assertions.
They do not count as native detector observations.
Disabled-rule controls produce 163 failing assertions and reject all ten disabled detectors.

The matching assertion now sends all 2,871 production files and 57,328,575 source bytes to native execution.
No native assertion report records a call to Python `_source_requires_matching_scan`.
Snapshot and workspace executions pass the original repository assertion separately.
Their pytest durations are 97.34 and 98.06 seconds, respectively.
These are assertion durations, not cold CPU benchmarks.
The other 743 cases pass in 74.56 seconds for snapshot and 76.53 seconds for workspace under concurrent validation load.
A combined initial attempt reached the 115-second command limit; separate commands preserve the repository-wide assertion unchanged.
CI uses the same separation.

An initial full run also found a harness normalization defect for tuple-valued file arguments.
It retained changing pytest directory prefixes in 13 HNSW observations.
The corrected recorder recursively identifies temporary file arguments, and both full native runs pass those observations.
Compact TypeScript replay separately exposed embedded temporary roots in generated-index string results.
Replay removes only that changing prefix, as the Python parity comparator already did.

The configured corpus has 62 original reporter observations.
Both APIs check shipped defaults, diagnostic fields, scopes, missing artifacts, and matching's selected file set.
The configured-default, repair, and new native-contract gate passes 25 tests.
All ten documents regenerate byte for byte. Typechecking, boundary checks, and DSL reference validation pass.
The rebuilt native DSL suite passes 1,576 tests, with 23 existing ignored tests and 114 tests outside its selected namespace.
The three existing flat-directory limit failures remain: native root 26, DSL scripts root 31, and native tests root 26; each limit is 25.

The complete corpus is now compressed to preserve the full source inventory in Git.
The refreshed receipt binds complete native results and controls to the current implementation.
See [the batch report](data-layer-batch-ten.md) for commands and the per-rule result counts.
These finite gates do not establish equivalence for every possible Python program.
The cumulative full-repository result still covers 80 rules. No cumulative cold benchmark or new Linear issue was added.

Final compact batch-ten replay passes all 23 tests after the temporary-root correction.
Batch nine's 228 assertion cases also pass again through both APIs on the rebuilt engine.
Its disabled-detector controls still reject all ten ports.
Its receipt is refreshed from those executions because the shared AST metadata and CI file changed.
The prior batch-nine kernel measurements remain historical; no old kernel received a new cold benchmark.

## 2026-11-05 — Batch-ten completion audit and contract repairs

The previous 744-case completion report was insufficient. This entry supersedes its coverage claims.
The audit found raw newline, malformed-file, ordering, source-selection, and Unicode regex defects.
The file recorder also normalized CRLF and CR, hiding differences before native execution.
A clean repository result could not expose these defects.

Failing tests preceded the fixes:

- `15eb62928`: raw newlines, malformed-file isolation, and textual line ordering.
- `b36301c05`: optimizer file/source line-limit differences.
- `6973b9c34`: reject original runs presented as native evidence and missing native call traces.
- `37e21ee64`: matching auxiliary-file admission and tree-only project checks.
- `b7420c8d4` and `780586db7`: CPython word boundaries and whitespace differ from Rust regex defaults.
- `a5b7fc215`: canonical expansion from SQL-selected source relations, without a CST index.

The fixes preserve raw source bytes and isolate malformed canonical modules.
They preserve the conditional linter's textual line ordering and the optimizer's distinct file/source line counts.
Matching excludes auxiliary operator files from default AST analysis and omits project obligations for tree-only calls.
Generated Unicode classes use CPython 3.13's Unicode 15.1 tables within existing SQL size limits.
No rule's diagnostic was suppressed to make these tests pass.

The original corpus now contains 1,098 cases: 279 upstream cases and 819 original-evaluated supplements.
There are 975 detector observations, including 870 positives, over 974 tests.
They contain 778 distinct normalized observations, including 680 positives.
The other 124 cases retain utility or CLI assertions. They are not native detector observations.
The configured-default corpus expands from 62 to 1,176 observations, including 825 positives.
It includes every distinct recorded input and all 39 focused file-contract inputs.
A further 106 diagnostic-family controls remove one code at a time while retaining the other original findings.

Two attempts at the repository matching assertion hit the 115-second command cap.
A short external sample showed corpus value-index insertion and SQLite writes during preparation.
It did not establish the earlier hypothesis that Unicode regex matching caused the timeout.
A cheaper AST predicate still exceeded the cap, because AST preparation preceded SQL source selection.
Matching therefore moves module demand onto raw source relations, selected in SQL.
The full inventory contains 2,870 production files plus one auxiliary operator file: 57,328,575 source bytes.
The original selector admits 139 files containing 2,771,856 bytes; SQL must select exactly that set.

The first source-expansion implementation dropped the expansion setting and returned module scalars.
Positive regressions caught this. The next implementation exposed missing diagnostic-span proofs.
These intermediate builds are rejected evidence, even though a clean repository assertion passed in one of them.
The producer must prove exact spans against the actual snapshot source hash and an admitted whole-file witness.
SQL must not fabricate offsets or substitute transformed source.
One stale Rust test build was stopped after native source changed; its output is not a final test result.

Final execution receipts and validation results follow below once the repaired build passes.

Repeated execution exposed a further lifecycle defect in the new source path.
The first owner evaluation passed; repeating the same evaluation failed its anchor check.
Prepared SQL rows survived, but source-span proofs belonged only to the first session.
The focused red test is `9028f8b42`.
Source proofs now share the existing bounded SQL owner, keyed by the exact source digest.
Their bytes count toward the existing owner budget, and owner reset drops them with cached row definitions.
This retains the proof requirement when later consumers reuse prepared rows.

The configured receipt validator found ambiguous test identities before certification.
Vitest truncates long interpolated labels, including trailing observation IDs.
Short numeric observation labels now bind each assertion to its oracle record.
The original test name remains in the fixture and failure text.
Eight evidence-verifier tests cover missing traces, original/native confusion, stale policy hashes,
API mode confusion, missing configured comparisons, duplicate assertions, and repeated upstream names.
The configured reports were rerun after correcting their labels.

A configured group reached the command cap under concurrent validation load.
It now runs as two independent groups, with the same cases and assertions.
Two unsharded batch-nine compatibility commands also reached the cap.
Their results were discarded; compatibility uses the four existing CI shards per API.
The repository matching assertion remains whole in each command.

### Final validation on the repaired build

Addon SHA-256: `464f424955f220cdf8ed74937c93d97cc461e9ba5b19e515f11cc68e86920ba8`.
The final native build and test build completed separately from bounded test commands.

| Gate | Result |
| --- | --- |
| Unchanged original assertions and supplements | 1,098 passed; all 975 observations reproduced |
| Native snapshot assertions | 1,098 passed; 975 complete results, including 870 positives |
| Native workspace assertions | Same complete results and case coverage |
| Shipped defaults | 2,352 complete-result comparisons over 1,176 original observations |
| Matching file selection | 612 comparisons; repository selection equals the original 139-file set |
| Focused contracts and repair regressions | 57 tests passed; file-contract and repair inputs exercise both APIs |
| Individual diagnostic removal controls | 106 tests passed, each through both APIs |
| Disabled whole-detector controls | All ten rejected; 163 deliberately failing source assertions |
| Compact batch-ten replay and receipt binding | 23 tests passed |
| Evidence verifier regressions | 8 tests passed |
| Native Rust DSL suite | 1,576 passed, 23 existing ignored, 114 outside the selected namespace; 68.45 seconds |
| Batch-nine compatibility | 228 source cases per API; 269 complete results, including 156 positives |
| Batch-nine replay and native contract tests | 32 tests passed |
| Rule generation | All ten documents reproduce byte for byte |
| Original file-contract and configured fixtures | Reproduced without changing original detector implementations |
| Typecheck, boundaries, DSL grammar and reference examples | Passed |

The final complete matching assertion passed in 39.16 seconds for snapshot and 49.29 seconds for workspace.
These are pytest durations under validation load, not cold CPU benchmark measurements.
Its full input remains 2,871 files; the SQL rule performs source selection itself.
The final receipt validates every source case, native call trace, current policy hash, and configured observation identity.
It distinguishes 778 distinct normalized observations, including 680 positives, from repeated observations.
It also identifies the 124 retained utility and CLI cases separately from native detector cases.
Batch-nine receipts were refreshed on the same addon. Its historical kernel measurements were not rerun.

No cumulative repository benchmark or new Linear issue was added.
The historical complete repository result still covers 80 rules, rather than the 100 authored ports.
The final structure check has 24 passing metrics and one failing metric with three pre-existing findings:
`native/src/code_rule_dsl/` has 26 flat files, `scripts/dsl/` has 31, and `test/engine/native/` has 26.
Each limit is 25.

The first final structure run also caught three new layout and packaging problems.
The recorder now uses the shared Python fixture resolver.
The cache-lifetime regression now lives with the conformance file-contract tests.
Both compressed corpora use Git LFS; their index entries are verified 133-byte pointers.
The existing CI LFS action fetches them. These changes leave only the existing flat-directory findings.
CI now validates the union of live source shards against the complete original corpus for each API.
It also validates the union of configured reports, including both whole-repository configured tests.
The workflow parses, and its three aggregation snippets passed against the final recorded reports.
The final relocated contract tests still pass all 57 tests, and the shared-resolver fixture reproduces exactly.

## 2026-11-05 — Retraction of the batch-ten completion claim

Commit `8b0c99311` passed the recorded gates but did not establish exact translation.
The subsequent completion message was wrong. Fixture parity was presented as completion without an adequate semantic audit.
A fresh differential audit found 43 counterexamples in two ports before any production-code repair.
The unchanged original rules supplied the expected results.

| Translation defect | Observed consequence |
| --- | --- |
| Python `lstrip()` replaced with SQLite `ltrim()` | Matching missed SQL f-strings prefixed by tabs, newlines, and Unicode whitespace |
| Python `upper()` replaced with SQLite `upper()` | Matching missed long-s, dotless-i, and a final character whose uppercase expansion crosses the prefix boundary |
| Uppercase-name equality tested with SQLite ASCII casing | Matching falsely rejected counters containing lowercase Unicode identifiers |
| Python decimal layer parsing replaced with ASCII regex and integer casts | Matching missed Unicode layer names and could select a later ASCII layer instead of the first Unicode layer |
| Python regex whitespace passed to Rust unchanged | Bounded paths missed four control-whitespace variants accepted by the original textual check |

The red test commit is `6e409fbeb`; the production fix is `6232dc2a1`.
The fixes generate CPython-compatible whitespace, uppercase preimages, uppercase-change classes, and decimal-digit relations.
SQL still makes the rule decisions. No engine capability or execution limit changed.
The expanded focused corpus has 102 original-evaluated inputs, including positive and clean controls.
All 103 focused tests pass, including the existing repeated-owner regression; both APIs replay every input.
Additional GraphQL scope and generated-index alias-timing probes matched the originals without production changes.

This record supersedes the prior completion wording. Passing a finite corpus is evidence for those inputs, not proof of exactness.
The 124 retained Python helper and CLI cases still do not constitute native detector comparisons.
The full-repository benchmark still covers 80 rules. No broader benchmark or Linear ticket was added in this audit.

The repair rerun preserves all 975 recorded detector results through both native APIs, including 870 positives.
Each source-suite run has 1,098 passing cases; 974 invoke native detectors and 124 remain Python helper or CLI cases.
The v3 receipt now lists those retained Python case IDs rather than presenting their pass status as native coverage.
Configured coverage is 1,239 original observations, including 875 positives: 2,478 result comparisons and 716 selection comparisons.
The whole-repository matching assertion passes separately through both APIs.
The 106 diagnostic-removal controls, nine earlier repair cases, 103 focused contract tests, and 23 replay gates pass.
Typecheck, boundaries, grammar validation, and byte-for-byte regeneration pass.
The native engine is unchanged in this repair; the earlier Rust-suite results were not rerun or presented as new measurements.
No claim of whole-batch semantic certification follows from these passing counts.

## 2026-11-05 — Batch-ten predicate audit and TDD repairs

The additional audit reproduced 37 failing detector regression cases before their repairs.
These are failing cases, not 37 independent engine requirements.
The defects affect matching, bounded paths, HNSW, optimizer v2, and generated-index checks.
The [semantic audit](data-layer-batch-ten-semantic-audit.md) maps all ten source collectors to their SQL contracts.
It also explains which original Python assertions remain outside native detector comparisons.

| Red commit | Repair commit | Reproduced failure |
| --- | --- | --- |
| `8f4bec231` | `ab50e2d7b` | Annotation recursion, bare `None`, Unicode layer imports, distinct MA031 messages, and repeated-finding order: 15 cases |
| `ab68164bc` | `ace8ed35a` | A bounded-path violation after NUL reported line 1 instead of line 7: one case |
| `b845ca96f` | `1a4cb6b83` | Standalone HNSW inventory checks lost caller cwd context: two cases |
| `ae1f47092` | `f90b0b3ae` | Separate imports became an import-from match; direct tree results lost collector ordering: 12 cases |
| `17a711ec7` | `8feee5787` | First/last declaration policies and complete string suffix checks across NUL: seven cases |

Matching now evaluates its actual annotation grammar in SQL.
That grammar stops at calls and preserves an attribute when its prefix has no annotation name.
It handles subscripts and unions without replacing Python semantics with a general qualified-name approximation.
The producer expands only demanded expression roots.

The precedence review checks the original traversal and replacement policies explicitly.
Matching uses the first eligible model-config declaration and the last duplicate delegate parameter.
Generated-index checks use the first source parameter and the last keyword-only default.
Optimizer suffix checks inspect the complete value, including characters after NUL.
Clean controls also reject a suffix followed by NUL.

The source adapter transports HNSW cwd context as input metadata.
SQL decides forbidden-file membership; Python does not precompute that rule decision.
The configured repository rule retains the original reporter behavior.

The harness received three separate corrections:

- The v4 receipt requires every focused assertion identity. Empty selections, skipped cases, partial coverage, and duplicate coverage fail verification.
- The source runner's `--mutant` option now creates an actual disabled-policy directory. Previously, it skipped baseline comparison and required separately supplied disabled policies.
- Original detector recording now holds a process lock. Original source APIs rewrite shared files in content-addressed directories.

An initial control invocation accidentally used active policies with `--mutant`.
The verifier rejected its successful assertion result. That run is not negative-control evidence.
The corrected disabled-policy run has 163 expected failures and 935 passes.
Its native call traces contain no findings, and positive assertions reject all ten disabled detectors.
The disabled programs reproduce byte for byte from the final policies.

One initial full control command reached the 115-second cap while it still used active policies.
It was stopped and was not counted as completed coverage.
The corrected complete disabled-policy run finished within the cap.

A parallel original recording also produced one inconsistent graph observation for unchanged recorded inputs.
The original APIs use `write_text` on shared hash-named source files, which permits a concurrent truncation/read race.
A separate recording reproduced the prior findings. The process lock prevents overlapping recorder calls.
The refreshed original observations preserve all 975 expected results.
Only 18 HNSW observations gain cwd transport metadata.
This distinguishes three previously coalesced input contexts: 781 distinct observations, including 683 positives.

The final evidence uses CPython 3.13.18 and Unicode 15.1.
The native addon SHA-256 remains `464f424955f220cdf8ed74937c93d97cc461e9ba5b19e515f11cc68e86920ba8`.
No Rust source, native capacity, or statement limit changed in this audit.

| Final gate | Result |
| --- | --- |
| Unchanged source assertions | 1,098 cases pass per API; 974 invoke native detectors and 124 retain Python helper or CLI targets |
| Complete source-result parity | 975 observations, including 870 positives, match through both APIs |
| Focused contracts | 276 original-evaluated inputs, including 194 positives; 552 API comparisons and the cache-lifetime test: 277 passing tests |
| Shipped defaults | 1,392 original reporter observations, including 978 positives; 2,784 result comparisons and 878 selection comparisons: 3,662 passing assertions |
| Whole matching inventory | Both APIs pass for source assertions and shipped defaults; the input remains 2,871 files and 57,328,575 source bytes |
| Diagnostic-removal controls | All 106 observed families pass through both APIs |
| Earlier repair and native contract tests | Nine repair cases and eight engine contract tests pass |
| Evidence verifier | Twelve tests pass, including subprocess exclusion for original recording |
| Adjacent batch-nine compatibility | 32 TypeScript tests pass; its unchanged source observations were reused when refreshing file bindings |
| Generation | All ten documents and the focused corpus reproduce byte for byte |
| Typecheck, boundaries, and DSL grammar | Pass |

The v4 receipt binds 126 implementation and evidence files.
The new recorders are grouped under `data_layer_ten/contracts/`, and recursive receipt hashing includes them.
This removes the new flat-directory violation introduced during the audit.
The structure check returns to its three pre-existing findings in `native/src/code_rule_dsl/`, `scripts/dsl/`, and `test/engine/native/`.
Its other 24 metrics pass.

The 124 retained Python cases are identified and categorized in the semantic audit.
They cover legacy helpers, decorator helpers, AST handoff, CLI behavior, Python DTOs, constants, paths, approval configuration, and Python cache instrumentation.
Their passes do not count as native detector comparisons.

This delivery validates the ten rules at the pinned source through the stated gates.
It does not prove equivalence for every possible Python program or reimplement the original Python CLI.
The branch has 100 authored ports out of 142, with 30 documents in the private pack.
The historical complete repository result still covers 80 rules.
No cumulative cold benchmark or new native-kernel measurement was run in this repair.
The earlier Rust-suite result was not rerun or presented as new evidence.

## 2026-11-05 — Execute the 67 unfinished batch-ten helper cases

The previous completion statement overstated native coverage.
It left 67 rule-relevant unit cases on their Python implementations.
Overlapping detector tests did not translate those assertions.
This delivery executes those cases through native contracts.
It retains the upstream assertions in 66 cases and explicitly translates one instrumentation assertion.

The assertion ledger names all 124 previously retained cases.
It assigns 67 to native helper execution and 57 to Python host assertions.
The host cases comprise 15 CLI cases, 14 dataclass cases, and 28 constant or module-layout cases.
They remain executable tests and do not count as native coverage.

The native cases cover 47 legacy AST-helper cases, 12 decorator cases, four parse/handoff cases,
two path cases, one approval-validation case, and one traversal-instrumentation case.
Five non-shipped SQL contract programs implement their twelve entry points.
The legacy helper contracts preserve their own semantics, which differ from the active graph collector.
Decorator extraction and path classification share production relations or predicates.
The matching helper uses the production matching-context relations.
All ten shipped rule documents regenerate byte for byte without changes.

Original helper bodies are forbidden during native source runs.
The adapter maps native node IDs to Python AST objects for the unchanged assertions.
It does not perform the original selection logic.
Native syntax facts decide parse success or failure before Python constructs its AST DTO.
A regression test rejects native acceptance followed by a Python parse failure.
The instrumentation case asserts actual native parse and module-walk counts, both exactly one.
Its receipt records the original and translated assertion hashes.

Three failing-test commits precede the implementation:
`76838b97f` requires all 67 cases, `e92143987` requires native call traces,
and `92e7a4f54` prevents DTO parsing from masking a native parser defect.
The verifier tests also reject missing observations, changed programs, wrong APIs,
original executions, and missing rows for false Boolean results.

| Gate | Result |
| --- | --- |
| Native helper source assertions | 67 cases and 104 complete observations pass per API |
| Helper replay | 104 original observations plus 15 original-evaluated supplemental observations; 238 comparisons across both APIs |
| Helper removal controls | All 12 entry points reject disabled implementations; 24 replay tests including parity tests |
| Complete source corpus | 1,098 cases per API: 974 native detector cases, 67 native helper cases, 57 Python host cases |
| Detector parity | 975 observations, including 870 positives, remain unchanged |
| Evidence verifier | 19 tests pass |
| Generation | Helper programs, original helper observations, and all ten production programs reproduce byte for byte |
| Typecheck and boundaries | Pass |
| Structure | 24 metrics pass; the same three directory-size findings remain |

Snapshot source evidence uses three quarter shards and two eighth shards.
Workspace source evidence uses eight eighth shards.
Some earlier larger or concurrent runs reached the 115-second cap.
Those incomplete runs are excluded; the remaining workspace shards passed on separate retries.
CI now uses eight bounded source shards for each API.

The production programs and native addon are unchanged.
The addon SHA-256 remains `464f424955f220cdf8ed74937c93d97cc461e9ba5b19e515f11cc68e86920ba8`.
The unchanged full matching-repository reports, configured-default reports, focused-contract reports,
and detector-removal report from the preceding audit are reused explicitly.
They are not new full-repository runs.
The new native helper reports and removal controls supply the additional evidence.
No cold repository benchmark, native build, or Rust-suite run occurred in this delivery.

The v5 receipt binds 137 files and verifies each helper case, observation, native call, and contract hash.
It records the 57 host case IDs and their reasons separately.
The branch still has 100 authored ports out of 142; the historical cumulative repository run covers 80.

The adjacent batch-nine file bindings were refreshed for the CI change.
Its existing source observations were reused without claiming new source executions.
All 32 TypeScript compatibility tests pass in this delivery.

The combined detector replay command reached the 115-second cap and is excluded.
A five-rule replay group also reached that cap and is excluded.
The combined graph replay and its single-API replay also exceeded their 90-second test timeouts.
Each detector replay test now covers half its observations for one API.
The two disjoint halves retain every observation.
CI runs metadata and controls separately, then each detector, API, and part.
Local verification uses those same bounded commands.

Final bounded replay verification passes all 40 detector/API partitions and 14 metadata or control tests.
Their exact union has 54 assertion identities, with no failures, duplicates, or missing identities.
Partitioning changes test grouping, not corpus size or expected results.
The final helper replay has 24 passing tests, and the evidence verifier has 19 passing tests.
Final typecheck, CI YAML and shell syntax, and both batch-nine and batch-ten file bindings pass.

## 2026-11-06 — Complete transferable assertions from the 57 retained host cases

The earlier host-only classification combined Python details with portable requirements.
In particular, the banned-symbol constants define detector behavior.
The existing recorded corpus had positive diagnostics for six of the thirteen banned symbols.
The other seven were `_PlanBackedGraphTraversal`, `_TxnBoundGraphQueryBuilder`, `_get_or_create_plan_flush_context`,
`_lazy_result_for_plan_ref`, `_make_graph_query_builder`, `GraphQueryTerminalStep`, and `_GraphQueryTerminalMode`.

The new recorder executes all 57 original cases without changing their assertions.
It retains their source, source hashes, assertion expressions, outcomes, and original CLI observations.
It verifies imported upstream files against the source pin.
Generation reproduces the committed transfer record byte for byte.

The audit finds 75 transferable assertion sites across 47 cases:

| Contract | Assertion sites | Rerail evidence |
| --- | ---: | --- |
| Banned constants | 15 | Exact vocabularies in the shipped SQL; all 13 positives, 13 allowed lookalikes, and 13 other-family controls |
| Result values | 15 | Fields, types, equality, and field-specific inequality through both native APIs |
| Command | 30 | Arguments, help, failures, text formatting, clean output, JSON output, and exit behavior |
| Target scope | 15 | Relative target identity, Python suffixes, native admission, rejected lookalikes, and independent command working directories |

The remaining 16 sites concern Python container types, hashing, mutation exceptions, checkout layout, or module documentation.
Ten cases contain only those sites. Three cases contain both portable and Python-specific sites.
The ledger identifies each assertion separately, rather than excluding an entire mixed case.
The original source harness still executes those 57 cases in Python.
The additional Rerail counterparts have separate evidence and do not inflate native source-case counts.

The recorder constructs a small, genuinely clean four-file graph baseline from upstream examples.
It removes deleted shortcuts and supplies required methods and classes.
The original detector evaluates every mutation and validates the clean controls.
The new corpus has 39 complete observations, including all thirteen positives.
Both native APIs compare every result: 78 comparisons.
Thirteen controls remove one banned SQL literal and reject the resulting missing finding.
The shipped rule documents and native engine require no changes for these cases.

The upstream CLI tests mock detectors to return synthetic `Violation` values.
Their counterparts author small native regex rules that emit those values at actual source locations.
The real command receives the native results. No original detector runs in those native checks.
Result-value tests use the same transport through snapshot and workspace execution.
These tests validate output transport and do not count as production detector positives.
The fields-only upstream test uses an original constructor example to inspect the native result schema.
Native spans map to the original one-based line numbers.

The command now supports `--help`, `--check`, and `--json`.
It defaults to text output and report-only exit behavior, as the original graph command does.
`--check` returns 1 for blocking findings. Report-only rules retain exit status 0.
Engine failures retain exit status 2. `--json` provides the prior structured output.
Existing JSON consumers must add `--json`.
Enforcement callers must add `--check`.
The compatibility tests now supply those flags explicitly.

TDD commit `30cfe2b4b` records failures for missing command help and missing transfer evidence.
Commit `407e2e269` records the classification boundary between field-name sets and Python hashability.
Development also exposed three adapter defects.
SQL inspection crossed UNION arms and alias boundaries. Production observations used Python DTO keys.
A fields-only assertion had no constructor input.
The adapters now preserve the intended arm, public DTO names, and schema example.
Those failures were harness defects, not missing engine capabilities.

The v6 receipt requires the exact union of 102 transferred tests and controls.
A separate completion assertion requires that receipt evidence.
The verifier rejects missing tests, duplicates, changed source, omitted observations, and erased assertion decisions.
CI regenerates the original record and runs six bounded native test groups.
Each command retains the 115-second cap.

The existing detector, helper, configured-default, focused-contract, and repository reports are reused for unchanged execution contracts.
This delivery does not repeat a cold run across the batch or the repository.
The native addon remains unchanged; no Rust build or engine profiling run occurs here.

Final validation passes all 102 transferred tests and the separate completion gate: 103 passing tests.
All 25 verifier tests and 32 production CLI compatibility tests pass.
The fourteen core completion/control tests pass with the v6 receipt.
Both batch receipts bind their current implementation files. The v6 receipt binds 147 files.
Typecheck, boundaries, CI YAML, and CI shell syntax pass.
Both the transfer fixture and existing helper observations reproduce byte for byte.
The structure check has 24 passing metrics and the same three directory-size findings.
A temporary duplicate fixture-root resolver was removed in favor of the shared Python resolver.

## 2026-11-06 — Recheck batch ten against the completion criteria

The review targets `9ceb4c8ef` and the pinned source.
It uses the restart plan's per-batch and migration criteria.
The [review](data-layer-batch-ten-dod-audit.md) and [record](../gate/data-layer/fixtures/host/data-layer-port/batch-10-dod-audit-20261106.json) retain the findings.

Six original scenarios run through both public native APIs in bounded commands.
Two default scenarios expose missing optimizer findings: malformed verifier syntax suppresses document checks.
The original reporter emits PV2-050 for a missing document and PV2-051 for a stale document code.
Both native modes return no finding and no engine failure.
A valid verifier and an absent verifier provide passing controls.

Two further scenarios identify unsupported production configuration.
Explicit matching consumer roots produce MA010 in the original but cannot enter the current public Rerail scope.
Invalid schema-approval entries produce SIDP000 in the original but have no production Rerail input or reporter branch.
These cases compare explicitly configured original contracts with missing Rerail configuration support.
They do not establish a default-discovery mismatch or an active approval failure with the pin's empty approval list.

The review also identifies missing persistent-workspace rule transitions and the cumulative 100-rule repository gate.
The existing 75 transferred assertion sites remain covered; that coverage does not complete every rule interface.
No production code, engine capacity, or Linear issue changed during this review.
No full repository cold execution occurred.
The original modules were checked against Git source blobs. The native addon hash remains unchanged.
All four default-control evaluations match. Eight evaluations expose the stated defect or configuration gaps.

## 2026-11-06 — Complete the batch-ten repair

The optimizer file-presence regression first passed four controls and failed four positive evaluations.
The fixed SQL guard passes all eight evaluations.
Production matching roots now drive filesystem discovery, scope, SQL classification, and repeated CLI options.
Fourteen original approval scenarios pass through both APIs: 28 exact reporter comparisons.
These include entry errors, signature errors, duplicates, Unicode canonicalization, and verified suppression.
The shipped verifier uses the original public key. Signing delegates to the original source command only on explicit invocation.
Tests use isolated fixture keys. No deployed signing credential was read or used.

The new persistent matrix compares five revisions per rule with original configured reporter results.
It sends changed files only and preserves unedited dependencies.
All 52 focused DoD tests pass on the pre-rebuild addon.

The twenty-rule cumulative preflight exposed an engine capacity defect.
Several coalesced canonical producers exceed the 8 MiB transport limit and fail the entire wave.
That run records 3,122 engine failures and cannot establish parity or performance.
The repair partitions transport by complete source document, while preserving the full rule scope and row order.
A new Rust regression covers multiple documents above the aggregate capacity.
The first Rust test command reached its 115-second compilation bound before executing the test.
Its log is not a failing assertion result.
A production rebuild and the complete 100-check oracle are in progress.
No completion claim or refreshed receipt is issued at this point.

### Recovery runs after the capacity preflight

The complete original oracle finished at the pinned source with 100 checks.
It contains 9,021 code files and seven required documents: 138,077,148 input bytes.
The original reporters produce 25,262 findings, with no source-blob mismatch.
Their phase uses 392.407 CPU seconds; process-family CPU is 429.978 seconds.
Process wall time is 511.261 seconds; maximum single-process RSS is 4,859,510,784 bytes.
Concurrent build activity means that wall time is not a controlled comparison.

The first complete native run failed: 25,027 findings and 4,776 engine failures.
All 235 missing findings belong to `pure_lazy_runtime_purity`; no extra findings appear.
Cold parent CPU is 2,013.703 seconds and cold wall time is 2,100.616 seconds.
Compilation takes 1.530 seconds, separately from source configuration and cold execution.
Process-family CPU is 2,046.194 seconds, with 8,121,270,272 bytes of peak root RSS through close.
These numbers describe a failed run, not an accepted performance baseline.
The local `native-100*` files preserve its report and process measurements.

Three reduced regressions drive the subsequent engine repairs.
A streamed canonical protocol removes the aggregate 64 MiB wire limit while retaining per-consumer output bounds.
Repository consumers retain source lengths after buffer release and use the existing scope-byte work allowance.
Physical subtree input partitions preserve one logical repository relation and its file/node identities.
The 100-stage dependent-consumer guard remains unchanged and passes its regression.
The identity rule selects constructor calls before unrelated lexical ownership.
Its new original-evaluated positive and clean cases each include 7,000 unrelated functions.

A three-kernel run over all 9,028 inputs first fails on producer capacity and identity ownership.
It uses 98.455 process-family CPU seconds, with 3,464,069,120 bytes of maximum single-process RSS.
After those repairs, the identity kernel has no failures, but both lazy kernels exhaust argument-selection work.
That failed run uses 421.981 process-family CPU seconds and 3,689,611,264 bytes of maximum single-process RSS.
The file-local parent lookup had searched every file's postings before SQLite applied its path filter.
A 128-file regression reduces 1,280 candidates to ten for the same ten matches.
The repaired virtual table passes file and value constraints together and retains SQLite's comparison checks.
All four binding tests pass, including IN batches, duplicates, NULL, numeric affinity, and NOCASE controls.

The rebuilt addon passes all 54 focused DoD tests.
Typecheck and boundary checks pass. The structure check retains only the three existing directory-size findings.
Fresh source, native replay, and complete-domain kernel gates are running.
An external five-second sample now exists for the kernel run; temporary SQLite writes dominate that interval's active stacks.
The earlier sampling attempt missed the process and supplies no profiling evidence.
Final cumulative acceptance remains open until the recovered kernels and all 100 rules pass.

### Indexed graph and recursive-container recovery

The [recovery record](performance-decisions/data-layer-batch-ten-global-recovery-20261106.json) retains query text, plans, budgets, measurements, and regression identities.
The later captures distinguish SQL work exhaustion from the outer clause budget.
At both failures, SQL work is zero while the outer clause still has allowance.

`graph_edges` searched the function relation by execution scope for each target.
The symbol index could not serve its file/id join.
A composite identity index returns all 85,677 captured edges in 0.163 CPU seconds, including result hashing.
The unindexed offline command reaches its 115-second bound and supplies no completed comparison result.
The original-evaluated graph regression fails before the index repair and passes afterward.

Purity's recursive child query used an OR between parent and call-argument relationships.
Separate recursive arms preserve the original disjoint cases and permit indexed access.
A referenced 1,000-entry static-container regression fails before this repair and passes afterward.

The next complete-domain run still fails at lazy `rollups` and purity `global_state`.
The rollup join concatenates category prefixes onto function columns, hiding the symbol index.
Decoding the producer's category prefix restores the plain indexed symbol lookup.
Purity's reference seed searches function-body identifiers separately for each module binding.
A grouped reference-name batch replaces that repeated search and preserves the original seed multiplicity.
The expanded fixture uses populated graph rollups and referenced/unreferenced globals beside active functions.
Both retained kernel regressions fail at the demonstrated stages before these repairs, then pass through both APIs.
No work allowance changes in these SQL repairs.

The recovered three-kernel run covers all 9,021 code files and seven required documents.
It matches 235 expected findings with no extras, omissions, or engine failures.
Cold parent CPU is 324.736 seconds; cold wall time is 373.948 seconds.
Compilation is 0.086 seconds, recorded separately.
Process-family CPU through close is 327.361 seconds; process wall time is 377.162 seconds.
Maximum root RSS through close is 3,783,933,952 bytes.
This is a three-kernel result, not a 100-rule performance comparison.

All 60 earlier checks pass their original recorded corpus, preserving each observation's complete file domain.
The last 748-observation check needs eight bounded observation shards; no repository domain is split.
Both batch-eight assertion APIs pass all 456 original cases, 305 complete API observations, 85 fact inputs, and 12 graph-state inputs.
Their disabled-detector controls reject positives in all nine original test modules.
The refreshed batch-eight/nine replay and metadata gates pass 88 assertions.
The expanded DoD matrix passes all 56 tests, including 57 persistent revisions across ten rules and the optimizer dependency sequence.

Direct full native-unit commands reach the 115-second diagnostic bound without a completed result.
Exact-name test partitions execute all 1,674 active unit tests successfully.
The 22 existing ignored external-corpus benchmarks remain named exclusions from the standard unit suite.
The first local coverage check incorrectly treats ignored benchmarks as missing work and reads an inner subprocess's summary.
The final inventory validator checks every test identity, the expected ignore set, and the final parent summary.
The isolated slow assignment-chain regression also passes.

The cumulative 100-rule attempt initially stops before engine evaluation because the command omits the Python environment.
Its logs remain under `native-100-preparation-env*`; they provide no rule parity evidence.
The corrected command selects the validated environment explicitly and is running without concurrent validation jobs.
The cumulative receipt remains pending at this point.

### Disk-pressure failure during the cumulative gate

The first corrected-environment 100-rule run reaches all 9,028 inputs, then fails.
The first three displayed errors concern matching. The saved report contains 25,354 failure rows across 56 distinct clause/error combinations.
This count includes repeated disk I/O failures and the bootstrap work interruption diagnosed in the subsequent ample-disk run.
The filesystem has about 600 MB free at diagnosis.
This run uses 2,251.489 process-family CPU seconds and 2,825.660 wall seconds.
Maximum single-process RSS is 9,218,375,680 bytes.
It is an incomplete diagnostic run, not a parity pass or an accepted performance baseline.
Its files remain under `native-100-disk-pressure*`.

Duplicate later SQL snapshots are removed after retaining their query plans, budgets, and compact recovery record.
The original captures are preserved as verified lossless zstd archives: approximately 600 MB and 805 MB.
Their local manifests include restoration commands.
An inactive batch-seven profiling build cache accounts for 161 GiB.
Cargo cleans that rebuildable cache in two bounded commands; the first command reaches its time bound.
The saved engine binaries, source snapshot, reports, and journals remain intact.
Available filesystem space rises to about 163 GiB before the retry.

A red/green regression adds filesystem headroom to the complete-run profiler.
It records initial, minimum, and final free bytes for workspace and SQLite temporary storage.
The tests also retain the existing waited-for grandchild CPU and exit-status checks.
No execution backend or measurement configuration flag is added.
The complete 100-rule gate is running again with the same native addon and rule documents.

### 2026-11-06: isolate and repair bootstrap query exhaustion

The ample-disk 100-rule retry fails only `bootstrap-invariants`.
Process CPU is 2,229.418 seconds; wall time is 2,314.323 seconds; peak RSS is 10,844,078,080 bytes.
The filesystem retains at least 152,371,703,808 free bytes. This failure is independent of the earlier disk-pressure failure.
There is no accepted cumulative result from that attempt.

The complete bootstrap kernel reproduces the failure in isolation.
Its final query exhausts SQL work after preparation. Isolated packets require 43,679,249 VM steps in total.
The helper-directory regex also runs 304,753 times despite testing a property of each file.
File-leading indexes cannot serve the consumers' repository-wide kind and callee lookups.
A captured-data experiment adds three relevant lookup indexes and replaces that exact regex with SQL string predicates.
It returns the same empty result bag with 765,030 VM steps and no regex calls.
This replay excludes preparation and is not an end-to-end speed result.

TDD commits `aa4ea3388` and `0b4af5c47` retain the real 153-file source domain and a positive helper mutation.
Both APIs fail before the repair and pass afterward, including the clean control.
The production fix replaces two unsuitable selection indexes, keeps file/id indexes, and indexes function basename/name.
It uses the exact literal helper-prefix/no-later-slash predicate in SQL. Budgets remain fixed.
The normal addon passes the kernel over the complete 9,028-input repository:
10.032 process-family CPU seconds, 10.409 wall seconds, and 1,786,707,968 bytes peak RSS.
The complete 100-rule gate remains pending. No aggregate speed claim follows from the isolated kernel result.

### 2026-11-06: optional host-configuration decoding boundaries

A review during the final cumulative run finds two independent host defects.
Explicit matching-root discovery strips UTF-8 BOM with Node's default decoder, unlike Python's `read_text(encoding='utf-8')`.
Original-evaluated BOM, bare-CR, and CRLF records now pass through both production APIs after preserving BOM and translating disk-file newlines.
TDD commits: `92b9dcc98`, `dc28f5445`.

Node's Base64 decoder also accepts non-ASCII text appended to a valid approval signature; original Python rejects it.
Eight encoding scenarios preserve both original acceptance and rejection, including ignored ASCII junk and malformed padding.
The first attempted decoder fails the trailing-data scenario and 366 systematic original-Python cases; it is rejected.
The committed compatibility wrapper uses Node's standard byte decoder after matching the recording environment's string validation and padding rules.
All 68,008 recorded Python decoder cases and sixteen production-API encoding comparisons pass.
TDD commits: `01a1d1d0b`, `f667f6770`.
The permanent fixture is generated and compared byte for byte in CI.
The combined approval and root groups pass 31 tests.

The cumulative run uses neither explicit application roots nor nonempty schema approvals.
These fixes do not change its selected paths, rule documents, workload, or native addon.
Brief focused host validations overlap the cumulative run; unrelated native build activity is also present on the machine.
Process-family CPU remains attributed to the measured process and its waited-for workers. Wall time is not a controlled speed comparison.

During the final full run, a one-second external `sample` capture observes the active global-evaluation worker under `Storage::evict`.
The active stacks contain string copying, sorting, allocator work, and `BTreeSet` construction.
The source at `relational/storage/contexts.rs:359` rebuilds a set of live slot names from all context keys when layout capacity is exceeded.
That is a concrete lead for the next performance investigation. This repair leaves the engine source and measured addon fixed.
One wall-stack snapshot does not establish aggregate CPU attribution; blocked threads also appear in the sample.
The compact recovery record preserves the sample hash, collapsed stacks, and source location.
The sampling overhead is separate background work during this baseline, alongside the previously noted brief host checks and unrelated builds.

### 2026-11-06: accepted complete 100-rule baseline

The normal addon `21554b9bf6df95211d14407ececdeb5657b0a861eb44cba11193b65feffc0910` completes all 100 rules on 9,021 code files and seven required documents.
Expected and actual findings are both 25,262. There are zero extras, missing findings, or engine failures.
The native result, process receipt, and workspace-close receipt are under `dod-completion-20261106/native-100-complete*`.

| Boundary | CPU seconds | Wall seconds | Peak RSS bytes |
| --- | ---: | ---: | ---: |
| Original 100-rule `run_plugins` phase, including waited-for workers | 392.407 | 468.218 | 4,859,510,784 for the original process |
| Original complete process family | 429.978 | 511.261 | 4,859,510,784 largest process |
| Native rule compilation | 2.151 | 2.249 | Included in native process high-water mark |
| Native cold parent phase, excluding child CPU | 2,768.671 | 3,025.548 | Included in native process high-water mark |
| Native complete process family through exit | 2,856.494 | 3,035.380 | 10,234,003,456 largest process |

Native host preparation is separately recorded: 1.613 parent CPU seconds and 3.789 wall seconds.
The close receipt confirms the same 10,234,003,456-byte root high-water mark through cleanup.
Minimum workspace free space is 122,239,995,904 bytes. Disk pressure does not invalidate this run.

This is a correctness pass and a fresh baseline, not a performance success.
Measured complete-family CPU is about 6.64 times the original run's CPU; maximum process RSS is about 2.11 times as large.
The original wall measurement overlapped build activity. Brief host checks, unrelated builds, and one external stack sample overlap this native run.
Wall-time ratios are therefore not controlled speed comparisons.
The historical accepted 80-rule run used a different engine and inventory; its 695.641 parent CPU seconds remain historical evidence.
The native cache-eviction sample provides a concrete next performance lead. It does not establish aggregate attribution from a one-second capture.

The final evidence gates pass: 57 focused DoD tests, 88 batch-eight/nine replay tests, 72 batch-nine production/parser/shape tests, and 36 evidence-validator tests.
The broader native assertion groups contain 4,317 passing assertions; the earlier sixty-rule replay has 3,079 native calls and no missing coverage or mismatches.
Receipt generation catches one previously unrun legacy check after the earlier `serverplan_steps` timeout.
The complete step-observation shards pass, and `serverplan_target_architecture` now passes twelve observations, including ten positives.
The accepted v7 receipt binds the complete current evidence rather than treating the interrupted legacy orchestration as success.

Type checking, boundaries, Windows path checks, and CI Bash syntax checks pass.
The structure baseline remains 24 passing metrics and three pre-existing directory findings under one failing metric, with no new finding.
No original linter assertion is skipped by the repaired source harness.
The existing 22 ignored Rust external-corpus benchmarks are named in the validation summary and do not represent linter-test exclusions.
All 75 transferable assertion sites have native counterparts; the 16 Python-object-specific sites remain identified in the source assertion ledger.
The completed repair does not claim all 142 ports or resolved CPU performance. Forty-two checks remain, and cache eviction is the next measured performance lead.

The cumulative 100-rule run is cold-only. Full-repository unchanged/edit phases were not run.
Its empty incremental-difference array is not an incremental parity claim.
The permanent 57-revision lifecycle matrix separately validates edits, restoration, removal, readdition, and dependencies for the latest ten rules.

## 2026-11-06: Batch-ten completion re-audit

The user rejected the completion claim. Fresh source-backed tests exposed configured HNSW discovery errors.
The original plugin ignores a regular `vector_types.py` but scans a directory with that name.
The previous scope arm inverted that behavior. The standalone scanner legitimately accepts the regular file.

The red commit is `8712eaa8a`: 14 passing and four failing API comparisons.
The repair commit is `7e2629be2`: all 18 pass.
The expanded fresh audit contains 324 inputs, including 251 positives, across all ten rules.
It compares configured reporters through the public audit API and retains standalone HNSW controls.

Only the changed HNSW kernel was rerun over the historical full repository domain: 9,028 inputs, zero mismatches or engine failures.
The final rerun took 72.570 seconds of cold wall time and 4.896 milliseconds of compilation.
Peak parent RSS was 2,333,802,496 bytes.
This was a correctness rerun beside other tests. Parent phase CPU excludes producer processes.
It is not a controlled performance comparison, a process-family CPU measurement, or a new joint 100-rule benchmark.
The historical 100-rule timing remains unchanged. The current evidence must distinguish its 99 unchanged policies from the revalidated HNSW policy.

The first targeted recording inherited the old joint run's oracle timing metadata.
That recording was rejected as a standalone measurement. The final recording explicitly sets the unmeasured oracle timing to null.
Its files and original HNSW findings remain identical to the historical full-domain recording.

Both original-suite API runs pass all 1,098 cases, with 975 complete detector observations and 870 positive observations per API.
The 60 broader native groups pass all 4,317 assertions. The fresh audit passes 660 tests; evidence validators pass 49 tests.
The detector-removal control run fails as required and proves rejection for all ten disabled detectors.
The regenerated v8 receipt and its 14 metadata/control tests pass. The transfer-evidence guard also passes.
The structure baseline remains unchanged at three existing findings. No new Linear issue was created.

## 2026-11-06: Independent tranche-ten test-suite completeness check

The inventory scans all 4,466 pinned Python test files and collects whole relevant modules without port adapters or test filters.
The first pass found 279 direct/shared original cases. Literal plugin-name references then exposed 175 registry cases that it had missed.
The final inventory records 461 related cases across 14 files: 279 direct/shared, 175 registry, and seven workflow cases.
The original registry/workflow assertions pass unchanged. They remain distinct from native detector coverage.
Three explicit CI-blocker registry cells now have real shipped-rule CLI counterparts, with positive and clean command checks.

Eight bounded original-only replay commands pass all 279 direct/shared cases through both native APIs.
Each API matches all 156 complete detector observations, including 135 positives, with no original-detector fallback.
The 67 helper and 57 host cases retain their separate execution contracts.
There is no optimizer-v2 linter test file at this source pin; its original-evaluated supplementary observations remain labelled separately.
Raw reports and exact assertion-phase evidence are under `.rerail/data-layer-batch-ten/suite-completeness-20261106/`.
No cold batch benchmark or engine build was performed. The historical repository measurements are unchanged.


## 2026-11-06: Public `.rr` migration, in progress

The user confirmed all 100 existing ports. The remaining 42 checks will use `.rr` when implemented.
The production addon and historical JSON acceptance receipts remain unchanged.
The source stays pinned to `2b1504adc4291daaf080b2e932962b134da1a69a`.

The inventory has 53 drafts and 47 absent translations. None has full migration acceptance.
The syntax snapshot is `fa50cd94c`; the integrated final-shape snapshot is `5332989f8`.
Imported bytes retain their commit and local-repair provenance.
Eight rules were authored here: removed compatibility surfaces, lifecycle surfaces, source inspection,
ServerPlan name inference, PermissionError, async capability checks, grammar bypass, and admin async purity.

The engine integration branch is `jeremy/data-layer-rr-engine-20261106`, pushed through `8104bdd7fc9a7e0899ae4f0677d04ad11a50a579`.
The frozen tested CLI has SHA-256 `93383c986bf6b4f9c777772c3a04898c3683ed257d0dd1aae82f48732d91f833`.
It includes the public reader, quality runtime, parser and typing merges, and the repairs below.

Port-driven TDD ladders:

- Full diagnostic DTOs preserve messages, tags, parameters, and distinct witnesses.
- A source-span/document-column regression prevents source spans from being joined as AST nodes.
- `d59446237` -> `5b6ae78ac`: demanded Python logical lines preserve all separators, CRLF, empty lines, and long lines.
- `55b132e34` -> `71e4d6a66`: empty input retains statistics and executes global absence findings.
- `f1423713c` -> `008886b11`: borrowed scratch arenas flush through their owner, not scratch `atom_segment` tables.
- `7b7d262bb` -> `064f6b6cc`: explicit input batches retain hidden paths and nonstandard extensions.
- `97d050875` -> `8104bdd7f`: logical-line predicates and safe aliases use vocabulary batches.
  The regression first reports zero batched predicates instead of two. All 11 string-engine tests pass after the repair.
  Every writer must copy a prepared symbol. Computed and constant-producing arms prevent unsafe rewrites.

Original source assertions:

- Eight unchanged direct modules collect 531 cases. Their original Python run passes 528 and skips three async cases.
- The native run executes all 531 original bodies without skips or original-detector fallbacks, across eight bounded shards.
- It makes 477 native calls; 171 cases make no native call and are individually listed.
- Calls with rows include helper truth-value probes. They are not a detector-positive case count.
- The source recording has 317 detector observations, including 286 DTO-comparable calls and 209 positive calls.
- The new companion recorder preserves lossless source results for all 317 observations.
  All receive field, type, order, and multiplicity comparisons, including the legacy normalizer's retained observations.
  The old normalized recording does not retain every field, such as source-inspection snippets; it is not sufficient for full DTO parity.
- The native adapter transfers regex operations and the three reviewed skipped assertions, retaining the original assertions.
- A full DTO check exposed nondeterministic generated input: a test iterates a Python set.
  `PYTHONHASHSEED=0` now pins both runs. Two new harness regressions reject different source inputs and scopes.
- Full capture first exposed process-specific temporary paths stored as strings. A failing regression proves the transport mismatch.
  Only the caller-root prefix is normalized; strings remain strings, Paths remain Paths, and the full relative path remains.
- `rr-direct-source-assertions.json.gz` retains the complete lossless source recording and every shard report.
  It binds source files, harness files, rule includes, the engine, and the fixed seed.

These are direct module and snapshot contracts. Configured reporting, persistent workspaces, complete associated suites,
remaining portable helper assertions, and mutation controls remain outstanding. No rule receives full acceptance from these counts.
The initial four-rule, 92-observation receipt is explicitly historical. Its older binary is preserved.

Changed-kernel measurement:

The 2,000-line source-inspection workload compares frozen builds `c2a0b80aaf8b0c63` and `93383c986bf6b4f9`.
Each run starts with a separate empty persistent cache. `/usr/bin/time -lp` reports total process CPU and maximum RSS.
CPU is 1.49 seconds before and 0.99 seconds after. Maximum RSS is 94,879,744 and 104,251,392 bytes.
The complete native diagnostics match. This is one sample per build, not a repository benchmark.
Other assertion processes were active, so wall times are descriptive only.
The fixture `rr-line-vocabulary-profile.json` retains source and rule hashes, phase measurements, and external time output.

Remaining failures and limitations:

- Shared-cache parallel execution still reproduces a foreign-key constraint error.
  One cache per assertion shard isolates rule semantics; it does not fix shared-cache concurrency.
- The reader refuses five policies using `match_all`, one using `dfs_first_memo`, and one using a slice pattern.
- Repository readers, exact Python AST values/order, configured frontend execution, and signatures still need implementation or validation.
- Nine active atom tests and five active file-local tests pass. A focused reader run passes 19 tests, with two existing report entry points ignored.
  That focused run excludes the separate broad source-check case that exceeds the command limit.
- The current reader suite's broad source-check case reaches the 115-second limit. It is not a passing full-suite run.
- Earlier broad native shards also expose the stale admission interpreter hash, widened-walk regression, Vue corpus failure, and timeouts.
- The user authorized deferring only the missing pinned Ruff and SQLFluff archive tests. No data-layer assertion is exempted.
- A build encountered disk exhaustion; failed logs remain. Superseded binaries were compressed without deleting their contents.
- The latest CLI build takes 29.55 seconds. The latest test build takes 54.05 seconds. Build time is separate from execution.

Sixteen Python migration guard tests pass. The authoring guard passes two tests.
The full migration check must fail until all 100 ports have complete evidence.
No repository-wide cold batch was run. Raw reports and failed attempts remain under `.rerail/rr-migration-20261106/`.

The structure check passes 24 metrics. Its only failing metric retains the same three pre-existing flat-directory counts.
The newly introduced duplicate fixture resolver was removed; the harness uses the existing shared resolver.
Typecheck, boundary checks, and Windows path checks pass.

## 2026-11-06: Native `.rr` rules and complete direct-suite comparisons

The user confirmed native execution as a requirement. Rules cannot call Python.
Python remains available for offline source recordings and original assertion bodies.
Existing signed data remains unchanged. Native subtree serialization still requires work for the stored CPython hashes.

The migration now contains 57 drafts and 43 absent translations from the 100 existing ports.
No policy has full migration acceptance. The production addon remains the earlier executable baseline.
The new drafts cover async executor, Rust SQL grammar boundary, dynamic module loading, and ClassVar checks.

The integration engine is `d85ccee0064fbeacee2a00ecc5120ea7471ba197`.
The frozen binary has SHA-256 `480a4fd20114631dc32f0b0eee2550fbbb87524fa24dec62648f9b2dbd471ade`.
Both identifiers refer to the isolated engine branch. The primary production addon remains unchanged.

Direct source evidence:

- Eleven complete original modules pass all 771 test bodies through native snapshot execution.
- The run makes 714 native calls and compares all 430 recorded API returns.
- Every shard reports zero skipped native cases, zero source detector fallbacks, and zero missing recorded calls.
- The source recording has 768 passes and three skips. The native run executes the three reviewed assertion transfers.
- Eleven disabled-policy controls reject original positive cases through the complete source API comparison.
- The lifecycle control stops before two later calls in the same test. Its mutation receipt retains those unreached calls.
- Passing runs require every recorded call. Mutation failures do not relax that requirement.

`rr-eleven-direct-contracts.json.gz` retains the complete source recording, sixteen native shards, and all eleven mutation controls.
The receipt binds the engine, policies, libraries, source files, and harness files.
Its acceptance field is explicitly false. These results do not cover configured reporting or persistent workspaces.

The receipt lists 242 cases without native calls. All 242 cases run and pass as Python assertions.
They do not count as native rule coverage. Portable counterparts remain required for assertions that describe rule behavior.
Forty-three observations lack legacy normalized DTOs. Their complete API values still receive comparisons.

The independent inventory scans 4,467 original test files and identifies 125 related modules.
It retains 5,754 detector-related cases, 33 integration cases, and 52 workflow cases.
Its collection also retains assertion sites and definition hashes.
Seven inventory guard tests reject missing shards, duplicate shards, mixed collectors, omitted modules, and false module ownership.
Five receipt guard tests reject missing source returns, substituted expected values, skipped assertions, and incomplete independent coverage.

Source-derived evidence remains separate from original pytest coverage.
The ClassVar port matches seventeen additional cases, including duplicate chained findings, comments, Unicode columns, helper exemptions, and source-argument gates.
The Rust boundary port matches thirty-one API observations from seven existing probes and seven fresh cases.
Its native policy selects the Rust scope from all supplied documents. It does not reuse the oracle's selected file list.
The pinned source has no dedicated pytest suite for that rule.

Native repairs from actual failures:

1. Async executor requires LF-only splitting. The native `lf_line` view preserves other separators and terminal empty lines.
2. Repository readers now emit typed rows from JSON, YAML, JSON Lines, tab-separated files, and text lines.
   They cache file contents and decoded documents across selectors before source-file partitioning.
   Malformed inputs emit no partial rows. Revision and stamped-artifact inputs still fail explicitly.
3. Repository-only rules now run once in the global phase. They also run with no source documents.
4. Native Ed25519 checks preserve the existing public signature bytes, including the source helper's base64 transport rules.
5. The shared signing regression initially reported an empty list as both valid and invalid.
   SQL generation replaced an already-bound aggregate result and lost its equality test.
   The repair retains the equality. The public signing-library regression now passes.
6. An external `sample` report exposed a symbol-lock deadlock during computed SQL view loading.
   The loader held the symbol write lock while `sym_text` requested the same lock for reading.
   A scratch cursor now shares the atom arena without holding the registered cursor's lock across SQL execution.
   The corrected regression changes from a five-second timeout to a pass in 0.03 seconds.
7. Noqa ratchet and test budget both name different inputs `base_sig`.
   Combined programs now namespace those readers before they share decoded files.
   A regression proves that each policy receives only its own baseline signature.

The focused native runs pass six reader tests, seven set-engine tests, twelve string tests, and five active file-local tests.
The complete native suite still lacks a passing receipt.
The user deferred only the unavailable Ruff and SQLFluff fixture archives.
Shared-cache concurrency and the earlier unrelated native failures remain open.

The source guard previously treated every dataclass method as passive storage.
New regressions prove that this rule wrongly exempted collector methods and custom DTO methods.
The guard now permits generated storage methods and their standard `repr` wrappers. It covers future threads too.
The stricter run exposed a lifecycle formatter call through the legacy normalizer.
The adapter now obtains the message from native diagnostics. It does not call that source formatter.

Failed attempts remain recorded. One loader regression initially omitted symbol-function registration and failed before the deadlock.
The corrected regression reproduces the timeout. Its passing run also exercises input loading after a probe replacement.
An earlier replay selected an obsolete source recording with process-specific temporary paths.
The current receipt uses the corrected complete recording and the original caller-root normalization.

The latest sibling survey found final-shape `4358d6e55`, quality `cb569f948`, and unchanged syntax `fa50cd94c`.
Final-shape includes a copy of this branch's signature implementation and a new structural `tree_digest`.
That digest uses a different hash format. It does not produce the existing CPython dump hashes.
Those later sibling changes are not yet part of this frozen engine.

No whole-repository cold run occurred. Runtime commands remain bounded at 115 seconds.
The existing exception permits native builds to finish. Build time remains separate from execution time.
No new Linear tickets were created.

## 2026-11-07: Native AST order and two further `.rr` drafts

This checkpoint advances the migration to 59 drafts and 41 absent translations. Full migration acceptance remains zero.
The primary production addon remains unchanged. No rule invokes Python.

The engine is `93cee2a43e10020ca7639cf2ade1cb974c789c18`, pushed on the isolated engine branch.
The frozen executable SHA-256 is `c60f5696278accdeb3a885db6bc64d92213d7e38a16190354675c651f8f657fd`.
Its CLI build took 18.07 seconds. Build time is separate from rule execution.

The direct acceptance run covers thirteen complete original modules and all 969 test bodies.
It makes 922 native calls and compares 486 complete source API returns.
All 24 shards pass without native skips, missing recorded calls, or source detector fallbacks.
Thirteen disabled-policy controls reject original positive cases through the full API comparison.
The source recording has 966 passes and three reviewed skips. Native execution transfers those three assertions.
The receipt retains 273 cases without native calls. They ran as assertions and do not count as native detector coverage.
Forty-eight legacy normalized DTOs remain unavailable; their complete API returns are compared.

`rr-thirteen-direct-contracts.json.gz` retains this evidence with `fullMigrationAcceptance: false`.
The supplementary bundle retains 47 ForwardRef, 37 type-safety, 17 ClassVar, and 31 Rust contracts, separate from original pytest coverage.
They also pass through the same native adapters, with no detector fallbacks.

The ForwardRef port exposed incorrect traversal order. The engine compared preorder node ids, which represent source order.
CPython visits a function's body before its decorators and return annotation.
It also visits dictionary keys before values and positional call arguments before keyword arguments.
These differences caused missed positives in the fresh source-derived corpus.

A native projection now computes one dense rank column per demanded document. All rules share it.
The iterative pass preserves source ids and geometry. It stores four bytes per projected node when demanded.
Temporary overrides cover only reordered fields. No rank column is built when no consumer reads it.
`earlier` now joins this column rather than comparing source ids.
The regression covers parameter annotations, keyword defaults, positional defaults, bodies, decorators, returns, dictionaries, and call arguments.
The focused reader suite passes 23 tests. The tape-view suite passes nine tests.

The type-safety port preserves textual signature windows, early terminators, and matches in invalid Python.
The original suite contains 193 cases. Its regex, discovery, deduplication, scanner, and CLI assertions run through native decisions.
Enum and dataclass definition assertions still run in Python; the receipt identifies cases without native calls.
One original assertion checks `re.Match`. Its operation now targets the native match DTO, with the predicate retained.
The generator preserves CPython Unicode word classes. Boolean branch boundaries use regular expressions without lookbehind.

The regex experiment first exposed an incorrect named-capture diagnostic for lookbehind.
An execution test then showed that the text dialect refuses lookbehind and can silently treat an uncompiled scalar matcher as false.
The final regression requires literal invalid or unsupported regexes to fail compilation.
The port uses supported regular expressions. It does not add an optional regex backend.

Two fixture mistakes were corrected before accepting their regressions.
The first traversal test used the SQL keyword `Order` as a relation name and failed before testing traversal.
After renaming the relation, the corrected test failed on the missing body-before-decorator result, then passed after the repair.
An earlier regex test expected execution of unsupported lookbehind. The final test checks explicit refusal instead.

Fresh contracts also exposed a host-adapter mismatch: the content API ignores a missing file, whereas the file-reading API raises.
The bridge now preserves that distinction. Binary source cases retain every exception argument byte.
The fresh runner records failed case identities and resolves report paths before changing directories.
Its reports now bind imported adapter code as well as the primary harness.

The migration guards pass 34 tests. The two format tests and TypeScript checking pass.
Boundary and Windows path checks pass. Structure checking retains the same three flat-directory violations.
A duplicate fixture-root resolver was replaced with the existing shared resolver.
No whole-repository cold benchmark was run. All runtime commands remain bounded at 115 seconds.
Phase reports are thread CPU observations, not a new process-total or repository performance baseline.

Configured reporting remains unfinished. The type-safety draft formats a category name where its source reporter uses a label.
That difference requires a reporting regression and correction before acceptance.
Associated suites, portable helper counterparts, persistent workspaces, and the other translations remain outstanding.

The sibling survey now includes `266c56c64`, `f412b667a`, `8cbc858b6`, and `c6fc887f4`.
Their document lookup, ordered DFS, capture, and error-reporting changes are not part of this frozen result.
The capture implementation contains per-match inserts that must become bulk operations before adoption here.
Withholding all source facts on parse failure would also break the proven raw-text scanner contract.
No new Linear tickets were created.


## 2026-11-07: Type-safety reporter parity ladder

The reporter comparison found a gap that the direct DTO interface could not expose.
The native policy formatted enum member names; the upstream reporter formats human-readable labels.
A source-derived reporter contract now compares every `StructuredViolation` field, including severity and the complete message.
Native mode metadata comes from the compiled rule. The adapter rejects modes it cannot map.

The ladder retained three distinct failures before correction:

1. The first source-positive case failed on its category label.
2. After correcting labels, invalid Python still produced a native finding where the plugin reporter returned none.
3. After separating raw text and framework behavior, a test-directory file still produced a finding.

All 26 reporter cases now pass. They include all nine categories, clean input, invalid Python, allowlists, and production discovery.
Unknown layer packages and Unicode decimal digits follow the source's dynamic directory predicate.
The engine receives every supplied document; it does not borrow the oracle's selected file list.
A disabled policy with the same id and blocking mode is rejected by the original positive reporter value.
The native reporting process imports no original detector modules.

The updated policy also passes all 193 original direct cases, making 203 native calls and comparing 51 complete API returns.
Its original-positive mutation control still fails when detection is disabled.
All 37 supplementary direct API contracts pass, including raw-text positives on invalid Python.
The older thirteen-module receipt remains tied to the earlier policy and engine hashes.

The engine commit is `aa7ce065a5e9a9bee148a7905dde6be95628304b`.
The frozen executable SHA-256 is `dce138fed59e4f7c565b8e1d4c1f81da5516f2746c5680516992f3cf0ce67d70`.
Its CLI build took 12.87 seconds. Twenty-four focused reader tests pass.
The metadata test initially used the SQL keyword `Nothing` as a relation name and failed before reaching its assertion.
After renaming the fixture, the regression reproduced missing metadata.
Named `.policy` headers are settings in the parser; the metadata export now decodes them with that same native lexer.

The reporter receipts keep full migration acceptance false. Production frontend and persistent-workspace acceptance remain outstanding.
No repository performance comparison or whole-repository cold execution was performed. No Linear tickets were created.


## 2026-11-07: Native capture integration and exact alias migration

Rule execution remains native. Python runs only in the offline oracle and test transport.
The user explicitly rejected Python execution inside rules. Existing signed hashes remain unchanged.

The integration imported bound-document lookups (`266c56c64`), ordered DFS (`f412b667a`), and repeated captures (`8cbc858b6`).
The merge retained the complete repository readers, LF-only lines, Python visit ranks, and aggregate equality checks.
It excluded the upstream Merkle digest. That digest cannot reproduce the existing CPython `ast.dump` hashes.

The imported capture executor submitted one SQL insert per match.
A regression reproduced 1,021 SQLite executions for 1,000 symbol captures.
Bulk insertion and direct batch interning now pass the same test below 100 executions.
This is a statement-count regression, not a repository CPU benchmark.
The test initially failed to compile because its statistics constructor was wrong; the corrected test reproduced the execution count.

The frozen capture engine is `e0d307fcba9991457b962c2ec52998749beec482`.
Its executable hash starts with `0d78f8cbe4b287bc`. Its CLI build took 32.97 seconds.
Twenty-nine focused reader tests passed.
A broader reader invocation reached the external `check_rr` test and exceeded 115 seconds.
That invocation did not pass. Focused execution checks exclude that broad command and fixture-writing tests.

ForwardRef reporter cases exposed incorrect production roots and output ordering.
The native policy now selects `api`, `impl`, and `public` for configured reporting.
The reporter adapter preserves source order and sorts native output by each interface's declared key.
At the capture checkpoint, all 19 reporter cases, 47 supplementary API cases, and five original cases passed.

The new facade alias draft passed all 17 original tests. Supplemental cases then exposed four defects:
parenthesized targets, non-Python file selection, missing grammar validation, and literal source-trigger gates.
Each failure is retained separately. Ordinary and duplicate aliases retain distinct witness identities.
The helper adapters transfer typed caller DTO fields to native predicates. They do not run source detector methods.

Python grammar validation uses pinned Ruff Rust crates with Python 3.13 as the target version.
The tape caches one demanded verdict across projections. Raw-text consumers do not request it.
A regression confirms that invalid syntax still leaves raw source available.
Thirty focused reader tests, eleven tape-view tests, the cache test, and the bulk-capture test pass.
The first dependency build took 99 seconds; the following unit build took 19.90 seconds.
The CLI build took 35.87 seconds. These compile times are separate from execution measurements.
The engine commit is `1d43e471380442dcf95052f2ec0b539d47142b59`.
Its executable hash is `f4b9ad82af5213b35dd6d4303263549708dc21c08686ece017f52ac6cd4e3398`.

The test guard also missed nested callbacks and decorated detector bodies.
Two failing tests established the gaps. Recursive code registration and unwrapping fix them.
The migration tooling now passes 36 tests and 21 subtests.

The rule inventory is 60 drafts and 40 absent translations. Full migration acceptance remains false.
The automatic converter refused every remaining policy; none was installed as a partial conversion.
The changed-engine checkpoint now passes all 215 original cases for ForwardRef, type safety, and facade aliases.
It makes 224 native calls and compares 63 complete API values.
The 35 cases without native calls are individually retained.
All three original-positive mutation controls reject disabled detection.
All 116 supplemental API cases and 61 configured reporter cases pass on the same frozen executable.
Each reporter also rejects its own disabled rule.
These are snapshot contracts. Associated suites, production integration, and persistent-workspace acceptance remain outstanding.
The cache test is included in the eleven tape-view tests.

An offline AST probe corrected an earlier assumption about synthetic tuples.
Python 3.13.18 supplies a Load context and omits empty list fields in ast.dump by default.
Native subtree serialization must use captured interpreter behavior; it is still unimplemented.
The source-trigger regression also shows why AST-only selection can change behavior:
`import foo\tas\tbar` produces no source finding until another token satisfies the literal source gate.
No repository performance claim or whole-repository cold run was made. No Linear tickets were created.

The two format tests, TypeScript typecheck, boundary check, and Windows-path check pass.
The structure check remains at 24 passing checks and one failure, with the same three pre-existing directory-size violations.


## 2026-11-07: native HNSW assertions and string semantics

The user requires native rule execution. No Python AST producer was enabled.
The 215-case three-rule checkpoint above remains historical and tied to its recorded engine and program hashes.

HNSW now passes all 13 original assertions and 13 complete API-return comparisons.
Its 37 supplementary API cases cover positive findings, clean inputs, source gates, path scope, Unicode, and malformed Python.
All 23 configured snapshot reporter cases pass. These are additional contracts, not original pytest cases.
The original positive mutation control rejects empty native results.

The TDD ladder found these successive defects: normalized identifiers, collapsed repeated constants, wrong fragment starts,
BOM acceptance, and surrogate escape boundaries. The new native word projection fixes the last defect without lossy string substitution.
The projection pins CPython 3.13's Unicode 15.1 word class. Its generator runs offline.
A later ladder exposed missing nested format literals, comment text misread as literal text, and broken CRLF continuations.
The production reader now follows parser-defined interpolation and format-expression children on the existing tape.
It does not parse each literal again. The former handwritten expression scanner was removed, including its test-only copy.
Word-only consumers share their decode pass with positions and do not request full decoded-value rows.
The projection still prepares requested columns for the batch; node-demand pushdown remains separate work.

Four source reporter repetitions produced three file orders because FactEngine consumes as_completed results.
Reporter parity permits only inter-file reordering. It retains every duplicate and each file's original ordered values.
A failing harness regression proved that within-file reordering and duplicate removal remain detectable.

The focused checks pass 34 reader tests, 12 tape-view tests, four decoder tests, and seven Python-value tests.
One reader test remains ignored upstream. The broad external syntax checker remains excluded after its earlier timeout.
The tooling passes 37 Python tests. Build time is recorded separately: the word, CST, and final CLI builds took
15.84, 18.63, and 9.62 seconds respectively. No full-repository cold run or performance claim was made.
The final native checkpoint is 96329a41e40555871833f81280b60bd27e57965a.
The rule inventory remains 60 drafts and 40 absent translations. Full migration acceptance remains false.
Native canonical AST serialization, associated suites, and production workspace acceptance remain required.
No Linear tickets were created.

The final engine reran the 215 original cases for ForwardRef, type safety, and facade aliases in eight bounded shards.
All pass, including 224 native calls and 63 complete API returns. The 35 cases without native calls remain individually listed.
No shard exceeded 39 seconds. Their previous supplemental and reporter receipts remain historical.
The HNSW reporter also rejects a policy with all ten violation clauses disabled.
The two format tests pass. Full migration acceptance remains false.


## 2026-11-07: removed-surface API and reporter parity

The unchanged original module passes all four tests through native execution, with four complete API-return comparisons.
A 54-case audit first exposed six defects in visit order and identifier normalization.
The expanded 73-case corpus passes. It checks every forbidden name and additional AST name positions.
These cases include positive findings and clean declaration contexts. They are supplemental, not original pytest cases.

The configured reporter preserves AST occurrences, while scan_file deduplicates by line and name.
The former draft reported two findings where the source reporter reported three.
The repaired policy has separate outputs for these two contracts. All 15 reporter cases pass.
The generator retains normalization, first-visit ordering, source gates, and configured scope.
An unchanged original positive test rejects the disabled policy.

The supplemental harness initially confused a path keyword dictionary with a path marker.
A failing unit established that transport error. The fix preserves both dictionary and path types.
The format suite then exposed an unnecessary pytest import in that pure helper.
Runner imports now load only during native assertion execution. All 38 tooling tests and both format tests pass.
No engine build or repository benchmark was needed for this rule correction.
The earlier checkpoint receipts retain their exact harness hashes and remain historical.
The inventory remains 60 drafts and 40 absent translations. Full migration acceptance remains false.
The configured reporter also rejects its disabled policy. Its control retains the complete native report.


## 2026-11-07: Final-shape merge and the 61st draft

The engine now merges Final rerail shape through af51b565dcda2ef9de05ecda996b46e7c42677cf.
The merge retains the local typed readers and bulk capture inserts instead of restoring older implementations.
It also retains demanded, cached grammar verdicts and raw-text views on invalid Python.
Upstream chunk-mix statistics and batch release are integrated. The guest-document proposal remains documentation only.
A new regression found that error directives wrote the old violation index but missed the public diagnostic transport.
Native lowering now publishes syntax and input errors through the same lossless report format.
All 40 focused reader tests pass; one upstream test remains ignored, and the broad external syntax checker remains excluded.
All 12 tape-view tests pass. The merge and repair unit builds took 56.41 and 44.60 seconds.
The CLI build took 46.48 seconds. These compile times are separate from execution.

The sync/async-signature rule is newly translated, rather than repaired from an existing draft.
Its original positive test first failed against an empty native policy.
The final Datalog predicate preserves dictionary overwrites, insertion order, positional-parameter selection, annotation preference, and async flags.
Five original tests and five complete API returns pass. All 52 supplemental API cases and 21 reporter cases pass.
Both positive mutation controls reject disabled detection.
The rule needs no Python producer or new rule-specific native detector.
The inventory is 61 drafts and 39 absent translations. Full migration acceptance remains false.
No repository cold run was performed. No Linear tickets were created.


## 2026-11-07: the 62nd draft and portable facade assertions

Facade purity is newly translated to native Datalog. Its syntax whitelist preserves the original AST distinctions.
The seven original tests pass with eight native calls and six complete API returns.
Every original case has native calls, including the CLI and the default-target constant assertion.
The constant assertion reads the same native table that selects configured targets.
Attempted-file statistics come from a native aggregate. The original directory case rejects a native count of 99 against two files, with findings unchanged.
The 54 supplemental API cases and 16 reporter cases pass. Disabled detection fails both positive controls.

Bare linter imports now bind to the same guarded module as package imports.
Two alias regressions failed first and now pass. The source tests remain unchanged.
The generator manifest now lists both new rule files. The format tests pass.
The inventory is 62 drafts and 38 absent translations; full acceptance remains false.
The generator changes require no engine build. No repository-wide cold run was performed, and no Linear tickets were created.


## 2026-11-07: the 63rd draft, backend coupling

Backend coupling now has a native rule for both the checker and framework collector contracts.
All 115 original tests pass across four bounded shards, with 153 native calls and 62 complete returned-value comparisons.
The framework and CLI consume one native demand batch, including native file selection.
Twenty-four constant assertions consume native configuration snapshots; eight assertions concern host identity or the repository environment.
The receipt identifies each case without an additional native call. No original cases are skipped or deselected.
The 45 supplemental API cases and 33 reporter cases pass. Both positive controls reject disabled detection.
Added cases first exposed elif ownership, Unicode TYPE_CHECKING normalization, and the distinct FactEngine directory exclusions.

The rule also exposed invalid SQL for nullary relations. The native fix represents their empty tuple with an integer unit.
Constant-only views keep DISTINCT, so multiple witnesses still produce one Boolean row and an aggregate count of one.
The original failing SQL and passing regression are retained. All 41 focused reader tests and two SQL-block tests pass.
The replacement compiler required a dependency rebuild. The completed unit and CLI passes took 240 and 79 seconds, respectively.
These are compile times, not execution measurements. Earlier bounded dependency attempts are not performance evidence.
All 42 tooling tests and both format tests pass. Structure retains the same three pre-existing flat-directory violations.
The engine checkpoint is fed982e1393ecc815735f766941c0b3785512589.
The inventory is 63 drafts and 37 absent translations. No rule has full migration acceptance yet.
No repository cold run was performed, and no Linear tickets were created.


## Final-shape follow-up merge, 7 November

The integration engine now merges Final rerail shape through f2a67f63e6c22ec8351dd69150939fbaf87cefd0.
The integration checkpoint is f0f13c481c73a8665531993cb16e13122fa3575a.
Upstream adds the prior workstream's documentation, fixtures, scripts, TypeScript tests, and a revised guest-document design.
Native Rust source and the rule libraries are unchanged by this merge.
The SQLite build disables its global memory-management LRU, and Cargo adds the upstream laboratory profiles.
The rebuilt CLI took 41.69 seconds. Compile time is separate from execution.
All 115 backend-coupling assertions pass again, with 153 native calls and 62 complete returned-value comparisons.
All 45 API cases and 33 reporter cases pass again. Both positive controls still reject disabled detection.
The receipts retain the exact rebuilt executable hash and source commit.
The inventory remains 63 drafts and 37 absent translations. This focused merge check does not certify the entire integrated test suite.


## 2026-11-07: the 64th draft and a shared text-view defect

The connection-cache port passes all 151 original cases, with 134 native calls and 55 complete returned-value comparisons.
Its 47 regex assertion functions now use native Boolean probes. Thirty-three configuration assertions read native setup snapshots.
Five host identity or CLI-help assertions remain individually identified. No original test is skipped or deselected.
The 34 supplemental API cases and 34 reporter cases pass, including positives. Both detector mutation controls reject disabled detection.
Earlier failures exposed symbol-valued option IDs and a configured long-source false negative; both are retained.
Configuration and detection now read shared tables, rather than duplicate constants.

A native regression also exposed variable regexes becoming an undeclared rx relation during internal serialization.
The fix preserves search functors and whole-string public matching. All 42 focused reader tests pass, with one existing ignored test.
The first test build read the file before the appended test; its zero-test result was rejected and is retained explicitly.
The actual old-CLI failing execution and rebuilt single-test pass are separate evidence.
The unit and CLI builds took 35.99 and 23.48 seconds. These are compile times.
All 43 tooling tests and both format tests pass. Structure retains the three pre-existing flat-directory findings.

A separate native control proves missing source and physical-line text rows for a 5,013-character line.
The audit identifies 24 other draft consumers for follow-up; it does not claim they were all executed.
The next shared repair must keep requested source text exact while avoiding whole-tree text copies.
The inventory is 64 drafts and 36 absent translations. Full migration acceptance remains false.
No repository cold run was performed, and no Linear tickets were created.

## 2026-11-07: demanded text and positional-product repair

Complete requested source and physical-line text now survives the 256-byte AST-text limit.
Plan-time span provenance avoids copying unused container text. Six storage-demand cases pass.
All 43 focused reader tests and 13 syntax-view tests pass; one reader test remains ignored.

Nine rule families pass all 36 source-positive padded reporter cases.
ForwardRef initially timed out at 100 seconds on 1,024 comment lines.
An external sample showed nested native joins. The operator profile exposed ordered node triples created before candidate filtering.
With 32 comment lines, two extracted operators each performed 147,741 steps.
The splitter now keeps independent node groups fused with semantic filters. The original policy remains unchanged.
An attempted scope narrowing did not fix the timeout and was reverted.

The small debug profile changes from 0.939 to 0.571 summed phase CPU seconds and 77.9 to 70.0 MB peak RSS.
The before profile used the ineffective narrowing; the after profile uses the unchanged policy. This is not an isolated same-policy benchmark.
The 1,024-comment positive now completes at 0.65 summed phase CPU seconds and 78.2 MB peak RSS.
All five unchanged ForwardRef tests and five focused locality tests pass. One locality profiling test remains ignored.
Cross-file base retention still needs memory assessment; no repository-scale speed or memory claim follows from these measurements.
The native unit and CLI builds took 48.94 and 17.01 seconds. Compile time is separate from execution.
Evidence is retained in rr-long-source-native-repair.json.gz, including the external sample and native operator profiles.
The inventory remains 64 drafts and 36 absent translations. Full migration acceptance remains false.

## 2026-11-07: current guest and typing merge

The integration engine merges Final rerail shape through 34bfb09451f6849be5f0b2e5c447d159a89bc46a.
Checkpoint e51dddeea8b0ffbdc03d62afc7d143a8cef8019b preserves explicit input batches and typed repository readers.
Two existing tests caught raw source disappearing on parse failure. Both pass after the merge repair.
Broken guests retain no tree by default; primary files retain raw text.

All 64 draft policies pass the reader. This is not execution acceptance for all 64.
All 36 padded reporter comparisons and five original ForwardRef tests pass again.
Focused suites pass 47 reader tests, 14 syntax-view tests, five locality tests, and 43 tooling tests.
One reader and one locality profiling test remain ignored. Broad syntax-checker, fixture writer, and GPU suites were not run.
The final unit and CLI builds took 121 and 62 seconds. Compile time is separate from execution.
The inventory remains 64 drafts and 36 absent translations. Full migration acceptance remains false.
Evidence is in rr-final-shape-guest-typing-merge.json.gz. No repository cold run or Linear issue was added.

## 2026-11-07: the 65th draft and batched regex occurrences

The API-root-import port passes all four original tests, with four native calls and four complete API comparisons.
No original case is skipped or host-only. The supplemental corpus passes 50 API cases and 49 reporter cases.
Those corpora contain 35 and 34 positive cases, respectively. Import and example positive controls both reject disabled detection.
Earlier executions exposed a gate without a document anchor, consumed-newline locations, and nested constant order.
The repaired order query uses only matched literals and their AST descendants.

The shared symbol regex producer now returns occurrence IDs and Unicode scalar offsets in one demand batch.
It preserves repeated equal captures. Coalescing and namespaces now update native input references and retain their dependencies.
The actual missing-context failure is retained. Rejected round-trip fixture attempts are explicitly identified as failures.
All 49 focused reader tests and nine simplifier tests pass; one reader test remains ignored.
The checkpoint is b95deaf706d7c991e8294074c06139bd2036b270, with Final rerail shape through 34bfb0945.
The final unit and CLI builds took 45.15 and 38.17 seconds. These are compile times.

All 65 draft policies pass the reader. All 43 tooling tests and four format tests pass.
The new generated-file registration failure was repaired. Structure retains the same three pre-existing directory-size findings.
Regex occurrence producers remain cross-file operators. Repository-scale CPU and RSS are not certified by these focused tests.
The inventory is 65 drafts and 35 absent translations. Full migration acceptance remains false.
Evidence is in rr-no-api-root-imports-native-direct-contracts.json.gz, rr-no-api-root-imports-native-supplemental-contracts.json.gz, and rr-regex-occurrence-native-tdd.json.gz.
No repository cold run or Linear issue was added.


## 2026-11-07 — Retire the migration engine fork

User direction: use Final rerail shape as the active engine and stop extending the nested integration checkout.

- Preserved all current work at `bed5123f9fe9998c7307febb4d94817302168f8b` and backup branch `archive/data-layer-before-modern-engine-bed5123f9`.
- Adopted the complete committed Final shape tree at `879486272cd1b44b3567320c85847ba0fd19a918`, including its conflict resolutions. No old engine files were overlaid.
- Moved the migration policies and assertion evidence into the existing `gate/data-layer/` layout. Retained the lossless Python test transport and source pin.
- Rebound imported library hashes to the authoritative modern library bytes. Historical receipts remain historical.
- Added a regression test proving that identical library bytes from a retired checkout were accepted by the old provenance check. The repaired check rejects that checkout. All 44 Python tooling tests pass.
- Preserved the unfinished 66th translation as a draft; 34 of the 100 translations remain absent. No full migration acceptance is claimed.

The five-argument regex occurrence producer was developed after the engine merge already included by Final shape. It requires a focused forward port onto this baseline, not restoration of old engine files. Root builds and acceptance results will be recorded below.

### Root build and acceptance results

The adoption commit `56fdb0a5a` has Final shape `879486272` as its second parent. Its native code, frontend, build files, and locks have no diff against that parent. The sole subsequent native change is the positional regex forward port, committed after failing regressions (`36a7cef59`, then `cbd504d18`).

Builds used the root checkout, Rust 1.97.1, and default language features. The unchanged headless CLI build took 82 seconds. The final headless CLI build took 37.38 seconds. The standard unit-test builds took 157 seconds before the repair and 69 seconds after it. These are compile wall times, not execution benchmarks. Builds were allowed to finish under the existing authorization.

The final frozen root CLI is `38164d10366011ccfb131813f753d728f4b44e71f47fe066c3b07045ab3b4b06`. Reader receipts identify only libraries under the active root checkout.

| Check | Result |
| --- | --- |
| Draft policy reader | 66 passed; baseline rejected only the missing positional regex API |
| Rule-reader unit suite, excluding the broad inventory check | 49 passed, two upstream ignored writer/report tests |
| Simplifier unit suite | Nine passed |
| Syntax-view unit suite | 14 passed |
| File-local unit suite | Six passed, one upstream ignored benchmark |
| Python tooling | 44 passed |
| Gate format contracts | Two passed |
| API-import original tests | Four passed; four complete API returns; no omitted calls or semantic fallbacks |
| API-import supplemental APIs and reporting | 50 API cases and 49 reporter cases passed |
| API-import positive mutation control | Original positive assertion rejected an empty native detector |
| Long-source reporter regressions | All 36 cases passed on both baseline and final root builds |
| Unfinished error-handling port | All five original tests passed on the final root build; broader acceptance remains outstanding |

The full library inventory unit test exceeded the fixed 115-second execution limit in a combined run and again in isolation. Both attempts are retained as timeouts. This test is not certified. The first unit-test build used `headless`, which removes addon methods called by workspace tests; it failed with 15 compile errors. The standard unit-test build then succeeded. The first API-import replay omitted the fixed hash seed; that run was retained but replaced by a passing replay with `PYTHONHASHSEED=0`.

The first Vitest command also discovered obsolete test copies under `.rerail`. Discovery now excludes that workspace data. The authoring contract resides in Final shape's existing data-layer gate. The two gate tests pass there. No full repository cold execution was run.

Evidence: [root adoption and focused acceptance](../gate/data-layer/fixtures/host/data-layer-port/rr-modern-root-adoption.json.gz) and [original assertions with positive control](../gate/data-layer/fixtures/host/data-layer-port/rr-modern-root-direct-contracts.json.gz). The inventory remains 66 drafts and 34 absent translations. Full migration acceptance remains false. Continue ports against the root engine.


## 2026-11-07 — Error-handling parity repair on Final shape

This delivery uses the root Final shape engine; the sibling remains at `879486272`. No native engine code changed.

TDD commits: `2d8d15478` adds source-derived edge cases and failing replay-partition tests; `4486a5e66` implements bounded native replay; `430773944` repairs the rule and its input transport.

The first corpus had 101 API cases. Its unsharded native command exceeded 115 seconds after recording two mismatches. Three bounded red partitions then completed: 99 passes and two failures. Relative roots were incorrectly suppressed, and invalid UTF-8 was swallowed.

The repaired rule determines file admission and read-error exposure in Datalog. The host supplies only file status, decoded text or the actual read exception, and reconstructs the public result. Read failures obey the source's supplied-text trigger, suffix, and test-path gates. Outside-root read failures happen before the source collector's path rejection and must still propagate. No source detector executes in native acceptance.

| Contract | Result |
| --- | --- |
| Original source suite | Five cases, seven assertion sites, five native calls and complete API returns; no excluded cases, omitted calls, or semantic fallbacks |
| Supplemental public APIs | All 110 pass across three verified partitions: 53 finding-positive, 55 clean, two decoding errors |
| Configured reporter | All 97 pass, including 45 positives |
| Original positive control | Disabled detector fails the original positive assertion |
| Selective reporter controls | Disabling either log-reraise or transaction-commit detection independently fails parity |
| Authoring checks | 46 Python tooling tests, two gate format tests, changed-rule reader all pass |

Cases exercise direct handler adjacency, comments and continuations, `except*`, nested callback ordering, same-line duplicate findings, exact Unicode function names, parenthesized calls, async and class scopes, source gates, directory boundaries, invalid syntax, and long source. Annotation strings create no spurious commit findings.

The broader structure gate is not green: 15 categories fail on Final shape `879486272`, on the pre-port branch `f853b52af`, and here. Snapshot checks use the checker's existing in-memory repository interface over committed Git blobs. They record additional fixture-location findings from the required gate layout and the repaired generator registration. Both the failures and the snapshot comparisons are retained in the receipt.

The relocation had also removed LFS coverage from parity archives. A gate-local `.gitattributes` restores it; 67 compressed archives use LFS, with all 65 existing payloads verified byte-for-byte against the preceding commit. The binary metric returns to its sole upstream finding, `native/src/code_rule_dsl/logic/plan/golden/facts.sql.zst`.

Evidence: [original assertions](../gate/data-layer/fixtures/host/data-layer-port/rr-error-handling-native-direct-contracts.json.gz) and [red/green supplemental contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-error-handling-native-supplemental-contracts.json.gz). No full repository cold execution or new Linear issue was needed. Global associated contracts and persistent-workspace acceptance remain outstanding; full migration acceptance stays false. The next missing translation is `backend_type_strings`.


### Next translation: backend type strings — survey and regex prototype

All 148 original tests pass in a fresh offline recording at the source pin. It captures 62 complete API returns. The other 88 cases have no outer `check_file` recording; they still require their native helper, configuration, CLI, or passive DTO contracts. They are not excluded from the migration. No native rule port is claimed yet.

A direct native regex prototype proves a false negative: Python `\bbackend` matches after a combining mark, while Rust's Unicode word class includes that mark. The converse also matters: Python treats other-number characters such as superscript two as word characters. A lookbehind spelling of Python's exact word class is refused by the current occurrence producer, consistent with Final shape's deliberate regex restriction.

An alternative uses existing Datalog functors. Normalize Python-word/Rust-nonword characters to a non-ASCII word marker, and Rust-word/Python-nonword characters to a nonword, nonspace marker. Both substitutions preserve Unicode scalar length. The eight source patterns use ASCII literals, word/space classes, and a `[^)]` class; both markers preserve those predicates. Explicit Python whitespace ranges retain its `\s` behavior. Native occurrence positions then slice the original line for messages and columns. This avoids consuming boundary delimiters or filtering already-greedy matches.

The prototype compares all eight source patterns over 18 strings, with at least one source-positive match for every pattern. Every occurrence, ordinal, character start/end, and original matched text agrees with the source. It includes combining marks, superscript digits, join controls, adjacent matches, and nested membership text. This is a focused representation proof, not completed rule acceptance. No engine change or new Linear issue was needed.

[Source recording and regex prototype](../gate/data-layer/fixtures/host/data-layer-port/rr-backend-type-strings-restart-survey.json.gz). Next: write the full rule and native API/helper adapters, then execute all 148 original cases and fresh positive/negative contracts. The migration inventory remains 66 drafts and 34 absent translations.


## 2026-11-07 — Backend-type port and Final shape ca76 integration

The previous delivery made concrete progress on error-handling and established the backend regex representation. This delivery adopts Final shape `ca76fe78f74d9e6b74f9d8cf25cd69f526b0adf5`, then implements the backend-type scanner and public helpers in `.rr`.

Merge `00997cf1c` takes the upstream engine and shared libraries exactly. Final shape already merged the occurrence producer. The merge removes the duplicated local test copies and preserves the gate policies and receipts. Uncommitted sibling code remains owned there.

The initial metadata batch failed before pytest assertions: demand planning created a zero-column `pre_*` relation after the initial SQL normalization. The full rule produced an invalid zero-column table; a minimal public rule produced an empty SELECT list. Test commit `d04372704` captures this shape. The test fails on the unchanged modern engine. Repair `25b580262` normalizes new Boolean demand contexts before SQL emission, preserving existing physical columns. The regression covers both true and false results on an empty corpus and now passes. This is the only local native delta, plus its test.

| Contract | Evidence |
| --- | --- |
| Original backend-type suite | All 148 cases and 182 assertion sites pass; no excluded or skipped cases |
| Complete public API returns | 122 match exactly: 62 file APIs, 27 skip predicates, 33 file-discovery returns |
| Shared native constants | Four configuration batches, one per assertion shard; all three constant collections come from the rule |
| Cases without a per-case native call | 27 shared native configuration cases, nine passive DTO cases, six host interface cases; every case and assertion site identified |
| Fresh APIs | 97 pass: 79 detector cases with 59 positives, plus 18 helper cases |
| Configured reporter | 78 pass, including 56 positives |
| Positive mutation controls | Original positive test and all six selectively disabled BT000–BT005 reporter outputs reject the mutation |
| Integration refresh | 67 drafts parse; four API-import and five error-handling original cases pass; all 36 long-source reporter cases pass |
| Authoring checks | 46 Python tooling tests and two gate format tests pass |

The full suite recording previously omitted helper returns. The recorder now captures them, normalizes their paths against the actual test fixture, and preserves complete values. Recorder metadata calls unwrap recording wrappers so they cannot appear as extra public calls. Native acceptance forbids source detector/helper execution. The original CLI argument parsing and result printing run through native detector and file-discovery adapters. No original test body was rewritten for this linter.

The regex translation uses two typed string transformations to preserve Python's Unicode word class. All eight patterns then run through the existing batched occurrence producer. Original text supplies captures, snippets, and columns. Message classification follows the matched text and source branch precedence, including BT000 whitespace cases and cross-pattern precedence. Native helpers preserve the source file finder's inclusion of hidden paths and directories named `*.py`, deduplication, Path ordering, and raw substring skip tests.

The final engine is the root headless CLI with hash `7b9a2694bad1eeff…`. Build wall times, separate from execution: initial CLI 97 seconds; standard unit build 182 seconds; red regression build 59.70 seconds; repaired unit build 54.66 seconds; repaired CLI build 25.41 seconds. Native execution commands remain bounded at 115 seconds. No whole-repository cold benchmark was run.

The broader rule-reader suite has **51 passes, one failure, two upstream ignored report/writer tests**. The failure is the calls.rdl guest-demand roundtrip, with extra request/context declarations. It also fails on the frozen pre-repair binary. The sibling's uncommitted changes track this as O101; they were not imported or used to hide the failure. The full library inventory test was not repeated after its previous two command-limit timeouts. Nine simplifier and six file-local tests pass; one upstream benchmark remains ignored.

Evidence: [original assertions](../gate/data-layer/fixtures/host/data-layer-port/rr-backend-type-strings-native-direct-contracts.json.gz), [supplemental cases and assertion provenance](../gate/data-layer/fixtures/host/data-layer-port/rr-backend-type-strings-native-supplemental-contracts.json.gz), and [modern-engine integration and Boolean TDD](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-ca76-merge.json.gz). Associated global and persistent-workspace contracts remain outstanding. The inventory remains 67 drafts and 33 absent translations; no full migration acceptance is claimed.
### Final shape alignment: one active engine (2026-11-07)

The data-layer migration now builds and tests the engine in this repository root.
Final shape commits `dde9f3dc4` and `ee59790f0` are merged; the latter includes the world-demand and environment-fact model.
Sibling uncommitted files were not copied. The retired nested engine was not built or used.

The native code, frontend, dependencies, and rule libraries were compared with `ee59790f0`.
Only the existing Boolean-context repair and its regression test differ in the engine (`25b580262`).
No separate runtime or rule-specific native implementation was added.
The migration manifest records the merged baseline and current library hashes.

Both acceptance binaries were built from this root and frozen after successful build completion.
The CLI build took 46.73 seconds; the unit-test build took 129 seconds under the existing build-time exception.
The CLI SHA-256 starts with `6987f9cd4c78d649`; the unit binary starts with `80e95e77b87db835`.

| Check | Result |
| --- | --- |
| Migration tooling | 46 passed |
| Gate format | Two passed |
| Current draft reader | All 67 passed |
| Native reader/execution tests, bounded subset | 55 passed; two upstream file-writing tests ignored |
| Newly merged library/stub loading regression | One passed |
| Original backend-type suite | All 148 cases passed across four disjoint shards; 122 complete API comparisons and 126 native calls |
| Original error-handling suite | All five cases and complete API comparisons passed |
| Original API-import suite | All four cases and complete API comparisons passed |
| Configured backend-type reporters | All 78 passed, including 56 positives |

The backend-type shards cover exactly the original source case identities. They include 51 finding-positive file returns.
Four shared native configuration calls account for the difference between 122 API observations and 126 calls.
Two helper observations have no legacy findings-list normalization; their complete source-return values still match exactly.
The existing passive DTO, CLI, and shared-configuration case classifications remain explicit in each report.

The combined native suite hit its 115-second limit with one test outstanding: `rule_syntax_files_pass_check_rr`.
That test validates the entire library corpus. Its completion is not claimed.
The remaining suite was then run with that single test filtered out and finished successfully in 14.93 seconds.
The targeted 67-rule reader check is separate and completed successfully.

[The alignment receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-ee597-merge.json.gz) retains build provenance, source recordings, complete native reports, reader results, and logs.
This work aligns the runtime; it does not certify all migration contracts.
The inventory remains 67 drafts and 33 absent translations, with full acceptance false.
`serverplan_grammar_surface` remains the next missing translation. Its source survey is not a completed port.

### Native grammar-surface draft: exact source assertions and shape repairs (2026-11-07)

The 68th draft translates `serverplan_grammar_surface` into `.rr` on the root Final shape engine.
The upstream branch remains at `ee59790f0`; no sibling changes or retired-engine code were copied.
Detection, scope, triggers, file selection, and helper-name tables execute in native relations.
The Python harness transports I/O, CLI output, and passive DTOs. It guards the linter and its shared fact module against semantic fallback.

The first native original-suite run passed eight cases and failed one complete result comparison.
Ruff reported a malformed parameter differently from CPython. The source assertion checks less text, but the stronger result comparison retained the failure.
An independent corpus then found 20 diagnostic mismatches among 183 cases. The other 163 matched.
The existing parser adapter now formats the 21 recorded error forms, with native regression coverage.
No interpreter is linked or invoked by rules.

Further probes exposed ten failures across 11 cases. They covered future imports, match-pattern attributes, conditional order, and wrapper-sensitive breadth-first order.
The shared `python/ast-shape.rr` projection expands dotted pattern attributes and computes semantic depth over existing child edges.
The child edge stays explicit so the compiler can prove that numeric recursion terminates.
The native tape visitor now visits conditional expressions as test, body, then else, matching Python's AST fields.
The final reporter check also caught an incorrect severity directive. It now uses `.severity Surface error`.

| Contract | Final result |
| --- | --- |
| Original unchanged suite | Nine passed; seven complete API comparisons; 12 native calls; no excluded or skipped cases |
| Supplemental public APIs and discovery | All 197 passed; 174 positive file scans, including 21 syntax errors; one positive discovery result |
| Configured reporter | All 207 passed; 155 finding-positive cases; complete fields, order, and multiplicity compared |
| Original positive control | Disabling detection fails the unchanged original positive assertion |
| Selective controls | Each of the six disabled detection families independently fails complete reporter parity |
| Native reader/execution tests | 56 passed; two upstream file-writing tests ignored; whole-library validation remains outside this bounded subset |
| Native parser regressions | Two passed, including all 21 recorded diagnostic forms |
| Tooling and format | 48 Python tests and two gate tests passed |
| Current draft reader | All 68 passed |

The larger final acceptance shards hit the 115-second cap under concurrency. Those runs do not count as acceptance.
Eight disjoint shards per corpus then covered every case exactly once. Two workers ran at a time.
The recorded remaining shards took 45–54 seconds each. No full repository or whole-batch cold run was performed.

The original-suite receipt is [here](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-grammar-surface-native-direct-contracts.json.gz).
The [supplemental receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-grammar-surface-native-supplemental-contracts.json.gz) retains every final shard, source oracle, selective control, validation script, and earlier red result.
Separate red receipts preserve the [original diagnostic failure](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-grammar-surface-original-red.json.gz) and [shape failures](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-grammar-surface-projection-red.json.gz).

Full migration acceptance is not done. Production and persistent-workspace contracts remain outstanding.
Diagnostic compatibility covers 21 recorded forms; other parser variants still need parity work.
The `backend_coupling` draft also uses raw tape depth. Audit it with wrapper-sensitive ordering cases before acceptance.
The inventory remains 68 drafts and 32 absent translations. No new Linear ticket was created.

The next missing translation is `unresolved_forward_refs`. Its source survey and offline run cover all 116 original cases.
The recording captures 62 outer calls and identifies 54 cases without one. Those helper, configuration, and CLI assertions still require native transfers.
No native port or acceptance is claimed for that next rule.

## 2026-11-07 — Final shape 31d3 alignment and FieldInfo contracts

Merge `4fb37a504` adopts committed Final shape `31d3d15dc2882f3dc81eb79f24729f32d723656b`.
It includes the current planner, native relation reclamation, indexed SQL maintenance, and corpus publication protections.
The active engine is this repository root. Neither retired nested engine received changes or a build.
The sibling has another unfinished merge. Its uncommitted files remain untouched.

The root CLI build passed. Acceptance used frozen executable SHA-256
`f3d121eb3da0a71f2c12cf190a8f6751ee7948deea9dddef2937429d90446ae4`.
The remaining native differences from Final shape are the existing generic Boolean-context and Python AST diagnostic/order repairs.
No new native implementation was added for this rule or alignment.

The 69th draft translates `unresolved_forward_refs`. Its `.rr` predicates implement FieldInfo receiver recognition,
safe conversion and fallback contexts, inspection calls, comparisons, scope, ordering, and configuration.
Private helper assertions execute native queries. Supplied ASTs enter as flat typed columns, including edges and field order.
They use the same decision predicates as parsed source. Rules do not invoke Python.
The CLI keeps the original argument parsing and rendering, with native batches for file selection and detection.

Focused acceptance on the merged engine passed:

- All 116 unchanged cases in the direct source module: 114 native calls and 62 complete API returns.
- The original associated supplied-AST case, including the complete mutated instance state and return value.
- All 206 supplemental cases: 195 file API cases and 11 supplied-AST state cases.
- All 205 configured reporter cases, including 109 cases with findings.
- One disabled-detector control rejected by an original positive assertion, plus three independent reporter-family controls.
- All 52 migration tooling tests, both authoring gate tests, 59 native reader/runtime tests, and four collection-lease tests.

The direct module includes 25 cases without a per-case native call. Nineteen assert configuration decoded from a native query.
Six assert callable/export or argparse interfaces. The supplemental receipt identifies each case and its backing contract.
No helper detector assertion was removed. The associated module's transaction DTO and structural-helper tests belong to their respective ports.
They remain required for the full migration; this rule's receipt does not claim their coverage.

The supplemental corpus caught a real duplicate: parenthesized attributes produced three findings instead of one.
The fix restricts attribute candidates to attribute nodes before receiver analysis. The original red result remains in a separate archive.

The current draft reader sweep also rejected `la_cls` in the shared lazy-analysis library as an unbounded value cycle.
The `.rr` library now states its existing class-parent preorder bound explicitly. A native execution check preserves
`Outer`, `Outer.Inner`, `Outer.Inner.Deep`, and `Next`, while excluding a function-local class.
All 69 drafts then passed the reader. The failed sweep and successful execution are retained in the supplemental receipt.

Eight disjoint shards per corpus covered every case exactly once, with two workers. Individual acceptance commands took at most 67 seconds.
The unit build took 82 seconds. No full repository or whole-batch cold run was performed.

Evidence: [direct original contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-unresolved-forward-refs-native-direct-contracts.json.gz),
[supplemental contracts and merge checks](../gate/data-layer/fixtures/host/data-layer-port/rr-unresolved-forward-refs-native-supplemental-contracts.json.gz),
and [duplicate-finding regression](../gate/data-layer/fixtures/host/data-layer-port/rr-unresolved-forward-refs-wrapper-red.json.gz).

The inventory is 69 drafts and 31 absent translations. Full migration acceptance remains false.
Production integration, persistent-workspace contracts, and the remaining rules still require work.
No Linear ticket was created. The next missing translation is `conn_or_txn_union`.


## 2026-11-07 — Canonical annotation rows and connection/transaction unions

The previous delivery made progress: it adopted Final shape and completed the focused FieldInfo contracts.
This delivery translates `conn_or_txn_union`, the next missing rule. The inventory is now 70 drafts and 30 absent translations.
Full migration acceptance remains false.

Merge `8df8997cf` adopts committed Final shape `cbf255bb130afb7580bbae7b67b2d4a5f79b5ba2`.
It includes the Python module world, typeshed integration, and the latest demand-planning correction.
Builds and tests use this repository root. The retired nested engines remain untouched.

The source's messages require Python's canonical annotation text. The advertised node form of `@unparse` has no implementation;
the existing tape-value operation returns tape text. Source text cannot preserve the annotation message contract.
The initial regression failed with `no columns for annotation_text` on the root engine.
Commit `237d49d2c` records the relation and its failing integration test. Commit `e6968ad82` implements the native producer and rule.
The producer's missing planner registration produced a second integration failure, retained in the receipt.

`python/print.rr` declares `annotation_text(node, text)`. The producer uses the existing native Ruff parser and emits typed rows.
It runs one additional AST pass per prepared file batch only when the program reads that relation.
Repeated batch preparation can repeat that pass. It does not invoke Python or make SQL scalar callbacks.
The rule's receiver, annotation, parameter, qualification, allowlist, and reporting decisions remain Datalog predicates.
No connection or transaction type names appear in the native printer implementation.

The formatter passes 107 comparisons recorded offline from Python 3.13.18. They cover literals, Unicode printing, surrogates,
precedence, tuples, slices, calls, lambdas, comprehensions, and f-strings. The Unicode printing table records all 712 printable ranges.
Both artifacts have a registered deterministic generator. This capability covers annotation expressions, not arbitrary statement printing or signature hashes.
No repository-scale performance improvement is claimed for this additional native pass.

The rule preserves the source's unusual argument order: ordinary positional arguments, then positional-only arguments, then keyword-only arguments.
It ignores variadic arguments and `self`, checks both bare and nearest-class allowlist names, and stops at the first union parameter.
The separate-parameter diagnostic retains every eligible parameter in source order. Quoted annotations remain single names; they are not parsed as unions.
Configured reporting verifies the original signed allowlist through the shared native signing library.

The acceptance ladder caught real defects before the final receipt:

- The finding name `Union` collided with a SQL keyword. Renaming the Datalog finding fixed execution.
- `Callable[(Connection, ReadTransaction)]` was a missed positive. The rule now strips CST wrappers before classifying tuple slices.
- Reporter findings use severity `error`, independently of the plugin's `hard-fail` enforcement. All three configured finding clauses now state it.
- Configured discovery excludes `api/`, despite a stale discovery docstring. Native scope now follows the executable source.
- Layer-directory matching now follows Python's Unicode decimal digits and its treatment of newlines after the underscore.

Final acceptance uses frozen root CLI SHA-256 `8da9477947d9b95637d39e579c44468d5b3e61cd5075e97aad2b8ac06f7ab205`:

- All five unchanged original cases pass, including both positives. Five native calls preserve all five complete API values.
- All 83 supplemental API cases pass, including 56 positive cases and exact I/O exception observations.
- All 94 configured reporter cases pass, including 60 positive cases and complete messages, paths, codes, order, and severities.
- The original positive rejects a disabled detector. Two independent reporter controls reject disabled union and separate-parameter decisions.
- Unsigned and malformed allowlist signatures match the complete source reporter failure result.
- All 107 formatter comparisons, 60 focused engine tests, 52 migration tooling tests, and both authoring gate tests pass.
- All 70 drafts pass the reader. The two ignored engine tests are upstream file-writing utilities; the whole-library checker was excluded.

Each final corpus uses eight disjoint shards, with two workers. Every case appears exactly once in its corpus receipt.
Final shard commands took at most 24 seconds. The native unit build took 81 seconds; the first implementation build took 119 seconds.
No full repository or whole-batch cold run was performed. The receipt retains earlier failing runs and the exact validation script.

Evidence: [direct original contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-conn-or-txn-union-native-direct-contracts.json.gz)
and [supplemental contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-conn-or-txn-union-native-supplemental-contracts.json.gz).
Production integration, persistent-workspace contracts, and the other ports still require work. No Linear ticket was created.
The next missing translation is `async_patterns`.

## 2026-11-07 — Adopt Final shape process caches before further ports

The user required alignment with Final rerail shape before further rule development.
Checkpoint `c4f17a5a0` preserves the unfinished async translation and its failing path-scope cases.
It does not certify that port.

Merge `2bf167c90` adopts Final shape through `a2c9ee1231fd2be9d8ecc505a3170c67ca5134a3`.
The merge required no conflict resolution. It includes decision 0060's process caches and collection changes.
The corpus planner, rule reader, lowering bridge, and file-local typing file match the upstream commit exactly.
No sibling work in progress was copied.

All builds and checks use this repository root. The nested engines remain retired.
The remaining native differences provide Boolean demand normalization, Python diagnostics and traversal order, and canonical expression text.
Those capabilities use the current engine's relation interfaces. There is no separate planner or execution path for these ports.
No native implementation changed during this alignment.

The frozen CLI has SHA-256 `ed6ce0db6fa7c9fa268a7b7dc1c63089d4e4ae9b9cdcde28a9c7c8b2db25d50e`.
Its source commit is `2bf167c90`; the receipt records the full commit, command, library hashes, and remaining file differences.
The CLI build took 47 seconds. The native unit build took 89 seconds.

Focused validation on the merged engine:

- All 71 draft policies pass the reader.
- All 61 focused reader/runtime tests pass. Two upstream file-writing utilities remain ignored; the whole-library checker was excluded.
- All 107 recorded native expression-printing comparisons pass in their unit test.
- All 56 migration tooling tests pass, including rejection of libraries from retired checkouts.
- All 11 unchanged async source tests pass, including positives, with 12 complete native API comparisons.

The three configured async path-scope defects remain open. They concern exclusions and API/implementation context inherited from the absolute repository root.
The current rule uses document-relative paths for that context. The source linter uses absolute paths.
The next repair must use the modern repository-fact interface and retain these regression cases.

The inventory is 71 drafts and 29 absent translations. Full migration acceptance remains false.
This alignment does not recertify prior ports on every API or claim that the async port is complete.
No full repository cold run was performed. No Linear ticket was created.

Evidence: [Final shape alignment and focused validation](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-a2c9-alignment.json.gz).

## 2026-11-07 — Exact async patterns on the modern root engine

The 71st draft now passes its focused native acceptance corpus. The inventory remains 71 drafts and 29 absent translations.
Full migration acceptance remains false. Final shape remains at `a2c9ee123`; all native changes use the active repository root.

The rule preserves temporal query/session aliases, nested async and sync scopes, decorator inheritance, transaction parameters, fallback markers, and canonical receiver messages.
Rule decisions remain Datalog. The native printer supplies typed expression text without a Python process.
The supplied-AST adapter transports caller inputs separately from raw source. It does not run the detector in Python.

The acceptance ladder caught these faults:

- Fallback windows needed finite query facts to bind their radius and search fragment.
- Parenthesized receiver expressions needed unwrapping, while actual call nodes needed distinct identities.
- Absolute collector paths inherited exclusions and API/implementation context from the repository root.
- Repository metadata needed ordinary SQL text tests. The per-file vocabulary does not contain those values.
- The external file adapter needed the native exclusion result before it opened a file.

Test commit `f916331eb` fails with `no columns for workspace_root` on the aligned engine.
The modern repository producer now supplies that generic fact once per run. It does not scan or canonicalize the root.
The regression preserves two different caller spellings across repeated runs, even when neither directory exists.
The three configured root-path cases now pass. Test commit `79980c9b4` captures the file-access ordering defect.
Commit `283fe69a0` corrects that adapter ordering through the native predicate.

Final validation uses frozen root CLI SHA-256 `d69a3de9a29acc4238095f4e94f449a18fd8e062e3768e4f6c0114932358fdc0`.
The receipts record the full executable hash and source commit.

- All 11 unchanged original cases pass. Twelve native calls preserve twelve complete API values, including eight positive returns.
- All 158 supplemental API cases pass: 125 positives, 31 clean cases, and two exact exception results.
- All 136 configured reporter cases pass: 98 positives and 38 clean cases.
- An original positive rejects a disabled detector. Seven independent reporter controls reject disabled diagnostic families.
- All 71 drafts pass the reader. All 62 focused reader/runtime tests and 107 printer comparisons pass.
- All 58 migration tooling tests and both authoring gate tests pass.

Each supplemental corpus uses 16 disjoint shards, with two workers. The slowest shard took 26 seconds.
No case was omitted or counted twice. The two ignored native tests are upstream file-writing utilities.
The whole-library checker and full engine suite were not run. No full repository cold run was performed.
No CPU or memory improvement is claimed. Canonical printing can repeat for each prepared file batch.

Evidence: [direct original contracts](../gate/data-layer/fixtures/host/data-layer-port/rr-async-patterns-native-direct-contracts.json.gz)
and [supplemental contracts, regressions, and family controls](../gate/data-layer/fixtures/host/data-layer-port/rr-async-patterns-native-supplemental-contracts.json.gz).
Production integration and persistent-workspace contracts remain required. No Linear ticket was created.
The next missing translation is `async_sync_boundary`.

## 2026-11-07 — Boundary collectors, helpers, and source completion order

The 72nd draft translates `async_sync_boundary` on the same modern root engine. It needs no additional native capability or build.
The native tree and dependencies still match the frozen executable's source commit. Final shape remains at `a2c9ee123`.

The translation preserves three bridge categories, textual call-name rules, function-stack behavior, source triggers, allowlists, and informational paths.
Lambdas and classes remain transparent to the collector's function stack. Decorators and defaults use the decorated function's context.
An attribute chain retains its suffix when its receiver is a call or subscript. Executor matching uses the source's literal suffix test.
The raw scanner retains allowlisted findings; configured reporting suppresses them. The source's substring path rules remain unchanged.

Filesystem enumeration and DTO construction remain transport operations. Datalog counts the requested inventory, including repeated paths.
Three native category queries back `LintResult` properties and return the original DTO objects by index.
Call-name, allowlist, and informational helpers execute natively. No original detector method executes in the native test run.

Commit `bcc05af1a` retains a real failing step: three original positive tests fail against an empty detector; both clean tests pass.
The complete rule then passes all five tests. The supplemental corpus catches a decoding fault: the source propagates invalid UTF-8.
Commit `ecc54a17e` preserves that exception through a native failure row instead of silently omitting the file.

The first complete supplemental run also catches a cross-file ordering mismatch. All fields and counts agree.
The source fact engine inserts bundles in `as_completed` order, then the scanner appends each whole file's findings.
Twelve unchanged source runs produce both permutations. The receipt retains all twelve raw results and the source-file hashes.
The comparator now permits only whole-file group permutations. It preserves in-file order, duplicates, every field, counts, and exact exceptions.
It rejects interleaved file groups. Three new regression tests protect that boundary. No raw report value is rewritten.

Final focused acceptance:

- All five unchanged original tests pass: three positives and two clean cases. Nine native calls preserve five complete scanner returns.
- The original category-property assertions also execute native queries. They do not count as additional original test cases.
- All 295 supplemental cases pass: 165 raw scans and 130 helper contracts.
- Raw scans include 148 positives, 16 clean cases, and one exact decoding exception.
- Helper contracts include 112 allowlist cases, seven informational-path cases, and eleven supplied-AST call-name cases.
- All 158 configured reporter cases pass, including 74 positives and 84 clean results.
- An original positive rejects a disabled detector. Three independent reporter controls reject disabled bridge categories.
- All 72 draft readers, 62 migration tooling tests, and both authoring gate tests pass.

The final corpus uses 16 disjoint shards per interface, with two workers. Every case appears exactly once.
The slowest final shard took 36 seconds. No full repository cold run was performed.
The existing engine's focused native checks remain recorded in the preceding delivery; this translation changes no native code.

Evidence: [direct original assertions](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-boundary-native-direct-contracts.json.gz)
and [complete supplemental corpus, regressions, ordering evidence, and family controls](../gate/data-layer/fixtures/host/data-layer-port/rr-async-sync-boundary-native-supplemental-contracts.json.gz).
The inventory is 72 drafts and 28 absent translations. Full migration acceptance remains false.
Production integration, CLI/enforcement, persistent-workspace acceptance, and the remaining translations still require work.
No Linear ticket was created.

## 2026-11-07 — Plan-scope ownership, canonical calls, and primary document boundaries

The 73rd draft translates all thirteen ownership rules into `.rr`. The inventory is 73 drafts and 27 absent translations.
Full migration acceptance remains false. The active engine remains this repository root.
Merge `c372d1556` adopts Final shape through `b505d5121`, including its proposed `.rr` tape specification.
That upstream change contains documentation only. No sibling work in progress or retired engine code was copied.

Ownership needs canonical whole-call and callee text for substring checks. The source uses `ast.unparse` for both.
Test commit `029351a86` fails on the existing modern engine with missing `call_text` statistics.
Commit `eea5ed2da` extends the existing generic printer with `call_text` and `call_function_text`.
The relation projection requests the needed columns. All four text columns share the existing native parse.
There is no rule-specific Rust detector or Python execution in the rule.
The new oracle covers 118 source cases and nineteen canonical call/callee pairs. All 107 existing annotation comparisons still pass.

The Datalog rule preserves ordinary positional-argument order, exact nominal annotation spelling, descendant checks, direct class members, and Store contexts.
Its nested-runner checks use the existing tape containment input, with a separate AST ownership edge for decorator expressions.
It does not build a complete production ancestor closure.
File selection is one batch over the available filesystem inventory. The file adapter then reads only selected files.
The collector adapter transports caller-owned AST fields. Its raw source remains an independent input.

The regression ladder caught these faults:

- Five original positives failed because the public API bypasses the configured plugin's source-trigger gate.
- A valid multiline literal gained a parser error from an intermediate fragment document with the same path.
- The minimal engine query accepted that literal. A diagnostic query identified the error document's role as `fragment`.
- The rule now selects primary documents. Original-file analysis cannot consume errors from synthetic fragments or annotation guests.
- Public parser-error facts require the `SyntaxError: ` prefix. Raw error facts omit the severity key.
- NUL errors have no filename or line suffix. The configured reporter omits parser errors entirely.
- Several raw API positives lack plugin triggers. The corpus now retains both those cases and trigger-enabled counterparts for every code.

The frozen root CLI has SHA-256 `11d9746f23c2bda97ff8eedada4bcf8c9dbf9e4f646ed7e0d69e929ca3f88db3`.
Its production source is `eea5ed2da`, with Final shape baseline `a2c9ee123`.
The branch later merged the documentation-only `b505d5121`. That merge does not change the binary's recorded source.
The unit receipt separately records its later test-only source and binary hash.
Final focused acceptance:

- All 15 unchanged original tests pass. Fourteen complete API returns match across 28 native calls.
- The single construction-only test retains its exact Python inheritance assertion. It does not count as native detector coverage.
- Seventeen source/native callback roundtrips cover that interface, including all original witnesses with separate raw source and supplied AST inputs.
- All 223 API cases pass: 163 detector positives, 55 clean results, four parser-error fact results, and one exact decoding exception.
- All 202 configured reporter cases pass: 110 positives and 92 clean results.
- Every configured code from PSO001 through PSO013 has a positive observation and an independent disabled-detector control.
- An unchanged original positive rejects a disabled detector with its file inventory preserved. The API parser-error output has a separate control.
- All 73 draft readers, 63 focused native reader/runtime tests, 63 migration tooling tests, and both authoring gate tests pass.

Each final corpus uses 16 disjoint shards with two workers. The slowest final shard took 40 seconds.
Every case appears exactly once. Two upstream file-writing native utilities remain ignored; the whole-library checker was excluded.
No full repository cold run or full engine suite was performed. No CPU or memory improvement is claimed.
Native canonical printing can repeat when a prepared file batch is built again.

Evidence: [direct original assertions](../gate/data-layer/fixtures/host/data-layer-port/rr-plan-scope-ownership-native-direct-contracts.json.gz)
and [complete supplemental contracts, failed steps, and positive controls](../gate/data-layer/fixtures/host/data-layer-port/rr-plan-scope-ownership-native-supplemental-contracts.json.gz).
Production integration, CLI/enforcement and persistent-workspace acceptance remain required. No Linear ticket was created.
The next missing translation is `transaction_closures`.

The alignment audit confirms that local and remote Final shape both point to `b505d5121`.
That commit is an ancestor of this branch. No committed Final shape engine change remains unmerged.
The planner and executor retain its resolutions. Local native differences provide the generic capabilities described in this entry and previous entries.
The tooling rejects libraries from retired checkouts, including identical copies. All 63 tooling tests pass again.
Regeneration leaves all 88 inspected rule, manifest, corpus, and printer-oracle files unchanged.
The build receipt now names the actual upstream ancestor, `a2c9ee123`, instead of the later documentation-only commit.
Both evidence bundles were regenerated and checked without changing the recorded test results.

## 2026-11-07 — Transaction closures on Final shape 43c8c521d

The 74th draft ports `transaction_closures` into native `.rr`. The inventory is 74 drafts and 26 absent translations.
Full migration acceptance remains false. Production registration, CLI/enforcement, and persistent workspace acceptance remain open.
Merge `54d160500` adopts Final shape `43c8c521d`, including the `.rr` tape implementation.
All engine work uses this repository root. This rule requires no Rust engine change.

The rule uses ordered detector jobs, with one job for each transaction call.
It preserves keyword callback precedence, the second positional callback, and the last direct named definition.
Named callbacks require a definition on an earlier line. Repeated calls retain repeated findings.
Detector locals accumulate in visit order, including nested function parameters and assignments.
Function headers do not enter detector traversal. Nested lambdas stop traversal, as the source implementation specifies.
Module and enclosing-function bindings determine inline callbacks. Named callbacks use the source's narrower direct outer bindings.
Production containment uses existing tape relations. Supplied ASTs use flat external argument columns, with no Python invocation in the rule.

The regression ladder found and corrected these faults:

- The first generated output name, `Scope`, collided with the library's `scope` relation after lowering.
- The external AST transport converted zero columns into empty strings. Failing tests now protect numeric zero locations.
- The recorder did not normalize a string equal to the fixture root. A separate failing test protects that boundary.
- A generic document gate bypassed the supplied-source trigger. An independent raw-source and AST case exposed the error.
- Resolving absolute paths changed `/var` into `/private/var`. The adapter now preserves the caller's absolute spelling.
- Configured reporting allows `impl/test_*.py`. The draft incorrectly excluded those files in addition to test directories.
- Constant assertions initially read Python globals. They now consume the native rule's constant facts from one batch per suite.
- Native constants bind after collection and retain imported set aliases. Mutating a constant cannot omit parametrized original cases.

The original module runs unchanged. All 131 cases pass, with 126 complete API observations and 162 native calls.
Ninety-eight cases make native queries. Twelve additional cases assert values from the native constant batch.
The remaining 21 cases retain host protocol, storage, DTO, export, constructor-identity, and argparse assertions.
The direct receipt identifies them individually. None counts as detector-positive coverage.
Forty-seven original cases produce a native TXN001–TXN004 finding.
The test guard rejects calls to the original Python detector methods.

All 199 supplemental API cases pass. They contain 113 detector positives, 81 clean detector states, four I/O or parser findings, and one exact ValueError.
All 73 configured reporter cases pass: 44 positive and 29 clean.
The corpus covers repeated visits, duplicate findings, supplied function callbacks, independent raw source and ASTs, parameter kinds, ordering, and file selection.
Four original positive tests and four configured positives reject independent disabled-detector mutations.
The parser-output control also rejects empty output. Two original constant tests fail when the native rule omits a required fact.
Those constant controls retain all 131 collected cases and fail inside the selected test call.

The broader reader gate exposed an older imported draft's unsupported slice-pattern syntax in the new frontend.
`versioned-table-names.rr` now uses typed slice children and token adjacency to select the upper bound.
A focused source comparison also exposed its generic slice message. The slice output now preserves the source message.
All 18 compatibility cases pass, including ten positive findings, with complete code, path, line, and message comparisons.
This is a compatibility check for that slice component. The complete versioned-table-names rule still requires acceptance.

All 85 candidate readers pass, including all 74 target drafts.
The focused native suite passes 70 tests, with two upstream file-writing utilities ignored.
Both canonical-printer tests pass. All 65 migration tooling tests and both authoring gate tests pass.
Regeneration leaves all 88 inspected rule, manifest, and corpus files unchanged.
The final corpus and control run uses 37 disjoint tasks with two workers. Its slowest task takes 45 seconds.
An earlier combined control command reaches the 115-second limit. Final controls run separately by family and auxiliary contract.
No timed-out run certifies acceptance. No full repository cold run or full engine suite runs in this checkpoint.

The frozen CLI hash is `e1530a835e190557d019cc863fef727acbd041701d3e6e5d2670bb71c61dd72b`.
Its engine source is `54d160500`, with upstream baseline `43c8c521d`.
The CLI build takes 1 minute 27 seconds. The separate unit build takes 2 minutes 43 seconds.
These build times are not rule CPU measurements. No CPU or memory improvement is claimed.

Evidence: [all original assertions and positive controls](../gate/data-layer/fixtures/host/data-layer-port/rr-transaction-closures-native-direct-contracts.json.gz)
and [supplemental corpus, failed steps, reader checks, and slice compatibility](../gate/data-layer/fixtures/host/data-layer-port/rr-transaction-closures-native-supplemental-contracts.json.gz).
No Linear ticket was created. The next missing translation is `transaction_callback_wrappers`.
Final shape advances to `824c07935` during acceptance. The next step merges that committed head and checks the updated engine.

## 2026-11-07 — Replay after Final shape's tape lowering and relation lifetimes

Merge `eec59e31c` adopts Final shape `824c07935`, including decision 0062 and the ServerPlan lifetime changes.
The root engine now lowers rule programs from their tapes.
Its planner hands exports to their last reader, shares guest copies by relation, and drops native relations after export.
The merge retains those upstream resolutions. No retired engine file is restored and no sibling work in progress is copied.

The new frozen CLI has SHA-256 `4acaab90727fc0c8d883653dabcb6dfeb4592a40d495259ef0b7cbb914a38b94`.
Its source is `eec59e31c`, with upstream `824c07935`. The separate unit binary records that same source commit.
The CLI build takes 39.91 seconds. The unit build takes 1 minute 29 seconds.

The complete focused replay passes again:

- All 131 unchanged original cases, 126 complete API observations, and 162 native calls.
- All 199 supplemental API cases, including 113 detector positives.
- All 73 configured reporter cases, including 44 positives.
- Four original and four configured detector controls, plus the parser and two native constant controls.
- All 18 source/native slice comparisons, including ten positive findings.
- All 85 candidate readers, including the 74 target drafts.
- All 74 focused native reader/runtime tests and both canonical-printer tests.

The two upstream file-writing utilities remain ignored. The whole-library checker and full engine suite do not run.
All corpus indices appear exactly once. No original detector method runs in native acceptance.
The 21 original host protocol/storage cases retain their assertions and remain separate from detector coverage.

Receipt validation catches a packaging defect: the controller overwrites its parser result while it records constant controls.
The parser checks themselves pass. The faulty bundle stored a constant report under the parser key.
The controller now retains the parser result separately. The validators require the fresh-API report format and its exact failed-positive result.
Auxiliary controls run again against both frozen engines. Both sets pass, and both historical bundles are regenerated.
The new receipt retains the incorrectly labeled bundles as regression evidence.

The post-merge run uses 40 bounded tasks with two workers. The slowest task takes 62 seconds.
No full repository cold run occurs and no CPU or memory improvement is claimed.
The migration remains 74 drafts and 26 absent translations, with production and persistent workspace acceptance open.
No Linear ticket is created.

Evidence: [post-merge source values, native observations, controls, and validators](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-824-transaction-closure-alignment.json.gz).
Final shape advances to `251fed5c0` during this replay. That committed backend update is the next integration checkpoint before further ports.
# 2026-11-07: Adopt current Final shape before further migration

The user required this worktree to use the current Final rerail shape engine.
Merge `a15fa3c73` adopted its committed changes through `b10c6c996`.
Merge `98ceb503a` then adopted `1e4840f3c`, including Bitlex, the tape pipeline, and residency changes.
Both merges retained the upstream resolutions. The withdrawn library stage and replay stayed withdrawn.
Decision 0065 remains a draft. Its typing direction replaced the older staged-library description in the migration guide.

The frontend, executor, corpus scheduler, typing, transducer, residency, and content-store sources match the adopted upstream commit.
The existing generic Python compatibility extensions remain on this runtime.
They supply canonical expression text, parser diagnostics, traversal order, root metadata, and Boolean-context normalization after demand planning.
Their relation registrations and tests also remain. No new Rust detector or execution engine was added for this checkpoint.

The manifest now records all 18 extension files, including the dependency lock, with exact hashes.
The migration inventory command checks root-checkout selection, upstream ancestry, extension boundaries, and extension bytes.
The regression ladder first failed because the audit was absent. The implemented audit then passed its tests.
Additional cases reject missing ancestry, unrecorded native files, and an executor outside the permitted extension boundary.
All 72 migration-tool tests passed. The identical-library test still rejects a retired checkout.

Acceptance used a newly built root CLI, frozen as `engine-3b5314e26f3b4b1e`.
Its SHA-256 is `3b5314e26f3b4b1e5ba3184767926fedfc83d8a45de8a260b02af5da6e0fc31e`.
Its build receipt records source commit `98ceb503a`, upstream `1e4840f3c`, and the native, Bitlex, Cargo manifest, and lock tree hashes.
The CLI build took 81 seconds. The unit build took 147 seconds under the existing build-duration authorization.
These compile times are separate from runtime measurements. This checkpoint makes no performance claim.

The in-progress `transaction_callback_wrappers` port passed all four unchanged original tests, with two positive and two clean cases.
All four complete raw-fact observations match the source. All four native calls succeeded, with no source detector fallback.
All 116 supplemental API cases passed, including 71 positives. All 103 reporter cases passed, including 69 positives.
The two finding families each passed enabled controls and rejected disabled controls in both interfaces.
Each disabled original control retained the full collection and failed its positive assertion through successful empty native output.

All 86 candidate reader checks passed, including every one of the 75 target drafts.
Focused native groups passed 118 tests. The counts were RR 74, printing 2, diagnostics 2, and versioned SQL 5.
Bitlex passed 19, block plans passed 14, and corpus planning passed 2.
Six upstream cases remained ignored. The receipt names each case and retains its reason:
`hand_written_rules_lowering_report`, `write_rr_translations`, `dump_json_rows`, `json_batch_cost_per_block`,
`json_corpus_tapes_equal_tree_sitter`, and `json_tier1_is_simdjsons_stage1`.
The RR unit command excluded the broad `rule_syntax_files_pass_check_rr` survey. The 86 candidate readers ran separately.
No original callback assertion, supplemental API case, or reporter case was skipped.

The [alignment receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-1e484-alignment.json.gz)
retains the build provenance, commands, source oracles, native observations, mutation controls, logs, and receipt validator.
The validator checks exact shard coverage, ordered values, source pinning, executable hashes, and source tree identity.
Its initial run expected the API report status `passed`; that report uses `completed`.
The corrected validator checks `completed` and every individual passed result. It does not discard failures.

The migration inventory remains 75 drafts and 25 absent translations. Full migration acceptance remains false.
Production registration, CLI/enforcement, and persistent workspace acceptance remain open.
Further ports use this root engine and its current shared libraries. The nested historical engines remain retired.
No whole-repository cold execution or whole-batch benchmark ran for this checkpoint.


## 2026-11-07: Adopt Final shape prepared-program storage and verify the current rule

The active engine now includes Final shape `9993e7e5fbadc8fd8340c4c01e515f3328ffec60`, through merge `8f9f001a4`.
Its decision 0073 keeps prepared programs in the existing content store across processes.
The merge retained its frontend, corpus scheduler, typing, tooling, and prepared-program storage implementations.
These sources match Final shape. The text vocabulary retains only four additional generic Python text registrations.
The recorded native differences comprise 19 extension and test files, including the dependency lock.
No retired engine checkout supplied a build, library, or runtime for this checkpoint.

A new audit regression initially failed because acceptance ignored newer commits on the local Final shape branch.
The implemented audit requires those commits in this branch's history.
A standalone clone without that local branch still requires the recorded upstream ancestor and exact extension hashes.
The earlier root-checkout and extension-boundary checks remain enforced. All 79 tooling tests pass.

The root CLI build took 40.26 seconds. The standard unit build took 55.60 seconds.
Compile times remain separate from runtime measurements. This checkpoint makes no performance claim.
The frozen CLI has SHA-256 `a9e68e528e1ea4f1158300c47cbd4ac17d057fedc4f9abe587a169aa02cdd696`.
Both build receipts record the merged source and native, Bitlex, Cargo manifest, and lock tree hashes.

The first unit command incorrectly selected the headless CLI feature.
It failed with 15 missing `WorkspaceStorageTask.compute` errors in unchanged upstream storage tests.
The standard unit build succeeded without an engine change.
Focused native groups pass 102 tests: RR 66, inputs 9, literal decoding 6, printer 2, simplification 11, and typing 6.
The prepared-program store regression and demanded Boolean regression each pass one test.
Three upstream reporting or benchmark helpers remain ignored: `hand_written_rules_lowering_report`, `write_rr_translations`, and `nearest_costs_by_solver`.
The stored-program test proves identical rows after preparation, reuse from the store, and reuse from memory.

The current `transaction_patterns` draft passes all 118 unchanged original tests, with no skipped case or detector fallback.
All 117 recorded API observations match, including helper returns and collector state.
Twelve cases make no recorded helper call. Their individual identities remain in the receipt.
Eleven inspect constants obtained from the native configuration query. The export assertion uses host reflection.
Ephemeral signing creates test fixtures in the host harness. Signature canonicalization and verification execute natively.
Filesystem discovery and metadata transport also remain host adapter work.

All 151 supplemental API cases pass. They include 124 scans, with 96 positives, and 27 helper cases.
All 61 reporter cases pass, including 17 positives. Every supplied expected value comes from the pinned original source.
Ten independent controls disable the detector, call collection, category override, inference, signature message, signature verification,
baseline loader, constants, constructor classification, or category membership output.
Each disabled output makes its selected original positive assertion fail through a successful native query.
The first three controls also fail their configured reporter positives. Their enabled forms pass.

The final API ladder includes `_check_call` on `conn.read(conn.write())`.
The source single-call helper adds only `read`. The recursive visitor owns the nested `write`.
The red replay exposed an extra `write` in the native helper result.
The rule now restricts single-call demand to the supplied root occurrence. The final 151-case replay includes this regression.

Earlier ladders in this checkpoint also exposed category precedence, prefix matching, bytes escapes, and known-empty input errors.
The rule retains source annotation precedence, category dictionary order, first-parameter selection, duplicate calls, and function-exit ordering.
The modern input reader preserves ordered JSON wildcard positions and shares one decoded document across selectors.
Its type projection distinguishes invalid shapes. Its JSON projection preserves large integers in the public loader result.
The bytes decoder preserves unknown byte escapes instead of interpreting them as Unicode string escapes.
All nine input-reader tests and six core literal tests pass on the merged engine.

Source FactEngine workers return complete file groups in completion order.
Six source recordings demonstrate both two-file permutations. The harness permits those group permutations explicitly.
It retains every field, duplicate, and within-file order, and rejects interleaved file groups.
The receipt preserves the source and native orders before comparison.

All 87 candidate reader checks pass, including every one of the 76 target drafts.
Every acceptance command stayed within its 115-second bound. No full-repository or whole-batch cold benchmark ran.
The [alignment receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-9993-alignment.json.gz)
contains complete oracles, native results, controls, commands, compile diagnostics, logs, build identities, and its validator.
The validator checks exact shard coverage, complete values, source pinning, native hashes, and source tree identity.

The inventory now contains 76 drafts and 24 absent translations. Full migration acceptance remains false.
Production registration, CLI/enforcement, and persistent workspace acceptance remain open.
Further migration work uses this merged root engine and its shared libraries.


## 2026-11-07: Port lazy-result eligibility and the baseline ratchet on Final shape

The 77th native draft is `lazy_result_usage`. The modern engine remains Final shape through `9993e7e5f`.
The migration audit confirms its ancestry and the same 19 reviewed native extension files.
This port requires no native engine change or rebuild. It uses the frozen root CLI with SHA-256
`a9e68e528e1ea4f1158300c47cbd4ac17d057fedc4f9abe587a169aa02cdd696`.

The rule preserves transaction, query, and table class eligibility and the source's explicit skip list.
It excludes classes whose names start with Async or Lazy after leading underscores.
It recognizes direct sibling lazy methods, protocol bases, legacy decorators, abstract methods, placeholders, and lazy references throughout function syntax.
Nested functions retain the source collector's current class. Findings retain function completion order and duplicate occurrences.
Return annotations use the existing native canonical printer. Supplied ASTs enter as flat syntax columns through the external test adapter.
No rule invokes Python.

Production metadata selects the data-layer roots before parsing. Relational predicates retain the exact layer-name and skip semantics.
API buffer transport uses a reserved layer-shaped path. The rule reads each buffer's original scope from the supplied path columns.
This preserves explicit-file behavior while the production selector avoids unrelated repository roots.
The factory applies skip patterns. Direct collectors honor their explicit disabled argument instead.
All files in a scan enter one native query. Collector transport sends one complete tree per query.

The original module has five tests: two positive and three clean cases.
All five unchanged tests pass. Their five complete returned DTO lists match the pinned source recording.
All 231 supplemental API cases pass: 102 file scans, 108 collector cases, sixteen repository scans, and five skip-helper cases.
There are 117 positive list results. All 114 reporter cases pass, including 52 positives.
Reporter cases cover integer, string, Boolean, fractional, and large numeric baseline thresholds, plus a two-file repository total.
Single-value and sequence methods have independent controls. Each enabled original and reporter positive passes.
Disabling the corresponding native eligibility arm makes both positive interfaces fail through successful empty native output.

The TDD ladders exposed four translation defects.
First, the supplied-AST placeholder rule requested a nonexistent literal-type column. The existing column is `astValueTypes`.
The red ellipsis case returned a false positive. The corrected rule passes it and the full corpus.
Second, the baseline declaration requested a symbol from a numeric JSON value.
The typed reader correctly rejected that shape. The rule then used its absent-baseline default of zero.
The rule now uses the existing JSON projection, which preserves numbers and strings as serialized input values.
Third, the factory witness exposed missing skip-pattern application. The factory path now applies it without changing direct collector behavior.
Fourth, an integer count was compared with the engine's encoded float key.
Both operands now enter the float domain explicitly. Fractional thresholds and the repository-wide count pass their recorded source results.
These repairs change Datalog and test transport. They add no specialized native detector or alternate runtime.

One earlier reporter process reached its 100-second timeout on the default shared store.
The final replay uses isolated per-shard store paths and completes every command within 115 seconds.
This change isolates test state. It does not establish the cause of that earlier timeout or demonstrate a performance gain.
The receipt retains the original traceback. No full-repository or whole-batch cold benchmark ran.

The reporter harness now writes the active case and completed values before each case starts.
A simulated process failure proves that the checkpoint retains both the failed case identity and earlier full results.
All 80 harness tests pass. All 88 candidate reader checks pass, including every one of the 77 target drafts.
The reader lists unsuccessful library-search probes as absent files before it resolves tables beside a rule.
The receipt validator checks those absences and every successful file hash. It does not treat a missing probe as a missing rule dependency.

The [native receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-lazy-result-usage-native-contracts.json.gz)
contains the original and supplemental source recordings, complete native observations, disabled-arm controls, regression history, commands, and its validator.
The validator checks exact shard coverage, complete values, source pinning, the frozen executable, library identities, and the final rule hash.
It also retains the failed intermediate reporter records. Accepted results use only the final verified replay.

The inventory now contains 77 drafts and 23 absent translations. Full migration acceptance remains false.
Full shared registry/scope assertions, production registration, CLI/enforcement, and persistent workspace acceptance remain open.


## 2026-11-07: Align with Final shape and port module-level caches

The root engine now includes committed Final shape through `f0571b1e7`.
Merge `57ad2b1ee` takes `6c22fca95`: prepared programs persist as typed facts, printed rules, and fact tapes (decision 0074).
Merge `0ca019c6d` takes persistent matcher automata (decision 0075), definite guest specialization, and content-store fixes.
The upstream executor, scheduler, typing, residency, and content-store files have no local diff.
The same nineteen reviewed generic extension files remain. This rule requires no new native detector.

The migration audit rejects a retired checkout, unreviewed native files, or a missing committed Final branch update.
Acceptance uses only a frozen CLI built from this repository root.
The two nested historical engines remain retired. Their old receipts do not certify the current engine.
The adoption manifest records Final shape `f0571b1e78d2cb6922084136743441c34bf7cf68` and the reviewed extension hashes.

The new `module_level_caches` draft implements mutable assignments, cache constructors, decorators, and allowlist behavior in Datalog.
It preserves source scope, severities, duplicate findings, ordering, formatting, match keys, and signatures.
Managed cache constructors retain the source's first-keyword behavior and exact Boolean identity test.
The production metadata selector limits parsing demand. Relational filters also enforce source exclusions for explicitly supplied buffers.
CLI scans and the configured reporter have different source admission rules. The translation preserves those differences.

The TDD ladders exposed these defects and corrected them:

- The first original positive missed a cache decorator. Function and classmethod arms now supply the missing findings.
- A parenthesized name used a tuple-pattern CST node. Single-name targets now unwrap without admitting comma-separated tuple targets.
- AST child edges lost the `None` key in `{**{}}`. Separate list-cardinality transport preserves that source AST distinction.
- Comment-only containers appeared nonempty. Container classification now ignores comments.
- Repository scans swallowed a decoding exception. They now retain strict source reads; the single-file helper preserves its own catch behavior.
- Explicit buffers bypassed metadata exclusions. Relational admission filters now preserve the source reporter's hidden-directory exclusions.
- The loader's missing-field `KeyError` escaped oracle recording. The harness now records its exact type and arguments.
- The configured reporter converted that uncaught exception into an ordinary finding. A typed native outcome now preserves the recorded exception contract.

The initial synthetic reporter regression omitted its policy envelope and failed before the intended assertion.
The corrected red test compares the expected exception DTO with the ordinary finding list.
The receipt retains the meaningful red result and the passing real reporter case.
Rules invoke no Python. The external harness transports input syntax, files, DTOs, and recorded exceptions.

All eight unchanged original tests pass on the current engine, with seventeen complete API and property observations.
All 278 supplemental API cases pass, including 157 detector-positive cases and eight expected exceptions.
All 120 reporter cases pass, including 67 positive finding cases and one expected uncaught configuration exception.
No original module-cache assertion is excluded. Twelve independent controls cover detector families, allowlist properties, formatting, counts, and exception transport.
Enabled controls pass. Disabling each selected output makes its positive assertion fail through a successful native query.

All 89 candidate reader checks pass, including the 78 target drafts. All 85 harness tests pass.
The previous lazy-result rule also passes its five original cases and focused multi-file and fractional-baseline reporter shards.
The focused native groups pass 124 test executions, including one repeated Boolean-demand regression; 123 passing test identities are distinct.
Four upstream tests remain ignored: the two rule-generation helpers, the nearest-solver benchmark, and the matcher determinization benchmark.
The Python typing oracle passes its 44 runtime comparisons after the upstream guest-specialization fix.

One upstream prepared-program test failed with parallel test threads.
Its cache lookup selects any matching program from a process-wide cache, rather than the program that the test just ran.
The failure selected the other test's `negated` query. Roundtrip Debug output differed by an empty `input_error` fact entry.
All three tests pass with `--test-threads=1`. The receipt retains both runs; no runtime change hides the failure.

The root CLI build finished in 55.76 seconds. The standard-feature unit build took 126.92 seconds, separately from execution.
Builds used the existing extended authorization. Each acceptance command stayed below 115 seconds.
The final CLI SHA-256 is `53756501a7e401facb2ddf8d2d2295620aa1791c4c7390edfecfbee1ad610af0`.
Its source commit is `0ca019c6d7dfb0c5d6b3dff4609582719ea683b0`.
Earlier complete runs on `6c22fca95` remain historical evidence after the subsequent merge and full current-rule replay.
No full-repository or whole-batch cold benchmark ran. These results establish parity for the recorded inputs, not a performance improvement.

The [native receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-module-level-caches-native-contracts.json.gz) contains complete observations, commands, hashes, controls, failures, and its validator.
Rule regeneration leaves the generated bytes unchanged. The inventory now contains 78 drafts and 22 absent translations.
Full migration acceptance remains false. Shared registry/scope assertions, production registration, CLI/enforcement, and persistent workspace acceptance remain open.


## 2026-11-08: Port render f-strings on the modern root engine

The 79th native draft is `render_fstrings`. Final shape remains at committed `f0571b1e7`; this branch includes that tip.
The original suite has five tests, including two finding positives. The harness records every test and ten complete API/property observations.
The initial native replay failed both finding positives and the render-method count. Commit `0edb6a6be` records that red translation stage.

The rule tracks the outermost active render function. Nested functions and classes retain that context.
It recognizes `render` and the `render_` prefix, with native identifier normalization for source files.
Class names come from the class active when the outer render function starts.
Decorators, defaults, return annotations, nested strings, adjacent literals, and format specifications follow source traversal behavior.
Each format specification is another `JoinedStr` finding, including an empty specification.
Findings retain visit order and duplicates. Previews join decoded constants and interpolation placeholders, then truncate by Unicode characters.

Joining literal pieces to individual format-specification spans required absolute byte starts.
The pre-extension root reader rejected the undeclared `str_part_offset` relation; the receipt retains that probe.
Commit `9968bb71f` exposes those existing starts from the current native decoded-literal producer.
The producer still uses the same tape and decoder. It emits the column only when demanded.
The planner and typed shard mapping preserve the literal node column across file batches; offsets remain numeric byte positions.
The generic offset test covers non-ASCII text, format-specification constants, and a debug field.
Its initial debug-field expectation omitted the opening brace. The corrected expectation matches the existing piece-start projection.
The receipt retains the failed initial expectation. No runtime change moved that existing start.

The first supplemental replay exposed three translation or producer defects.
Named Unicode escapes remained literal text because the literal producer used its older partial decoder.
Commit `bad5c0824` reuses the shared Python value decoder for valid scalar text, including Unicode character names.
The existing tolerant fallback remains for incomplete or non-scalar input.
Embedded NULs caused SQLite string length to stop early, so long previews escaped truncation.
The rule now tests its Unicode slice at character fifty and takes the first forty-seven characters when truncation is required.
Explicit API scans also inherited reporter directory exclusions incorrectly.
The source FactEngine admits explicit files in those directories. The rule now preserves that behavior.
The collecting API separately excludes the source's `__pycache__` substring, while reporter discovery excludes its five ignored directory names.

Both summary and detailed formatting execute in Datalog over transported report fields.
The rule preserves report counts, stable ordering, class/method groups, blank lines, and exact preview text.
Additional cases exposed a path-conversion error: empty outside-root reports must format successfully; positive reports must raise the source ValueError.
Path transport now supplies relative-path results or filesystem errors. Native counts and ordering select whether an error applies.
The harness records the red empty-report case and all four final outside-root cases.
Those fixtures use relative paths so exception strings have no temporary directory names.

A final adapter review found method-label construction and empty-report selection in host code.
Both decisions now belong to Datalog. The adapter transmits separate method/class fields and follows a native report-selection result.
The first selection relation had a negative self-dependency, which the engine correctly rejected.
An explicit zero-count arm removes that dependency. The receipt retains the rejection and the final execution results.
The host handles filesystem I/O, supplied AST columns, and passive DTO reconstruction. Rules invoke no Python.

All five unchanged original tests pass, including all five `violation_count` property observations.
All 165 supplemental API cases pass. They include 92 detector-positive cases and three expected exceptions.
The corpus covers 34 file analyses, 35 source analyses, 31 collectors, and 31 statistics collectors.
It also covers sixteen directory scans, ten name probes, and eight report-formatting cases.
All 44 reporter cases pass, including 33 positives. Production scope cases include tests, conform files, hidden directories, and unrelated roots.
Eight independent controls disable findings, statistics, count properties, previews, names, source gates, summary output, or detailed output.
Each enabled witness passes. Each disabled output makes the selected positive assertion fail through successful native execution.

All 90 reader checks pass, including 79 target drafts. All 85 harness tests pass.
The focused native checks pass 74 tests, including the new byte-offset contract and literal-demand regression.
The two upstream rule-generation helper tests remain ignored. All eight original module-cache tests also pass on the new engine.
Rule decisions and helper formatting remain in Datalog on the modern root engine.

The final CLI build took 20.137 seconds; the standard-feature unit build took 43.067 seconds.
Both times are separate from execution. The CLI source commit is `bad5c08242da3585a2d03d3b8b1dd2135b724a90`.
Its SHA-256 is `72c02cbf3a548c67ebb2d119e63d8b03917b21c7a4bef37edf84874644d4b029`.
The current reviewed extension set has twenty files. Upstream runtime architecture remains intact.
Every acceptance command remains bounded at 115 seconds. No full-repository or whole-batch cold benchmark ran.
These measurements establish recorded-input parity, not a performance improvement.

The [native receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-render-fstrings-native-contracts.json.gz) contains source/native observations, controls, failures, hashes, commands, and its validator.
The inventory contains 79 drafts and 21 absent translations. Full migration acceptance remains false.
Shared registry/scope assertions, production registration, CLI/enforcement, and persistent workspace acceptance remain open.


## 2026-11-08: Port raw render strings and take the current Final shape resolutions

The 80th native draft is `render_raw_strings`. All five original tests run unchanged, including both finding positives.
The direct harness retains ten complete API/property observations. The initial translation failed both finding positives and the render-method count.
Commit `75b22f6cf` records that red stage. The final replay passes all five tests and all ten observations.

The rule has three finding types: `string_literal`, `template_call`, and `static_call`.
The source mentions `str_concat` in a DTO comment but never emits it. The translation does not add that finding type.
The outermost active render function owns findings in nested functions and classes.
Only that outer function's docstring is exempt. Nested function and class docstrings remain visible to the source collector.
Raises and asserts suppress ordinary literals, while special constructor calls still report there.
Allowed call names exempt direct positional literals. Keyword literals remain visible because their AST parent is a keyword node.
`Template` requires a bare name. `SqlFragment.static` requires a bare `SqlFragment` receiver.
Neither detector resolves aliases. The first special-call argument is suppressed as an ordinary literal, preserving the collector's identity and visit rules.

Ordinary previews use Python string repr, sliced to fifty characters. Special-call previews use decoded text, truncated after forty characters.
Commit `2e99c7aa8` exposes the existing native printer through the pure `python_repr` text functor.
The function shares quote choice, printable-character tables, and control escapes with the canonical Python printer.
It takes text, with no AST lookup or Python process. The engine probe and focused native test retain the missing-function failure and passing contract.
The rule uses existing literal pieces, byte starts, visit ranks, and typed AST relations.
Both render rules share their context generator. Regeneration preserves the earlier f-string rule's exact bytes.

The first 669-case API replay passed. Reporter replay exposed a literal code label: `.rr` did not interpolate `{violation_type}`.
Three clauses with fixed codes express the source behavior directly. No additional engine capability is required for that shape.
The final reporter replay passes all 169 cases. The policy maps the source warning behavior to log mode and info findings.

The supplemental corpus contains 669 API cases, including 285 detector positives and three expected exceptions.
It covers 160 file analyses, 161 source analyses, 157 collectors, and 157 statistics collectors.
It also covers fifteen directory scans, ten name probes, eight formatting cases, and the cleanup-hint helper.
The reporter corpus contains 169 cases, including 97 positives.
Every source/native comparison retains complete values, traversal order, duplicates, preview text, and exception arguments.
The corpus covers all three finding types, special-call argument shapes, allowed names, nested scopes, adjacent literals, and f-string constant pieces.
Both report formatters run in Datalog, including stable type-frequency ordering and cleanup hints.

Fifteen controls independently disable finding types, statistics, count properties, repr, formatting, hints, source gates, names, or exemptions.
Each enabled witness passes. Each disabled output fails the corresponding original, API, or reporter assertion through successful native execution.
The exemption controls also reject added false positives on the source's clean cases.
The host handles filesystem discovery, supplied AST columns, and passive DTO transport. Rule decisions execute in Datalog.

Final shape advanced during this work. Merge `2ffa7a028` takes `044449ca7`, including combined-rule planning and cycle-analysis resolutions.
Merge `0b094980e` takes `07a6b4171`, including renewed memo reuse and session behavior.
The complete rule corpus, controls, and focused regressions were replayed after the second merge.
The current reviewed native extension set has twenty-one files. The upstream executor, scheduler, typing, and content-store resolutions remain intact.

An initial build receipt sampled a moving Final branch tip that its source commit did not yet contain.
The corrected historical receipt records the actual adopted ancestor and preserves the original incorrect metadata separately.
The build script now reads the adopted pin and requires both adoption and current local-tip ancestry before building.
Acceptance uses the final frozen root binary. Earlier complete runs through `044449ca7` remain historical evidence.

All 91 reader checks and 85 harness tests pass. The previous f-string rule passes its five original tests and a focused reporter shard.
The focused native checks pass 101 test executions. They include the new repr contract and both upstream memo/session regressions.
Two upstream rule-generation helper tests remain ignored.
A function-test filter initially selected zero tests. The corrected filter executes all twelve string tests; only that nonempty run is counted.
The prepared-program group failed both parallel and serial execution because its cache lookup can select another test's program.
The comparison then differs by an empty `input_error` fact entry. Each of the three original tests passes in a separate process.
The receipt retains both failed groups and all isolated passing executions. No runtime change suppresses the failure.

The final CLI build took 28.885 seconds. The standard-feature unit build took 74.026 seconds, separately from execution.
The CLI source commit is `4c1b6c82583fdd307ecbb9142eebaca259fe7d31`.
Its SHA-256 is `6bf5243f886ed3c3ff03cd3e5fb1fcdbfba5691feafe43a0f4eb685ad8171e35`.
Every acceptance command remains bounded at 115 seconds. No full-repository or whole-batch cold benchmark ran.
These results establish recorded-input parity, not a performance improvement.

The [native receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-render-raw-strings-native-contracts.json.gz) contains source/native observations, controls, failures, commands, hashes, and its validator.
The inventory now contains 80 drafts and 20 absent translations. Full migration acceptance remains false.
Shared registry/scope assertions, production registration, CLI/enforcement, and persistent workspace acceptance remain open.

# 2026-11-08: Check Final shape adoption before further migration

The fetched Final rerail shape tip is `07a6b4171d8ca6aea400beefee04fabdbcc979f8`.
The local sibling has the same tip. This branch already includes that commit through merge `0b094980e`.
No new merge or engine source change was necessary for this checkpoint.

The frontend, rule lowering, prepared-program storage, session, lowering pipeline, transducer, content store, and Bitlex match that tip.
The engine is not entirely identical: 21 engine/build files contain reviewed generic extensions.
These include Python text APIs, typed input projections, diagnostics, traversal repairs, registrations, and their tests.
They also include the existing Boolean demand-normalization repair.
The receipt lists their exact paths and hashes. No retired engine supplied source, libraries, or a build.

Two gaps allowed acceptance without a current source audit.
The audit checked the local branch, but ignored a newer fetched remote tip.
The native assertion worker also started without an alignment audit.
Regression tests first demonstrated both gaps. Native suites now audit the active root before they start their worker.
Both retired engine paths also have rejection tests for executable selection.

All 88 migration tooling tests pass. All five original raw-render tests pass again, including positives and ten complete API/property calls.
The frozen executable still matches its recorded root source trees. No rebuild was necessary because engine source did not change.
The in-progress 81st rule remains unfinished. This checkpoint does not claim full migration acceptance.

Evidence: [Final shape source audit and native assertion replay](../gate/data-layer/fixtures/host/data-layer-port/rr-final-shape-07a6-acceptance-guard.json.gz).

## 2026-11-08: Port SQL regex classification on the current Final engine

The 81st draft is `regex_sql_parsing`. Engine source still matches the existing root build receipt.
Final shape remains at `07a6b4171`. No engine file changed for this port.

All 125 original tests pass with all setup, call, and teardown phases present.
The replay preserves all 128 API/property observations, including 14 positive file checks.
The framework test also preserves the two private checker calls and the reporter result from its shared AST traversal.
The collector sends one complete AST argument batch to the native rule, then replays the recorded callbacks from those results.
It makes no native query per call node.

All 343 supplemental API cases pass. They contain 44 file checks, including 27 positives, and 254 classification cases.
All 64 reporter cases pass, including 41 positives. Seven dedicated witnesses cover every emitted category.
The rule preserves bare `re` receiver matching, first positional arguments, f-string placeholders, truncation, quoting, and traversal order.
It also preserves source discovery gates and reporter ordering.

Python and Rust disagree about Unicode word classes and case-insensitive matching.
The generator maps input characters to distinct representatives for Python word, decimal, whitespace, and other characters.
It handles the four special ASCII case-fold equivalents before that mapping.
The pinned indicator patterns are ASCII; generation refuses unsupported pattern alphabets or flags.
The Boolean search translation replaces the fixed negative percent-prefix assertion with a consuming prefix alternative.
The first lookaround approach failed against Final shape's explicit regex restriction. No engine workaround remains.

The custom argument graph initially failed the modern value-cycle check.
The test transport now supplies each AST occurrence's depth as mechanical metadata.
Native source execution uses the existing syntax relations. It does not execute Python or rebuild an older engine.
The source extractor's breadth-first API order and the framework collector's depth-first order remain distinct.

Additional supplied-fact tests exposed 20 DTO mismatches in the first adapter.
The adapter now preserves invalid-shape skips, category defaults, scalar conversions, exceptions, duplicates, and ordering.
These operations transport supplied finding DTOs; they do not classify source code.
All 34 transport/count cases pass, including a result that changes between property reads.
All count properties share one native aggregate response for identical category/severity rows.
A changed row sequence produces a new query.

Nineteen independent controls fail as intended through successful native execution.
Fifteen cover detector categories, predicates, extraction, metadata, and compiled-regex exports.
Four change one count output at a time. Original and reporter controls retain their complete mismatched results.
Both comment-strip and parameter-normalization positives are supplemental; the original classifier tests omit those positive categories.

All 92 policy/table files pass the reader check. All 88 migration tooling tests pass.
Regeneration preserves both this rule and the earlier f-string rule's bytes.
The engine did not change, so this delivery reuses the recorded CLI build and records no new compile time.
No whole-repository or whole-batch cold benchmark ran.

The original nine CLI cases run the source host protocol with native detectors and counts.
They mostly use clean trees. These assertions do not establish complete production CLI or ratchet acceptance.
Shared registry/scope assertions, production registration, CLI/enforcement, and persistent workspace acceptance remain open.
The inventory contains 81 drafts and 19 absent translations. Full migration acceptance remains false.

Evidence: [SQL regex source/native contracts and validator](../gate/data-layer/fixtures/host/data-layer-port/rr-regex-sql-parsing-native-contracts.json.gz).

## 2026-11-08: Grammar escape-hatch translation and native annotation values

The 82nd draft is `sql_grammar_escape_hatches`. Final shape remains at `07a6b4171`, including the fetched remote tip.
All development and builds use the active root. Neither retired engine changed.

All 165 original tests pass with their complete setup, call, and teardown phases.
The replay preserves all 134 observations, including 21 positive file checks, visitor state, exceptions, and captured output.
The nine CLI assertions retain their host protocol. Native relations provide classification, signature verification, counts, formatted reports, and return status.
No rule invokes Python.

The implementation follows two non-obvious source details.
`SAFE_PARAM_TYPES` is static, despite its discovery comment. Inheritance closes before aliases propagate.
The native rule keeps those stages separate. Directory discovery also preserves the source's full-path `test` substring behavior.
It does not apply that class-definition skip to an explicitly supplied file.

The original suite passed before the additional corpus did.
The 65 supplemental API cases exposed twelve missed non-string annotation literals and one missing syntax warning.
The tape contains the literal nodes, but the rule lacked their Python value representations.
The shared native AST producer now emits `annotation_constant_repr` for non-string constants within annotations.
It shares its demanded parse with the other Python text columns and skips constants outside annotations.
The rule reads typed rows. It adds no Rust detector or per-node callback.

The first value implementation preserved Ruff's source spelling for a large hexadecimal integer.
Python reports its decimal value. The printer now uses the existing bigint library for radix conversion.
The same correction applies to canonical annotation text. No new dependency or parallel evaluator was added.
The native regression checks nested bytes, floats, infinity, complex values, large integers, booleans, None, and Ellipsis.
It also checks that an assignment outside annotations contributes no row.

The source-call guard needed a diagnostic repair: CPython clears a profile callback when that callback raises.
The new contract rearms it before each test. The first failed run then exposes all 36 unported helper cases.
That failed run remains rejected because it records source calls; its other passing cases never count as native evidence.
Four subsequent failures had equal detector returns but different fixture-root string arguments.
The transport now normalizes an exact fixture-root string, and the source oracle was recorded again.

All 65 API cases now pass, including 39 positive results. All 17 production-reporter cases pass, including eight positives.
The reporter corpus tests the ceiling on both sides, deduplication before counting, inheritance across files, literal values, triggers, and scope.
Explicit `.severity` declarations preserve the source's `error` DTO value.
A path helper also exposed the existing SQL emitter's unquoted `join(...)` function name.
The rule uses scoped path concatenation instead. That SQL-emitter defect remains outside this delivery; no Linear ticket was created.

Eleven disabled-output controls fail as intended through successful native execution.
One initial control selected an attribute annotation rather than a forward reference.
The unchanged result exposed the wrong witness. The corrected control uses the recorded positive forward-reference case.

All 93 policy/table files pass the reader check. All 89 migration tooling tests pass.
The new native integration test and both canonical-printer tests pass: three tests, with none ignored.
The preceding regex SQL rule also passes all 125 original tests again on the new engine.

The final frozen CLI is `cli-f6f0fbc47f877092`, built from `97aada35a`.
Its build takes 10.816 seconds wall time; the standard-feature unit build takes 29.064 seconds.
The earlier value implementation's builds and failed comparisons remain in the receipt.
Compilation is separate from execution. No whole-repository or whole-batch cold benchmark ran.

The inventory contains 82 drafts and 18 absent translations. Full migration acceptance remains false.
Shared registry/scope assertions, production registration, CLI/enforcement, and persistent workspace acceptance remain open.
The direct original suite does not exercise every legacy helper entry point; the receipt does not claim exhaustive source-domain coverage.

Evidence: [Grammar escape-hatch source/native contracts and validator](../gate/data-layer/fixtures/host/data-layer-port/rr-grammar-escape-hatches-native-contracts.json.gz).

### 2026-11-08 — Batch decoding during exact scanner migration

World loading previously submitted one `SELECT sym_text` for every requested directory, file, and load argument.
The native atom arena already holds those values.
Loading now decodes each request batch directly from that arena and interns loaded paths as one batch.
A traced native test decodes one thousand atoms and requires zero SQL statements.

The scanner also uses one weighted partition batch over typed task, occurrence, path-key, and byte-size rows.
Forty-six recorded partitioner cases and thirty complete discovery cases pass.
Eight repeated runs in one workspace preserve exact results across worker changes, file changes, and threshold changes.
The [acceptance receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-weighted-discovery-acceptance.json.gz) retains the individual phase measurements.

The final CLI build took 13.450 seconds. The final unit build took 25.007 seconds.
These compile times remain separate from execution measurements.
No full repository cold run occurred, and these fixture results do not establish a repository performance improvement.

### 2026-11-08 — Survey report integration on Final library modules

The branch now includes Final `0ab7966c9` and its first library-module implementation.
Native text and JSON reports consume typed category, occurrence, and symbol rows.
A two-consumer module check preserves separate answers instead of combining the input scopes.
This fixture check does not establish a pack performance improvement.

The wider tests exposed a cache correctness defect before acceptance.
Cold AST-pattern matching passed, but a second process lost every positive match.
Stored programs omitted the pattern requests that source projection needs.
The repaired codec preserves those requests and recovers them from legacy rule rows.
Cold and repeated warm runs now retain the same positive and negative findings.

The repaired CLI build took 27.992 seconds. The repaired unit build took 41.053 seconds.
Compile time remains separate from execution time.
No full repository cold run occurred.
The [reporting receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-report-acceptance.json.gz) retains the fixture phase measurements and the failed cache experiment.

### 2026-11-08 — Equal-request sharing adopted between acceptance cohorts

Merge `60839bc48` adopts Final `2b494d4ef`, including request sharing by key.
A three-consumer fixture produces two request keys: two equal requests share one cone, and the different request retains its own cone.
All three answers match their separate runs.
The profile records the two keys explicitly, with both copy-comparison switches unset.

The updated CLI build took 39.249 seconds. The unit build took 66.225 seconds.
These compile times remain separate from execution measurements.
The [stage-two receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-report-stage2-acceptance.json.gz) retains the fixture profiles and exact source hashes.
This check proves request sharing and isolation for the fixture. It does not establish a full repository performance improvement.

### 2026-11-08 — Survey CLI oracle and failing native observation

The pinned Python CLI produces 28 complete survey observations.
They record output bytes, exit status, directories, graph files, dynamic roots, and all analysis categories.
The cases include positive findings, empty output limits, Unicode exclusions, installation defaults, empty directory arguments, and absolute scan paths.
Absolute scan paths expose a source distinction: parsed graph modules can suppress the fallback scan of invalid files.
Empty CLI directories also differ from the graph constructor's public-directory fallback.

The native dynamic-import case runs successfully but fails parity: CLI discovery, output, roots, and exit status are absent.
The source and red receipts retain this starting point. No CLI completion is claimed.
Five transport and acceptance tests pass, with eight subtests.
The root engine still adopts Final `2b494d4ef`. No engine code or build changes occur in this step.

### 2026-11-08 — Native survey CLI implementation and integration regressions

The native CLI relations now derive scan directories, dynamic-import roots, normalized exclusions, reports, and check status.
Loading uses unordered membership. Ordered arguments and graph assembly remain separate consumers.
Only active discovery callers request host metadata.

The CLI cases exposed two defects in earlier work.
Excluded result rows left ordinal gaps, so text reports omitted surviving findings despite correct counts and analysis.
The report bridge now assigns dense positions after filtering.
The module dictionary also retained syntax-invalid files. Graph modules now require successful parsing.
The saved integration failures retain both defects and their observable results.

The expanded source corpus has 29 cases, including a module-collision counterexample.
Twelve changed-clause controls require successful execution with different results. Two additional controls require explicit refusal without output or success status.
The complete regression cohort remains pending at this commit.
Deletion mode, production CLI registration, registry scope, and full migration acceptance remain open.
No engine code or build changes occur. This step makes no repository performance claim.

### 2026-11-08 — Survey CLI regression cohort recorded

The committed implementation passes all 29 CLI observations and 14 controls.
The same bytes pass 106 producer cases, 130 analysis cases, 30 discovery cases, and 15 scope cases.
All 54 reporting cases, four original linter tests, and eight persistent-workspace steps also pass.
All 100 readers pass in one batched invocation. All 153 tooling tests pass, with 112 subtests.

The 64 cohort commands run with three workers. Every command finishes within its 115-second bound.
The longest command takes 34.642 seconds wall time. This is not CPU time or a repository performance result.
No build occurs. The cohort reuses the registered Final-stage-two CLI artifact.

The [CLI acceptance receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-survey-cli-acceptance.json.gz) retains full results, source observations, program snapshots, harness snapshots, commands, and its validator.
It also retains the tooling tally from the recorded command output.
The earlier red receipts remain unchanged.
Deletion mode, import cleanup, CLI/registry integration, eleven absent translations, and full migration acceptance remain open.

### 2026-11-08 — Raw-execute native authoring and workspace boundary failures

The raw-execute detector first fails both original positive tests through successful native execution.
Both clean tests pass. The saved red receipt retains all four tests and five complete observations.
The native call, definition, text, and raising-stub implementation then passes all four original tests.
A first supplemental cohort passes 132 source-derived API cases, including positives, ordering, duplicates, and nested function ownership.

The expanded offline corpus records 187 API cases and 31 reporter cases.
Fifteen reporter cases contain findings. They include Unicode layer names, compatibility wrappers, and paths containing colons.
Native fact conversion preserves the source's split-at-two-colons behavior and Unicode digit handling.
These expanded cases still need the final native acceptance cohort.

Actual workspace checks expose two frontend exclusions that the source plugin does not apply: `.ruff_cache` and `.pytest_cache`.
The new typed FileScopeSpec discovery library requests those files explicitly and prunes only the source's ignored trees.
The first sixteen workspace cases pass after this change.
A separate saved failure proves the source's first-character newline restriction on layer names.
Four verifier tests pass, including rejection of a detector that drops all fifteen reporter positives and rejection of changed enforcement.

Final advances to `32b16e4fe` before final acceptance.
This checkpoint preserves the authored rule and evidence before the engine merge.
The earlier observations use the registered Final-stage-two artifact. They do not certify the newer engine.
The migration inventory now contains 90 drafts and ten absent translations. Full migration acceptance remains open.
### 2026-11-08 — Raw-execute acceptance on the merged Final engine

The branch adopts Final through `d60c1a832`.
The final CLI and unit artifacts use root engine source `fa47dda79`.
Their separate builds take 22.716 and 46.820 seconds.

The Final merge exposes two generic engine defects.
World dependency snapshots omit demanded metadata and host values.
New failing tests prove stale file sizes and changed or absent host values.
The repaired snapshots hash each demanded projection and share one directory read.

Combined policies also rename their filesystem requests away from the engine dispatcher.
An implicit source walk hides this defect for ordinary files.
A hidden-cache positive exposes the missing request.
Canonical request bridges preserve each producer namespace and deliver its requests to the existing world dispatcher.
A functional test covers both policy and library requests without primary source documents.

The final cohort passes all four original tests and five complete observations.
It also passes 187 supplemental API cases, 31 snapshot reporter cases, and 31 actual workspace cases.
Fifteen workspace cases contain findings.
Fourteen changed-clause controls produce mismatches through successful native execution.
Three combined-scope cases preserve complete diagnostics and duplicate occurrences.
Eight persistent-workspace steps for rule 89 pass on the same engine.

The acceptance verifier initially mishandles the engine's JSON-lines diagnostic output.
Three regression tests now cover full fields, Unicode escapes, and duplicate ordinals.
The Unicode mutation also moves to the active scope filter after the scope repair.
The earlier failed verifier runs remain in the receipt.

All 101 policy/table readers pass in one invocation.
All 136 focused native tests pass.
Three native report or translation helpers remain ignored by their existing annotations.
All 160 tooling tests pass, with 124 subtests.
The external Ruff and SQLFluff fixture suites remain outside this focused cohort, as previously authorized.

The [acceptance receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-raw-execute-native-contracts.json.gz) retains complete observations, program bytes, harness bytes, builds, logs, commands, and its validator.
No full repository cold run occurs. This cohort makes no performance claim.
The raw-execute callback-collector/factory/CLI and registry/lifecycle contracts remain open.
Rule 89 deletion and production integration also remain open.
The inventory remains 90 drafts and ten absent translations. Full migration acceptance remains open.
### 2026-11-08 — Native raw-execute CLI and persistent consumers

The native rule now implements CLI file selection, ordered violation strings, and check status.
The source CLI and plugin use different exclusion sets.
The CLI preserves production-root order and Python Path component order within each root.
Its text findings precede AST findings and survive syntax errors.
Only AST findings pass through the source fact formatter.

The first native CLI shard fails all ten selected cases.
The repaired CLI passes 38 source observations.
Two further source cases expose layer-name and absolute-ancestor exclusions.
The expanded corpus contains 44 cases, including file aliases, root symlinks, nested directory symlinks, and unusual names.
All 44 cases pass after those repairs.
Nine changed-clause controls produce complete mismatches through successful native execution.

Thirteen steps reuse one workspace and output cache.
Both the CLI and plugin match their source recordings on all 26 runs.
The steps cover file deletion, restoration, same-path stub edits, hidden caches, Unicode roots, syntax failure, and unchanged repeats.

The final rule bytes also pass all four original tests and five complete observations.
They pass 187 API cases, 31 snapshot reporter cases, and 31 actual workspace cases.
All fourteen detector controls and three combined-scope cases pass again.
All 101 readers pass in one invocation. All 163 tooling tests pass, with 127 subtests.
The 38 cohort commands finish within their 115-second bounds.
The same immutable engine supplies the preceding 136 native regression results.
No engine build or full repository cold run occurs in this CLI cohort.

The [CLI receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-raw-execute-cli-acceptance.json.gz) retains source observations, results, controls, update traces, consumed programs, logs, commands, and its validator.
It records Final engine base `d60c1a832` and the documentation-only merge `06b9b015d`.
Final advances to `49035cc90` during the cohort.
The completed cohort retains its actual engine identity before that next merge.

The next missing rule, imperative-private-functions, now has a complete offline source recording.
All 144 original tests pass, with 129 API observations, including the collection-time directory call.
Its native port remains absent.
The inventory remains 90 drafts and ten absent translations.
Raw-execute callback/factory transport, shared registry integration, rule 89 deletion, and full migration acceptance remain open.
### 2026-11-08 — Final 490 adoption and complete raw-execute rerun

The branch merges Final through `49035cc90`.
It adopts in-memory exports, native computed steps, and the revised crossed-atom estimates.
The source conflict retains Final's arithmetic functions and the existing Python integer functor.

The merge first exposes an aggregate interface mismatch.
The initial string-join test passes over native stored rows, which retain every column.
The stronger test requires a projected table input through the production atom-native runner.
Its valid fixture supplies stored column statistics.
An earlier probe uses the older SQL reference runner and rejects the new opcode.
The production probe then panics: it reads column three from a row with two columns.
The repair maps both the value and ordering keys through the projected columns.
The same regression passes after the repair.

The broader cohort exposes a separate stored-program failure.
The codec omits explicit empty fact tables, including `input_error`.
A small marker preserves those tables without extra rows for ordinary nonempty tables.
New checks preserve empty tables, zero-arity rows, and legacy records.
The original prepared-program assertion passes after the repair.
Both [projection](../gate/data-layer/fixtures/host/data-layer-port/rr-final490-string-join-projection-red.json.gz)
and [codec](../gate/data-layer/fixtures/host/data-layer-port/rr-final490-empty-facts-red.json.gz) failures remain recorded.

The registered engine uses source `4721b1d81` and Final `49035cc90`.
Its final unit and CLI builds take 51.847 and 23.435 seconds.
The build receipts record compilation separately from execution.
All 153 focused native tests pass, including the new export and codec checks.
Three existing report, translation, or benchmark helpers remain ignored by their annotations.

The new engine passes all four original raw-execute tests and five complete observations.
It passes all 187 supplemental API cases, 31 snapshot cases, 31 workspace cases, and 44 CLI cases.
Fifteen workspace cases contain findings.
All fourteen detector controls, nine CLI controls, and three combined-scope cases pass.
Both consumers match the source through thirteen persistent-workspace steps.
Eight rule-89 update steps pass on the new engine too.
All 101 policy/table readers and 163 tooling tests pass, with 127 tooling subtests.

All forty acceptance commands finish within 115 seconds.
The longest takes 29.905 seconds wall time. This is not a CPU benchmark.
The six focused native commands also meet their individual limits.
The [Final acceptance receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-raw-execute-final490-acceptance.json.gz) retains complete results, source recordings, programs, harness files, commands, logs, builds, and its validator.
No full repository cold run occurs.

The next rule's [ordered source recording](../gate/data-layer/fixtures/host/data-layer-port/rr-imperative-private-functions-ordered-source.json.gz)
preserves 144 original tests and 129 API observations.
It also retains raw caller iteration order and guards imported graph algorithms.
That native translation remains absent.
The inventory remains 90 drafts and ten absent translations.
Callback/factory transport, shared registry integration, rule 89 deletion, and full migration acceptance remain open.

### 2026-11-08 — Imperative analysis and report checkpoint

The next draft implements native signature seeds, direct execution, breadth-first propagation, and exact text/JSON reports.
All 144 original tests pass across the focused runs, after a CLI help repair.
The real-file corpus still fails one of 32 cases: reversing definitions exposes Python caller-set ordering.
The [failure receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-imperative-private-functions-order-red.json.gz) preserves the complete ordered difference.
These checks precede the final source integration bytes. Final acceptance, plugin scope, production CLI, and lifecycle checks remain open.
The inventory contains 91 drafts and nine absent translations. This rule is not complete.

### 2026-11-08 — Final 6ae merge and imperative ordering

Final `6ae30c48a` is merged. The native set-order operator passes five tests, including Unicode hashing, duplicate insertions, and growth to 50,001 values.
The 96 frontend tests pass. The codec suite exposes duplicate AST-pattern metadata after the merge.
The repair keeps one upstream writer and schema, while retaining legacy request recovery.
The expanded real-file source recording contains 37 cases and actual plugin planning, extraction, and complete reporter results.
Two [production mismatches](../gate/data-layer/fixtures/host/data-layer-port/rr-imperative-production-red.json.gz) expose missing extractor fields and a missing trigger-file gate.
Those repairs now await a new engine build and acceptance rerun. Full migration acceptance remains open.

### 2026-11-08 — Imperative real-source acceptance on Final 24d

Final `24d05da80` is merged and registered. Its root CLI and unit builds take 36.588 and 98.763 seconds.
All 144 original imperative tests and 129 complete observations pass, with no semantic fallback or missing results.
All 39 real-file cases match ordered findings, complete extractor fields, plugin metadata, and complete reporter lists.
All twelve persistent-workspace steps pass, across 24 runs sharing one cache.
The new operator preserves normal and legacy insertion phases and fixed-seed Python caller-set iteration.
Native producer ordinals preserve source reporter order across diagnostic clause groups.
The initial transport mistakenly reused a variable for two result lists. Its captured failure remains explicit; the corrected transport passes.
The [acceptance receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-imperative-source-acceptance.json.gz) contains programs, source recordings, harnesses, builds, and results.
All 127 focused native tests pass, plus 163 tooling tests and 127 tooling subtests. All 102 policy/table readers pass.
Two existing annotated native helpers remain ignored. No external fixture suite or full repository cold benchmark runs.
Real-source CLI and mutation-control acceptance remain open. Full migration acceptance remains open.
The next draft, serverplan-bypass, has seven recorded source tests. Its native scaffold passes three controls and fails four actual positive assertions.
The [red receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-bypass-scaffold-red.json.gz) retains those failures.
The inventory now contains 92 drafts and eight absent translations. Draft count is not completion coverage.

### 2026-11-08 — CLI and bypass source gaps, still under repair

The bypass detector now passes all seven original tests, including the four previously failing positives.
Its 33-case real-file cohort exposes missing production bootstrap, explicit test-path filtering, and a Unicode trigger failure.
The Unicode probe proves that inferred constant domains omit literal facts when the same relation also has rules.
The engine adds an unsound text prefilter and rejects a file whose literal fetch trigger is present. A generic regression and repair are next.
The imperative CLI has a complete 26-case source recording. Its first native shard fails for empty directory selection. The initial inventory-leak diagnosis is later rejected.
The next investigation must identify the source fallback precisely. Neither cohort is complete.

### 2026-11-08 — Final d8 imperative and bypass parity checkpoint

The branch includes Final `d8fad367b`, including stored combined programs and prepared execution splits.
The registered root CLI uses engine source `c3a50416c`. Compilation takes 17.142 seconds for the CLI and 35.871 seconds for unit tests.

Imperative acceptance passes all 144 original tests and 129 complete API observations.
All 39 real-file cases pass, with 25 producer positives and 23 plugin positives.
All 27 real-source CLI cases pass, including 24 nonzero check results.
All twelve persistent-workspace steps pass across 24 runs sharing one store.
The source oracle relocates only the bound repository path for its physical CLI fixtures.
Native execution invokes no Python detector or graph implementation.

Bypass acceptance passes all seven original tests and eight complete API observations.
All 37 real-file cases pass, including 25 scanner positives and 27 reporter positives.
These cases include Unicode names, decorators, nested scopes, source gates, signatures, duplicate calls, and discovery exclusions.
All thirteen persistent-workspace steps pass across 26 runs sharing one store.
Six imperative producer controls, eight CLI controls, and six bypass controls detect disabled native rules through complete result mismatches.

The text-domain regression fails before the repair and passes afterward.
The old inference returns only rule-produced constants and drops literal facts from mixed relations.
The repair includes both producers and rejects bounded inference for external inputs.
The native set-order producer preserves the source's caller-set iteration without a Python runtime.
A single SQL window now assigns vertex ranks instead of repeated preceding-row counts. No performance improvement is claimed from these fixtures.

The first CLI inventory diagnosis was wrong. SymbolGraph treats an empty directory list as `public`.
The unnecessary inventory marker is removed. Both legacy and modern-layout fallback cases now pass.
Final shares existing physical documents without adding loaded aliases.
The bypass rule accepts those documents and keeps discovery membership authoritative.
The update ladder then exposes a separate single-file scope leak.
The API now joins its explicit input path, so a loaded neighbour cannot inherit the requested filename.

All 143 focused native tests pass, with two existing annotated helpers ignored.
All 163 tooling tests and 127 tooling subtests pass. All 103 policy/table readers pass.
Commands remain bounded at 115 seconds. No full repository cold run occurs.
The previously authorized external fixture-suite exclusion remains unchanged.

The [checkpoint receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-imperative-bypass-finald8-acceptance.json.gz) binds recordings, complete results, consumed programs, harnesses, builds, controls, and update ladders.
Its validator checks assertion coverage, complete result equality, engine identity, and consumed program hashes.
The inventory contains 92 drafts and eight absent translations. Draft count is not completion coverage.
The bypass standalone CLI and legacy callback adapters remain open, along with shared registry integration and earlier recorded gaps.
The next absent translation is serverplan-steps. The 100-rule migration is not complete.

### 2026-11-09 — Final integration and partial ServerPlan step contracts

The branch takes Final through `4dd2dec74`, including observed filesystem preparation and CPU/GPU placement.
The root CLI build takes 81.655 seconds. The unit build takes 304.179 seconds, including shared target contention.
Compilation remains separate from rule execution. The acceptance engine is frozen as `f82abb72f58bf9e5`.

The merge exposes two stale integration assumptions.
The library request fixture now uses Final's library include resolution.
Metadata consumers retain one directory snapshot through the observer. Directory-only consumers do not construct child metadata.
All 130 focused engine tests pass. Two existing helper tests remain ignored.
All 171 harness tests pass.

The complete pinned serverplan-steps source suite passes all 164 tests and records 212 API observations.
Its test IDs exactly match the independent original inventory.
Python 3.13 TypeAlias.name is an AST child. The shared transport now retains that child instead of storing an AST object as text.

The new native draft defines 21 of 31 detector functions.
Its partial acceptance passes 95 original tests and 155 complete observations, including 30 positive detector observations across nineteen functions.
Another thirteen source/native cases cover the MISC_RUNTIME check, including seven positives, Unicode identifiers, and repeated targets.
The remaining function intentionally returns no findings in the pinned source: obsolete runtime-builder kind/name validation.
No Python detector runs in native acceptance.

Twenty-check acceptance first passes 83 tests, but one shard exceeds its 115-second limit.
That live-source case calls three APIs for each of five files.
A batched observation path now shares typed native results across those APIs.
The case passes all fifteen observations with five native runs in 68.51 seconds.
Transport regressions reject reuse after source, path, configuration, or supplied-AST changes.
This result measures the test API path. It is not a repository performance baseline.

Native parity also exposes a signed-message defect: the shared library drops named empty allowlists.
Explicit list names now preserve those keys. Eight comparisons match Python's exact message bytes and signature status.
These cases change signed content, not the signature. The tampered-signature API assertion remains untranslated.

A negative recursive name fallback initially prevents execution. The repair uses nonrecursive shape tests.
Source review also replaces an incorrect remediation string with the pinned text.
SQL planning errors now identify their rule head.
That diagnostic isolates a lost function-name binding in the authority-return producer; an explicit definition anchor executes correctly.
Disabled branching and callback consumers each fail their original positive test, with no fallback.

The [partial checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-final4dd-checkpoint.json.gz) records exact programs, observations, builds, controls, and remaining cases.
The [initial red recording](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-scaffold-red.json.gz) retains the complete source observations and initial failures.
The checkpoint explicitly lists 69 original tests outside native coverage.
Ten detector functions and seven scanner, signature, or CLI entry points remain untranslated.
Production reporting, discovery, parameter-shape acceptance, update tests, and full serverplan-steps acceptance remain open.
Seven other translations remain absent. Earlier migration gaps remain open.

Final advances to `e3dbedd64` after this immutable acceptance run starts.
The live alignment audit correctly requests another merge. This checkpoint continues to identify its actual `4dd2dec74` engine.
No full repository cold run or external fixture suite runs.

### 2026-11-09 — Parameter-shape TDD after the checkpoint

Final `e3dbedd64` is merged. Its per-rule fanouts replace shared observations that can misprice another rule's document lookup.
The root CLI and unit builds take 41.124 and 121.411 seconds.

Additional source cases expose two missed positives: a positional-only separator and a comment before a call argument.
Both fail before their repairs. Callback positions now exclude separators. Call argument positions now exclude comments and keywords.
The same ladder retains keyword-only carrier and authority behavior.
Duplicate keyword cases prove that grammar dictionaries use the last value. Helper keyword lookup uses the first value.
All nineteen source/native cases pass, including eleven positives.

The [TDD receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-step-arguments-tdd.json.gz) binds each red and green result to its exact consumed program hash.
The source recorder now retains engine identity and program text before comparison, including failed runs.
Original-suite regression after these repairs remains to run. The earlier 95-test checkpoint identifies its earlier program exactly.
Serverplan-steps and the full migration remain incomplete.

### 2026-11-09 — Runtime-ref summaries and original-suite regression

The draft now defines 24 of the 31 detector functions.
The three new functions track consuming helper returns, refless runtime-ref helpers, and helper-anchored post-ops.
Their SQL clauses preserve last-assignment lookup, dictionary-value traversal, helper fixed points, and source finding order.

The source's alias-cycle result depends on its starting name.
The translation retains a separate seen-name state for each expression path.
The engine initially rejects that finite value cycle because it cannot infer the string-membership bound.
A finite counter domain, derived from the number of local bindings, supplies an existing supported termination proof.
There is no fixed depth limit and no new native engine implementation.

Duplicate runtime-ref keywords expose another defect: the draft reports both values instead of the dictionary's last value.
The captured red result has an extra finding and an incorrect line. The repaired lookup matches the source.
All twenty-five added source/native cases pass, including sixteen positives.
These cases include alias cycles, short-circuit behavior, helper chains, and opaque dictionary keys.

Original-suite regression passes 102 tests and 162 complete observations on Final `e3dbedd64`.
Thirty-seven positive detector observations cover twenty-two functions.
The supplemental corpus covers MISC_RUNTIME; obsolete kind/name validation remains empty by source definition.
All twelve original-suite shards finish within 115 seconds. No semantic fallback or missing result occurs.
A disabled runtime-ref consumer fails the original recursive-wrapper positive test as expected.
All 130 focused engine tests and 171 harness tests pass. Two existing engine helpers remain ignored.

The [new checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-finale3db-checkpoint.json.gz) records these results and the remaining 62 original tests.
Seven detector functions and seven scanner, signature, or CLI entry points remain untranslated.
Production reporting, discovery, complete controls, update acceptance, and global migration acceptance remain open.
The inventory still contains seven absent translations. The new draft is not a completed port.

### 2026-11-09 — Seven remaining detector functions

Final `8bd924c72` is merged. Root CLI and unit builds take 28.802 and 84.861 seconds, respectively.
The draft now defines all 31 detector functions. Seven scanner, signature, and CLI entry points remain open.

The seven new functions fail 27 original tests before translation and pass all 27 afterward.
They cover materializer shaping, literal dispatch, role contracts, callable registries, grammar-less refs, static exceptions, and legacy callbacks.
An additional duplicate-materializer case fails before its repair and preserves both identical findings afterward.
All 45 supplemental source/native cases pass. Comparisons retain complete findings, order, and duplicates.
All 171 tooling tests pass.

The broader detector regression is not accepted: ten shards pass, but two exceed the 115-second command limit.
Those timeouts remain unresolved. The earlier 102-test checkpoint remains the last assembled original-suite checkpoint.
The [TDD receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-seven-detectors-tdd.json.gz) records the direct red/green results and timed-out regression commands.
No repository cold run occurs. The full migration remains incomplete.

### 2026-11-09 — Scanner APIs and demand-bounded helper summaries

The native draft now translates `_scan_source` and `_scan_file`.
The source scanner retains caller-text gates independently of the supplied AST. Native phase ordinals preserve check order.
File scanning uses the root engine's parser and preserves the complete parse-error finding.
A recorded failure proves that embedded annotation errors must not become enclosing-file errors. Primary-document selection repairs that defect.

The [checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-scanners-checkpoint.json.gz) audits 136 original tests and 200 complete observations.
Its detector portion passes 129 tests, including 58 positive detector observations.
Its scanner portion passes seven tests and eleven observations, including the five-file ratchet assertion.
All detector shards finish within 115 seconds. The two previously timed-out shards finish in 71.398 and 76.958 seconds.

The test adapter previously requested all 31 checks for tests that called only one or three.
It now batches the original test's requested API names. Expected findings never enter rule options.
Global function summaries run only when a requested check consumes them.
The recorded transport regression rejects cache reuse across changed demand batches.
The checkpoint auditor accepts pytest's no-tests exit only when the independently derived shard is empty.
All 173 tooling tests pass. The root engine passes 99 rule tests and two parser tests; two existing helpers remain ignored.

The supplemental scanner corpus still finds a native diagnostic spelling defect in indented-block errors.
Ten independent parser cases record nine mismatches against Python 3.13. The shared formatter repair remains next.
Five signature, blocker, legacy-scan, and CLI entry points remain untranslated.
Production, discovery, controls, updates, seven absent rules, and global migration acceptance remain open.

### 2026-11-09 — Final engine merge and scanner diagnostic repair

Commit `852e1c343` merges Final `134521978`, including its relational laboratory. No conflict requires a parallel engine implementation.
Commit `bb6d6fd14` repairs the root Python diagnostic formatter.
It preserves CPython's keyword quotes, statement labels, and class-definition wording for missing indentation.
The recorded ten-case parser comparison has nine mismatches before the repair and none afterward.
The shared parser oracle now includes 31 cases. Native rules invoke no Python interpreter.

The root CLI and unit builds take 26.211 and 57.935 seconds, recorded separately from acceptance.
The [Final checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-final134-checkpoint.json.gz) binds their exact binaries and source trees.
Its combined replay passes 136 original tests, 200 complete observations, and 164 native calls.
The replay retains all 58 positive detector observations. Its slowest shard takes 74.217 seconds.
All 45 detector cases and thirteen scanner cases pass on this engine.
All 101 focused engine tests and 173 tooling tests pass. Two existing focused engine helpers remain ignored.
No full repository cold run occurs.

The checkpoint individually lists all 28 original cases outside native acceptance.
Signature verification, both blocker collectors, legacy scanning, and CLI reporting remain untranslated.
Production, discovery, mutation controls, updates, the seven absent translations, and earlier migration gaps remain open.
These results do not complete ServerPlan steps or the 100-rule migration.

### 2026-11-09 — Exact allowlist verification

The signature API now uses native `.rr` rules, the shared signed-message library, and the existing public-key verifier.
The generic `ed25519_error` functor exposes the verifier's result text. It never signs artifacts or reads private keys.
All 72 API cases pass, including valid signatures, unsigned markers, changed lists, Unicode, noise, padding, and malformed inputs.
Both original signature tests pass with their complete return values and stderr.

A single SQL batch checks 1,110 recorded decoder/verifier cases. The test fails before the new functor exists and passes afterward.
The first implementation also exposes an existing decoder discrepancy: it stops at completed padding and ignores later data.
The pinned runtime is Python 3.13.18, whose [decoder](https://raw.githubusercontent.com/python/cpython/v3.13.18/Modules/binascii.c#L365) continues through interior padding.
The repair matches that runtime and retains bounded signature storage. Expected results remain unchanged throughout the repair.
All sixteen string-function tests and 99 rule-engine tests pass. Two existing rule-engine helpers remain ignored.
The final root CLI and unit builds take 15.498 and 27.118 seconds.

The [TDD receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-signature-tdd.json.gz) retains the original failures, complete API results, parser corpus, and build identities.
Both blocker collectors, legacy scanning, and CLI reporting remain untranslated.
The complete original-suite replay and production acceptance remain open, along with seven absent translations and earlier migration gaps.

### 2026-11-09 — Blocker collectors, shared definitions, and logical load answers

Both blocker collectors now execute native `.rr` policies. Filesystem discovery uses the engine's existing observer and load requests.
The collector policy imports the same AST and candidate definitions as the full step policy.
Static API routing selects that consumer. The parity auditor checks each declared policy and rejects conflicting shared-library hashes.

Positive cases expose two detector defects. Side-effect exceptions require `path:class` keys, and duplicate class names share candidate membership.
The repairs retain duplicate findings in the direct checker APIs.
Additional cases preserve decorators, hidden/test/temporary files, overlapping roots, relative discovery, and a separate read root.
Direct checks preserve outside-root `ValueError` results. File scanning converts those errors to `PARSE` findings; collectors ignore those failed files.
An error in one batched checker no longer suppresses an independent check's result.

A file-link case exposes a generic engine defect: physical-load deduplication discards a later logical request's answer.
The engine now answers every accepted logical request and parses each new physical document once.
Dependency snapshots also record the requested path's target. A test rejects a changed symlink target even when both files have identical bytes.
All seven filesystem tests and 99 rule-engine tests pass. Two existing rule-engine helpers remain ignored.

The first repository-fixture attempts exceed the 100-second native timeout in the debug build.
An external `sample` report and relational-lab captures show planning work and unrelated producers from the full rule program.
The focused collector removes those producer dependencies. Its debug execution still exceeds the original fixture limit.
The optimized root build takes 478.732 seconds. Both original collector tests then pass over all 546 files in about 31.7 seconds per command.
Compilation remains separate from acceptance timing. Diagnostic captures use different subsets and include instrumentation overhead.

The [checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-collectors-checkpoint.json.gz) audits 140 original tests and 204 complete observations through 168 native calls.
All twelve combined shards pass within 115 seconds; the slowest takes 97.880 seconds.
All 45 detector cases, thirteen scanner cases, 72 signature cases, fifteen collector/path cases, and 177 tooling tests pass.
The checkpoint retains all 58 positive detector observations and names the 24 original cases outside native acceptance.
Legacy scanning, CLI reporting, configuration, production acceptance, controls, updates, seven absent translations, and earlier migration gaps remain open.

### 2026-11-09 — Legacy scan and CLI parity

The native legacy scanner now preserves the source's two passes, callback occurrences, and category order.
It uses the last function definition by simple name, including nested definitions.
A shared callable library keeps its shape predicates identical to the detector predicates.
All twenty source-derived cases pass, with 238 complete findings across the corpus.
The original positive legacy-scanner test also passes.

The CLI now selects files in native rules and submits their contents as one scanner batch.
The adapter retains each root's independent scan multiplicity after classification.
Native report rules render the output and decide the status.
Argument parsing uses the same Python standard-library grammar as the original CLI.
All seven original CLI tests pass with complete stdout, stderr, and return or exception values.
All twenty additional CLI cases pass, including overlapping roots, repeated files, links, parse errors, signatures, and ratchet boundaries.

The original verbose test exposes a missing extra-file selection in the first implementation.
Binding directory observation directly to its environment root repairs that omission.
The [TDD receipt](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-legacy-cli-tdd.json.gz) retains the failures and passing comparisons on the registered release engine.
No Python detector or discovery function runs during native acceptance.
No native engine code changes in this increment.

These are focused API results, not a new combined checkpoint.
The signing action still raises an explicit unimplemented error.
Configuration assertions, production acceptance, controls, updates, seven absent translations, and earlier migration gaps remain open.
A new fetch finds Final `735bc4422`; the next commit merges it before further acceptance.

### 2026-11-09 — Final 735 engine and complete original step-suite replay

Commit `c55ecc3bc` preserves the legacy scan and CLI port before the engine merge.
Commit `d28f39870` merges Final `735bc4422` into the root engine.
The merge retains upstream phase maintenance, row statistics, scalar execution, and aggregate semantics.
Its conflict resolutions preserve the generic native contracts required by existing ports.
Two local JSON-input tests require the new dependency-tracking argument; `997fcb4ab` repairs those call sites.

The [checkpoint](../gate/data-layer/fixtures/host/data-layer-port/rr-serverplan-steps-final735-checkpoint.json.gz) audits all 164 original tests.
All 212 complete API observations match through 205 native calls.
All 58 positive detector observations remain present. No original cases are excluded.
Sixteen tests without API observations run after native settings replace the imported Python configuration.
All thirteen configuration values also match the source recording in full.
The detectors consume the same native vocabulary tables.
The auditor streams shard recordings, and both harness layers share one parsed source baseline.

All sixteen original-suite commands pass within 115 seconds; the slowest takes 71.310 seconds.
All 199 supplemental cases pass on the registered optimized engine.
These include 45 detector, thirteen scanner, 72 signature, fifteen blocker, twenty legacy, twenty CLI, and fourteen production cases.
Five production mutation controls produce complete mismatches through successful native execution.
Twelve updates pass in one persistent workspace and store.
They cover edits, removals, changes between scan roots, signature changes, restoration, and an unchanged repeat.

Production testing exposes a real membership defect before repair.
The first candidate includes test and temporary trees and the CLI-only extra file, while omitting hidden source files.
Native directory requests now implement the fact engine's actual membership.
A wider corpus catches another difference: the fact engine includes `conform`, `node_modules`, `.pytest_cache`, and `browser_build`.
Its generic discovery helper excludes those directories; the plugin does not use that helper's exclusions.
The final comparison checks every selected file, full producer facts, structured findings, stderr, order, and duplicates.

API transport and production reporting now consume one typed producer relation.
Production diagnostics use their real `STEP-*` codes and preserve signature failures, including an empty repository.
The API protocol retains its existing DTO shape.
Native rules invoke no Python interpreter.

All 99 rule-engine tests pass; two existing helpers remain ignored.
All seven filesystem, nineteen JSON-input, fourteen string-function, and 182 tooling tests pass.
The first rule-engine attempts encounter a shared content-store writer and hit the command limit.
An isolated cache gives a passing 76.590-second run. The failed attempts remain recorded.
All six step policies pass the native reader.

Compilation is recorded separately from execution.
The final unit build takes 233.888 seconds and the debug CLI build takes 127.373 seconds.
The registered optimized build takes 1,425.738 seconds, including its wait for another build.
An earlier 947.586-second release build precedes the two test-call repairs and is excluded from acceptance.
Final advances to `ce80f6873` during compilation.
A TDD repair permits an optimized artifact from the already registered, identical source cohort to finish registration.
Both binaries, the current source tree, and the reviewed extensions are checked.
A different source identity still requires live upstream alignment.

This closes the original ServerPlan steps test-suite gap, not its full migration definition of done.
The native signing action and source-update acceptance remain open.
Seven translations, earlier port gaps, and global 100-rule acceptance also remain open.
The next engine cohort must merge and validate Final's newer row-program change.
No full-repository cold run occurs in this increment.
