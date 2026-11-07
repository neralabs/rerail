# Lessons ledger: PR 2539 and ast-index-spike vs the current Rerail branches

Compiled 2026-10-25, read-only.

**Abbreviations**
- Prior work:
  - **PR** = nera-data-layer PR 2539, tip `281f500d7c` (2026-06-27). Docs are under `docs/design/`.
  - **AIS** = ast-index-spike, `main` plus `origin/jeremybarnes/seadragon`. **TAIL** = seadragon's `TAIL_LATENCY_WORKPLAN.md`.
- Current branches:
  - **DLRE** = `rerail/Data-layer-rule-expressivity`, 2026-10-19 → 10-24. **J** = its `docs/data-layer-performance-journal.md`; **WP** = its `docs/sql-performance-recovery-workplan.md`.
  - **SSC** = `rerail/…Moti-Velz-sql-set-composition`, 10-16 → 10-24.
  - **N1562** = `rerail/ner-1562-scale-test-the-re-railer`, 10-16 → 10-25.
  - **Tempo** = `nera-landing-2/…/tempo/docs`. "-sql" means the 09-24 → 10-16 docs; "-metal" means the 09-30 → 10-25 docs.
  - **Main** = `rerail/Pulling-it-all-together`.

**Status words**
- **ADOPTED**: the branch applies the lesson.
- **RE-LEARNED**: the branch rediscovered it. The entry says where, when, and how long the detour took.
- **VIOLATED**: the current design contradicts the lesson.
- **N/R**: not yet relevant.

**Dates.** All dates are 2026. DLRE commit dates often trail the journal's own section dates, because the 10-21 and 10-22 entries were first committed in `5b09437c` on 10-23. Section dates are used below.

---

## Part A: runtime comparison

### Corpora

| | PR 2539 | DLRE |
|---|---|---|
| Repository revision | nera-data-layer at `281f500d7c` | data-layer main at `2b1504ad` |
| `.py` files | 5,078 | 9,010 |
| `.py` bytes | 83.5 MB | 137.9 MB |
| Files and bytes run | — | 9,021 files including 11 Rust grammar files; 137,924,324 B |

- The repository grew 1.77x by files and 1.65x by bytes between June and October.
- The "projected 5,000-file" numbers in PR 2539 cover the whole repository as it was then.

### Best like-for-like numbers

| Run | Files / MB | Rules | Cold time | ms per file | ms per file-rule | Source |
|---|---|---|---|---|---|---|
| PR, 500-file sample, final | 500 / 9.9 MB (506 plan files) | 101 linters | **17.99 s wall**: plan 0.46, refresh 13.67, report 3.73. 3-run median, 8 workers | 36.0 | **0.36** wall | `DECLARATIVE_LINTER_FACT_ROW_WIDTH_PLAN.md` Step 6, `281f500d7c`, 06-27 |
| PR, full repository | **5,073 / 82.6 MB** | all declarative | **70.6 s wall** (was 96.6 s), 8 workers | 13.9 | about 0.14 if about 100 linters | commit message `cfed5a4ab6`, 06-20 |
| PR, 200-file cold | 200 | all registered | refresh 8.7 s plus SQL 47.8 s on 1 worker; SQL 36.6 s on 8 workers | about 225–280 | about 2.5 | `DECLARATIVE_LINTER_200_FILE_PERFORMANCE_PLAN.md`, `3cb58f01e5`, 06-25 |
| DLRE, first 50, before the SQL change | 9,021 / 137.9 MB | 50 | **125.8 s CPU**, 4 workers | 13.9 | **0.28** CPU | J "Aggregate optimization checkpoint", 10-23 |
| DLRE, first 50, 10-21 build | 9,021 | 50 | 212.6 s CPU, **233.7 s wall** | 25.9 wall | **0.52** wall | J "Full repository verification", `3285b48f`, 10-21 |
| DLRE, first 50, SQL | 9,021 | 50 | **2,507.5 s CPU** (41.8 min), wall not trustworthy | 278 | **5.56** CPU | J "Aggregate optimization checkpoint"; WP |
| DLRE, all 60, before the SQL change | 9,021 | 60 | 709.2 s CPU; failed with 23 failures | 78.6 | 1.31 | same |
| DLRE, all 60, SQL | 9,021 | 60 | 7,211.7 s CPU; OS `real` 3,516 s (controller recorded 15,877 s); failed with 34 failures | 799 CPU / 390 wall | 13.3 CPU / 6.5 wall | same |
| DLRE, small control | 12 files | 10 rules (batch six) | Rerail 37.2 s vs the upstream Python linters 4.32 s (8.6x) | — | — | J "CPU and memory attribution", 10-21 |

The full-repository PR run does not record whether linter SQL is included. Its total is mostly refresh: a sibling run in `28219e7d99` spent 96.5 s of 97.9 s in refresh.

### Verdict

**The claim holds in substance. "Seconds" is true only for the 500-file sample.**

PR 2539 did run on its whole repository: 5,073 files in 70.6 s wall, cold, with 8 workers, on 06-20. That run predates the physicalization work that later took the 500-file sample from 47 s to 18 s.

Scaling 70.6 s by bytes to the 138 MB corpus gives about 2 minutes for about 100 linters, or less after the later fixes. On the same corpus size, DLRE's SQL build takes 41.8 CPU-minutes for 50 rules and about 1 hour of wall time for 60.

| Comparison, per file-rule | Ratio |
|---|---|
| DLRE SQL (5.56 ms CPU) vs PR (0.36 ms wall) | about 15x |
| DLRE SQL wall (6.5 ms) vs PR wall | about 18x |
| DLRE SQL vs PR's full-repository run (about 0.14 ms) | about 40x |

**DLRE before the SQL change was on par with PR 2539, not behind it.**
- 0.28 ms CPU per file-rule (1.5x PR on the 10-21 wall-time build).
- 13.9 ms per file, the same as PR's full-repository 13.9 ms per file.

**The gap is a self-inflicted regression of about 20x.** It came from moving candidate (anchor) selection into a SQLite FTS5 positional index with generated per-batch SQL:
- File-ingestion CPU went from 73 s to 2,456 s.
- The anchor-selection counter went from 8.4 s to 2,248 s.
- Selection cost per indexed node grew from 50 to 115 µs as the corpus filled.

**Caveats**
1. **CPU vs wall.** DLRE reports process CPU with 4 workers. PR reports wall time with 8 workers. PR's CPU could be up to several times its wall time during the parallel extraction phase, which narrows the 15x gap to somewhere between about 3x and 15x.
2. **The rule sets are not the same.**
   - PR's 101 linters are YAML ports of PR-era checks (104 declared).
   - DLRE's 60 rules port a subset of main's 142 checks; 99 check names are shared, and all of those scripts have changed.
   - DLRE includes `serverplan-state-tripwires`. That one rule takes 12.5 s cold CPU on 7 files and dominates the batch-six cost.
3. **The corpora differ.** They are 1.65x apart in bytes. PR's 500-file sample is a deterministic stratified sample, not random.
4. **Build and run setup differ.** DLRE uses a reference build: opt-level 1, a profiling feature compiled in, a system-malloc shim, and a heavily loaded shared host. PR runs are local runs with no machine recorded.
5. **Both 60-rule DLRE references fail.** WP says they do not support a clean ratio. The 10-21 all-60 pass at 1,114.6 s was contaminated by profiling.
6. **Remeasurement gap.** No DLRE full-repository run exists after 10-23; all 10-23 and 10-24 work was measured on a 2-observation guard. No PR full-repository run exists after the 06-26 and 06-27 row-width work.
7. **DLRE does not compare against PR 2539.** Its `docs/data-layer-rule-port.md` "Historical SQL costs" (`905212f6`, 10-19) cites only PR's early 200-file plan (SQL 36–48 s), not the final 18 s / 500-file result or the 70.6 s full-repository result. It concludes "historical SQL times … cannot establish a current Rerail bottleneck."

---

## Part B: the ledger

| # | Lesson | Source (doc §, commit, date) | Evidence | Status in current branches |
|---|---|---|---|---|
| 1 | Storing one row per AST node is fatal; keep a compact tape as the source of truth. | AIS `results/storage_report.md`; `DESIGN.md` S089 / S021; README "Why the obvious design fails". `820dd9d` 07-01, `abdaa4d` 07-27 | Row-per-node is 120 B/node, about 14.5x source, to carry about 6 B of information. The tape is 6 B/node; the logical encoding is 8.66 B/node. | **ADOPTED for storage everywhere** (Main, DLRE, SSC, N1562, Tempo-sql). **RE-LEARNED at runtime**, as per-node or per-tuple intermediates: <br>• **DLRE** retained decoded-property tables keyed `(fid, pre)`, filled by all-node demand (J "Value postings…", 10-21: positive control 1.87 → 2.50 s, data statements 72 → 550). These are still in production, so partly **VIOLATED**. <br>• **SSC**: frame B-trees were 48% of the profile (`ecb0cdf2`, 10-16); 43.8M frame rows (`42ab496a`, 10-21). <br>• **N1562** walked 3.49B rows of `rerail_nodes` (`8fec85c0`, 10-16). <br>• **Tempo-sql**: 15.7–85M result rows took 56% of wall time (`1a9f39565`, 10-14). <br>• **Tempo-metal**: one 24-byte task per tuple needed 1.99 GB (09-30 → 10-16, about 16 days). |
| 2 | Anchor on the rarest constrained kind, chosen from kind frequencies, not on the leaf and not with a fitted ratio. | AIS `DESIGN.md` S050 (`3eb9875`, 07-04); SQL_ONLY Step 4c; TAIL S290 (`5698ed6`, 07-27) | Leaf anchor 112 s with tail timeouts; rarest anchor 33 s, worst query 1.2 s. | **DLRE: ADOPTED, then corrected.** Estimates counted kinds instead of selected postings (J "SQLite positional kind anchors – Rejected trials", 10-21). <br>**SSC: RE-LEARNED.** A fixed 1,048,576-row estimate caused 1.18M advances for 1,875 rows (`5efcb830`, 10-21). Rare anchors arrived only in `85b78bbf` on 10-24, test-only, about 5–8 days. <br>**Tempo-sql: ADOPTED** (09-24) but weak: 5,663 of 5,725 tapes were still decoded. <br>**Main**: presence bitsets only, no frequencies. |
| 3 | A sound filter is not a useful filter: emit a probe or prefilter only when measured selectivity says it rejects most rows. | AIS `DESIGN.md` S164, `9a52c7c` 07-31 | Ungated, the tail went 47.6 → 71.1 ms. README R14b still calls the gating corpus-dependent. | **DLRE: RE-LEARNED at least 4 times between 10-20 and 10-23.** <br>• Shared scope lost its path gates: 3.86 → 5.69 s. <br>• `direct_events` went 0.77M → 16.3M VM steps (10-22). <br>• Unrelated files raised probe ticks 1 → 361 and property joins 6 → 806 (10-23, `5b09437c`). <br>**SSC: RE-LEARNED.** `RERAIL_SQL_FIRST_PROBE` won 213x on a synthetic case and regressed the real corpus (`5e70ced3`, 10-20); removed the same day. |
| 4 | Planner statistics for recursive or materialized CTEs are untrustworthy (Postgres estimates rows=1), and statistics fixes fall short. | AIS `DESIGN.md` S069 / S153 / S156; SQL_ONLY Step 3d / 3f. `a6622be` 07-30, `9a52c7c` 07-31 | `recursive_worktable_factor` recovers 2 of 3 queries; wrong ROWS hints make 13 queries 2.5–80x slower. | **DLRE: partly RE-LEARNED** on 10-20 (`reachability-spikes.md`: "Full ANALYZE did not repair the plan", "Treat statistics as part of the physical plan"). <br>**SSC: RE-LEARNED** as fixed `xBestIndex` estimates; fixed with forced CROSS JOIN (`235dde6a` 10-20, `14c0d580` 10-21). <br>**N1562**: recursive CTEs for utility SCCs, no statistics discussion (latent risk). |
| 5 | Giving the planner real statistics through temp tables plus ANALYZE loses badly to a fused plan. | AIS `DESIGN.md` S155 "What does not work, with numbers"; README "Rejected". `9a52c7c` 07-31 | **57–115x slower** (0.02 → 1.15 s; 0.24 → 27.8 s); 27 statements instead of 1. | **DLRE: VIOLATED.** <br>• The architecture is CTAS temp stages plus indexes plus ANALYZE: 24 relations, 22 indexes and 14 ANALYZE per state-flow execution (`sql-execution-opportunities.md`). <br>• Full statistics made SQL slower, 7.22 → 8.01 s (10-20). <br>• WP says "Do not remove ANALYZE merely to improve the execution count." <br>**SSC: RE-LEARNED twice** (`ca06829d` 10-21; `4dcd60b2` 10-24: "forced-join, and ANALYZE variants all lose"). It had 9,904 temp setup statements per job before 10-21. |
| 6 | `AS MATERIALIZED` and naming CTEs do not bound evaluation or preparation; only physical cuts or a restructured plan do. | AIS `DESIGN.md` S037 (`fd1aaf1`, 07-02: 471 s materialized vs 15.9 s pipelined); S052 / S069. Not stated in these words. | 30x from S037. | **DLRE: RE-LEARNED 4 times.** <br>• 178 table-reference-limit errors (10-21). <br>• "Naming alone does not bound SQLite preparation work": 162.8 vs 10.9 vs 5.35 ms (10-22). <br>• 1,101 inline materialized CTEs with a 68.5 MiB peak (10-23). <br>• Family packing hit a 53 MB heap (10-24). <br>**SSC: RE-LEARNED 4 or more times**, 10-21 → 10-24 (`9207ff45`: 12.26M advances; `541df6f9`: "executed its atomic prefix twice, despite AS MATERIALIZED"; the 65,535-reference limit). <br>**N1562: RE-LEARNED** as text expansion: one clause generated 164 MB of SQL (`c5876491`, 10-24). |
| 7 | On Postgres, forbid nested-loop and merge joins unconditionally, because every join is between rows=1 CTEs. | AIS `DESIGN.md` S069, S175, S269. `a6622be` 07-30, `54843f2` 08-01, `1a5b109` 08-06 | 37 queries faster with them off, none faster with them on; html query 120 s → about 1 s. | **N/R** in every branch (SQLite only). The SQLite analog is ADOPTED in SSC and DLRE: forced CROSS JOIN fences, plus SSC's emitted-plan audit test. |
| 8 | Sidecar tables blow the bytes-per-node budget; delete them first so the portable path is the only path and gets fixed. | AIS SQL_ONLY §2.6 / Step 4a; README "Rejected". `021f138` / `abdaa4d` / `ecc702b`, 07-27 | Sidecars were 211 (SQLite) and 456 (MariaDB) B/node against a budget of about 5. Guard test caps them at 0.25 rows/node. | **Main / SSC: ADOPTED.** <br>**DLRE: partly RE-LEARNED, partly VIOLATED.** The postings sidecar cost +25–31% and was rejected (`sql-preparation-spikes.md`, 10-22). The span, child-field and structural-property tables remain; there is no bytes-per-node budget or guard. <br>**N1562: RE-LEARNED.** Each relation copied the 35-column row (`bc7840d4`, 10-24). <br>**Tempo-sql: RE-LEARNED in 2 hours** (Schema 11 → 12, 09-27). |
| 9 | Batch sharing across a union scope loses (scope inflation); share only per group, and only exact closures with 2 or more consumers. | AIS README "Batching and sharing"; `DESIGN.md` S149 (`a50ec62`, 07-31); S176 (`56c785a`, 08-01); TAIL S287 / S306 | Union 0.86x with 7.27x scope inflation; per-group 1.45x. `shared_once` is worth 1%. | **DLRE: RE-LEARNED 3 times.** <br>• 10-20: one broad scope 5.69 s vs three scopes 3.86 s. <br>• 10-22: retained shared sets +15.6%. <br>• 10-24: family packing cut submissions 7,733 → 4,109 but CPU went 1,679 → 1,855 ms. <br>**SSC: RE-LEARNED.** <br>• 10-21: +2–18.5% CPU; a heterogeneous batch took 935 vs 118 ms. <br>• 10-24: adopted seadragon S287 / S306 by name. <br>**N1562: VIOLATED.** One statement for the whole workspace; per-clause host loops prohibited. <br>**Tempo-metal: VIOLATED** (335 programs × 2,079 files in one evaluation; still over the memory cap). |
| 10 | Measure the code that is actually running: confirm a path is live before trusting or rejecting its number. | AIS SQL_ONLY §2.2 (default-off `--text-anchor` flag: 328 of 331 timeouts, `021f138` 07-27); `DESIGN_HISTORY` "two consecutive measurements of a feature that was not running" (S149 / S176, `56c785a` 08-01). PR handover 06-16 "GOTCHA": a stale fact store shows 0, then 9, then 16 findings (`ca2777e58d`, 06-17). PR harness scope bug: 27,365 vs 1,381 violations (`da037d275c`, 06-26). | See source column. | **RE-LEARNED in every branch.** <br>**DLRE:** <br>• Default-off experiment flags; "one permanent execution path" rule (`7da151b8`, 10-21). <br>• The profiling feature added about 316 GB of rescans and invalidated the 1,114.6 s reference (10-21). <br>• The profiler import silently dropped the statement gate (`00e04cd4`, 10-23). <br>• All timings still use opt-level-1 profiling builds. <br>**SSC:** <br>• The 17.189 s run measured "native composition behind a SQLite demand boundary" (10-20). <br>• An overwritten artifact gave a wrong size target (10-24). <br>**N1562:** the 17.189 s, −98.8% result measured the deleted path (`a4339124` → `525a0124`, 10-18). The history file **remains uncorrected**. <br>**Tempo-sql:** Entry 44 timings "measure ast-index-spike, not Tempo" (09-27). |
| 11 | Generic entity-attribute-value (EAV) facts, and one table per YAML relation, are both too wide: physicalize per relation family, children included, and stop writing the generic entity. | PR `LINTER_SKINNY_FACT_SCHEMA_REDESIGN.md` "Problem" (`df5cdfc81e`, 06-19); `DECLARATIVE_LINTER_FACT_ROW_WIDTH_PLAN.md` Step 5 / 6 (`d670bc58c8`, `8632e9a965`, 06-26) | 1.65 GB store with zero findings. 500-file wall time 47.1 → 39.5 → 19.5 s. Parent-only physicalization rejected. | **DLRE: cited 10-19, then RE-LEARNED.** Queue held 577,199 JSON rows, about 1.26 GB, including 103.7 MB of field-name strings (`a92c126a`, 10-21); fixed with typed column batches on 10-22. About 3 days. <br>**N1562: VIOLATED.** Generic `fact(kind, key, value)` plus a 35-column union; narrowing started 10-24. <br>**SSC:** partial. |
| 12 | Intern strings as atoms with 32-bit keys, and fix duplicate slot references before changing atom storage. | PR `LINTER_SQL_FACT_STORAGE_DESIGN.md` "ID Model" (`e76569ce7f`, 06-08); row-width Step 6 (`3f263865bc`, 06-26) | `fact_text_value` has 77k atoms and 3.4 MB; `idx_lint_fact_slot_value` is 8 MB. | **Main / N1562 / Tempo: ADOPTED.** <br>**DLRE: RE-LEARNED.** Repeated path strings were 54% (61.5 MB) of closure keys (10-20); 103 MB of field names (10-21) led to shared typed schemas (10-22). <br>**SSC: ADOPTED**, with `node.atom`, from PR 2539's `fact_store.py`. |
| 13 | Extract demand-driven: only the slots that selected rules need, and join only the demanded columns. | PR skinny-schema "Demand Model" (`df5cdfc81e`, 06-19); `cfed5a4ab6` 06-20; row-width Step 6 context slice (`5125122a2a`, 06-27) | Full-repository cold run 96.6 → 70.6 s. Annotation rows 44,275 → 8,043. Full-catalog projection regressed report time to 10.1 s; demanded joins gave 4.9 s. | **DLRE: ADOPTED as the stated "transferable lesson"** (`data-layer-rule-port.md`, 10-19), then **RE-LEARNED**: "Automatic demand removes only six of 1,993 logical columns" (`f52817fc`, 10-20). <br>**SSC: ADOPTED**, but where to restrict was re-learned ("restriction belongs before demand insertion", 10-21). <br>**N1562: ADOPTED** (`xFilter` demand); re-learned for utilities on 10-24. |
| 14 | Each file owns its rows, so an edit deletes and rewrites only that file's rows; whole-graph artifacts must not force whole-repository rebuilds. | PR `LINTER_EXPRESSION_FACT_QUERY_DESIGN.md` "Dependency and Invalidation Model" (`7c4469493d`, 06-10); `LINTER_MEMOIZATION_AND_DEPENDENCY_PLAN.md` (`ab2e3f714c`, 06-04) | Design rule only; nothing measured. | **Main / Tempo-sql: ADOPTED.** <br>**DLRE: ADOPTED for storage, but edits still pay for global evidence reconstruction.** One-file edit costs 121.6 s CPU on all 60 SQL, vs 62.6 s before. <br>**SSC / N1562:** unqualified or unmeasured. |
| 15 | Partial physicalization or sharing is slower than none: commit to a whole family or path, or don't. | PR row-width Step 5 (parent-only `python_call` was slower); AIS S149 (shared flat 0.23x vs unshared 1.35x); TAIL S287 ("partial sharing without closure caused a 50-fold regression", `de6d285`, 08-09) | See source column. | **DLRE: RE-LEARNED.** Decoded properties (10-21); copied column waves and retained sets (10-22). Decoded-property tables remain, so partly **VIOLATED**. <br>**SSC: RE-LEARNED.** Admitting 55 of 141 clauses was slower than none (`507b5ebb`, 10-21), leading to "no per-clause fallback" (`605ed842`, 10-22). |
| 16 | The cost is repeated per-linter scans over the same broad relations, but fusing everything into one statement does not remove repeated work; fix plans and candidate sets first. | PR 200-file plan "Bucket C" (`3cb58f01e5`, 06-25: SQL 47.8 s vs refresh 8.7 s); AIS `LINTER_CASE_STUDY.md` §2 (`55d9157`, 08-01) | AIS: PR's 28.2 s became 3.7 s by fixing plans; removing rescans was worth about 1%. | **DLRE: RE-LEARNED.** <br>• 14 rebuilds of the nodes and ownership relations (10-20). <br>• One statement had 710 CTEs (10-21). <br>• One definition repeated across 61 and 90 statements (10-23). <br>• WP (`00e04cd4`, 10-23): "One large statement that repeats the same expensive work does not satisfy the CPU requirement." <br>**SSC: RE-LEARNED.** 24k–34k statements, then packing to 22–24 executions with 2.78B VM (10-24). <br>**N1562: VIOLATED.** One statement for the whole catalog; now blocked by the 8 MiB SQL limit, with 7 of 12 policies failing to compile (`fed9342c`, 10-24). |
| 17 | No Python callbacks or UDFs in the SQL phase. A per-row host callback was nearly all of SQL time. | PR handover 06-16 "The Goal" / "CRITICAL" (`ca2777e58d`, 06-17), with `f80c0709e6` evidence; AIS SQL_ONLY C1, README R2 / R6 (`89ff3da` 07-18); TAIL S295 | Python REGEXP took about 72 ms for nine regexes vs about 1 ms for the scan. Rust regex 24 ns vs Python 332 ns. | **Main / Tempo: ADOPTED.** <br>**DLRE: ADOPTED for Python, partial VIOLATION with native UDFs and virtual tables.** <br>• `nera_child` per missing key; 2.86M BLOB reads (10-21). <br>• Dynamic regex per term cost 14.3 s (10-22). <br>**N1562: RE-LEARNED.** Restored the callback engine and measured 2.70B callbacks (`b88a7dde`, 10-16); deleted in `f492b954` on 10-18. About 2–3 days, after Landing had learned it at `697bd2bd7`. <br>**SSC: VIOLATED then re-adopted.** Frame-action callbacks inside a recursive CTE (`5f1960f2`, 10-21); rejected 10-22. |
| 18 | Don't replace brittle regex with brittle LIKE / SUBSTR / INSTR; use structured AST facts with exact equality. | PR handover 06-16 "CRITICAL" / "KEY DISCOVERY" (`ca2777e58d`, 06-17); AIS `LINTER_ARCHITECTURE.md` F4 / C5 (`9b8ca2d`, 08-01) | Forwardref port gave 42, then 29, findings vs the oracle's 33. `INSTR` silently dropped UG002. The structural fact matched the oracle exactly (8 findings). | **N/R / ADOPTED by construction.** Regex stays native and LIKE is never proposed (Tempo-sql says so explicitly). |
| 19 | Lower patterns to embeddable SQL fragments over `(fid, pre)` roots; the caller writes the anti-join, aggregate or recursion. Don't build a SQL interpreter. | AIS `LINTER_ARCHITECTURE.md` §8 / C4; `LINTER_CASE_STUDY.md` §4 (`9b8ca2d` / `55d9157`, 08-01) | 41 specs: 16 negation, 24 counting, 1 closure. The API is still unbuilt (`NotImplementedError`). | **DLRE: implicitly ADOPTED** (bounded candidate SQL plus authored relational stages). <br>**SSC: ADOPTED in design, VIOLATED in execution.** <br>• Built a SQL program-counter interpreter ("A large statement can still behave like a procedural interpreter", `9207ff45`). <br>• SIM905 visits 303k states (10-24). <br>• Counter-example: the hand-written B023 query went 4,616 → 49 ms (`a0051ae8`, 10-22). <br>**N1562: RE-LEARNED.** Inlining produced 164 MB of SQL; one CTE per operator produced 8,397 CTEs (10-24). Still open. <br>**Main / Tempo-metal:** SQL runs no rules. |
| 20 | 77 of 99 linter rules need only single-file syntax; partition by file and reserve global machinery for the rest. | AIS `LINTER_ARCHITECTURE.md` §4 (`9b8ca2d`, 08-01) | 12 rules cross-file, 7 repository state, 3 policy-only. | **DLRE: ADOPTED** (`partitionBy: "file"`, `data-layer-batch-six.md`, 10-20). Re-learned at the edges: a file gate expanded one global clause to the whole repository (`4def200d`, 10-21). <br>**Tempo-sql: ADOPTED** (105 file-regex rules, 80 AST). |
| 21 | Expose node extents (G1) and child ordinals (G2) as columns, and let the caller join, instead of computing closures or extending the syntax. | AIS `LINTER_ARCHITECTURE.md` §7 (`9b8ca2d`, 08-01) | G1 needed by 16 specs, G2 by 15. | **DLRE: RE-LEARNED, now ADOPTED.** Recursive descendant closure with 1.1M rows was 27% of CPU (10-20); interval join 0.99 ms vs 267 ms (10-22). About 2 days. <br>**SSC / Main / Tempo: ADOPTED.** <br>**N1562:** extents only. |
| 22 | Ordered sibling semantics need an NFA (a recursive CTE over pattern position and cursor), not independent `cpos <` joins. | AIS `bench/AST_GREP_SQL_GAPS.md` P0 (main only, `4598600`, 09-21) | rust-use gave 15,783 vs 15,285 with joins; exact with the NFA. | **Tempo-sql: RE-LEARNED** (09-24). <br>**SSC:** first "Not adopted" (10-19); built in substance on 10-22 (`ffc685ce`). <br>**Tempo-metal:** GPU analog (10-21). <br>**DLRE:** N/R; ordering stays in the Rust matcher. |
| 23 | ast-grep's `stopBy` default is `neighbor`, not `end`. | AIS `bench/AST_GREP_SQL_GAPS.md` (`4598600`, 09-21) | `python-function` gave 5,366 vs 56,465 roots; exact after the fix. | **Main / DLRE: ADOPTED** (`code-rule-dsl-reference.html`, `042d82eb`, 10-08). <br>**Tempo-sql: RE-LEARNED.** <br>**Tempo-metal: RE-LEARNED** on 10-20 and fixed the same day. <br>**SSC:** related (`stopBy: end` expansion, 10-20). |
| 24 | Comments are tree-sitter "extras": carry an `is_extra` flag per kind (zero bytes per node) so wildcards skip them. | AIS `bench/TAPE_AST_GREP_RECONSTRUCTION.md` (main only, `9449260`, 09-22) | 0 vs 1 matches across a comment; no kind varies in extra status over 18.5M nodes. | **Main / DLRE / SSC / N1562 / Tempo-sql: ADOPTED.** <br>**Tempo-metal: RE-LEARNED** on 10-20 and fixed the same day. |
| 25 | Restoring a cached tape document is 20–26x faster than reparsing. | AIS `bench/TAPE_AST_GREP_RECONSTRUCTION.md` (`9449260`, 09-22) | 20.06x median and 26.19x on the largest files, with 0 mismatches. | **ADOPTED** in Main, DLRE and Tempo-sql. Parse is about 11 s of DLRE's 2,507 s, so it is not the bottleneck. |
| 26 | A fixed deadline or budget does not scale with corpus size; acceptance must be structural (work proportional to anchors and results, not total nodes). | AIS README Status "Correctness scales; the deadline does not" (`28810b7`, 08-01); TAIL "Structural definition of solved" / S294 (`de6d285`, 08-09) | At 59.5 MB: p50 103 ms, p99 5,006 ms. The target corpus is 21x larger. | **DLRE: RE-LEARNED** on 10-21. Full-snapshot-only failures: the 32 MB input budget, the 64 MiB arena (889 failures), and 34 work-allowance failures. Still unresolved (WP R6). <br>**N1562: RE-LEARNED** (126,142 budget failures, 10-16). <br>**Tempo-metal: VIOLATED** (fixed 2e9-byte cap). |
| 27 | Per-query or per-partition precomputed indexes are rejected: indexes must be query-independent and built once. | AIS README R14b (`d88209d`, 08-01); SQL_ONLY "Rejected approaches" (`021f138`, 07-27); TAIL S292 | Temp node table 5000 → 9 ms per query, but it fails at 7.55M nodes; the fitted predicate table hid a wrong answer. | **DLRE: RE-LEARNED 3 times.** <br>• 695 field-index rebuilds over 31.4M nodes (10-21). <br>• 88 per-partition builds (10-21). <br>• 1,588 per-selector `CREATE INDEX` statements reduced to 4 (`75a39700`, 10-24). <br>**Tempo-sql: RE-LEARNED in 2 hours.** <br>**Main / N1562: ADOPTED.** |
| 28 | Statement count, SQL size, VM steps and self-reported durations are proxies; only end-to-end CPU and wall time on the real corpus count. | PR `LINTER_FACT_FIRST_HANDOFF.md` (`7c4469493d`, 06-10: reported 3.4 s vs 79 s wall); AIS S225 (`953ae50`, 08-03); README "Answer size … does not predict" (`28810b7`); TAIL | PR: reported 2.5 s vs 51.3 s wall. AIS: three counters gave small numbers while the profiler showed `likeFunc` at 22.8%. | **DLRE: acknowledged, then VIOLATED in practice.** <br>• WP (`00e04cd4`, 10-23): "Counts alone cannot establish recovery." <br>• 10-23 → 10-24 work was nonetheless driven by a 100-statement gate on a 2-observation guard: R5-B −38.9% submissions for −0.3% CPU; admission −190 submissions for +2% CPU. <br>• The full repository has not been remeasured. <br>**SSC: RE-LEARNED.** 72 executions met the budget while CPU went 5.1 → 14.9 s (`5f1960f2`, 10-21; review `e5783859`, 10-22); "SQL size alone is not a cost model" (10-24). About 3–4 days. <br>**N1562: RE-LEARNED and repeating now.** Statements −94.6% for runtime −4.6% (10-16). Since `bc7840d4` on 10-24, about 60 "compact SQL" commits with no runtime benchmark. |
| 29 | Keep diagnostics when removing a gate, and change one variable at a time. | AIS SQL_ONLY §2.3; RECONSTRUCTION "The corpus doubled" (`946c2ee`, 07-19) | 331 failures left unclassified; doubling to 2 MB and turning the flag off together hid the cause. | **DLRE: partly RE-LEARNED.** The gate removal of 10-23 needed restoring. |
| 30 | A small or fresh store hides volume pathologies: test against full-suite volume. | PR `lazy_architecture_yaml_regression_analysis.md` "Combined-Store" (`f8c5ebc437`, 06-25) | 15 ms in isolation vs a 10 s timeout on the combined store. | **DLRE: RE-LEARNED.** The 3.3x and +363% regressions were visible on 12-file controls on 10-21, but the 19.9x only appeared at full scale on 10-23. |
| 31 | Use 3-run medians, judge slices on structural counters, and expect page-size noise below about 2%. | PR row-width Step 3 / path slice (`1ed30f8551`, `3ee9e99233`, 06-26) | Outliers of 31.6 s and 44.9 s against a median of about 20 s. | **DLRE: ADOPTED** (5-run medians on the baseline). |
| 32 | Store anonymous nodes; parity depends on them. | AIS `DESIGN.md` S040 (`307e1b2`, 07-03) | 210 more shapes translatable at 1.54x the nodes. | **ADOPTED** (the tape stores all nodes). |
| 33 | Excess relation construction is the common tail-latency fault, and removing it alone is not sufficient. | TAIL S280–S289 (`de6d285`, 08-09) | S285: 12.55 → 32.11 s. S289: total down only 5.7%. | **DLRE / SSC: RE-LEARNED** (1.36M materializations; 1,101 CTEs; frame expansion). |
| 34 | Packed, query-independent child arrays are the selected design point (S303). | TAIL S296–S303 (`247e628`, 08-10) | Median 766.8 → 219.1 ms at 4.63 B/node. | **DLRE: RE-LEARNED.** Field postings cut child examinations by 99.67% (427.8M examinations, 10-21). |
| 35 | A wrong or rows=1 estimate on an unmaterialized tiny candidate joined to a broad relation blows up; pin the join order. | PR `lazy_architecture…` "Combined-Store"; AIS S069 | 10 s timeout became 0.094 s. | **DLRE / SSC: RE-LEARNED** (CROSS JOIN fences on 10-22 and 10-23; SSC on 10-20 and 10-21). |
| 36 | Physicalizing relations exposes wrong semantics hidden by generic, zero-defaulted fields. | PR row-width import and annotation slices (`5125122a2a`, `8723da96fa`, 06-27) | 5 false positives removed; `type_safety_patterns` went 0 → 161 real findings. | **N/R**, but DLRE's parity harness covers the same risk. |

---

## Narrative

PR 2539 and the ast-index-spike together cost about two months (06-04 → 08-11), and their conclusions are consistent:
- Store a compact, query-independent tape.
- Physicalize narrow facts per relation family, driven by demand.
- Anchor on rare kinds and gate every probe on measured selectivity.
- Express rules as SQL fragments the caller composes, never as a SQL interpreter.
- Accept nothing except end-to-end time on the real corpus.

The current branches adopted the storage half: the tape, interning, `is_extra`, `stopBy: neighbor` and file ownership. They re-learned the execution half, often several times within a week.

**Part A.** The headline gap is real, but its framing needs correcting.
- PR 2539 ran on its whole repository: 5,073 files in 70.6 s wall on 06-20. Its "seconds" figure is the 500-file sample (18.0 s, 101 linters).
- DLRE before the SQL change was on par with it: 0.28 ms CPU per file-rule, against PR's 0.36 ms wall.
- The 20x regression (first 50 rules: 125.8 → 2,507.5 s CPU) came entirely from moving candidate selection into SQLite FTS5 and generated per-batch SQL.
- So the problem is not "Rerail is slow". It is that the Rerail SQL path regressed by about 15–40x per file-rule against what PR 2539 had already achieved.

**The lessons that matter most for the direction we pick now**

1. **The proxy-metric trap (#28).** Statement counts, SQL bytes and VM steps on a 2-observation guard drove the whole 10-23 → 10-24 DLRE effort and all of N1562's current work. The full repository has not been remeasured since 2,507 s. Any direction we pick needs a full-corpus CPU and wall gate first.
2. **Temp tables plus ANALYZE lose to fused plans (#5, 57–115x).** DLRE's execution model is built on CTAS, indexes and ANALYZE. This is the clearest live violation.
3. **Probe and anchor selectivity (#2, #3, #35).** DLRE re-learned this at least four times and SSC re-learned it once. The FTS5 anchor path that caused the 20x is exactly the ungated-probe failure.
4. **Fragments, not an interpreter (#19, #16).** Both SSC's program-counter SQL and N1562's 164 MB, 8,397-CTE statements re-learn this. The hand-written B023 query (94x faster) shows the intended shape.
5. **Union sharing loses; share exact closures only (#9, #15).** DLRE re-learned it 3 times and SSC re-learned it. N1562's one-statement-per-workspace design is built on the opposite assumption.
6. **Demand-driven narrow facts per family (#11, #13).** Cited on 10-19 and still re-learned 3 days later (1.26 GB of JSON rows).
7. **Measure what is running (#10).** All four branches re-learned it. N1562's invalid 17.189 s result is still in its benchmark history.

**The most expensive re-learnings**
- **Tempo-metal's per-tuple task rows:** about 16 days.
- **DLRE's FTS5 / SQL anchor path:** visible as 3.3x on 10-21, pursued anyway, and measured at 19.9x (about 40 CPU-minutes) on 10-23. It is still unrecovered at full scale.
- **SSC's and N1562's statement-count detours:** 3–4 days each, and N1562 is still in its detour.
- **N1562's callback revival:** 2–3 days for a lesson Landing, PR 2539 and AIS had all already recorded.
