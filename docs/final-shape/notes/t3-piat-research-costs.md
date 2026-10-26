# T3 notes: Pulling-it-all-together research, costs, ledger, contracts

Research track T3. Read-only notes on the sources under
`/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/docs/rule-engine-direction/`.
All ten sources were read in full. `experiments/` was not needed to resolve ambiguities (only its listing was checked).

Citation prefixes (line numbers refer to the source file):
- **HRP** = `research/heavy-rule-profiles.md` (370 lines)
- **FM** = `research/fusion-model.md` (295 lines)
- **HI** = `research/heavy-inputs.md` (262 lines)
- **M1** = `research/m1-baseline.md` (88 lines)
- **SSC** = `research/ssc-fragments.md` (209 lines)
- **WS** = `research/workstream-scan-2026-10-25.md` (155 lines)
- **SF** = `research/shape-fixes.md` (330 lines)
- **LL** = `lessons-ledger.md` (167 lines)
- **FC** = `findings-contract.md` (89 lines)
- **AS** = `authored-sql.md` (172 lines)

Number tags: **(M)** measured, **(D)** derived/taken from elsewhere, **(A)** assumed, as the source marks them. Where a source does not tag a number, the tag is given as the source's general statement of method (e.g. "all figures measured") or marked "(untagged)".

The common corpus unless stated otherwise: nera-data-layer at the pinned commit (`2b1504ad`), 9,010 `.py` files, 137.9 MB (9,021 files including 11 Rust grammar files, 137,924,324 B) (HRP:61, M1:10, LL:32-35).

---

## 1. research/heavy-rule-profiles.md (HRP)

### Purpose and method
- Execution profiles of the 8 heaviest data-layer rules, plus a whole-gate (60-rule) CPU estimate (HRP:1-9).
- Tools/raw output in `../experiments/rule-profiles/`; `facts.sqlite`, `flow.sqlite`, corpus copy not checked in (HRP:3).
- Tags: M = measured on the gate corpus; D = from DLRE or research docs; A = assumed. "All runs were single shots on a busy machine, ±20–40%" (HRP:3).
- No per-rule full-repository timing existed (DLRE runs rules together); rules chosen by structure, scope sizes, DLRE failures and batch totals (HRP:40).
- Caveats (HRP:366-370): SQLite inserts went row-by-row through rusqlite; 588 AST programs is a lower bound (each rule option counted once); pure-SQL AST matching assumed 3–10× slower than CPU interpreter, from SSC frame engine (D).

### Superseded statements (HRP:5-7)
- §3.6(f) and the fusion "utilities" note saying recursive utilities run only on CPU/JIT are superseded by `metal-enqueue.md`: under spec §8 there is no CPU fallback; monotone recursive utilities run on Metal as GPU-scheduled waves; anything else that cannot be stratified is rejected by the verifier or reported `Unknown(resource)` (HRP:5).
- **Gap to target**: expected whole gate (25 s CPU cold, ~15 s wall, without parsing) is an order of magnitude above the 1–2 s full-repo target; target reachable only warm (parse and facts cached by content hash) and with the M3 virtual-table result at the low end. See spec §19 (HRP:7).

### Bottom line (HRP:11-18)
| Quantity | Low | Expected | High |
|---|---|---|---|
| 8 profiled rules, cold CPU, no parsing | 3 s | 10 s | 45 s |
| Whole 60-rule gate, cold CPU, no parsing | 8 s | 25 s | 110 s |
| Whole gate, cold CPU, with parsing (35 s, M) | 43 s | 60 s | 145 s |
| Whole gate, cold wall at 8–12 cores | 8 s | 15 s | 40 s |
| One-file edit, warm | 0.02 s | 0.3 s | 2 s |
| Peak memory | 1 GB | 2 GB | 4 GB |

Comparison points (D) (HRP:20-27):
| Run | Cold CPU | Memory | Edit |
|---|---|---|---|
| Pre-SQL native, 60 rules | 709 s | 16.1 GiB | — |
| DLRE SQL path, 60 rules | 7,212 s | 15.0 GiB | — |
| DLRE first 50, current | 958.6 s | — | 40.2 s |
| DLRE batch six | — | — | 54–80 s |

Statements (HRP:29-36):
- Expected gain over pre-SQL native ~10× (credible 5–80×).
- Gain sources: DLRE copies the whole syntax tree into SQL rows plus an ancestor table **8.2× node count (M)**; proposal reads the tape directly and uses interval ranges. Identifier regexes evaluated once per distinct name; fusion plus masks shrink remaining matching.
- Not removed: parsing (35 s cold), data-flow work (category H), cost of loading fact tables into SQLite.
- Biggest swing: how fact tables reach SQLite. Real SQLite tables ~16 s on this corpus (M). Virtual table over native data estimated ~1 s (A, not measured).
- SQLite query wording alone changes run time 20–35× (M).
- Metal barely matters for the gate: can only take AST-matching share, about 6 of 25 expected CPU-seconds.

### Rules chosen (HRP:42-55)
| Rule | Files | Nodes | AST programs | SQL clauses | Reason |
|---|---|---|---|---|---|
| serverplan-state-tripwires | 3,095 | 11.77M | 43 | 28 dedupe + 4 multi-stage (25–27 stages, data flow) | batch six; category H |
| serverplan-steps | 264 | 2.34M | 36 | 7 (16–31 stages each) | largest rule: 1.2 MB, 5,426 operators |
| serverplan-quality | 9,010 | 28.23M | 31 | 6, incl. `quality-contracts-1` (54 stages), `quality-conditional-1` (20 inputs, whole repo) | caused DLRE's 889 producer-arena errors; copies whole tree |
| internal-dead-code | 3,110 | 11.79M | 13 | 1 (13 inputs, 25 stages, 6 recursive) | cross-file reachability |
| public-api-triples | 3,110 | 11.79M | 11 | 1 (22 stages) | DLRE failure "relational query interrupted" |
| async-sync-delegation | 3,110 | 11.79M | 13 | 0 | AST-only, most operators (5,940), 174 shared utilities |
| transaction-patterns | 3,095 | 11.77M | 58 | 0 | most AST programs, one shared root utility |
| async-sync-boundary | 2,919 | 12.54M | 63 | 0 | AST-only, largest programs × nodes (790M) |
- Risk notes only: sync-async-signatures (hit DLRE work budget on `generated_facades.py`, 206k nodes, biggest file); grammar-callable-constructors (failed name closure) (HRP:53-55).

### Corpus facts (M) (HRP:59-73)
- 9,010 `.py` files, 137.9 MB; 11 Rust inputs excluded.
- Syntax-tree nodes **28,230,057** (18.5M named); mean depth 7.98.
- Nodes/file p50 1,333 · p90 7,056 · p99 27,996 · max 206,043.
- Parsing **1,232 ns/node** = 34.8 s CPU, 7.0 s wall on 8 threads.
- Kinds: identifier 6.02M · call 1.06M · attribute 872k · string 781k · assignment 467k · function 162k · class 32.7k.
- Subtrees of all functions sum to 27.0M nodes → one naive "function has X anywhere inside" ≈ one full pass over corpus.
- 60 rules use only **33 distinct file scopes**; one scope serves 13 rules, another 10. 588 AST programs (lower bound) cover **6.33 billion program-nodes**.

### Fact tables (M) (HRP:77-91)
| Table | Rows |
|---|---|
| Definitions | 194,281 |
| Calls | 993,650 |
| Imports | 226,642 |
| Class bases | 22,061 |
| Assignments | 411,275 |
| `__all__` names | 16,740 |
| Name references | 6.02M occurrences; 1.18M after per-file dedupe |
| Distinct identifier names | 232k |
- Extracting all: **54.8 ns/node** (1.55 s one thread). Data-flow facts 41.4 ns/node.

### SQLite at real row counts (M) (HRP:95-111)
| Operation | Time |
|---|---|
| Insert 7.9M fact rows | 5.86 s (0.74 µs/row) |
| Remove duplicate references | 3.79 s |
| Build indexes | 6.72 s |
| **Total load as real tables** | **16.4 s** |
| Resolve calls→defs by unique name | 1.08 s |
| Resolve calls via local defs + imports | 0.73 s |
| Same as join on CASE expression | **24.8 s** (34× slower) |
| Resolve imports | 0.43–0.68 s |
| Reachability over proper edge table | **0.80 s** (175k reachable) |
| Same with correlated subquery | **17.8 s** (22× slower) |
| Dead-code anti-join | 0.25 s |
| Class hierarchy + transitive closure | 0.28 s |
| Count per file and callee | 0.89 s |
- DLRE path-string enumeration of call-graph sinks: 5,853 rows vs 5,687 with plain sets; both 5 ms → no path explosion on this corpus.

### Data flow (category H), taint shape of `state-flow-1`, 11.77M-node scope (M) (HRP:113-123)
| Measure | Value |
|---|---|
| SQLite load + indexes | 3.06 s + 7.59 s |
| SQLite recursive query | **0.49 s** |
| Native worklist (unoptimized) | 0.94 s |
| Tainted nodes | 51,626 in both, same set |
| Rows DLRE ancestor table would hold | **96.9M (8.2× nodes)** |
- "For data flow, loading the facts costs far more than the fixpoint itself."

### Regex (M) (HRP:125-132)
- 376 distinct identifier regexes across 60 rules (state-tripwires: 559 uses, 63 distinct).
- Once per distinct name: 2.1 s one at a time; 1.17 s as one combined set.
- Once per occurrence: 25 ns per regex per occurrence; ~56 s for all 376.
- Whole-file text regexes (111 clauses): ~0.1 s each, mostly I/O.

### Unit costs from elsewhere (D) or assumed (A) (HRP:134-143)
| Unit | Cost | Source |
|---|---|---|
| Fused programs, simple templates | 0.3–0.97 ns per program-node | fusion.md (D) |
| Masks | 6.4–17.3 ns/node for 210 programs | fusion.md (D) |
| Kind index | 12.4 ns/node | fusion.md (D) |
| Metal waves | 19–22 µs each GPU-encoded; ~70 µs pre-encoded with real work | metal-enqueue run (D) |
| Metal set operators | 0.005–10.6 ms on 5.4M nodes | metal-recursion.md (D) |
| **Real heavy-rule programs** | **0.3 / 1 / 3 ns per program-node** | **A**: fusion tests used 3–6-operator templates; serverplan-steps averages ~150 operators/program |

### Per-rule profiles (HRP:145-307)
Reading convention: CPU = interpreter/JIT one thread; Metal = GPU for AST part only, SQL/data-flow stay on CPU; SQLite = AST matching as pure SQL, assumed 3–10× slower than CPU (D: SSC frame engine ~3× slower than native); gate-shared costs counted in §4 (HRP:147-151).

**3.1 serverplan-state-tripwires** (HRP:153-210)
- (a) 28 dedupe clauses matching calls/functions/attributes/classes by name, restricted to `state-project-source` (86 uses), capture line, scope, terminal, dedupe per file. `state-signatures-1`: 18 stages with reachability. `state-flow-1` (92 KB): inputs DLRE node table, ancestor table, ownership; fact stages (functions, attributes, calls, arguments, keywords, assignments, parameters); class contexts recursive; recursive data flow `taints`, `semantic_values`, `return_kinds`; in-file call graph → `reachable` → `transitive_sinks` from path strings; signed exceptions allowlist; option switching per-file vs whole-repo partitions. `state-flow-project-1`: 27 stages, whole-repo variant.
- (b) Lowering: 28 dedupe → one fused program group per root kind over the shared scope; `state-project-source` → one ancestor mask bit; name regexes → bits looked up per distinct name; dedupe free because findings are a set (decision 0001). `state-flow-1`: facts only for in-scope files; ownership by interval ranges, no ancestor table; call graph and `reachable` as SQL recursive queries; sinks as set closure, no path strings; data-flow stages remain category H (native relation or SQL over narrow tables); signed exceptions → anti-join. Order: masks → fused AST groups → fact tables → SQL joins → data-flow fixpoint → consolidation.
- (c) (M) Scope 11.77M nodes, 412k calls, 64k functions. Data-flow facts: 2.64M identifiers, 774k expression edges, 170k assignments, 140k parameters. 51.6k tainted nodes, ~5.7k sink rows. DLRE runs this flow twice, each implying 96.9M ancestor rows.
- (d) Cost:
  | Stage | Low | Exp | High | Notes |
  |---|---|---|---|---|
  | AST matching | 0.15 | 0.5 | 1.5 | Metal 0.02–0.15; SQLite 1.5–15 |
  | Data-flow facts | 0.2 | 0.5 | 0.5 | |
  | Loading data-flow facts | 0.1 | 1 | 10.6 | low = vtab path (A); high = real tables (M) |
  | Data-flow fixpoint (2 variants) | 0.2 | 1 | 3 | |
  | Call graph, reachability, sinks | 0.05 | 0.2 | 1 | |
  | **Total** | **0.7** | **3** | **17** | |
  Memory 0.2 / 0.5 / 1.5 GB.
- (e) No per-rule figure. Batch six (10 rules incl. this) ~583 s pre-SQL (709.2 − 125.8), 1,054–1,278 s SQL path, 54–80 s per edit. Tree + ancestor copy ≈217M rows, "probably hundreds of CPU-seconds" — inference, not measurement. Expected 50–200× faster.
- (f) Risks: rewritten data flow must match oracle exactly; whole-repo mode needs cross-file data flow (unmeasured); path strings→sets allowed only if findings key excludes path; fact loading swings 0.1–10.6 s.

**3.2 serverplan-steps** (HRP:212-228)
- (a) 14 forbid clauses on step structure, 674 regex instances; `step-contracts-1…6` (2 inputs each, 16–31 stages, 3–4 recursive): class-hierarchy and callback closures; `runtime-reference-summaries` (10 inputs).
- (b) One fused group for forbids; contracts → definition, base, decorator, call, keyword, assignment facts, SQL for hierarchy closure and joins.
- (c) 2.34M nodes, 94k calls, 16k functions, 2.4k classes; largest file 148.5k nodes.
- (d) AST 0.03/0.08/0.25; facts+joins 0.05/0.2/1; recursive closures 0.01/0.05/0.3; **total 0.1/0.35/1.6 s**. Metal 0.01–0.05 s. Memory <0.2 GB.
- (e) part of batch six. (f) ~150 operators/program, 10× tested templates; compile time for a 1.2 MB rule unmeasured.

**3.3 serverplan-quality (most uncertain)** (HRP:230-254)
- (a) `quality-conditional-1` whole-repo, 20 inputs copy every node into SQL (category C); builds nodes, edges, ownership/ancestors, conditional class hierarchies, data-flow-like stages `always_raises`, `branch_variables`, `self_stores`, `function_colors`, `integer_returns`. `quality-contracts-1`: 54 stages: scope chains, scope imports, annotations, call paths.
- (b) Node copy → engine node relation read from tape as vtab; scopes/imports → fact tables; hierarchy → closure. **Data-flow-like stages need a new per-function relation contract; does not exist yet.**
- (c) 28.23M nodes, 6.02M identifiers. DLRE copy ≈260M rows; as real SQLite tables ≈990 s at measured rates — behind the 64 MiB arena failures.
- (d) AST 0.26/0.9/2.6; node-relation scans (~3) 0.2/0.6/2; facts+joins 0.3/1/4; data-flow-like 0.5/2/15; **total 1.3/4.5/24 s**. High case = data flow in SQL over 6M copied identifier rows. Memory 0.3–1 GB, or 3 GB if identifiers copied.
- (e) ≥50× faster, bounded memory.
- (f) Rewriting data-flow-like stages under decision 0002 = biggest single job and biggest parity risk. Must show "every other node" input never read in full; if read, one tape scan (~0.2 s) via vtab, fatal if copied.

**3.4 internal-dead-code** (HRP:256-275)
- (a) 13 raw inputs, all fact tables in disguise (functions, classes, decorators, parameters, annotations, imports, bases, `__all__`, modules, dynamic imports). 25 stages, 6 recursive (hierarchy, public methods, reach edges, reachable, high-confidence). Mode `log`.
- (b) facts, then SQL: import resolution → call resolution → edge table → recursive reachability over indexed edge table → hierarchy → anti-join with allowlists.
- (c) whole corpus (scope = 42% of it): 194k defs, 994k calls, 1.18M distinct refs, 352k resolved edges, 175k reachable, 9.4k dead candidates.
- (d) resolution 0.3/0.8/1.5; reachability 0.3/0.8/18 (badly written query); anti-join+hierarchy 0.2/0.5/1; **total 0.8/2.1/20 s**. Real-table loading adds ~7 s for this scope.
- (e) part of first-50 global evaluation: 52.1 s pre-SQL, 83.0 s current.
- (f) Query plans must be held stable (recorded plans, checks catching statistics changes). Recomputing reachability after an edit 0.34 s; no incremental design. Resolver must match oracle's.

**3.5 public-api-triples** (HRP:277-282): sync/async/lazy triples across public API; 11 inputs, 22 stages, 3 recursive. Same facts plus 16.7k `__all__` names; re-exports via imports → recursive SQL closure. (d) 0.5 / 1.5 / 8 s. (f) Reproduce DLRE "interrupted" failure first; if a cycle under `UNION ALL`, set semantics (`UNION`) fixes it.

**3.6 async-sync-delegation** (HRP:284-291): 12 forbid + 1 budget clause on 174 shared utilities (up to 21.6 KB each). Utilities deduped by digest; those used inside relations → mask bits; large ones called with depth bound; budget → count. 11.79M nodes, 64k functions. CPU 0.05/0.15/0.5 s; Metal 0.01–0.1 s. Part of the 58.2 s native AST matching across first 50 rules. (f) [superseded per HRP:5] recursive utility only CPU/JIT; utility compile-time expansion unmeasured; fusion.md saw 974k expansions → 316k after dedupe.

**3.7 transaction-patterns** (HRP:293-297): 58 forbid clauses all rooted at `connection-function` utility → ideal fused group: root set once (64k functions), 58 short checks on survivors; separate total 682M program-nodes largely disappears. (d) 0.05 / 0.2 / 2 s (high = no fusion). Metal 0.01–0.05 s. **Should be first fusion measurement (M2).**

**3.8 async-sync-boundary** (HRP:299-302): 63 forbid with mixed root kinds, fused by root kind; 12.54M nodes; 0.24 / 0.8 / 2.4 s; Metal 0.03–0.2 s.

**Risk notes** (HRP:304-307): sync-async-signatures: `generated_facades.py` (206k nodes) parses in 198 ms, extracts facts 2.5 ms, should need 2–4 ms mask work. DLRE failed on compiler budget of 256 nodes per clause, not data volume; interval/mask lowering shouldn't trip it — still has to be shown, with 1,000-sync-function synthetic (M7). grammar-callable-constructors: about how `Complete`/`Unknown` decided, not speed.

### Whole-gate total, CPU (HRP:309-327)
| Component | Low | Exp | High |
|---|---|---|---|
| Parsing cold (M; 0 warm) | 35 | 35 | 35 |
| Tape encoding + kind index | 0.3 | 1 | 9 |
| Regexes (per-name set + whole-file text) | 1.5 | 3 | 10 |
| AST matching: 588 programs, 6.33G program-nodes (D check: native matcher 58.2 s for 50 rules) | 1.9 | 6.3 | 19 |
| Fact extraction | 0.3 | 1 | 1.6 |
| Loading facts (vtab vs real tables) | 0.5 | 2 | 30 |
| Cross-file SQL | 2 | 5 | 25 |
| Data flow (H) | 1 | 4 | 20 |
| Consolidating findings | 0.1 | 0.3 | 1 |
| **Total without parsing** | ≈8 | ≈25 | ≈110 |
| **Total with parsing** | ≈43 | ≈60 | ≈145 |
- Memory: tape ~0.24 GB, sources 0.14 GB, facts 0.2–0.5 GB, data flow <0.5 GB; total 1–4 GB vs 15–16 GiB today.
- Metal: 0.2–2 s GPU for AST row, ~0.5 GB resident; saves ≤ ~6 CPU-s, ~1 s wall.
- Pure-SQL AST matching: AST row becomes 6–190 s; not the right default for heavy rules.

### One-file edit, warm (HRP:329-339)
| Step | Cost |
|---|---|
| Re-parse | 1.6 ms typical, 198 ms largest |
| Fact extraction | <2.5 ms |
| AST matching | ~2 ms typical, 0.12 s largest |
| Delete + reinsert file's facts | 135 ms |
| Re-resolve file's calls | 436 ms naive; ms with maintained table |
| Recompute global reachability | 342 ms |
| **Total** | 0.02–2 s, ~0.3 s expected (DLRE 40–80 s) |

### Biggest unknowns, largest swing first (HRP:341-349)
1. Facts → SQLite: vtab vs real tables; swing 0.5–30 s; vtab unmeasured.
2. Query-plan stability: wording gave 22× and 34×.
3. Data flow (H) in quality and state-tripwires: cost small (0.5–0.9 s per variant); unknowns are expressibility under decision 0002 and parity.
4. Real heavy program cost: 3–6 operators tested vs ~150; 6× swing on AST row.
5. Whether every-node inputs in `quality-conditional-1` reduce to tape scans.
6. Compile time for 1.2 MB rules and 174 utilities.
7. Cross-file data flow in whole-repo mode (only in-file measured).

### Minimal measurements before any merge (HRP:351-364)
| # | Measurement | Pass condition |
|---|---|---|
| M1 | each of 8 rules alone, existing native matcher, CPU + memory | baseline |
| M2 | fuse transaction-patterns and async-sync-delegation with interned regex + masks, findings match | AST ≤ 1 ns per program-node |
| M3 | facts via vtab; rerun SQLite queries (B1–B4) + taint query | loading + joins ≤ 5 s for gate |
| M4 | re-express internal-dead-code and public-api-triples over facts, after reproducing "interrupted" | same findings, recorded plans |
| M5 | `state-flow-1` data flow as native relation | matches oracle on all 1,259 observations, ≤ 2 s |
| M6 | count rows consumed per `syntax_*` input of `quality-conditional-1` | no input copies every node |
| M7 | `generated_facades.py` + 1,000-sync-function case under mask lowering | no budget failure |
| M8 | one-file change through facts, resolution, reachability | ≤ 1 s p95 |
- M1, M2, M3, M7 cheap and settle most of the range; M4, M5 carry parity risk.
- Status of these in other sources: M1 done (M1 file). M2–M8: not found as completed in these sources (SF mentions "tracked against M3" at SF:93, SF:141, SF:181).

---

## 2. research/fusion-model.md (FM)

### Purpose and method
- Fusion opportunity and performance model on data-layer rules (FM:1).
- Gate corpus, 9,010 Python files, 28.2M tree-sitter nodes, single-threaded, shared machine; **identical runs differed up to 2.2×** (FM:3, FM:9, FM:294).
- Code in `../experiments/fusion-model/`; reproduce commands (FM:43-58). Sample A = every 18th file; sample B = every 18th file from offset 9 (FM:57). `out/v1/` earlier full run lacking newer counters (FM:58).
- Superseded: "Recursive set stage (CPU only)" layer in §1 predates metal-enqueue; under spec §8 monotone recursion runs on Metal as GPU waves, no CPU fallback (FM:5).

### Harness (FM:60-80)
- `analyze.py` sets each `$option` to default (`oracleAnalyzer=false`, `sourceGate=true`), drops clauses whose `when.option` false.
- `crate/` compiles all queries into one hash-consed graph; recursive `matches` become back-edges.
- Exact ast-grep semantics for `kind`, `regex`, `value`, `filePath`; `pattern` with metavariables, `$$$`, `context`/`selector`; `nthChild` with `ofRule`, `reverse`; `all`/`any`/`not`; four relations with `stopBy` neighbor/end/rule plus `field`; recursive `matches`.
- Approximation: fact keys evaluate true → findings are superset.
- All modes must produce same findings: asserted **8,650,988 matches** on full corpus.
- Modes: `sep` (separate anchored programs per query); `fused` (one root-kind dispatch per node shared by all); `memo` (fused + per-file memo columns for shared rule-graph nodes); `mask` (memo + eager masks for range-scanning relations); `maskall` (masks for every relation).

### Static analysis (FM:11-16, FM:82-141)
- Clauses: 496 forbid (112 regex over file text), 52 relational (in 10 rules), 44 define, 14 budget. 588 AST queries, 535 active (FM:85-86).
- Summary also says: 535 active AST queries = 330 forbid, 14 budget, 191 inputs to relational clauses; and **113** regex clauses over whole-file text (FM:12). [Note: 112 at FM:85 vs 113 at FM:12; HRP:132 says 111.]
- One shared rule graph: 279,204 expanded rule nodes → 6,821 operations (41×) (FM:13, FM:167).
- Nesting 14 levels; 137 queries depend on recursive utilities; rule scopes fall into 42 distinct groups (FM:14, FM:109).
- Root kinds: `call` 166, `function_definition` 94, `assignment` 50, `identifier` 50, `module` 41, `string` 30, `concatenated_string` 28, `attribute` 20, `import_from_statement` 16, `class_definition` 15; 64 groups; 10 queries have no root-kind constraint (FM:88-90).
- Max root-kind group 166 (`call`): no group reaches K=512 where earlier bench saw 2.3–2.5× (FM:15).
- Relation sites after utility expansion: `has` neighbor 16,537 (~60% with `field`); `inside` end 1,256; `inside` neighbor 1,198; `inside` stopBy rule 640; `has` end 309; sibling range 215; `has` stopBy rule 53 (FM:92-102).
- Boolean ops: `all` 22,203; `any` 8,241; `not` 2,614. Atoms: `kind` 24.7k, `regex` 9.4k, `pattern` 4.4k, `nthChild` 1.2k (FM:105-106).
- Utility bodies 278; references to recursive utilities 4,710; 137 queries depend on recursion (FM:107).
- Distinct query digests 497; **distinct regexes 816**; only 20 patterns structural, rest `$X` captures (FM:108).
- Scopes: 42 distinct for 60 rules; largest identical-scope groups 6, 6, 4, 4 rules (FM:109).
- Queries by layer (FM:113-118):
  | Layer | Rule queries | Relational inputs |
  |---|---|---|
  | Instruction program | 46 | 62 |
  | Set stage | 129 | 117 |
  | Recursive set stage (CPU only — superseded) | 101 | 7 |
  | Fact family | 68 | 5 |
- Fact-family keys per query count: `compare` 34, `layout` 26, `parse` 17, `count` 6, `binds` 4, `arguments` 4, `host` 3, `calls` 1 (FM:120).
- Rules by heaviest layer: fact family 20, SQL composition 10, set stage 9, recursive set stage 9, regex only 9, instruction program only 3 (FM:122-131).
- 52 relational clauses under decision 0002 (FM:16, FM:133-141): 29 per-file join (join on captures in one program, or SQL join over exported rows); 11 per-file `EXISTS`/`NOT EXISTS` (one fused program or `select_when`); 7 use `syntax` node table (engine relation + SQL); 4 cross-file (`require_each`/`select_when`); 1 reference graph (fact family). 47 per-file.

### Corpus statistics (FM:143-162)
- 28.23M nodes (18.5M named); 1,066 distinct (parent, field, child) edges; depth mean 8.0, max 86; nodes/file p50 1,333, p90 7,056, p99 28k, max 206k.
- Root kinds: identifier 6.0M (subtree 1); call 1.06M (mean subtree 17.8; p50/p90/p99 11/35/106; max 11k); string 781k (4.5); assignment 467k (21); function_definition 162k (167; 88/365/1,157); class_definition 32.7k (346; p99 3.9k); module 9,010 (3,133; max 206k).
- Heaviest candidate set: `quality-conditional-1`, whose unconstrained root visits all 28.2M nodes.

### Fusion opportunity (FM:164-203)
- Sharing: 279,204 → 6,821 (41×). Of 2,014 relations, 229 shared, 341 depend on recursion. Relation targets 2,185 distinct; 1,029 used by ≥2 queries.
- Root-kind groups: 9 groups with K ≥ 16 covering 495 query–root pairs; none at K=512; groups of 128–166 in earlier bench's 1.3–1.5× band.
- Queries per stratum 0..8+: 49, 99, 163, 58, 43, 61, 23, 16, 23.
- Mask targets per stratum (non-recursive) 1..7–14: 789, 373, 193, 112, 67, 42, 99.
- Mask candidates: 380 range-scanning relations; 1,673 including neighbor relations.
- Measured speed-up over `sep` (M) (FM:186-192):
  | Corpus | fused | memo | mask | maskall |
  |---|---|---|---|---|
  | A | 0.90–0.97× | 1.9–2.1× | 1.9–2.1× | 0.74–0.91× |
  | B | 1.06–1.24× | 2.3–2.6× | 2.5–3.3× | 1.4–1.5× |
  | Full (run1/run2) | — | 3.1× / 1.4× | 2.7× / 2.2× | — |
  Full corpus: `sep` 171–173 s; `memo` 55.6 s and 119.9 s; `mask` 63.3 s and 78.6 s.
- Summary statements (FM:17-21): sharing evaluated sub-rules 1.4–3.1×, almost all from not re-running regexes over large nodes: **regex text scanned 2.59 TB → 3.1 GB**. Root-kind-dispatch-only 0.9–1.2× ("nothing"). Eager masks for every relation slower (0.74–1.5×); masks only for range-scanning relations break even with plain sharing (1.9–3.3× overall).
- Explanations (FM:194-203): in `sep`, regexes on module/function bodies reached via `inside … stopBy end` from every candidate; memo columns cut scan to 3.1 GB; evaluations 4.98G → 3.19G; "lazy per-node sets are enough". Dispatch-only fusion buys nothing because candidates are already selective; cost sits in relations. maskall builds 1.13G column cells vs memo ~0.1G lookups. Confirms `research/fusion.md` rule: **mask only when summed scan cost over roots exceeds nodes × cost of a pass** — holds for a few hundred scanning targets, never for neighbor relations.
- Time concentrated: 56 queries = 80% of `sep` time; top ten are relational inputs: `quality-conditional-1` 14.5 s; `step-contracts-1/3/5/6` 4–8.6 s each; `legacy-execution-graphs` 3.5 s. Rewriting them ≈30% of structural work (FM:19, FM:198-203).

### Performance model (FM:205-260)
Level 1 per-operation NNLS fit, 23 counters, every mode of sample A (FM:209). Fitted unit costs (FM:213-224):
| Item | Cost |
|---|---|
| Per file | 12.7 µs |
| Candidate scan | 11.3 ns/node |
| Dispatch | 13.2 ns per (node, query) |
| Range scan visit | 6.4 ns |
| Mask cell | 7.4 ns |
| Mask lookup | 5.9 ns |
| Memo hit | 94 ns (absorbs column allocation) |
| `not` | 273 ns |
| Pattern match | 1.14 µs |
| `has` end | 160 ns |
- Harness averages **35 ns per evaluation** (FM:226).
- Calibration error (FM:228-236): sample A (training) totals +0.7% to −5.5%, median per-file error 58–91%; sample B (held out) −34% to +46%; full corpus −50% to −17%. Totals within ~1.5×; per-file not predicted (huge files and contention dominate).
- Summary rephrasing: per-op model within 6% on fit data; held-out −50% to +46% (FM:22-23).
Level 2 planner estimate: query time = candidates × κ (FM:238-247):
| Variant | κ source | Median per-query error A→B / A→full | Total error A→B / A→full |
|---|---|---|---|
| Feedback | measured on A | 1.13× / 1.40× | −31% / −36% |
| Cold static | rule structure only | 3.1× / 3.0× | −31% / −38% |
- Verdict: plans need measured κ from `plan_feedback`; static estimates only good enough for first-run grouping (FM:24-25, FM:247).

Stage costs, full gate, one core (FM:249-260):
| Stage | Full gate |
|---|---|
| Parse cold | 34–60 s (measured 38–50 s) |
| Restore cached trees | 0.10–0.22 s |
| Kind postings | 0.23–0.45 s |
| Structural `sep` | 129–232 s |
| Structural fused | 56–120 s measured |
| Regex over file text (4.3 GB) | 9–17 s |
| Relational SQL (8.5M rows upper bound × 1–5 µs) | ≤ 8.5–43 s |
| Consolidation (~25k recorded findings) | < 0.5 s |

### Predictions (FM:26-37, FM:262-285)
| Configuration | Full gate |
|---|---|
| `sep`, 1 core | 147–293 s (native baseline 125.8 s) |
| Fused, 1 core | 77–167 s |
| Fused, 4 workers | 19–52 s, +9–14 s if trees must be parsed |
| Fused, 16 workers | 5–17 s |
| Faster executor (3–5 ns/eval), 16 cores, cached trees | **1.2–3.5 s** |
| Metal (not run) | 10–86 s (kernels 1–41 s; regex stays CPU) |
| Pure SQL lowering (3.4G rows × 100–500 ns) | **360–1,760 s** (DLRE measured 958.6 s) |
- DLRE 958.6 s falls in SQL-lowering range; native baseline 125.8 s matches `sep`.
- **1–2 s target needs all of**: cached trees (cold parse alone 38–50 s CPU), shared evaluation, executor 7–10× faster than harness, 16-way parallelism, cheaper regex stage, ten heavy relational queries rewritten (FM:37, FM:276).
- Per edit (one file, all 535 queries, one core, parse included, cached plan) (FM:278-285): `sep` p50 3.7 ms, p90 32 ms, p99 358 ms, worst (206k nodes) 5.6 s; `mask` 2.9 / 22 / 189 ms / 1.3 s. Cold plan compile 5–6 s → plan must be cached.

### Not modelled / uncertain (FM:39, FM:287-295)
- Fact families: **73** queries evaluate true at zero cost (FM:289) [summary FM:39 says 68; layer table FM:118 gives 68 rule queries + 5 relational inputs = 73].
- Relational SQL row counts upper bound; per-row cost assumed.
- Regex line state machine not run.
- 42 scope groups not modelled; fused runs filtered scope per query, flattering fusion.
- Patterns match named children only.
- Metal figures derived from `research/metal-recursion.md` kernel costs, not run.
- Parallel scaling assumed, not measured.

---

## 3. research/heavy-inputs.md (HI)

### Purpose and method
- Rewriting heavy relational inputs and lowering them to the AST/IR (HI:1). Scripts in `../experiments/heavy-inputs/` (HI:3, HI:256-263).
- **Nothing implemented or run end to end.** Before figures measured (fusion harness `sep` seconds + exported rows from `inputs.json`); after figures are estimates from measured unit costs; every clause classified by reading its SQL (HI:5, HI:45).
- After = root counts × κ, κ = measured per-candidate cost of ordinary forbid queries with that root kind, p50–p90: call 134–468 ns; function 0.9–5.4 µs; class 1–8.8 µs; module 8.6–17.5 µs. Set-stage passes 6.4–17.3 ns/node. Script `estimate_after.py` (HI:45).
- Caveats (HI:248-254): before = single-core `sep` in harness treating fact keys true, runs varied up to 2.2×; κ from ordinary forbid queries, root counts are upper bounds over whole rule scope; 28 dedupe savings overlap memo fusion; quality-contracts-1/2/3, state-signatures-1, runtime-reference-summaries read less closely; **no findings were compared**; design analysis only.

### Headline (HI:7-17)
- "Top ten ≈ 30%" undercounts: all **191 relational-clause inputs take 146.6 s of the 171.5 s `sep` total (~85%)**; six clause groups = 139.6 s.
- Almost none is rule logic: (1) whole-tree copy into SQL rows; (2) AST queries whose only SQL step is "one finding per line"; (3) string literals fed into SQL to compare with a constant.
- Rewritten as transducer programs, six groups est. **0.40–1.29 s one core** vs 139.6 s.
- Exported rows ~6.7M rows / 163M cells → essentially zero; only finding keys plus one shared fact family (22,061 class-base rows).
- Of 52 clauses, **45 run entirely as IR programs** (one reading a per-document fact column); 3 more IR + small cross-file SQL tail; only 4 genuinely cross-file.
- Needs three IR extensions E15–E17 beyond E1–E14, plus one spec reversal: spec §7 and spec-fusion §6.6 say capture equality across matches stays in SQL — should apply only across documents; within one document lower to the IR.

### Heavy input shapes (HI:19-28)
| Shape | Clauses | `sep` time | Rows exported |
|---|---|---|---|
| C: syntax tree copied into rows (`syntax_*`/`raw_nodes`, every node, up to 17 field captures; SQL rebuilds nodes, edges, child order via `row_number`, ownership recursive CTE, full descendant closure) | quality-conditional-1, step-contracts-1..6, legacy-execution-graphs, executor-provenance | 64.7 s | 6.41M rows / 162.6M cells |
| A: dedupe only (SQL keeps one row per (path, line, scope, terminal)) | 28 state-tripwire clauses | 54.9 s | 0 (no violations) |
| Literal feeders (`value IN (...)`) | `raw_strings_*` in state-flow-1, state-flow-project-1, state-projection-recovery-2 | 20.0 s | 325,944 |
| Fact tables in disguise | internal-dead-code, public-api-triples, required-contracts, authority-sites, runtime-reference-summaries, quality-contracts-1 | ≈5.9 s | ≈1.7M |

Why written that way (HI:30-41): DLRE generators `scripts/dsl/data_layer_six/`:
- Exact oracle parity bar "Model the exact state tracked by the oracle" (`docs/data-layer-rule-port.md`) → generator reproduced the tree in SQL (category C).
- Sharding to fit arena: `typed_input_partitions()` in `ast_relations.py` splits the every-node input into 20–27 typed shards (identifiers by first letter `^[a-f]`, `^[g-m]`…; strings by prefix; one per kind group; catch-all) to stay under per-input witness bound and **64 MiB producer arena**. Batch-six table: "A large step file exceeded its witness bound | Split independent typed inputs".
- quality-conditional-1 could not use native syntax table: performance journal ~line 3210: selection includes module declarations → whole-file gate admitted every file and hit **32 MB source guard**; kept projected AST inputs and SQLite stages; `syntax_18` unconstrained root takes 14.5 s alone.
- Category A exists only to reproduce oracle's one-finding-per-line identity. Literal feeders and allowlists (category I) exist because rules could not test a string value or exception list in place.

### Clause by clause (HI:43-201)
**2.1 quality-conditional-1** (whole repo): **29.34 s → 4–14 ms** (HI:47-89)
- Inputs: 20, each `<kind shard> AND NOT leaf-kind AND matches quality-conditional-1-selection` (large `any`); export node + parent, ancestor, up to 14 field captures; 2,093,271 rows / 69.2M cells.
- SQL (32 stages): union shards (`syntax_rows_0/1`), rebuild `node_ordinals`, `nodes`, `edges`, `ownership` (recursive), `descendants` (closure over demanded forest, up to ~16.7M rows); builds functions, classes, calls, attributes, returns, assignments, parameters, aliases; module names from paths via regex UDFs; `conditional_classes` latest class per name; `class_bases` via import aliases; `derived_conditionals` recursive subclass closure of `ConditionalStep`; `function_colors`, `block_results` (JSON fold "always raises TypeError"), `integer_returns`, `branch_variables`, `self_stores`, `callable_parameters`; 17 `SPQ-029` detectors.
- Actual checks: 13 detectors on 4 named classes/functions in 4 known files (76,417 nodes, 77 classes, 411 functions), e.g. `ConditionalStep.compile` must raise `TypeError`; `level` returns integer > 0; `SqlConditionalStep.execution_role` returns `StepExecutionRole.SQL`; `_lower_conditional_step` calls `SqlConditionalStep.from_semantic`. 4 "missing" detectors. 5 detectors on subclasses of `ConditionalStep` anywhere (runtime-state, runtime-callback, declared-callback, child-compilation, backend-control); corpus has **3** such classes (measured on `facts.sqlite`).
- Rewrite: (a) 13 anchored detectors: one program each gated by `filePath`, rooted at class/function def with name test: `NOT_EXISTS` method `compile` with `EXISTS raise TypeError`; `COUNT` of integer returns in `level` ≠ 1 (E4); `EXISTS` return text `StepExecutionRole.SQL`; `EXISTS` call matching `compile_sql_conditional_step$`. "Always raises": bottom-up attribute over blocks with negation (block raises if a statement before first return/raise is `raise TypeError…`, or an `if` all of whose branches raise) — needs **E17**. child-compilation: `$X.compile()` where `$X` bound earlier in same function from `then_plan.steps…` or `for $X in …`: `EXISTS` from enclosing function with `TEXT_EQ $X` (allowed by spec-fusion §2.3, sub-runs read outer captures) plus "earlier than" (**E16**). runtime-callback: `self.x = p` where `p` is `Callable`-annotated `__init__` param: `TEXT_EQ`. compiler-contract: annotation resolved through in-file import alias: `EXISTS` import with `TEXT_EQ` alias. (b) `fact_class_base(doc, class, base_name)` 22,061 rows shared with internal-dead-code and public-api-triples. (c) recursive closure from `ConditionalStep` gives 3 names, fed back as seeded key set (E7 → **E15** `IN_KEYS`) to the 5 subclass detectors → 32,728 classes each get one cheap key test. (c) "missing" guard: `require_each` over 4 `t_match` rows. Dropped: "latest definition wins" and module-name regexes (replaced by `filePath`).
- After: 4–14 ms; rule-specific exports = 3 names + findings.

**2.2 step-contracts-1..6** (serverplan-steps, 264 files, 2.34M nodes): **26.08 s → 0.08–0.27 s for all six** (HI:91-129)
- One `raw_nodes` input each: every node inside a strict module matching a large `any` demand filter; 33–45 columns; 551,463 rows / 21.3M cells over six. step-contracts-6 exports only 563 rows but costs 8.6 s because demand filter incl. `inside step-strict-module stopBy end` evaluated on all 2.34M nodes.
- SQL: rebuilds tree incl. `descendants` full `WITH RECURSIVE` (~4.4M rows over six); `step_classes` (base text matches `ServerPlanStep`; 268 such classes in 99 files), `first_methods`, `method_returns`, `levels`, `roles`, `empty_tables` (`frozenset()` no args), `compiled` (keyword flags on `CompiledStep(...)`), `compile_aliases`, `local_bindings`. Recursive: `column_derived` (flow from `cte_output_columns` via local names), `user_callable` (lambda/callable identifiers up attribute/call/argument edges), `raw_shapers` (iterates raw param or calls a shaper; intra-file call graph by name), `payload_values`. Findings STEP-016, 018, 019, 023–026, 028, 033–036, 039, 040, 045–050.
- Rewrite all (a): every check is "class with ServerPlanStep base, method named N, inspect body". Example YAML for STEP-018 (raise TypeError/NotImplementedError directly in compile() of a level-0 step) with `step-class` and `level0-step-class` utils (HI:109-122).
- Within-file capture joins: `TEXT_EQ` inside `EXISTS` when one side reached by navigation; **E15** keyed sets when both sides free (e.g. keyword `post_op=<name>` vs function of that name STEP-026/028; `self._x` aliases STEP-019; materializer class name vs identifier STEP-039).
- Recursion through names (`raw_shapers`, `column_derived`) → monotone FIX over E15 key set. Recursion along fields (`user_callable`, `payload_values`) → ordinary E14 FIX in has-direction. "First definition wins" dropped under loose contract, or `NOT_EXISTS` earlier sibling with `TEXT_EQ` + E16.
- After: 0 exports; `t_match` = findings. 2,386 class roots, 94k call roots, 16k function roots + three set passes over 2.34M nodes → 0.08–0.27 s; matches HRP's independent 0.08 s.

**2.3 legacy-execution-graphs** (702 files, 4.31M nodes): **7.99 s → 0.10–0.37 s** (HI:131-153)
- 25 typed syntax shards (`syntax_24` catch-all alone 3.45 s, 1.64M rows) + `from_imports`, `direct_imports`; 3.75M rows / 71.4M cells.
- SQL: `function_symbols` qualname by interval containment + `group_concat`; `decorators`, `valid_boundary`; `receiver_tokens` recursive (lowercased identifier tokens up attribute/call/`cast`/subscript edges); `call_candidates` → `resolved_calls` (`HAVING count(*)=1`) → `eager_reach` (recursive callers of functions calling `resolve`); `eager_anchors` (first call per function); 22 detectors LEG-010…LEG-085, mostly "call to X under path prefix P minus path allowlist".
- Rewrite all (a): path tests/allowlists → `filePath`; `receiver_tokens` → E14 FIX utility; `eager_reach` → E15 keyed FIX over function names in document (drop unique-name requirement under loose contract, else `COUNT` on key); LEG-085 `TEXT_EQ` + E16; LEG-021 `X_lazy` from `X_async` needs E5 `transform` then `TEXT_EQ`; qualname allowlist (3 entries) → `inside` chains.
- After: 0 exports, 0.10–0.37 s; two set passes over 4.3M nodes dominate.

**2.4 28 dedupe-only state-tripwire clauses**: **54.94 s → 0.08–0.25 s** (HI:155-170)
- Each single `events` input: `inside state-project-source (stopBy: end) AND filePath … AND <call/attribute shape>`; `state-project-source` is a module-wide regex. SQL final only `row_number() OVER (PARTITION BY path,line,scope,terminal)=1`. 0 rows exported. 2–3 s per clause all matching: in `sep`, each of 412k calls walks to its module and re-runs module regex = the 2.59 TB rescan.
- Rewrite: plain `forbid` with `key: [line, scope, terminal]`. New **lowering rule L1, document-invariant hoisting**: `filePath` conjuncts and `inside {module + regex} stopBy end` evaluated once per file as gate. Option residue such as `filePath: '\b\B'` (never) and `'(?s).*'` (always) constant-folded.
- After: 3,095 module-gate evaluations + one fused call dispatch → 0.08–0.25 s. Overlaps memo-mode fusion.

**2.5 String feeders** (state-flow-1, state-flow-project-1, state-projection-recovery-2): **20.01 s → ~0.05 s** (HI:172-184)
- `raw_strings_0/1/2`: non-interpolated strings in project-source scope, decoded via `transform`, sharded by prefix; 325,944 rows; only consumer `string_values` tests `s.value IN ('_conn', 'materialized_pipeline', …)` on `getattr`/`.get`/subscript keys; also folds module-level constants.
- Rewrite: key test into `getattr`/`get`/subscript sub-program as value DFA on decoded literal; constant folding → `EXISTS` from module with assignment whose LHS `TEXT_EQ $NAME` and RHS matching literal; 0 rows exported.
- Remaining: category-H core of state-flow (`taints`, `semantic_values`, `call_graph`, `reachable`, `transitive_sinks`); §3 shows it fits E15 + FIX within a file.

**2.6 executor-provenance** (450 files): **1.26 s → 0.09–0.32 s** (HI:186-190): 25 typed shards, 24,598 rows / 700k cells; SQL: executor types (recursive over annotations), name bindings, owners (recursive over attribute, subscript, binding edges). Rewrite (a): E14 FIX over field edges + E15 keyed FIX over name bindings scoped to functions. 0 exports.

**2.7 Smaller clauses with large exports** (HI:192-201)
| Clause | Before | Rewrite |
|---|---|---|
| internal-dead-code | 13 inputs, 554k rows, 1.82 s; 25 stages incl. import resolution, hierarchy, reachability | (b)+(c): `fact_def`, `fact_import`, `fact_ref`, `fact_class_base` (194k / 227k / 1.18M / 22k rows, shared). SQL reachability 0.80 s on proper edge table |
| public-api-triples `exported-families` | 11 inputs, 551k rows, 1.15 s | (b)+(c): per-symbol traits (decorators, suffixes) as IR exports; family grouping across modules via re-export closure in SQL |
| required-contracts | 10 inputs, 103k rows, 0.89 s; anti-joins JSON lists of required (path, name) | (a)+(c): one `NOT_EXISTS` program per requirement gated by `filePath`; missing file needs 1-row SQL existence check |
| authority-sites | 5 inputs, 166k rows, 0.89 s; qualnames for 3 allowlists | (a): AUTH001–003 as `forbid`; allowlist qualnames → `not inside` chains, or E8 SIDE qualname |
| runtime-reference-summaries | 10 inputs, 266k rows, 0.40 s; recursive `consumers`/`refless` | (a): E15 scoped keyed FIX |
| quality-contracts-1 | 7 inputs + native syntax, 0.78 s, 54 stages | (a)+F: IR programs reading per-document `fact_scope`/`fact_binding` column (E8 `SIDE_EQ`) |

### IR extensions (HI:203-235)
Already in E1–E14 and used: E1 set mode; E2 `EXISTS`/`NOT_EXISTS` with sub-runs reading outer captures (enables navigational joins); E3 axes; E4 `COUNT`; E5 patterns and transforms; E6 utilities; E7 seeded starts and keys from SQL; E8 `SIDE`; E12 lazy enumeration; E14 set stage with FIX; `TEXT_EQ` (spec-fusion §8).

New (proposed):
- **E15 Keyed sets**: set-stage definition `KEYS(S, key = text(x) or a capture, scope = document or nearest ancestor of kind K)` producing per-document sorted (scope, hash) pairs, plus `IN_KEYS k [negate]`. Admissible inside a monotone FIX. Metal: sort/dedupe + binary search, same machinery as postings. Needed by every within-document capture join where neither side reaches the other by navigation; recursion through names (`raw_shapers`, `eager_reach`, `bad_configs`, executor owners, local value flow, state-flow taint); SQL-seeded name sets (E7).
- **E16 Order against a capture**: `ORDER cmp, capture` comparing preorder (or byte) position with a captured node. For "assigned before use", "earliest anchor", `strategy.line <= call.line`, alias imported earlier.
- **E17 Bottom-up fold**: `FOLD_UP` set `S = {x : φ(x, S ∩ children(x))}`; φ may negate because recursion strictly into subtree → well-founded. One reverse-preorder pass on CPU; deepest-first depth waves on Metal as for has-direction FIX. Requires amending spec §6.8 / spec-fusion §6.8, which reject any negation through recursion. For quality-conditional-1 "always raises" and similar "all branches".

Lowering rules (no new opcodes): **L1** hoist document-invariant conjuncts to per-document gate, constant-fold option residue; **L2** drop `row_number` pick-first and "latest definition" selections (findings contract); **L3** compile allowlist literals into constant tests or `inside` chains.

Spec change: spec §7 "Capture equality across matches | stays in SQL" and spec-fusion §6.6 → within one document lower to `EXISTS`+`TEXT_EQ` when one side reached by navigation, else E15. SQL hash joins remain for cross-document joins only.

Risks: quadratic sub-runs (per-candidate `EXISTS` scanning whole function body) — planner must choose E15 set form by cost (spec-fusion §4.8). Category H stays a parity risk (state-flow, runtime-reference-summaries, executor-provenance), though it fits the IR as monotone keyed FIX.

### All 52 relational clauses (HI:237-246)
| Target | Clauses | Input `sep` time | Which |
|---|---|---|---|
| (a) fully in IR | **44** | 95.7 s | 28 dedupe, step-contracts-1..6, legacy-execution-graphs, executor-provenance, authority-sites, direct-execution-lines, quality-contracts-2 and -3, quality-local-taxonomy, state-signatures-1, state-projection-recovery-2, runtime-reference-summaries |
| (a) + per-document fact column | 1 | 0.8 s | quality-contracts-1 |
| (a) + small cross-file SQL tail | 3 | 38.0 s | quality-conditional-1 (class-base closure + guard), required-contracts (missing files), state-flow-1 (cross-file return-kind summaries; currently baked as `returnKindsHex` option) |
| (b)+(c) genuinely cross-file | 4 | 12.1 s | internal-dead-code, exported-families, state-flow-project-1, quality-project-taxonomy (repository count ≥ 2, i.e. a `budget`) |
- **48 of 52 have all per-document work in the IR**, vs fusion model's 11 `EXISTS` + 29 "join" split. The 7 native-syntax users and all 29 per-file joins lower to IR once E15 exists. Remaining SQL: fact-family joins, two cross-file closures, a re-export closure, one repository count, three small tails.

---

## 4. research/m1-baseline.md (M1)

### Purpose and method
- M1 measurement: per-rule baseline of the 8 heavy rules on the current engine (M1:1). Scripts `../experiments/m1/`; recordings not checked in.
- Stopped early by direction; per-rule table complete (all 8 ran twice); stage breakdown complete only for serverplan-steps. 60-rule run skipped: serverplan-quality alone needs 10 GiB and ~350–410 s CPU; DLRE first-50 run took 1,990 s wall before all-60 was cancelled (M1:3).
- Timing addon (runs -1, -2): Pulling-it-all-together build `native/rerail_engine.darwin-arm64.node` (sha256 `c5a261c0…`), built 09:38 after all native sources changed; tree = staged DLRE merge at `d3470da2` (head DLRE measured its gate at); production release build: thin LTO, one codegen unit, embedded jemalloc, no `code-rule-profile` (no stage spans) (M1:6-8).
- Profile addon (-prof): DLRE `.rerail/profiles/full-scale-direction-20261025/matched-addon/engine.node` (`0b748ebd…`), same engine, opt-level 1, `code-rule-profile`, system-malloc shim; absolute times inflated, proportions only (M1:9).
- Driver: unchanged `conformance/data-layer/suite.ts`; recording `all-60-recording.json` (pinned `2b1504ad`, 9,021 files, 137.9 MB) filtered to one rule; PIAT policies; 4 native workers; no `--cold-only` (M1:10). Command (M1:11-12).
- Columns: Cold = driver's fresh-process measurement; Ingest = per-file applies (parse, SQL anchor selection, AST matching); Global = repo-wide relational apply; Edit = warm one-file edit of `impl/common/l2200_sql_dialect/sql_connection.py` (M1:13).
- Machine: load average run -1 35–62 on 18 cores; run -2 8–48, mostly 8–17 (M1:16). Shared noisy machine, ≤2 runs each; OS file cache not flushed (M1:76-79).

### Per-rule results (M) (run -1 / run -2) (M1:18-28)
| Rule | Wall s | user/sys s | Peak RSS GiB | Cold CPU s | Ingest/Global CPU s | Warm edit CPU s (wall) | Findings |
|---|---|---|---|---|---|---|---|
| state-tripwires | 304/186 | 148/44 · 133/46 | 1.90/1.90 | 190/177 | 149/41 · 142/35 | 0.4/0.3 | 0/0, **1 failure** |
| steps | 213/97 | 52/8 · 54/5 | 1.36/1.33 | 59/57 | 25/34 · 23/34 | ~0 (out of scope) | 0/0 |
| quality | 1145/448 | 250/186 · 242/125 | **10.0/10.5** | 413/346 | 341/72 · 279/67 | 19 (41)/18 (18) | 0/0 |
| internal-dead-code | 418/192 | 181/48 · 147/40 | 3.25/3.85 | 195/161 | 160/36 · 133/28 | 32 (76)/25 (25) | 4931/4931 |
| public-api-triples | 234/126 | 125/42 · 87/32 | 2.06/2.05 | 158/113 | 151/7 · 109/4 | 6.8/4.3 | 0/0 |
| async-sync-delegation | 133/67 | 48/41 · 30/31 | 1.28/1.39 | 86/60 | 86/0 · 60/0 | 0.16/0.12 | 0/0 |
| transaction-patterns | 87/48 | 23/36 · 14/29 | 1.40/1.40 | 58/42 | 58/0 · 42/0 | 0.14/0.12 | 0/0 |
| async-sync-boundary | 91/47 | 20/43 · 12/28 | 1.28/1.45 | 61/39 | 61/0 · 38/0 | 0.16/0.07 | 0/0 |
| **Sum of 8 cold CPU** | | | | **1,220 / 994** | | | |
- Noise: wall roughly halves between passes; cold CPU drops 3–35%; CPU itself load-sensitive; treat run -2 as better (M1:30).
- Compile under 0.6 s for every rule (M1:31).
- Warm edit: out of scope for steps (meaningless); large where in scope: internal-dead-code 25–32 s CPU, quality ~18 s (M1:32).
- Sum (994–1,220 s) double-counts fixed per-run corpus ingest cost; not a 60-rule estimate; empty-rule baseline never run (M1:80).

### state-tripwires exit 1 (M1:34-37)
- Engine error, not findings mismatch (0/0, no FP/FN). `parity.json` records one failure: `state-flow-project-1`, `source-incomplete`, "repository input budget exceeded"; repeats in unchanged and edit phases ("2 incremental differences"); both passes.
- Likely factor: merged policy lacks `requireNonemptyInput: "events"` guard on four clauses (DLRE added later, commit `7222d7e8`; merged engine supports it). Test run with DLRE policy cancelled → unverified.

### Engine path per rule (M1:39-42)
- Native matcher via SQL anchor selection, no relational stage: async-sync-delegation, transaction-patterns, async-sync-boundary (global CPU 0).
- Relational executor (SQLite) at global stage: state-tripwires (32 relational clauses), steps (7 relational + 14 forbid), quality (6 relational + 1 forbid), internal-dead-code (1), public-api-triples (1).
- Even AST-only rules run SQL candidate (anchor) selection first, so "existing native matcher" = SQL-assisted matching.

### serverplan-steps stage breakdown (profile addon, cold CPU 81.4 s) (M1:44-66)
Ingestion 25.0 s (native worker counters, exclusive): anchor selection (SQL candidate selection) **20.7**; anchor index 2.8; parse (in-scope only) 0.56; manifest 0.43; tape 0.3; AST match 0 (selection rules out every candidate). Trace spans for anchor SQL: working-set 9.7, materialize 4.3, results 4.2, prepare 0.9; fact preparation 1.4.
Global relational 56.4 s (exclusive span CPU): SQLite 38.4 s (68%: sql-result 20.4, sql-materialize 10.5, sql-index 5.5, sql-input 1.2, sql-analyze 0.75); AST witness production 11.6 s (21%: input-stream 9.1, producer-prepare 1.9, input-preparation 0.6); batch overhead (relational-batch) 6.4 s (11%). Release-build global stage 34 s; apply same proportions.
- serverplan-quality-prof unusable: on opt-1 profile addon failed 858 times (856 "producer stage limit" on `quality-contracts-1`, plus "interrupted" and "budget exceeded" on `quality-conditional-1`); release addon passes cleanly (M1:68).
- state-tripwires-prof and internal-dead-code-prof killed partway; empty-rule baseline and tripwires/DLRE-policy run never started (M1:69).

### High sys time (M1:71-74)
- Sys is 30–70% of CPU for scope-wide rules (transaction-patterns 14 s user / 29 s sys; quality 125–186 s sys). steps (264-file scope) 5–8 s sys. Sys seems to scale with in-scope file count; fell as load dropped.
- Not profiled. Likely causes: per-file content-store writes/reads, page faults from large SQLite anchor working sets, contention (DLRE recorded ~16.5 GiB swap during its gate run). `sample`/`fs_usage` would separate them.

---

## 5. research/ssc-fragments.md (SSC)

### Purpose and method
- What the SSC fragment work (b87eba3b → c17d2e47) proves for the unified engine (SSC:1). Worktree clean, HEAD c17d2e47, nothing modified (SSC:11).
- Summary (SSC:5-9): strong evidence for the vtab boundary shape (§9): native matching behind one batched vtab call per fragment per batch; SQL sees only completed typed outcomes; ≈ native CPU with exact finding parity; **SQL + boundary overhead 4.9% of matching CPU**. No evidence about speed (same AstMatcher executor): 1.02× native. Contradicts spec in one place: 17 of 146 clauses run independent structural predicates as SQL over a per-node `node` table.

### e32b280b "Retire SQL interpreters and spike batched matcher fragments" (SSC:15-22)
- Removed generic finite, mixed-recursive, parse-recursive, pure-instruction-loop SQL emitters + utility-projection adapter: ~3.7k lines (`sql/relational/outcomes/*_emit.rs`, `finite_*`).
- `outcomes/retired_control.rs` keeps old control IR; every emission returns error ("SQL state machine removed: use a batched matcher fragment contract").
- Decision (`docs/sql-matcher-fragment-transition.md:10-13`): "The generic SQL state machine is a rejected compiler architecture. SQL must compose typed relations… Coherent matching operations remain inside batched native matcher fragments."
- Spike: all 11 previously refused grammars pass on 597 fixture files, 196,678 roots, 226 matches; each grammar's table called once; no matcher instruction crosses back to SQL. Per-grammar median CPU sum 232.1 ms native vs 240.5 ms SQL. Combined job: CPU 288.2 ms native vs 287.5 ms hybrid; setup 84.7 ms, prepare 0.175 ms, matching 200.6 ms; one statement, 1,625 SQL bytes, 11,873 VM instructions (M).

### c17d2e47 "Integrate typed matcher fragments with bounded SQL execution" (SSC:24-53)
- Replaces public SQL compiler/session (`sql/mod.rs` exports `engine::{Compiled, ProgramCompiler, Session}`).
- `sql/fragments/` (1,208 lines): `Fragment` + eponymous vtab schema `file,node,truth,ordinal,c0..cN,request HIDDEN,demand HIDDEN` (capture columns per fragment, declared only). Three filter modes (`fragments/mod.rs:540-603`): whole scope one file at a time; `request IN (…)` via `sqlite3_vtab_in`; `demand = handle`. Requests grouped by file, sorted by root; each file inside `with_file_memo` and `begin_clause`/`end_clause`. Outcomes 1/0/2 (logical unknown); negative tags for resource failures (`failure.rs`; −5 deadline). File's provisional rows discarded if it fails (`mod.rs:316-338`). `demand.rs` aggregate UDF builds immutable typed demand (node, list, count, span inputs), returns single-use handle; limits 1M requests or 128 MiB; empty aggregate = empty batch, never a scan. `candidates.rs` root-domain vtab, only for structural-SQL arms.
- `sql/engine/` (1,515 lines; `qualification.rs` and `tests.rs` test-only, 539 lines): `planning.rs::Template::compile` splits each clause into pure structural `Predicate` lowered to SQL (`structural.rs`: kinds, all/any/not, has/inside/precedes/follows with `neighbor`/`end` stop, unbound count) and a native remainder (AstMatcher fragment). `queries()` emits `UNION ALL` publication with 128 arms per statement (`ARMS`). Compile cache keyed `(source_digest, VERSION, each)`, LRU 1,024. `Session` prepares once, replaces batch slots, re-prepares only when a statistics-driven decision flips (`engine.rs:333-345`). `statistics.rs` + `ContentStore::search_kind_counts` does one `fts5vocab` point lookup per requested kind via `json_each LEFT JOIN`.
- Workspace: `workspace/sql.rs` hard budget **100 matching SQL executions per apply**; producer pass reserves half for consumer. `workspace/evaluate.rs` resident partitions sized at **4 MiB of source**, plans reused across partitions; statement executions set by SQLite compound limit, never file count. Per-file suspended budget passed through; `Unknown` → `budget-exceeded`.
- Other: `matcher::fragment_at`, `run::enumerate_bound` (rematch from original inputs, never previous output); `compile/mod.rs` `source_digest` (SHA-256 of AST, language, gates, utility definitions); `producers/mod.rs` adds only `OptionValue::Text`.

### Full-workload result (M) (SSC:55-86)
Workload: 5,725 files, 95.7 MB, 146 grammars from 138 policies (shipped Python catalog, not data-layer). Build opt-level 1, jemalloc. Single run. Source `benchmark/sql-matcher-fragment-integration.json`.
| Measurement | Native | SQL fragments |
|---|---:|---:|
| Initial-job CPU | 109.53 s | 111.79 s (1.021×) |
| Wall | 107.49 s | 60.27 s (unfair: SQL uses 4 worker partitions) |
| Peak RSS | 684 MB | 1,233 MB (2,933 MB before partitioning) |
| Median edit CPU, whole apply | 180.1 ms | 202.6 ms (1.125×) |
| Findings | 97,798 | 97,798, identical key hash `f1c746f8…` |
| Matching SQL executions | — | 46 (23 partitions × 2 statements) |
| Compile/prepare | 129 ms compile | 151 ms compile; 6.32 ms prepare for 8 statements; 2.87 ms lowering + assembly |
- Matching CPU 83.56 s: 79.50 s inside native boundaries; **4.06 s (4.9%) SQL + boundary**. 73.3M VM instructions; 3,289 xFilters (143 fragments × 23 partitions); 73.2M candidate roots; peak 133,408 request rows.
- Derived (D): native AstMatcher **1,086 ns per candidate root** on workspace (79.50 s / 73.2M); **509 ns/root** on catalog fixtures (876 ms / 1.72M).
- Arm split: whole clause native 129; SQL structural prefilter feeding native remainder by demand handle 14; pure SQL structural 3.
- Statistics not used in full run: `compiler.statistics` empty → all 146 `descendantAnchors` false; `fts5vocab` exercised only in test `fragment_integration_statistics_flip_shape_not_findings`.
- Zero-match rules dominate: ASYNC221 5.04 s (4.56M roots, 0 rows), F633 2.68 s, B007-uncertain 2.55 s, ASYNC230 1.96 s, ASYNC222 1.93 s (all 0 rows); top 10 = 31.0 s = 39% of native boundary CPU.
- Catalog: 87,162 rule/file pairs, 2,477 raw matches, 0 mismatches, 0 unknowns; CPU 1,034 ms native vs 1,116 ms SQL.
- Validation: SQL suite 176 passed, 0 failed, 23 ignored; 16 fragment checks, 11 hard cases, 10 three-valued, 4 registry.
- Checker `benchmark/check-matcher-fragment-integration.py`: ≤100 executions, ≤1.30× cold CPU, ≤1.50× edit CPU, ≤1.5 GiB RSS; requires parity, complete partition traces, source and binary hashes.

### Defects found (SSC:88-91)
- Logical unknown mapped to incomplete coverage (bug).
- Global `ORDER BY` over `UNION ALL` kept every producer cursor alive; removing it did not fix memory.
- Real memory problem: restored navigation data 760,717,400 bytes across 4 partitions (decoded tapes + semantic caches retained for every file); fixed by resident partitioning.

### Confirms (SSC:95-107)
- §9 boundary and principle 1: one xFilter per (fragment, batch) 3,289 = 143 × 23; completed typed output only; narrow per-fragment schemas, union schema only at final publication; SQL as driver costs 4.9%; old state-machine emitters ~3× native (`research/sql-vs-transducer.md` §0). First full-repo proof boundary cheap when native does matching.
- §13 and dead end D73 (no generic SQL interpreter): ledger rows 16 and 19 settled on this branch.
- §12 / decision 0001: set semantics, budget → `Unknown`; drops exact work charges and witness order as requirements (`doc:183-186`); one migrated test differs only in attempted-witness metric after exhaustion, accepted on findings + coverage (`doc:361-365`).
- §11 plan cache: only changed decision re-prepares; numeric drift does not; cache by digest + compiler version; cache key does not imply sharing between clauses (ledger row 9).
- §19 shape: executor is the whole cost (79.5 of 83.6 s native matching); 10× gain must come from AST row (M2), not boundary.
- §5.1: `k%04x` tokens overlap across dialects → counts are "cost estimates, not exact language frequencies" (`doc:225`) → evidence for global kind codes.

### Contradicts (SSC:109-115)
- Structural predicates in SQL: spec §7 "If a rule feature cannot lower, the IR is extended. It is never composed in SQL"; spec §9 forbids rows per node. SSC lowers kinds/not/has/inside/precedes/follows/count to correlated `EXISTS` and `count(*)` subqueries over per-node `node` vtab for 17 clauses (`structural.rs:96-162`). Cost not separately measured (inside 4.06 s; 136 ms of 1,012 ms on catalog). Verdict: wrong boundary for the main engine; correct home is pure-SQL fallback (§13); predicates map onto E2, E3, E4, E12 `DESCEND_KIND`.
- Output contract: fragments registered `multiplicity: bag`, ordered (file, node, ordinal), each file's matches sorted by root (`planning.rs:96-120`, `engine.rs:501`) vs spec E1/§12 set mode, no order. Ordinal needed only for `each` enumeration.
- Unit of batching: one fragment per clause, no program groups, no label demultiplexing (§11); principle 5 (batch across rules within identical scope) and §7 fusion forms not exercised.

### Refines (SSC:117-141)
1. Boundary rule (`doc:54-56`): "Keep capture rollback, recursive utility calls, and ordered searches inside a fragment when they share state." A fragment is cut only where no rollback, utility or ordered-search state crosses the cut.
2. Typed matcher fragments = per-program shape of `t_match` + `t_export_<schema>`; typed capture cells (node, list, span, count) keep document identity; span from another file rejected (`engine.rs:458`); output columns only declared captures; registry contract `completion: domain` (absent rows within requested domain mean false).
3. Capture-bearing demand (E7) needs a demand handle: `sqlite3_vtab_in` carries one scalar; tuples (request, file, node, typed inputs) via aggregate UDF → single-use handle read through equality on hidden `demand` column; expiry; cleared on success and failure; second consumption rejected; transport budget. "First working design for E7; adopt verbatim."
4. Three outcome classes: matched; logical unknown (completed verdict from three-valued matcher, never coverage loss); resource failure (`Unknown(reason)`, discards file's provisional rows). Decision 0001 excludes three-valued semantics but frontend still produces logical unknowns (`three_valued` grammars). SSC publishes `WHERE truth=1 OR truth<0` (drops truth=2).
5. Batching bounded by resident memory, not file count: 760 MB navigation data; ~4 MiB source per partition; RSS 2,933 → 1,233 MB at unchanged CPU; executions capped by job budget (100), count follows compound-select limit (128 arms). SQL path still 1.8× native RSS — unexplained, open.
6. Unit costs for §19: native AstMatcher 0.5–1.1 µs per candidate root (opt-level 1, root-kind-only candidate selection, no file pruning); boundary + publication ~55 ns per candidate root (4.06 s / 73.2M), ~1.2 ms per xFilter; preparation negligible (6.32 ms for 8 statements).
7. Fusion evidence on second ruleset (author's inference, not measured fusion): on Python catalog, zero-match rules anchored on very common kinds (4.56M roots, looks like `call`) take 39% of native CPU in top 10. Case for root-kind dispatch + tries + shared sub-rules. §7 item 3 says tries "not expected to pay" on data-layer; scope claim per ruleset.
8. Per-edit scope: 180–203 ms is whole workspace apply incl. response with ~97.8k violations; not comparable to fusion model's 2.9 ms p50 (AST matching only). §14/§15 must name what an edit measurement includes.
9. Line-count correction: `sql-vs-transducer.md` sized pivot at 614 + 419 test lines; now `sql/fragments/` 1,208 + `sql/engine/` 1,515; `sql/` still 104,176 lines (frame compiler kept "for historical tests").

### Not proven (SSC:143-149)
Any speedup; fusion/program groups; SQL composition across fragments at workload scale (only unit test `fragment_spike_composes_typed_outputs_with_join_antijoin_and_aggregate`); statistics planning in full run; pure-SQL bindings for sequential contracts (unresolved, `doc:357`); data-layer rules; optimised build.

### Merge map (SSC:151-174)
| SSC source | Target | Notes |
|---|---|---|
| `sql/fragments/mod.rs` (vtab Table/Cursor, 3 filter modes, per-file grouping, failure isolation, Metrics) | `engine/vtab/transducer.rs` | swap `matcher::fragment_at` for transducer program-group execution; keep AstMatcher binding behind same contract as interim executor + parity oracle |
| `sql/fragments/demand.rs` | `engine/vtab/demand.rs` | E7 / §9 capture-bearing demand |
| `capture_cell.rs`, `span_capture.rs` | `engine/vtab/` (typed `t_export` codec) | |
| `capture_env.rs` | `matcher/` | AstMatcher-specific |
| `failure.rs` | `engine/vtab/` → `t_outcome.reason` codes | |
| `candidates.rs`, `engine/structural.rs` | `engine/fallback/` | pure-SQL structural subset; reuse `Predicate` as lowering checklist in `engine/transducer/lower/`; not an execution path |
| `planning.rs` Template split, `decisions()` | `engine/plan/` | |
| `planning.rs` `queries()` | `engine/compose/` | |
| `planning.rs` `manifest()` | `engine/relations/` (DLRE `producers/`) | |
| `statistics.rs`, `search_kind_counts` | `engine/stats/`, `content_store/` | change to global `k%06x` tokens (§5.1); add decision-interval storage of `compiled_plans` |
| `engine.rs` (Session etc.) | `engine/plan/` + session in `engine/vtab/` | drop per-file `sort_by_key` |
| `workspace/sql.rs` budget; `workspace/evaluate.rs` partitioning, `Unknown`→failure | integration workspace / `engine/plan/` batch formation; spec §18 memory pools | |
| `compile/mod.rs` `source_digest` | `engine/plan/` identity | |
| `matcher::fragment_at`, `run::enumerate_bound` | `matcher/` | |
| `producers/mod.rs` | integration tree | only `OptionValue::Text` |
| checker + `workspace/fragment_integration.rs` | §15/§18 measurement tooling next to `full_repository.py` | |
| test files | differential/parity tests | |
| `retired_control.rs`, frame compiler, state-machine IR | **do not merge** | |
- Update `architecture.md` phase 4 to merge at c17d2e47 (not b87eba3b); scope grows to include boundary.

### Recommended spec text changes (SSC:176-210)
1. §9 demand handle paragraph (aggregate `demand_<group>(request, doc, root, inputs…)`; expiry at batch end; cleared on success/failure; reject second consumption; empty aggregate = empty batch; transport bounded 1M / 128 MiB; scalar demand via `sqlite3_vtab_in`).
2. §9 relations: `completion: domain`; only `t_outcome` reports incompleteness; resource failure discards (program, document) provisional rows; other documents publish.
3. §9 "Measured cost" paragraph (4.06 of 83.56 s; 4.9%; 73.2M roots; 3,289 xFilters; 5,725 files; exact parity; emitters ~3× native).
4. §6 E11: three outcome classes; logical unknown never counts against coverage; boundary publishes it as no match until frontends stop producing three-valued results; record as decision.
5. §7: fragment boundaries follow shared state; structural predicates lower to E2/E3/E4/E12; not composed in SQL over a node relation in main engine; SSC structural lowering is §13 fallback emitter.
6. §7 measured list item 3: scope tries claim to data-layer; Python catalog 39%; re-evaluate per ruleset from `plan_feedback`.
7. §8/§18 resident-batch pool: CPU batches cut by resident bytes; 760 MB; 4 MiB-source target; 2,933 → 1,233 MB.
8. §11: explicit job budget of matching statement executions shared across producer/consumer; statement count follows compound limits (128 arms); statistics refresh re-prepares only on decision change.
9. §5.1 evidence: `k%04x` overlap.
10. §12: ordinal only to key multiple witnesses of an `each` rule; boundary outputs carry no order; consolidation sorts.
11. §13: starting points SSC `structural.rs`, `candidates.rs`, surviving non-recursive `sql/relational/` emitters; sequential contracts (recursive utilities, capture rollback, ordered search, parsed regions) have no pure-SQL binding; fallback reports them uncovered with a coverage metric.
12. §15: adopt SSC checker pattern (ceilings: cold CPU ratio, edit CPU ratio, peak RSS, execution count; source/binary/corpus hashes; one process per engine for RSS; edit control add/add/restore with retraction checked); edit measurement names its scope.
13. §19.1 rows: native AstMatcher 0.5–1.1 µs/candidate root; vtab boundary + publication ~55 ns/root, ~1.2 ms/xFilter (M). §19.2: third data point — current native executor same order as fusion harness; executor speed (M2) decides AST row.
14. §16 module-map source column updates; correct `architecture.md` phase 4 and `sql-vs-transducer.md` §2.1.

---

## 6. research/workstream-scan-2026-10-25.md (WS)

### Purpose and method
Read-only scan of streams since the survey, measured against `docs/rule-engine-direction/` (README, architecture, findings-contract, authored-sql, prior-work); "HI" there = `prior-work/ast-index-spike-history-index.md` (WS:3).

### Summary table (WS:7-17)
| Stream | New | Heading | Verdict |
|---|---|---|---|
| DLRE (uncommitted, 68 files, +2,952/−398) | full-repo measurement reset; shared SQL execution owner `CodeRuleDslExecution`; SQL producer identity; materialization boundaries; candidate retirement | authored-SQL relational execution shareable and bounded | measurement converges (adopted gate, read README); execution work deepens authored SQL, conflicts with decision 0002 |
| N1562 (15 commits `16343282..8b6576f0`) | `docs/sql-scope-and-operational-semantics-design.md` (776 lines); `query/plan/{identity,scope,grouping,utility,semantics}.rs` | exact-scope grouping, semantic plan ids, utility dedup, SQL statistics, no SQL continuation state | strong convergence on planning; diverges on operational semantics (EffectSummary/replay ledger, ordered regions conflict with decision 0001) |
| SSC worktree (branch `jeremy/sql-capture-demand-spikes`, uncommitted −3,753 lines + `fragments.rs`) | `docs/sql-matcher-fragment-transition.md` | SQL composes typed relations; native fragments own sequential semantics | converges (README §4, principle 1; HI D73); still holds "exact work charges" open |
| Moti `facts-plan` (7 commits past `facts-engine`, tip `9a901915`) | `docs/facts/DECISION-CROSSFILE.md`; Y1 maintained reference graph; Y2 local queries | maintained compact reference graph + rules as local queries | converges with authored-sql.md; new evidence for per-edit cross-file |
| Moti `facts-engine`, `sql-dl60`, `sql-set-composition` | nothing new | | |
| Tempo Metal (6 commits `a091aa063..e95613625`) | terminal ownership, retirement, containment fixes | full-corpus run still fails | old `policy_dsl` transducer; full run 701.9 s, ~2 GB tracked, 577 dispatches vs 600 limit, failing on containment memory |
| ast-index-spike | no new commits (origin tip `9449260`) | | |
| PR 2539 | OPEN, last update 2026-08-12 | | |
| Other | `tbelanger/rule-pack-generators`, one-line touches | pack generation | not engine execution |

### DLRE (WS:19-64)
- Numbers from `docs/data-layer-performance-journal.md` "Full-repository measurement reset", first-50 over 9,021 files (M as reported by DLRE):
  | | Pre-SQL reference | Last SQL run | Current |
  |---|---|---|---|
  | Cold CPU | 125.8 s | 2,507.5 s | 958.6 s |
  | Ingestion CPU | 73.4 s | | 875.3 s |
  | Wall | | | 1,978 s |
  | Full-process RSS | | | 5.83 GiB |
  | Findings | | | 24,820 (match) |
  | Execution failures | | | 3 |
- Failures: clause work budget on one generated file; name-closure incompleteness; interrupted relational query.
- Exclusive counters: **anchor selection 537 s**, index preparation 137 s, AST matching 58 s, parse 11 s. Current still 7.6× worse than pre-SQL. 60-rule run in progress.
- Self-diagnosis: `Lower::pattern` builds every possible posting and runs `count(*)` to price each arm at run time (HI T18: seeds located only to be priced); producer range path estimates constant 32 rows; physical anchor planning must move after index publication to read SQL statistics; statement counts demoted to diagnostics; read the direction README and ledger.
- Execution owner (`docs/sql-execution-owner.md` etc.): one FTS store shared across consumers; SQL producer identity via SQLite authorizer; materialization boundaries (shared producers materialize once; inlining everything hit SQLite's 65,535-reference limit); CTAS type inference; candidate retirement at 131,072 rows. Makes red test `test/engine/native/recovery/execution-owner.test.ts` go green.
- Perspective: measurement reset and anchor diagnosis "exactly right"; stats-chosen anchors from `fts5vocab` fix the largest cost (537 of 958 s). Rest is infrastructure for authored SQL; under decisions 0001/0002 most loses purpose; temp-table/materialization path = HI D50 territory. Keep: shared source owner and version isolation; `relational_facts.py` capture diff; `scripts/profiling/full_repository.py` runner ("the most valuable new artifact anywhere"). Matched controls use `JEMALLOC_OVERRIDE` with system-malloc shim — allocator confounds comparisons.
- Integration: re-merge DLRE when committed (conflicts expected on `relational.rs`, `workspace/global.rs`, `anchors/sql/corpus.rs`, `relational/syntax.rs`; `file_tape` returning `Option`, `FileImage`); take `full_repository.py` and execution owner in phase 1b; producer-identity/materialization/retirement frozen with `relational`.

### N1562 (WS:66-90)
- New: design commit `ae305827` + seven TDD pairs: architecture boundary checks (`check-boundaries.mjs`, `query-plan-boundaries.mjs`: no SQL continuation state in plans); deterministic semantic plan identities (no `Arc::as_ptr` memo keys); exact candidate-scope identities (`ScopeRecipe` digest of typed fields, not SQL text); clause grouping only by exact candidate scope; utility dedup by semantic demand identity; "assign ordered work to typed operators" (`semantics.rs`: `DslBehavior` → declarative contract or pushed operator e.g. `OrderedRegion`, `EffectLedger`).
- Design also: statistics in SQL-visible tables incl. `fts5vocab`; vtab `estimatedRows` from statistics; one statement per exact-scope group, split only on measured limits; 8 MiB limit as safety check only.
- Perspective: most direction-aligned planning code (principles 3, 4, 5; README §3, §5); better `engine/plan/` source than SSC `physical.rs`/`job.rs`. Diverges: `EffectSummary { replay_work, relative_peak_depth, fills }`, `LogicalEffectLedger`, `OrderedRegion`, `FirstWitnessRelation`, "budget contract" keep memo replay/allowances/first-decisive order exact — removed by findings-contract §3; under loose contract `semantics.rs` collapses, `EffectLedger` disappears.
- Integration: merge at `8b6576f0` in phase 3; phase 4 move `query/plan/{identity,scope,grouping,utility}.rs` → `engine/plan/`; phase 5 mutate `semantics.rs` and effect ledger out.

### SSC (WS:92-109)
- Uncommitted (worktree `…Moti-Velz-sql-set-composition`): decision doc 2026-10-25; removes ~3.7k lines of finite/recursive SQL interpreter emitters; adds `fragments.rs`; composition control passes (join of two fragment relations + exclusion + aggregation); targets 11 catalog refusals (B615, SIM905, B020, F405…); production planning reads statistics from SQL; compiled output cached by digest + compiler version.
- Perspective: convergence; retires frame/emit engine; HI D73 (query-as-data interpreter 12× slower) predicted it; "open semantics" section still preserves exact work charges, cache-fill accounting, depth "within a complete execution region" — goes away under decision 0001. Fragment contract is right shape for `terminal_match` relation native binding.
- Integration: phase 2 merge SSC at next commit; else merge `b87eba3b` and delete same files in phase 5. [The SSC file reports this has since been committed as c17d2e47.]

### Moti facts-plan (WS:111-133)
`DECISION-CROSSFILE.md`, 40 worktrees, 1,111 events, 5 repository plans:
| Design | Memory | Latency | Outcome |
|---|---|---|---|
| Full differential-dataflow maintenance | 28–42 GB | | cannot reach 250 MB; floor ~50 B × 120M records |
| Recompute | | 5–70 s per edit (dead code, exported-families) | inputs change on 88% of edits |
| Shared declared-facts layer (X6) | | 0.3–0.7 ms per rule per file | exact, but does not fix cross-file arithmetic |
| **Y1** maintained reference graph (keyed tables, cached per-key queries with read sets, base + overlays + worktree deltas) | 72 MB base | 0.73 / 2.4 ms p50/p95 per edit | exact against 145,431 edges |
| **Y2** dead code + exported-families as local queries over graph | 24 MB across 41 worktrees | 0.07 / 1.5 ms | 0 diffs |
- Conclusion: target unreachable "over these rules as authored"; needs rule-shape change (local queries instead of global fixpoints) + one maintained compact reference graph.
- Perspective: strongest evidence for authored-sql.md recommendation; quantifies why DLRE's global fixpoints (E, G) cannot meet per-edit; confirms cross-file per-edit needs maintained state; direction docs don't yet cover incremental maintenance.
- Integration: phase 7 takes Y1 (and `DECLARED-FACTS.md`) as spec for `reference_graph`/`symbol_ref` contract; phase 5 re-express dead-code and exported-families as local queries; "one maintained reference graph" as decision record candidate.

### Tempo Metal (WS:135-146)
- Six commits; handover: frozen full run on `441c218c3`, 577 dispatches complete, failing at containment scratch (22.6 MB request with 1.995 GB reserved), 701.9 s, process-group RSS 3.24 GB, 600-dispatch limit; no full-corpus parity.
- Perspective: still full-corpus batch executor on pre-v7 `policy_dsl` transducer in another repo; direction needs GPU for per-edit, where README's limiting factor is launch count per file; current trajectory (more dispatches, ~2 GB) opposite regime; exactness work partly serves exact output order (dropped by findings-contract §3). Keep typed output pages, IN-demand; phase 6 port onto v7 behind `terminal_match`; re-pin `e95613625`.

### Integration plan changes (WS:148-155)
1. N1562 merge at `8b6576f0`, `query/plan/` as `engine/plan/` source. 2. SSC prefer after transition committed. 3. DLRE phase-1b re-merge of execution owner + `full_repository.py`. 4. facts-plan (Y1/Y2) as phase-7 input + decision candidate. 5. Tempo re-pin `e95613625`. 6. Three streams (N1562 semantics/EffectSummary, SSC open semantics, Tempo ordering) still encode exact operational semantics; need to hear decision 0001.

---

## 7. research/shape-fixes.md (SF)

### Purpose and method
- Data-layer rule shapes: measured costs and fixes; date 2026-10-25; engine = merged-tree release addon; samples = 300 random files from the 9,021-file recording, plus full corpus for single checks; every run capped at 5 minutes; "before" numbers = recorded phase A runs, not re-run (SF:3).
- Later sections use execution-vs-construction split with per-thread CPU clocks; runs had 10 GB swap in use on a shared machine; system time 20–35% of every figure (SF:213-218).

### §1 Why AST rewrites looked slower single-rule (SF:5-22)
- Phase A rewrote text-regex clauses as syntax clauses (backend-type-strings, hasattr-async…). Single-rule full corpus CPU 1–4 s → 30–50 s; 1,500-file sample backend-type-strings 2.0 → 9.2 s CPU, ~half system time.
- Stack sample: dominated by `pwrite`, `fsync`, WAL checkpoints under `anchors::sql::corpus::Corpus::put`; tree-sitter parse ~5% of active samples.
- Cause: every file a syntax clause may match is first published into a temporary SQLite positional index; candidate roots selected from postings (`AnchorPlan::needs_index`); text-only rules never publish.
- All 60 rules, 300 files: before phase A 46.3 s user + 4.4 s sys, 1.75 GB; after 46.7 + 4.6, 1.57 GB, identical set, same 22 failures → rewrite cost-neutral when run together. `synchronous=OFF` on temp store: sys 4.0 → 3.5 s on 1,500-file case.
- Structural fix = spec direction: local rules execute over resident tapes (§20) as transducer programs; only facts and relational candidates reach SQL; per-file publication of every node into a positional index should not be on local rules' path. [Note: §11 below withdraws/qualifies the related opt-in conclusion: index is mandatory.]

### Summary, all 60, 300-file sample (SF:24-30)
| State | CPU (user+sys) | Peak RSS | Failures | Findings |
|---|---|---|---|---|
| Before phase A | 50.7 s | 1.75 GB | 22 | baseline |
| Phase A + shape fixes (§3, §5–§8) | 32.9 s | 1.03 GB | 10 | identical |
| + engine fixes (§9) | 29.7 s (−41%) | 1.10 GB | 0 | identical |

### §2 One rule at a time, 300 files (SF:32-50)
- Floor for any rule alone ~1.7 s (process start, compile, publication for syntax rules).
- legacy_execution_surfaces 15.43; serverplan_quality 11.62; serverplan_state_tripwires 5.89; internal_dead_code 4.77; serverplan_dispatch_architecture 4.66; plan_scope_ownership 4.65; execution_authority_bypass 4.43; serverplan_grammar_surface 4.17; public_api_triples 3.69; conn_or_txn_union 3.59; 48 more 2.8 → 1.7.
- Splitting all-60: 8 M1 heavy checks 23.7 s, other 52 31.0 s → cost not confined to M1 set. Every check above ~3 s is a relational clause fed by whole-tree copies.

### §3 Worked fix legacy_execution_surfaces, lowering rule L5 (SF:52-70)
- Before: one relational clause, 25 inputs (`syntax_0…24`) exporting every identifier (sharded by first letter), string, call, attribute, argument list, statement, block, literal; 23 SQL stages rebuilding `nodes`, `edges`, recursive `ownership`, `receiver_tokens`, call graph; final selects ~15 issue codes, mostly local; oracle reports zero findings on corpus.
- After (`scripts/dsl/data_layer_six/legacy.py`), L5: local AST clauses for LEG-020/030/040/050/060/080/081/082/084 (call, definition, import, if-statement shapes with literal name sets, `filePath` scopes, allowlists) plus receiver token (`has` whose `stopBy` halts at anything other than attribute, call, subscript value, parenthesized expression; cast's 2nd arg own alternative) and not-inside-legacy-decorated-function (`inside`). One relational clause for joins only: LEG-010, LEG-021, LEG-083 (eager-call reachability fixpoint over in-file call graph), LEG-085 (import aliases); narrow inputs `fns`, `legacy_fns`, `wrapper_fns`, `classes`, `ident_calls`, `attr_calls`, `other_calls`, `from_imports`, `direct_imports`; innermost enclosing function by span containment, no tree walk.
  | | Before | After |
  |---|---|---|
  | Fixture parity (80 oracle rows) | exact | exact (setFP 0, setFN 0, failures 0) |
  | 300-file CPU | 15.4 s | 1.8 s (floor) |
  | Full corpus CPU | >1,290 s (killed) | 34 s; 0/0, 0 failures |
  | Relational inputs | 25 whole-tree shards | 9 narrow candidate sets |
- Receiver-token relation and in-file call graph (`resolved_calls`, `eager_reach`) recur; under decision 0005 candidates for catalog facts `py.receiver_tokens`, `py.call_edge_local`.

### §4 Findings on the way (SF:72-76)
- New hasattr-async gate finding (`l2070_batching/execution_runtime.py:5978`) genuine: `getattr(self._backend, "…_async", None)` split across lines; oracle line regex misses it.
- Rule-level trigger gate (`inside: module` + regex) evaluated once per file via relation memoization; literal sets in regexes give per-file `memmem` prefilter (`GrammarMatcher::admits_source`) → literal-name clauses near free on files lacking names.
- `filePath` needs ≥1 of `regex`/`notRegex`; empty allowlist must emit no gate.

### §5 Plain syntax rules: prefilter and recursion (SF:78-93)
- `value` tests disable per-file literal prefilter: `{'value': {'regex': …}}` makes clause three-valued; compiler drops `memmem` prefilter (`reads_values`). Testing quoted source text (`regex` on string node) keeps prefilter, same matches except implicit concatenation/escapes.
- Self-recursive helpers (`has: {matches: self}`) roughly double clause cost even when few nodes reach them; plain `has … stopBy: end` or helper unrolled to fixed depth falling back to recursive form matches exactly at floor cost.
- Cheap tests first: text conditions on candidate before tree walks; allowlist exclusions last.
  | Check | Parity | 300-file CPU before → after | Cold-phase user ms (floor) |
  |---|---|---|---|
  | plan_scope_ownership | exact (30) | 4.65 → 3.13 | 2178 → 1040 (1058) |
  | transaction_callback_wrappers | exact (20) | 2.54 → 1.73 | 946 → 533 (407) |
  | conn_or_txn_union | exact (15) | 3.59 → 2.49 | 1734 → 720 (721) |
  | serverplan_grammar_surface | unchanged | at floor (scope `**/*.py`; cost = parsing) | |
- Engine implication: prefilter and transducer `TEXT_ID` literal tests (spec-transducer §5.2) must survive three-valued value tests; recursion through `matches` should compile to a bounded walk, not re-entrant match. Tracked against M3.

### §6 Batch six (SF:95-106)
| Check | Parity | 300-file CPU before → after |
|---|---|---|
| serverplan_dispatch_architecture | exact (59) | 4.66 → 1.19 |
| execution_authority_bypass | exact (5) | 4.43 → 3.94 (scope floor ≈3.3) |
| serverplan_state_tripwires | exact (79) | 5.89 → 5.33 |
- Gate order matters: whole-file `syntax_gate` inputs tested source gates (walk to module) before `kind: module` → every node walked to root; test kind first, own shape before gates.
- Path-only inputs get no prefilter: `filePath` doesn't skip node-by-node matching; dispatch's 25 inputs checked in every file though only `base_server_plan.py` can match; module-text requirement (`class BaseServerPlan`) gives `memmem` prefilter. Engine should treat constant-false `filePath` as file prefilter (residency/scope work §20).
- `value` predicate costs per file even where prefilter should skip: serverplan-ddl ~0.45 s on sample with no `ddl_effect_lazy`/`grammar_effect_lazy`; removing `value` → floor.
- Remaining: state-flow taint relation still whole-tree copy; `direct_sinks` counts every ancestor of each sink, ~1.1 s over floor; next L5 rewrite; natural consumer of `py.local_flow` catalog fact (decision 0005).

### §7 serverplan_quality (SF:108-126)
- Removing one clause at a time: `quality-conditional-1` ~5.8 s of 11.6 s (copied tree nodes from every file); `quality-contracts-1` ~2 s; `-2` and `-3` together ~1 s; taxonomy ~0.5 s.
- `conditional.py` rewritten as one global clause `conditional-architecture`: same detectors, locations, key, code (SPQ-029); ~17 narrow inputs (classes, base lists, import names, functions, `self.x` stores, call shapes, validation-method blocks); span containment instead of nodes/edges/ownership/descendants; remaining 2.4 s all AST input matching; SQL negligible.
- SPQ-051/052 → forbid; SPQ-053 → small relational `ordered-semantic-inventory`; removes `quality-contracts-2`, `-3`. Shadowing test in `test/scripts/test_shared_sql.py` ported to span-based stages (`cclasses`, `cmethods`, `cfunctions`).
- Parity exact (200 rows); 300-file CPU 11.62 → 8.18 s.
- To do: `quality-contracts-1` (~80 detector rows on whole-file syntax relations): most → forbid; narrow relational for SPQ-001/006 (duplicate methods by name), SPQ-020 (pre-op functions, fixpoint over names), SPQ-030/031 (call paths), SPQ-042 (module string constants), SPQ-050 (import and scope resolution).

### §8 Batch five, cross-file rules (SF:128-141)
| Check | Parity | 300-file CPU |
|---|---|---|
| internal_dead_code | exact (43) | 4.77 → 3.02 |
| public_api_triples | exact (38) | 3.69 → 2.56 |
| imperative_private_functions | unchanged | 2.83 (~650 ms engine reference call graph) |
| async_sync_delegation | unchanged | 2.80 (spread across clauses) |
- No tree copy; cost from inputs engine could not narrow to root kinds: `identifier` inside an `any` with other kinds; `inside` whose target is a `not` or contains a nested relation → every identifier a candidate. Fix: split each input by kind of place it reads; add redundant narrowing `inside class_definition (field superclasses)`.
- Lesson for M3: root-kind inference must see through `any` of kinds and negated/nested `inside` targets; transducer gets this from its `KIND_SET` entry test.

### §9 Engine fixes (SF:143-146)
- Prefilter kept for three-valued matchers (`native/src/code_rule_dsl/compile/mod.rs`): `RequiredText::of` reads only two-valued source-text tests (node regexes, pattern leaves), never `value` tests, through Kleene-monotone connectives `all`, `any`, `has`/`inside`, double `not`; file missing required literal fails positive match outright → skipping sound even if rule can be unknown elsewhere. Test `matcher::tests::three_valued::a_three_valued_rule_keeps_its_source_prefilter`.
- Stage index names unique per physical table (`relational.rs`): deferring stage stores rows in `__stage_<name>` behind view `<name>`; non-deferring in `<name>`; both named indexes `__<name>_<n>`; SQLite index names global → "index … already exists" → clause Unknown. Caused all 22 baseline failures on 300-file all-60 (10 after rewrites).

### §10 State-flow taint on narrow inputs (SF:148-161)
- `state-flow-narrow`: one module gate, identifiers with roles, string literals, node kinds fixpoints read; span containment for ownership; ancestor ordering of sinks reproduced by counting `branches` rows containing each event.
- Parity exact (79); clause cost over no-clause variant ~1.07–1.10 s → ~1.05 s. No CPU gain: old CST stages still feed `state-flow-project` (tree still copied); only 24 files pass gate on sample; remaining ~0.4 s input extraction (input not headed by one node kind walks whole tree) + ~0.6 s per-file SQL compile/execution over small data. Win arrives when `state-flow-project` moves to narrow inputs. Unconfirmed lead: per-connection regex cache (64 entries, cleared when full) may churn [§12 later: not churning].

### §11 Positional SQL index mandatory; measure execution without construction (SF:163-174)
- **Withdrawn**: earlier version made index opt-in (`RERAIL_AST_SQL_INDEX`) and concluded net loss. Wrong twice: (1) index not optional — anchor index (kind postings, value postings) is how engine identifies/matches anchors; anchors chosen as rarest mandatory concrete variable, value anchor before kind (ast-index-spike design index topics 3 and 5, "kidx", "vidx is mandatory"; D S049-S054, S151, S053); built every time; opt-in reverted. (2) Whole-run CPU mixes construction and execution; parse, tape build, index publication = construction; matching/querying = execution; comparisons report execution only, construction on its own line.
- Publication-path costs to fix inside the index: (1) `Corpus::put` holds corpus-wide `state` mutex during whole publication → four workers wait on one writer; (2) `Store::temporary` named file in WAL mode → `pwrite`s and checkpoints though discarded; (3) value-term index written row by row via trigger upserting `vlex` = **68% of publication time** in sample.

### §12 Round 2: no data-layer rule shape copies the tree (SF:176-188)
- state_tripwires: project-mode builder contracts and signatures clause read narrow rows (signatures was real cost); call-tail regexes combined, distinct 51 → 16; parity exact (79); 300-file CPU 5.0 → 4.5 s.
- quality: `quality-contracts-1` → ~100 forbid + six small relational (sync/async pairs, pre-op fixpoint, module string constants, projection repairs, annotation scope resolution, ordered-semantic inventory); parity exact (200); 8.17 → 7.31 s.
- **Cost is per admitted file**: parse cost same whatever the rule does; neither `filePath` nor literal prefilter avoids parse; `ast.file: {path: …}` does; entry-point/production domains belong there; a single clause with an every-file domain sets the floor for the whole rule.
- Regex cache not churning (no clause > 64 distinct patterns); each pattern recompiled once per file session because SQL functions registered per connection lease; small cost noted for M3.
- Approximations (fixtures exact; edge-case diffs possible): SPQ-006 drops "later definition"; SPQ-002/022 class level = 4 when `level` returns 4 and no other integer; string values matched on quoted source text (SPQ-013/014/032/035/042/047/050); SPQ-035 doesn't check "exactly one `through`"; SPQ-047 skips interpolating f-strings; SPQ-021 `Sql…` callee test covers only name and attribute callees.

### §13 Full-corpus check: two round-1 rewrites regressed (SF:190-209)
| Rule | Pre-rewrite | Round-1 rewrite |
|---|---|---|
| internal-dead-code | 4,931/4,931, 0 failures, 68 s CPU | 0 findings: `reach_edges` interrupted |
| public-api-triples | 0 failures, 24 s CPU | `traits` interrupted |
- Cause: relational session SQL allowance `max(maxVisitedNodes, input rows × maxVisitsPerNode)` (`relational.rs`) assumed whole-tree inputs; narrow inputs shrink allowance while joins do same work → engine penalizes the shape the spec asks for.
- Two splits changed results: params only through annotations +177 FPs (internal-dead-code); splitting parameter forms +23 (public-api-triples). Neutral: splitting base classes into two inputs; `direct_legacy` file gate. Both rules restored to verified shapes. Every rewrite needs full-corpus check of each repo-wide clause.
- Third finding: `serverplan_quality` FP on `SqlConditionalStep` (SPQ-029): base `ConditionalStep` never resolved — import rows held only first name of `from m import (a, b, …)`. A relation with `field` reads only that field's first child (as upstream ast-grep); all names of an import share `name` field → `has: {field: name}` with `each` cannot enumerate an import, nor `inside` with field. 10 import rules (conditional, legacy, quality, quality_annotations, target) had the shape; now use `imported(...)` (`common.py`): `has` over statement children excluding `from` module. Fixtures exact (80, 200, 12); full corpus clean.
- Allowance fix: now per node of scoped source (est. one node per 8 bytes) when that exceeds per-row count; repo clause's step budget grows with scoped source like `file_work_budget`. Narrow internal-dead-code/public-api-triples variants then finish exact (4,931/4,931). Narrow internal-dead-code slower (90 s vs 68 s CPU) → restored shapes stay.

### §14 Execution apart from construction; anchor selection fixes (SF:211-243)
- Anchor index built every time (decision 0006). Phase clocks count calling thread's CPU (`phase_clock.rs`), lock waits count nothing. Construction: parse, index publication, lowering+preparing anchor SQL (`anchorCompile`, nested in selection and subtracted). Execution: clause matching, anchor selection, relational SQL. Suite reports both as `coldProfile`.
  | Change | Effect | Kept |
  |---|---|---|
  | File rows built outside corpus lock; vocabulary per file not per row | publication 7.1 s wall (lock waits) → 2.1 s CPU, 300 files | yes |
  | Inside walk selects by `IN` membership, not join of two unindexed CTEs | public_api_triples selection 96.5 → 39.3 s; findings identical | yes |
  | Value postings inserted in key order | no measurable change | yes (free) |
  | Group plans by files that demand them | fewer templates but each file runs once per scope: selection +6% to +35% | no |
  | 64 MiB writer page cache | −4% (noise), +80 MB | no |
- Inside-walk join quadratic: SQLite planned `w JOIN m ON fid,pre` over two materialized CTEs as `SCAN m × SCAN w`: up to **610M VM steps for a four-file batch**. Attribution via profiling build (`RERAIL_PROFILE=1` spans) and `EXPLAIN QUERY PLAN` on captured batch DBs.
- Full corpus (9,021 files), execution CPU s, at `be03e110`:
  | Check | Match | Anchor selection | Relational | Construction | Result |
  |---|---:|---:|---:|---:|---|
  | serverplan_target_architecture | 0.2 | 11.6 (was 17.0) | 0.1 | 11.4 | 0 failures |
  | backend_type_strings | 0.0 | 1.3 (1.9) | 0.0 | 32.3 | 0 |
  | type_safety_patterns | 0.0 | 6.7 (10.1) | 0.0 | 33.6 | 0 |
  | legacy_execution_surfaces | 0.1 | 14.2 (17.1) | 2.3 | 9.5 | 0 |
  | serverplan_state_tripwires | 16.2 | 38.7 (54.8) | 34.3 | 45.9 | 24 `source-incomplete` as before |
  | serverplan_quality | 3.3 | 82.3 | 3.3 | 131.4 | 0 (timed out before) |
  | internal_dead_code | 7.2 | 83.8 | 30.0 | 36.9 | 4,931/4,931 (timed out before) |
  | public_api_triples | 2.1 | 41.0 (100.8) | 6.7 | 36.8 | 0 |
- Anchor selection still largest execution cost; matching never > 16 s.

### §15 Tree neighbourhoods instead of every node's parent (SF:245-288)
- Spike settled it (design index topic 2; DH S037, S058, S064): record carries parent delta and subtree size → parent = one record read; children via native walk (`nera_kids`: first child `pre + 1`, next sibling `pre + size`). Node table, or parent for every node, rejected. Commits `702ee406`, `813cc78b`.
- `nera_tree(fid, node, axis)` (`tree.rs`) reads children (sibling skips) or ancestors (parent deltas) from tape.
- Unanchored inner plans take the neighbourhood: field-less `has` uses scope's children; recursive `has` uses scope's subtree intervals; recursive `inside` uses scope's ancestors.
- Anchored inner plans keep postings; anchored = selects from postings before reading scope: root kind or pattern; conjunction with any anchored member; disjunction of anchored branches; non-recursive `inside` or field-less `has` whose inner plan is anchored. First version climbed ancestors of every node in a common posting set for `any` of kinds — cost more than postings (internal_dead_code +36% steps).
- Conjunctions evaluate anchored members first (`all: [not …, inside {kind: module}]` used to start with `all_nodes EXCEPT …`).
- Over every node, non-recursive `inside` reads inner postings' children instead of decoding every node's parent (old 15 s stage).
- Anchor lab (`lab.rs`, `cargo test --profile lab`): lowers policy over captured batch DBs, reports SQLite VM steps (load-invariant) + rows per plan. Base `c42c8239` vs `813cc78b`, 12 batches (1,984 Python files), rows identical:
  | Policy | VM steps before | after |
  |---|---:|---:|
  | legacy_execution_surfaces | 0.779G | 0.338G (−57%) |
  | serverplan_quality | 2.522G | 1.612G (−36%) |
  | internal_dead_code | 1.069G | 0.833G (−22%) |
  | serverplan_target_architecture | 0.470G | 0.396G (−16%) |
  | serverplan_state_tripwires | 0.851G | 0.727G (−15%) |
  | public_api_triples | 0.541G | 0.541G |
  | backend_type_strings | 0.054G | 0.054G |
- internal_dead_code no dominant stage (largest 1.2 s of 9.3 s traced stage CPU).
- Full corpus at `1e707281`, anchor selection s vs `be03e110`: target_architecture 11.6 → 11.2; backend_type_strings 1.3 → 1.2; type_safety_patterns 6.7 → 5.6; legacy 14.2 → 4.3; state_tripwires 38.7 → 29.5; quality 82.3 → 58.7; internal_dead_code 83.8 → 68.7; public_api_triples 41.0 → 41.3. Findings: internal_dead_code 4,931/4,931; all60 unchanged (1,653 actual, 1,052 false positives); tripwires same 24 source-incomplete.

### §16 Set algebra (SF:291-330)
- Shapes found via lab dumps (`RERAIL_LAB_DUMP`, `RERAIL_LAB_PER_PLAN`): (1) disjunctions were left-deep chains of binary UNIONs from an empty seed, each prefix materialized + deduped → now one compound UNION over distinct non-empty branches; kind-only branches share one posting scan with one IN list. (2) union of scope's own subsets intersected with scope again → sets record what contains them (`within`); union within what contains every branch; text predicates and subtree relations record scope (`narrowed`); intersection with known superset is no-op. (3) union containing a branch's superset is that superset (`r5 ∪ (r5 within ranges) ∪ … = r5`). (4) union of one set's kind restrictions is one restriction (`(r20 ∩ kinds{1}) ∪ (r20 ∩ kinds{5,6,7}) = r20 ∩ kinds{1,5,6,7}`, = `r20` if no other kind). Kinds 0 and MAX (error-node conventions) excluded.
- Lab, 12 batches (1,984 files), `b327af45` vs `88a48c18`:
  | Policy | VM before | after | exec CPU before | after |
  |---|---:|---:|---:|---:|
  | internal_dead_code | 0.833G | 0.265G (−68%) | 13.3 s | 3.2 s |
  | public_api_triples | 0.541G | 0.343G (−37%) | 9.5 | 5.5 |
  | legacy_execution_surfaces | 0.338G | 0.233G (−31%) | 6.9 | 3.3 |
  | serverplan_quality | 1.612G | 1.176G (−27%) | 33.4 | 23.3 |
  | serverplan_state_tripwires | 0.727G | 0.544G (−25%) | 13.2 | 6.9 |
  | serverplan_target_architecture | 0.396G | 0.318G (−20%) | 9.2 | 5.0 |
  | type_safety_patterns | 0.107G | 0.104G | 2.1 | 1.8 |
- Rows identical except tripwires plans 27 and 31: needed > **256-relation admission allowance** and were broadened to every node of their root kind (43,731 and 1,984 rows); now fit (193 relations for plan 31), select 0 and 79 rows. Lab lists broadened plans; 14 policies still broaden some: transaction_patterns broadens all 58 plans, serverplan_quality 7. Broadened plan sound but costs matcher every root.
- Remaining in quality: ~430 stage statements per batch, top 5 hold 54% of steps; vocabulary fill (regex predicates vs new terms) next, but lab overstates it (each capture starts with empty vocabulary; corpus store fills once per term).
- Full corpus at `88a48c18` vs `1e707281` (machine loaded), execution CPU s:
  | Check | anchorSelect before | after | exec total before | after |
  |---|---:|---:|---:|---:|
  | internal_dead_code | 68.7 | 21.5 | 106.4 | 54.6 |
  | public_api_triples | 41.3 | 26.5 | 49.5 | 34.0 |
  | serverplan_target_architecture | 11.2 | 6.9 | 11.6 | 7.2 |
  | serverplan_state_tripwires | 29.5 | 23.5 | 77.7 | 69.3 |
  | serverplan_quality | 58.7 | 53.7 | 65.1 | 60.4 |
  | legacy_execution_surfaces | 4.3 | 4.0 | 6.5 | 6.2 |
  | type_safety_patterns | 5.6 | 5.9 | 5.7 | 5.9 |
  | backend_type_strings | 1.2 | 1.2 | 1.2 | 1.3 |
  | all60 (300 files) | 7.4 | 5.5 | 10.2 | 7.9 |
- Findings unchanged (internal_dead_code 4,931/4,931; all60 1,653 actual, 1,052 FPs; tripwires 24 source-incomplete). quality gains less on corpus (−9%) than lab (−27% steps) → remaining anchor time not in stages touched.

---

## 8. lessons-ledger.md (LL)

### Header (LL:1-22)
- Compiled 2026-10-25, read-only. Prior work: **PR** = nera-data-layer PR 2539 tip `281f500d7c` (2026-06-27), docs `docs/design/`; **AIS** = ast-index-spike `main` + `origin/jeremybarnes/seadragon`; **TAIL** = seadragon's `TAIL_LATENCY_WORKPLAN.md`. Current: **DLRE** `rerail/Data-layer-rule-expressivity` 10-19 → 10-24 (J = journal, WP = `sql-performance-recovery-workplan.md`); **SSC** 10-16 → 10-24; **N1562** 10-16 → 10-25; **Tempo** (-sql 09-24 → 10-16; -metal 09-30 → 10-25); **Main** = `rerail/Pulling-it-all-together`.
- Status words: ADOPTED, RE-LEARNED (where/when/detour), VIOLATED, N/R. All dates 2026; DLRE section dates used (commits trail; 10-21 and 10-22 entries first committed in `5b09437c` on 10-23).

### Part A: runtime comparison (LL:26-89)
Corpora: PR = nera-data-layer `281f500d7c`, 5,078 `.py`, 83.5 MB; DLRE = `2b1504ad`, 9,010 `.py`, 137.9 MB (9,021 files incl. 11 Rust, 137,924,324 B). Growth 1.77× files, 1.65× bytes June → October. PR "projected 5,000-file" numbers = whole repo then.

Best like-for-like (LL:42-52):
| Run | Files / MB | Rules | Cold time | ms/file | ms/file-rule | Source |
|---|---|---|---|---|---|---|
| PR 500-file sample, final | 500 / 9.9 MB (506 plan files) | 101 linters | **17.99 s wall** (plan 0.46, refresh 13.67, report 3.73), 3-run median, 8 workers | 36.0 | **0.36** wall | `DECLARATIVE_LINTER_FACT_ROW_WIDTH_PLAN.md` Step 6, 06-27 |
| PR full repo | **5,073 / 82.6 MB** | all declarative | **70.6 s wall** (was 96.6), 8 workers | 13.9 | ~0.14 if ~100 linters | `cfed5a4ab6`, 06-20 |
| PR 200-file cold | 200 | all registered | refresh 8.7 s + SQL 47.8 s 1 worker; SQL 36.6 s 8 workers | ~225–280 | ~2.5 | `3cb58f01e5`, 06-25 |
| DLRE first 50 pre-SQL | 9,021 / 137.9 MB | 50 | **125.8 s CPU**, 4 workers | 13.9 | **0.28** CPU | J 10-23 |
| DLRE first 50, 10-21 build | 9,021 | 50 | 212.6 s CPU, **233.7 s wall** | 25.9 wall | **0.52** wall | `3285b48f` 10-21 |
| DLRE first 50 SQL | 9,021 | 50 | **2,507.5 s CPU** (41.8 min), wall untrustworthy | 278 | **5.56** CPU | J; WP |
| DLRE all 60 pre-SQL | 9,021 | 60 | 709.2 s CPU; failed, 23 failures | 78.6 | 1.31 | |
| DLRE all 60 SQL | 9,021 | 60 | 7,211.7 s CPU; OS real 3,516 s (controller 15,877 s); 34 failures | 799 CPU / 390 wall | 13.3 / 6.5 | |
| DLRE small control | 12 files | 10 (batch six) | Rerail 37.2 s vs upstream Python linters 4.32 s (8.6×) | | | J 10-21 |
- PR full-repo run doesn't record whether linter SQL included; mostly refresh (sibling `28219e7d99`: 96.5 of 97.9 s refresh).

Verdict (LL:56-89): claim holds in substance; "seconds" true only for 500-file sample. PR 5,073 files 70.6 s wall cold 8 workers on 06-20 (predates physicalization that took 500-file 47 → 18 s). Scaling by bytes to 138 MB ≈ 2 min for ~100 linters. DLRE SQL 41.8 CPU-min for 50 rules, ~1 h wall for 60.
- Per file-rule ratios: DLRE SQL 5.56 ms CPU vs PR 0.36 wall ≈ 15×; DLRE SQL wall 6.5 vs PR ≈ 18×; vs PR full-repo ~0.14 ≈ 40×.
- DLRE pre-SQL on par with PR: 0.28 ms CPU per file-rule (1.5× PR on 10-21 wall build); 13.9 ms/file same as PR full repo.
- Gap = self-inflicted ~20× regression from moving candidate (anchor) selection into SQLite FTS5 positional index with generated per-batch SQL: file-ingestion CPU 73 → 2,456 s; anchor-selection counter 8.4 → 2,248 s; selection cost per indexed node 50 → 115 µs as corpus filled.
- Caveats: (1) CPU vs wall (DLRE CPU 4 workers; PR wall 8 workers; narrows 15× to ~3–15×). (2) Rule sets differ (PR 101 YAML ports of 104 declared; DLRE 60 rules port subset of main's 142 checks; 99 names shared, all scripts changed; DLRE includes state-tripwires: 12.5 s cold CPU on 7 files, dominates batch six). (3) Corpora 1.65× apart; PR 500-file sample deterministic stratified. (4) DLRE reference build opt-level 1, profiling feature, system-malloc shim, loaded host; PR machine not recorded. (5) Both 60-rule DLRE references fail; WP says no clean ratio; 10-21 all-60 pass at 1,114.6 s contaminated by profiling. (6) No DLRE full-repo run after 10-23; 10-23/24 work measured on 2-observation guard; no PR full-repo run after 06-26/27 row-width work. (7) DLRE doesn't compare against PR (cites only 200-file plan SQL 36–48 s; concludes "historical SQL times … cannot establish a current Rerail bottleneck").

### Part B: all 36 lessons (LL:93-132)
Format: # — lesson. Source. Evidence. Status per branch.

1. **Storing one row per AST node is fatal; keep compact tape as source of truth.** Src: AIS `results/storage_report.md`; `DESIGN.md` S089/S021; README "Why the obvious design fails"; `820dd9d` 07-01, `abdaa4d` 07-27. Ev: row-per-node 120 B/node (~14.5× source) to carry ~6 B info; tape 6 B/node; logical encoding 8.66 B/node. Status: ADOPTED for storage everywhere (Main, DLRE, SSC, N1562, Tempo-sql). RE-LEARNED at runtime as per-node/per-tuple intermediates: DLRE decoded-property tables keyed `(fid, pre)` filled by all-node demand (positive control 1.87 → 2.50 s; data statements 72 → 550; 10-21) still in production → partly VIOLATED; SSC frame B-trees 48% of profile (`ecb0cdf2` 10-16), 43.8M frame rows (`42ab496a` 10-21); N1562 walked 3.49B rows of `rerail_nodes` (`8fec85c0` 10-16); Tempo-sql 15.7–85M result rows = 56% wall (`1a9f39565` 10-14); Tempo-metal one 24-byte task per tuple needed 1.99 GB (09-30 → 10-16, ~16 days).
2. **Anchor on rarest constrained kind from kind frequencies, not leaf, not fitted ratio.** Src: AIS `DESIGN.md` S050 (`3eb9875` 07-04); SQL_ONLY Step 4c; TAIL S290 (`5698ed6` 07-27). Ev: leaf anchor 112 s with tail timeouts; rarest anchor 33 s, worst query 1.2 s. Status: DLRE ADOPTED then corrected (estimates counted kinds instead of selected postings, 10-21). SSC RE-LEARNED (fixed 1,048,576-row estimate → 1.18M advances for 1,875 rows, `5efcb830` 10-21; rare anchors only `85b78bbf` 10-24, test-only, ~5–8 days). Tempo-sql ADOPTED (09-24) but weak: 5,663 of 5,725 tapes still decoded. Main: presence bitsets only, no frequencies.
3. **Sound filter ≠ useful filter: emit a probe/prefilter only when measured selectivity says it rejects most rows.** Src: AIS `DESIGN.md` S164, `9a52c7c` 07-31. Ev: ungated tail 47.6 → 71.1 ms; README R14b calls gating corpus-dependent. Status: DLRE RE-LEARNED ≥4 times 10-20 → 10-23 (shared scope lost path gates 3.86 → 5.69 s; `direct_events` 0.77M → 16.3M VM steps 10-22; unrelated files raised probe ticks 1 → 361 and property joins 6 → 806, 10-23 `5b09437c`). SSC RE-LEARNED (`RERAIL_SQL_FIRST_PROBE` 213× synthetic win, regressed real corpus, `5e70ced3` 10-20; removed same day).
4. **Planner statistics for recursive/materialized CTEs untrustworthy (Postgres rows=1); statistics fixes fall short.** Src: AIS S069/S153/S156; SQL_ONLY Step 3d/3f; `a6622be` 07-30, `9a52c7c` 07-31. Ev: `recursive_worktable_factor` recovers 2 of 3 queries; wrong ROWS hints make 13 queries 2.5–80× slower. Status: DLRE partly RE-LEARNED 10-20 (`reachability-spikes.md`: "Full ANALYZE did not repair the plan", "Treat statistics as part of the physical plan"). SSC RE-LEARNED as fixed `xBestIndex` estimates; fixed with forced CROSS JOIN (`235dde6a` 10-20, `14c0d580` 10-21). N1562 recursive CTEs for utility SCCs, no statistics discussion (latent risk).
5. **Real statistics via temp tables + ANALYZE loses badly to a fused plan.** Src: AIS S155 "What does not work, with numbers"; README "Rejected"; `9a52c7c` 07-31. Ev: **57–115× slower** (0.02 → 1.15 s; 0.24 → 27.8 s); 27 statements instead of 1. Status: DLRE VIOLATED (CTAS temp stages + indexes + ANALYZE: 24 relations, 22 indexes, 14 ANALYZE per state-flow execution; full statistics made SQL slower 7.22 → 8.01 s 10-20; WP "Do not remove ANALYZE merely to improve the execution count"). SSC RE-LEARNED twice (`ca06829d` 10-21; `4dcd60b2` 10-24 "forced-join, and ANALYZE variants all lose"); had 9,904 temp setup statements per job before 10-21.
6. **`AS MATERIALIZED` and naming CTEs do not bound evaluation/preparation; only physical cuts or restructured plan do.** Src: AIS S037 (`fd1aaf1` 07-02: 471 s materialized vs 15.9 s pipelined); S052/S069; "not stated in these words". Ev: 30× from S037. Status: DLRE RE-LEARNED 4 times (178 table-reference-limit errors 10-21; "Naming alone does not bound SQLite preparation work" 162.8 vs 10.9 vs 5.35 ms 10-22; 1,101 inline materialized CTEs, 68.5 MiB peak 10-23; family packing 53 MB heap 10-24). SSC RE-LEARNED ≥4 times 10-21 → 10-24 (`9207ff45` 12.26M advances; `541df6f9` "executed its atomic prefix twice, despite AS MATERIALIZED"; 65,535-reference limit). N1562 RE-LEARNED as text expansion: one clause 164 MB of SQL (`c5876491` 10-24).
7. **On Postgres forbid nested-loop and merge joins unconditionally (all joins between rows=1 CTEs).** Src: AIS S069, S175, S269; `a6622be` 07-30, `54843f2` 08-01, `1a5b109` 08-06. Ev: 37 queries faster off, none faster on; html query 120 s → ~1 s. Status: N/R in every branch (SQLite only); SQLite analog ADOPTED in SSC and DLRE (forced CROSS JOIN fences + SSC emitted-plan audit test).
8. **Sidecar tables blow bytes-per-node budget; delete them first so portable path is the only path.** Src: AIS SQL_ONLY §2.6/Step 4a; README "Rejected"; `021f138`/`abdaa4d`/`ecc702b` 07-27. Ev: sidecars 211 (SQLite) and 456 (MariaDB) B/node vs budget ~5; guard test caps 0.25 rows/node. Status: Main/SSC ADOPTED. DLRE partly RE-LEARNED, partly VIOLATED (postings sidecar +25–31%, rejected `sql-preparation-spikes.md` 10-22; span, child-field, structural-property tables remain; no bytes-per-node budget or guard). N1562 RE-LEARNED (each relation copied 35-column row, `bc7840d4` 10-24). Tempo-sql RE-LEARNED in 2 hours (Schema 11 → 12, 09-27).
9. **Batch sharing across a union scope loses (scope inflation); share only per group, only exact closures with ≥2 consumers.** Src: AIS README "Batching and sharing"; S149 (`a50ec62` 07-31); S176 (`56c785a` 08-01); TAIL S287/S306. Ev: union 0.86× with 7.27× scope inflation; per-group 1.45×; `shared_once` worth 1%. Status: DLRE RE-LEARNED 3 times (10-20 one broad scope 5.69 vs three scopes 3.86 s; 10-22 retained shared sets +15.6%; 10-24 family packing submissions 7,733 → 4,109 but CPU 1,679 → 1,855 ms). SSC RE-LEARNED (10-21 +2–18.5% CPU; heterogeneous batch 935 vs 118 ms; 10-24 adopted seadragon S287/S306 by name). N1562 VIOLATED (one statement for whole workspace; per-clause host loops prohibited). Tempo-metal VIOLATED (335 programs × 2,079 files in one evaluation; over memory cap).
10. **Measure the code actually running; confirm a path is live before trusting/rejecting its number.** Src: AIS SQL_ONLY §2.2 (default-off `--text-anchor`: 328 of 331 timeouts, `021f138` 07-27); DESIGN_HISTORY "two consecutive measurements of a feature that was not running" (S149/S176, `56c785a` 08-01); PR handover 06-16 GOTCHA stale fact store 0, 9, 16 findings (`ca2777e58d` 06-17); PR harness scope bug 27,365 vs 1,381 violations (`da037d275c` 06-26). Status: RE-LEARNED in every branch. DLRE: default-off experiment flags; "one permanent execution path" rule (`7da151b8` 10-21); profiling feature added ~316 GB rescans and invalidated 1,114.6 s reference (10-21); profiler import silently dropped statement gate (`00e04cd4` 10-23); all timings still opt-level-1 profiling builds. SSC: 17.189 s run measured "native composition behind a SQLite demand boundary" (10-20); overwritten artifact gave wrong size target (10-24). N1562: 17.189 s, −98.8% result measured deleted path (`a4339124` → `525a0124` 10-18); history file remains uncorrected. Tempo-sql: Entry 44 timings "measure ast-index-spike, not Tempo" (09-27).
11. **Generic EAV facts and one table per YAML relation are both too wide: physicalize per relation family, children included; stop writing generic entity.** Src: PR `LINTER_SKINNY_FACT_SCHEMA_REDESIGN.md` "Problem" (`df5cdfc81e` 06-19); row-width plan Step 5/6 (`d670bc58c8`, `8632e9a965` 06-26). Ev: 1.65 GB store with zero findings; 500-file wall 47.1 → 39.5 → 19.5 s; parent-only physicalization rejected. Status: DLRE cited 10-19 then RE-LEARNED (queue 577,199 JSON rows ~1.26 GB incl. 103.7 MB field-name strings, `a92c126a` 10-21; fixed with typed column batches 10-22; ~3 days). N1562 VIOLATED (generic `fact(kind, key, value)` + 35-column union; narrowing started 10-24). SSC partial.
12. **Intern strings as atoms with 32-bit keys; fix duplicate slot references before changing atom storage.** Src: PR `LINTER_SQL_FACT_STORAGE_DESIGN.md` "ID Model" (`e76569ce7f` 06-08); row-width Step 6 (`3f263865bc` 06-26). Ev: `fact_text_value` 77k atoms, 3.4 MB; `idx_lint_fact_slot_value` 8 MB. Status: Main/N1562/Tempo ADOPTED. DLRE RE-LEARNED (repeated path strings 54% (61.5 MB) of closure keys 10-20; 103 MB field names 10-21 → shared typed schemas 10-22). SSC ADOPTED with `node.atom` from PR's `fact_store.py`.
13. **Extract demand-driven: only slots selected rules need; join only demanded columns.** Src: PR skinny-schema "Demand Model" (`df5cdfc81e` 06-19); `cfed5a4ab6` 06-20; row-width Step 6 context slice (`5125122a2a` 06-27). Ev: full-repo cold 96.6 → 70.6 s; annotation rows 44,275 → 8,043; full-catalog projection regressed report to 10.1 s, demanded joins 4.9 s. Status: DLRE ADOPTED as stated "transferable lesson" (`data-layer-rule-port.md` 10-19) then RE-LEARNED ("Automatic demand removes only six of 1,993 logical columns", `f52817fc` 10-20). SSC ADOPTED, where to restrict re-learned ("restriction belongs before demand insertion" 10-21). N1562 ADOPTED (`xFilter` demand); re-learned for utilities 10-24.
14. **Each file owns its rows; edit deletes/rewrites only that file's rows; whole-graph artifacts must not force whole-repo rebuilds.** Src: PR `LINTER_EXPRESSION_FACT_QUERY_DESIGN.md` "Dependency and Invalidation Model" (`7c4469493d` 06-10); `LINTER_MEMOIZATION_AND_DEPENDENCY_PLAN.md` (`ab2e3f714c` 06-04). Ev: design rule only, nothing measured. Status: Main/Tempo-sql ADOPTED. DLRE ADOPTED for storage but edits pay global evidence reconstruction: one-file edit 121.6 s CPU on all 60 SQL vs 62.6 s before. SSC/N1562 unqualified or unmeasured.
15. **Partial physicalization or sharing is slower than none: commit to whole family/path or don't.** Src: PR row-width Step 5 (parent-only `python_call` slower); AIS S149 (shared flat 0.23× vs unshared 1.35×); TAIL S287 ("partial sharing without closure caused a 50-fold regression", `de6d285` 08-09). Status: DLRE RE-LEARNED (decoded properties 10-21; copied column waves and retained sets 10-22); decoded-property tables remain → partly VIOLATED. SSC RE-LEARNED (admitting 55 of 141 clauses slower than none, `507b5ebb` 10-21 → "no per-clause fallback" `605ed842` 10-22).
16. **Cost is repeated per-linter scans over same broad relations, but fusing everything into one statement doesn't remove repeated work; fix plans and candidate sets first.** Src: PR 200-file plan "Bucket C" (`3cb58f01e5` 06-25: SQL 47.8 vs refresh 8.7 s); AIS `LINTER_CASE_STUDY.md` §2 (`55d9157` 08-01). Ev: AIS: PR's 28.2 s → 3.7 s by fixing plans; removing rescans worth ~1%. Status: DLRE RE-LEARNED (14 rebuilds of nodes and ownership 10-20; one statement 710 CTEs 10-21; one definition repeated across 61 and 90 statements 10-23; WP `00e04cd4` 10-23 "One large statement that repeats the same expensive work does not satisfy the CPU requirement"). SSC RE-LEARNED (24k–34k statements, then packing to 22–24 executions with 2.78B VM 10-24). N1562 VIOLATED (one statement for whole catalog; blocked by 8 MiB SQL limit; 7 of 12 policies fail to compile, `fed9342c` 10-24).
17. **No Python callbacks/UDFs in SQL phase; per-row host callback was nearly all SQL time.** Src: PR handover 06-16 "The Goal"/"CRITICAL" (`ca2777e58d` 06-17) with `f80c0709e6`; AIS SQL_ONLY C1, README R2/R6 (`89ff3da` 07-18); TAIL S295. Ev: Python REGEXP ~72 ms for nine regexes vs ~1 ms scan; Rust regex 24 ns vs Python 332 ns. Status: Main/Tempo ADOPTED. DLRE ADOPTED for Python, partial VIOLATION with native UDFs and vtabs (`nera_child` per missing key, 2.86M BLOB reads 10-21; dynamic regex per term 14.3 s 10-22). N1562 RE-LEARNED (restored callback engine, 2.70B callbacks `b88a7dde` 10-16; deleted `f492b954` 10-18; ~2–3 days after Landing learned it at `697bd2bd7`). SSC VIOLATED then re-adopted (frame-action callbacks in recursive CTE `5f1960f2` 10-21; rejected 10-22).
18. **Don't replace brittle regex with brittle LIKE/SUBSTR/INSTR; use structured AST facts with exact equality.** Src: PR handover 06-16 "CRITICAL"/"KEY DISCOVERY" (`ca2777e58d` 06-17); AIS `LINTER_ARCHITECTURE.md` F4/C5 (`9b8ca2d` 08-01). Ev: forwardref port 42 then 29 findings vs oracle 33; `INSTR` silently dropped UG002; structural fact matched oracle exactly (8 findings). Status: N/R / ADOPTED by construction (regex stays native, LIKE never proposed; Tempo-sql explicit).
19. **Lower patterns to embeddable SQL fragments over `(fid, pre)` roots; caller writes anti-join, aggregate, recursion. Don't build a SQL interpreter.** Src: AIS `LINTER_ARCHITECTURE.md` §8/C4; `LINTER_CASE_STUDY.md` §4 (`9b8ca2d`/`55d9157` 08-01). Ev: 41 specs: 16 negation, 24 counting, 1 closure; API still unbuilt (`NotImplementedError`). Status: DLRE implicitly ADOPTED (bounded candidate SQL + authored relational stages). SSC ADOPTED in design, VIOLATED in execution (SQL program-counter interpreter, "A large statement can still behave like a procedural interpreter" `9207ff45`; SIM905 visits 303k states 10-24; counter-example hand-written B023 4,616 → 49 ms `a0051ae8` 10-22). N1562 RE-LEARNED (inlining 164 MB SQL; one CTE per operator 8,397 CTEs 10-24; still open). Main/Tempo-metal: SQL runs no rules. [SSC file: settled on SSC branch by c17d2e47.]
20. **77 of 99 linter rules need only single-file syntax; partition by file, reserve global machinery for rest.** Src: AIS `LINTER_ARCHITECTURE.md` §4 (`9b8ca2d` 08-01). Ev: 12 rules cross-file, 7 repository state, 3 policy-only. Status: DLRE ADOPTED (`partitionBy: "file"`, `data-layer-batch-six.md` 10-20), re-learned at edges (file gate expanded one global clause to whole repo `4def200d` 10-21). Tempo-sql ADOPTED (105 file-regex rules, 80 AST).
21. **Expose node extents (G1) and child ordinals (G2) as columns; let caller join instead of closures or syntax extensions.** Src: AIS `LINTER_ARCHITECTURE.md` §7 (`9b8ca2d` 08-01). Ev: G1 needed by 16 specs, G2 by 15. Status: DLRE RE-LEARNED now ADOPTED (recursive descendant closure 1.1M rows = 27% CPU 10-20; interval join 0.99 ms vs 267 ms 10-22; ~2 days). SSC/Main/Tempo ADOPTED. N1562 extents only.
22. **Ordered sibling semantics need an NFA (recursive CTE over pattern position + cursor), not independent `cpos <` joins.** Src: AIS `bench/AST_GREP_SQL_GAPS.md` P0 (main only, `4598600` 09-21). Ev: rust-use 15,783 vs 15,285 with joins; exact with NFA. Status: Tempo-sql RE-LEARNED (09-24). SSC first "Not adopted" (10-19); built in substance 10-22 (`ffc685ce`). Tempo-metal GPU analog (10-21). DLRE N/R (ordering in Rust matcher).
23. **ast-grep `stopBy` default is `neighbor`, not `end`.** Src: AIS `bench/AST_GREP_SQL_GAPS.md` (`4598600` 09-21). Ev: `python-function` 5,366 vs 56,465 roots; exact after fix. Status: Main/DLRE ADOPTED (`code-rule-dsl-reference.html`, `042d82eb` 10-08). Tempo-sql RE-LEARNED. Tempo-metal RE-LEARNED 10-20, fixed same day. SSC related (`stopBy: end` expansion 10-20).
24. **Comments are tree-sitter "extras": carry `is_extra` flag per kind (zero bytes/node) so wildcards skip them.** Src: AIS `bench/TAPE_AST_GREP_RECONSTRUCTION.md` (main only, `9449260` 09-22). Ev: 0 vs 1 matches across a comment; no kind varies in extra status over 18.5M nodes. Status: Main/DLRE/SSC/N1562/Tempo-sql ADOPTED; Tempo-metal RE-LEARNED 10-20, fixed same day.
25. **Restoring a cached tape document is 20–26× faster than reparsing.** Src: AIS `bench/TAPE_AST_GREP_RECONSTRUCTION.md` (`9449260` 09-22). Ev: 20.06× median, 26.19× on largest files, 0 mismatches. Status: ADOPTED in Main, DLRE, Tempo-sql. Parse ~11 s of DLRE's 2,507 s → not the bottleneck.
26. **Fixed deadline/budget does not scale with corpus size; acceptance must be structural (work ∝ anchors and results, not total nodes).** Src: AIS README Status "Correctness scales; the deadline does not" (`28810b7` 08-01); TAIL "Structural definition of solved"/S294 (`de6d285` 08-09). Ev: at 59.5 MB p50 103 ms, p99 5,006 ms; target corpus 21× larger. Status: DLRE RE-LEARNED 10-21 (full-snapshot-only failures: 32 MB input budget, 64 MiB arena (889 failures), 34 work-allowance failures; unresolved WP R6). N1562 RE-LEARNED (126,142 budget failures 10-16). Tempo-metal VIOLATED (fixed 2e9-byte cap).
27. **Per-query/per-partition precomputed indexes rejected: indexes query-independent, built once.** Src: AIS README R14b (`d88209d` 08-01); SQL_ONLY "Rejected approaches" (`021f138` 07-27); TAIL S292. Ev: temp node table 5000 → 9 ms per query but fails at 7.55M nodes; fitted predicate table hid a wrong answer. Status: DLRE RE-LEARNED 3 times (695 field-index rebuilds over 31.4M nodes 10-21; 88 per-partition builds 10-21; 1,588 per-selector `CREATE INDEX` → 4, `75a39700` 10-24). Tempo-sql RE-LEARNED in 2 hours. Main/N1562 ADOPTED.
28. **Statement count, SQL size, VM steps and self-reported durations are proxies; only end-to-end CPU and wall on the real corpus count.** Src: PR `LINTER_FACT_FIRST_HANDOFF.md` (`7c4469493d` 06-10: reported 3.4 s vs 79 s wall); AIS S225 (`953ae50` 08-03); README "Answer size … does not predict" (`28810b7`); TAIL. Ev: PR reported 2.5 s vs 51.3 s wall; AIS three counters small while profiler showed `likeFunc` 22.8%. Status: DLRE acknowledged then VIOLATED in practice (WP `00e04cd4` 10-23 "Counts alone cannot establish recovery"; 10-23 → 10-24 driven by 100-statement gate on 2-observation guard: R5-B −38.9% submissions for −0.3% CPU; admission −190 submissions for +2% CPU; full repo not remeasured). SSC RE-LEARNED (72 executions met budget while CPU 5.1 → 14.9 s, `5f1960f2` 10-21, review `e5783859` 10-22; "SQL size alone is not a cost model" 10-24; ~3–4 days). N1562 RE-LEARNED and repeating (statements −94.6% for runtime −4.6% 10-16; since `bc7840d4` 10-24 ~60 "compact SQL" commits with no runtime benchmark).
29. **Keep diagnostics when removing a gate; change one variable at a time.** Src: AIS SQL_ONLY §2.3; RECONSTRUCTION "The corpus doubled" (`946c2ee` 07-19). Ev: 331 failures unclassified; doubling to 2 MB and flag off together hid cause. Status: DLRE partly RE-LEARNED (10-23 gate removal needed restoring).
30. **A small or fresh store hides volume pathologies: test at full-suite volume.** Src: PR `lazy_architecture_yaml_regression_analysis.md` "Combined-Store" (`f8c5ebc437` 06-25). Ev: 15 ms isolated vs 10 s timeout on combined store. Status: DLRE RE-LEARNED (3.3× and +363% regressions visible on 12-file controls 10-21; 19.9× only at full scale 10-23).
31. **Use 3-run medians, judge slices on structural counters, expect page-size noise < ~2%.** Src: PR row-width Step 3 / path slice (`1ed30f8551`, `3ee9e99233` 06-26). Ev: outliers 31.6 s and 44.9 s vs median ~20 s. Status: DLRE ADOPTED (5-run medians on baseline).
32. **Store anonymous nodes; parity depends on them.** Src: AIS S040 (`307e1b2` 07-03). Ev: 210 more shapes translatable at 1.54× nodes. Status: ADOPTED (tape stores all nodes).
33. **Excess relation construction is the common tail-latency fault; removing it alone not sufficient.** Src: TAIL S280–S289 (`de6d285` 08-09). Ev: S285 12.55 → 32.11 s; S289 total down only 5.7%. Status: DLRE/SSC RE-LEARNED (1.36M materializations; 1,101 CTEs; frame expansion).
34. **Packed, query-independent child arrays are the selected design point (S303).** Src: TAIL S296–S303 (`247e628` 08-10). Ev: median 766.8 → 219.1 ms at 4.63 B/node. Status: DLRE RE-LEARNED (field postings cut child examinations 99.67%, 427.8M examinations, 10-21).
35. **Wrong or rows=1 estimate on unmaterialized tiny candidate joined to broad relation blows up; pin join order.** Src: PR `lazy_architecture…` "Combined-Store"; AIS S069. Ev: 10 s timeout → 0.094 s. Status: DLRE/SSC RE-LEARNED (CROSS JOIN fences 10-22, 10-23; SSC 10-20, 10-21).
36. **Physicalizing relations exposes wrong semantics hidden by generic, zero-defaulted fields.** Src: PR row-width import and annotation slices (`5125122a2a`, `8723da96fa` 06-27). Ev: 5 FPs removed; `type_safety_patterns` 0 → 161 real findings. Status: N/R, DLRE's parity harness covers same risk.

### Narrative (LL:136-167)
- PR 2539 + AIS cost ~two months (06-04 → 08-11); consistent conclusions: compact query-independent tape; narrow demand-driven facts per relation family; anchor on rare kinds, gate every probe on measured selectivity; rules as SQL fragments caller composes, never a SQL interpreter; accept only end-to-end time on real corpus.
- Current branches adopted storage half (tape, interning, `is_extra`, `stopBy: neighbor`, file ownership); re-learned execution half, often several times a week.
- Part A framing: PR whole repo 70.6 s wall; "seconds" = 500-file sample; DLRE pre-SQL on par; 20× regression (125.8 → 2,507.5 s CPU) entirely from moving candidate selection into SQLite FTS5 + generated per-batch SQL; problem = SQL path regressed ~15–40× per file-rule vs PR.
- Most important lessons for direction: (1) proxy-metric trap #28 — direction needs full-corpus CPU and wall gate first; (2) temp tables + ANALYZE lose (#5, 57–115×) — DLRE clearest live violation; (3) probe/anchor selectivity (#2, #3, #35) — FTS5 anchor path = ungated-probe failure; (4) fragments not interpreter (#19, #16) — B023 94× shows intended shape; (5) union sharing loses, share exact closures only (#9, #15) — N1562 opposite; (6) demand-driven narrow facts (#11, #13); (7) measure what is running (#10).
- Most expensive re-learnings: Tempo-metal per-tuple task rows ~16 days; DLRE FTS5/SQL anchor path (3.3× visible 10-21, 19.9× ≈ 40 CPU-min 10-23, unrecovered); SSC and N1562 statement-count detours 3–4 days each (N1562 still in it); N1562 callback revival 2–3 days.

---

## 9. findings-contract.md (FC)

Status: proposed as decision 0001 (FC:3). Loosest interface that still lets end-to-end task work; deliberately not a drop-in replacement for native output; strictness beyond any consumer's need dropped; integration normalises (FC:5). Order, multiplicity, witness choice, spans, metadata, execution steps, SQL statement split all free; engine work may not be spent reproducing them (FC:7).

### §1 What the engine returns (FC:9-26), per evaluation (rule set × file set, snapshot or edit)
1. **Set of finding keys**: tuple (`ruleId`, `code` (rule may have several), `path` (absent for repo-level findings, e.g. missing required file), `line` (1-based primary location; absent for file/repo-level)). A rule may declare one more field: `column` (one data-layer check does; some language packs score line+column) or `tag` (explicit category). Nothing else is identity.
2. **Coverage outcome per (rule, file)**: `Complete` or `Unknown(reason)`; reasons include budget, parse failure, unsupported construct, timeout. File-independent rules: one outcome per evaluation.
3. **Optional payload per key**: any of span, message, captures, fix, clause id, evidence; all optional, content free within §3.
- Keys in any coordinates mappable to a line: byte offset, span, (document, preorder node) on tape. Embedded documents map through source map to host line. Mapping is integration's job.

### §2 Guarantees (FC:28-38)
| Guarantee | Statement | Needed by |
|---|---|---|
| Soundness | every confirmed key is a finding under declared semantics | steer text and enforcement (`code-rule.ts:1510-1565`); audit history |
| Completeness where Complete | for every (rule, file) `Complete`, every key returned; otherwise say `Unknown`; never silently fewer | enforcement, missed-findings digest (`runtime.ts:876`) |
| Deterministic set | same rules + inputs → same key set and outcomes | change detection (`code-rule-enforcer.ts:1100`), audit history, incremental |
| Subset evaluation | caller can evaluate subset of files down to one edited file; repo facts from snapshot | per-edit checks |
| Payload on demand | message and fix (if defined) producible for any key, by engine or rehydration | steer text, rule-draft CLI (`cli/rule-draft.ts:342`) |

### §3 Explicitly not required (FC:40-55) — none may be asserted by parity or relied on by consumers
- Order (set; consumers sort). Multiplicity (duplicates collapse, incl. oracle's). Witness choice (incl. anchors, first alternative). Span precision (need only cover primary line; edited-range filter works on whole lines, `code-rule-enforcer.ts:271,1199`, `diffLines`). Message text (`$NAME` from any valid witness or template later). Captures, variables, evidence, engine ids (no external consumer; `code-rule.ts:1383-1395`; UI reads only modes and counts). Clause structure (consumer-needed category must be declared `tag`, never from `clauseId`; `conformance/data-layer/replay.ts:183` defect). Operational semantics (work allowances, step counts, memo cache-fill vs replay, depth peaks, evaluation order, short-circuiting, first alternative, statement/SQL shape); budgets are resource limits → `Unknown` for (rule, file), never change which keys are findings; ordered meaning ("report A unless B matched first") stated declaratively as priority/exclusion (anti-join). Which files end Unknown (performance metric with own threshold). Oracle quirks (accepted difference with reason; not reproduced). Over-approximation inside engine (a stage — SQL prefilter, GPU candidate pass — may emit unconfirmed candidate keys if a later stage confirms/discards before final).

### §4 Integration (FC:57-66): one shared consolidation function for every caller
1. Normalise locations to (`path`, `line`[, `column`]) using tape line table and source maps. 2. Collapse duplicates; differing payloads → fixed rule: smallest span start, then smallest message. 3. Apply suppressions (`rerail-ignore`, tool comments) and rule scope unless engine did (twice harmless). 4. Rehydrate payloads: for keys missing needed payload, run native per-file matcher for that rule/file/line, take its witness → batch engine only produces keys. 5. Presentation: `mode` from rule; stable `id` = hash of key; display order (path, line, column, ruleId, code). 6. Diff by key (added/retracted, change detection, audit history on key sets).

### §5 Parity (FC:68-73)
- Agree when key sets equal (with declared extra fields) on every (rule, file) both sides report `Complete`. Unknown pairs excluded, counted separately. Accepted differences listed with reasons and counted. Current harnesses compare multisets of `[path, line, code]` or `[ruleId, path, line, code]` (`replay.ts:176-186`, `suite.ts`, `incremental.ts`, pack scoring script) → move to sets and declared tags.

### §6 Integration changes implied (FC:75-83)
- `code-rule-enforcer.ts:1100` ordered `JSON.stringify` → key sets. `audit-history.ts:20` keys on clause id + full span JSON + code → re-key on finding key (migration). `native/src/code_rule_dsl/eval/output.rs` ids/`added`/`retracted` from witness bytes → move to consolidation. `replay.ts:183` category from `clauseId` → `tag`. Port bar (`data-layer-rule-port.md:83`, `:264`) replaced by §5.

### §7 Consequences (FC:85-89)
- Batch/SQL executor may return keys only; payloads via rehydration only for shown findings. No executor needs memo replay, allowance accounting, first-match ordering. About half of DLRE authored SQL exists only to reproduce oracle identity (`row_number()` dedupe, per-entry-point `$option` modes) → unnecessary (categories A, J).

---

## 10. authored-sql.md (AS)

Status: proposed as decision 0002 (AS:3).

### Decision (AS:5-20)
- Rule document carries no SQL, no code, no expression strings evaluated as code; rules state wanted relations with declarative operators over named relations; engine decides computation.
- `relational` authored fields frozen now, removed once the 10 rules are re-expressed: `query`, `stages[].query`, `stages[].indexes`, `stages[].deferred`, `rx_*` UDF surface. Executor may survive only as internal lowering target.
- Non-declarative computation → engine producer behind a relation contract (reviewed engine code).
- Why it appeared: port bar required exact reproduction of imperative Python oracles; authored SQL was the escape hatch; under findings-contract bar is gone; ~half (A, J) existed only for it.
- Incompatible because compiler cannot see inside it → cannot choose statistics-driven shape, share across batch, push down behind vtab, bind to GPU. Also where DLRE runtime went: **733 of 760 CPU samples in batch six in relational evaluation**.
- Scope: DLRE `jeremybarnes/Data-layer-rule-expressivity` at `d3470da2` as merged into PIAT (AS:22).

### §1 Mechanisms (AS:24-47)
Exactly one SQL-bearing clause kind, `relational`:
| Mechanism | Author writes | Executor |
|---|---|---|
| `relational.query` | final `SELECT` producing findings (path, start_byte, end_byte, identity fields, optional code) | `relational.rs` (`evaluate` :975, `evaluate_batch` :999, result read :822); schema `code-rule-schema.ts:1497-1524`, type `code-rule.ts:189-207`, `CLAUSE_KINDS` `code-rule-schema.ts:72`; compiler `compile/relational.rs:30` from `compile/mod.rs:546`, validates statements against empty schemas before any source read |
| `stages[].query` | ordered materialized `SELECT`s (joins, windows, `GROUP BY`, `WITH RECURSIVE`); each a temp table | `relational.rs:265` `create_stage` (`INSERT INTO`/`CREATE TEMP TABLE … AS`) |
| `stages[].indexes` | physical indexes on a stage | `relational.rs` / `relational/schema.rs` |
| `stages[].deferred` | column → SQL expression evaluated lazily through a view | `relational/deferred.rs` |
| `stages[].reachability` | declarative closure (roots, edges or syntax, batchSize, graph: directed\|forest); edge/root relations are authored SQL stages | `relational/reachability.rs`, `relational/forest.rs` (native) |
| `relational.syntax` | request native CST node table (columns, fields, excludeKinds, edges, ownership, owners); declarative, no SQL; native pushdown of what rules re-encoded in SQL | `relational/syntax.rs`, `relational/syntax/packed.rs` (vtab) |
| `relational.references` | native declared-reference edges (`reference_edges`), sibling-link and constructor-field conventions | `relational.rs` / inputs |
| `relational.inputs` | named AST evidence queries with `project` maps (declarative; become SQL tables) | `compile/evidence.rs`; `relational/inputs.rs`, `relational/insert.rs` |
| SQL dialect | SQLite core + `json_each`/`json_tree` + UDFs `rx_matches`, `rx_extract`, `rx_replace` | UDFs `relational.rs:~127-190`; whitelist authorizer `relational/authorizer.rs` |
| `$option` switches | `{"$option": "entryPoint"\|"oracleAnalyzer", "cases": {…SQL…}}` | `compile/options.rs` |
- Declarative, no SQL: `forbid`, `require_each` (subject/evidence anti-join), `budget` (group_by + count), `select_when` (semi-join), `define.report_when`; `Predicate` AST (all/any/not/eq/lt/…); `NameClosure` (seeds/edges/extends/partition_by); AST rules with `calls`/`resolves`/`binds`, regex state machines, `transform`.
- Relational engine ~7,000 lines Rust (`relational.rs`, `relational/**`, `compile/relational.rs`); rule JSON produced by ~2,400 lines Python generators (`scripts/dsl/data_layer_six/*.py`, `scripts/dsl/data_layer_relational_five.py`) — SQL is generated, not hand-written per rule.

### §2 History (AS:49-63)
- Introduced `7dd15e1f` (2026-10-19) "Add bounded project relations and declared reference contracts"; then `6cb8dc06` (batch six port), `d97e1239` (batched reachability), `f52817fc` (deferred SQL facts, native forest), `c4bd76bf` (shared syntax facts); 19 commits touch the relational engine.
- Rationale (`data-layer-batch-five.md:86-99`, `data-layer-engine-gaps.md:144-158`): "Actual ports proved two further execution shapes… Relational clauses join projected source facts, group exported families, and compute recursive reachability with native reference edges."
- Governing requirement `data-layer-rule-port.md:83` "Model the exact state tracked by the oracle"; admitted "as exact only after its diagnostics and coverage match the oracle" (`:264`); batch five "No oracle source, expected findings, or existing AST work limit changed to obtain parity" (`batch-five.md:117`); root-cause example "Dead-code parity had 42 extra and one missing diagnostic | Plain imports vanished through SQL null arithmetic…" (`:114`).
- Consequence: oracle scripts are imperative analyzers (scope tracking, import resolution, class hierarchy, call graphs, taint) → exact parity demanded a Turing-ish escape hatch = SQL; performance work then on SQL materialization (`batch-six.md:148`: "733 of the active worker's 760 samples in relational evaluation. Of these, 716 included SQL stage materialization"); "subset takes >20 min" path.
- Diagnosis: symptom of over-strict parity contract (user's item 3), not a missing declarative primitive.

### §3 Census (AS:65-108)
- **10 of 60** rules use `relational`, **52 clauses**; **191 evidence inputs, 431 SQL stages, ~926 KB authored SQL text**.
- Per rule: serverplan-state-tripwires 32; serverplan-steps 7; serverplan-quality 6; 1 each: execution-authority-bypass, impl-direct-execution, internal-dead-code, legacy-execution-surfaces, public-api-triples, serverplan-dispatch-architecture, serverplan-target-architecture.
- Also `fixtures/host/data-layer-port/profiling/{batched-sql,evidence,native-relations-metrics}.json`, `test/scripts/test_sql_variants.py`.
- By primitive (per clause; one clause may be in several; counts from stage names and query features):
  | # | Primitive | Clauses (rules) | What |
  |---|---|---|---|
  | A | output dedupe / pick-first only | 28 (1) | 27 state-tripwire clauses no stages; SQL only `row_number() OVER (PARTITION BY path,line,scope,terminal ORDER BY start) = 1`; `impl-direct-execution` same per line |
  | B | count/threshold per group | 11 (7) | `count(*) >= 2` per file/project; `HAVING count(*)=1` unique resolution; `group_concat` qualnames |
  | C | CST re-encoded as rows | 15 (5) | generators (`ast_relations.py`) capture every node with 17 optional field captures; SQL rebuilds `nodes`, `edges`, `fields`, `descendants`, sibling `ordinal` (`row_number`), `ownership` (recursive CTE); `relational.syntax` is pushdown of exactly this |
  | D | lexical scope/qualname/ownership | 13 (6) | nearest enclosing fn/class by interval containment (`p.__start<f.__start AND p.__end>=f.__end ORDER BY … LIMIT 1`); recursive qualname (`a.<locals>.b`); module name from path via regex |
  | E | import/name resolution | 8 (5) | tokenizing `import` text with recursive CTE over `substr/instr`; relative imports; alias maps; `resolved_names` fixpoint; re-export via `__all__` |
  | F | class hierarchy closure | 5 (3) | `base_edges` (base name via imports, then same module, then global unique); recursive `hierarchy` with cycle guard; MRO-ordered `public_methods` |
  | G | call graph / reachability | 5 (4) | call candidates → unique functions (`HAVING count(*)=1`); recursive `reaches`/`flow`; dead-code roots (exports, public methods, dynamic imports); dispatch through overrides |
  | H | value/data flow | 11 (5) | taint over expression edges, receiver token propagation, callback carriers, string-value constant folding, branch variables, self-stores |
  | I | authored allowlist literals | 6 (5) | `json_each('[[path, qualname], …]')` signed exceptions; `p.path NOT IN ('…py', …)` |
  | J | oracle-mode switching (`$option` entryPoint/oracleAnalyzer) | 50 (8) | different SQL/inputs per oracle entry point and analyzer flag |
- Raw SQL features (clauses (rules)): `row_number` 47 (9); anti-join 20 (10); regex UDF 18 (9); string tokenizing 18 (7); `WITH RECURSIVE` 17 (8); interval containment 13 (7); path literal tests 13 (8); identity string building with `||` 13 (5); aggregates 12 (7); JSON literal tables 5 (4).

### §4 Declarative replacement per category (AS:110-144)
Principle: rule states what relation; engine owns how; semantic facts are engine-produced relation contracts computed once per file/snapshot, shared; lower to SQL fragments or vtab pushdown so compiler keeps stats-chosen shapes, batching, GPU bindings.
| Cat | Construct | Exists? | Lowering |
|---|---|---|---|
| A | none in rule; findings are a set keyed by declared identity (existing `key`); collapsing in integration | mostly; delete SQL | `DISTINCT` on identity or consumer |
| B | `budget` (group_by, count lte/distinct); `define.report_when.count.gt`; extend `gte`/`min`, `scope: file\|repository` | mostly | `GROUP BY … HAVING` |
| C | delete; AST rules (`inside`/`has` with `stopBy: end`, `field`, `nthChild`, `precedes`/`follows`); structural joins via engine `node`/`kids`/`ancestor` contracts; containment = interval test on tape extents (AIS G1) | yes | native matcher per file or `node` vtab |
| D | engine relation `scope(file, node, owner_fn, owner_class, top_fn, qualname)` as capture projections (`capture.X.qualname`, `capture.X.owner`) and `inside: {owner: function}` | partly (`syntax.ownership/owners` seed) | per-file native producer behind contract |
| E | `resolves`/`binds`/`calls {graph: references}` + declared-reference graph, backed by `symbol_ref(file, node, target)` + `import(module, name, target)` | largely | repo-scoped relation; incremental by dependency cone |
| F | `NameClosure` or engine `class_base` + closure operator | partly | recursive CTE with cycle guard, or native forest |
| G | first-class `reaches: {from, via: calls\|references, to}`; native `reachability` stage exists, needs declarative roots/edges | engine yes, surface partly | recursive CTE or native batched closure |
| H | genuine residual: (a) engine def-use/value-flow relation with `flows: {from, to, through}`, or (b) loosen to site-level candidates; AIS declined as "a resolver, not an index" for 2/99 specs (§7 G3) | no | (a) native producer behind contract |
| I | policy data: `allowlist:` relation from declared (signed) artifact joined by `except: {allowlist, on: [path, qualname]}`; path lists in `paths` scope | no surface; host has signed-allowlist verification | plain table join |
| J | delete; keep only product-option `$option` cases | n/a | n/a |
- General composition: small closed relational algebra over named evidence relations with typed column refs + `Predicate` AST (`require_each`, `select_when`, plus `join`/`union` node); no SQL text.
- PR 2539 (`DECLARATIVE_LINTER_SQL_ONLY_BOUNDARY.md`): generic framework-owned facts (`python_definition` with qualname, bases, decorators; `python_function_body_call` with resolved symbol; imports; parameters) + YAML operators (`from`, `join`, `left_join`, `recursive`, `group_by`, `aggregate`, `project`, `union`, named queries); restricted `expr:` dialect; signed allowlists; one SQL statement per query; scope and name resolution as ingestion facts, never per-rule SQL. "Process Failure" warns against "Python prepares convenient rows, YAML queries them" and rule-shaped relations. DLRE failed in mirror direction.
- AIS (`LINTER_ARCHITECTURE.md` §5, §7, §8, §10): 77 of 99 single-file syntax; anti-join (16 specs), counting (24), closure written by caller around fragments; real gaps containment-at-depth (extents), child ordinal, data flow (declined); "caller" = engine compiler.
- **Update (decision 0005)**: D–H now fact programs + named derived facts in `rerail.facts/v1` (`spec.md` §21): D → `py.scope`/`py.def`; E → `py.import_res`; F → `py.subclass_closure`; G → `py.call_edge(_local)` + `reaches`; H → `py.flow_edge` + `py.local_flow`. C deleted outright (see heavy-inputs §1).

### §5 Recommendation and risks (AS:146-172)
1. Freeze then remove `relational` authoring: no new rules; schema marks `query`, `stages[].query`, `deferred` deprecated; executor only as internal lowering target; then delete `query`, `stages`, `deferred`, `rx_*` UDFs, authorizer.
2. Replace parity contract first: A and J (28 + 50 clauses) exist only for exact oracle identity/modes; under set contract A disappears: **27 of 52 clauses become plain `forbid` with identity key**.
3. Promote generic parts to engine contracts: existing native work (`syntax`, `ownership`/`owners`, `reference_edges`, `reachability`/forest); new contracts `node`/`kids` (C), `scope` (D), `symbol_ref`/`import` (E), `class_base` (F), reachability operator (G); computed once, shared, bound to SQL or vtab under two-binding rule.
4. Minimal declarative surface: capture projections for scope/qualname; `reaches`; `budget` extensions; `except`-allowlist relation; small join/union node with `Predicate` AST.
5. Re-express the 10 rules by rewriting generators, not JSON; measure against loosened contract.
6. Residual H (11 clauses in state-flow, steps, dispatch): scoped engine value-flow relation behind a contract, or candidate sites with looser semantics accepting recall difference; decide per rule with numbers; not an argument for authored SQL.
- Risks: engine semantic facts become a versioned schema (keep generic; suspect any relation named after a rule); loose contract can hide regressions unless normalization explicit and tested; data-flow rules may lose recall; removing `$option` oracle modes changes entry-point attribution.

---

## 11. Cross-source

### 11.1 Contradictions and inconsistencies between sources
1. **Recursion on CPU vs Metal.** FM:117 layer "Recursive set stage (CPU only)" and HRP:291 "recursive utility can only run on CPU or JIT" are both flagged superseded (FM:5, HRP:5): under spec §8 monotone recursion runs on Metal as GPU-scheduled waves, no CPU fallback. HI:224 (E17) says "one reverse-preorder pass on CPU; deepest-first depth waves on Metal".
2. **Share of `sep` time in relational inputs.** FM:19, FM:203: top ten relational inputs ≈30% of structural work. HI:9: all 191 relational inputs = 146.6 of 171.5 s (~85%); HI says FM undercounts.
3. **Relational clause classification.** FM:136-141: 29 per-file join / 11 EXISTS / 7 syntax table / 4 cross-file / 1 reference graph. HI:241-246: 44 fully IR / 1 + fact column / 3 + SQL tail / 4 cross-file; 48 of 52 per-document in IR. AS:153: 27 of 52 become plain forbid immediately (category A). HI:241 and HRP:44 count 28 dedupe clauses; AS:84 says 28 clauses in category A but 27 state-tripwire clauses have no stages, plus `impl-direct-execution` — while listing "(1)" rule.
4. **Text-regex clause count.** FM:12: 113; FM:85: 112; HRP:132: 111.
5. **Distinct regexes.** HRP:129: 376 distinct identifier regexes. FM:108: 816 distinct regexes (scope of count differs: identifier vs all).
6. **AST query/program counts.** HRP:73, HRP:316: 588 AST programs (lower bound). FM:86: 588 AST queries, 535 active.
7. **Distinct scopes.** HRP:73: 33 distinct file scopes (one serves 13 rules, another 10). FM:14, FM:109: 42 distinct scope groups (largest identical groups 6, 6, 4, 4).
8. **Fact-family zero-cost queries.** FM:39: 68; FM:289: 73 (= 68 rule queries + 5 relational inputs per FM:118).
9. **Cold parse CPU.** HRP:68: 34.8 s (1,232 ns/node), whole gate 35 s. FM:253: 34–60 s, measured 38–50 s. WS:33: DLRE parse counter 11 s for first-50. LL:121: parse ~11 s of DLRE's 2,507 s. M1:52: steps parse 0.56 s (in-scope only).
10. **Native baseline matching cost per first-50.** HRP:316 and WS:33: native AST matching 58.2/58 s for 50 rules. FM:30: native baseline 125.8 s matches `sep` (147–293 s predicted). 125.8 s is the whole pre-SQL first-50 cold CPU (LL:47), not matching alone.
11. **Per-edit costs differ by scope of measurement.** HRP:17, HRP:339: 0.02–2 s, ~0.3 s expected (estimate). FM:282-283: AST matching only p50 2.9 ms, p99 189 ms, worst 1.3 s. SSC:63: whole apply 180–203 ms (catalog). M1:32: measured warm edit 25–32 s CPU internal-dead-code, ~18 s quality, 4.3–6.8 s public-api-triples on current engine. HRP:26: DLRE first-50 edit 40.2 s; LL:110: one-file edit 121.6 s CPU on all 60 SQL vs 62.6 s before. WS:121: Y1 0.73/2.4 ms p50/p95; Y2 0.07/1.5 ms. SSC:140 says edit figures are not comparable unless scope is named.
12. **Expected per-rule cost vs measured current cost.** HRP expects state-tripwires 3 s, quality 4.5 s, dead-code 2.1 s (cold CPU, excluding shared costs); M1 measured 177–190, 346–413, 161–195 s on the current engine (includes ingest/fixed costs; M1:80). SF:240-241,320-324 at `88a48c18`: execution-only tripwires 69.3 s, quality 60.4 s, dead-code 54.6 s (construction separate).
13. **Index/publication framing.** SF:22 says per-file publication of every node into a positional index "should not be on the path of local rules"; SF:165-168 withdraws the opt-in conclusion: the anchor index is mandatory (decision 0006) and built every time; construction is reported separately.
14. **Where anchor-selection cost sits.** WS:33: DLRE anchor selection 537 s of 958.6 s. LL:76: anchor-selection counter 8.4 → 2,248 s. M1:49: steps anchor selection 20.7 of 25.0 s ingest. SF:243: anchor selection still largest execution cost; matching never > 16 s. HRP's cost model has no anchor-selection row (it assumes tape-direct execution with kind index 0.3–9 s).
15. **SSC structural predicates in SQL.** SSC:110-113: SSC lowers structural predicates to SQL over per-node `node` vtab for 17 clauses, contradicting spec §7/§9; SSC file's verdict: fallback only. AS:118 lists "`node` vtab" as a lowering for category C ("Native matcher per file, or the `node` vtab"). HRP:236 says node copy becomes "engine's node relation, read straight from the tape as a virtual table". SF:247: a node table or a parent for every node was rejected (spike).
16. **Capture equality across matches.** HI:17, HI:231: spec §7 and spec-fusion §6.6 say it stays in SQL; HI proposes reversal (within one document → IR via `EXISTS`+`TEXT_EQ` or E15). FM:138 still lists "Join on captures inside one program, or an SQL join over exported rows".
17. **Negation through recursion.** HI:224: E17 requires amending spec §6.8 / spec-fusion §6.8, which currently reject any negation through recursion.
18. **Tries/root-kind dispatch.** FM:20, FM:196: dispatch-only fusion buys nothing (data-layer). SSC:139: on Python catalog, zero-match fragments on common kinds take 39% of top-10 CPU — case for root-kind dispatch and tries; claim must be scoped per ruleset.
19. **Narrow inputs in the current engine.** SF:201: narrow inputs shrank the relational allowance and caused "interrupted"; after the fix, narrow internal-dead-code was slower (90 s vs 68 s) and narrow parameter splits changed results (+177, +23 FPs), so verified shapes were restored (SF:202-209). HI/HRP assume narrow fact inputs are cheaper.
20. **Three-valued semantics.** FC (decision 0001) and SSC:126 note decision 0001 excludes three-valued semantics; the frontend still produces logical unknowns; SF:82, SF:145 show `value` tests make clauses three-valued and the prefilter was then dropped (since fixed).
21. **State-tripwires failure.** M1:35: one `source-incomplete` failure ("repository input budget exceeded") on `state-flow-project-1`. SF:238, SF:288, SF:330: 24 `source-incomplete` failures. Same rule, different counts (different runs/configurations; the sources do not reconcile them).
22. **all60 findings.** SF:288, SF:330: all60 1,653 actual, 1,052 false positives (300-file sample); SF:29-30 says "identical set" before/after. Whether the 1,052 FPs are vs oracle: the source says "false positives" without further explanation — not found.
23. **SSC commit state.** WS:94-109 describes SSC as uncommitted and recommends merging at its next commit; SSC file (dated with HEAD c17d2e47) reports it committed (e32b280b, c17d2e47).

### 11.2 Cost-model inputs a design must use

**Corpus constants (M):** 9,010 `.py` files / 137.9 MB; 28.23M nodes (18.5M named); depth mean 8, max 86; nodes/file p50 1,333 / p99 28k / max 206k; identifier 6.02M, call 1.06M, attribute 872k, string 781k, assignment 467k, function 162k, class 32.7k, module 9,010 (HRP:61-69, FM:145-160). Function subtrees sum 27.0M nodes (HRP:72). 588 programs × nodes = 6.33G program-nodes (HRP:73). Facts: defs 194k, calls 994k, imports 227k, bases 22k, assignments 411k, `__all__` 16.7k, refs 6.02M (1.18M deduped), 232k distinct names (HRP:79-88).

**Construction unit costs:**
- Parse 1,232 ns/node → 34.8 s CPU cold (M) (HRP:68); FM measured 38–50 s (FM:253).
- Restore cached tape 0.10–0.22 s gate (FM:254); restore 20–26× faster than reparse (LL:121).
- Kind postings 0.23–0.45 s (FM:255); kind index 12.4 ns/node (D) (HRP:140); tape encoding + kind index 0.3/1/9 s (HRP:314).
- Fact extraction 54.8 ns/node = 1.55 s one thread (M); data-flow facts 41.4 ns/node (HRP:90-91).
- Tape 6 B/node; logical 8.66 B/node; row-per-node 120 B/node (LL:97). Packed child arrays 4.63 B/node (LL:130). Tape ~0.24 GB for corpus (HRP:325).
- Current anchor-index publication: value-term trigger 68% of publication time (SF:174); publication 2.1 s CPU per 300 files after lock fix (SF:222).

**Execution unit costs:**
- Fused programs, simple templates: 0.3–0.97 ns per program-node (D); real heavy programs 0.3 / 1 / 3 ns per program-node (A) (HRP:139, HRP:143). M2 pass condition ≤ 1 ns/program-node (HRP:356).
- Harness: 35 ns per evaluation (FM:226); target executor 3–5 ns per evaluation (FM:33); fitted: per file 12.7 µs, candidate scan 11.3 ns/node, dispatch 13.2 ns/(node,query), range scan visit 6.4 ns, mask cell 7.4 ns, mask lookup 5.9 ns, memo hit 94 ns, `not` 273 ns, pattern 1.14 µs, `has` end 160 ns (FM:213-224).
- Per-candidate κ (p50–p90) for forbid queries: call 134–468 ns; function 0.9–5.4 µs; class 1–8.8 µs; module 8.6–17.5 µs; set-stage pass 6.4–17.3 ns/node (HI:45).
- Native AstMatcher (current, opt-1): 0.5–1.1 µs per candidate root (509 / 1,086 ns) (D from M) (SSC:71, SSC:136).
- vtab boundary + publication: ~55 ns per candidate root, ~1.2 ms per xFilter; prepare 6.32 ms for 8 statements (SSC:137-138).
- Regex: 25 ns per regex per occurrence; once per distinct name 2.1 s serial / 1.17 s combined set (M) (HRP:130-131); Rust regex 24 ns vs Python 332 ns (LL:113); whole-file text regex 4.3 GB → 9–17 s (FM:257); per-file ~0.1 s per whole-file clause, mostly I/O (HRP:132).
- Mask rule: mask only when summed scan cost over roots exceeds nodes × pass cost (FM:197).
- Metal: waves 19–22 µs GPU-encoded, ~70 µs pre-encoded with real work; set ops 0.005–10.6 ms on 5.4M nodes (D) (HRP:141-142).

**SQL unit costs (M, HRP:95-111):** insert 0.74 µs/row; real-table fact load 16.4 s (7.9M rows: insert 5.86 + dedupe 3.79 + index 6.72); vtab load ~1 s (A, unmeasured); call resolution 0.73–1.08 s (24.8 s badly worded); import resolution 0.43–0.68 s; reachability 0.80 s (17.8 s correlated subquery); anti-join 0.25 s; hierarchy closure 0.28 s; per-file-callee count 0.89 s; taint recursive query 0.49 s (native worklist 0.94 s) but load+index 10.65 s for that scope. FM relational SQL upper bound 8.5M rows × 1–5 µs = ≤ 8.5–43 s (FM:259). Pure SQL lowering 3.4G rows × 100–500 ns = 360–1,760 s (FM:272).

**Per-stage gate estimates (HRP:311-323, CPU, low/expected/high):**
- Parsing cold 35 (0 warm); tape + kind index 0.3/1/9; regex 1.5/3/10; AST matching 1.9/6.3/19; fact extraction 0.3/1/1.6; fact loading 0.5/2/30; cross-file SQL 2/5/25; data flow 1/4/20; consolidation 0.1/0.3/1 (FM: < 0.5 s for ~25k findings). **Total without parse ≈8/25/110 s; with ≈43/60/145 s; wall at 8–12 cores 8/15/40 s; memory 1–4 GB.**
- FM full-gate one core: structural fused 56–120 s measured; predictions fused 16 workers 5–17 s; faster executor + 16 cores + cached trees **1.2–3.5 s** (FM:30-33).
- HI after-rewrite estimates for the six heavy groups: 0.40–1.29 s one core (vs 139.6 s `sep`) (HI:14); per group: qc1 4–14 ms; step-contracts 0.08–0.27 s; legacy 0.10–0.37 s; 28 dedupe 0.08–0.25 s; string feeders ~0.05 s; executor-provenance 0.09–0.32 s.
- Per-rule HRP expected totals: tripwires 3 s, steps 0.35 s, quality 4.5 s, dead-code 2.1 s, public-api 1.5 s, delegation 0.15 s, transaction 0.2 s, boundary 0.8 s (≈10 s for 8; HRP:13).

**Measured current-engine reference points:**
- DLRE first-50: pre-SQL 125.8 s CPU; SQL 2,507.5 s; current 958.6 s (anchor selection 537, index prep 137, AST matching 58, parse 11); all-60 pre-SQL 709.2 s, SQL 7,211.7 s (WS:23-33, LL:47-51).
- M1 8 rules cold CPU sum 994–1,220 s (double-counts ingest) (M1:28).
- SF full-corpus execution CPU at `88a48c18`: dead-code 54.6, public-api 34.0, target-arch 7.2, tripwires 69.3, quality 60.4, legacy 6.2, type-safety 5.9, backend-type-strings 1.3 s; anchorSelect dominates (SF:318-328). Construction (at `be03e110`): 9.5–131.4 s per rule (SF:232-241).
- SF 300-file all-60: 29.7 s CPU (from 50.7 s), 1.10 GB, 0 failures (SF:30); single-rule floor ~1.7 s (SF:34).
- SSC catalog (5,725 files, 95.7 MB): native 109.53 s CPU, SQL fragments 111.79 s; matching 83.56 s (SSC:60, SSC:68).

**Gap to the 1–2 s target (as stated by sources):**
- HRP:7: expected whole gate 25 s CPU cold / ~15 s wall without parsing is an order of magnitude above target; reachable only warm (parse + facts cached by content hash) and with M3 vtab at the low end.
- FM:37, FM:276: needs all of: cached trees (cold parse 38–50 s CPU), shared evaluation, executor 7–10× faster than harness (35 ns → 3–5 ns per evaluation), 16-way parallelism, cheaper regex stage, ten heavy relational queries rewritten → 1.2–3.5 s.
- HRP:326: Metal saves at most ~6 CPU-s / ~1 s wall on the gate.
- WS:123: facts-plan concludes target unreachable "over these rules as authored"; requires local queries over one maintained reference graph (Y1 72 MB, 0.73/2.4 ms per edit).
- Current measured state vs target: DLRE 958.6 s first-50; SF execution-only per heavy rule still 34–69 s on full corpus.

**Pending measurements / open items collected:**
- HRP M2–M8 (fusion ≤1 ns/program-node; vtab facts ≤5 s; dead-code/public-api over facts; state-flow-1 native ≤2 s on 1,259 observations; qc1 rows consumed; generated_facades/1,000-sync no budget failure; one-file edit ≤1 s p95) — completion not found in these sources.
- vtab fact-loading cost unmeasured (A ~1 s) (HRP:34, HRP:343).
- Compile time for 1.2 MB rules / 174 utilities unmeasured (HRP:348); FM cold plan compile 5–6 s (FM:38); M1 compile < 0.6 s per rule (M1:31); SSC compile 129–151 ms (SSC:66).
- Cross-file data flow in whole-repo mode unmeasured (HRP:349).
- Per-function relation contract for quality data-flow-like stages does not exist (HRP:238).
- Parallel scaling assumed (FM:296); Metal on this workload not run (FM:295).
- Empty-rule baseline (fixed ingest cost) never run (M1:69, M1:80); sys-time cause unprofiled (M1:74).
- SQL-fragment path 1.8× native RSS unexplained (SSC:133).
- Statistics planning never exercised in a full run (SSC:82).
- tripwires `requireNonemptyInput` hypothesis unverified (M1:37).
- quality-contracts-1 remaining rows (SF:121-126, partly done SF:179); state-flow-project still copies tree (SF:159-161); quality remaining anchor time not in touched stages (SF:330); 14 policies still broaden plans past the 256-relation allowance (SF:312).
- Spec amendments proposed but not recorded as made: E15–E17, L1–L3, capture-equality reversal (HI); SSC's 14 spec text changes (SSC:176-210); decision candidates: three-outcome classes (SSC:186), one maintained reference graph (WS:133).
