# T11 notes: the streams since the review (DLRE, N1562, SSC)

Research track T11. Read-only notes; nothing was built, run, or benchmarked.

## Conventions

- Worktree roots (all paths below are relative to these):
  - **DLRE** = `/Users/jeremy/orca/workspaces/rerail/Data-layer-rule-expressivity` (HEAD `b1d36739` "test: require schema reuse, scope isolation and bounded completed rows").
  - **N1562** = `/Users/jeremy/orca/workspaces/rerail/ner-1562-scale-test-the-re-railer` (HEAD `e481c192` "TDD-RED: packetize expanded relational navigation").
  - **SSC** = `/Users/jeremy/orca/workspaces/rerail/https-github.com-neralabs-rerail-tree-Moti-Velz-sql-set-composition` (HEAD `b3e97cd4` "Define transition from matcher continuations to shared execution plans").
- Citations are `file:line` (line numbers of the files at those HEADs).
- Tags: **M** = a measured number reported by the source; **D** = a decision the source records (keep/reject/selected); **A** = an assertion, proposal, or design requirement that the source does not claim to have measured or implemented.
- Journal citations for DLRE use the current line numbers of `docs/data-layer-performance-journal.md`. The `d3470da2..HEAD` diff adds two blocks: lines 3695–4253 and 4261–5056 (verified with `git diff -U0`; the hunks are `+3695,559` and `+4261,796`). Lines 4254–4260 ("Future entries") predate the range.
- SSC `c17d2e47..HEAD` changes exactly four docs: `sql-matcher-batch-implementation.md` (184 lines), `sql-matcher-batch-optimization.md` (193), `sql-matcher-fragment-transition.md` (1,210 added; the file is 1,583 lines), `sql-structural-execution-plan.md` (387). I read all four in full.
- Workloads differ between streams and are **not comparable**. DLRE uses the "data-layer" repository: 9,021 files, 137,924,324 source bytes, 50 or 60 ported rules. SSC uses the shipped Python catalog: 5,725 files, 95,697,853 bytes, 146 AST grammars, 138 policies. N1562's design doc has no measurements.

---

## Source 1: DLRE (Data-layer-rule-expressivity)

### 1.1 Problem statement and the frozen baselines

Frozen uninstrumented references (`docs/cold-cpu-recovery-plan.md:13-27`; journal 4834-4870):

| Cold CPU, seconds | First 50 | All 60 |
| --- | ---: | ---: |
| File ingestion and candidate preparation | 628.327 M | 1,148.124 M |
| Deferred evaluation | 44.165 M | 1,392.224 M |
| Complete cold execution | 672.782 M | 2,540.634 M |
| Selection counter | 347.871 M | 729.928 M |
| Index preparation counter | 141.067 M | 129.736 M |
| Physical planning counter | 78.008 M | 159.368 M |
| OS peak RSS, GiB | 4.620 M | 6.391 M |

- Both runs have exact finding parity (24,820 and 24,993 findings) and zero failures (journal 4836-4837, 4853). First-50 split: 448.716 user + 224.066 sys; wall 725.358 s; compile 0.830 CPU s (journal 4838-4840) **M**. All-60 split: 1,855.030 user + 685.604 sys; wall 2,611.045 s; compile 1.533 CPU s; OS high-water RSS 6,862,143,488 B (journal 4854-4856) **M**.
- The phase totals and component counters are two separate views and must not be added together (`cold-cpu-recovery-plan.md:23`). Deferred evaluation includes per-file and repository analyses; it is not one global query (`:24`; journal 4866).
- The pre-SQL passing first-50 reference is **125.804 s** cold CPU. It remains the recovery target, and smaller improvements do not close recovery (`cold-cpu-recovery-plan.md:26,34`; `sql-schema-lifecycle-plan.md:405-408`) **D**. No clean pre-SQL all-60 reference exists (`cold-cpu-recovery-plan.md:27`).
- The current first-50 result is 5.348 times the pre-SQL reference (journal 4845) **M**.
- The all-60 run's default worker phase records another 1,398.713 s, dominated by the deferred apply (journal 4868) **M**.
- Both frozen runs predate the final import-membership correction and the allocator-profiling correction. Every new report must carry native, policy, workload, driver, and spike identities (`cold-cpu-recovery-plan.md:29-30`; `sql-schema-lifecycle-plan.md:384-386`).

Earlier history inside the diff range, in order:
- **Measurement reset (2026-10-25)**, journal 3695-3699 **D**: statement counts are diagnostic. Acceptance means cold CPU, wall time, peak RSS, and complete findings parity on all 9,021 files for 50 and 60 rules, with compilation recorded separately. This **supersedes the earlier fixed-100 acceptance statements**. The earlier first-50 SQL run took 2,507.496 s against 125.804 s pre-SQL (journal 3701) **M**. "Reducing the two-observation guard from 975 to 264 engine submissions did not demonstrate recovery" (journal 3703) **M**.
- At that point `Lower::pattern` still built each possible posting, ran `count(*)` to price each arm, and joined each posting to its runtime choice. This conflicts with plan-time selection from stored SQL statistics. The producer adapter's range path still estimated 32 rows (journal 3708-3710). "Physical anchor planning must follow index publication … Logical rule validation can remain earlier" (journal 3711-3713) **D**.
- First-50 run at the reset: 958.590 s cold CPU, 875.286 s ingestion, 82.953 s global evaluation, 1,978.154 s wall, 3 failures; OS 963.920 CPU s and 6,261,948,416 B peak (journal 3752-3765) **M**. Exclusive counters: anchor selection 537.411 s, index preparation 136.765 s, AST matching 58.161 s, parsing 11.148 s (journal 3776-3777) **M**. `manifestCandidateLookupNs` (191.489 s) is a default worker phase, i.e. residual CPU (journal 3778-3779). Host load was about 73 on 18 cores with 16.5 GiB swap in use (journal 3782).
- First complete 50 after anchor recovery: 899.070 s cold CPU, 1,897.468 s wall, OS peak 5,332,975,616 B, 2 failures (journal 4330-4337) **M**.
- Frozen first-50 candidate `dc697479…`: 903.546 s cold CPU (600.330 user, 303.215 sys), 1,312.792 s wall, OS peak 4,822,499,328 B. It had 4,931 false negatives, all in `internal-dead-code` (journal 4574-4586) **M**. Exclusive counters: selection 453.404, index preparation 206.421, physical planning 93.175 s (journal 4592) **M**. Both runs publish 28,230,245 nodes and select 6,425,028 root pairs (journal 4594) **M**.
- Complete all-60 diagnostic on an older addon: 2,714.768 s cold CPU, 3,914.650 s wall, OS peak 12,607,602,688 B, 7 failures (journal 4697-4705) **M**. The final all-60 shows a 45.57% reduction in OS peak RSS against this run (journal 4860) **M**.
- An earlier traced all-60 diagnostic: 24,993 findings but 1,694 engine failures; 2,701.305 s cold CPU; OS peak 14,610,890,752 B. "Independent capacity partitions incorrectly share this execution allowance." An external sample attributes 60.35% of active native observations to evidence-key interning (journal 4425-4430) **M**.

### 1.2 The complete traced diagnostic (S0 / WP0)

`docs/cold-cpu-spike-results.md:14-70` and journal 4962-4987:
- 9,021 files; 24,993 expected and actual findings; zero FP, FN, or failures. Logical compile 1.958 CPU s / 2.170 wall s. Traced cold run 3,586.594 CPU s / 4,308.203 wall s; OS peak RSS 7,530,659,840 B. This is diagnostic only; it overlapped lab work and ran with tracing (`spike-results:16-26`) **M**.
- Measured self CPU by span (`spike-results:28-38`) **M**:

| Span | Seconds | Calls |
| --- | ---: | ---: |
| SQL analysis | 679.109 | 110,742 |
| Candidate materialization | 619.630 | 198,648 |
| Relational materialization | 455.858 | 110,780 |
| Relational indexes | 341.870 | 132,500 |
| Pool trimming | 190.128 | 22,098 |
| Candidate statement preparation | 161.016 | 203,989 |
| Candidate result reads | 145.748 | 5,341 |
| Consumer release | 132.883 | 11,045 |
| Relational result reads | 51.483 | 11,045 |

- Span counts are not statement submissions. An analysis or index call can reuse state and submit no SQL (`spike-results:40-42`).
- The executor admits up to **792 suspended SQL consumers** in four capacity partitions. Peak suspended SQL storage is 249,905,204 B. The admission limit is 256 MiB and the input limit 64 MiB. "Those limits do not prevent expensive schema interactions between admitted consumers" (`spike-results:44-47`) **M**.
- **Root-cause mechanism** (`spike-results:49-53`) **A, corroborated by samples**: `Storage::analyze` runs SQLite ANALYZE during stage construction. SQLite's `sqlite3AnalysisLoad` clears schema statistics, reloads all of `sqlite_stat1`, and visits schema tables and indexes. Repeated small analyses therefore interact with every other live schema, which "explains why optimizing the final predicate leaves substantial CPU unchanged". Supplemental Node samples show `sqlite3FindTable` and `sqlite3FindIndex` dominating many stacks, alongside paging and `sqlite3AnalysisLoad`. Wall-time weights are not CPU shares (`:55-57`).
- **Sampler bug**: the scheduled samples targeted `/usr/bin/time`, not the engine, and are invalid evidence. The fixed sampler picks the workload child and records its PID (`spike-results:58-59`; journal 4947-4951).
- Clause attribution **M** (`spike-results:61-64`): `quality-contracts-1` 776.922 s (889 consumers, `recovery-plan:268`); `state-flow-1` 305.016; `authority-sites` 192.103; `legacy-execution-graphs` 114.153. The latter three total about 611 s (`recovery-plan:310`).
- Reconciliation: 3,204.752 s observed across spans. Of that, 1,527.242 s lacks a complete rule/clause identity (includes shared candidate and pool work). There are 103 overlap flags and 7 CPU-overdraw flags among 1,953,732 events: "this is not perfect CPU reconciliation" (`spike-results:66-70`) **M**.
- The 128-file instrumentation check observed 9.017 CPU s, of which 5.318 s had no complete rule/clause identity (journal 4930) **M**.
- Attribution method: nested CPU is subtracted (immediate child CPU on the same thread), unknown coverage is preserved, stages carry stable logical labels, and consumer, admission accounting, release, and trimming have separate spans (journal 4924-4927) **D**.

### 1.3 Target execution shape (recovery plan) — **A/D**

`docs/cold-cpu-recovery-plan.md:39-59`:
- "The executor has four specialized phases. It needs no generic graph scheduler":
  1. Publish immutable tape versions, positional kind/value indexes, and SQL-owned statistics in bounded batches.
  2. Gather exact consumer demand and choose one physical access path from stored statistics.
  3. Execute narrow operator families over the whole demand batch.
  4. Consolidate findings and release consumer state.
- **Demand row** (`:48`): consumer, source version, file, range, producer, parameters, requested columns. "Separate IN lists must not create a Cartesian product of those fields" (`:49`).
- **Native binding contract** (`:50-53`): it receives the complete demand batch in one xFilter and returns typed result rows. xNext emits output rows and "does not implement an interpreter or request another scalar node operation". The same relation contract also has a SQL binding. "These contracts preserve the future CPU, JIT, and GPU boundary."
- **Sharing** (`:55-59`): shared producers require equal definitions, versions, parameters, columns, and candidate scopes. A shared producer completes once per exact execution domain, including a complete empty result. Different scopes stay separate even when ranges overlap. "A measured second consumer justifies materialization. Repeated query text alone does not." "A native producer implements a generic relation operator. It does not encode a data-layer rule."

### 1.4 Measurement ladder and rules — **D**

`cold-cpu-recovery-plan.md:61-79`:
- Ladder: (1) reproduce the production shape and keep its complete output rows; (2) compare setup-inclusive CPU and OS RSS in fresh processes; (3) vary files, demand rows, source nodes, consumers, and scope overlap independently; (4) repeat the original positive and clean observations; (5) measure the complete repository after each selected implementation delivery.
- Commands have a 110 s target and a 115 s outer limit, and a timeout is an incomplete result. Acceptance timings must not overlap other work. CPU tracing excludes allocator enumeration by default. External samples are bottom-up. "SQL VM counts, callback counts, preparation counts, and statements explain results. They are not performance gates." Matched trials rotate variant order. RSS is measured in separate processes through exit. Every spike records keep/reject/unresolved plus integration risk.

### 1.5 Spike S1: choose one anchor before execution

`spike-results:72-91`; plan WP1 `recovery-plan:100-119`.
- Setup: a captured structural family with 333 result rows in four overlapping request scopes. One query over persisted statistics chooses between function and identifier anchors, without counting each posting alternative at execution time.

| Four-scope control | Median total CPU ms | Median query CPU ms |
| --- | ---: | ---: |
| Broad construction | 67.378 M | 0.954 M |
| Forced identifier anchor | 73.168 M | 8.442 M |
| Statistics-selected anchor | 66.203 M | 0.826 M |

- The total includes lab fact reconstruction, which dominates this small case. Tests that reverse the statistics flip the choice with full row parity. An empty outer demand gate prevents entry into a failing inner operation (`:84-87`).
- **D** (`:89-91`): select from SQL-owned statistics and emit one arm. Extend execution ordering beyond the current compiler-complexity ordering in `Lower::plan`. Keep compile-complexity limits separate from cardinality estimates.
- WP1 also requires (`recovery-plan:106-114`): read statistics once per plan batch from `kfreq`, `file_kind_counts`, and value summaries; never count postings to price arms at execution time; put the gate outside the expensive join or in the recursive seed; keep regex coverage fallback, with the existing matcher as semantic authority; prove rejection with a producer-entry counter or a failing unchosen arm.
- Earlier A1 history (journal 3811-3906): SQL-owned value cardinalities were added beside kind frequencies (`vlex.n` keeps its segment-count meaning; value estimates include uncovered nodes). One batched statistics query serves the admitted frontier. Template reuse depends on the selected requirements. The compiler emits one anchor path and checks necessary root kinds after selection (journal 3824-3830) **D**. SQL schema version 3 backfilled older segment counts once. A separate exclusive `anchorPlanningNs` counter was added.
- A1 alone was a **regression** **M** (journal 3874-3877): 32 files / 50 rules went 9.588→11.270 s (+17.5%); 128/50 went 15.395→17.937 s (+16.5%). For 128 files, matcher CPU fell 2.771→0.637 s, but selection rose 8.472→12.161 s and physical planning added 3.267 s (journal 3893-3895). D: keep statistics, selected-arm correctness, and the signature correction as groundwork; the next delivery must fix the SQL shape (journal 3897-3898).
- Bounded-conjunction rung (journal 3838-3854): candidate conjunctions are ordered by bounded structural compile cost, and completed necessary filters are kept when a later conjunct cannot fit (explicit partial admission metadata). The native matcher still evaluates the full authored rule. The synthetic 1,000-sync-function case selects 0 roots instead of 1,000. A duplicate-parent-root bug was fixed: each SQL parent climb is now distinct (journal 3857-3860). "Partial compilation cannot restore exactness through a cached utility" (journal 3860).

### 1.6 Spike S2: fuse SQL before physical cuts

`spike-results:93-122`; plan WP2 `recovery-plan:121-135`.

| Physical shape (2 consumers, 17,033 rows, 256 scoped requests) | Median total CPU ms | Median query CPU ms |
| --- | ---: | ---: |
| CTAS, index, ANALYZE, read, release | 109.918 M | 44.109 M |
| Repeated inline computation | 124.949 M | 58.363 M |
| One shared materialized CTE | 101.101 M | 36.347 M |
| Fused consumer query | 97.916 M | 33.847 M |

- Every trial preserves the complete multiset. Fused is simplest. A materialized CTE helps only when it avoids a real second computation. Repeated inline expansion loses. These are structural controls, not a production rewrite (`:105-108`).
- Policy overlay: removing two terminal diagnostic cuts keeps all 192 original quality observations passing (130 positive, 62 clean), with exact finding and execution-path parity (`:110-113`) **M**. Broader overlays exceed the existing **16-KiB authored-query limit** (`:115`). A small repository control shows no useful CPU gain from this policy-only change (`:117`) **M**.
- **D** (`:120-122`): perform broad fusion after logical validation, inside the physical planner. Keep the authored limits. Partition physical families at real producer boundaries and existing SQLite limits. Do not promote the small policy overlay.
- WP2 requirements (`recovery-plan:125-130`): the timed interval includes table construction, indexes, ANALYZE, both consumer reads, and retirement. Predicates, NULL behaviour, affinity, recursive termination, and multiplicity stay the same. "Reject a shared relation over the union of different scopes." Measure source bytes and typed fields read, not relation count.

### 1.7 Spike S3: typed whole-demand batch; native child operator not a CPU win

`spike-results:124-147`; plan WP3 `recovery-plan:137-153`.
- The generic **first-field-child** operator gets the complete request-ID batch through `sqlite3_vtab_in`. It returns typed columns and applies the kind predicate *after* selecting the first field child, with no fall-through to a later child of a preferred kind. The first prototype visited unnecessary siblings; the corrected one matches `nera_child` semantics (`:126-133`).
- Regression: 80,000 correlated requests and 80,000 outputs; **one xFilter, two broad output waves, at most 65,536 buffered rows**; xNext serves completed columns (`:130-132`). The journal's earlier statement says "40,000 correlated requests and 80,000 output rows" (journal 4957); the later entry (journal 4993) and the spike results say 80,000 requests. **Internal inconsistency.**

| Prepared operator, 20,556 outputs | Median total CPU ms |
| --- | ---: |
| SQL child index | 106.832 M |
| Scalar control | 100.773 M |
| Native typed batch | 105.781 M |

- **D** (`:144-147`): keep the typed whole-demand contract for expensive operators and future GPU execution. Do not promote the native child implementation as a CPU optimization. Use SQL for the selected production binding until a complete native comparison wins. The scalar control is a measurement control, not the target shape. Descriptor, transport, and output costs stay visible (`:141-142`).
- WP3 contract requirements **A** (`recovery-plan:141-153`): use a generic structural operator (field-qualified children, ownership). Pass correlated scope IDs through `sqlite3_vtab_in` or an equivalent typed frame; "Do not send independent file and node IN lists." Preload cardinality metadata from SQL statistics before xBestIndex, and "Do not query SQLite recursively during planning." Replace the fixed 32-row range estimate with an estimate for the actual constraint shape. Keep duplicate-request semantics, missing values, source versions, and per-file failures. Keep criterion: one xFilter per operator batch, output-only iteration, lower setup-inclusive cost. GPU check: typed contiguous demand/output columns and a bounded number of broad operator launches. "No new rule interpreter or parallel implementation of rule semantics."

### 1.8 Spike S4: publication batching

`spike-results:149-174`; plan WP4 `recovery-plan:155-170`.

| Publication of 1,024 copies of one captured file | Main cache | Median total CPU ms |
| --- | --- | ---: |
| Per-file transactions, triggers | 1 MiB | 3,458.821 M |
| 32-file transactions, triggers | 1 MiB | 529.826 M |
| 32-file transactions, grouped value statistics | 1 MiB | 489.235 M |
| Grouped statistics | 8 MiB | 431.765 M |
| Grouped statistics | 32 MiB | 449.563 M |

- Each variant keeps the production positional FTS declaration and complete value-posting bytes, and passes statistics, rollback, replacement, and pinned-SQLite-reader checks (`:151-153`).
- The 32-file trace shows 7 host submissions and 46 SQLite internal completions. Per-file shows 224 host submissions and 295 internal completions. The 2,336 trigger entries are not host submissions. FTS still maintains per-document metadata inside a host batch (`:168-171`) **M**.
- **D** (`:173-174`): publish 32-file packets with the existing triggers, positional index, and 1-MiB cache. Do not promote grouped statistics (the gain overlaps trial variance and adds maintenance) or larger caches (they raise residency without a consistent win). The replicated tiny file cannot predict the repository gain (`:166`).
- WP4 constraints (`recovery-plan:159-170`): keep the ast-index-spike positional FTS layout and exact value postings; keep deletion, replacement, pinned old versions, rollback, and retry. "Retain SQL ownership of statistics. Do not replace indexed values with an always-resident Rust index." "No per-rule index rebuild."

### 1.9 Spike S5: schema fan-out and template reuse

`spike-results:176-199`; plan WP5 `recovery-plan:172-187`.
- For 64 consumers, retained schemas 128→1,024 raises repeated-analysis consumer CPU from **17.209 to 102.277 ms**. At 1,024 retained tables, omitting repeated analysis drops it to 22.220 ms. Stable backing costs 1.365 ms; the fused control 0.060 ms. Complete trials include retained-schema construction and cleanup, which dominate their totals. This control does not measure the full run's schema count (`:178-183`) **M**.
- 256-consumer planning: 118.637 ms with private schemas, 95.699 ms with one batch. Reusing a statement takes 100.604 ms but still executes it 256 times. A regression keeps preparation counts separate from execution counts (`:185-187`) **M**.
- Real compiler replay (5 recorded programs × 128): shared metadata 1,470.930 ms versus 2,712.551 ms without sharing; inference statements 4,480→35. "The production engine already has this bounded cache. This is evidence to retain it, not a newly implemented engine improvement" (`:189-193`) **M/D**.
- **D** (`:195-199`): gather demand before creating private SQL schemas and execute one physical family batch. Reuse templates while the chosen plan is unchanged. "Count changes alone must not invalidate a physical template." Pool trimming and release still mix metadata scans with SQLite cleanup; instrument visited objects and actual drops before proposing owner indexes.

### 1.10 Spike S6: construction dominates

`spike-results:201-223`.

| Complete structural control (256 requests, 17,033 rows) | Median CPU ms |
| --- | ---: |
| Staged SQL | 109.005 M |
| Fused SQL with common reconstruction | 99.925 M |
| Native operator with common reconstruction | 109.374 M |
| Direct postings and scalar control | 23.895 M |
| Direct postings and native batch | 30.764 M |

- Empty demand still costs about 59–65 ms when the broad lab fact table is built first. The direct controls take under 1 ms and never enter the native operator. The direct FTS query gates demand outside the expensive join and scans the chosen term once (`:213-215`) **M**.
- "The laboratory table is a reconstruction of captured facts, not production publication. Its removal is not a measured fourfold end-to-end engine improvement" (`:218-219`).
- **D** (`:222-223`): remove unnecessary construction before layering operator improvements. Do not multiply isolated ratios or present the native control as the recovery result.

### 1.11 Memory frontier

`spike-results:225-240`; Delivery 4 `recovery-plan:322-343`; journal 4711-4730, 4238-4245, 4504-4506.
- The deferred snapshot shows about 7.2 GiB physical footprint and 4 GiB of swapped allocator pages: "A falling current RSS therefore does not demonstrate memory recovery." The complete run's peak footprint is 7,906,637,232 B (`:227-229`) **M**.
- 128-file census: zero JSON projection fields; 6,977,240 typed projection bytes; 23,776,878 resident document bytes after deferred execution. Live allocator bytes fall 160,906,480→14,461,872 after workspace closure (`:231-234`) **M**.
- Next memory work must separately account for retained syntax, typed projections, source graphs, SQL schemas, and completed producers. "Do not trade memory for repeated parsing without a complete cold CPU comparison" (`:238-240`) **D**.
- Projection capacity fix: published projection columns now use boxed slices. Typed projection storage went 93,716,350→11,696,974 B (−87.5%), but driver peak RSS only 1,655,357,440→1,649,115,136 B (journal 4719-4729) **M/D**. Before the fix, 1,749 batches held 49,185 rows: 28 rows per batch against 256 reserved (journal 4712-4713).
- vmmap during all-60: 10.7 GiB allocated across about 25.45 M native allocations (journal 4711). During conditional ingestion: 4.1 GiB malloc, 4.5 M allocations, 2.3 GiB resident, and 1.9 GiB swapped/compressed malloc pages (journal 4504-4505) **M**.
- Census checkpoints: about 597 MiB before compilation, 957 MiB after, 1,427 MiB after ingestion. The workspace estimate covers only 35.8 MiB. 19 physical templates hold 11,689,277 SQL bytes and 4,636,222 graph bytes. The recording buffer contributes about 145 MiB even in the 32-file control. Shim allocator counters return unavailable zeros (journal 4239-4245) **M**.
- Delivery 4 plan **A** (`recovery-plan:328-343`): pin syntax and producer frames only through their final dependent family. Release parser state once demanded typed facts are complete and later work can use the immutable tape. Reuse templates and empty schema backing without retaining source-dependent rows across domains. Add owner postings or cached byte totals only when counters show a material repeated scan. Reject cache policies that save memory by forcing harmful repeated cold parsing. Measure live allocations, physical footprint, and OS peak RSS separately.

### 1.12 Evidence-ranked implementation deliveries (recovery plan)

`cold-cpu-recovery-plan.md:218-351`. Each is a complete delivery with a regression ladder and full cold measurements. Statement counts stay diagnostic. Intermediate requirement: less cold CPU, no peak-RSS regression against the preceding uninstrumented result. Completion also requires the 125.804 s first-50 target (`:220-224`) **D**.
- **Missing measurements to carry first** (`:226-237`): coarse counters at admission, producer completion, family completion, and workspace closure, covering active consumers, schema objects, pinned producers, metadata bytes, and rows per owner. Separate attempted analysis from executed ANALYZE, index reuse from creation, and trimming scans from actual drops.
- **Delivery 1: cold candidate selection and packet publication** (`:239-263`). Steps: gather exact file, source-version, range, root-kind, value, and column demand before producer construction; read SQL statistics once per planning batch; choose conjunction order and anchor from them; keep compile limits separate; emit only the selected arm and refine its keys; gate empty demand before posting iteration, decoding, source matching, or temp construction; publish up to 32 validated files per transaction with bounded parameter packets; keep positional FTS, value triggers, SQL statistics, and cache budgets. Integration points: candidate lowering, source-wave preparation, `Store::put_known`. "Physical template identity includes the chosen arm and structural definition. Counts can change without changing that identity. Source versions and candidate domains still determine data validity" (`:254-256`). Exclusions: replacing the matcher, a second in-memory rule engine, raising allowances, upward cache tuning (`:263`).
- **Delivery 2: one physical quality pipeline** (`:265-305`). Expanded as SC1–SC4 below. Key requirements: keep logical SQL validation and the 16-KiB limit; resolve options, types, deferred columns, names, and producer dependencies before physical composition; gather lightweight request descriptors without a SQL schema per suspended consumer; partition the validated program at native readiness, recursive operator, and SQLite statement limits; emit one reusable statement per physical family; preserve opaque SQL scope; batch only partition-safe SQL; carry consumer and source-domain IDs as data; use a CTE cut only for an observed second consumer in the same domain; send rows directly to finding consolidation when no later SQL consumer needs them; remove the CTAS/index/ANALYZE/release lifecycle. "Deferred source expressions must remain lazy after fusion. Projection demand must reach the producer before it creates capture columns. A syntactic SELECT of fewer columns is insufficient if the producer already built the full row" (`:281-283`). "Use the existing linear stage program and typed producer barriers. This requires a specialized family executor, not a generic graph scheduler. No compiler branch should recognize a data-layer rule ID" (`:287-289`).
- **Budget and failure rule** (`:295-302`): keep each consumer's remaining allowance. A partition-safe SQL batch may be capped at the minimum remaining allowance; if the cap trips, discard uncommitted results and subdivide complete consumer domains with the same caps. "Never multiply a rule allowance by batch size. Never split one repository-wide consumer into independent file analyses." The retry policy needs proof before use and is not a prerequisite for context reuse.
- **Delivery 3** (`:307-320`): the same family lowering for `state-flow-1`, `authority-sites`, and `legacy-execution-graphs`. Keep current batched recursive SQL and its cycle semantics; keep global candidate domains complete; use native algorithm relations only when construction-inclusive measurements justify them. "The first-field-child spike does not justify a general switch to native execution."
- **Delivery 4**: memory ownership (§1.11).
- Implementation order recorded in the journal **D** (journal 5013-5015): candidate demand/statistics and packet publication; then the full quality pipeline; then the next three clauses; then measured ownership and reuse cleanup. "Demand gathering must precede private SQL schema creation." No algorithm flags, alternate rule interpreter, allowance increase, or Linear issue.

### 1.13 Schema lifecycle plan (SC1–SC4)

`docs/sql-schema-lifecycle-plan.md` expands Delivery 2.

**Outcome** (`:10-25`): "The number of pending consumers must no longer determine the number of live SQL schemas." SQL owners reuse a bounded set of family contexts. Physical cuts keep data only for an identified dependency. Statistics work follows cut lifetimes. The first target is the full 54-stage `quality-contracts-1` program using generic SQL stages and existing producer contracts, with no rule-ID branch.

**Current implementation points** (`:46-58`):

| Source | Current | Required change |
| --- | --- | --- |
| `relational/batch.rs` | Builds SQL sessions before demand forms a cohort | Gather demand without a private SQL session per consumer |
| `relational.rs` | Inputs, aliases, stages, findings | Separate consumer state from reusable physical execution |
| `relational/storage.rs` | Tables, indexes, statistics, ownership, pins | Own cuts and reusable contexts with explicit lifetimes |
| `relational/pool.rs` | Private names, leases, trim, release | Reuse stable contexts without schema changes per consumer transition |
| `relational/database.rs` | SQLite, resident storage | Report owner capacity at coarse boundaries |
| `compile/relational.rs` | Logical checks, stage program | Keep logical checks before physical grouping |
| `compile/relational/inference.rs` | SQLite output types | Supply contracts for fusion eligibility and retained cuts |
| `relational/deferred.rs` | Deferred expressions via views | Keep lazy expressions without a view per consumer |
| `relational/shared.rs` | Compares producer definitions and versions | Keep data identity separate from context reuse |

`create_stage` already reuses some declarations and indexes, but private names and source-dependent physical identities block broad reuse. `Storage::analyze` repeats analysis for populated indexed stages, and SQLite then reloads statistics for the whole affected schema (`:60-63`).

**Target pipeline** (`:72-90`): logical checks and physical planning → consumer descriptors and exact demand → whole-batch producer execution → reusable SQL family context → findings or next producer barrier → release data pins and reuse context. The stage program determines the next barrier. "The coordinator needs ordered family steps and readiness states. It does not need a generic graph scheduler."

**SQL scope versus producer batching** (`:92-117`) **A/D**:
- A producer receives the complete demand batch. Its output uses shared typed columns and indexed scope slices. The SQL reader consumes completed output and does not launch native work per consumer or node.
- "Opaque authored SQL retains its original evaluation scope. Adding a consumer column does not automatically preserve aggregates, windows, DISTINCT, limits, or correlated subqueries. `partitionBy: file` describes the evaluation domain. It does not prove that arbitrary SQL distributes over files."
- Initially a reusable family statement runs once per original scope and reads that scope's complete rowsets. This removes schema construction without a general partition rewrite. There are no per-node or per-field SQL queries.
- Compiler-generated operators whose contracts define the partition key may combine scopes, keeping every request identity and source domain; distinct candidate scopes do not become one shared producer result.
- Three counts are kept apart: native producer computations (one per admitted batch or bounded output wave), SQL preparations (one per family template and valid schema context), and SQL executions (whole-rowset family executions, original scope kept unless partition safety is shown). "Reducing submissions alone is not an acceptance gate. Opaque SQL batching is not a prerequisite for removing schema overhead."

**Runtime objects** (`:119-134`) **A**. The names describe responsibilities; existing structures may be reused:

| Object | Owned state | Lifetime |
| --- | --- | --- |
| `FamilyProgram` | Ordered SQL families, producer barriers, schemas, field reads, cut reasons | Compiled logical program |
| `ConsumerState` | Source domain, versions, options, budget, next family, result destination | One logical consumer |
| `DemandBatch` | Complete correlated producer requests and required columns | One producer wave |
| `FamilyContext` | Stable aliases, functions, views, backing slots, prepared statements | Reusable SQL owner context |
| `ExecutionBinding` | Immutable references to active scope, input slices, budget | One statement execution |
| `CutHandle` | Definition, exact data identity, completion state, dependency pins | Through final dependent read |

Rules: `ConsumerState` owns no private SQLite schema and no connection per file. `FamilyContext` must not establish data equality from a reusable table name. The producer registry stays authoritative for equal definitions and domains.

**Stable storage and execution bindings** (`:136-157`) **A**:
- Required SQL cuts use stable backing tables with an internal **binding key**. The key selects one complete producer definition, source version, and candidate domain. "Indexes begin with that key before the reader's lookup columns." Different bindings can share one physical table without sharing logical results.
- Stable views expose only rows selected by an engine-owned execution binding: a small relation fixed during statement execution and joined on indexed binding keys, "must not require a scalar callback for every data row". Authored SQL cannot reach other bindings or engine metadata.
- At a producer barrier the consumer keeps handles to its live cut rows and releases the SQL context while native work completes. The next execution rebinds those handles to the same logical input names. Required boundary output goes directly into its final backing or typed frame, with no scratch copy followed by a second full copy for suspension.
- Local scratch leaves the context before another scope uses it. Shared cut data follows dependency pins independently of context reuse. "No table name alone establishes that cached data belongs to the active scope."

**SC1: separate consumer admission from schema ownership** (`:159-220`).
- Scope: the admission/session split, stable family contexts, and the full quality program on that path, initially keeping its logical stages and necessary storage semantics. Not combined with a blanket ANALYZE removal.
- Steps: (1) move source checks, input discovery, and producer demand out of `Session::new` into consumer preparation; (2) keep the native input gate before body preparation; (3) represent suspension as descriptor + budget + next step + data handles; (4) form producer batches from descriptors before binding private SQL state; (5) allocate stable physical names from program and context layout; (6) reuse declarations, indexes, and empty backing across scopes; (7) install common functions once per compatible context; (8) bind active source and budget without reinstalling functions at each consumer transition; (9) keep that binding immutable until the statement and its cursors close; (10) return the context once the result or producer request completes.
- Invariants: a statement-owned cursor pins its input frame; a context cannot switch scopes while a cursor is active; native production stays a separate batch operation; "Repeated `xFilter` calls that expose completed slices must perform no producer computation." Scope lookup uses actual ranges or reverse postings over shared columns; "It must not scan the complete frame behind an interface marked indexed. Build that index once per frame and include its construction cost." Use current byte budgets. Report active schema objects separately from pending descriptors. Evict contexts as whole units after cursors close.
- Required observations: pending descriptors and active contexts; live tables, views, indexes, modules, and prepared statements; declaration and index reuse versus actual schema changes; attempted versus executed ANALYZE; metadata objects visited, pins, eviction candidates, and drops; resident SQL bytes and typed-frame bytes. Inventories at admission, producer completion, family completion, and owner closure. Host submissions kept separate from SQLite internal executions.
- TDD ladder: hundreds of suspended consumers without a schema each; two scopes through one context with exact duplicate and NULL rows; every cursor closed before rebinding; a large frame completed through indexed slices without scanning all rows per scope; rebind source and budget while keeping prepared statements and isolation; cancel one consumer without invalidating another's frame; two consumers suspended at a barrier keep distinct cut rows through reuse; boundary output produced once without a second copy; all quality observations plus repository parity.
- Completion: pending-consumer growth no longer causes proportional schema growth; the full quality program runs through reusable contexts with exact results; cold CPU improves with no peak-RSS regression.

**SC2: compile SQL families after logical checks** (`:222-271`).
- Steps: keep checks for every authored query and option branch; keep the 16-KiB authored limit; record dependencies, output contracts, field reads, source reads, and recursive properties in `FamilyProgram`; compute downstream column demand before producer construction; fold eligible single-use SQL intermediates into the consumer; keep shared cuts only for measured repeated reads in the same domain; keep native readiness barriers and required recursive boundaries; respect the **1-MiB SQLite statement limit**; send terminal results to finding consolidation; remove the matching declaration, population, index, analysis, and release operations.
- "Every retained SQL cut needs a recorded reason and its dependent readers." Unresolved semantic barriers go into the cut inventory explicitly; logical stages are not silently classified as required cuts.
- Fusion must preserve SQLite affinity, values, and comparisons: "Even with equal column names, removing typed storage can change behavior." Use the inference machinery to check transformations. If equivalent semantics are not established, keep the typed boundary. No blanket CAST rewrite and no native scalar callback per row.
- Deferred expressions stay lazy; an unused source-text column must not trigger source reads. Named intermediates keep their lexical SQL scope. CTE shadowing and quoted identifiers go through checked bindings.
- TDD ladder: fuse producer + sole reader preserving the multiset; typed comparisons for integers, text, blobs, NULLs, compounds, windows; deferred expression unevaluated until demanded; one shared result for equal definitions and domains, including complete empties; overlapping unequal domains separated; conflicting producer definitions reported with both origins; every authored option and unknown-option default preserved; an internal statement segmented at the limit without raising authoring allowances; the 192 quality observations (130 positive, 62 clean); the broader corpus and both workloads.
- Completion: each eligible stage loses its old lifecycle, and the cut inventory explains every remaining boundary.

**SC3: statistics for the remaining cuts** (`:273-308`).
- Scope: cuts left after SC2 plus the unchanged published base relations; source kind/value index layout unchanged.
- Steps: classify cuts by their exact reader access paths; keep indexes with measured repeated reads; use stored statistics for discovery choices and native row estimates; maintain cut cardinalities from bulk publication results where already known; collect required statistics after a complete population batch; group statistics maintenance at the family or publication boundary; prepare readers after that boundary; remove automatic ANALYZE from stages that no longer exist or need no refresh.
- "Equal row counts do not establish equal value distributions. A previous `analyzed` row count is insufficient as a freshness policy. A populated cut must not trigger a complete schema reload solely because it is populated."
- During SC1, legacy analyses may stay within the smaller context. In SC3 each refresh needs evidence that the reader plan benefits; a refresh that lowers complete cost is kept. "This decision belongs to the physical plan, with no user configuration flag."
- TDD: a populated cut read twice without rebuilding its index or recollecting statistics; source-version changes invalidate data and statistics; distributions changed at equal cardinality; empty↔nonempty transitions; skewed joins and rare/common anchors; preparation, refresh, query, and release CPU compared together.

**SC4: bound retention and cleanup** (`:310-343`).
- Scope: family contexts, shared cuts, typed frames, and their pins. Parser and source-graph retention stay visible but need their own ownership evidence first.
- Steps: keep schema identity separate from source-dependent data identity; pin cut data through its final dependent reader; record complete empty output as success; clear private rows in reusable backing without changing its schema; retire shared rows after the final pin or source-version invalidation; evict at explicit owner boundaries; keep prepared statements while their context is valid; when a context leaves the cache, remove its definitions, indexes, views, and modules together.
- Pool trimming must separate cache lookup, registry traversal, row deletion, and SQLite DDL; SC1 supplies these counters. Add owner postings and maintained byte totals only if repeated owner scans stay material, and charge their maintenance to that comparison.
- TDD: repeated reads with one construction and one retirement; pinned old version kept during a later edit; failed producer rollback not marked complete; retry without stale rows, names, or statistics; repeated edits with bounded contexts and metadata; workspace close releases frames, cuts, cursors, dependencies.
- Completion: retained objects follow live dependencies and capacity, not historical consumer count; cleanup is inside the CPU measurements.

**Budget, failure, cancellation contract** (`:345-363`): SQL runs under the original consumer budget during the initial reuse delivery. Batch producer work keeps per-consumer accounting. No allowance increases and no per-node scalar budget callbacks. A partition-safe batch cap is the minimum remaining allowance and is conservative; it needs a measured retry bound before use. Exhaustion discards uncommitted results and then subdivides complete consumer domains, with each retry keeping original limits. Files inside a repository-wide consumer are never split. The first delivery needs no SQL retry mechanism ("This avoids coupling schema reuse to a new generic failure scheduler"). Cancellation closes cursors before rebinding or reuse. Failed or cancelled work never publishes a complete cut. Tests put a failing consumer beside successful ones.

**Delivery order** (`:365-380`): SC1 (from spike evidence) → SC2 (needs SC1) → SC3 (needs SC2) → SC4 (needs SC1 counters and SC2 lifetimes). "An empty abstraction, a reduced submission count, or one faster helper is insufficient." Afterwards apply to the other three clauses. "Do not fork their rule logic or introduce a general SQL partition rewriter. Keep the existing batched recursive SQL and whole-demand producer contracts."

**Resource gate** (`:382-413`): freeze a matching uninstrumented control (including the import-membership correction) first. Per delivery: focused ladder; affected originals; broader native, shape, static, and profiling checks; 50 rules × 9,021 files; 60 rules × 9,021 files, with tracing, SQL capture, census, and sampling off; compile CPU recorded separately; identities kept; expected and actual findings and failures recorded; journal plus retained cut inventory before the next delivery.

### 1.14 Shared SQL execution ownership (current production description)

`docs/sql-execution-owner.md` describes the existing system, with ownership/materialization choices "provisional until measured at that scale" (`:90`).
- An execution owns immutable source facts and completed SQL producers. Each snapshot or workspace owns its membership and results. Closing one consumer does not remove another consumer's source version; closing the final owner removes its databases (`:3-6`).
- **Source publication** (`:8-17`): one file-backed positional FTS store holds `(path, language, source digest)` versions, and different contents at one path can coexist. SQL requests select admitted file IDs. Candidate completion records the admitted domain, including complete empties. Logical membership pins versions. Idle versions are kept under a **64 MiB** estimated budget and retired in batches. Structural facts also provide an index on **`(fid, parent, field, pre)`**, so a batched SQL query can select the first child without rebuilding a child relation.
- **Producer identity** (`:19-33`): the compiler records dependencies through SQLite's **authorizer**. Identity compares the complete definition and dependencies; "Local stage names and hash values alone cannot establish sharing." SQL identities include query text, declared types, deferred expressions, index requirements, dependency identities, and the regex bound. Syntax identities include file versions, grammar, exclusions, empty-text behaviour, and owner definitions. "Projection demand does not change values in the referenced syntax columns." Prepared evidence needs the full input context; workspace version sets come from authoritative document metadata, and file partitions have a separate domain identity. Raw evidence conservatively includes the full rule definition and clause. "These restrictions can miss sharing. They prevent sharing that the engine cannot prove."
- **Execution** (`:35-47`): one controller advances consumers through SQL and broad native producer waves. Consumers share one SQLite connection per execution. "No cursor survives a native yield." Each consumer has private input and native-module names, permissions, work counters, and diagnostic construction; activation restores its functions and source visibility. A producer becomes reusable after construction and indexing succeed; final result queries always run for their consumers; no cached findings are substituted. Completed tables live in SQLite TEMP storage with disk spill and bounded page caches. Registry and schema metadata are bounded; eviction keeps pinned producers; binding IDs are never reassigned. Admission counts the shared database once.
- **Materialization boundaries** (`:49-60`): every authored stage is validated, including unused ones. Producers shared across scopes materialize once; repeated references within one scope use a materialized CTE. Native operations, deferred columns, recursive queries, and explicit collation boundaries stay physical. The planner bounds SQL text and estimated expansion at **1 MiB**; authored fragments keep their own limit. "**Inlining everything exceeded SQLite's 65,535-reference limit** and failed the corpus check" **M**. SQLite infers types for selected producers; independent metadata queries share bounded CTAS batches. "An empty left operand prevents entry into the entire inference query. An inner zero-row limit prevents flattening that boundary. An outer `WHERE 0` or `LIMIT 0` alone can still initialize a shared CTE and execute its source."
- **Candidate retirement** (`:62-67`): logical bindings expire at last use. Physical candidate rows retire together when retained rows reach **131,072** and retired slots exist; the final owner DROP discards remaining retired rows without per-set DELETEs.
- **Inspection** (`:69-89`): `--sql-profile` saves a TEMP DB copy per consumer plus its logical-to-physical bindings; `relational_facts.py` compares typed row multisets. The fixed 100-statement reference is diagnostic.

### 1.15 Performance decisions directory

**Finite kind alternatives** (`docs/performance-decisions/finite-kind-alternatives.md`):
- Evidence: Q224 spent 1.247 CPU s and 50,792,117 VM instructions partitioning and reuniting one candidate scope over nested finite kind alternatives (`:7-9`) **M**.
- Prior work: ast-index S066 (lines 3855–3874) kept a single SQL integer `IN` list after a bitmap alternative lost, 123 versus 110 ms. "The shared indexes contain no directly measured finite-kind Boolean folding experiment" (`:20-22`).
- **D** (`:26-31`): recognize exact finite kind alternatives before emitting individual SQL branches. Union their kind sets, intersect any enclosing restriction, emit one filter over the original scope. Memoize utility references. Approximate or non-kind conditions stay on the conservative path. "Negation remains caller SQL and may subtract only an exact result. No scope is widened."
- Result: 1,516 DSL tests, 42 shape tests, 2,350 originals, and 120 controls pass. The three-run median ingestion CPU on the 128-file/60-rule control fell **25.675→21.827 s (−15.0%)**; wall 33.310→24.824 s; RSS effectively unchanged (`:43-45`; journal 4146-4158) **M**. One-file SQL CPU fell 6.369→4.977 s; VM 301,856,025→259,876,714; full-scan steps 8,450,565→4,280,207 (journal 4134-4135) **M**.

**Whole-frame source confirmation** (`docs/performance-decisions/source-confirmation-batches.md`) — a typed vtab contract, see §1.17.
- Evidence: the 32-file/60-rule profile performs hundreds of thousands of source scalar calls (`:7`).
- Result: 36 native source batches, 455,541 requested predicate/key pairs, 77,365 outputs, 88,824 text reads (journal 4234) **M**. The 128-file control shows "lower compilation cost and RSS, with little execution CPU improvement" (`:51`). First source control: 21.263 s ingestion CPU, overlapping verification (journal 4227-4229) **M**.
- Compiler control: a compilation-scoped regex dictionary shares regex programs and is dropped when compilation ends; there is no persistent global cache (`:45-46`). Policies contain 4,808 regex occurrences / 881 distinct patterns (journal 4247) **M**. Compiler reuse cuts compile CPU about 31% (2.546→1.764 s) (journal 4271-4275) **M**.

### 1.16 Other journal mechanisms and lessons (d3470da2..HEAD)

In journal order:
- **Indexed parent membership** (journal 3908-3942): direct-parent membership became quadratic when SQLite underestimated a materialized child projection. Q187–190: 838 M VM instructions, 91 M scan steps, 4.277 CPU s (nested scans of `m` and `w`, no automatic index) **M**. Fix: seek `decoded_child_lookup(fid,parent,field,pre)` from selected parents. Final intersection keeps the exact original scope. The upward `has` path reads narrow properties directly. Controls: 32/50 9.588→7.002 s (−27.0%); 128/50 15.395→13.563 s (−11.9%) **M**. The 128/60 candidate exceeded 115 s (journal 3955-3961).
- **Scoped refinement and fused alternatives** (journal 3980-4031): refine existing candidate keys without building another complete kind posting. One deduplication boundary per alternative family; restore the previous domain on failure (a failed literal admission had leaked its kind restriction into the global input domain). Skip other candidate arms when one already covers the entire input scope. "These unions compose alternatives over the same input scope; they do not merge different consumers' scopes." Combined: 128/50 −19.1%, 128/60 −6.8%; 60-rule RSS rose 1,774,993,408→1,868,857,344 B **M**.
- **Guarded projection streaming** (journal 4044-4088): a broad single-reader inliner passed the copy test but failed the next rung: "One syntactic reader can perform many dynamic probes: the predicate runs 16,384 times instead of 1,024." **D: reject the general inliner.** Stream only generated key aliases with no computation, join, or rename. Keep source-span barriers and computed predicates materialized before fan-out. Planning breakdown on one diagnostic: physical planning 2.020 s (fragments 0.801, composition 0.278, ordering 0.071, rendering 0.389, ownership 0.385) versus SQL materialization 6.244 s. "Planning is measurable, but execution remains the larger frontier" **M/D**. Later, ownership discarded streaming decisions (16-layer pipeline 503,800 vs 110,500 VM instructions); fixed in `8d111677` (journal 4203-4207).
- **Template ownership and fragment reuse** (journal 4293-4318) **D**: the template cache admits at most **64 MiB** of SQL and graph strings; a separate **32 MiB** fragment cache reuses per-plan lowering across admitted frontiers. Entries include language, logical plan identity, scope mode, path filter, and stored-statistic decisions. "A changed anchor winner replaces the fragment. Consumer IDs are rebound during physical composition. One-reader posting relations remain inside their consumer statement; indexed membership and range probes retain required indexes." Physical planning 6.173→3.619 s; selection unchanged (11.063→11.049) on 128/60 (journal 4321-4323) **M**.
- **Constructor evidence failure** (journal 4347-4362): lexical-scope utilities ran before the immediate argument-position check. The fix moves the guard first; the rule revision goes to 2; no limit changed. Lesson: put cheap structural rejection before expensive scope analysis.
- **Indexed family boundary** (journal 4407-4413) **D**: fusion put a repeatedly read stage inside a materialized CTE and **discarded its declared lookup index**. The fix keeps one indexed owner when validated consumers read the producer more than once; single-use and unused stages stay fusible. The read graph and conservative occurrence counter select this, with no runtime switch.
- **Instrumentation overhead**: 909 dbstat working-set snapshots consumed 142.096 CPU s (journal 4417) **M**. `malloc_zone_statistics` at every SQL span boundary was removed from CPU tracing; deferred CPU on a 128-file trace went 6.775→2.174 s (journal 4886, 4899-4901) **M**. Lesson: instrumentation must not request censuses implicitly.
- **Shared producer identity** (journal 4429-4446) **D**: repository fingerprints are computed once per immutable version set; evidence keys hash the existing rule digest while full canonical equality still prevents collision-based sharing; shared owners are charged once in memory accounting (the memory test went from charging 54,631 to 3,551,197 B for shared owners). The producer stage limit stays 100 native passes **per logical consumer**, with total launches recorded separately; a global producer allowance had failed at consumer 33 of 120.
- **Public API** (journal 4448-4466): owner indexes used only `__path` because dynamic owner columns lacked integer affinity. Explicit casts of byte-offset owners restored the second seek column. An explicit SQL join order (exported classes before method expansion) brought traits from 148,856,406→28,266,543 VM steps **M**. Lesson: affinity determines index usability.
- **Empty repository gate** (journal 4469-4474): queue source-free metadata and hydrate source only after admission.
- **Candidate transport** (journal 4492-4517) **D**: one result statement did 400 sorts (514 ORDER BY nodes, 38,423,234 VM steps) because transport required `ORDER BY qid,fid,pre` for one-file buffering. Fix: bounded sparse-to-bitmap accumulators for unordered typed result rows, bounded by each file's candidate representation, with no repository-sized ordinal vectors. "The matcher must still receive canonical, sorted, unique ordinals." Contradictory kind constraints now propagate exact empty demand (previously eleven unnecessary relations were built). DISTINCT boundaries for UNION/EXCEPT with empty arms are preserved after a duplicate-row regression.
- **Conditional ownership** (journal 4519-4533): the preceding nodes statement wrote 1,419,253 wide rows (523,686,704 VM steps). "The fused query materializes two wide pure projection unions, then copies the wide data into an automatic join index. The narrow ordinal consumer cannot push its column demand through those forced materialization boundaries." Fix: stream pure column/NULL projection unions. Native sources and deferred columns keep their boundaries.
- **Failure-only snapshots** (journal 4535-4538): main and temporary SQL owners are snapshotted after failure only. "Native virtual-table frames and external source callbacks are not serialized by SQLite backups."
- **Dead-code reachability** (journal 4607-4646): the edge arm expanded matching method names across unrelated classes before checking the owner's protocol status. Fix: protocol classes first plus ordered CROSS JOINs. Edge construction 34.738→2.544 s (388,693,756→68,781,444 VM); whole reachable query 51.819→20.196 s **M**. Rejected: UNION ALL plus one outer DISTINCT removed all sorts but raised edge CPU to 39.815 s and the full query timed out **D**. Single-rule full validation: 4,931 findings, 157.516 s cold CPU **M**.
- **Regex candidates** (journal 4668-4693) **D**: the regex value path put cached matching terms before the admitted-file relation, producing a vocabulary-by-requested-file product. Fix: scan `vidx_file` within admitted files, then seek cached predicate membership; exact literals keep direct term lookup. Snapshot probe: 2.968→0.033 s, 81,708,000→84,000 VM steps for identical 3,876 rows (host SQLite 3.54.1) **M**. A 128/60 control showed no CPU change (18.866→19.175 s).
- **Quality imports** (journal 4733-4751): a separate import-subtree producer receives all import roots through the existing reachability contract (`py_names` 86,986,185→5,588,856 VM) **M**. Conditional ordinals are computed only within four demanded parent kinds, with a LEFT JOIN keeping NULL ordinals for other nodes.
- **Projection scopes** (journal 4753-4778): a narrow `projection_scopes(__path,__start,__end)` relation selects eligible scopes once per file, and all conditional consumers reuse that exact domain. Result queries went from interrupted 89.9 M / 91.8 M VM lower bounds to 288,939 / 275,619 **M**. Rejected: hiding the range bound inside CASE (still fails) **D**. A narrow module-binding relation precedes one identifier scan (64,000 text reads → below 2,000).
- **Column demand** (journal 4781-4817): unused stored columns are removed while logical names survive as NULL expressions in compatibility views; the 32-field deferred limit is unchanged. Functions 53→21 stored columns, classes 55→21, conditional methods 25. Rejected: removing logical names outright (fails SQL binding in unused nested expressions) **D**. Node relation construction: 373,032,217 VM / 5.485 s; four node indexes 4.365 s; ownership 4.295 s **M**.
- **SQL accounting**: 296 engine submissions and 45 SQLite internal executions exceed the fixed 100 reference, which "remains diagnostic … neither removed nor reported as passing" (journal 4806-4808) **D**.

### 1.17 DLRE typed vtab / relation contracts (as stated)

- **First-field-child** (spike S3): input is the request-ID batch via `sqlite3_vtab_in`; output is typed columns; the kind predicate applies after selecting the first field child; one xFilter, two waves, ≤65,536 buffered rows; xNext serves completed columns (`spike-results:126-133`). Backing index `(fid, parent, field, pre)` (`sql-execution-owner.md:16`).
- **`syntax.source_confirmation.v1`** (`source-confirmation-batches.md:15-31`): SQL publishes a narrow request frame `(producer, immutable file, node)` keys; a separate dictionary holds each producer's expression and literal/regex mode. One native filter consumes each dependency wave and outputs matching `(producer, file, node)` keys. "The cursor advances completed output rows only. **It claims no ordering.**" The producer decodes spans internally and reuses source text at identical points within the wave. It processes the requested predicate/key pairs and "never applies all predicates to the union of their scopes". SQL composes matches with relational clauses and matcher confirmation. Frame cardinality is published once during preparation and read by the vtab planner ("not alternative-anchor posting pricing"). Empty demand gates the producer on the outer side of a join. The registry declares domain completion. Discovery labels use dependency and bound-value fingerprints, while runtime sharing compares complete typed definitions. The scalar SQL binding stays exercised by differential tests; no production switch selects it.
- **Generic demand row** (`recovery-plan:48-53`): consumer, source version, file, range, producer, parameters, requested columns; one xFilter; typed rows out; SQL binding exists.
- **Producer stage limit**: 100 native passes per logical consumer (journal 4445).

### 1.18 DLRE lessons, verbatim or near-verbatim

1. Statement counts, VM counts, and submission counts explain results and are not gates (journal 3697; `recovery-plan:77`; `lifecycle:413`).
2. A count-driven proxy chose the wrong work after the repository measurement had already identified anchor selection (journal 3703-3704).
3. Plan-time anchor choice must come from stored SQL statistics, never runtime `count(*)` (journal 3708-3712; `spike-results:89`).
4. Physical planning follows index publication; logical validation can precede it (journal 3711-3713).
5. Compile-complexity limits and cardinality estimates are separate (`spike-results:90-91`).
6. Repeated small ANALYZE calls interact with all live schemas through `sqlite3AnalysisLoad` (`spike-results:49-53`).
7. Pending consumers must not determine live schema count (`lifecycle:12`).
8. Fusion belongs in the physical planner after logical validation; authored limits stay (`spike-results:120-121`).
9. Materialize only for a measured second consumer; repeated query text alone does not justify it (`recovery-plan:58`).
10. One syntactic reader can perform many dynamic probes, so single-reader inlining is unsafe in general (journal 4049-4050).
11. Fusing into a materialized CTE can drop authored indexes (journal 4410).
12. Forced materialization boundaries block column-demand pushdown (journal 4530); a syntactic narrow SELECT is not enough if the producer already built the full row (`recovery-plan:283`).
13. Native batching of a cheap operator gives no CPU win; transport costs stay visible (`spike-results:141-147`).
14. Construction dominates small controls; remove construction first (`spike-results:213-223`).
15. Do not multiply isolated ratios (`spike-results:223`; `recovery-plan:196`).
16. Transaction batching (32 files) is the big publication win; larger caches and grouped statistics are not (`spike-results:163-166`).
17. Count changes alone must not invalidate a physical template; template identity includes the chosen arm (`spike-results:197`; `recovery-plan:254-255`).
18. `partitionBy: file` does not prove opaque SQL distributes over files (`lifecycle:100`).
19. Equal row counts do not establish fresh statistics (`lifecycle:289-291`).
20. Dynamic owner columns without integer affinity disable multi-column seeks (journal 4449-4450).
21. Requiring SQL ORDER BY for transport caused hundreds of sorts; use unordered accumulators and canonicalize at the matcher boundary (journal 4497-4501).
22. Falling current RSS is not memory recovery; swapped pages hide footprint (`spike-results:227-228`).
23. Instrumentation can dominate: dbstat census 142 s, allocator enumeration at every span (journal 4417, 4886).
24. External samplers must target the workload child (`spike-results:58-59`).
25. Never multiply a rule allowance by batch size; never split a repository-wide consumer (`recovery-plan:299-300`).
26. Independent capacity partitions must not share one execution allowance (journal 4428).
27. Put cheap structural guards before expensive lexical scope analysis (journal 4353-4357).
28. Order joins so the selective protocol/owner relation precedes fan-out (journal 4623-4625).
29. Bound regex vocabulary work to admitted files (journal 4668-4692).
30. "No production algorithm flags", no alternate rule interpreter, no rule-ID branches (journal 5015; `lifecycle:24`).

---

## Source 2: N1562 (ner-1562-scale-test-the-re-railer)

### 2.1 Design doc: `docs/sql-scope-and-operational-semantics-design.md` (776 lines)

Status: "proposed design. This document does not describe completed implementation" (`:3`). Everything here is **A** unless marked otherwise.

**Purpose** (`:5-22`). There are two risks: (1) splitting one large statement can create hundreds of unrelated executions; (2) generated SQL can become an interpreter for budgets, ordered matching, and memo costs. Remedies: one statement per exact candidate-scope group; split only when a measured resource limit requires it; intern utility definitions at compile time; compute utility answers from batched semantic demands; keep logical work charges separate from physical producer work; push stateful ordered regions into typed operators; keep ordinary relational composition declarative. "The **8 MiB statement limit** remains a safety check. SQL size is not a planning objective" (`:22`).

**Survey of current implementation** (`:24-99`):
- `query/relational.rs` builds one `SqlCatalogPlan`. `finish` joins every clause through one CTE catalog, then emits one final union and one global order (`:28-32`). The plan reads one **35-column `rerail_matches`** relation whose row kinds distinguish status, matches, captures, navigation, facts, and document maps (`:34`). Problems: every clause shares one preparation unit; the wide union hides each producer's constraints; utility sharing is limited to the CTE catalog (`:36-40`).
  - **Doc/code discrepancy**: at N1562 HEAD, `rerail_matches` is registered only under `#[cfg(test)]` (`native/src/code_rule_dsl/transducer/sql_backend.rs:34-35, 2516-2531`). Production registers ten typed per-row-kind relations (`:2533-2662`). The doc's survey either predates the split or describes the plan text, not the module registry; I cannot tell which.
- `compile/utils.rs` provides deterministic `BTreeMap` utility dependencies, Tarjan components with sorted members, exact transitive closure, and explicit parameter lists. These stay, with catalog-stable semantic identities (`:42-51`).
- **Pointer identities**: anchor selection and relational rendering use `Arc` addresses in memo keys (e.g. `UtilitySelections` keys an `AnchorPlan` by `Arc::as_ptr`). "A pointer identifies one allocation. It does not identify one operation" (`:53-59`).
- Candidate scope "is more than a policy file glob. It includes document eligibility, root selection, completion state, and stable root order" (`:61-68`).
- **Matcher work semantics** (`:70-83`): memoization has observable work behaviour. A remembered verdict replays its recorded logical steps and depth, excluding cache-fill work. Memo entries become clause-local when they read clause state. Nothing is remembered after a limit failure or an unsafe nested clause start. Ordered alternatives evaluate in source order; only reached alternatives consume work or publish effects; capture dependencies can make a later operand depend on an earlier one. "These rules cannot emerge accidentally from SQL query order."
- Evidence adopted (`:85-99`): complete jobs need shared relation graphs, not host loops per node or rule call; shared stages are safe only over their exact input closure; a shared stage needs enough consumers to justify materialization; ordinary Boolean composition must not need continuation records; logical budgets must not change when physical work is shared; producer dependencies are explicit graph edges, not references found in SQL text.

**Terms and identities** (`:101-213`):
- **Candidate scope**: an immutable, ordered set of candidate roots plus status, keyed by `(document_id, node_ordinal)`. It also holds `snapshot_id`, `node_order_version`, `eligible_document_ids`, completion rows for every eligible document, error rows for failed selection, and ordered roots. "Two equal row sets with different failure states are not equal scopes" (`:103-125`).
- **`ScopeRecipe`** (`:127-148`): `language: GrammarDialect, document_kind, file_applicability: FileApplicabilityId, required_text: RequiredTextId, anchor_plan: AnchorPlanId, document_map_mode, node_order_version: u16, selector_contract_version: u16`. `ScopeRecipeId` is a digest of canonical typed fields, not of rendered SQL or a Rust address.
- **`ScopeInstance`** (`:150-167`): `{id, recipe, snapshot, candidates: CandidateSetHandle, completion: CompletionSetHandle}`. The digest covers canonical candidate and completion rows; postings and bitmaps may be used without hashing expanded SQL.
- **Exact-scope group** (`:169-175`): clauses with the same `ScopeInstanceId`, possibly also the same execution profile and document-map generation. "A group must not use a union or an intersection as a substitute for equality. Extra roots change work, failures, ordered effects, and producer demand."
- **ProducerId** (`:177-193`) = digest(contract version, operation, typed parameters, input relation identities, exact scope identity, projected frame schema, execution options). Excludes SQL names, CTE order, addresses, cache capacity.
- **Utility identities** (`:195-213`): `UtilityDefId` (canonical body, parameter schema, language, semantic options); `UtilityComponentId` (sorted UtilityDefIds in one SCC); `UtilityDemandKey` (UtilityDefId, document, node, projected parameter frame, three-valued mode, exclusion state, semantic context). A name is provenance only.

**Statement planning** (`:215-316`):
- `ClauseLogicalPlan {clause, scope_recipe, roots: RelationId, utility_closure, effect_profile, output: OutputContractId}`, with no SQL rendered at this step. Relation and utility nodes are interned by semantic identity, and exact utility closure is computed once per clause. No `Arc::as_ptr` or rendered SQL names (`:217-236`).
- The workspace planner evaluates each distinct `ScopeRecipeId` once and interns equal results as one `ScopeInstanceId`. Different recipes producing the same runtime scope may share (optional; needs a collision-safe equality check after the digest match) (`:238-248`).
- **Execution group key**: `(ScopeInstanceId, EffectProfileId, DocumentMapGeneration, FailureIsolationMode)`. One root relation and one completion relation per group; terminal and utility producers are requested in batches (`:250-267`).
- **Split only after grouping** (`:269-283`): possible limits are SQL AST nodes, bind parameters, estimated rows, and prepared bytecode. Deterministic algorithm: sort by stable `ClauseId`; add each clause with its exact dependency closure; reuse dependencies already in the packet; close the packet before the next clause would exceed a limit; record limit, measured value, and clause range. "A split does not change relation identity. Shared producer pages remain available to all packets in the group."
- **`PlanShapeId`**: SQL AST with clause constants replaced by typed parameters. Same shape → one prepared handle. "Prepared-shape reuse does not authorize relation sharing." Report `exact_scope_groups, statement_packets, distinct_plan_shapes, statement_preparations, statement_executions`; "A low preparation count must not hide excessive executions" (`:285-303`).
- **No statement storm** (`:305-316`): report statement counts before execution; full-catalog tests set an expected range; "A large increase fails the performance gate"; never fix by raising a timeout. "No host loop is permitted per node, terminal, utility call, or recursive round. The only host loop is over admitted statement packets."

**Compile-time utility deduplication** (`:317-389`):
- Canonical form includes the typed operation tree, positional parameter names, language/dialect, three-valued behaviour, capture reads and writes, file-state and clause-state reads, stop and recursion semantics, and contract version. It excludes source paths and local SQL names (`:319-334`).
- One `UtilityComponentPlan` per recursive SCC, with members at stable indexes sorted by `UtilityDefId` and transitions holding typed argument relabeling and stop conditions. It is compiled once per component and instantiated by batched `UtilityDemandKey` rows, keeping the existing recursive frame and rule-stop behaviour (`:336-346`).
- **Demand/answer relations** (`:348-380`):
  - `utility_demand(producer_id, demand_id, document_id, node, frame_id, exclusion_id, three_valued)`
  - `utility_answer(producer_id, demand_id, verdict, answer_frame_id, effect_summary_id, complete, error_code)`
  - Demand rows are `DISTINCT` on the full semantic key before execution. The native binding receives batched keys through `sqlite3_vtab_in`, and the ordinary-table binding reads the same schema.
- "One physical answer can serve many clauses. Each clause still applies its own logical effect summary. Therefore, sharing cannot increase or reduce a clause's budget. This property requires a logical effect ledger" (`:382-388`).

**Operational semantics: the split** (`:390-446`) — the central part of this source:
- Rule: every DSL operation gets one classification, either (1) a declarative relation contract defining complete behaviour, or (2) a pushed-down typed operator owning the behaviour. "No operation can rely on SQLite row-production order. No generated CTE network can act as a general continuation machine" (`:392-400`).
- **Declarative operators** (closed set behaviour) (`:402-416`): kind, field, text, and pattern predicates; capture-free conjunctions over the same input frame; capture-free negation; semi-joins and anti-joins; bounded counts over complete child relations; navigation with a declared stable order; capture projection after one answer is selected; file applicability and required-text filtering. Each contract defines key columns, bag behaviour, null behaviour, completion rows, and guaranteed order.
- **Ordered operators**: a typed `ordered_region` operator is used "when evaluation order affects observable behavior". Inputs are batched demands plus an immutable operation program (`:418-436`). It owns:
  - alternatives with reached-only work or effects;
  - capture-dependent conjunction operands;
  - first-match capture publication;
  - `each` exclusion and stable enumeration;
  - ordered relation searches with dependent captures;
  - recursive utility regions with ordered stops.
  It returns answers and effect summaries, not continuation rows. "A region must be maximal. The compiler must not push one node at a time. A maximal region avoids rebuilding a SQL interpreter through many tiny calls."
- **Why**: ordering is observable because work charges, effects (captures), first-witness publication, and exclusion/multiplicity depend on it (`:70-83, 418-436`).
- **Ordered alternatives with no dependent effects** (`:438-446`): these stay declarative when every branch reads the same frame and has no reached-only effects. The branch relation carries `choice_order` and three-valued verdicts; a reducer selects the first decisive branch by declared order, preserving unknown barriers and capture publication rules. Without an independence proof, use `ordered_region`.
- **Operator assignment table** (`:539-557`):

| DSL behaviour | Default owner | Reason |
| --- | --- | --- |
| File applicability | Declarative scope relation | Closed file set |
| Anchor selection | Typed candidate producer | Uses postings and returns exact roots |
| Atomic predicates | Declarative relation or terminal producer | Closed answers |
| Independent `all` | Declarative joins | No sequential frame dependency |
| Dependent `all` | `ordered_region` | Later inputs read earlier captures |
| Independent `any` | Declarative ordered reducer | Same input frame and closed effects |
| Effectful `any` | `ordered_region` | Reached branches affect work |
| Capture-free relation test | Declarative semi-join | No first witness publication |
| First-witness relation | Ordered navigation producer | Stable witness order is observable |
| `each` | Ordered enumeration producer | Exclusion and multiplicity are stateful |
| Acyclic pure utility | Utility answer producer | Demand deduplication |
| Recursive utility SCC | Component answer producer | Owns fixed point and stop rules |
| Logical budget | Effect ledger | Must survive physical sharing |
| Physical memo | Relation broker | Optimization only |
| Output order | Declarative final merge | Stable clause, file, node, and choice keys |

**Budget contract** (`:448-488`): "Physical work is not logical work."
```
EffectSummary { replay_work: u64, relative_peak_depth: u16, fills: Box<[LogicalFill]> }
LogicalFill { key: LogicalFillKey, work: u64, owner: FillOwner }
enum FillOwner { File, Clause }
```
Each clause has a `LogicalEffectLedger` per file. Applying an answer: (1) check relative depth against the logical depth limit; (2) charge `replay_work`; (3) charge each fill new for its declared owner; (4) stop before publishing if a limit is exceeded; (5) publish reached effects only after the charge succeeds. "A replay charges recorded work but does not charge completed fills again. Clause-local fills remain clause-local, even when physical answers are shared." The summary belongs to the semantic answer and is independent of native cache hits and SQL materialization.

**Memo contract** (`:490-512`): memoization is a physical optimization, not a source of truth. Key = `UtilityDemandKey` plus required owner context, never a pointer. An entry stores verdict, answer-frame delta, effect summary, completion state, and semantic generation. Nothing is published after a limit failure. Reuse is rejected when clause-state ownership differs. A hit applies the stored effect summary to the caller's ledger.

**Depth and memory limits** (`:514-525`): nested utility depth is a declarative logical limit enforced through `relative_peak_depth` after reuse. Native stack bytes are a safety limit and "must not define cross-backend query truth"; a pushed operator uses explicit frames or fails closed before stack exhaustion. Memory admission, deadlines, and cancellation are execution controls producing typed incomplete status rows; "They do not change a false result into a true result."

**Pure-SQL binding** (`:527-537`): every typed relation keeps an ordinary-table or pure-SQL test binding proving relation algebra, keys, completion, and output extraction. It "must not implement a general procedural matcher". For `ordered_region`, tests provide the same typed answer and effect relations as the native binding. Small closed reducers may use SQL windows or bounded recursive CTEs. "A policy that requires an unavailable operational operator fails closed before execution. There is no matcher fallback."

**RelationBroker** (`:559-581`): execution-lifetime, scoped to one workspace or policy request, not process-global.
```rust
trait RelationBroker {
    fn request(&mut self, producer: ProducerId, demands: DemandBatch) -> RelationHandle;
    fn status(&self, producer: ProducerId) -> ProducerStatus;
}
```
A request merges missing demand keys with prior demand, and the producer evaluates only those. Readers get immutable pages and completion rows. Sharing requires an exact `ProducerId` match, including exact scope and typed frame identity. "Materialization is a physical choice. The planner considers it only for at least two consumers. Statistics decide between recomputation, broker pages, and SQL backing."

**SQL AST boundary** (`:583-607`): physical planning finishes before rendering. AST nodes: `RelationScan, DemandScan, Join, SemiJoin, AntiJoin, OrderedReduce, Aggregate, Project, Union, Materialize, OutputMerge`. Each stores input `RelationId`s; dependency order follows those edges; "The renderer does not parse generated SQL to find dependencies." Rendered names are late, local, and irrelevant to identity.

**Statistics** (`:609-627`): stored in SQL-visible tables, "not hide selectivity guesses in Rust constants". Initial set: documents by language and dialect; nodes by document and kind; kind frequencies from `fts5vocab` over `search_kind_fts`; candidate counts per `ScopeRecipeId`; demand counts per producer family; average and maximum answer multiplicity; completion and failure counts. Vtabs use these for `estimatedRows` and "declare only ordering that their cursor guarantees". Statistics select physical shape and never change semantic identities.

**Execution sequence** (`:629-647`): (1) compile typed logical plans; (2) intern utilities and SCCs; (3) evaluate distinct scope recipes for the snapshot; (4) intern scope instances; (5) form exact-scope groups; (6) build demand closure and physical plans per group; (7) split only at recorded limits; (8) prepare each distinct SQL shape once; (9) execute admitted packets and batch native demands; (10) apply effect summaries to clause-local ledgers; (11) merge outputs by declared stable keys; (12) commit only complete results. A failed producer emits status for every affected file and clause dependency. "Negative work requires explicit completion rows."

**Rejected designs** (`:649-684`) **D**:
- One statement for the whole catalog (repeats the current preparation and memory failure; blocks narrow vtab pushdown).
- One statement per clause without grouping (duplicates scope selection, producer reads, and preparation; statement storm).
- Grouping similar scopes ("A union, superset, or matching file glob is not exact scope equality").
- Pointer-address memoization.
- SQL continuation machines ("A generated state row per Boolean operand, utility call, or recursive round rebuilds an interpreter. It causes transfer traffic and blocks native batching").
- Native shortcuts without a relation contract (they create a second operator definition; every pushed operator keeps a typed contract and an ordinary-table substitution test).
- Charging physical work to rule budgets.

**Test plan** (`:686-756`): scope identity cases (equal recipes share; equal canonical rows share after full verification; one extra root prevents sharing; different order versions prevent sharing; successful-empty and failed-empty do not share; a changed document map prevents sharing; overlapping scopes do not share derived relations). Catalogs of 1, 2, 32, and 1,000 clauses measuring groups, packets, shapes, preparations, executions. Utility dedup cases. Operational parity against native matcher oracles at every boundary (limit one step before and at the exact boundary; memo hit replays work excluding fills; a shared answer gets separate clause charges; later alternatives not reached; unknown and capture publication block later alternatives; later conjunct reads earlier capture; `each` preserves exclusion, multiplicity, order; recursive depth uses stored relative peak; failed answers not memoized), each run with sharing on and off. Pure-SQL substitution with independently authored fixtures ("Do not generate the fixture from compiler SQL"). Full-corpus gate: every policy compiles and completes with parity; no structural safety limit exceeded; statement and producer counts recorded; memory bounded; compile, prepare, and execute reported separately. The benchmark reports CPU, wall, max RSS, and parity; "SQL bytes remain diagnostic data only."

**Implementation order** (`:758-776`): 14 steps, from semantic IDs and replacing pointer keys, through `ClauseLogicalPlan`, ScopeInstance interning, exact-scope groups, split per group, one acyclic utility's demand/answer relations, the effect ledger, one maximal `ordered_region` slice, demand-driven recursive components, replacing the wide union with narrow typed vtabs, building SQL from the post-planning AST, statistics-based physical choices, to the full catalog before the canonical benchmark. A TDD-red and a separate TDD-green commit per behaviour change; unsupported lowering keeps failing closed.

### 2.2 Typed vtab schemas in `native/src/code_rule_dsl/transducer/sql_backend.rs` (code at HEAD, observed)

**Module mechanics**:
- Header (`:1-5`): "SQL-demanded packed batch relation. Registration stores immutable verified programs, tapes, limits, and a sparse task grid. SQLite calls `xFilter` with the batch token. The first call executes the packed backend and stores its normalized relation. No host callback or semantic matcher can be registered here."
- Modules are `Module::eponymous_only_module()` with `VTabConfig::DirectOnly` (`:2487, 2687`).
- `MatchRegistry` maps an i64 token to an `Arc<MatchBatch>` (`:2318-2443`). `register(demand)` returns a new token. `MatchBatch.materialized` is a `OnceLock`: the first xFilter executes `PackedDemand::execute` exactly once (`demand.take()`), and later filters reuse `Arc<Vec<PackedRow>>` (`:2290-2316`). Execution runs under `catch_unwind`; a panic yields one batch-status row `complete=0, "packed producer panicked"` (`:2271-2288`). `remove_batch(token)` releases it (`:2428-2433`).
- Test evidence (code): two scans → `virtual_table_scans: 2, filters: 2, demand_executions: 1, backend_invocations: 1` (`:3130-3200`). An empty batch is inert (0 executions, 0 filters) until SQL scans it, then yields exactly one row `(row_kind=1, NULL, NULL, complete=1, NULL)` (`:3286-3322`). Metal backend: one command buffer and one packed dispatch per batch (`:3203-3230`).

**`PackedDemand`** (the typed input frame) (`:326-343`): `backend: ExecutionBackend, programs: Vec<Program>, producer_kinds: Vec<ProducerKind>, value_facts/semantic_facts/parse_facts/transform_facts/file_facts: Vec<Option<…Descriptor>>` (one per program), `document_maps: Vec<DocumentMapSpan>, file_paths: Vec<String>, files: Vec<TapeDoc>, anchors: Vec<Vec<Vec<u32>>>` ([file][program] → root ordinals), `task_grid: Vec<Vec<bool>>` ([file][program] demanded), `sound_under_errors: Vec<bool>, result_limits: Vec<u32>` (per program), `failure_injections` (test).
- Validation (`:364-383`): dimensions must agree, and "roots for an excluded file/producer pair" are rejected (test `:3066-3086`). The sparse task grid is the demand.
- `ProducerKind` (`:48-57`): `Terminal, Pattern, Navigation, ValueFact, SemanticFact, ParseFact, FileFact, TransformFact`. Each fact kind needs exactly its descriptor (`:452-555`). Navigation programs must be navigation descriptors with anchors `[0]` (whole file) (`:430-448`).
- Descriptors: `ValueFactDescriptor {language, follow, expression_kinds, reading, predicate: Option<ValueTest>}`. Simple equality emits a canonical value for SQL comparison, compound predicates emit a three-valued verdict, and SQL composes either (`:160-171`). Value facts are known (verdict 1 + text), missing (0), or unknown (NULL) (`:184-210`). `SemanticFactSource` = `Resolves | Binding{test,request} | Binds{test,request} | Arguments | FormatArgs | Layout | Calls`. "The descriptor emits only predicate-local rows. Nested user rules remain relational operands and are never executed by this producer" (`:100-142`). `ParseFactDescriptor {grammar, host_kinds, reading}`: "Nested user rules remain relational SQL operands" (`:82-89`). `FileFactDescriptor {regex, path: bool}` (`:76-80`). `TransformFactDescriptor {transform, outputs: Vec<(String,String)>, sources: Vec<usize>}` consumes captures of other producers in the same batch and file (`:58-63`, `:900-1002`).
- **Hard partitions** (`:345-362, 589-630`): `stable_partition_ranges` binary-splits the file range until `packed_workspace_fits` (or a single file). Partitions come out in ascending file order and each records `HardPartition {cause: "packed-workspace-capacity", limit, task_range}`. Each partition is one backend invocation (`:666-678`).
- **Counters** `PackedExecutionCounters` (`:302-318`): registrations, virtual_table_scans, filters, demand_executions, backend_invocations, hard_partitions, partitions, demanded_producers, terminal_tasks, command_buffers, packed_dispatches, anchor_dispatches, matcher_calls, host_callbacks.
- **Status semantics**: one leading `BatchStatus` row is inserted at index 0 on success (`:1786`). On backend error: a batch status `complete=0` plus a `ProducerStatus` `complete=0` row for every demanded (file, producer) carrying the message (`:685-710`). Per-producer statuses: complete with verdict (true/false = any matches) or incomplete with an error code. Codes seen: `source-partial` (program not sound under errors and file has syntax errors) (`:723-735`), `parse-fact-capacity`, `parse-invalid-map`, `parse-incomplete`, `transform-fact-failure`, `transform-fact-inconsistent`, `transform-fact-capacity`, `semantic-fact-capacity`, `WorkLimit` (navigation > 2^26 edges) (`:1580-1605`). On failure, provisional rows are truncated before the status row (e.g. parse `:874`).
- Navigation producer: emits **every parent→child edge of the file** (`for parent in 0..len`), with `axis_kind=1`, `axis_order=child order`, `field_id`, `related_named` (`:1576-1620`, constructor `:2013-2036`). It is whole-file, not demand-restricted.
- `document_id` is filled in afterwards when all maps for a source file point to one generated document; `DocumentMap` rows are appended after all others (`:1768-1785`).

**Broad column numbering** (shared by all modules; `column()` maps a module-local index through `module.columns` to these) (`:2933-2981`):
0 batch, 1 row_kind, 2 producer_id, 3 file_index, 4 task_index, 5 match_index, 6 capture_index, 7 capture_occurrence, 8 capture_sequence_start, 9 capture_sequence_count, 10 node, 11 related_node, 12 axis_kind, 13 axis_order, 14 fact_kind, 15 document_id, 16 mapped_document_id, 17 start_ordinal, 18 output_label, 19 capture_label, 20 text_value, 21 start_byte, 22 end_byte, 23 verdict, 24 complete, 25 work_used, 26 error_code, 27 field_id, 28 related_named, 29 source_text, 30 structural_key, 31 named_leaf, 32 capture_kind, 33 capture_named, 34 equality_occurrence.
`RowKind` (`:1797-1806`): BatchStatus=1, ProducerStatus=2, TerminalMatch=3, Capture=4, Navigation=5, Fact=6, DocumentMap=7.

**Registered relations** (`:2484-2662`). Each schema begins `batch INTEGER HIDDEN`. Ordering lists the "guaranteed" sort order a cursor may consume:

| Relation | Row kind / fact family | Columns (after `batch HIDDEN`) | Declared ordering | Seeded `estimated_rows` |
| --- | --- | --- | --- | ---: |
| `rerail_batch_status` | BatchStatus | `complete NOT NULL, error_code` | none | 1 |
| `rerail_producer_status` | ProducerStatus | `producer_id, file_index, verdict, complete NOT NULL, work_used, error_code` | file_index, producer_id | 10,000 |
| `rerail_terminal_match` | TerminalMatch | `producer_id, file_index, task_index, match_index, node, document_id, mapped_document_id, start_ordinal, output_label, start_byte, end_byte, verdict` | file, producer, task, match, node | 100,000 |
| `rerail_capture` | Capture | `producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, output_label, capture_label, text_value, start_byte, end_byte, verdict, field_id, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence` | file, producer, task, match, capture_index, capture_occurrence, node | 500,000 |
| `rerail_navigation` | Navigation | `producer_id, file_index, task_index, match_index, node, related_node, axis_kind, axis_order, start_byte, end_byte, verdict, complete NOT NULL, work_used, error_code, field_id, related_named` | file, producer, task, match, axis_order, node, related_node | 1,000,000 |
| `rerail_semantic_fact` | Fact / Semantic | all 34 non-row_kind columns (same as the broad schema minus `row_kind`) | file, producer, task, match, fact_kind, capture_index, node | 100,000 |
| `rerail_value_fact` | Fact / Value | `producer_id, file_index, task_index, match_index, node, fact_kind, text_value, verdict` | (FACT_ORDER) | 100,000 |
| `rerail_parse_fact` | Fact / Parse | `producer_id, file_index, task_index, match_index, capture_index, node, axis_order, fact_kind, start_byte, end_byte, verdict, source_text, named_leaf, capture_kind, capture_named` | (FACT_ORDER) | 10,000 |
| `rerail_transform_fact` | Fact / Transform | `producer_id, file_index, match_index, node, related_node, fact_kind, output_label, capture_label, text_value, verdict, source_text` | (FACT_ORDER; note no `task_index` column) | 10,000 |
| `rerail_document_map` | DocumentMap | `file_index, match_index, node, related_node, document_id, mapped_document_id, start_byte, end_byte` | file, match, node, related_node | 1,000 |
| `rerail_matches` (`#[cfg(test)]` only) | all | 35 broad columns incl. `row_kind NOT NULL`, `complete NOT NULL` | none | 100,000 (constant) |

Column-index arrays are at `:2445-2471`; schema strings at `:2523-2654`.

**Statistics source** (`:2489-2513`): `CREATE TABLE IF NOT EXISTS rerail_relation_statistics(relation TEXT PRIMARY KEY, estimated_rows INTEGER NOT NULL CHECK(estimated_rows > 0))` is seeded with `INSERT OR IGNORE` constants (the values in the table above) and read once at registration. These are SQL-visible but are fixed seeds, not measured counts.

**xBestIndex: pushed constraints** (`:2699-2795`):
- **`batch = ?` (EQ) is mandatory.** Without it the plan gets `idx_num 0`, cost `f64::MAX`, rows `i64::MAX`, and xFilter errors "requires one batch equality constraint" (`:2711-2720, 2865-2869`).
- Optionally pushed, all with `omit = true`: `producer_id = ?` (EQ), upgraded to **`producer_id IN (…)` via `sqlite3_vtab_in`** when `is_in_constraint` and `set_in_constraint(true)` succeed (flag `USE_PRODUCER_IN`); `file_index = ?` (EQ); `node >= ?`; `node <= ?` (`:2728-2774`). Flags: USE_BATCH=1, USE_PRODUCER=2, USE_FILE=4, USE_NODE_MIN=8, USE_NODE_MAX=16, USE_PRODUCER_IN=32 (`:37-43`).
- ORDER BY is consumed only if the request is non-empty, all ascending, and a prefix of the module's declared ordering (`:2775-2785`).
- Estimate: `estimated_rows = seeded / 4^(pushed_constraints − 1)`; cost = rows (`:2786-2793`). This is a heuristic divisor, not a statistic.
- Fact-family relations also restrict to producers of that family (`family_producers`) (`:2896-2909`).

**xFilter / xNext** (`:2858-2931`): xFilter reads the token, builds a `BTreeSet` of producers from IN values, sets file and node bounds, increments `filters`, triggers the once-only materialization, and then `seek_relation_row` **linearly scans the whole batch row vector**, skipping rows that fail the row kind, family, producer, file, or node filters (`:2828-2856`). xNext advances and seeks again. No per-file or per-producer index exists inside the cursor: pushed constraints are row filters over a sequential scan of all rows in the batch.

Ordering is not independently verified: I checked that rows are produced file-major (partitions ascending), then program-major, then per-producer match order. I did not verify for every producer that `task_index` is non-decreasing within `(file, producer)`, as TERMINAL_MATCH_ORDER implies (the generic path sets `task_index = result.task_ordinal()` while enumerating results by `match_index`, `:1661-1667`). Value facts set `match_index = task_index` (`:2097-2098`).

---

## Source 3: SSC (sql-set-composition)

### 3.1 `docs/sql-structural-execution-plan.md` (proposed, 2026-10-26; baseline `c6ff8110`)

Status: "proposed transition plan … changes no engine behavior" (`:3`). The integration reference is `Pulling-it-all-together` at `0b941fb4` (`:5`).
- **Thesis** (`:7-9`): "The target compiler emits a graph of local pattern groups, shared facts, and relational operators. The unit of execution is a producer or a compiled group over a bounded document batch. The cohort interpreter remains a migration oracle. It is absent from the final production execution path."

**Evidence and governing decisions** (`:11-35`):
- The full workload submits **54.4 M roots** and records **385.5 M chained control advances**. Removing 8.26 M owned semantic requests cuts instructions only 0.48% against the adjacent control; "This experiment removes leaf transport without removing the composition that generates those requests" (`:13-16`) **M**.
- Sources cited: sibling `findings-contract.md` §3, §7; decisions 0005, 0006, 0007; `spec-transducer.md` §0; `spec.md` §9, §21; prior-work topics 5, 6, 7, 10; lessons 11, 12, 14, 16, 17 (`:18-26`).
- "The findings contract and named-fact decision remain proposed in the sibling decision index. This plan proposes their adoption as prerequisites." "Later decisions take precedence over older examples that put every per-document analysis inside the transducer" (`:28-31`).
- "The prior work supports selective anchors, bounded tape reads, narrow facts, and explicit materialization boundaries. It does not support a global node table, a generic SQL interpreter, or an unconditional scan for every rule" (`:33-35`).

**Required output semantics** (`:37-74`) **A**:
```
finding(rule, code, path, line [, declared_column, declared_tag])
coverage(rule, document, Complete | Unknown(reason))
```
- "Order, duplicate witnesses, incidental captures, utility depth peaks, and memo replay charges do not define finding identity. Explicit priorities remain rule semantics. The compiler lowers them to exclusions or priority relations." Captures that affect a later condition or a declared output location are kept; presentation-only captures may be omitted (`:46-49`).
- One consolidation step maps report nodes to locations, applies suppressions, dedupes keys, and computes additions and retractions. Payloads (message/fix) are requested only for findings that need them. "Native rehydration can supply a valid witness for those keys. It cannot decide whether the replacement engine found a match" (`:51-54`).
- Example: `has(foo($X)) AND has(bar($X))` needs a join on the declared equality of `$X`; two independent Boolean existence tests are incorrect. Required export tuples are kept until the join finishes; existence-only outputs may stop after one valid witness (`:56-59`).
- "Finding-set equality alone does not prove that every legacy choice can become an unordered union. A later condition can read a capture chosen by an earlier alternative. Phase 0 records the intended semantics … The compiler preserves an intended priority through explicit algebra. It does not infer permission to change finding keys from witness freedom" (`:61-64`).
- "An empty complete relation means false. An incomplete relation does not prove absence. Anti-joins and universal conditions require complete coverage of their probe domain. Legacy logical unknown remains distinct from resource exhaustion during migration" (`:66-69`).
- A domain is published only after its required producers succeed. Failure gives `Unknown` for affected rule/document pairs and discards their provisional outputs; other documents finish. Cache hits, eviction, and scheduling must not change complete finding sets (`:71-74`).

**Target plan and scheduler** (`:76-127`):
- Mermaid flow: Rules + documents → typed logical plan; statistics in SQL → physical planner → selective anchor demands → {compiled local pattern groups, shared native fact producers} → named derived relations → {dependent local groups, SQL joins/anti-joins/aggregates/fixpoints} → finding keys and coverage (`:78-92`).
- "The graph is stratified … An arbitrary cycle across SQL and native execution is a compile error. A declared positive recursive component has one fixpoint owner" (`:94-96`).
- **Plan records** (`:98-117`), verbatim:
```
RelationSpec {
  id, schema, key, functional_dependencies, partition,
  semantic_digest, inputs, completeness_domain, estimated_rows
}
Stage {
  id, input_relations, output_relations,
  physical_operator, scope_digest, memory_bound, reuse_count
}
ProgramGroup {
  grammar, exact_scope_digest, canonical_programs,
  shared_prefixes, labels, required_facts, export_schemas
}
Plan {
  stages, dependencies, SQL_statements, prepared_parameters,
  statistics_decisions, execution_budget, finding_projection
}
```
- `physical_operator` selects a local program, native fact producer, relational query, or declared fixpoint, "never … an opaque `AstMatcher` remainder". A program group runs across many documents, and many labels share one traversal. "The scheduler does not create a general continuation frame for each `all`, `any`, `not`, or utility return" (`:119-122`).
- Local programs may contain bounded cursor state and branches. CPU reference executor and CPU JIT share set semantics and export schemas. Metal uses the same contracts with compact task arrays and declared resource bounds. "Moving the existing frame stack to Metal is outside this plan" (`:124-127`).

**Initial relation schemas** (`:129-161`). These are targets, not current table names. Document and node IDs are stable within a plan invocation; persistent cache keys add source, grammar, and producer identities; atoms are interned integers with exact value equality.

| Relation | Columns | Key and purpose |
| --- | --- | --- |
| `py.call` | `doc, call, callee` | `(doc, call)` |
| `py.call_site` | `doc, report_node, call, ref_node` | `(doc, report_node)`; valid callee positions incl. transparent parens |
| `py.exec_context` | `doc, node, owner, context_kind` | `(doc, node)`; nearest applicable execution boundary |
| `py.call_arg` | `doc, call, arg, ordinal, keyword_atom, spread_kind` | `(doc, call, arg)`; signature-independent syntax facts |
| `py.binding_site` | `doc, binding, site, value_node, element_index` | binding/site key; assignment and destructuring |
| `py.ref_resolution` | `doc, ref_node, reading_mode, binding, qualname_atom, status` | `(doc, ref_node, reading_mode)` within one semantic configuration |
| `wanted_symbol` | `label, symbol_atom, requirement` | literal policy relation, compiled once |
| `t_export_<schema>` | `group, label, doc, root, <typed exports>` | distinct tuples under declared schema |
| `t_match` | `group, label, doc, report_node` | positive results before location consolidation |
| `t_outcome` | `producer, doc, domain, complete, reason` | coverage for the exact demanded domain |

- Ambiguity is explicit in `status` or a multi-valued relation; the compiler cannot assume one binding per reference. `py.exec_context` respects function bodies, lambda bodies, class boundaries, defaults, decorators ("not 'any enclosing async function'"). `report_node` is separate from `ref_node` because normalization can change the line. Embedded and parsed documents keep their own identities and source maps (`:149-156`).
- "Each fact has one owner for its semantic identity and covered domain. Selective resolution can cover only a requested reference set. The cache records that set, including completed negative results. A partial cache entry never implies document-wide completeness. Reuse requires compatible inputs and domain containment, not only an equal document id" (`:158-161`).

**First conversion** (`:163-233`): ASYNC210, ASYNC221, ASYNC222, ASYNC230, including argument and binding conditions. "The compiler recognizes generic algebra and producer contracts. It contains no checks for these rule ids." Physical graph: document scope + call anchors → shared callee-position/exec-context program → distinct callee refs in relevant contexts → one resolution batch → join `wanted_symbol` → demand argument or binding facts only for surviving branches → compiled local checks + typed joins → finding-key projection + complete-domain publication.
- Example SQL (`:184-205`): CTEs `sites AS MATERIALIZED (… FROM local_call_sites WHERE demand = :call_context_demand)`, `refs AS MATERIALIZED (SELECT DISTINCT doc, ref_node FROM sites)`, `resolved AS MATERIALIZED (… FROM resolve_refs WHERE demand = (SELECT ref_demand(doc, ref_node, :semantic_config) FROM refs))`, then a `SELECT DISTINCT w.label, s.doc, s.report_node` join on `wanted_symbol` and `sites`.
- `local_call_sites` returns completed structural exports from one shared group; `resolve_refs` returns completed resolution rows from one batch. "Neither virtual table emits interpreter instructions, search edges, or suspended frames. Empty demand means no work. Missing required demand is an error, not permission for a full scan." `wanted_symbol` holds only the active group's labels, and every label in a group shares the same candidate scope (`:207-212`).
- "`MATERIALIZED` protects a reused producer boundary … The plan must check `xFilter` counts and actual loops. The keyword alone does not prove one execution." Join order may change with statistics while data dependencies stay fixed (`:214-216`).
- Work bound: anchor reads + local checks over selected calls and bounded neighbourhoods + distinct requested (document, reference, semantic configuration) resolutions + demanded argument and binding rows + necessary joins and output tuples. "There is no factor for utility-stack depth or repeated evaluation of the same prefix by each rule" (`:222-233`).

**Compiler transformations** (`:235-261`):

| Source construct | Target |
| --- | --- |
| Local kind, field, text, pattern | One compact local program with selective anchors |
| Pure conjunction | Fused tests, ordered by cost and rejection rate |
| Disjunction | Shared prefix plus labelled alternatives; set union across independent exports |
| Negation | Local absence test or SQL anti-join against a complete domain |
| Capture-dependent composition | Typed export and equality join, with the declared comparison mode |
| Small non-recursive utility | Canonical inline expression |
| Shared local utility | Shared program region by digest |
| Shared relational utility | Named derived relation with one physical owner |
| Tree relation | Anchored posting access or bounded tape neighbourhood |
| Count or ordinal | Local bounded reduction or grouped relation with explicit order requirements |
| Graph recursion | Positive or stratified fixpoint over typed edges and newly discovered tuples |
| Parse, binding, arguments, values | Batch producer plus dependent local programs or joins |

- Capture liveness, equality modes, dependencies, keys, and effects are computed before physical planning; existence outputs are distinguished from all-tuple exports. "Explicit priority becomes an exclusion relation, not accidental traversal order. Unstratified negation or a dependency cycle requires a semantic design decision before catalog completion. It never triggers a generic interpreter fallback" (`:252-256`).
- "Fusion requires identical candidate scopes, grammar, semantic configuration, and compatible output obligations … Different scopes remain separate consumers, even when they reuse compatible immutable document facts. The planner does not widen a selective scope to a union merely to create a larger batch" (`:258-261`).

**Physical planning, storage, SQL budget** (`:263-297`):
- One bounded lookup reads stored kind, value, edge, and fact statistics; statistic keys and decision intervals are recorded with the plan; anchors, traversal direction, join order, grouping, and materialization are chosen before execution; "It never recounts every batch in Rust or through runtime `count(*)` probes." Missing statistics use a documented conservative plan, shown in the plan report (`:265-269`).
- "The accepted anchor index remains mandatory. Construction cost has a separate report." Unanchored inner plans read their scope's neighbourhood. "The planner does not build an ancestor-pair table or decode every parent to answer a local relation" (`:271-273`).
- "Single-consumer local intermediates stream through typed buffers or inline SQL. Reused or recursive derived relations have one narrow materialization and explicit indexes. Rows contain integer identities and declared values. They do not contain full tapes, general capture maps, or JSON unions. Materialized results survive all consumers in their scope, then release their resident storage. Persistent facts use semantic digests and input generations, with explicit completeness records" (`:275-279`).
- **"The frozen job retains the hard budget of 100 rule-execution SQL statements"**, including rule-triggered fact materialization and recursive execution statements; preparation, diagnostics, and independent index construction are reported separately. `job statements = shared SQL producer stages + bounded consumer statement groups across resident partitions + explicit additional stages <= 100`. The first family targets one consumer statement per resident partition. "A recursive CTE counts as one execution. An external loop that reissues SQL counts every execution." The compiler rejects plans whose declared stages exceed the budget, and runtime tracing checks actual counts including hidden producer statements (`:281-297`) **D**.

**Phases and exit gates** (`:299-340`):
- Phase 0: contract and inventory (finding-set adapter; logical-unknown truth tables; semantic-difference ledger; utility/program census). Phase 1: executable target (typed contracts; independent reference implementation for the async family). Phase 2: generic compiler. Phase 3: structural and capture families (all/any/not, bounded walks, stops, counts, ordinals, equality joins). Phase 4: recursive and semantic families. Phase 5: incremental and integration. Phase 6: cutover with one production compiler, the old scheduler removed, and CPU/JIT and Metal behind the same contracts, "no silent CPU fallback from Metal" (`:304-312`).
- Proposed Phase 2 go/no-go targets **A** (`:318-329`): at least 2× lower execution CPU than native for the complete family; generated within 1.25× of the reference shape; zero continuation advances; one resolution per distinct demanded reference and configuration; no repeated shared producer for another label in the group; doubling identical-predicate labels leaves producer input work unchanged; unrelated nodes outside selected neighbourhoods cause no extra matcher visits; zero resource unknowns on complete fixtures. "These are proposed acceptance thresholds, not measured improvements."
- Full-job hard ceilings **D**: **CPU ratio 1.30, median edit ratio 1.50, RSS 1.5 GiB, 100 SQL executions**. "The target is to outperform native. Passing the ratio ceiling alone does not establish the product's one-to-two-second audit target." The shipped catalog and data-layer workloads have separate, non-interchangeable reports (`:332-334`). Construction, compile/prepare, execution, and end-to-end are reported separately; "The replacement must also pass the CPU ceiling on execution alone, so construction costs cannot hide a matching regression." At least three alternating samples. CPU, wall, and RSS are acceptance evidence; instructions, visits, and dispatch counts explain (`:336-340`).

**Instrumentation** (`:342-360`): plan dumps include schemas, keys, dependencies, exact scopes, generated SQL, materialization owners, and statistics decisions; per-stage input/output rows, distinct keys, node visits, bytes, CPU, wall, cache coverage, outcome counts; producer versus consumer work; total versus exclusive time; parallel stage times not added to wall. Profiler flags: repeated `(producer digest, input identity, covered domain)`; a root entering incompatible copies of the same candidate scan; a one-consumer materialization without a boundary need; join multiplicity exceeding the declared key bound; **"A virtual table filtered once per row instead of once per batch"**; an opaque matcher callback in a converted family; missing coverage behind an anti-join, or cache-dependent complete findings.

**Integration and completion** (`:362-387`): target module boundaries follow sibling `engine/plan`, `engine/facts`, `engine/transducer`, `engine/vtab`, `engine/compose`. One production cutover after complete coverage; "no user-facing opt-in paths and no per-rule interpreter fallback after cutover". "The pure-SQL binding remains a separate backend contract with explicit coverage … Missing coverage never recreates a program counter inside CTEs. CPU and Metal are backend implementations of the same plan, not different rule languages." Completion criteria 1–7: every grammar maps to explicit programs, facts, and operators; no plan runs through `cohort::Bank`, `State::Util`, or an opaque whole-rule matcher; named facts have one physical owner, complete domain records, and dependency invalidation; finding sets and coverage pass with reviewed differences recorded; workload gates met; SQL and native budgets met; journal complete.

### 3.2 `docs/sql-matcher-batch-implementation.md` (current mechanism description)

- Lanes are `(program, document, preorder node)`. SQL transports demands and completed typed results. The CPU executor groups lanes by operation, advances active lanes, and scatters results back into request order. Structural and pattern kernels do not call `AstMatcher` or install thread-local matcher state (`:3-7`).
- Ownership table (`:16-25`): `producers/prefix.rs` (ordered semantic keys, reusable stages), `matcher/factor.rs` (closed predicate proof, utility definitions, residual construction), `matcher/wave.rs` (interning, numeric lanes, cohorts, shared navigation, bounded caches), `matcher/cohort.rs` and `pattern_batch.rs` (capture frames, utility calls, ordered continuations), `matcher/cohort_host.rs` (typed facts, child demands, parsed contexts), `sql/fragments/batch.rs` (consumer domains, two-axis tiles, allowances, residual execution, publication), `sql/engine/planning.rs` (candidate access paths, demand aggregation, final projection), `tape.rs` (node links, sibling start semantics).
- The wave module has no dependency on SQLite or rule names; an index-provider function supplies kind postings (`:34-35`).
- **Native execution** (`:37-65`): at most **2,048 lanes** per submission; tiles both file and program axes; active masks preserve ordered short-circuit; programs with shared closure or sibling-table fills admit roots in file-clause order, others full width. Results record truth, logical utility work, max utility depth, and incompleteness. Supported exact operations: kinds, regexes, booleans, tree relations, stop predicates, counts without exports, closed recursive utilities. Literal regexes become byte predicates. Navigation forms: neighbour/stop walks advance one cursor per lane; ancestor and sibling end-walks share a demanded suffix graph (each suffix evaluated once); filtered descendant end-walks seek preorder postings inside subtree intervals; repeated unfiltered descendant walks may materialize a zero-cost local predicate over the subtree union after **7 failed probes, ≥16 active requests, and ≥4× remaining traversal versus unique nodes**. Completion intervals distinguish proven negatives from uncovered nodes.
- **Mixed programs and captures** (`:67-93`): negative admission proofs inside mixed alternatives and utility definitions; "A false result proves that native execution stops before reaching an opaque operation. A possible match admits the original program." Conjunction proofs stop at the first opaque term; every disjunction branch needs a proof; partial proofs cannot pass through negation or move across capture or semantic operations. Rejected paths charge proven work; admitted paths keep original charges. The capture-dependent stage handles ordered alternatives, capture frames, utility calls, constraints, counts, and enumeration. Each resident lane owns its context, truth mode, and binding history once. Equivalent pattern plans share results for identical immutable input environments, documents, and nodes. Enumeration carries exclusion sets and rematches from the original input environment. "No semantic callback runs an entire child matcher."
- "This remains a CPU executor. It does not implement a GPU backend. A device backend still needs packed operation tables, device text predicates, capture storage, and queue execution. Semantic child submissions currently use recursive cohort calls on the host" (`:95-97`).
- **Scope, resources, caches** (`:99-119`): each consumer keeps its roots, file context, and allowance; sharing a closed `(program, document, node)` fact does not expand a consumer's domain; program identity includes complete predicate definitions; walk admission mode is part of lane identity. Utility and short-text caches hold ≤32,768 entries each; posting caches ≤131,072 node IDs. All caches belong to immutable document revisions held by one executor. "Incomplete computation never becomes a negative fact." Residual matcher objects stay stable for the whole batch to preserve the native memo's **address-based ownership keys** (`:117-118`).
- **SQL and measurement** (`:121-134`): "a small set of partition plans with typed native batch boundaries"; selective parent relations materialize before child expansion where that avoids repeated qualification; production planning reads SQL kind statistics; "No program counter or matcher state machine is emitted into SQL." Whole-job gates: 100 matching statements, 1.30 CPU, 1.50 edit CPU, 1.5 GiB RSS.
- **Published baseline** (`:136-184`) **M**: 5,725 files, 95.7 MB, 138 policies, 146 AST clauses; 97,798 finding identities and three edits match.

| Measure | Native | SQL with native waves |
| --- | ---: | ---: |
| Whole-job CPU | 113.85 s | 111.72 s |
| Whole-job wall | 66.80 s | 69.67 s |
| Median edit CPU | 202.82 ms | 213.17 ms |
| Peak RSS | 0.79 GiB | 1.10 GiB |

  23 matching SQL statements, 25,216,565 VM instructions; 30,044,209 input lanes in 14,759 submissions; 242,632,839 operation lanes in 5,726,033 kernel dispatches (42.37 per dispatch). Scalar matcher roots 62,364,147→53,109,000 (−14.8%); residual CPU 73.24→64.49 s; wave preparation and execution 2.64→13.10 s; total matching CPU about unchanged (82.24 vs 82.64 s). "The batch execution model is real, but these numbers do not demonstrate a whole-job speedup." The later full conversion (capture cohort): 188.48 SQL vs 114.29 native CPU s, ratio 1.65, instruction ratio 1.295, **rejected**. The zero-cost subtree materialization does not activate on the full workload.

### 3.3 `docs/sql-matcher-batch-optimization.md` (audit at `c17d2e47`)

- At that point each `matcher_i` vtab owned one clause; its xFilter processed the clause's files and buffered outputs. The two UNION ALL statements combined publication but not native matching across rules (`:12-14`).
- Full-workload trace: 23 partitions, 46 matching statements, 73,199,829 native root evaluations, 3,289 fragment filters. 146 clauses = 129 native-only + 14 mixed + 3 SQL-only. Matching 83.56 CPU s, 79.50 inside native; whole job 111.79 s. "Eliminating SQLite overhead alone cannot remove most of that cost" (`:16-24`) **M**.
- Focused experiments (597 catalog fixtures) **M** (`:33-38`):
  - Resetting the file memo per clause versus keeping it across clauses: 1,286.61 → 1,019.49 ms (−20.8%).
  - Six async rules with independent prefixes versus one shared prefix: 156.69 → 29.42 ms (−81.2%).
  - Same, root evaluations: 144,906 → 27,103.
  - Five clauses with redundant SQL kind predicates: 806,803 → 1,529 VM steps, same outputs.

  "Their improvements cannot be added together." B028 fell 362,379→91 VM steps, but CPU only 33.20→32.59 ms: "reducing VM instructions is useful, but it does not automatically remove native work" (`:50-51`).
- Native memo: stable cross-rule identities already exist (keys cover capture inputs, enumeration exclusions, binding continuations, three-valued mode, clause ownership when required). The SQL fragment's per-clause `with_file_memo` destroyed it (`:55-58`). "Sharing by utility name or node ordinal alone is incorrect" (`:63`).
- Common async prefix: `callee-position` and `async-context` shared by 6 rules; 24,151-node equal domains; the shared producer admits 492 nodes; 2,952 residual roots instead of 144,906. The full workload attributes 14.69 s and 13,916,702 root evaluations to these rules, with domains differing in production: "does not establish … an 81.2% whole-workload improvement. Production sharing must compare actual domain identities" (`:73-85`).
- Proposed: SQL starts from call sites joined to enclosing scope facts before resolution. Parenthesized callees need a small structural closure; async context needs the nearest relevant scope including lambda/class boundaries. "A generic ancestor test is insufficient" (`:87-91`).
- F707 is the largest SQL arm (1,707,968 VM). The proposed handler-first relation is unmeasured (`:98-103`). The node vtab already recognizes parent, sibling, ancestor-chain, kind-posting, and subtree-range access paths (`id < root AND last >= root` walks the ancestor chain). "Changing the seed relation and avoiding repeated probes matters more than replacing every `EXISTS` with a join" (`:105-108`). The **candidate table accepts no lookup constraints** and only scans; "The engine needs domain membership or batch intersection" (`:110-113`).
- Statistics: the full trace has an empty statistics map. "Global kind counts alone cannot price a scoped join or estimate ancestor fanout. Statistics must remain SQL-owned. Recounting each batch in Rust repeats the work that planning is supposed to avoid" (`:122-127`).
- Boundary to implement (`:129-161`): a native invocation needs a multi-fragment plan plus a demand relation with explicit rule and request identities. SQL supplies filtered domains, capture inputs, required outputs, and consumer requirements (existence, bounded count). The native batch evaluates compatible producers once, schedules consumers by file, returns typed outcomes, and "retains sequential recursion, ordered decisions, and semantic resolution internally". Conceptual SQL: `eligible AS MATERIALIZED` (call_sites ⋈ scope_facts where is_async) → `requests` (⋈ requested_domains) → `SELECT … FROM match_batch(:residual_plan, (SELECT collect_requests(rule, request, file, node) FROM requests))`. "`requested_domains` needs efficient membership. Materializing every rule/node pair defeats the design." The registry needs canonical definition and domain identities "rather than the current unique `clause:{id}:candidate-scope` bindings".
- Cache lifetimes: first one file revision within a batch; cross-edit reuse needs semantic dependencies (path, module, import/call-graph revision, options). "Resource failures cannot become cached negative answers. Logical unknown remains distinct from false" (`:163-167`).
- Parallelism: four workers already; nested pools oversubscribe and multiply caches; one SQL connection per worker; distribute by measured candidate work. "More threads can reduce wall time. They do not establish a reduction in CPU work" (`:169-173`).
- Statement arithmetic: 23 partitions + one producer statement each = 69 statements including the two publication statements; "The compiler cannot add statements per rule or file" (`:184-185`).

### 3.4 `docs/sql-matcher-fragment-transition.md` (journal; 1,210 lines added in range)

**Decision, 2026-10-25** (`:8-17`) **D**: "The generic SQL state machine is a rejected compiler architecture. SQL must compose typed relations with joins, exclusions, aggregation, and structural closure. Coherent matching operations remain inside batched native matcher fragments. The compiler must not convert ordinary composition into program counters, call stacks, or continuation rows." This supersedes the shared-function scheduler. "The wrong decision introduced shared instructions and return continuations to solve expansion and semantic problems together. The replacement separates those problems. Relational factoring addresses duplication. Matcher fragments own sequential semantics" (`:27-28`).
- Before `049c5873`, eleven grammars were refused: five exceeded compiler operator limits (B615, SIM905, …) and six lacked recursive context contracts. "These were SQL compiler limitations. The native matcher already executes these constructs" (`:21-25`).
- **Fragment contract** (`:41-56`): a fragment keeps its matcher, utility definitions, capture schema, and context requirements. Demand = request identity, document identity, root node, required input captures. Output = completion, explicit outcome, match order, typed exported captures; false, unknown, incomplete, and failed stay distinct. "One filter invocation consumes an operator's whole demand batch … Native code never asks SQL to evaluate a child matcher or utility. File failure discards provisional output for that file." "Fragment boundaries follow semantics, not individual terminal nodes. Keep capture rollback, recursive utility calls, and ordered searches inside a fragment when they share state."
- **Open semantics** (`:58-67`): exact work charges, cache-fill accounting, depth peaks, and clause-wide memo history can constrain fragment boundaries; splitting a region requires a session contract or semantic decision; "The spike must not claim exact accounting across independently executed fragments." Each logical contract needs a pure-SQL binding; "A generic SQL interpreter is not an acceptable implementation."
- First boundary and combined job **M** (`:111-156`): eleven refusals over 597 fixtures, 196,678 roots, 226 matches; each grammar enters its native table once. Combined job: native 288.2 ms vs fragments-through-SQL 287.5 ms CPU; one matching SQL statement; SQL preparation 0.175 ms.
- **Integration 2026-10-25** (`:184-297`): findings form a set and budget exhaustion means `Unknown`; "Exact work charges and witness order remain diagnostic controls, not requirements for relational composition" (`:186-188`) **D**. Demand carries node identities and typed captures, never a tape or serialized matcher state; only an aggregate handle crosses from SQL into a fragment; it builds immutable demand including explicit empty; expired handles fail; handles transfer ownership and reject a second consumption (`:192-195, 278-279`). The statement split follows SQLite compound limits, not file or root counts (`:209`). Compiled cache keyed by query digest, compiler version, and enumeration contract; "No producer sharing is assumed from a common query digest" (`:210-212`). Planning reads kind counts via **`fts5vocab`** over the content-store index in one statement; "Numeric kind tokens overlap across dialects, so these store-wide counts are cost estimates"; used only to choose between equivalent root and descendant anchors; prepared SQL replaced only when the anchor choice changes (`:223-227`). A logical unknown is a completed verdict, not resource exhaustion; the adapter bug that mapped it to incomplete coverage was fixed (`:250-254`).
- Memory lesson **M** (`:259-297`): the first complete workspace run took 113.90 SQL vs 109.74 native CPU s, but peak RSS was **2.95 GB vs 603 MB**. Removing the global `ORDER BY` after UNION ALL did not cut RSS materially. Allocator snapshots found **760,717,400 B of restored navigation data**: "The principal problem is retaining every file's decoded tape and semantic caches together." Fix: target **4 MiB of source per resident partition**, which gives 23 partitions; one atomic budget covers the apply's producer and consumer passes; "The 101st execution is refused before SQL runs"; the producer pass reserves half the budget when a consumer pass follows; request construction bounded to one file inside each batch invocation, with one xFilter still consuming the whole batch.
- Qualified integration result **M** (`:299-339`): native 109.53 vs SQL fragments 111.79 CPU s; wall 107.49 vs 60.27 s; RSS 684 MB vs 1,233 MB; edit 180.10 vs 202.62 ms; 46 matching executions. 2,933 MB → 1,233 MB after resident partitioning. 73,275,953 VM instructions are "diagnostics, not a substitute".
- One migrated recursion case differs only in an attempted-witness metric after exhaustion; "It does not restore discarded-work accounting as a semantic requirement" (`:362-365`) **D**.
- Reusable batch (`:421-511`): the native batch groups ordered root domains by complete equality, dialect, and resource options; processes files individually with one memo across compatible clauses; "Stable compiled matcher objects remain alive throughout that memo's lifetime, because native caches can use object addresses as keys" (`:437`). Shared-prefix work reduces each consumer's remaining allowance. First run: 4,429,551 shared-prefix rows, "Most come from direct kind predicates already guaranteed by candidate admission. This is unnecessary materialization, not useful reuse" (`:454-456`). Candidate membership indexes files and binary-searches sorted root domains. Qualified: 115.63 native vs 110.24 SQL CPU s; 23 executions; 4 prepared instances; RSS 903 vs 1,261 MB **M**. "Batch construction and transport still cost CPU; reducing statements does not eliminate that work" (`:488`).
- Statistics qualification (`:513-534`): search indexing runs on a 30 s maintenance timer after tape publication, so the diagnostic finished before statistics existed. "Merely attaching the store does not exercise data-driven planning." With indexing completed first: nonzero counts for 47 of 49 kinds (689,701 occurrences); the planner moves from one parent-driven arm to four; index preparation 347 ms reported separately **M**.
- Final physical-plan corrections (`:536-547`): EXPLAIN showed SQLite moving F707's parent-level sibling test after child expansion. The compiler now materializes qualifying parents before expanding children when the parent predicate has more than a kind test (34,360→17,983 VM in focus), but adds 161 VM instructions on the full workload; "Its benefit depends on candidate scope and parent fanout" (`:578-580`).
- Final publication qualification **M** (`:555-571`): native 112.56 vs SQL 110.41 CPU s; wall 84.04 vs 129.93 s; RSS 853 vs 985 MB; edit 190.20 vs 211.92 ms; 23 executions; 25,216,565 VM. Native root evaluations 73,199,829→64,235,386. "The end-to-end CPU improvement is modest; the larger structural counter reductions must not be presented as an equivalent speedup."
- **Native operation batching** (`:587-723`): `matcher/wave.rs`, a SQL-independent CPU executor with numeric lanes. "Per-key sharing is legal here because a closed syntactic predicate has no capture or semantic inputs … This is a keyed fact contract, distinct from the earlier domain-complete producer contract" (`:607-609`). An external review found repeated relation walks, a cancellation gap, and missing node validation, all fixed (`:655-661`). Sibling initial positions are normalized by source bytes even at document root (zero-width tokens) (`:671-676`). Final wave qualification: native 113.851 vs SQL 111.716 CPU s; scalar roots −14.8%; residual −8.75 s but wave +10.47 s; "a qualified change in native execution granularity, not a demonstrated whole-job performance win" (`:695-710`) **M**. The checker ratchets scalar native root calls at 53,109,000 and requires more than 4 lanes per dispatch (`:721-723`).
- **Complete native cohort conversion** (`:725-1574`): capture frames, utility calls, and ordered structural operations moved into a native cohort scheduler, with zero scalar matcher calls as the gate. It was **rejected on CPU at every full measurement** **M**:

| Build / change | SQL CPU s | Native CPU s | Ratio |
| --- | ---: | ---: | ---: |
| First complete conversion | 615.603 | — | fails |
| After shared kernels | 270.058 | 111.246 | 2.428 |
| Leading-kind lowering | 208.301 | 107.112 | 1.945 |
| Compact action | 212.614 | 116.546 | 1.824 |
| Compiled class + exclusion runs | 198.093 | 110.819 | 1.788 |
| Compact node | 150.963 | 92.021 | 1.641 |
| Lane metadata | 123.338 | 82.797 | 1.490 |
| Build 36 (phase counters) | 188.482 | 114.289 | 1.649 |
| Build 44 (lookup facts) | 186.965 | 110.961 | 1.685 |
| Build 49 (import-suffix guard, retained) | 188.619 | 119.023 | 1.585 |

  (`:777-779, 937-942, 1038-1041, 1071-1074, 1136-1139, 1221-1222, 1239-1240, 1341-1344, 1483, 1563-1565`). Peak RSS stays about 1.33–1.46 GB. The "lane metadata" ratio of 1.490 was not reproduced: the same unchanged build-32 later measured 1.926 (`:1279`). "These timing differences do not establish a causal speedup or regression on this busy host" (`:1281-1282`).
- Hardware counters (`:1306-1345`): whole-process native 1,238,262,733,524 instructions vs SQL 1,619,961,681,854. At the phase level (build 36), cold SQL 1,569,218,661,305 vs native 1,212,125,529,119 instructions (ratio 1.295) and 595 G vs 363 G cycles. "The cycle gap exceeds the instruction gap. This suggests memory access and execution latency need direct attention" **M**.
- **Rejected experiments (each kept-or-removed)** **D**: blanket root ordering (kernel width 2.37; rejected, `:840-845`); persistent refill (CPU regressed despite fewer dispatches, `:856-858`); conjunction runs (fewer lanes/steps but CPU +13%; "Another direct counterexample to treating fewer dispatches or control steps as a runtime improvement", `:971-979`); member-major admission (rejected; also lacked a resource-order proof, `:1085-1098`); local `Rc` (rejected, `:1107-1110`); fixed-pattern capture facts (saved 11 of 1,387,324 executions, removed, `:1152-1157`); utility frame reuse (no CPU win, removed, `:1180-1194`); list-dispatch split (more instructions, removed, `:1306-1321`); frame arena +3.45% instructions; deferred static-kind rejection +1.53%; single-family dispatch +0.88%; prehashed fingerprint +0.68%; borrowed descriptors +0.15% / +7.29% CPU (`:1425-1433`); explicit-global name guard (+0.63% instructions, removed, `:1517-1522`); cache-bypassing utility shortcut (rejected by review because "cache residency is observable" through cold-false capture replay, `:1508-1515`).
- **Retained**: compact layout (frame 256→160→144→128→112 B); pattern guards; constrained pattern pipeline; scope-interned fingerprints (builds 15,669,077→8,370,880); immutable Python lookup facts in published ScopeTables (native instructions −1.62%, SQL −0.48%); single-pass import chain; import-suffix guard (−0.48% instructions, −1.44% cycles on the full job) (`:1471-1574`).
- Semantic-demand probe (`:1441-1457`): of 2,528,414 resolution demands, only 1,403 resident repeats have agreeing known zero-cost results, so no result cache. Of 1,289,794 repeated scope-reading nodes, 1,288,634 use a different descriptor (`:1466`). Legality constraint: "file tables charge their construction separately to each clause … Any eventual result cache must validate a scope epoch and clause identity" (`:1449-1452`).
- Closing statement (`:1560-1561, 1574`): "structural fused control steps remain about 385.5 million. The data rejects the hypothesis that avoiding these leaf requests alone will close the main performance gap." "The next structural target is the remaining control work, rather than another per-leaf cache or allocation change." The plan (§3.1) follows (`:1576-1583`).

### 3.5 SSC lessons (stated)

1. A generic SQL state machine / continuation rows is a rejected architecture (`transition:10-13`).
2. Relational factoring addresses duplication; native fragments own sequential semantics (`:27-28`).
3. Fragment boundaries follow semantics, not terminal nodes (`:54-55`).
4. Logical unknown is a completed verdict, distinct from resource exhaustion (`:250-252`; plan `:68`).
5. Peak memory came from retaining all files' decoded tapes and semantic caches; bound resident partitions by source bytes (`:289-291`).
6. Reducing SQL statements or VM instructions does not remove native work (`optimization:50-51`; `transition:488, 571`).
7. Fewer dispatches or control steps is not a runtime improvement (`transition:975`).
8. Shared materialization of predicates already implied by admission is waste (`:454-456`).
9. Attached statistics must be populated before measurement; the maintenance timer hid them (`:523-525`).
10. Store-wide fts5vocab counts are estimates, not cardinalities (`:225, 533`).
11. Materializing a selective parent before child expansion helps only where fanout repeats (`:578-580`).
12. Cache residency can be observable through replay semantics; shortcuts that change admission or eviction change results (`:1508-1515`).
13. Busy-host timing is unreliable; use paired runs, adjacent unchanged controls, and hardware counters (`:1176, 1281, 1307`).
14. The cycle gap exceeding the instruction gap points to memory latency (`:1323`).
15. Measure duplicate demands before adding caches; repeated requests ≠ reusable results (`:1438-1439`).
16. Leaf-transport removal does not remove the composition that generates requests (`plan:13-15`).
17. Finding-set freedom does not license unordered unions where a later condition reads an earlier alternative's capture (`plan:61-64`).
18. `MATERIALIZED` does not prove one execution; check xFilter counts (`plan:214-215`).
19. Catalog-fixture ratios do not extrapolate to full workloads with different domains (`optimization:82-85`).

---

## Synthesis (restating only what the sources establish)

### S.1 Points of agreement across all three streams

- **Exact-scope sharing only.** Sharing or fusion requires identical candidate scope (and definition, version, parameters, columns, configuration). Unions, supersets, overlaps, or glob matches are never substitutes (DLRE `recovery-plan:55-57,129`; N1562 `design:169-175,662-664`; SSC `plan:258-261`, `implementation:101-104`).
- **No generic interpreter in SQL.** No program counters, continuation rows, or CTE continuation machines (DLRE `recovery-plan:51,153`; N1562 `design:398-400,671-674`; SSC `transition:10-13`, `plan:121-122,376`).
- **Whole-demand batch vtab contract.** One xFilter consumes the full typed demand batch (via `sqlite3_vtab_in` or an equivalent frame); xNext only emits completed rows; no scalar callbacks per node (DLRE `recovery-plan:50-53`, `source-confirmation:19-21`; N1562 `design:379` and code `:1-5, 2290-2316`; SSC `transition:49-51`, `plan:209-211`).
- **Every native relation keeps a SQL/ordinary-table binding** as a contract test, but that binding is not a procedural matcher (DLRE `recovery-plan:52`; N1562 `design:527-537`; SSC `plan:375-377`).
- **SQL-owned statistics, read once at plan time; no runtime recounting** (DLRE `spike-results:89`, journal 3708-3712; N1562 `design:609-627`; SSC `plan:265-269`, `optimization:126-127`).
- **Empty demand means no work; complete empty results are successful completions; incomplete never becomes negative** (DLRE `recovery-plan:56`, `lifecycle:319`; N1562 `design:124-125,646-647`; SSC `plan:66-73,211`, `implementation:111`).
- **No rule-ID branches, no runtime algorithm flags, one production path** (DLRE journal 5015, `lifecycle:19-24`; SSC `plan:167,373`).
- **Acceptance is CPU (user+sys), wall, RSS, and parity; statement and VM counts explain** (DLRE journal 3697-3699; SSC `plan:340`; N1562 `design:755-756`). SSC still keeps a hard 100-statement ceiling (see S.2).
- **Native batching is not automatically a CPU win.** DLRE S3: native child 105.781 vs SQL 106.832 vs scalar 100.773 ms. SSC: native waves gave about 0% whole-job change; the cohort conversion is 1.49–2.43× slower than native.

### S.2 Contradictions and tensions between sources

1. **Statement-count gate.** DLRE explicitly supersedes fixed-100 acceptance; statement counts are diagnostic, and 296 submissions are recorded as "neither removed nor reported as passing" (journal 3697-3699, 4806-4808). SSC keeps "the hard budget of 100 rule-execution SQL statements" as a compile-time rejection and full-job ceiling (`plan:281-297,332`). N1562 records counts, sets expected ranges, and fails on "a large increase" (`design:305-312`).
2. **Logical work accounting.** N1562 requires an exact `LogicalEffectLedger` that preserves memo replay charges, fill ownership, and relative peak depth across physical sharing, with parity tests at the exact limit boundary (`design:448-488,724-735`). SSC states that "utility depth peaks, and memo replay charges do not define finding identity" and that "Exact work charges and witness order remain diagnostic controls" (`plan:46`; `transition:186-188,365`). DLRE keeps per-consumer allowances unchanged, never multiplies them, and allows a conservative min-allowance batch cap (`lifecycle:345-363`). DLRE says nothing on replay-charge parity: not found.
3. **Ordered semantics.** N1562 routes order-dependent behaviour (effectful `any`, dependent `all`, first-witness, `each`, recursive stops) to a native `ordered_region` and fails closed otherwise (`design:418-436,536`). SSC lowers explicit priorities to exclusion relations and dependent captures to typed equality joins (`plan:47,243,254`), yet warns that finding-set freedom does not license unordered unions where a later condition reads an earlier capture (`:61-64`). The two agree that order is not accidental SQL order; they disagree on whether ordered regions stay native (N1562) or become algebra (SSC target).
4. **Statement size limits.** DLRE: authored 16 KiB, planner 1 MiB (`lifecycle:231,237`; `sql-execution-owner.md:55`), plus the 65,535-reference SQLite limit hit by full inlining (`owner:56`). N1562: "The 8 MiB statement limit remains a safety check" (`design:22`). SSC: splits follow SQLite compound limits (`transition:209`). The branches use different limits.
5. **Ordering contract of vtab outputs.** N1562 code declares guaranteed orderings and consumes ORDER BY (`sql_backend.rs:2466-2471,2775-2785`); its doc says vtabs "declare only ordering that their cursor guarantees" (`design:625`). DLRE's source-confirmation "claims no ordering" (`source-confirmation:20`), and DLRE removed a required `ORDER BY qid,fid,pre` transport (400 sorts) in favour of unordered accumulators (journal 4497-4501). Not a direct contradiction, but the two branches chose differently.
6. **Single-consumer streaming versus dynamic probes.** SSC: "Single-consumer local intermediates stream through typed buffers or inline SQL" (`plan:275`). DLRE found that one syntactic reader can probe 16× more and rejected the general single-reader inliner (journal 4048-4051), and found that fusion into a materialized CTE dropped an index needed by a repeated reader (journal 4409-4413). DLRE's evidence qualifies SSC's rule: "single consumer" must mean a single dynamic read, not a single syntactic reference.
7. **Materialization threshold.** DLRE: "A measured second consumer justifies materialization" (`recovery-plan:58`). N1562: "considers it only for at least two consumers. Statistics decide" (`design:580-581`). SSC: reused or recursive relations get one narrow materialization (`plan:276`). These are consistent in direction; only DLRE requires the second consumer to be measured.
8. **Scope lookup inside the vtab cursor.** DLRE SC1: scope lookup "must not scan the complete frame behind an interface marked indexed" (`lifecycle:183-185`). N1562's cursor pushes `producer_id`/`file_index`/`node` constraints but implements them as filters over a linear scan of every row in the batch (`sql_backend.rs:2828-2856`). Its estimate divides a seeded constant by 4^(pushed−1) (`:2786-2793`); DLRE WP3 asks to replace fixed estimates with estimates for the actual constraint shape (`recovery-plan:146`), and N1562's own design forbids "selectivity guesses in Rust constants" (`design:611-612`).
9. **N1562 doc versus code.** The doc surveys a single 35-column `rerail_matches` plan (`design:34`); the code at HEAD registers that relation only under `#[cfg(test)]` and production uses ten typed relations (`sql_backend.rs:2516-2662`).
10. **Memo identity by address.** N1562 rejects pointer-address memo keys (`design:53-59,666-669`). SSC's implementation deliberately keeps stable matcher objects alive "because native caches can use object addresses as keys" (`implementation:117-118`; `transition:437`). SSC accepts the address-keyed native memo as a current fact; N1562 targets its removal.
11. **Navigation producer shape.** N1562's navigation producer emits every parent→child edge of a whole file (anchors must be `[0]`) (`sql_backend.rs:430-448,1576-1620`). SSC's plan: "The planner does not build an ancestor-pair table or decode every parent to answer a local relation" (`plan:273`). DLRE uses a persistent `(fid,parent,field,pre)` index (`owner:16`). Whole-file edge emission is in tension with the other two.
12. **Native/GPU readiness.** N1562 code already has a Metal backend dispatching one command buffer per partition for packed programs (`sql_backend.rs:3203-3230`). SSC says moving the frame stack to Metal is out of scope and its executor "remains a CPU executor" (`plan:127`; `implementation:95-97`). DLRE keeps the typed contract "for future GPU" (`spike-results:144`). These describe different operator subsets, not a direct contradiction.
13. **Partition / packet sizing.** DLRE: 32-file publication transactions and 32-file ingestion requests (`spike-results:173`; journal 4566). SSC: 4 MiB source per resident partition giving 23 partitions (`transition:291,318`); 2,048-lane native submissions (`implementation:39`). N1562: binary-split partitions by `packed_workspace_fits` (`sql_backend.rs:345-362`). The units differ.
14. **DLRE S3 request count.** Journal 4957 says 40,000 requests / 80,000 outputs; `spike-results:130` and journal 4993 say 80,000 / 80,000.

### S.3 What the sources establish about shared plan records

- Only SSC defines `RelationSpec`, `Stage`, `ProgramGroup`, `Plan` (`plan:98-117`); field lists are reproduced verbatim in §3.1. N1562's analogues are `ScopeRecipe`, `ScopeInstance`, `ClauseLogicalPlan`, `ProducerId`, `UtilityDefId`/`ComponentId`/`DemandKey`, `PlanShapeId`, `EffectSummary`, the `RelationBroker` trait, and the SQL AST node set (§2.1). DLRE's analogues are `FamilyProgram`, `ConsumerState`, `DemandBatch`, `FamilyContext`, `ExecutionBinding`, `CutHandle` (`lifecycle:119-134`). No source maps these sets onto each other: not found.

### S.4 Measured facts most relevant to the final shape (cross-stream)

- DLRE: SQL analysis 679 s, materialization 456+620 s, and indexes 342 s dominate traced deferred CPU; 792 suspended consumers; retained schema count scales analysis CPU about 6× from 128 to 1,024 tables (§1.2, §1.9). The current all-60 is 2,540.634 s, the first-50 672.782 s, the target 125.804 s.
- DLRE: per-file to 32-file publication transactions gives 3,459→530 ms (§1.8). Finite-kind folding −15% ingestion CPU; indexed parent membership −27%/−12%; regex file scoping 2.968→0.033 s component (§1.15-1.16).
- SSC: the native/SQL fragment boundary runs at about native CPU (0.95–0.98 ratio) with 23 statements. Full native cohort conversion of capture semantics is 1.49–2.43× native CPU, with about 385.5 M control steps remaining (§3.4).
- SSC: memory was dominated by retaining all files' decoded tapes and semantic caches; bounded resident partitions brought 2.95 GB → 1.23 GB (§3.4).
- N1562: no measurements in the design doc; the code shows a once-per-batch lazy materialization, a mandatory `batch=` constraint, and `producer_id IN` via `sqlite3_vtab_in` (§2.2).

### S.5 Not found in these sources

- An end-to-end measurement of N1562's design (it is proposed only).
- Any DLRE measurement of SC1–SC4 in production (the plan describes completed experiments only; the HEAD commit adds tests "require schema reuse, scope isolation and bounded completed rows", which I did not read because it is not a doc).
- An SSC measurement of the structural plan's Phase 1/2 targets (proposed thresholds only, `plan:329`).
- Any reconciliation document mapping DLRE family contexts, N1562 scope groups, and SSC `Stage`/`ProgramGroup` onto one plan.
