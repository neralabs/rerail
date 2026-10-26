# Review: the Pulling-it-all-together design and the work since

Date: 2026-10-26. Branch `jeremybarnes/wolfeel`, at main `5cf4246e`.

This is the starting point for re-integrating the Rerail rule-engine worktrees into one final shape. It covers:

1. the Pulling-it-all-together (PIAT) design;
2. what each stream has learned since PIAT took its snapshots;
3. what that means for the final shape.

Sources, with tips as surveyed:

| Stream | Ref | Since PIAT snapshot |
|---|---|---|
| PIAT | `jeremybarnes/Pulling-it-all-together` `85098ee3` | (the design itself; 118 commits on main) |
| Shared-structural-execution | `jeremy/shared-structural-execution` `85098ee3` | same commit as PIAT; no work of its own |
| Typing-support | `jeremybarnes/Typing-support` `e031592d` | 14 commits, all new (`spikes/datalog-engines/`) |
| DLRE | `jeremybarnes/Data-layer-rule-expressivity` `a1fcdeb9` | 89 commits since `d3470da2` |
| N1562 (rerail) | `jbarnes/ner-1562-sql-rerailer` `60e7affa` | 82 commits since `16343282` |
| SSC | `jeremy/sql-capture-demand-spikes` `b3e97cd4` | 10 commits since `c17d2e47` |
| Moti facts | `origin/Moti-Velz/facts-engine` `5b0d63a7`, `facts-plan` `a582db97` | 19 and 8 commits since `685db3f7` |
| Moti on main | `origin/main`, author Moti-Velz | engine-relevant commits since 2026-10-14 |
| Tempo Metal | nera-landing-2 `jbarnes/ner-1562-scale-test-the-re-railer` `ed69e91f8` | 64 commits since `a091aa063` |

Numbers are the streams' own recorded measurements. Nothing was re-run for this review.

---

## 1. The PIAT design

### What it got right, and should be kept

- **The three targets:** per-edit checks, a 1–2 s repository audit, and everything runnable inside an outer SQL query with a pure-SQL fallback.
- **The loose findings contract** (`findings-contract.md`): a set of finding keys plus a per-(rule, file) Complete/Unknown outcome.
- **Rules carry no SQL or code** (`authored-sql.md`).
- **One relation contract with two bindings** (pure SQL and native vtab), differentially tested.
- **Query shape comes from statistics held in SQL.** Rust reads them; it does not own them.
- **The anchor index is always built** (0006). **Relations read tree neighbourhoods from the tape** (0007).
- **The boundary verdict** (`research/sql-vs-transducer.md`): about 85–90% of the SQL lowering in the streams computes tape-local, per-document facts that belong in a compiled per-document program.
- **Named derived facts** (0005): shared and relational intermediates are defined apart from rules, cached by digest, and invalidated by dependency cone.
- **The lessons ledger and the prior-work protocol.** The ledger is the most valuable single document across the streams. Every stream since has re-learned at least one of its entries (§2).
- **The cost model and measurement gate** (spec §19): CPU, with construction reported apart from execution.

### Why it bogged down

1. **What it executed contradicted what it planned.** The architecture said transducer-first: N1562's packed IR as phase 2, Tempo's Metal executor as phase 3. Shape-fixes §1 itself says that publishing every node into a positional index "should not be on the path of local rules".

   The last 36 hours went into DLRE's anchor-selection lowering instead:
   - set algebra over postings, run as about 430 SQLite stage statements per batch for serverplan_quality;
   - the gains were real: internal_dead_code's anchor selection fell from 83.8 s to 21.5 s;
   - but anchor selection is still the largest execution cost (serverplan_quality 53.7 s);
   - and it is the component the spec says to replace.

   The index is not in question (0006). What is in question is what consumes it. Under the spec, the planner reads postings to seed per-document programs. Under the current code, the inner plan's whole set algebra runs as SQL stages. That second form is the interpreter-in-SQL shape that ledger L6, L16 and L19 warn about.

2. **"Merge, then mutate" stalled at the merge.** Merging DLRE first brought in three things:
   - DLRE's executor (CTAS stages, `ANALYZE`, authored relational SQL);
   - its exact-parity harness;
   - the 60 ported data-layer rules as the gate.

   The gate's failures then set the agenda, and phases 2–7 never started. Meanwhile the sources moved. N1562 restructured into `query/plan/render/`. SSC's tip routes through code it has since rejected. Tempo still does not complete the corpus. A merge plan pinned to snapshot SHAs goes stale within a day while every stream is moving.

3. **The spec ran ahead of the evidence.**
   - About 3,100 normative lines: IR extensions E1–E16, the v9np tape, residency slabs, fusion.
   - Five of seven decisions are still "proposed".
   - Of the pre-merge measurements, M3–M6 and M8–M11 were never run.

4. **The relational layer had no language and no evaluator.** Decision 0005 says "declarative relational algebra compiled to SQL" but names no language. The cost model gives relational SQL 2–25 s, fact load 0.5–30 s and data flow 1–20 s, the widest ranges in the model. This is the hole Typing-support has since filled.

5. **The gate workload is the least typical one.** The 60 data-layer rules are DLRE parity ports, many of them carrying authored SQL. They are the slowest and least representative workload, and every iteration on them took tens of minutes. Typing-support iterated on CPython `Lib/` (574 files) and `native/src` (331 files) against exact goldens, in seconds to minutes.

---

## 2. The work since PIAT, stream by stream

### Typing-support: the most important new information

The spike asked whether facts, types and dependencies should run on an off-the-shelf Datalog engine (`spikes/datalog-engines/FINDINGS.md`, `rdl/RESULTS.md`, `lattice/RESULTS.md`). It then built its own engine, `rdl`, through seven rounds.

**Measured:**
- **One program, six engines, identical results.** The 140-line workload (B110, SELF_ASSIGN, NO_TIMEOUT through imports, a set-valued `type_of`, constant propagation over a lattice) matched exactly on Soufflé, Ascent, egglog, SQLite and others. The semantics are Datalog and portable. No engine provides what Rerail needs:
  - state in SQL;
  - per-document derivation with invalidation;
  - three-valued facts;
  - the transducer with demand pushdown;
  - runtime-loaded rules.
- **One Datalog AST, a statistics-driven planner, several backends.**
  - The planner chooses join order by Selinger DP over subsets. The anchor is the rarest mandatory variable (spike S049–S050).
  - Climb or descent is chosen by a cost expression over `efreq` (an absorbing Markov chain) and subtree size, not a fitted constant.
  - Statistics are built in SQL at ingestion (0.73 B/node), merged per batch and fingerprinted.
  - The plan emits compiled SQL, generated Ascent, or a native set engine. All are exact against the goldens.
- **Tree structure is never Datalog recursion.**
  - `enclosing` was 5.3 s of Soufflé's 7.8 s.
  - As a tape built-in (`nearest`, `within`), or as one preorder pass, it costs 4 ms. This agrees with 0007 and topic 2.
- **Recursive strata run natively, with SQL as the store** (rounds 3–7):
  - `lattice_fixpoint` and `set_fixpoint` sit behind vtabs.
  - Then "program mode": whole strata run natively and relations stay native between strata.
  - Join plans are replanned between chunks of waves from the store's live counts. Fanouts are weighted by postings and corrected from observed fanouts.
  - Storage is an arena with CSR indexes.

  Python call resolution, CPU:

  | Engine | Execution |
  |---|---|
  | Plain SQL | 14.0 s |
  | SQL + native pieces | 5.0 s |
  | **Native program** | **1.48–1.67 s** (construction 1.4 s, 640 MB) |
  | Generated Ascent | 1.8 s, 2.8 GB |
  | Main's native resolver | 1.9 s including parse, 0.3 GB |

  Rust: native program 1.13–1.20 s against the resolver's 2.0 s.
- **Call resolution is type inference.** `calls.rdl` (362 lines) and `calls_rust.rdl` (281 lines) resolve more calls than main's resolver (Rust: 18,119 named against 12,289). Against references:
  - Jedi: 99.18% agree;
  - CPython runtime: 84/9/0;
  - rust-analyzer: 37/7/0.
- **Lattices must be first-class.**
  - Per document, the efficient strategies take 6–27 ms per pass for the whole corpus: semi-naive waves, worklist, or SCC condensation.
  - Across files: layers with interface-digest cutoff. An interface edit re-derives 123 modules instead of 408.
  - On Metal: semi-naive waves over 32-bit values, estimated under 1 ms.
- **Not solved: per-edit queries.** Magic sets for a one-file query were 13× slower (168 s against 12.7 s). Demand binds only the first column, and hand-written demand rules spread demand everywhere. The spike's conclusion is that a one-file query needs per-file summaries, or a whole-program result maintained per changed file.

**What it means:**
- It gives 0005 its language and evaluator. The algebra for named derived facts is this Datalog fragment: `ast(...)` atoms holding patterns, tape built-ins, lattices with widening, stratified negation and aggregates.
- The same fragment can carry the relational part of rules. That is the replacement for DLRE's authored `relational` clauses (0002).
- The pure-SQL fallback (target 3) comes from the same plan, through the SQL backend.
- It refines "SQL drives execution" without making it a SQL-versus-Rust choice. SQL owns state, statistics, composition and the fallback. A planned stratum is pushed down whole behind a vtab, because one SQL statement per wave lost 3–10×. This is your pushdown principle applied per stratum, not per operator.
- It confirms the planner lessons as compiler checks:
  - an enforced join order;
  - threaded walks;
  - typed columns;
  - postings-weighted fanout;
  - observed-fanout feedback, which is spec §19's `plan_feedback`, now measured.
- **Caveats:** the compiler is a spike-quality Python script. Construction copies facts into hash indexes, where it should read the tape and anchor index. Memory is 2× main's resolver. The per-edit path is open.

### Moti's facts engine: the per-edit answer for cross-file state

`facts-plan:docs/facts/DECISION-CROSSFILE.md` and `facts-engine:spikes/facts-b/src/{ygraph,y2,y3}`.

**Measured** (his agents' runs, not re-verified):
- **Two alternatives are eliminated:**
  - Full differential-dataflow maintenance has a memory floor of about 6–7 GB.
  - Recompute-per-edit costs 5–70 s, and inputs change on 88% of edits.
- **What remains is one maintained reference graph plus a symbol table:**
  - keyed tables, with memoised per-key queries that record their read sets;
  - level-ordered re-run with early cutoff;
  - state layered across worktrees as base, then overlays, then deltas.
- **Rules become local queries over the graph.** Dead code becomes a reached set with level witnesses.
- **Results:**
  - single-file edit: p50/p95/p99 of 0.53 / 3.76 / 8.62 ms;
  - real-commit p95: 470 ms;
  - exact on 1,151 from-scratch checks, and on 4,931 of 4,931 dead-code findings.
- **Memory misses its target:** 2.1 GB against 250 MB. The query bodies are hand-written Rust with no SQLite and no planner.

**What it means:** PIAT's rejection still holds for the executor. The design, however, is the incremental mode the Datalog layer lacks:
- per-key read-set memo with early cutoff, which is dependency-cone invalidation at key granularity;
- interface cutoff, the same mechanism as Typing's digest cutoff;
- worktree layering, which no other stream has.

The final shape should make this the incremental evaluation mode of the Datalog strata, derived from the rules instead of hand-written. His replay harness becomes the per-edit gate.

**On main, from Timo,** these must be kept:
- embedded programs as guest documents with `Origin` (tape v6, NER-1698);
- per-clause work allowances and blind files (NER-1865);
- "unread" files above 50% error coverage;
- the language adapter seam and Rust parsing;
- per-file module system and package facts (NER-1868);
- suppressions and per-repository config;
- ai-output steering scope.

Any per-document IR has to respect main's document model.

### SSC: the native continuation interpreter is also a dead end

**Measured** (5,725 files, 146 clauses, exact parity throughout):
- Multi-rule native batching with a shared prefix reached 0.98× native. It is the last cleanly qualified state (`92b1978e`).
- A lane-wave executor was CPU-neutral (`14306e68`, the accepted baseline).
- The full "cohort" conversion turned captures, utilities and alternatives into a native continuation-frame scheduler. It ran at 1.49–1.65× native, with cycles 1.64× against instructions 1.3×. It was rejected, together with a dozen frame, dispatch and arena micro-optimizations.
- The tip is at 1.585, failing its gate.

**Its new plan** (`docs/sql-structural-execution-plan.md`, proposal only):
- the unit is a `ProgramGroup` over a bounded document batch with an exact scope;
- every stage is one of: a local program, a native fact producer, a SQL relational query, or a declared fixpoint;
- no interpreter remainder;
- resolution as named facts.

It cites PIAT 0005–0007.

**What it means:** an independent convergence on the same shape. An interpreter of continuations loses whether it lives in SQL or in native code, so local structure has to be compiled programs, not scheduled frames. Its "declared fixpoint" stage is Typing's native fixpoint.

To take from SSC:
- the boundary at `c17d2e47`/`92b1978e` (demand handles, typed batches), without the cohort routing;
- shared-prefix factoring;
- resident partitioning (2.9 GB → 1.2 GB RSS);
- the Python lookup-fact cache and import-suffix guard;
- the hardware-counter harness.

### DLRE: correctness recovered; its SQL-stage executor is confirmed as the bottleneck

**Measured** (9,021 files, cold CPU):
- first 50 rules: **672.8 s**, exact (24,820/24,820), 4.62 GiB. That is still 5.35× the pre-SQL 125.8 s;
- all 60 rules: 2,540.6 s, exact, of which 1,392 s is the final deferred evaluation.

Traced costs:
- SQL analysis 679 s, candidate materialization 620 s, relational materialization 456 s;
- cost grows with the number of live private schemas, through repeated `ANALYZE`;
- 1,527 s is unattributed.

**Spikes:**
- statistics-chosen anchors: keep;
- fusion: keep, but in the planner, not in rules;
- native batches for cheap operators: no win (105.8 against 106.8 ms);
- 32-file publication transactions: 6.5× cheaper;
- plan reuse: reduce live schemas first.

The SC1–SC4 schema-lifecycle plan is written but not implemented.

**What it means:** DLRE confirms ledger L5, L6 and L16 once more. The SQL-stage executor carries the cost, and its own diagnosis is per-consumer schema objects.

DLRE still gates on exact parity and still keeps authored SQL. CPU fixes were made by hand-reordering joins in rule SQL, which shows what a planner must do.

**To take:**
- the full-repository runner;
- CPU attribution without double counting;
- failure replay;
- statistics-driven anchor choice;
- 32-file publication;
- the 60 rules and their observations, as a workload to re-express in the new rule language.

**Not to take:** the executor.

### N1562 (rerail): per-scope statements, typed vtabs, no measurements

**Built:**
- one statement per exact-scope group instead of per catalog;
- the 35-column `rerail_matches` replaced by **10 narrow typed vtabs**, each declaring its ordering and pushing down producer, file and node-range constraints;
- domain-separated digest identities;
- boundary-check scripts;
- `MATERIALIZED`/dead-recursion fences against SQLite's CTE blow-up;
- a 776-line design for splitting operational semantics: `ordered_region`, utility demand/answer, an effect ledger. The operators exist only as test enums.

**Not done:**
- No CPU, parity or coverage was measured in the range.
- `packed/`, the interpreter and the JIT are untouched apart from `SUPERTYPE`.
- Statistics are seeded constants.

**What it means:** whole-catalog SQL is abandoned here too. The rendered relational IR (`relational_ir.rs`, 5,462 lines) and its fences are a dead end.

**To take:**
- the typed vtab schemas and their view substitution, which are the two-binding contract made concrete;
- the identity scheme;
- the boundary checks.

PIAT's own M2 finding still holds: the packed IR lowers none of the M2/M7 rules, while the M2 prototype's fused executor met the 1 ns/program-node target.

### Tempo Metal: not integrable yet

**Measured:**
- The 5-file differential agrees on all 109,114 matches in 431 dispatches.
- The full 335-pattern × 2,079-file run **still fails**. At the latest commit it stopped at 595 dispatches with 1.98 GB of tracked Metal memory. Each fix moved the failure point between the dispatch cap and an allocation refusal.
- No single-file latency has been measured.
- The executor drives a host-side, generation-by-generation frontier sized by readback. It has no device-side enqueue and reads Tempo's own `TapeDoc`, not v7.

**What it means:** the GPU requirement stands, and so does the rule that Metal executes every admitted program with no fallback. But the current architecture runs the whole corpus as one batch, about 18–20 dispatches per generation. That serves neither the live-edit target nor PIAT's device-side enqueue, and the handover says trimming allocations cannot replace dispatch work.

**To take now:**
- the IR spec and verifier;
- the CPU interpreter and JIT as oracles;
- kernel techniques: cooperative scan, tile sort, typed merge, sealed columns, workgroup NFAs, first-byte proofs;
- capture ranges;
- the budget harness.

The executor itself should be re-aimed at a per-file or small-batch request against the same contract before it is integrated.

---

## 3. What it means for the final shape

### Convergence

Without coordination, the streams have converged on one shape. Three parts were already in PIAT; Typing-support and Moti added the two it lacked.

1. **Substrate.**
   - Main's v7 tape and document model (embedded guests, `Origin`).
   - The anchor index, always built.
   - Statistics in SQL, built at ingestion and fingerprinted: kind and edge frequencies, postings, column moments.
2. **Local programs.**
   - Per-document compiled programs for structural patterns: transducer IR, with a CPU interpreter, JIT and Metal.
   - Seeded by the planner from postings.
   - Results come out through narrow typed vtabs with demand handles.
   - No continuation interpreter in SQL or native (SSC, DLRE, N1562).
3. **Relations and inference (new).**
   - One Datalog fragment is the language for named derived facts, type inference packs, and the relational part of rules. It has `ast(...)` atoms, tape built-ins, lattices, stratified negation and aggregates.
   - The fragment is planned from SQL statistics, and one plan has three evaluation modes:
     - **pure SQL:** the fallback, and the outer-SQL target;
     - **native strata behind a vtab, with SQL as the store:** the audit target;
     - **incremental keyed maintenance with read-set memo and early cutoff:** the per-edit target (Moti's Y1 design, derived from the rules instead of hand-written).
4. **Consolidation:** findings as a set, Complete/Unknown per (rule, file).

### Where this departs from PIAT

- **SQL is not the evaluator for relational work.** SQL holds state and statistics, composes the outer query, and is the fallback. Planned strata run natively behind vtabs. This is pushdown at stratum granularity, consistent with "no SQL-versus-Rust dichotomy"; Typing measured SQL-per-wave at 3–10× slower.
- **Anchor selection becomes a planner step.** It reads postings by point lookup and seeds programs, as Typing's planner and the spike's rarest-anchor rule do. It is no longer a set-algebra program of SQL stages. The index is unchanged (0006).
- **A language for 0005, and a resolution for 0002.** DLRE's authored relational SQL is re-expressed in the Datalog fragment, not in a new algebra.
- **IR extensions are trimmed.** PIAT already withdrew keyed sets (E15) and bottom-up folds (E17), and the Datalog layer takes those over. The transducer stays small, as spec-transducer §0 asks.

### Integration method: a proposal

The PIAT experience argues against merging whole branches and then mutating them.

- **Start from main.**
- **Port components by interface,** in this order:
  1. statistics;
  2. typed vtabs;
  3. the Datalog compiler (native, from `rdl`);
  4. the native set/lattice engine;
  5. local-program lowering and the executor;
  6. the incremental mode.
- **Run a differential test at each step** against the native matcher and the Typing goldens.
- **Gate on the small exact corpora** in the inner loop (CPython `Lib/`, `native/src`, a data-layer sample). The full data-layer corpus runs once per phase.

---

## 4. Questions for you before the design is written

1. **Is the Typing-support Datalog fragment the language** for named derived facts and the relational part of rules, replacing both `rerail.facts/v1` and the DLRE `relational` clause?
2. **Integration method:** port components onto main by interface, as above, or keep PIAT's "merge, then mutate"?
3. **Anchor selection:** may it move from SQL stage programs into the planner, seeding local programs from postings? The index stays mandatory either way.
4. **Gate:** small exact corpora in the loop, with the 60 data-layer rules re-expressed rather than ported with their authored SQL?
5. **Metal:** take the IR, oracles and kernel techniques now, and re-aim the executor at per-file requests before integrating it?
