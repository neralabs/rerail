# Explore X6: the rule side, hundreds of rules on one declared-facts layer

Branch `Moti-Velz/facts-engine-x6-rules` from integration `eee7d6ac3`. Code under `spikes/x6-rules/` (hand-written facts and rules, linker, harness; disposable), plus a stage-dump hook in `spikes/facts-a` (`X6_STAGE_DUMP`). Runs under `~/rerail-bench/facts/explore/x6-rules/runs/`. All runs were unlocked on a shared machine (load 10–60). Arranged-record counts are deterministic; RSS and wall times are calibration only. Binaries: integration `facts-a`/`facts-b`, code unchanged.

## Decision-table row

| Design | Memory, load / after 1,000 edits (40 worktrees) | Per-edit p50 / p95 / p99 | Exactness | What it breaks | Kill point | Commits | Report |
|---|---|---|---|---|---|---|---|
| **Rule side (X6).** One declared-facts layer, shared:<br>• 23 per-content facts (21 `py_*`, 2 `sel_*`, as matchers), computed once per content<br>• membership `proj_*` facts (the 12 symbol-table stages and the linker), computed once per repository<br>• the five heaviest rules rewritten by hand as short reader stages<br>• all rules linked into one plan graph (`link.py --mode merged`) | Dataflow, dc+ef+rc pack, matched-load pair (footprint; RSS):<br>• load: 19.1 GB; 25.3 GB, 126M arranged (originals: 26.3 GB; 30.4 GB, 146M)<br>• edit 1,000: 25.6 GB; 34.6 GB, 224M arranged (originals: 32.7 GB; 41.4 GB, 253M)<br>Facts alone hold 80.6M records at load (symbol table 19.3M, linker 61.4M). | Dataflow pack: 22.8 / 72.6 / 226 ms (originals, same binary and load: 21.1 / 74.4 / 228). Unchanged.<br>Per-file engine, re-evaluating one content for N rules: 0.27–0.72 ms per rule for the symbol and contract shapes; facts 0.5–1.4 ms once. | **Exact**:<br>• 0 differences on `2b1504adc` over every compared output: dc+ef 115 outputs / 9.04M rows; dc+ef+rc merged 121 / 9.04M; rc probe 107K; qc 31 stages × 9,010 contents (8.0M rows) plus 12 repository shards; sfp 53.3M rows in its live configuration<br>• dc 4,931 = 4,931 against the oracle<br>• incremental ≡ from-scratch: 300-event prefix, 18 worktree checks, 0 differences, 0 runtime refusals<br>• qc is still refused by the compiler, unchanged | I3, I8: nothing.<br>I6/I7: per-content facts carry `__path`; `py_all` reads the file path, so it is not path-free.<br>Gate-free facts are 6–11× the rows of the gated inputs they replace (contracts 637K vs 103K; `py_nodes` 16.05M vs 1.42M).<br>Merged plan:<br>• one refusal or content failure takes down every rule in the pack<br>• per-rule scope is emulated (scope union plus path filters)<br>• facts-b's export-id `STRIDE` caps one program at 999 exports, about 120 rc-shaped rules | **Killed for full maintenance at load**: 19 GB against a 250 MB target. The shared facts alone are 80.6M records.<br>Per rule:<br>• dc shape +18.9M records (~1.7 GB) on 1 worktree<br>• ef shape +1.3M (~0.5 GB) on 1 worktree<br>• rc shape +20.5K records, 5.6–7.9 MB, on 40 worktrees<br>So 32–45 light rules alone fill 250 MB.<br>**Alive** as the per-file side: linear, cheap per content, facts paid once. | `7870ad1c8`, `195a7a6e5`, `fbf16af3a`, `3b3a93136`, `515989afc`, `bf55d9957`, this report | `docs/facts/reports/explore-x6-rules.md` |

Readings:

1. **Sharing works and is exact. It removes duplicated facts only.**
   - Of the originals' 145.9M records at load, 19.3M are the symbol table that dc and ef each compute; merged they are computed once (−19.95M, with 0.6M of ef's dropped identity stages).
   - The linker (61.4M) and each rule's private stages (dc 36.9M, ef 8.4M) stay.
   - At 1,000 edits the saving is 28M records (11 %).
2. **On the dataflow, every rule costs its private cross-file state, and that scales with N.**
   - Facts are shared, but a rule that keeps cross-file stages keeps their arrangements.
   - Even a rule whose logic is entirely per content (rc shape) costs about 7 MB in the dataflow on one worktree (9,010 contents), as exported rows plus small arrangements.
   - Hundreds of rules cannot be held in full maintenance, with or without shared facts.
3. **On the per-file engine, the design scales linearly at 0.27–0.72 ms per content per rule for these shapes**, with the fact layer paid once at 0.5–1.4 ms per content.
   - Heavier shapes cost more: `quality-conditional-1` ≈ 1.9 ms per content, and `state-flow-project-1` in scan mode ≈ 22 ms per selected content.
   - So "maintain the facts, recompute rule logic over them" (X4's design) is where the rule side pays off. The rewrites are its input.

## 1. What was built

- **Facts layer**, `spikes/x6-rules/pack/facts.*.json`, in DECLARED-FACTS form (`over`, `define.match` | `define.sql`):

  | File | Facts | Per content | Membership |
  |---|---|---|---|
  | `facts.symbols.json` | 11 `py_*` (`py_functions`, `py_classes`, `py_decorators`, `py_parameters`, `py_parameter_annotations`, `py_return_annotations`, `py_imports`, `py_bases`, `py_all`, `py_all_names`, `py_modules`) + 12 `proj_*` (`proj_inventory`, `proj_declarations`, `proj_parameters`, `proj_annotation_names`, `proj_imports`, `proj_resolved_names`, `proj_base_edges`, `proj_hierarchy`, `proj_public_methods`, `proj_explicit_exports`, `proj_export_names`, `proj_exported_roots`) | the 10 matchers dc and ef held byte-identically | the 12 stage texts they held byte-identically. `proj.module_of` / `proj.reference_edges` (the linker) stay the compiler's built-in `references` views |
  | `facts.contracts.json` | 9 gate-free `py_*` (`py_definitions`, `py_decorations`, `py_callees`, `py_keyword_arguments`, `py_assignments`, `py_typed_parameters`, `py_annotation_tokens`, `py_base_tokens`, `py_assigned_tokens`) | 637K rows on the tree (rc's 10 gated inputs: 103K) | — |
  | `facts.nodes.json` | `py_nodes` (gate-free node dump, 63 columns) + `sel_sfp`, `sel_qc` (rule-specific selections) | `py_nodes` 16.05M rows (mean 1,782 per content); `sel_qc` 1.42M | — |

- **Rules**, `pack/rules/*.json`: short stage lists that read facts by name. The rule's own selection is an ordinary filter or semijoin.

  | Rule | Before | After |
  |---|---|---|
  | `unreachable-internal-symbols` (dc) | 13 inputs, 25 stages | 2 private matchers, 9 stages |
  | `exported-families` (ef) | 11 inputs, 22 stages | 1 private matcher, 7 stages |
  | `required-contracts` (rc) | 10 inputs, 14 stages; logic 11.0 KB | 0 inputs, 16 stages; logic 3.7 KB, reading 6.2 KB of shared facts |
  | `state-flow-project-1` (sfp) | 29 inputs (87.7 KB, of which 81.7 KB dumps), 32 stages | 4 private matchers (5.9 KB), 31 stages |
  | `quality-conditional-1` (qc) | 20 dump inputs (25.8 KB), 32 stages | 0 inputs, 31 stages |

  - The 25/20 dump chunks became one `syntax_rows` stage over `py_nodes ⋉ sel_*`, which nulls the fields the original chunk did not project.
  - `sfp`'s gate selects whole contents.
  - `qc`'s selection is not closed under descendants (SCHEMA § 2.3), so it recomputes `ordinal`, `edges` and `ownership` over its own subset. That is the rule's cost, as INVARIANTS says.
- **Reader binding**, `link.py`: what a compiler with a `facts` block would do, done by hand.
  - A per-content fact becomes a matcher input under its name. The compiler shares one input per evidence text in a document, so the fact is computed once per content.
  - A membership fact becomes a stage before its readers.
  - `--mode separate` gives one clause per rule.
  - `--mode merged` gives one clause, so the normaliser's hash-consed graph computes each shared node once. Rule stages are prefixed `<alias>__`, and rule scopes are unioned, with per-rule path filters where a rule's scope is narrower.
- **Harness**:
  - `stages_equal.py`: every original output against its counterpart, as multisets.
  - `synth.py`: N rules of a shape. Variant k changes one regex or literal of its first private stage and adds `not regex ^x6v<k>$` to every private matcher, so it finds what its shape finds but shares no plan node or evidence text.
  - `unshare.py`: today's shape, where each rule has its own copy of every fact.
  - `scale_perfile.py`: `facts-a xexport` over the 9,010 oracle contents.
  - `scale_flow.py`: `greplay --parity` load with `FACTS_PROFILE=1`, arranged records split into linker, facts and per rule.
  - `facts-a X6_STAGE_DUMP`: stage dumps of facts-a's repository run, for qc and the live sfp configuration.

## 2. Exactness

| Rule | Check | Result | Run |
|---|---|---|---|
| dc + ef, separate | every original output (findings + 113 stages including the 77 linker views), parity run with `FACTS_STAGE_OUTPUTS=1` | 115 outputs, 9,041,025 rows, **0 differences** | `20261023-155254-parity-symbols-separate-stages` vs `20261023-154919-base-parity-4-stages` |
| dc + ef, merged | same, `<alias>__` mapping | 115 outputs, 9,041,025 rows, **0 differences** | `20261023-155643-parity-symbols-merged-stages` |
| dc vs oracle (`suite-b5`) | `findings_parity.py` | **4,931 = 4,931**, 0 missing, 0 extra | both runs above |
| ef vs oracle | same | 0 = 0 | both runs above |
| rc | stages + per-content stage probe; exports `cmp` | 6 outputs + 15 probe outputs (107,209 rows), 0 differences; exports byte-identical; oracle 0 = 0 | `…162324-parity-contracts-separate-stages`, `…162200/162325-…probe-*` |
| dc + ef + rc, merged | every original output | 121 outputs, 9,041,129 rows, **0 differences** | `…162725-parity-symbols-contracts-merged-stages` |
| sfp | dataflow, default configuration | 0 = 0 rows. Vacuous: the default `entryPoint` gate holds on no file | `…170745-nodes-sfp-parity-final` |
| sfp | per-file engine, live `scan_project_sources` configuration | 3,095 contents × 35 tables, 53,258,431 rows, 0 differences; 12 repository shards, 0 differences | `harness/perfile-sfp-equal.json`, `harness/shards/` |
| qc | per-file engine | 9,010 contents × 31 stages, 7,998,252 rows, 0 differences | `harness/perfile-qc-equal.json` |
| qc | repository shards | 12 shards covering every file (+ its 4 `locations` files), 31 stages, 8,326,656 rows, 0 differences. One full pass of the rewrite would need ≈ 50 GB in facts-a, because it loads the gate-free `py_nodes` of every file | `harness/shards/` |
| dc + ef + rc merged, delta maintenance | `greplay --events … --limit 300 --check-every 25 --sweep 2`, engine from-scratch | 13 checks, 18 worktree checks, **0 differences**, 0 runtime refusals | `20261023-163835-exactness-pack-dcefrc-300` |

Refusals:

- `qc` is refused by the compiler, identically for the original and the rewrite: `stages/block_results` is an order-dependent bag (`UNION ALL`) recursion. This comes before the generator's `IN (subquery)` under OR refusal, and both stages are the rule's own text.
- No other refusal: no static, no runtime, none live.

## 3. Memory at 40 worktrees (dataflow, full maintenance)

### 3.1 Load, 0 events, `FACTS_PROFILE=1`, same binary

| | Arranged | Symbol facts | Linker | dc private | ef private | RSS | Footprint | Run |
|---|---|---|---|---|---|---|---|---|
| originals dc + ef (2 plans) | 145.89M | 38.63M (twice) | 61.38M | 36.91M | 8.96M | 24.5 GB | 26.6 GB | `20261023-161935-mem-load-base-dcef` |
| pack dc + ef (merged) | **125.94M** (−13.7 %) | **19.26M** | 61.38M | 36.91M | 8.38M | 19.1 GB | 21.1 GB | `20261023-160600-mem-load-symbols-merged` |

### 3.2 Full replay (1,111 events, 1,000 single-file edits), originals dc+ef+rc vs pack dc+ef+rc

| | Loaded (RSS / footprint) | Edit 1,000 | End | Arranged at edit 1,000 / end | Peak RSS | Edit p50 / p95 / p99 / max | Load avg | Run |
|---|---|---|---|---|---|---|---|---|
| originals (3 plans), run 1 | 30.8 / 23.8 GB | 41.4 / 31.4 GB | 64.8 / 49.2 GB | 252.8M / 349.7M | 68.5 GB | 26.0 / 96.1 / 299 / 2,206 ms | 34 → 10 | `20261023-175317-protocol-base-dcefrc` |
| pack (1 plan), run 1 | 25.1 / 18.0 GB | 27.4 / 29.4 GB | 43.2 / 43.4 GB | 224.4M / 311.2M | 46.1 GB | 43.4 / 153.5 / 406 / 3,016 ms | 40 → 34 | `20261023-171753-protocol-pack-dcefrc` |
| **originals (3 plans), run 2** | 30.4 / 26.3 GB | 41.4 / 32.7 GB | 65.6 / 44.8 GB | 252.8M / 349.7M | 68.9 GB | **21.1 / 74.4 / 228 / 1,610 ms** | 10 → 8 | `20261023-183640-protocol-base-dcefrc-r2` |
| **pack (1 plan), run 2** | 25.3 / 19.1 GB | 34.6 / 25.6 GB | 55.3 / 33.9 GB | 224.4M / 311.2M | 58.1 GB | **22.8 / 72.6 / 226 / 1,445 ms** | 10 → 10 | `20261023-181709-protocol-pack-dcefrc-r2` |

- RSS samples swing with macOS memory compression; footprint and arranged records are the comparable figures.
- Arranged records are deterministic and identical across runs. The pack saves 20–38M records throughout: 146.2M → 126.2M at edit 100, 252.8M → 224.4M at edit 1,000, 349.7M → 311.2M at the end. That is the symbol table computed once instead of twice.
- Growth from divergence is the same: +98M (pack) against +107M (originals) from edit 100 to edit 1,000.
- Rows 1 ran under different load (pack 40, originals 34 → 10). Rows 2 are the matched pair (§ 4).

## 4. Per-edit latency at lower load

Run 2 of each, back to back at load ≈ 10, same binary, same events:

| | Single-file edit p50 / p95 / p99 / max (n = 1,050) | Bulk p50 / max (n = 5) | Commit p50 / max (n = 50) | New worktree p50 (n = 5) | Commit init | user s |
|---|---|---|---|---|---|---|
| originals dc + ef + rc (3 plans) | 21.0 / 74.4 / 228 / 1,610 ms | 7.8 / 22.4 s | 2.2 / 32.0 s | 28.3 s | 92.7 s | 1,137 |
| pack dc + ef + rc (1 merged plan) | 22.6 / 72.6 / 226 / 1,445 ms | 7.9 / 20.2 s | 2.0 / 24.4 s | 28.1 s | 89.3 s | 1,126 |

Per-edit latency is unchanged by sharing: p95 73 against 74 ms. Computing the symbol table once saves memory, not edit time. An edit's delta still flows through every rule's private stages. Both are inside the 500 ms p95 target; the memory is not.

## 5. Scaling with N rules

### 5.1 Per-file engine

`facts-a xexport` over the 9,010 oracle contents, 8 threads. User CPU seconds, so per content per rule = s / 9,010.

Symbol shapes (dc:ef = 1:1; `--stages reference`, the only evaluator that plans the linker's views; `20261023-160820-perfile-scale-symbols`, `…162025-…-unshared`):

| N | user s | wall s | max RSS | exports | plan nodes | inputs |
|---|---|---|---|---|---|---|
| 0: facts only (11 `py_*` computed, keys exported) | **12.8** | 3.1 | 0.28 GB | 47 MB | 100 | 11 |
| 2 | 37.9 | 10.7 | 1.25 GB | 770 MB (linker summaries) | 1,576 | 13 |
| 20 | 82.5 | 18.8 | 1.15 GB | 815 MB | 4,672 | 40 |
| 50 | 154.8 | 35.0 | 1.19 GB | 890 MB | 9,832 | 85 |
| 100 | 277.5 | 61.4 | 1.66 GB | 1,015 MB | 18,432 | 160 |
| 200 | 515.7 | 142.1 | 1.97 GB | 1,265 MB | 35,632 | 310 |
| today's shape (each rule its own facts), 20 | 157.2 | 41.3 | 3.2 GB | 2,102 MB | 8,990 | 230 |
| today's shape, 50 | 346.9 | 110.2 | 6.1 GB | 4,211 MB | 21,110 | 575 |

Contract shape (rc; `--stages engine`, facts-a's columnar engine; `20261023-165900-perfile-scale-rc-engine{,-unshared}`):

| N | user s | wall s | max RSS | exports | plan nodes |
|---|---|---|---|---|---|
| 0: facts only (9 `py_*`) | **4.7** | 1.0 | 0.24 GB | 50 MB (keys) | 6 |
| 1 | 9.2 | 1.4 | 0.18 GB | 2.9 MB | 31 |
| 20 | 100.4 | 16.2 | 0.33 GB | 32 MB | 525 |
| 50 | 259.3 | 47.4 | 0.56 GB | 80 MB | 1,305 |
| 100 | 562.3 | 101.7 | 0.93 GB | 160 MB | 2,605 |
| 200 | 1,265.9 | 226.5 | 1.71 GB | 325 MB | 5,205 |
| today's shape, 20 / 50 | 139.3 / 356.3 | 29.5 / 64.7 | 1.0 / 2.3 GB | 32 / 80 MB | 525 / 1,305 |

Notes on the rc runs:
- With `--stages reference`, rc costs 32 s per rule, because the reference evaluator runs nested loops over the larger general facts. At N = 100 that run terminated abnormally after 2,863 s user; not diagnosed (`20261023-163555-perfile-scale-rc`).
- The engine is the per-file engine's real evaluator.

Node-dump shapes (X6Dump's harness, facts-a per-file thread-seconds):

| Shape | Original inputs | Rewrite inputs | Rule SQL, original → rewrite |
|---|---|---|---|
| qc | 20 dumps, 16.7 s | `py_nodes` 11.8 s (fact) + `sel_qc` 2.7 s (rule) | 15.1 → 14.8 s |
| sfp, scan mode, 3,095 contents | dumps 5.6 s | `py_nodes` 5.9 s + `sel_sfp` 0.05 s | 70.6 → 68.6 s |

**Cost per rule and slope, fact layer separated:**

| | Fact layer, once | Per rule, shared facts | Per rule, today's shape |
|---|---|---|---|
| symbol shapes (dc/ef) | 12.8 s = **1.42 ms/content** (11 facts) | **2.41 s = 0.27 ms/content/rule**; +2.5 MB exports, +172 plan nodes | 6.32 s = 0.70 ms/content/rule; +70 MB exports, +404 nodes |
| contract shape (rc) | 4.7 s = **0.52 ms/content** (9 facts) | **6.48 s = 0.72 ms/content/rule**; +1.6 MB exports, +26 nodes | 7.23 s = 0.80 ms/content/rule |
| qc shape | `py_nodes` 11.8 s = **1.31 ms/content** | (2.7 + 14.8) s = **1.9 ms/content/rule** | 31.8 s = 3.5 ms/content/rule (dumps + SQL) |
| sfp shape, scan mode | `py_nodes` 1.9 ms/selected content | **22 ms/selected content/rule** (its own SQL) | 24 ms |

Notes on the slope:

- The per-rule cost is the rule's own per-content work: its private matchers, and its stages pushed to the per-file plan.
- Sharing removes the per-rule copies of the facts: 62 % of the per-rule CPU for the symbol shapes, 10 % for rc, whose own joins over general facts dominate.
- Re-evaluating one edited content for 200 rules of the symbol/contract shapes costs 54–144 ms of CPU on one thread, plus 2 ms of facts.
- 200 qc-shaped rules would cost ≈ 380 ms; 200 sfp-shaped rules (scan mode) 4.4 s. So rule logic, not facts, sets the per-edit budget.
- Compile size grows linearly: 35,632 plan nodes for 200 symbol rules, one plan per document.

### 5.2 Dataflow (full maintenance), one worktree (the oracle tree), load

`scale_flow.py`; runs `20261023-163326-flow-scale-ef`, `…163550-flow-scale-dc`, `…170742-flow-scale-rc`.

| Shape | N | Arranged total | Linker | Facts | Rules | Per rule | RSS loaded | Exported-row cache |
|---|---|---|---|---|---|---|---|---|
| ef | 1 / 2 / 4 / 8 | 7.42M / 8.74M / 11.39M / 16.68M | 0 | 4.76M | 2.66M → 11.92M | first 2.66M, then **1.32M** | 2.15 / 2.69 / 3.70 / 5.73 GB (**+0.51 GB/rule**) | 39 MB, +0.14 MB/rule |
| dc | 1 / 2 / 4 | 88.9M / 107.8M / 145.5M | 58.19M | 4.62M | 26.1M → 82.7M | first 26.1M, then **18.9M** | 9.0 / 12.3 / 15.7 GB (**+1.67 GB/rule**) | 326 MB, +2.0 MB/rule |
| rc | 0 (facts probe) / 1 / 20 / 50 / 100 | 4.8K / 22.8K / 455K / 1.14M / 2.28M | 0 | 0: rc's facts never leave the per-file side | **22.8K/rule** | 22.8K | 0.17 / 0.04 / 0.22 / 0.40 / 0.76 GB (**+6.7 MB/rule**) | 1.4 MB, **+1.43 MB/rule** |

Light shape at 40 worktrees (load, 0 events; replay exports, `--stages engine`; `20261023-185708-flow40-rc`):

| rc rules | Arranged | Exported-row cache | RSS loaded | Footprint |
|---|---|---|---|---|
| 1 | 20,490 | 1.6 MB | 58 MB | 53 MB |
| 50 | 1,023,128 | 80.7 MB | 444 MB | 328 MB |
| per rule | **20.5K** | **1.6 MB** | **7.9 MB** | **5.6 MB** |

So 32–45 light rules alone fill the 250 MB budget for 40 worktrees, before any fact or heavy rule.

- **First rule vs marginal.** The normaliser hash-conses nodes, so within one merged plan the stages that two variants hold identically (those not reached by the varied literal) are computed once. The first variant of a shape therefore pays more than the next.
  - For truly different rules, the marginal cost lies between the two figures: dc 18.9–26.1M, ef 1.3–2.7M.
- **The fact layer in the dataflow:**
  - symbol membership facts: 4.6–4.8M records on one worktree, 19.3M on 40;
  - linker: 58.2M on one worktree, 61.4M on 40 (mostly base);
  - contract and node facts: 0 records, because readers whose logic is per content export only their results.
- **Slope.** The cost is linear in N with the shape's private cross-file state:
  - 200 light (rc-shaped) rules ≈ 1.4 GB on one worktree, and ≈ 1.1–1.6 GB on 40 worktrees at load;
  - 200 ef-shaped ≈ 100 GB;
  - 200 dc-shaped rules are not runnable (≈ 3.8G records).
  - rc at N = 200 was not run: its program needs 1,600 exports, and greplay's global export id is `(program + 1) × 1000 + export` (`gen/real.rs` `STRIDE`), so more than 999 exports per program collide.

## 6. What it breaks, and caveats

- **I3 (no text recognition): unaffected.** Binding facts by name is the language's job: `link.py` is a hand-written compile step. Sharing comes from one plan graph, not from recognising text.
- **I6 / I7: per-content facts are not path-free.**
  - Every matcher row carries `__path`.
  - `py_all` keeps the original's `file.path` predicate, so it is keyed by (path, content) and not by content.
  - Proper declared facts need the path predicates moved to the readers.
- **Gate-free facts are larger than the gated inputs they replace**: contracts 6×, nodes 11×.
  - That is cheap per content.
  - It is fatal if a fact is ever materialised for the whole repository: facts-a's one-pass repository run of qc would need ≈ 50 GB.
  - Per-content facts must stay on the per-file side and reach the cross-file side only through readers' projections. The compiler already pushes them there (rc's facts: 0 records in the dataflow).
- **I8 / I9: unaffected.**
  - The merged plan goes through the same generator, with 0 differences against from-scratch.
  - qc is still refused, and for the same reason.
- **Merged plan (one plan graph across rules), as built by hand here:**
  - one rule's refusal or one content's failure fails the whole pack;
  - per-rule scope needs a union scope plus per-rule path filters (`<alias>__scoped_<fact>`), which a per-clause scope gives for free;
  - findings come out through one union output;
  - export ids are capped by `STRIDE`.
  - A real compiler should share nodes across plans (CSE by digest) and keep the plans separate.
- **Harness gaps:**
  - sfp's dataflow check is vacuous in the default configuration. Its live configuration is checked on the per-file engine, and its dataflow export took more than 45 minutes in the reference evaluator, so it was stopped.
  - qc's repository check is sharded: 12 shards, each with the 4 `locations` files.

## 7. Kill reasoning

- **Full maintenance** (every rule's cross-file stages kept in the dataflow) dies at the first number: 18–19 GB footprint at load for three rules (two runs), against a 250 MB budget.
  - The shared facts alone are 80.6M records: the symbol table (19.3M) and the linker (61.4M).
  - Each rule then adds its private state, measured on one worktree: from 23K records (≈ 7 MB) for a light rule up to 19–26M records for a reachability rule.
  - The rule side cannot rescue it. Sharing only deletes duplicates, here 11–14 % of records.
- **Kept alive**: the facts-once / rules-as-readers split on the per-file engine.
  - It is exact.
  - It is linear in N at 0.27–0.72 ms per content per rule for the symbol and contract shapes.
  - It pays its fact layer once per content (0.5–1.4 ms).
  - It feeds a design that maintains only the facts and recomputes rule logic: X4.

## 8. Reproduce

- Rewrite and link:
  - `python3 spikes/x6-rules/gen_symbols.py`; `gen_contracts.py`; `gen_nodes.py`.
  - `python3 spikes/x6-rules/link.py --facts pack/facts.symbols.json[,…] --rules pack/rules/R.json[,…] --mode separate|merged --out linked/X.json`.
- Exactness:
  1. `facts-a xexport --rule linked/X.json --clauses C --jobs …/oracle-b6-jobs.tsv --out E.jsonl --stages reference`.
  2. `FACTS_STAGE_OUTPUTS=1 run.sh L -- greplay --plans none --real SPEC --parity …/oracle-b6-jobs.tsv --no-oracle`.
  3. `stages_equal.py BASE NEW --map …`; `findings_parity.py`.
  - qc / live sfp: `spikes/x6-rules/run_harness.sh perfile|shards`.
- Delta maintenance: `greplay --real specs/symbols-contracts-merged-replay.json --events …/seed-5eedb/events.jsonl --limit 300 --check-every 25 --sweep 2`.
- Protocol: `greplay --plans none --no-oracle --real specs/{symbols-contracts-merged,base-dcefrc}-replay.json --events …`.
- Scaling:
  - `scale_perfile.py --facts … --mix-per-n "dc:{h},ef:{h}" [--unshared] [--stages engine]`.
  - `scale_flow.py --facts … --mix-per-n "ef:{n}" --n 0,1,2,4,8 [--stages engine]`.
