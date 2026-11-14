# Execution profiles of the heaviest data-layer rules

Tools and raw outputs are in `../experiments/rule-profiles/`. The database files (`facts.sqlite`, `flow.sqlite`) and the corpus copy are not checked in. Figures are tagged **M** (measured on the gate corpus), **D** (from DLRE or the research docs) or **A** (assumed). All runs were single shots on a busy machine, ±20–40%.

> **Superseded statements.** §3.6(f) and the fusion "utilities" note say recursive utilities run only on CPU/JIT. That was written before `metal-enqueue.md`. Under spec §8 there is no CPU fallback: monotone recursive utilities run on Metal as GPU-scheduled waves, and anything else that cannot be stratified is rejected by the verifier or reported `Unknown(resource)`.
>
> **Gap to target.** The expected whole-gate figure (25 s CPU cold, ~15 s wall, without parsing) is an order of magnitude above the 1–2 s full-repository target. The target is reachable only warm (parse and facts cached by content hash) and with the M3 virtual-table result at the low end. See spec §19.

## 0. Bottom line

| Quantity | Low | Expected | High |
|---|---|---|---|
| The 8 profiled rules, cold CPU, without parsing | 3 s | 10 s | 45 s |
| Whole 60-rule gate, cold CPU, without parsing | 8 s | 25 s | 110 s |
| Whole gate, cold CPU, with parsing (35 s, M) | 43 s | 60 s | 145 s |
| Whole gate, cold wall time at 8–12 cores | 8 s | 15 s | 40 s |
| One-file edit, warm | 0.02 s | 0.3 s | 2 s |
| Peak memory | 1 GB | 2 GB | 4 GB |

What we compare against (D):

| Run | Cold CPU | Memory | Edit |
|---|---|---|---|
| Pre-SQL native, 60 rules | 709 s | 16.1 GiB | — |
| DLRE SQL path, 60 rules | 7,212 s | 15.0 GiB | — |
| DLRE first 50, current | 958.6 s | — | 40.2 s |
| DLRE batch six | — | — | 54–80 s |

- **Expected gain over pre-SQL native: about 10× (credible range 5–80×).**
- **Where the gain comes from:**
  - DLRE copies the whole syntax tree into SQL rows and also builds an ancestor table that is **8.2× the node count** (M). The proposal reads the tape directly and uses interval ranges instead.
  - Identifier regexes are evaluated once per distinct name, and fusion plus masks shrink the remaining matching.
- **What the proposal doesn't remove:** parsing (35 s cold), the data-flow work (category H), and the cost of loading fact tables into SQLite.
- **The biggest swing is how fact tables reach SQLite.** Copied into real SQLite tables they cost about 16 s on this corpus (M). Served through a virtual table over native data they are estimated at about 1 s (A, not measured).
- **SQLite query wording alone changes run time 20–35× (M).**
- **Metal barely matters for this gate.** It can only take the AST-matching share, about 6 of the expected 25 CPU-seconds.

## 1. Rules chosen

There is no per-rule full-repository timing, because DLRE runs rules together. I chose rules from their structure, their scope sizes on the real corpus, and DLRE's failures and batch totals.

| Rule | Files | Nodes | AST programs | SQL clauses | Why it's here |
|---|---|---|---|---|---|
| serverplan-state-tripwires | 3,095 | 11.77M | 43 | 28 dedupe clauses + 4 multi-stage (25–27 stages, data flow) | Batch six; category H data flow |
| serverplan-steps | 264 | 2.34M | 36 | 7 (16–31 stages each) | Largest rule: 1.2 MB, 5,426 operators |
| serverplan-quality | 9,010 | 28.23M | 31 | 6, including `quality-contracts-1` (54 stages) and `quality-conditional-1` (20 inputs, whole repository) | Caused the 889 producer-arena errors in DLRE's full-repository run; copies the whole syntax tree |
| internal-dead-code | 3,110 | 11.79M | 13 | 1 (13 inputs, 25 stages, 6 recursive) | Cross-file reachability |
| public-api-triples | 3,110 | 11.79M | 11 | 1 (22 stages) | DLRE failure: "relational query interrupted" |
| async-sync-delegation | 3,110 | 11.79M | 13 | 0 | AST-only rule with the most operators (5,940) and 174 shared utilities |
| transaction-patterns | 3,095 | 11.77M | 58 | 0 | Most AST programs, all sharing one root utility |
| async-sync-boundary | 2,919 | 12.54M | 63 | 0 | AST-only rule with the largest programs × nodes (790M) |

Two more rules are covered only as risk notes in §3:
- **sync-async-signatures** hit DLRE's work budget on `generated_facades.py`, the biggest single file (206k nodes).
- **grammar-callable-constructors** failed name closure.

## 2. Measured corpus facts and unit costs

### Corpus (M)

The corpus is the 9,010 `.py` files of nera-data-layer at the pinned commit, 137.9 MB. The 11 Rust inputs are excluded.

| Measure | Value |
|---|---|
| Syntax-tree nodes | **28,230,057** (18.5M named) |
| Mean depth | 7.98 |
| Nodes per file | p50 1,333 · p90 7,056 · p99 27,996 · max 206,043 |
| Parsing | **1,232 ns/node** = 34.8 s CPU, 7.0 s wall on 8 threads |
| Most common kinds | identifier 6.02M · call 1.06M · attribute 872k · string 781k · assignment 467k · function 162k · class 32.7k |

Two counts that drive the estimates:
- **Naive subtree scans are expensive.** The subtrees of all functions add up to 27.0M nodes, so one naive "function has X anywhere inside" check costs about one full pass over the corpus.
- **Scopes are heavily shared.** The 60 rules use only 33 distinct file scopes; one scope serves 13 rules and another serves 10. The 588 AST programs (a lower bound) cover **6.33 billion program-nodes** in total.

### Fact tables (M)

Fact tables are the per-symbol rows (definitions, calls, imports, references and so on) that the cross-file rules join on.

| Fact table | Rows |
|---|---|
| Definitions | 194,281 |
| Calls | 993,650 |
| Imports | 226,642 |
| Class bases | 22,061 |
| Assignments | 411,275 |
| `__all__` names | 16,740 |
| Name references | 6.02M occurrences, 1.18M after removing duplicates per file |
| Distinct identifier names | 232k |

- Extracting all of them takes **54.8 ns/node** (1.55 s on one thread).
- The data-flow facts take 41.4 ns/node.

### SQLite at real row counts (M)

| Operation | Time | Notes |
|---|---|---|
| Insert 7.9M fact rows | 5.86 s | 0.74 µs/row |
| Remove duplicate references | 3.79 s | |
| Build indexes | 6.72 s | |
| **Total cost of loading facts as real tables** | **16.4 s** | |
| Resolve calls to definitions by unique name | 1.08 s | |
| Resolve calls via local definitions and imports | 0.73 s | |
| Same, written as a join on a CASE expression | **24.8 s** | 34× slower |
| Resolve imports | 0.43–0.68 s | |
| Reachability over a proper edge table | **0.80 s** | 175k reachable |
| Same, written with a correlated subquery | **17.8 s** | 22× slower |
| Dead-code anti-join | 0.25 s | |
| Class hierarchy and its transitive closure | 0.28 s | |
| Count per file and callee | 0.89 s | |

On this corpus, DLRE's path-string enumeration of call-graph sinks finds 5,853 rows against 5,687 with plain sets, and both take 5 ms. So there's no path explosion here.

### Data flow (category H), taint shape of `state-flow-1`, on the 11.77M-node state-tripwires scope (M)

| Measure | Value |
|---|---|
| SQLite load + indexes | 3.06 s + 7.59 s |
| SQLite recursive query | **0.49 s** |
| Native worklist (not optimized) | 0.94 s |
| Tainted nodes | 51,626 in both, the same set |
| Rows DLRE's ancestor table would hold for this scope | **96.9M (8.2× nodes)** |

**For data flow, loading the facts costs far more than the fixpoint itself.**

### Regular expressions (M)

| Measure | Value |
|---|---|
| Distinct identifier regexes across all 60 rules | 376 (state-tripwires has 559 uses but only 63 distinct) |
| Once per distinct name | 2.1 s one at a time, 1.17 s as one combined set |
| Once per occurrence | 25 ns per regex per occurrence, about 56 s for all 376 |
| Whole-file text regexes (111 clauses) | about 0.1 s each, mostly I/O |

### Unit costs taken from elsewhere (D) or assumed (A)

| Unit | Cost | Source |
|---|---|---|
| Fused programs, simple templates | 0.3–0.97 ns per program-node | fusion.md, same grammar version |
| Masks | 6.4–17.3 ns/node for 210 programs | fusion.md |
| Kind index | 12.4 ns/node | fusion.md |
| Metal waves | 19–22 µs each (GPU-encoded), about 70 µs pre-encoded with real work | metal-enqueue run |
| Metal set operators | 0.005–10.6 ms on 5.4M nodes | metal-recursion.md |
| **Real heavy-rule programs** | **0.3 / 1 / 3 ns per program-node** | **A.** The fusion tests used 3–6-operator templates; serverplan-steps averages about 150 operators per program |

## 3. Per-rule profiles

How to read the tables:
- **CPU** is the CPU interpreter or JIT on one thread.
- **Metal** is GPU time for the AST part only; SQL and data-flow work stay on the CPU.
- **SQLite** means AST matching written as pure SQL, assumed 3–10× slower than CPU (D: the SSC frame engine was about 3× slower than native).
- Costs shared by the whole gate are left out here and counted in §4.

### 3.1 serverplan-state-tripwires

**(a) What it checks**
- 28 dedupe clauses. They match calls, functions, attributes and classes by name, restricted to code inside `state-project-source` (86 uses), capture line, scope and terminal, and remove duplicates per file.
- `state-signatures-1`: 18 stages, with reachability.
- `state-flow-1`, 92 KB:
  - inputs: DLRE's node table, its ancestor table and ownership;
  - fact stages for functions, attributes, calls, arguments, keywords, assignments and parameters;
  - class contexts, found recursively;
  - recursive data flow: `taints`, `semantic_values`, `return_kinds`;
  - a call graph within each file, then `reachable`, then `transitive_sinks` built from path strings;
  - signed exceptions, as an allowlist;
  - an option that switches between per-file and whole-repository partitions.
- `state-flow-project-1`: 27 stages, the whole-repository variant.

**(b) How it would be lowered**
- **The 28 dedupe clauses** become one fused program group per root kind over the shared scope:
  - `state-project-source` becomes one ancestor mask bit;
  - name regexes become bits looked up per distinct name;
  - dedupe comes free, because findings are a set (decision 0001).
- **`state-flow-1`:**
  - fact tables are produced only for files in scope;
  - ownership is answered by interval ranges, with no ancestor table;
  - the call graph and `reachable` are SQL recursive queries;
  - sinks are a closure over sets, with no path strings;
  - **the data-flow stages (`taints`, `semantic_values`, `return_kinds`) remain category H**, run either as a native relation or as SQL over narrow tables;
  - signed exceptions become an anti-join.
- **Execution order:** masks → fused AST groups → fact tables → SQL joins → data-flow fixpoint → consolidation.

**(c) Real counts (M)**
- Scope: 11.77M nodes, 412k calls, 64k functions.
- Data-flow facts: 2.64M identifiers, 774k expression edges, 170k assignments, 140k parameters.
- Results: 51.6k tainted nodes, about 5.7k sink rows.
- DLRE runs this flow twice, and each run implies 96.9M ancestor rows.

**(d) Cost**

| Stage | Low | Expected | High | Notes |
|---|---|---|---|---|
| AST matching | 0.15 s | 0.5 s | 1.5 s | Metal 0.02–0.15 s; SQLite 1.5–15 s |
| Data-flow facts | 0.2 s | 0.5 s | 0.5 s | |
| Loading data-flow facts | 0.1 s | 1 s | 10.6 s | Low is the virtual-table path (A); high is real tables (M) |
| Data-flow fixpoint (2 variants) | 0.2 s | 1 s | 3 s | |
| Call graph, reachability, sinks | 0.05 s | 0.2 s | 1 s | |
| **Total** | **0.7 s** | **3 s** | **17 s** | |

Memory: 0.2 / 0.5 / 1.5 GB.

**(e) Compared with today**
- No per-rule figure exists. Batch six (10 rules including this one) costs about 583 s pre-SQL (709.2 − 125.8), 1,054–1,278 s on the SQL path, and 54–80 s per edit.
- Copying the tree plus its ancestor table for this rule comes to about 217M rows. That probably accounts for hundreds of CPU-seconds by itself — an inference, not a measurement.
- **Expected: 50–200× faster for this rule.**

**(f) Risks**
- The rewritten data flow has to match the oracle exactly.
- The whole-repository mode needs cross-file data flow, which I didn't measure.
- Replacing path strings with sets is only allowed if the findings key doesn't include the path.
- How facts are loaded swings the cost between about 0.1 s and 10.6 s.

### 3.2 serverplan-steps

- **(a)** 14 forbid clauses on step structure, using 674 regex instances. Plus `step-contracts-1…6` (2 inputs each, 16–31 stages, 3–4 recursive): class-hierarchy and callback closures. Plus `runtime-reference-summaries` (10 inputs).
- **(b)** One fused group for the forbid clauses. The contracts become definition, base, decorator, call, keyword and assignment facts, with SQL for the hierarchy closure and the joins.
- **(c)** 2.34M nodes, 94k calls, 16k functions, 2.4k classes. The largest file in scope has 148.5k nodes.
- **(d)**

| Stage | Low | Expected | High |
|---|---|---|---|
| AST matching | 0.03 s | 0.08 s | 0.25 s |
| Facts and joins | 0.05 s | 0.2 s | 1 s |
| Recursive closures | 0.01 s | 0.05 s | 0.3 s |
| **Total** | **0.1 s** | **0.35 s** | **1.6 s** |

  Metal 0.01–0.05 s. Memory under 0.2 GB.
- **(e)** Part of batch six.
- **(f)** Programs average about 150 operators, ten times the tested templates. Compile time for a 1.2 MB rule hasn't been measured.

### 3.3 serverplan-quality (the most uncertain rule)

- **(a)**
  - **`quality-conditional-1`** runs over the whole repository, and its 20 inputs together copy **every syntax-tree node** into SQL (category C). On top of that it builds nodes, edges, ownership and ancestors, conditional class hierarchies, and data-flow-like stages: `always_raises`, `branch_variables`, `self_stores`, `function_colors`, `integer_returns`.
  - `quality-contracts-1` has 54 stages: scope chains, scope imports, annotations, call paths.
- **(b)**
  - The node copy becomes the engine's node relation, read straight from the tape as a virtual table.
  - Scopes and imports become fact tables, and the hierarchy becomes a closure.
  - **The data-flow-like stages need a new per-function relation contract. It doesn't exist yet.**
- **(c)** 28.23M nodes, 6.02M identifiers. **DLRE's copy is about 260M rows; as real SQLite tables that would take about 990 s at the rates measured here.** That is what lies behind the 64 MiB arena failures.
- **(d)**

| Stage | Low | Expected | High |
|---|---|---|---|
| AST matching | 0.26 s | 0.9 s | 2.6 s |
| Node-relation scans (about 3) | 0.2 s | 0.6 s | 2 s |
| Facts and joins | 0.3 s | 1 s | 4 s |
| Data-flow-like work | 0.5 s | 2 s | 15 s |
| **Total** | **1.3 s** | **4.5 s** | **24 s** |

  The high case is the data-flow work in SQL over 6M copied identifier rows. Memory 0.3–1 GB, or 3 GB if identifiers are copied.
- **(e)** At least 50× faster, and memory stays bounded.
- **(f)**
  - Rewriting the data-flow-like stages under decision 0002 is the biggest single job and the biggest parity risk.
  - We have to show that the "every other node" input is never read in full. If it is, that costs one tape scan (about 0.2 s) through a virtual table, but it is fatal if copied.

### 3.4 internal-dead-code

- **(a)** 13 raw inputs, all fact tables in disguise (functions, classes, decorators, parameters, annotations, imports, bases, `__all__`, modules, dynamic imports). 25 stages, 6 of them recursive: hierarchy, public methods, reach edges, reachable, high-confidence. Mode `log`.
- **(b)** Fact tables, then in SQL: import resolution → call resolution → edge table → recursive reachability over an indexed edge table → hierarchy → anti-join with allowlists.
- **(c)** Whole corpus (the rule's scope is 42% of it): 194k definitions, 994k calls, 1.18M distinct references, 352k resolved edges, 175k reachable, 9.4k dead candidates.
- **(d)**

| Stage | Low | Expected | High |
|---|---|---|---|
| Resolution | 0.3 s | 0.8 s | 1.5 s |
| Reachability | 0.3 s | 0.8 s | 18 s (badly written query) |
| Anti-join and hierarchy | 0.2 s | 0.5 s | 1 s |
| **Total** | **0.8 s** | **2.1 s** | **20 s** |

  Loading facts as real tables adds about 7 s for this scope.
- **(e)** Part of the first-50 global evaluation: 52.1 s pre-SQL, 83.0 s current.
- **(f)**
  - Query plans must be held stable, using recorded query plans and checks that catch statistics changes.
  - Recomputing reachability after an edit costs 0.34 s; there's no incremental design yet.
  - The resolver must match the oracle's resolver to reach parity.

### 3.5 public-api-triples

- **(a)** Sync / async / lazy triples across the public API. 11 inputs, 22 stages, 3 recursive.
- **(b)** The same fact tables plus the 16.7k `__all__` names. Re-exports through imports become a recursive SQL closure.
- **(d)** 0.5 / 1.5 / 8 s.
- **(f)** **Reproduce DLRE's "interrupted" failure on its own first.** If it is a cycle under `UNION ALL`, switching to set semantics (`UNION`) fixes it.

### 3.6 async-sync-delegation

- **(a)** 12 forbid clauses plus a budget clause, built on 174 shared utilities (up to 21.6 KB each).
- **(b)** Utilities are deduplicated by digest. Those used inside relations become mask bits, the large ones are called with a depth bound, and the budget clause becomes a count.
- **(c)** 11.79M nodes, 64k functions.
- **(d)** CPU 0.05 / 0.15 / 0.5 s. Metal 0.01–0.1 s.
- **(e)** Part of the 58.2 s that the native matcher spent on AST matching across the first 50 rules.
- **(f)** A recursive utility can only run on CPU or JIT. Compile-time expansion of utilities is unmeasured; fusion.md saw 974k expansions drop to 316k after deduplication.

### 3.7 transaction-patterns

- 58 forbid clauses, all rooted at the same `connection-function` utility. That makes it the ideal fused group: find the root set once (64k functions), then run 58 short checks on what survives. The separate total of 682M program-nodes largely disappears.
- **(d)** 0.05 / 0.2 / 2 s, where the high case means no fusion at all. Metal 0.01–0.05 s.
- **This should be the first fusion measurement (M2).**

### 3.8 async-sync-boundary

- 63 forbid clauses with mixed root kinds, fused by root kind. 12.54M nodes.
- **(d)** 0.24 / 0.8 / 2.4 s. Metal 0.03–0.2 s.

### Risk notes

- **sync-async-signatures:** `generated_facades.py` (206k nodes) parses in 198 ms, extracts facts in 2.5 ms, and should need 2–4 ms of mask work. DLRE failed on a compiler budget of 256 nodes per clause, not on data volume, and interval/mask lowering shouldn't trip it. **That still has to be shown, along with the 1,000-sync-function synthetic case (M7).**
- **grammar-callable-constructors:** this is about how `Complete`/`Unknown` results are decided, not about speed.

## 4. Whole-gate total (CPU)

| Component | Low | Expected | High |
|---|---|---|---|
| Parsing, cold (M; 0 when warm) | 35 s | 35 s | 35 s |
| Tape encoding and kind index | 0.3 s | 1 s | 9 s |
| Regexes (per-name set plus whole-file text) | 1.5 s | 3 s | 10 s |
| AST matching: 588 programs, 6.33G program-nodes (D check: native matcher 58.2 s for 50 rules) | 1.9 s | 6.3 s | 19 s |
| Fact extraction | 0.3 s | 1 s | 1.6 s |
| Loading facts (virtual table vs real tables) | 0.5 s | 2 s | 30 s |
| Cross-file SQL | 2 s | 5 s | 25 s |
| Data flow (category H) | 1 s | 4 s | 20 s |
| Consolidating findings | 0.1 s | 0.3 s | 1 s |
| **Total, without parsing** | **≈8 s** | **≈25 s** | **≈110 s** |
| **Total, with parsing** | **≈43 s** | **≈60 s** | **≈145 s** |

- **Memory:** the tape is about 0.24 GB, sources 0.14 GB, facts 0.2–0.5 GB, data flow under 0.5 GB. Total **1–4 GB**, against 15–16 GiB today.
- **Metal:** 0.2–2 s of GPU time for the AST row, about 0.5 GB resident. It saves at most about 6 CPU-seconds, roughly 1 s of wall time.
- **Pure-SQL AST matching:** the AST row would become 6–190 s. Not the right default for heavy rules.

## 5. One-file edit (warm)

| Step | Cost |
|---|---|
| Re-parse | 1.6 ms for a typical file, 198 ms for the largest |
| Fact extraction | under 2.5 ms |
| AST matching | about 2 ms typical, 0.12 s for the largest |
| Delete and reinsert the file's facts | 135 ms |
| Re-resolve the file's calls | 436 ms naive; milliseconds with a maintained table |
| Recompute global reachability | 342 ms |
| **Total** | **0.02–2 s, about 0.3 s expected** (DLRE: 40–80 s) |

## 6. Biggest unknowns, largest swing first

1. **How fact tables reach SQLite:** a virtual table over native data versus real tables. Swing: 0.5–30 s. I measured real tables; the virtual-table path is unmeasured.
2. **Query-plan stability:** wording alone gave 22× and 34× swings.
3. **Data flow (category H)** in quality and state-tripwires. The cost looks small, at 0.5–0.9 s per variant. The real unknowns are whether it can be expressed under decision 0002, and parity.
4. **Cost of real heavy programs:** the tested templates had 3–6 operators, while heavy rules average about 150. That's a 6× swing on the AST row.
5. **Whether the every-node inputs** in `quality-conditional-1` reduce to tape scans.
6. **Compile time** for rules of 1.2 MB and with 174 utilities.
7. **Cross-file data flow** in the whole-repository mode. Only flow within a file was measured.

## 7. Minimal measurements before any merge

| # | Measurement | Pass condition |
|---|---|---|
| M1 | Run each of the 8 rules alone with the existing native matcher, recording CPU and memory per rule. No per-rule baseline exists today | Gives the baseline for everything above |
| M2 | Fuse transaction-patterns and async-sync-delegation with interned regexes and masks, and check the findings match the native matcher | AST cost at or under 1 ns per program-node |
| M3 | Serve the fact tables through a virtual table and rerun the SQLite queries above (B1–B4) and the taint query | Loading plus joins at or under 5 s for the gate |
| M4 | Re-express internal-dead-code and public-api-triples over fact tables, after reproducing the "interrupted" failure | Same findings, recorded query plans |
| M5 | Implement the `state-flow-1` data flow as a native relation | Matches the oracle on all 1,259 observations, at or under 2 s |
| M6 | Count the rows actually consumed per `syntax_*` input of `quality-conditional-1` | No input copies every node |
| M7 | Run `generated_facades.py` and the 1,000-sync-function case under mask lowering | Completes with no budget failure |
| M8 | Push a one-file change through facts, resolution and reachability | At or under 1 s at p95 |

M1, M2, M3 and M7 are cheap and settle most of the range. M4 and M5 carry the parity risk.

## 8. Caveats

- SQLite inserts went row by row through rusqlite.
- The 588 AST programs are a lower bound, because each rule option is counted once.
- Pure-SQL AST matching is assumed 3–10× slower than the CPU interpreter, from the SSC frame engine (D).
