# Fusion opportunity and performance model on the data-layer rules

Code and raw output are in `../experiments/fusion-model/`; the corpus copy is not checked in. Measured on the gate corpus (9,010 Python files, 28.2M tree-sitter nodes), single-threaded, on a shared machine. Identical runs differed by up to 2.2×.

> **Superseded statement.** The "Recursive set stage (CPU only)" layer in §1 predates `metal-enqueue.md`. Under spec §8 monotone recursion runs on Metal as GPU-scheduled waves; there is no CPU fallback.

## Summary

Everything was run on the real gate corpus (9,010 Python files, 28.2M tree-sitter nodes), single-threaded, on a shared machine. Identical runs differed by up to 2.2×, so every range below carries that spread.

- **What the rules contain:**
  - 535 active AST queries (330 forbid, 14 budget, 191 inputs to relational clauses) and 113 regex clauses over whole-file text.
  - All of them compile into one shared rule graph: 279,204 expanded rule nodes collapse to 6,821 operations.
  - Nesting goes 14 levels deep; 137 queries depend on recursive utilities; rule scopes fall into 42 distinct groups.
  - Root-kind groups are at most 166 queries (`call`), so no group reaches the K=512 regime where the earlier bench saw 2.3–2.5×.
  - Of the 52 relational clauses, 47 are per-file: 29 become joins on captures or exported rows, 11 fold into one program with `EXISTS`/`NOT_EXISTS`, 7 need the `syntax` node table, 4 are cross-file and 1 needs the reference graph.
- **What fusion actually buys (measured, same findings in every mode):**
  - Sharing evaluated sub-rules across queries gives **1.4–3.1×**. Almost all of it comes from no longer re-running regexes over large nodes: regex text scanned drops from 2.59 TB to 3.1 GB.
  - Sharing only the root-kind dispatch gives 0.9–1.2×, i.e. nothing.
  - Building relation masks eagerly for every relation is slower (0.74–1.5×). Masks only for the range-scanning relations break even with plain sharing (1.9–3.3× overall).
- **Where the time goes:** 56 queries account for 80% of the time. The top ten, about 30% of the work, are relational inputs that export every node with captures — the shapes decision 0002 removes.
- **How good the model is:**
  - The per-operation model is within 6% on the data it was fitted to. On held-out data it misses totals by −50% to +46%, and it does not predict individual files.
  - Per-query cost measured on a 5% sample predicts each query to within 1.13–1.40× (totals −31% to −36%). Estimating from rule structure alone is off by about 3× per query.
  - So plans need measured feedback; static estimates are only good enough for first-run grouping.
- **Predictions for the full gate:**

  | Configuration | Predicted time |
  |---|---|
  | Separate programs, 1 core | 147–293 s (native baseline: 125.8 s) |
  | Fused, 1 core | 77–167 s |
  | Fused, 16 cores | 5–17 s |
  | Faster executor (3–5 ns per evaluation), 16 cores, cached trees | **1.2–3.5 s** |
  | Metal (not run here) | 10–86 s, held up by regex on the CPU |
  | Pure SQL lowering | **360–1,760 s** (DLRE measured 958.6 s) |

  The 1–2 s target needs all of these together: cached trees (a cold parse alone is 38–50 s of CPU), shared evaluation, an executor 7–10× faster than this harness, 16-way parallelism, a cheaper regex stage, and the ten heavy relational queries rewritten.
- **Per edit (one file, all queries, parse included):** p50 2.9 ms, p99 189 ms, worst file 1.3 s. Compiling the plan cold takes 5–6 s, so the plan must be cached.
- **Not modelled:** fact-family rules (68 queries) run at zero cost, so findings are a superset. The SQL row counts are only an upper bound. The scope grouping, the line state machine on regex clauses, and multi-core scaling are not modelled either.



## Reproduce

Everything is in `../experiments/fusion-model/`:

```
python3 -I analyze.py <worktree>/packs/data-layer/candidates/rules out
(cd crate && CARGO_TARGET_DIR=$PWD/target cargo build --release)
B=crate/target/release/fusion-model
$B out/queries_full.json out/text_regex.json corpus/src corpus/sample500.txt out/run_A.json sep,fused,memo,mask,maskall 2
$B out/queries_full.json out/text_regex.json corpus/src corpus/sampleB.txt   out/run_B.json sep,fused,memo,mask,maskall 2
$B out/queries_full.json out/text_regex.json corpus/src corpus/files.txt     out/run_full.json sep,memo,mask 1
python3 -I summarize.py <rules> out ; python3 -I model.py out
```

- Sample A is every 18th file; sample B is every 18th file starting at offset 9.
- `out/v1/` is an earlier full run, identical except that it lacks the newer counters.

## Harness

- **Lowering.** `analyze.py` sets each `$option` to its default (`oracleAnalyzer=false`, `sourceGate=true`) and drops clauses whose `when.option` is false.
- **Compilation.** `crate/` compiles all queries into one hash-consed graph; recursive `matches` become back-edges.
- **Exact ast-grep semantics** for:
  - `kind`, `regex`, `value`, `filePath`;
  - `pattern`, with metavariables, `$$$` and `context`/`selector`;
  - `nthChild`, with `ofRule` and `reverse`;
  - `all`/`any`/`not`;
  - the four relations with `stopBy` set to neighbor, end or a rule, plus `field`;
  - recursive `matches`.
- **Approximation.** Fact keys evaluate to true, so findings are a superset of the real ones.
- **Execution modes.** All modes must produce the same findings (asserted: 8,650,988 matches on the full corpus).

| Mode | What it does |
|---|---|
| `sep` | Separate anchored programs, one per query |
| `fused` | One root-kind dispatch per node, shared by all queries |
| `memo` | `fused` plus per-file memo columns for shared rule-graph nodes |
| `mask` | `memo` plus eager masks for range-scanning relations |
| `maskall` | Masks for every relation |

## 1. Static analysis

**Clauses and queries:**
- 496 forbid clauses (112 of them regex over file text), 52 relational (in 10 rules), 44 define, 14 budget.
- 588 AST queries, 535 active.

**Root kinds:**
- `call` 166, `function_definition` 94, `assignment` 50, `identifier` 50, `module` 41, `string` 30, `concatenated_string` 28, `attribute` 20, `import_from_statement` 16, `class_definition` 15.
- 64 groups in total; 10 queries have no root-kind constraint.

**Relation sites after utility expansion:**

| Relation | Sites |
|---|---|
| `has` neighbor | 16,537 (about 60% with `field`) |
| `inside` end | 1,256 |
| `inside` neighbor | 1,198 |
| `inside` stopBy rule | 640 |
| `has` end | 309 |
| sibling range | 215 |
| `has` stopBy rule | 53 |

**Other structure:**
- Boolean operators: `all` 22,203; `any` 8,241; `not` 2,614.
- Atoms: `kind` 24.7k, `regex` 9.4k, `pattern` 4.4k, `nthChild` 1.2k.
- Utility bodies: 278; references to recursive utilities: 4,710. 137 queries depend on recursion.
- Distinct query digests: 497. Distinct regexes: 816. Only 20 patterns are structural; the rest are `$X` captures.
- Rule scopes: 42 distinct for 60 rules; the largest identical-scope groups hold 6, 6, 4 and 4 rules.

**Queries by layer:**

| Layer | Rule queries | Relational inputs |
|---|---|---|
| Instruction program | 46 | 62 |
| Set stage | 129 | 117 |
| Recursive set stage (CPU only) | 101 | 7 |
| Fact family | 68 | 5 |

Fact-family keys appear in these numbers of queries: `compare` 34, `layout` 26, `parse` 17, `count` 6, `binds` 4, `arguments` 4, `host` 3, `calls` 1.

**Rules by their heaviest layer:**

| Heaviest layer | Rules |
|---|---|
| Fact family | 20 |
| SQL composition | 10 |
| Set stage | 9 |
| Recursive set stage | 9 |
| Regex only | 9 |
| Instruction program only | 3 |

**The 52 relational clauses under decision 0002:**

| Count | Clause shape | Becomes |
|---|---|---|
| 29 | Per-file join | Join on captures inside one program, or an SQL join over exported rows |
| 11 | Per-file `EXISTS` / `NOT EXISTS` | One fused program, or `select_when` |
| 7 | Uses the `syntax` node table | Engine relation plus SQL |
| 4 | Cross-file | `require_each` / `select_when` |
| 1 | Reference graph | Fact family |

## 2. Corpus statistics

- 28.23M nodes, of which 18.5M are named.
- 1,066 distinct (parent, field, child) edges.
- Depth: mean 8.0, maximum 86.
- Nodes per file: p50 1,333; p90 7,056; p99 28k; maximum 206k.

**Root kinds** (dashes mean the subtree-size percentile was not computed for that kind):

| Root kind | Count | Mean subtree size | p50 / p90 / p99 subtree |
|---|---|---|---|
| `identifier` | 6.0M | 1 | — |
| `call` | 1.06M | 17.8 | 11 / 35 / 106 (max 11k) |
| `string` | 781k | 4.5 | — |
| `assignment` | 467k | 21 | — |
| `function_definition` | 162k | 167 | 88 / 365 / 1,157 |
| `class_definition` | 32.7k | 346 | p99 3.9k |
| `module` | 9,010 | 3,133 | max 206k |

The heaviest candidate set belongs to `quality-conditional-1`, whose unconstrained root visits all 28.2M nodes.

## 3. Fusion opportunity

**Static measures:**
- **Sharing:** 279,204 expansions collapse to 6,821 operations (41×). Of 2,014 relations, 229 are shared and 341 depend on recursion.
- **Relation targets:** 2,185 distinct; 1,029 are used by two or more queries.
- **Root-kind group sizes:** 9 groups have K ≥ 16, covering 495 query–root pairs. No group reaches K=512; groups of 128–166 sit in the earlier bench's 1.3–1.5× band.
- **Queries per stratum:**

  | Stratum | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 or more |
  |---|---|---|---|---|---|---|---|---|---|
  | Queries | 49 | 99 | 163 | 58 | 43 | 61 | 23 | 16 | 23 |

- **Mask targets per stratum** (non-recursive):

  | Stratum | 1 | 2 | 3 | 4 | 5 | 6 | 7–14 |
  |---|---|---|---|---|---|---|---|
  | Targets | 789 | 373 | 193 | 112 | 67 | 42 | 99 |

- **Mask candidates:** 380 range-scanning relations, or 1,673 if neighbor relations are included.

**Measured speed-up over `sep`:**

| Corpus | `fused` | `memo` | `mask` | `maskall` |
|---|---|---|---|---|
| A | 0.90–0.97× | 1.9–2.1× | 1.9–2.1× | 0.74–0.91× |
| B | 1.06–1.24× | 2.3–2.6× | 2.5–3.3× | 1.4–1.5× |
| Full (run 1 / run 2) | — | 3.1× / 1.4× | 2.7× / 2.2× | — |

On the full corpus, `sep` took 171–173 s, `memo` 55.6 s and 119.9 s, and `mask` 63.3 s and 78.6 s.

**What explains it:**
- **Shared sub-rules on large nodes.** In `sep`, regexes scan 2.59 TB of node text, because regexes on module and function bodies are reached through `inside … stopBy end` from every candidate. Memo columns cut this to 3.1 GB, and evaluations fall from 4.98G to 3.19G. Lazy per-node sets are enough.
- **Dispatch-only fusion buys nothing.** Candidates are already selective, and the cost sits in the relations.
- **Eager masks lose when few nodes ask for them.** Masking every relation builds 1.13G column cells where `memo` needs about 0.1G lookups. Masks for range-scanning relations break even. This confirms the rule in `research/fusion.md`: mask only when the summed scan cost over roots exceeds nodes × the cost of a pass. That holds for a few hundred scanning targets and never for neighbor relations.
- **Time is concentrated.** 56 queries make up 80% of `sep` time, and the top ten are relational inputs:
  - `quality-conditional-1`: 14.5 s;
  - `step-contracts-1/3/5/6`: 4–8.6 s each;
  - `legacy-execution-graphs`: 3.5 s.

  Rewriting them is worth about 30% of the structural work.

## 4. Performance model

### Level 1: per-operation fit

The model fits per-file time against 23 operation counters with non-negative least squares, using every mode of sample A.

**Fitted unit costs:**

| Item | Cost |
|---|---|
| Per file | 12.7 µs |
| Candidate scan | 11.3 ns per node |
| Dispatch | 13.2 ns per (node, query) |
| Range scan visit | 6.4 ns |
| Mask cell | 7.4 ns |
| Mask lookup | 5.9 ns |
| Memo hit | 94 ns (absorbs column allocation) |
| `not` | 273 ns |
| Pattern match | 1.14 µs |
| `has` end | 160 ns |

The harness averages 35 ns per evaluation.

**Calibration error:**

| Data | Error on totals | Median per-file error |
|---|---|---|
| Sample A (training) | +0.7% to −5.5% | 58–91% (all data) |
| Sample B (held out) | −34% to +46% | |
| Full corpus | −50% to −17% | |

Totals come out within about 1.5×; individual files do not. Huge files and machine contention dominate per-file time.

### Level 2: planner estimate

Query time is modelled as candidates × κ (per-candidate cost).

| Variant | κ comes from | Median per-query error, A→B / A→full | Total error, A→B / A→full |
|---|---|---|---|
| Feedback | Measured on sample A | 1.13× / 1.40× | −31% / −36% |
| Cold static | Rule structure only | 3.1× / 3.0× | −31% / −38% |

Plans therefore need measured κ from `plan_feedback`.

### Stage costs for the full gate, one core

| Stage | Full gate |
|---|---|
| Parse, cold | 34–60 s (measured 38–50 s) |
| Restore cached trees | 0.10–0.22 s |
| Kind postings | 0.23–0.45 s |
| Structural, `sep` | 129–232 s |
| Structural, fused | 56–120 s measured |
| Regex over file text (4.3 GB) | 9–17 s |
| Relational SQL (8.5M rows, an upper bound, × 1–5 µs) | at most 8.5–43 s |
| Consolidation (about 25k recorded findings) | under 0.5 s |

## 5. Predictions

| Configuration | Full gate |
|---|---|
| `sep`, 1 core | 147–293 s |
| Fused, 1 core | 77–167 s |
| Fused, 4 workers | 19–52 s, plus 9–14 s if trees must be parsed |
| Fused, 16 workers | 5–17 s |
| Faster executor (3–5 ns per evaluation), 16 cores, cached trees | **1.2–3.5 s** |
| Metal | 10–86 s (kernels 1–41 s; regex stays on the CPU) |
| Pure SQL lowering (3.4G rows × 100–500 ns) | **360–1,760 s** |

- **DLRE, 958.6 s:** falls inside the SQL-lowering range.
- **Native baseline, 125.8 s:** matches `sep`.
- **1–2 s target:** needs all of cached trees, shared evaluation, a 7–10× faster executor, 16 cores, a cheaper regex stage and the heavy relational queries rewritten.

**Per edit (one file, all 535 queries, one core, parse included):**

| Mode | p50 | p90 | p99 | Worst file (206k nodes) |
|---|---|---|---|---|
| `sep` | 3.7 ms | 32 ms | 358 ms | 5.6 s |
| `mask` | 2.9 ms | 22 ms | 189 ms | 1.3 s |

These figures assume a cached plan; compiling it cold takes 5–6 s.

## 6. Not modelled, or uncertain

- **Fact families:** 73 queries evaluate to true at zero cost.
- **Relational SQL:** row counts are an upper bound, and the per-row cost is assumed, not measured.
- **Regex clauses:** the line state machine is not run.
- **Scope grouping:** the 42 scope groups are not modelled; fused runs filtered scope per query, which flatters fusion.
- **Patterns:** matching uses named children only.
- **Noise:** identical runs differed by up to 2.2×.
- **Metal:** figures are derived from the kernel costs in `research/metal-recursion.md`, not run on this workload.
- **Parallel scaling:** assumed, not measured.