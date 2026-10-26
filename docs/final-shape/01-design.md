# The final engine shape

Date: 2026-10-26. Supersedes §3–§4 of `00-review.md`.

## 0. The design in one paragraph

Every rule is a stratified Datalog program over one batch's tape relations. Pattern rules and relational rules alike compile into one shared operator DAG, planned from statistics held in SQL. The DAG runs **set-at-a-time over a batch of documents × all rules**, in semi-naive waves:

- one kernel launch (Metal), one vector loop (CPU) or one statement (SQL) per operator per batch;
- never per file, per root or per path.

Unbounded tree relations are set operators (axis joins, mask passes, fixpoints), and the frontier never holds continuations. What survives of the transducer is a bounded candidate kernel: a fused, branch-free test of one candidate, run data-parallel over every candidate in the batch.

SQL is the system of record (tape, anchor index, statistics, cached derived relations, findings). It is also the outer query, through typed vtabs, and the pure-SQL binding of the same plan.

- **Audit:** the plan over the repository, cut into resident batches as large as memory allows.
- **Per-edit:** the same plan over a delta batch, with derived relations maintained incrementally.

## 1. Why this shape: the evidence, read for shape rather than completion

Every stream that evaluated a rule per root, per file or per path paid for it, in every substrate:

| Where | Per-unit shape | Cost |
|---|---|---|
| Callback engine (ledger L17) | Rust per node/term from SQL | 889 node rows and 688 callbacks per output row; 1,141–1,405 s vs ~10 s |
| SSC scalar matcher | per root | 73.2M root evaluations; 79.5 of 83.6 s inside `AstMatcher` |
| SSC cohort scheduler | per-path continuation frames, native | 1.49–1.65× native; cycles 1.64× vs instructions 1.3× (memory-bound frame traffic) |
| SSC/DLRE/N1562 SQL stages | interpreter or continuation machine in SQL | ~3× native (SSC); 5.35× (DLRE first-50); 164 MB of SQL per clause (N1562) |
| Tempo Metal | per-path generations with cursor/repeat frames, sized by host readback | ~20 dispatches per generation; 19 generations dominate launches; per-task tuples 1.99 GB (ledger L1); the handover's blocker is "dynamic frontier continuation" |
| M2 prototype | per root, fused trie | passes at 0.26–0.40 ns/program-node, but by doing 20–40× fewer evaluations: each costs 13–17 ns, 74% interpretive |

Every set-at-a-time form won, in every substrate:

| Where | Set form | Result |
|---|---|---|
| fusion.md | mask passes: one reverse-preorder pass gives every node the subtree bitset of target kinds; `has`/`inside` become bit tests | 25–54×; 210 programs at 6.4–17.3 ns/node per pass |
| metal-enqueue.md | stratified Datalog over node sets, GPU-scheduled waves, each node entering once per stratum | CPython, 5.4M nodes, 27 waves: 1.57 ms GPU. 100k-wave chain: 113 ms in one command buffer. CPU makes no per-wave decision |
| Typing-support `rdl` | one Datalog AST, Selinger-planned from SQL statistics, native strata in waves over an arena | 14.0 s (plain SQL) → 1.48–1.67 s; beats Ascent; within 1–1.5× of the per-file resolver's resolution phases, while resolving more |
| Typing lattice | semi-naive waves | `type_of` in 10–11 ms over 1.2M nodes; naive rounds take 282–404 ms |
| Typing scopes | tape column pass | `enclosing` in 4 ms, against 4.5 s (SQL recursion) and 5.3 s (Soufflé) |
| ast-index-spike | structure lowered to set queries over the tape, rarest anchor | 112 → 33 s, 1.05 rows per answer (topic 3) |

The conclusion is about shape, not completion:

- **State must scale with nodes × relations, not with paths × frames.**
- **Launches must scale with plan operators × waves, not with files × roots.**

Only the set form meets both on every backend. This is PIAT's E14 "set stage" (spec §6) and `metal-enqueue.md` §3 taken to their end: the set stage is the execution model, not an optimisation inside the transducer.

## 2. Data layout: the batch

A batch is a set of documents (files and embedded guests, main's `FileImage`/`Origin` model) that is resident at once. Batches are cut to a memory budget, never to one file. For the data-layer gate (28.23M nodes) that is the whole corpus in one batch, or a few.

**Node columns.** Node ids are global u32 within the batch, preorder-contiguous per document. A document is an interval `[lo, hi)`, so a document-local join is a range test (Typing round 3). The columns are fixed-stride, so the same arrays serve CPU vector loops, Metal buffers and the SQL views:

| Column | Width | Meaning |
|---|---|---|
| `kf` | u16 | folded kind + field code (v9np, decision 0003) |
| `par` | u32 | parent (derived at batch load from v7 parent deltas) |
| `size` | u32 | subtree size, so a descendant test is an interval |
| `idx` | u16 | child index |
| `tid` | u32 | interned text id for leaves and identifiers; 0 otherwise |
| `doc` | — | implicit, by interval |

About 16 B/node: 450 MB for the data-layer corpus, built from the v7 tapes in the store.

**Base relations come from the anchor index.** The index is always built (0006). It stores:
- kind postings and value (text) postings as CSR id lists per batch;
- regex results per distinct text: `text_match(pattern, tid)`, computed once over the vocabulary, which is `TEXT_ID` from `research/regex.md`.

The postings are the planner's extensional relations and the source of anchor choice.

**Derived relations.** Each is stored in one of two forms:
- unary: a bitmap (n/8 B), or a sorted id list (4 B per element) below n/32 density, converting on device;
- arity > 1: columnar rows in an arena with CSR indexes (Typing round 7). Dedup by row id, the delta is a chain prefix, and there is no allocation per tuple.

**Arena bound.** Live sets × their bounds are computed at plan time from document node counts (metal-enqueue §3). A plan whose bound exceeds the budget at the largest batch the corpus statistics allow is rejected at compile time. Exhaustion at run time is `Unknown(resource)` per (rule, document), never rerouted.

## 3. The IR: one operator DAG for all rules

**Front ends:**
- **Pattern rules** (`rerail.policy/v1`, the shipped packs) compile structurally.
- **Relational rules and named facts** are written in the Datalog fragment from Typing-support:
  - `$X` variables and `ast(...)` atoms holding patterns;
  - tape built-ins `nearest`/`within`/`inside`;
  - `lattice` declarations with join and widening;
  - stratified negation and aggregates.
- Rules carry no SQL (0002). DLRE's `relational` clauses are rewritten in this fragment, as Typing-support's `calls.rdl` rewrote call resolution.

**Operators.** A relation in the DAG is a node set, or a typed tuple relation when captures are exported:

| Operator | Semantics | Covers |
|---|---|---|
| `Base(kind \| text \| regex \| field)` | a posting or column filter | kind, field, text equality, regex, flags |
| `Up(S, field?)` / `Down(S, field?)` | parents or children of S, through `par` and `idx` | `has`/`inside` one level, field-qualified |
| `Desc(S)` / `Anc(S)`, with `stop T` | subtree or ancestor closure: a mask pass, or a per-member interval/climb; gated by T | `has`/`inside` with `stopBy` |
| `Sib(S, rel)` | sibling order through `(par, idx)`; preceding/following through preorder | `precedes`, `follows`, `nthChild` |
| `∩ ∪ ∖` | bitmap or posting algebra | `all`, `any`, `not` (∖ only across strata) |
| `Join(R, S, on)` | hash or CSR join on columns: node, `tid`, doc interval | captures, `$A == $A`, cross-file joins |
| `Count(S, axis) cmp n` | prefix counts in preorder | `count` |
| `Kernel(P, S)` | bounded candidate kernel: a fused branch-free test of each member of S | `$$$` sequences, local field tests |
| `Fix(stratum)` | semi-naive waves to fixpoint; lattice components with registered transfers | recursive utilities, closures, types, reachability |

**The bounded candidate kernel** is what remains of the transducer. It is a fused program over one candidate with:
- `KIND`/`FIELD`/`TEXT_ID` tests and fixed-depth child steps;
- a bounded NFA over a sibling list for `$$$` patterns;
- `IN_SET` reads of materialised relations, in place of `DESCEND`/`FOLLOWING`/`REPEAT` over unbounded ranges.

It has no `FORK` over unbounded navigation and no continuation state. It is Tempo's "register class" and workgroup NFA, and M2's root trie, made the only form. Tempo's 32-bit word encoding, verifier and CPU interpreter/JIT carry over as the kernel encoding and its oracle.

**Compilation steps:**
1. Lower patterns and Datalog to the operators.
2. **Hash-cons across every rule in the run** (M2: 111,549 rule expansions became 1,629 operators), so shared sub-rules and utilities are one node.
3. Stratify (Tarjan; negation and aggregation only between strata).
4. Hand over to the planner.

The DAG is cached by (rule-set digest, compiler version).

## 4. The planner

The planner turns the DAG into a physical plan from statistics read from SQL in one statement, with a fingerprint check:
- `kfreq`, `efreq`, subtree size per kind, postings counts;
- column moments, with skew as the sum of squared postings;
- tag frequencies.

These are built at ingestion and merged per batch (Typing `stats.py`; spike `t_kfreq`/`efreq`/`npost`).

**Decisions,** each recorded with the statistic values that drove it:
- **Anchor:** the rarest mandatory base relation, chosen by Selinger DP over subsets of the join (spike topics 3 and 5, generalised in Typing round 2). This replaces DLRE's SQL stage programs for anchor selection.
- **Axis direction:** climb or descent by cost. A climb costs rows × expected parent reads, from the absorbing Markov chain over `efreq`; a descent costs rows × mean subtree size (Typing round 2; 0007).
- **Mask pass or per-member walk:** a pass when Σ(subtree scans) > nodes · c_pass / targets_per_pass (fusion.md). One pass carries the target bits of many rules.
- **Bitmap or list,** by expected density.
- **Fusion:** chains with a single consumer fuse into one kernel; conjuncts are ordered by cost/(1−selectivity) (fusion.md, M2).
- **Set-algebra rewrites:** containment, union of kind restrictions, anchored-first conjunctions, neighbourhood for unanchored inner plans (PIAT shape-fixes §15–§16, decision 0007). These are the same algebra, moved out of SQL stages into the plan.
- **Feedback:** observed fanouts per binding pattern replace estimates once 32 bindings have been seen, and probes are replanned between chunks of waves (Typing round 6: no probe over 67 ms, where one estimate error had cost 8.6 s).

**Enforced as compiler checks** (each cost 10–90 s when missed; Typing round 2, ledger L11):
- the planned order is enforced;
- walks are threaded, not joined back;
- columns are typed;
- every access pattern has an index.

**Plan cache** key: (DAG digest, statistic decision buckets). A rule recompiles only when a decision would flip. Compile time is reported apart from execution.

## 5. Execution: one plan, three bindings

Each operator has exactly the same semantics in three bindings. Each is differentially tested against the others and against the native matcher's finding set under the loose contract.

| | Unit | Recursion |
|---|---|---|
| **Metal** | one dispatch per fused operator per batch, grid sized on the GPU by indirect arguments | `Fix` as GPU-scheduled waves: one-thread `prep`, hybrid handoff to a 1,024-thread threadgroup loop at ≤ 1,024 frontier nodes, ICB only where the kernel choice depends on data (metal-enqueue). The CPU encodes the plan once and submits; it makes no per-wave or per-file decision |
| **CPU** | one vectorised loop per operator per batch, partitioned across cores by node range | `Fix` in the native set engine (Typing program mode: arena, CSR, join probes, wave chunks with replanning); lattices by semi-naive waves |
| **SQL** | one statement per operator or fused group, over SQL views of the same columns and postings | semi-naive delta tables (rdl SQL backend). This is the pure-SQL fallback, generated from the same plan |

Strata and relations stay resident between operators; nothing is written to SQLite except outputs and cached derived relations (Typing round 6). Kernel techniques from Tempo carry over unchanged:
- cooperative scan;
- 256-row tile sort;
- typed merge;
- sealed columns;
- first-byte value proofs;
- count → scan → scatter allocation;
- the resource-budget harness.

**Launch arithmetic.** Launches are (fused operators + waves) per batch, independent of file and root counts. A plan of about 2,000 hash-consed operators fused to a few hundred kernels, at 1.2–7 µs per launch slot (metal-enqueue §4), is on the order of milliseconds per batch. Today's form is ~20 dispatches per generation, multiplied by generations.

## 6. The three targets on this one shape

**Audit (1–2 s warm).** Shown as derived estimates, not measurements. Inputs: data-layer gate 28.23M nodes; Typing's rdl run at 1.2M nodes.

| Stage | Unit cost (source) | Gate estimate |
|---|---|---|
| Structural operators, CPU | mask pass 6.4–17.3 ns/node per pass (fusion.md); kernels 0.26–0.40 ns/program-node (M2) | 0.2–0.5 s per pass on 1 core; parallel by node range |
| Structural, Metal | 1.57 ms for a 27-wave fixpoint over 5.4M nodes (metal-enqueue) | ≈ 10 ms per fixpoint at 28M nodes |
| Relational/inference strata, CPU | rdl native: 1.19 s run phase for 1.2M nodes, 0.59 s for Rust's 1.16M | linear extrapolation ≈ 25 s on 1 core, ≈ 2 s on 16 cores; interface cutoff makes warm re-derivation a small fraction |
| Regex | per distinct text, once (`TEXT_ID`) | 0.1–0.3 s cold, ≈ 0.01 s warm (regex.md) |

**The relational strata are the gap to 1–2 s.** They need:
- Metal waves for monotone strata;
- warm reuse of cached derived relations;
- construction read from the tape and postings instead of copied (Typing round 6: construction is 1.2–1.5 s of copies today).

**Per-edit.** An edit is a delta batch, not a different execution shape:
- Local strata re-run the same plan over the edited documents' node range. Concurrent edits from every session and agent are coalesced into one delta batch.
- Derived relations are maintained incrementally:
  - retract and insert the documents' base facts;
  - propagate semi-naive deltas through the strata;
  - stop at interfaces whose digest is unchanged.

  This is Typing's interface cutoff (123 instead of 408 modules on an interface edit, 1 on a body edit) and Moti's read-set memo with early cutoff, both derived from the program rather than hand-written.
- Moti's measured shape for the maintained graph is 0.53 / 3.76 / 8.62 ms p50/p95/p99, exact on 1,151 checks. His layering (base, cluster overlays, worktree deltas) is the storage form for maintained relations across worktrees.
- On Metal, the plan's command buffer is pre-encoded and reused; empty operators cost about 3.5 µs.

**Outer SQL.**
- Every named relation and `finding(rule, doc, node)` is a narrow typed vtab (N1562's ten typed relations are the template) with declared ordering.
- `xBestIndex` turns constraints on doc, rule or node range into a demand batch through SSC's demand handle: one `xFilter` per (operator, batch).
- `estimatedRows` comes from the stored statistics.
- Warm reads come from the cached relations; cold ones run the plan over the demanded batch.

## 7. What each stream contributes, and what is retired

| Component | From | Retired |
|---|---|---|
| Tape, documents, `Origin`, embedded guests, allowances/blind files, adapters, suppressions | main (incl. Timo's NER-1698/1865/1868) | — |
| Anchor index store, 32-file publication, statistics-chosen anchors | PIAT/DLRE `matcher/anchors/sql`, DLRE S1/S4 | anchor selection as SQL stage programs |
| Statistics at ingestion, fingerprint, `fts5vocab` | Typing `stats.py`, spike, N1562/SSC readers | seeded constant statistics (N1562) |
| Datalog front end, stratifier, magic sets (with multi-column adornments still to do) | Typing `rdl` | DLRE authored `relational` SQL; Moti's hand-written query bodies |
| Planner, feedback, replanning | Typing rounds 2–7; fusion.md cost model; PIAT §15–16 rewrites | — |
| Hash-consing, tries, conjunct ordering, regex interning | M2 prototype; SSC `producers/prefix.rs`, `matcher/factor.rs` | per-root scalar `AstMatcher` in production (kept as oracle) |
| CPU set engine and lattices | Typing `sqlite-ext/setfix.rs` arena engine, `lattice/` | — |
| Metal executor | metal-enqueue design; Tempo kernels, IR encoding, verifier, budget harness | per-path generation frontier with continuation frames |
| Kernel encoding and oracle | Tempo IR, N1562 `packed/` interpreter and JIT | unbounded `FORK`/`DESCEND`/`REPEAT` execution |
| Vtab boundary | N1562 typed vtabs, SSC demand handles | 35-column match table, whole-catalog and per-scope rendered CTE networks (`relational_ir.rs`), `MATERIALIZED` fences, SSC `cohort.rs` |
| Incremental maintenance | Moti Y1 read sets and layering, Typing interface cutoff, `facts_memo.rs` | differential-dataflow maintenance (6–7 GB floor), recompute per edit (5–70 s) |
| Gates and tooling | DLRE full-repository runner and CPU attribution, Moti replay, Typing goldens, anchor lab, SSC hardware counters | exact-oracle parity (0001) |

## 8. Decisions this requires

These are to be recorded in `decisions/` with prior-work citations:

- **D1. The set form is the execution model.** It supersedes 0004 ("transducer is the concrete execution IR") in part: the transducer is the bounded candidate kernel, and every unbounded relation is a set operator. Citations: spike topics 1, 2, 7, 8; fusion.md; metal-enqueue §3; ledger L1, L16, L19.
- **D2. The Datalog fragment is the language** for named facts (0005) and for relational rules (0002).
- **D3. One plan, three bindings,** with the SQL binding generated from it. This replaces pure-SQL CTE definitions written per relation.
- **D4. The batch is the execution unit on every backend.** Per-edit is a delta batch, never per-file execution.
- **D5. Incremental maintenance** is semi-naive deltas with interface cutoff, derived from the program.

## 9. The first build step

Start from main (`5cf4246e`) and build one vertical slice through the whole shape, on CPU and Metal together:

1. The batch arena from v7 tapes and anchor postings.
2. `Base`, `Up`/`Down`, `Desc`/`Anc` (mask pass), `∩ ∪ ∖`, and `Fix`.
3. A port of Typing's planner.
4. Typing-support's W1–W3 workload (B110, SELF_ASSIGN, NO_TIMEOUT), written as pattern rules and as Datalog, over CPython `Lib/`.
5. Checks:
   - findings exact against the goldens on both backends;
   - launches per batch, which must be independent of file count;
   - execution CPU with construction reported on its own line.
