# T10: Moti's facts engine (facts-engine, facts-plan)

Research notes, read-only. Every claim cites a source. "not found" means I looked and the sources do not say it.

## 0. Refs, conventions, marks

- **E** = `origin/Moti-Velz/facts-engine`, tip `5b0d63a7` ("Merge y3-wire", 2026-10-25 21:08).
- **P** = `origin/Moti-Velz/facts-plan`, tip `a582db97` ("facts: Y3 wired path in the decision table").
- Merge base: `31a6208b`.
- Citations take the form `E:path:line` or `P:path:line`. A report citation names its section (`§`). For a file that is identical on both refs, I cite E.
- **What exists on only one ref:**
  - Only on P: `docs/facts/DECISION-CROSSFILE.md` and `docs/facts/reports/explore-x1..x6-*.md`.
  - Only on E: `docs/facts/PROTOCOL.md`.
- **What differs between the refs:**
  - `IR.md`: E is newer. It has the SQLite 3.53.2 typing rules for BLOB, FLEXNUM, compound affinity and UNION representatives.
  - `REWRITES.md`.
  - `SCRATCHPAD.md`: P is newer. It adds the exploration-wave, Y1/Y2/Y3 and G3b.1 rows.
  - `explore-y2-local.md`: E adds the Y1-wired rows.
  - `phase-3-p3b.md`: E adds a "Continuation (P3bScaling-2)".
  - `phase-3-p3c-daemon.md`: P adds the P3c-2 continuation.
  - `spike-b.md`.
- **Marks on numbers:**
  - **M**: the source reports it as measured on a run.
  - **D**: derived by arithmetic from measured numbers.
  - **A**: estimated, assumed or inferred. This includes any number the source tags `[INFERENCE]`, "est." or "estimate".
- **Time numbers.** Per-edit latencies are wall-clock in every source that states the clock. Batch costs (G2.3, A2, native's 393 s) are process CPU (user+sys). I copy each number as stated and do not convert wall to CPU. Most sources flag machine load: many runs were at 1-minute load 10–70 and marked "untrusted" by Moti's own protocol (PHASE-1 § Measurement: three runs, bench-lock, load < 8).

---

## 1. Source: INVARIANTS.md (E, same on P)

Thesis, verbatim (E:docs/facts/INVARIANTS.md:3): "**SQL in, facts underneath, one algebra in between.** SQL (and the pattern DSL) is the only rule language. Facts are the only data. One logical algebra with SQLite's semantics is the only thing the engine reasons about. Speed comes from a columnar physical algebra, and from memoization and incremental maintenance that are properties of the algebra, not of individual rules. Nothing is handled by recognizing its text, its stage name, or its author."

The document is binding from Phase 2 and overrides PHASE-2.md (line 5). The pipeline (lines 9–17):

rule → logical algebra (I1) → rewrites (I4) → dependency typing (I6) → physical plan compiled once (I5) → per-content memoised (I7) | membership-driven incremental (I8) → findings and refusals (I9) → gate (I10).

| Id | Content (abridged; verbatim fragments quoted) | Forbids (verbatim) | Line |
|---|---|---|---|
| I1 | Semantics are SQLite's, pinned to 3.53.2 / rusqlite 0.40.2. Where SQLite's own result depends on scan order (unproven window order, `LIMIT` ties, bare `GROUP BY` columns, ordered aggregates), the construct is runtime-checked: "a real tie is a named, retractable refusal, never a plan-dependent guess. The reference evaluator defines truth" | "'Close enough' results; tie-breaking that depends on the plan; emulating SQLite's scan order" | 23 |
| I2 | Facts are relations with declared structure: keys, FDs (`id → subtree_end`, `child → parent`), preorder order and its interval property, cardinality bounds. Derived relations are declared by the language (DECLARED-FACTS.md). Inference from rule text is "only as the compatibility path" | "Hand-written extractor code for anything a view can state; undeclared structure that a rewrite silently relies on; recovering a shared relation by recognising rule text when the language could declare it" | 24 |
| I3 | After the parser, only algebra. Matcher inputs are algebra over `node`/`child`/`field` | "Shape-recognized substitutions; kernels selected by name; per-rule fast paths" | 25 |
| I4 | Each rewrite is (pattern, side conditions, justification, differential test, mutant), registered in REWRITES.md | "A rewrite that fires on a name; a side condition that is assumed instead of checked" | 26 |
| I5 | "One physical algebra, columnar, planned once." Operators: scan with pushed predicate, key lookup, hash join, merge join, interval probe, semi/anti join, group, window over a declared total order, semi-naive fixpoint, memoised per-node predicate. "Plans are compiled once per rule at pack load, deterministic, printed by `EXPLAIN`, identical for every file; the plan audit rejects inner scans." | "Per-statement planning at run time; `Box<[V]>` rows; a plan that differs between files" | 27 |
| I6 | Classification is a type. A per-content plan "physically cannot reach a membership relation: its `FileFacts` handle has none" | "A flag that marks a plan per-file; a per-file plan that reads `calls`, `reference_edges`, closure facts or another content's facts" | 28 |
| I7 | Memoisation by expression digest: `(content hash, schema version, plan digest)`. "Nothing is recomputed for a known key anywhere in the fleet." | "Caches keyed by path, worktree, or rule name; results that depend on evaluation order" | 29 |
| I8 | Incrementality is generated by one transformation: bilinear join expansion, count-based aggregates and antijoins, semi-naive fixpoint demand-driven from targets. Membership is per-path base + cluster bases + worktree deltas; state ∝ divergence | "Hand-written delta rules in production; per-worktree state proportional to the tree; `distinct` over `member ⋈ facts`" | 30 |
| I9 | Exact or refuse | "Approximate matching; silent native fallback; narrowed semantics presented as the rule" | 31 |
| I10 | Admission by gate: one command, ≤ 10 minutes | "Merges on parity numbers alone; 'we'll add the test later'" | 32 |

The doc also lists the Phase 1 mechanisms it retires (lines 34–42):

- A3's `subst.rs`, with its kernels selected by name.
- A3's relation memo, replaced by I5's memoised per-node predicate.
- The OR-branch probes, which become an I4 rewrite.
- Spike B's hand-written delta expansions, which become test vectors.
- "bind this stage to that fact relation", which becomes R11 selection sharing, admitted only under subtree closure.

What is outside the engine (lines 44–47): "Rule shape. If a rule insists on recomputing a relation per clause over a clause-specific subset in a way no side condition admits, the engine computes it exactly and the cost is the rule's. That is a rule-authoring conversation (Jeremy), not an engine patch."

## 2. Source: PLAN.md (E, same on P)

- **Goal** (E:docs/facts/PLAN.md:7): "hundreds of complex data-layer rules … across ~40 parallel worktrees, exactly, under 1 GiB resident, with per-edit latency ≤ 100 ms for per-file findings and ≤ 500 ms for cross-file findings. Per-edit cost must grow with the edit, not with the rule count."
- **Measured basis**, all M as cited by the plan to its own sources (lines 11–23):
  - 262,567 `.py` files across 47 worktrees, 7,091 distinct contents (37×).
  - Batch 6 native: 323 s wall, 393 s CPU, 28.6 GiB.
  - The Python oracle runs batch 6 in 23.4 s.
  - The global phase takes 267 s: 198 s of SQLite stage builds (164,742 builds), 45 s of input re-extraction (81,532 calls), 13.5 s of setup and 1.8 s in the producer.
  - Building each (stage, file) once would save 0.08 s.
  - Cause of the 28.6 GiB: "all 11,044 per-file jobs collected (9.85M rows with JSON maps) before any SQL runs".
  - Edit cost: 50.6 / 20.2 / 12.3 s.
  - The native matcher costs 0.4–1.4 ms per rule per file.
  - Jeremy's `Facts::build` costs 27 ms per MB of source.
  - Store compaction: 204 → 93 s cold.
  - "SQL set-composition engine (done, parked) | exact; ~3× native cold; never faster on Jeremy's rules; loss was SQLite per-statement and B-tree cost" (line 23).
- **Architecture summary** (lines 27–36), in six points:
  1. One logical algebra with SQLite semantics from two front ends. Speed comes from general rewrites and "a columnar physical algebra planned once per rule".
  2. Facts are relations with declared structure.
  3. Classification is a type.
  4. Memo by `(content hash, schema version, plan digest)`; incrementality is generated.
  5. One daemon.
  6. Exact or refuse; admission by gate.
- **Decision log, verbatim** (line 82): "SQL set-composition engine retired as an executor: exact, but ~3× native on our rules and never faster on Jeremy's 60 (dl60). Native stays the production path until Phase 5."
- **Phase 5** (line 72): "retire the SQLite global phase and the SQL executor".
- **Roles** (lines 76–78). Jeremy "owns the relational executor and the rules … rule semantics are never changed by us."

## 3. Source: PHASE-1.md, spike-a.md, spike-b.md (Phase 1)

**PHASE-1 contract** (E:docs/facts/PHASE-1.md):

- Content hash: blake3-256.
- Node ids: `u32` preorder over the tree-sitter parse, root 0. Descendants are exactly `(id, id+subtree_size]` (line 29).
- Strings are interned per process as u32.
- Facts are columnar. Text comes from the source slice.
- Fact file format: `RRFACTS\0`.
- Evaluator (line 59): "Rules as Datalog over facts. Preferred: the `ascent` crate … No SQLite, no per-statement planner. Keep each rule's cost measurable on its own."
- Kill criterion (line 77): "A2 between 10% and 25% with A1 passing: the evaluator replaces only the SQLite stages; pattern matching stays native."

**Spike A** (E:docs/facts/reports/spike-a.md):

- Census: 775 units = 418 (54%) rebuild the syntax tree in SQL + 316 rule logic over facts + 41 needing facts outside v0 (P:SCRATCHPAD.md:162).
- Not `ascent`. Spike A uses its own SQLite-subset parser (`sqlparser` 0.58 fails 15 of 452 statements).
- Executor, verbatim (spike-a.md:234–242): "The plan is fixed by the text: FROM order, index nested loops; equality conjuncts … probe hash indexes built lazily once per table; other conjuncts are placed at the earliest bound level; correlated subqueries probe the same indexes, and uncorrelated ones are cached per statement; recursive CTEs run level by level (FIFO-equivalent); SQLite affinity and comparison rules; CTAS affinity conversion. There is no SQLite and no per-statement planner at run time."
- OR probes (line 281): "SQLite's OR optimisation as a fixed, text-determined rule, not a cost-based planner."
- Order caveat (line 288): ties are broken by input order, and no replayed call diverged.
- Gate results (P:SCRATCHPAD.md ledger), all M:
  - A1: 9/10 rules exact.
  - A2: 212.9 s CPU = 54.2% of native.
  - A3: 152.6 s CPU = 38.8% of 393 s.
  - A4: extraction p50 0.53 ms; facts 8.3× the source size.
  - A5: 8.46 GB.
  - Util memo: 1,616 s → 15 s.
  - A3 found 34.9 s of byte-identical stage text recomputed per clause.
  - Fact file: 34 B/node (v0).
- The kill criterion in PLAN's coarser form, "< 4× win → evaluator replaces only the SQLite stages", was triggered: the win was 1.85× / 1.94× (spike-a.md:80). The decider overrode it with GO.
- The GO rationale (P:SCRATCHPAD.md:80) centres on the product targets:
  - 16.9 ms per file;
  - cross-file 0.4 ms vs 12–50 s per edit;
  - fleet memory 195 MB for 40 worktrees vs 2.5 GiB / 28.6 GiB.

**Spike B** (E:docs/facts/reports/spike-b.md): base + delta differential dataflow over three stand-in symbol-level rules.

- Tag algebra and R1–R3 delta expansions, written by hand. They later became the generator's test vectors.
- Exported schema v0: def, ref, import, call_edge, ….
- Memory: base 255 MB vs clusters 195 MB for 40 worktrees (M). The naive variant held 5.7× more records (M).
- Extraction p50 0.98 ms (M). Divergence of 104–818 paths per worktree.
- Findings are keyed by symbol, not line: carrying lines churned records (9.73M → 6.24M; M).
- The SCRATCHPAD later says this did not predict the real cost (§ 11).

## 4. Source: PHASE-2.md and the Phase 2 slice reports

**PHASE-2 gates** (E:docs/facts/PHASE-2.md):

- G2.1: 60/60 compile + leak test.
- G2.2: parity.
- G2.3: batch-6 CPU ≤ 90 s "with no shape-recognized substitution left" (line 38).
- G2.4: repository plans on the dataflow, ≤ 250 MB for 40 worktrees.
- G2.5: memory.
- G2.6: gate strength.

S2's deliverable includes "memo keys `(content hash, schema version, plan digest)` and completion words; streaming with nothing accumulated across files" (line 51).

**S1 (IR, classifier, census)** (E:docs/facts/reports/phase-2-s1-ir.md):

- The IR is relational algebra over typed SQLite columns (§ 5.1). Classification is a type parameter: `Rel<FileSource>` vs `Rel<RepoSource>`, with a `compile_fail` doctest.
- `Program = { inputs, per_file, repository, exported, report_when, runtime_refusals }`. It is `Send + Sync`, with one `Arc` per process.
- Census (M):
  - Wave 1: 20/60 compile with Spike A's leaf executor; 55/60 at IR level.
  - Wave 2: 55/60 with S6's leaf executor.
- Sharing census (§ 5.7):
  - 60 groups / 224 stages of byte-identical text;
  - 15 groups / 30 stages with equal IR digest;
  - 26 groups / 57 stages with equal IR shape.
- The order audit (§ 5.2, § 8.1) became `ir::Total {InputOrder, Proven, Checked}`. IR-level count: 23 constructs, 2 proven, 21 checked.
- Risk (§ 8.3): "No input key is declared, so 21 of 23 order constructs stay `checked`."
- Physical EXPLAIN (§ 8.1, M): 1,150 files, 199,291 stage plans; "50 clauses each with exactly one plan text across all files". Two runs are byte-identical.
- The inner-scan audit found 2 plans (`block_results`, `function_symbols`).

**S2 (per-file backend)** (E:docs/facts/reports/phase-2-s2-perfile.md):

- Columnar layer (§ 4 step 1):
  - `Column::{Vals, Gather, Chain, Text}`, with a gather view sharing one row-index vector.
  - `SELECT n.*` stores one u32 per row.
  - Semi-naive fixpoint over columnar batches with `[u32;4]` keys.
- Shared memos per (file, policy set, utility table, valuedness), with a taint guard: "a verdict whose evaluation refused or decoded a `\N{...}` escape is never memoised" (line 60).
- Wave 2 compact columns:
  - `Column::{Const, U32, Sym, Interval}`.
  - Hash indexes without stored keys: 4 B/row + ~32 B/key.
  - 2-bit `Trits` memo cells.
- Effect: largest-file per-worker peak 607 → 230 MB (M, generic path).
- Compiled rules: one `Arc<[Rule]>` per process, 29–31 MB live heap (M).

**S7/S7b (one pass, `subst.rs` retired)** (E:docs/facts/reports/phase-2-s7-onepass.md):

- Correction (§ 1): A3's dump groups cost ≈ 59 s, not the 10 s in the ledger (M, in the decider's quiet pair).
- Pattern inputs are lowered to algebra: 340/622 inputs lower (§ 3).
- Predicates are hash-consed per rule and memoised per file over the nodes evaluated (R12/R13).
- `subst.rs` deleted (§ 7.1). R14 range probe: 114.2 → 106.5 s CPU (M, load 4.4–5.6, two runs).
- Full-corpus `pattern-check`: 0 mismatches on 25,427,160 rows (M).

**S8/S8b/S8c (engine levers, SQLite rulings)** (E:docs/facts/reports/phase-2-s8-perfile.md):

- Borrowed cells and key cells; R16 (text node ids as parts); R17 (constant key columns).
- CPU 104.7 → 93.7 s (M, two quiet pairs).
- R18 tree closure: `descendants` 5.37 → 4.22 s. R19: no gain.
- S8c (§ 10.1, M):
  - Rulings (c) (bare columns) and (d) (compound affinity), checked by random differentials against SQLite 3.53.2 (3,000 + 4,050 cases).
  - G2.3: 91.70 / 92.07 / 92.28 s CPU at load ≈ 5.
- Open finding (§ 10.3, 1), for the decider: "Ties of the one extremum across join rows". The reference and the engine take the first row in text-order nested-loop order, SQLite in its own join order: 32 of 150 random DBs differ. This is a residual plan-order dependence against SQLite.

**S3 (cross-file generator)**, E:docs/facts/reports/phase-2-s3-crossfile.md. Summarised in § 9 below.

**S4 (schema)** and **S5 (gate)**: §§ 5 and 12 below.

**S6 (leaf executor)** (E:docs/facts/reports/phase-2-s6-leaf.md):

- 55/60 compile clean with the leaf executor (header).
- Waves 2–3 added `binds`, budget-evidence `closure` and `printf`.
- S6b found a derived-table correlation-depth bug (`public_api_triples`), fixed with mutant 22.

**S10 (loop)** (E:docs/facts/reports/phase-2-s10-loop.md): see § 6 (memo) and § 12 (gate).

**Phase 2 exit** (P:SCRATCHPAD.md:102), verbatim highlights:

- "per-file engine exact on batch 6 and on 48 of 50 batch 1–5 checks, 90 s on the batch-6 corpus (native 393 s), 3.1 GB peak (native 28.6 GB), no shape recognition left";
- "cross-file generator exact and incremental on 5 of 8 repository clauses with the reference graph (145,431 edges) at 62 ms p95 per edit over 40 worktrees; a gate of 29 checks and 41 mutants";
- "Not achieved: cross-file memory (30–70 GB against 250 MB)".

## 5. Source: SCHEMA.md and phase-2-s4-schema.md (declared core facts, storage)

**Core facts, v1** (`SCHEMA_VERSION 1`, path-independent; E:docs/facts/SCHEMA.md):

- Node ids: u32 preorder over every tree-sitter node of the native tape, named and anonymous (line 8).
- `node` columns (line 19): `id:u32` (row), `kind:sym`, `parent:opt_u32`, `start:u32`, `end:u32`, `start_line:u32`, `end_line:u32`, `subtree_end:u32`, `flags:u8`, `field:opt_sym`.
- Declared structure (line 46):
  - key `id` (dense `0..n`);
  - FD `id →` everything;
  - "preorder: `id` ascending = document order";
  - "**interval**: `d` is a proper descendant of `x` iff `x < d < subtree_end(x)`";
  - `parent(x) < x`.
- Descendants are "the id range …, never a relation" (line 38).
- Other core relations: `child`, `field` (with `rank`), `identifier`, `line`, `syntax_error`.
- Parser facts: `string_value`, `string_value_unknown`, f-string parts, `string_type_definition` (from native's Ruff typedefs classifier), `py_binding`.

**Derived relations as views** (SCHEMA § 2; S4 § 5.1):

- 15, then 20, Python-derived views written as normative SQL (`-- view:` lines), e.g. `py_node`, `py_field`, `py_owner`, `py_function`, …, `py_context`.
- Unresolved call-graph views: `cg_scope/def/call/import/refusal`.
- Native call-summary exports: `cgn_*` (§ 3.4), with symbolic module paths substituted on the membership side.
- Differential, views vs the policy stage texts: **0 multiset differences** over 9,010 files, 699,620 file-checks and 236,564,067 rows (M, S4 § 2).
- Cost: views via the row interpreter 66.5 s summed (M, untrusted load). That is more than the 34.9 s of shared stage text they replace (S4 § 5.1).
  - With S2's columnar layer, 15 views take 68.7 → 42.0 s (M, W2.3).
  - `py_import_identifier` got slower: 8.7 → 10.3 s.
  - S4's own warning: "If these are not lowered, binding the views per clause would cost more than the stage texts do today."
- Binding caveat (SCHEMA.md:225): `runtime` and `string_values` join "**by name across the node set** … selection sharing by semijoin does not apply to these two stages even under subtree closure."

**Fact files and storage** (SCHEMA § 5; S4 § 5.2; `spikes/facts-a/src/factfile.rs` header, lines 1–12):

- One file per content hash: `<store>/<hash[0..2]>/<hash>.facts`.
- Layout: magic `RRFACTS\0`, u32 schema_version, the 32-byte blake3 hash, a string table, then relations of typed columns (u32/u16/u8/sym/opt_u32/opt_sym, little-endian, `opt` none = `u32::MAX`).
- v1 width: `kind:u16` (string index), `field:u8` (string index + 1). `parent`, start/end lines and the children CSR / kind postings are derived on read.
- **Anonymous nodes are stored.** They are 34% of nodes; dropping them saves 15% raw and 5.3% compressed, and every read would become a parse (0.70 ms vs 0.08 ms) (S4 § 5.2).
- **zstd level 3 chosen.** Measured (M):

  | Layout | Total | Per source byte | Per node | p50 per file | Write p50 | Read p50 |
  |---|---|---|---|---|---|---|
  | v1 + zstd 3 | 186.4 MB | 1.35 B | 6.6 B | 9,574 B | 0.109 ms | 0.081 ms |
  | v0 | 1,048.4 MB | — | — | — | — | — |
  | v1 raw | 532.3 MB | — | — | — | — | — |

  Level 9 gains 4.5% for 2.8× the write time (S4 § 5.2 table).
- `string_value` is not stored. It is recomputed on read, p50 0.012 ms (S4 W2.3).

**In-memory width and interning** (`spikes/facts-a/src/facts.rs`):

- `Facts` holds `Vec<u32>` columns `kind, parent, start, end, start_line, end_line, subtree_end`, plus `flags: Vec<u8>` and `field: Vec<u16>` (lines 70–85).
- A children CSR, kind postings (CSR) and the source `Arc<String>` (lines 86–99).
- `string_type_definition` and `py_binding` are lazy `OnceCell`s (lines 96–99).
- Kind and field names are interned process-wide, `Symbols` behind a `Mutex`, `intern(name) -> u32` (lines 13–37), with per-thread name caches.
- `SCHEMA_VERSION: u32 = 1` (line 11).

## 6. Source: DECLARED-FACTS.md (the declared facts proposal)

Status, verbatim (E:docs/facts/DECLARED-FACTS.md:3): "**proposal under evaluation, not scheduled.** Written 2026-10-23 as the decider's reading of Jeremy's suggestion; no implementation decision has been taken." It amends I2.

Origin: "34.9 s of byte-identical stage text recomputing the same relations per clause over clause-specific subsets; selection sharing admissible only under a subtree-closure side condition".

**Language surface** (lines 13–48). A `facts` block at pack or policy level, one entry per named relation:

- `over`: `"content" | "membership"`. This is the author's claim; the compiler infers the class from the leaves (I6) "and refuses a mismatch" (line 36).
- `columns` (typed), `key`, `fd`. Keys and FDs are checked: "provable ones at compile time through the IR's metadata; the rest at runtime per content (a duplicate key is a named refusal of that content's fact, never a silent dedup)" (line 37).
- `define.sql`: SQL over core relations and other declared facts (line 34).
- `define.match`: a pattern-DSL matcher "over all nodes of the content, with a projection" (line 35).
- `version`: part of the digest (line 38).
- Example entries: `py.parameters` (content, sql), `py.import_identifiers` (content, match) and `proj.module_of` (membership, sql) (lines 17–31).
- Rules read facts by name. A clause's selection becomes "an ordinary semijoin. The subtree-closure side condition disappears because nothing is being rewritten; the author wrote the semijoin" (line 48).

**Semantics** (lines 50–59):

- A declared fact is "compiled once at pack load, `EXPLAIN`-able, audited, rewritten by the registry, exact or refused".
- **Identity:** "`(fact digest, schema version)` where the digest covers the definition's algebra after lowering, not its text; two packs declaring the same definition under different names share one computation and one memo entry" (line 53).
- **Per-content facts:** "memoised by `(content hash, fact digest, schema version)` in the content store, with completion words; a fact is computed once per content across the fleet" (line 54).
- **Membership facts:** "compile to the dataflow (delta plans generated, base + cluster + worktree tags) … per-content facts they read are exported automatically (projection and distinct pushed down)" (line 55).
- Per-content rules may read per-content facts only (line 56).
- **Order:** "no order unless it declares a total one … otherwise order-dependent consumers are `Total::Checked`" (line 57).
- **Cost** is visible in the census and EXPLAIN (line 58).
- **Legacy:** "Inference is the compatibility path, not the design" (line 59).

**Expected effect** (A): "12 declared facts computed once per content … Expected: most of that 34.9 s, minus the semijoins (estimate; S9 measures)" (line 63). S9 was cancelled before its first commit ("the user asked for a judgement on the claim, not for work", P:SCRATCHPAD).

**Open questions for Jeremy** (lines 67–74):

1. Namespaces.
2. Spans and text vs node ids only.
3. Versioning.
4. A `py.*` library pack.
5. Budgets: "named refusal of that content's fact for all readers is the current rule".
6. His compiler as the `define.match` front end.

**Width/interning for declared facts:** not specified beyond question 2 ("ids keep facts small and let consumers read text lazily from the source"). No storage format for declared facts was found; only the core v1 fact-file format exists.

**Implemented form:** only by hand, in X6 (§ 10.6). X6 used 23 per-content facts in DECLARED-FACTS form plus 12 `proj_*` membership facts, bound by `link.py`; no compiler support was found.

## 7. The content-hash memo (code + LOOP + S10 + P3c)

**Key.**

- LOOP.md:26,30: `(content hash, plan digest, schema version)`. "A rewrite that changes three plans re-runs three plans."
- I7 (INVARIANTS.md:29): `(content hash, schema version, plan digest)`.
- P3c/PROTOCOL/facts_memo.rs: `(content hash, plan digest, fact schema version[, path])`. See `native/src/content_store/facts_memo.rs:1-2` on E, and E:docs/facts/PROTOCOL.md § State.
- Path rule (facts_memo.rs:7–10): "A clause whose result is a function of the content alone (the engine decides, statically, per clause) is stored under the empty path … A clause that reads the path … is stored under its path."
- In code: `FactsMemoKey { source: SourceId, fact_schema: u32, path: &str }` (facts_memo.rs, struct after line 60). `CONTENT_PURE = ""`.

**Plan digest** (`spikes/facts-a/src/memo.rs` on E, module doc lines 19–28; `digest()` ~lines 81–98):

- blake3 over the JSON of: `format` (`"rerail.facts-memo/v1"`), `engine` (`ENGINE_VERSION = 2`), `schema` (`facts::SCHEMA_VERSION`), `rewrites` (`lowered::rewrites()`), the resolved rule without clauses (id, scope, options), its `define` clauses (utils, `report_when`), the call's scope, the resolved clause with its evidence inputs, the physical plan text of every pattern input (`lowered::Plan::digest_text`) and the physical EXPLAIN of each relational stage, and the algebraic budget `A`.
- Verbatim: "A rewrite that changes a plan changes that plan's digest and no other. An operator whose results change while every plan's text stays the same must bump `ENGINE_VERSION`." The comment on `ENGINE_VERSION = 2` reads "P3b: K1 key checks and algebraic budgets changed fixpoint and budget behaviour (plan text unchanged)".
- Plan set (facts_memo.rs, `FactsPlanSet::new`): sha256 over `"rerail.facts-plan-set/v1\0"` + count + the ordered plan digests.

**Granularity:** one clause's per-file result on one (content[, path]) (memo.rs:1–3).

**What is cached** (memo.rs:7–11; `Entry`):

- findings, failures, counts (per-file part), the repository clause's per-file input rows (`spill::FileRows`), and budget rows.
- "A clause run with nothing to report is a completion bit of the run's plan set with its counts; anything else is a result row holding the encoded entry."
- Store tables (native `content_store/schema.rs` ~lines 276–310 on E):
  - `facts_plan_sets(set_digest, ordinal, plan_digest)`;
  - `facts_results(source_id FK ON DELETE CASCADE, fact_schema, path, plan_digest, codec_id, raw_length, frame)`, PK (source, schema, path, plan);
  - `facts_completions(…, set_digest, words, counts)`.
- Comment in schema.rs: "A clear bit is unknown, never absent."
- `put_facts_memo` ORs completion bits into words, and `merge_completion` merges them. Frames are zstd-compressed.
- Lookup (facts_memo.rs:16–18): "a lookup resolves completion bits of every set the content was evaluated under to plan digests, so a set that differs in a few plans still reuses every other clause's result."

**Content-pure path elision** (memo.rs:13–17): a finding's, failure's and budget row's path, and the first column of repository rows, are elided and "filled in on read, so identical content at two paths is evaluated once". The summary notes a panic tripwire on the elision; I read this in the code before compaction and did not re-verify it after.

**Invalidation:**

1. **By key change.** Any change to plan text, rewrite set, schema version, budget A or engine version gives a new digest, so stale rows never match (memo.rs:19–28, 44–46).
2. **ENGINE_VERSION bump by the integrator** when operator semantics change under unchanged text. S10 decision (P:SCRATCHPAD.md:101): "T1 runs `--no-memo` automatically when operator files (`sql/engine.rs`, `lowered.rs`, `ast.rs`, `rel.rs`, `eval.rs`, facts-b `gen/*`) changed since the merge-base; `memo::ENGINE_VERSION` is bumped by the integrator at merge; the gate is always `--no-memo`."
3. **By content reclamation.** Every row cascades from `source_objects`. The daemon pins the sources of held contents (owner `rerail-facts-daemon`) and unpins them when no pair holds them, and "the store's collection then reclaims them, their memo rows cascading" (PROTOCOL.md § State; facts_memo.rs:4–6, 264–299).
4. **Mutants:** the memo is off under `FACTS_MUTANT` and `--no-memo` (memo.rs:30–32, `store_path`).

There is no time- or LRU-based eviction in the memo itself (not found).

**Measured** (S10 report, M):

- sub200: cold 22.5 s, warm 3.5 s.
- Corpus: cold 51.2 s wall (180 s CPU), warm 19.6 s; 77 MB of memo.
- "A rewrite change re-ran 9/105 digests."
- P3c: an unchanged re-apply does 0 work (P:phase-3-p3c-daemon.md G3c rows).

## 8. Source: PHASE-3.md, P3a (memory model M1 vs M2), P3b, P3c, PROTOCOL.md

**PHASE-3** (E:docs/facts/PHASE-3.md):

- P3a: cross-file memory model, M1 (full maintenance, compacted) vs M2 (hybrid). Gates:
  - G3a.1: ≤ 250 MB at load.
  - G3a.2: ≤ 300 MB after 1,000 edits.
  - G3a.3: p95 ≤ 500 ms, p99 ≤ 2 s.
  - G3a.4: exact.
- P3b: scaling laws and budgets.
- P3c: store and daemon, gates G3c.1–3.

**P3a** (E:docs/facts/reports/phase-3-p3a.md):

- **M0 attribution:** 171.1M records (M); dead code 139.45M, of which the linker is 71.66M; interner 629 MB.
- **M1 steps** (M):
  - presence collapsed: 131.4M records;
  - R37 cross-plan stage sharing: 111.7M;
  - interner sweep (mutant 56): 3.15M strings, 353 MiB;
  - compact records not implemented (est. −3.1 GB: A);
  - trace compaction: 1.8%;
  - not caching rows saves 568 MB, but p95 goes 103 → 282 ms.
  - M1 result: 18.4 GB at load, 32.5 GB at 1,000 edits; 39 / 131 / 442 ms (DECISION-CROSSFILE.md:8).
- **M1 floor** (D): ≈ 112M × 130 B ≈ 15 GB. 250 MB would need 2.2 B/record.
- **M2 design:**
  - Resident: export blocks, 32 bits a value, hash-consed across exports/programs/paths, refcounted by worktrees and freed at 0. Membership. Last results per plan.
  - Each plan is recomputed from scratch by `batch::eval` over the worktree's members, in a forked child (P3a), unless D1 skips it.
  - Header verbatim (E:spikes/facts-b/src/gen/hybrid.rs:4–11): "Nothing else survives an edit: a plan's stages (the membership-dependent `module_of`, the linker views and the rule logic alike) are evaluated from scratch … and the strings that evaluation computes are dropped with it (the interner is truncated to its mark; retained rows are re-interned)."
- **M2 attribution** (M): blocks 149 MB, strings 113 MB, plans 62 MB, membership 9 MB, results 11 MB; heap 325 MB / footprint 470 MB; +22 MB over 1,000 edits.
- **M2 latency** (M): 38.8 / 48.8 / 80.6 s p50/p95/p99.
  - Dead code takes 32–34 s, of which the linker is ≈ 25 s (`link_reach` 15.8 s). The linker is 84% under contention.
  - `exported-families` 2.6 s; the other three plans ≤ 10 ms.
  - Work per edit: 55.5M rows out.
- **D1 skip rates** (M): light plans skipped 1,050/1,050; required-contracts 984; global plans 122.
- **M2 floors:** ≈ 345 MB in process; ≈ 85 MB with store-served exports (D/A). Latency would need ≈ 65× (≤ 10 ns/row) (D).
- **Key finding, verbatim** (as recorded in DECISION-CROSSFILE.md:23 and P3a): "Smallest set that must be membership-maintained for exactness: none beyond membership itself … Maintenance buys latency, not exactness."
- **Recommendation:** M2, with async delivery, cutting the work, or a demand-scoped linker. M1 is not viable.

**P3b** (E:docs/facts/reports/phase-3-p3b.md; E has the longer version):

- Operator cost bounds `Mono`/`Bound` = sums of `n^a·d^b·log^c` terms, over `Card` (rows, keys, path-constant columns) and `Analyzer` (line 26). Unbounded shapes are named: cross product, join without a usable key, join on path-constant columns, fixpoint without a decreasing measure, simple-path enumeration.
- Plan classes: O(n) 116, O(n·d) 420, O(n²) 3, unbounded 19, over 58/60 rules and 558 clauses (M, P:SCRATCHPAD.md:45).
- Measures W1/M1/M2/K1; mutants 57/59/61.
- Algebraic budget: work = match_nodes + input_rows + leaf_rows + stage_rows + fix_rows, limit A·m(class) with A = 256; corpus max ratio 51 (M).
- The exponent-lane prototype does not separate the mutants.
- Owed:
  - "K1's runtime check is not implemented … Until it lands, K1-based classes are claims, not proofs" (line 98).
  - Gate registration.

**P3c and PROTOCOL** (P:phase-3-p3c-daemon.md; E:docs/facts/PROTOCOL.md):

- **Process.** One daemon per machine. It compiles the policy set once and keeps the memo in the content store. Unix stream socket; frames are `u32 header len | JSON header | u32 body len | body`, with a 256 MiB cap.
- **Identities.** Content = blake3 (64 hex digits) or Git blob id. Paths are repo-relative. Worktree = a client-chosen name. Epoch per worktree. Finding = `[check, clause, path, line, code, column|null]`.
- **Ops:** `hello`, `sync` (commit or index entries; reads blobs from the Git ODB), `edit` (hash first; `need_bytes` if unknown), `findings`, `rebalance`, `close`, `stats`, `shutdown`.
- **Per-file vs cross-file.** Per-file clauses are answered synchronously. Cross-file clauses are pushed asynchronously as `findings` with `cross: true`, via `crossfile::CrossFile { fn apply(&mut self, worktree, members, delta: &[PathChange]) -> FindingsDelta }`; `FromScratch` is the reference implementation.
- **State.** Membership is copied from Spike B: per-path base, cluster bases by an exact DP over contiguous worktree runs, per-worktree deltas. Pairs `(path, content)` are refcounted. Findings are the members' pair results, "never a per-worktree copy".
- **G3c rows** (M):
  - unchanged re-apply: 0 work;
  - new worktree, first findings: p50 44 ms / p95 306 ms;
  - edit, first findings: p95 181 ms (fail), 24.7 ms without the largest file;
  - G3c.3: 40/40, 0 diff, 188,979 findings;
  - live heap 652 MB at 16 threads, 294 MB at 4 threads; floor ≈ 170 MB + 30 MB/thread (D); regex caches 26.3 MB/thread;
  - dominant clause `transaction-connection`: 2,991 ms (68%), scaling nodes^1.86;
  - warm restart 486 ms.

## 9. Source: S3 cross-file generator (Phase 2), the M0 baseline

E:docs/facts/reports/phase-2-s3-crossfile.md.

- **Generator** (§ 5.1). IR → delta operators (`norm.rs`) → base + delta expansions over tagged records `(tag, row)` (`lower.rs`). Tags are base 0, cluster bases and worktrees.
  - Join: `product(t1, t2)`, the deeper tag of one chain.
  - Presence: per-key counts per tag.
  - GROUP BY / window: group recompute, out(t) = logic(view(t)) − logic(view(parent(t))).
  - Set recursion: initially per touched view; in wave 4, a delta-mode `iterate` (§ 11.1).
- **Runtime refusals** are dataflow records `(t | REFUSED, [construct])`, maintained per view. They withhold the plan's findings (§ 8.1).
- **Membership model** (§ 5.2):
  - per-path base = "each path's most frequent content among the paths held by a majority";
  - cluster bases by exact DP over contiguous runs of worktrees in commit-date order;
  - resident exported rows refcounted by `(tag, path, content)`.
- **Module paths** (§ 6). Native summaries qualify names by module path, which depends on the worktree's `__init__.py` set. So "Exports must stay unresolved and content-keyed; `module_of(path)` becomes a membership-dependent relation computed in the dataflow."
- **Linker as SQL** (§ 8.1, § 9.1): 79 SQL stages, 145,431 = native's 145,431 edges (M).
  - On the generator, the first base load ran 3 h 8 min and was killed (M).
  - Cause (§ 11.1): an unmemoised recursive walk, `lower::reads_var`, at ≥ 1.06e17 calls for `link_result`.
  - After the fix and the delta-mode fixpoint: 69.0 s, then 50.1 s with R15 (M, locked). RSS 9.66 → 7.43 GB.
- **Memory attribution, wave 6** (§ 15.2, M): all 5 plans load at 29,260 MB RSS, with 145.91M arranged records.
  - Linker base 61.38M (42%); dead code outside the linker 56.17M (38%); `exported-families` 28.33M (19%).
  - Duplicated identical stages: 17.81M (12%).
  - By role: keyed arrangements 44%, presence in+out 29%, fixpoint join arms 17%, group 9%.
  - Interner 600 MB, never freed; 688 MB after the replay. Exported-row cache 613 MB → 1,048 MB.
  - About 200 B/record (dead code) and 353 B/record (`exported-families`) vs 40 B/record by hand.
  - Per-worktree state ≈ 8.2 records per delta row (R² 0.92).
- **Protocol rows** (§ 15.3, M, 3 locked runs):
  - RSS at load 31.6–32.0 GB; 42.5–42.8 GB at edit 1,000; peak 70.6–71.3 GB.
  - Arranged records: 252.8M at edit 1,000, 349.7M at the end.
  - Single-file edit p50/p95/p99/max ≈ 17 / 62 / 194 / 1,320 ms.
  - Bulk p50 7.1 s; commit p50 1.7–2.2 s; new worktree p50 24 s.
- **S3's Phase 3 list** (§ 15.2 items 1–6):
  1. Prune arrangements to what delta maintenance reads.
  2. One plan graph across clauses (CSE by digest).
  3. "The linker as a shared base, not a per-plan view".
  4. One interner with compaction, and ids instead of strings.
  5. Record size: "The 250 MB target for 40 worktrees cannot be met by pruning. It needs a columnar arrangement of u32 ids, and the linker outside the per-plan dataflow."
  6. Compaction and divergence.

## 10. Source: the exploration wave X1–X6 (P only)

The common setup is in P:docs/facts/DECISION-CROSSFILE.md:3:

- 40 worktrees from `~/tempo-qe/workspaces47`;
- S3's 1,111 events (a 300-event prefix where stated);
- the 5 running repository plans;
- target ≤ 250 MB at load, ≤ 300 MB after 1,000 edits, p95 ≤ 500 ms, exact.

Memory is footprint. "All agents' numbers; none re-verified, per the mode."

### 10.1 X1 prune (P:docs/facts/reports/explore-x1-prune.md)

- Cut 1 is `reduce_total`; cut 2 is column demand. Records fell 145.9 → 104.1M (M); the linker is then 48.5% of them. 167 B/record.
- Memory 16.0 / 17.0 GB on the prefix (M); 43 / 172 / 393 ms (M, load 40–80).
- Killed: "123M records remain, all read by delta maintenance" (DECISION-CROSSFILE.md:10).

### 10.2 X2 compact (explore-x2-compact.md)

- u32 `Val` with a 2-bit tag; `RowCol`, `TagRowCol`, `Offs`, `Splice`; one evicting interner.
- 55.1 / 53.3 B/record (M): payload 39 B, rows 14.1 words. Interner 630 MB live; sweep 2.0 s.
- Memory 10.2 / 10.7 GB (M); 36 / 99 / 343 ms (M, unlocked).
- Killed: "53 B/record is the floor; 630 MB of strings stay live because records hold them" (line 11). It also breaks something: "limits panic instead of refusing".
- X1 + X2 by arithmetic: ≈ 6.7 / 7.5 GB, 25–30× over target (D, line 12).

### 10.3 X3 choose (explore-x3-choose.md)

- Rule: recompute a clause iff its state is ≥ 50 MB and its recompute p95 is ≤ 500 ms. Result: maintain all.
- Per clause (M): exported-families 31.6M records, recompute 8.8 s; dead code 139.5M records, 71 s. 88% of edits alter their inputs. A mix of all 32 combinations fails.
- Lowest mix: 1.7 GB at load, 5.8 GB peak; 38 / 43 / 43 s (M).
- Breaks I8 for the recomputed clauses (and I3 per the report).
- Killed: "dead code 37–71 s and `exported-families` 4–9 s per recompute; their inputs change on 88% of edits" (line 13).

### 10.4 X4 facts (explore-x4-facts.md)

- Design: maintain declared facts only, recompute all rule logic.
- The demand frontier is exactly one node, `reference_edges`; 898 of 1,327 nodes are behind it (M).
- Side conditions S1–S6 for the demand rewrite: 0/5 plans scoped.
- Latency: 18.7 / 21.8 / 22.9 s measured (M), ≈ 7.3 / 8.5 / 8.9 s adjusted (D/A); 92 s with the linker recomputed.
- Running it on the facts-a engine: 1,202 s (`reach_edges` 999 s, `base_edges` 120 s) (M).
- 0.31 µs per rule row; ≈ 5 s per heavy plan; ≈ 145 s for 200 rules (D).
- Memory 0.37–0.40 GB (M), "+ ≈ 12 GB if `reference_edges` is maintained by today's generator" (A).
- Breaks I3 ("fact/rule boundary by stage label") and I8.
- Killed: "no plan passes the demand-scoping side conditions; cost ≈ +5 s per dead-code-sized rule" (line 14).

### 10.5 X5 tables (explore-x5-tables.md)

- Design: no dataflow. Indexed tables plus delta-scoped recompute. The linker's incremental rule uses classes P/C/G/R, with R(n) probe sets that include absent keys, plus fallbacks.
- Rule logic 15.05 s wall (≈ 8.6 s CPU) (M); ≥ 7.6 s per edit (M/D). facts-a's `base_edges` 324 s (257.9M subquery runs) (M).
- Memory ≈ 0.9–1.6 GB (A, estimated), of which names alone are 250–300 MB.
- Breaks I8: a hand-derived delta rule.
- Killed: "global fixpoints (`reachable`, `resolved_names`) and global windows can't be scoped" (line 15).

### 10.6 X6 rules (P:docs/facts/reports/explore-x6-rules.md)

- **Facts layer, in DECLARED-FACTS form** (§ 1):
  - 11 `py_*` symbol facts plus 12 `proj_*` membership stages (`proj_inventory` … `proj_exported_roots`);
  - 9 gate-free contract facts: 637K rows vs 103K for rc's 10 gated inputs (M);
  - `py_nodes`, a 63-column gate-free node dump: 16.05M rows, mean 1,782 per content, vs 1.42M gated (M);
  - `sel_sfp`, `sel_qc`.
  - Bound by hand with `link.py` (`--mode separate|merged`). No compiler.
- **Exact** (§ 2, M): 115 outputs / 9,041,025 rows, 0 differences; dc 4,931 = 4,931; incremental ≡ from-scratch on the 300-event prefix with 0 differences.
- **Memory** (M): 19.1 GB at load, 25.6 GB at 1,000 edits (footprint), vs 26.3 / 32.7 GB for the originals. Facts alone hold 80.6M records (symbol table 19.3M, linker 61.4M). Sharing saves 11–14% of records.
- **Per rule** (M):
  - dc shape: +18.9M records (≈ 1.7 GB) on 1 worktree;
  - ef shape: +1.3M (≈ 0.5 GB);
  - rc shape: +20.5K records, 5.6–7.9 MB, on 40 worktrees;
  - so "32–45 light rules alone fill 250 MB" (D).
- **Per-file engine** (M): 0.27–0.72 ms per rule per content; facts 0.5–1.4 ms once per content; qc ≈ 1.9 ms; sfp in scan mode ≈ 22 ms per selected content.
- **What it breaks** (row, line 9):
  - "I6/I7: per-content facts carry `__path`; `py_all` reads the file path, so it is not path-free."
  - Gate-free facts are 6–11× the rows they replace.
  - Merged plan: one refusal takes down the pack; per-rule scope is emulated; facts-b `STRIDE` caps a program at 999 exports (≈ 120 rc-shaped rules).
- **Verdict:** "Killed for full maintenance at load … **Alive** as the per-file side: linear, cheap per content, facts paid once."

## 11. Source: DECISION-CROSSFILE.md (P only), the decision table

The table, verbatim in substance (P:docs/facts/DECISION-CROSSFILE.md:5–16). All latencies are wall-clock.

| Design | Memory, load / after edits | p50 / p95 / p99 | Exact | Breaks | Verdict | Mark |
|---|---|---|---|---|---|---|
| M0 full differential-dataflow maintenance | 28–30 GB / 42 GB | 17 / 62 / 190 ms (locked) | yes | — | reference | M |
| M1 compacted (one presence, R37 sharing, interner sweep; compact records not built) | 18.4 / 32.5 GB | 39 / 131 / 442 ms | yes | — | fail: floor ≈ 15 GB in row values | M (floor D) |
| M2 hybrid (membership + exports + last results; recompute unless D1) | 325 MB heap / 470 MB footprint; ≈ 85 MB if the store serves exports; +22 MB over 1,000 edits | 38.8 / 48.8 / 80.6 s | yes, 0 diff, prefix and full | I8 for recomputed plans | fail on latency only | M (85 MB D/A) |
| X1 arrangement pruning | 16.0 / 17.0 GB | 43 / 172 / 393 ms | yes | nothing | killed: 123M records remain | M |
| X2 u32 columns + evicting interner | 10.2 / 10.7 GB | 36 / 99 / 343 ms | yes | limits panic instead of refusing | killed: 53 B/record floor; 630 MB strings | M |
| X1 + X2 | ≈ 6.7 / ≈ 7.5 GB | — | — | — | 25–30× over | D |
| X3 maintain vs recompute per clause | 1.7 GB load, 5.8 GB peak | 38 / 43 / 43 s | yes | I8 | killed: 88% of edits change inputs | M |
| X4 maintain facts, recompute logic | 0.37–0.40 GB (+ ≈ 12 GB with linker maintained) | 7.3 / 8.5 / 8.9 s adjusted; 92 s with linker | yes | I3, I8 | killed: no plan passes demand scoping | M / D / A |
| X5 indexed tables, delta-scoped recompute | ≈ 0.9–1.6 GB (estimated) | ≥ 7.6 / 7.6 / 10 s, linker free | by derivation, not run | I8 | killed: global fixpoints and windows | A / M |
| X6 shared declared facts | 19.1 / 25.6 GB; per-file 0.27–0.72 ms/rule | 23 / 73 / 226 ms | yes | I6/I7 | sharing saves 11–14%; 32–45 light rules fill 250 MB | M / D |

**"What the wave established"** (lines 18–24), verbatim:

1. "**Full maintenance cannot reach the target by engineering.** The floor after every generic cut is ≈ 50 B × ≈ 120M records ≈ 6–7 GB for five plans … At 50 B/record the target admits ≈ 5M arranged records, i.e. two orders of magnitude fewer than these five rules need as written."
2. "**Recompute cannot reach the latency target as the rules are written.** … an edit changes their inputs 88% of the time and a recompute costs 5–70 s."
3. "**The shared fact layer works and is cheap on the per-file side** … but it does not change the cross-file arithmetic: the membership-dependent state is the symbol table and the reference graph, 19M + 61M records, before any rule."
4. "nothing beyond membership must be maintained for exactness; maintained state buys latency only … So the one structure worth maintaining is the reference graph, compactly, and the rules above it should be local queries."
5. "**Exactness and latency were never the problem.** Every design that ran was exact, and maintained latency is 60–170 ms p95. The problem is the product of rule shape × maintenance granularity."

**Y1 + Y2** (lines 26–33):

- Y1: base 72 MB, overlays 193 MB, 40 deltas 32 → 50 MB, strings 89 MB, per-file inputs 288 MB; heap 869 / 956 MB; 0.73 / 2.4 / 3.9 ms.
- Y2 dead code: reached set with levels, 12 B/state, 24 MB for 41 worktrees as base + delta (182 MB as copies); scoped 0.07 / 1.5 / 3.1 ms.
- Rule-shape note: "85% of the reached states are per-method dispatch searches; keyed by class they would drop to 135K states."

**Y3** (lines 35–48): see § 14.

**Conclusion** (lines 50–58), verbatim key sentence: "The target (250 MB for 40 worktrees, hundreds of rules, ≤ 500 ms) is unreachable by any engine design over these rules as authored. What it takes is a change on the rule side and a different maintenance granularity on the engine side, together."

- Rule shape: write repository rules as *local* queries over a maintained reference graph.
- Maintained core: "The only state that must be membership-maintained is the reference graph and the symbol table … That is a design, not an optimisation, and it has not been built."
- Contradiction: this conclusion predates Y1 to Y3, which the same file then reports as built (§ 16).

## 12. Gate and mutant discipline (S5, LOOP, REWRITES, SCRATCHPAD)

**Command and lanes** (E:docs/facts/reports/phase-2-s5-gate.md § 5.1):

- `just facts-gate [--mutants] [--only <ids>] [--no-build] [--run-dir <dir>]`. Exit 0 = pass, 1 = fail, 2 = usage.
- Concurrent lanes:
  - a: replay, repo parity, subset/unrewritten, `a-vs-reference`.
  - b: `b-exactness`, Spike B's hand vectors.
  - g: `b-generated`, generator vs `rerail_facts::reference`.
  - c: lib tests, `classifier-leak`, `planner-audit`, `explain-stable` (3 byte-identical EXPLAIN texts of 60 rules, 2.27 MB), `harness-replay`, `views-derived`.
  - h: per-batch harness replay and suite.
  - t: `b-ties`.
  - Later lanes p, r, v (§ 9.4).
- `GATED` is a ratchet: 44 checks, then 48 (§ 7.1, § 9.1). Each gated check must be exact with "≥ 1 positive and ≥ 1 clean call".

**Mutant sources** (§ 5.1):

- A generic list: interim A3 views 1–3, delta generator 4–6, classifier 7, plan audit 8; later 9 (tie detection) and 10 (order proofs).
- "one mutant per row of `docs/facts/REWRITES.md`'s registry table (`mutant` column, `<n> <kind>`). A registered rewrite without a well-formed mutant fails the gate."
- Each mutant runs its family's checks with `FACTS_MUTANT=<n>`. "A kill is a detection the check names; a crash or a missing tool is `error`, never a kill."
- The memo is off under mutants (memo.rs).

**Growth** (M):

| Point | Mutants killed | Checks | Gate time |
|---|---|---|---|
| Wave 1 | 13/13 | 13/13 | median 295 s (268–389) |
| Wave 2 | 21/21 | 25/25 | 389 s |
| Wave 3 | 23/23 | 27/27 | median 323 s locked |
| S3 wave 4 | 24/24 | 28/28 | — |
| S8c | 30/30 | — | 946 s at load 72 |
| S3 wave 5 | 36/36 | — | 828 s incl. build |
| Wave 5 merged | 40/40 | 28/28 | — |
| S10 | 41/41 | 29/29 | 1,292 s, of which 845 s cold build |

The 10-minute target (I10) was met only with warm builds and trimmed lanes:

- `views-derived` was cut to shard 0/8.
- The edit replays were cut to a 300-event prefix (§ 9.1).

**Known weaknesses, stated by the sources:**

- Rewrite mutants 11–21 and 23 are killed by unit tests only; the corpus differential does not see them (§ 10 item 1; S7b § 7.1).
- Mutants 1 and 3 survive the subset differential (S7b § 7.1).
- The from-scratch dataflow check does not catch mutant 6, "because it changes the from-scratch dataflow the same way" (S3 § 15.1, negative control).
- Process lesson (P:SCRATCHPAD.md:143): "no merge without the gate on the merged tree, even for branches that report green." S3 wave 3 broke the leak tests on integration.
- Mode change (P:SCRATCHPAD.md:121): "Mode change (user): explore first, verify only ship candidates. Verification todos dropped; P3b merged as it stood (`eee7d6ac3`, gate was running, result not awaited)."

**LOOP tiers** (E:docs/facts/LOOP.md:7–11):

| Tier | Budget | What it runs |
|---|---|---|
| T0 | ≤ 60 s | unit tests, random differentials vs bundled SQLite 3.53.2 and the reference, and the mutants whose sites are in the changed files |
| T1 | ≤ 2 min | sub200 parity, engine-vs-reference on 50 files, 100-event replay prefix, archetype bench as work counts |
| T2 | ≤ 10 min | `just facts-gate --mutants`, integrator only |

- Counts are algebraic (engine-independent) and physical (line 21–24).
- Gate check (line 26): "no clause's algebraic or physical count grows by more than 5% at merge without a registry row explaining it."
- Measured: T0 3.4 s (M, SCRATCHPAD ledger).

**Rewrite registry** (E:docs/facts/REWRITES.md):

- Rows R1–R37 and D1, each with pattern, side conditions, justification, test and mutant. Examples: R3 file-locality cut (line 41), R4 export projection (line 42), R5 export distinct (line 43), R9 tree-reachability-to-interval (line 47), R11 selection sharing under subtree closure (line 49).
- The D1 row: "a repository plan is a function of its leaves' multisets over the members (I6, I7) … blocks are hash-consed on their exact row lists."
- "Planned" rows include selection pushdown and CSE by digest.

## 13. Y1: maintained reference graph (keyed tables, read-set memo, layers)

Sources: E:docs/facts/reports/explore-y1-graph.md; code `spikes/facts-b/src/ygraph/{mod,engine,world,view,intern}.rs` and `spikes/facts-b/src/lmap.rs` (E).

**Structure.**

- Per-path blocks (classes P and C), recomputed from per-content summaries.
- Twelve keyed tables (`T_BYNAME` … `T_EDGESRC`), rows `[u32;4]` plus a count (`ygraph/engine.rs`). `dep_key(t, h)` names a read.
- The aliases-by-target index is filtered; unfiltered it was 85% of rows.
- `DefId = (path, def name, occurrence)`. Switching to it cut mean re-runs from 11 to 1.6 (M).

**Read-set memo.**

- 17 query kinds, each with a static `level()`; for link it is `8 + (99 − hops)`.
- A memo is one `Box<[u32]>`: `[n_ins | n_qs<<16, ins as word pairs, qs, value]`. It holds the probed keys as 64-bit hashes, "absent keys included", plus the queries it read and its value.
- `BaseMemos` / `DeltaMemos` carry `readers` (dep_key → QIds) and `parents` (QId → QIds).
- `Ctx::run` records the read frame. It backdates the memo when the value equals the one below, and "adds readers only when reads changed".

**Maintenance.**

1. Old block out, new block in. `__init__.py` add/remove falls back to a scoped re-block of the subtree; this was 1 of 1,111 events (M).
2. Changed keys are collected.
3. `propagate`: a `BinaryHeap` by (level, id) re-runs readers lowest level first. On a value change it pushes the parents; an unchanged value stops propagation (early cutoff). A re-run equal to the entry below is dropped.

**Layers (`lmap.rs`).**

- `LMap<K,V>`: `below: Vec<Rc<FxHashMap<K, Option<V>>>>` (frozen, shared by `Rc`) plus a mutable top.
- `None` is a tombstone (lines 1–5, 10, 34–38). `set` drops an entry equal to the layer below, so "a worktree's state costs its difference" (lines 4–5, 83–97).
- `view()` requires an empty top. `freeze()` pushes the top as a new frozen layer, dropping tombstones over no layers (lines 135–149). Multimaps are kept as sorted vectors (lines 183–219).
- `Rev<K>` (lines 221–279): "A layered reverse index (key → ids), append-only like Y1's readers: an id once added stays (a stale entry only costs a re-run that changes nothing)."
- Layering is Base → cluster overlay (plurality over 8 commit-date ranges) → worktree Delta. `GraphView` materialises in ≤ 23 ms (M).
- Strings: an append-only arena interner shared across layers (`ygraph/intern.rs`).

**Exactness** (M):

- Base = native's 145,431 edges.
- Maintained = from-scratch at all 1,151 checks.
- The per-step edge delta equals a recomputation on all 1,111 events.
- Two deviations from the SQL linker, with counters at 0: links are computed at the caller's hops instead of `link_reach d<19`, and hop sets.

**Numbers** (M):

- Base 72 MB: 406,920 memos, 375,334 rows, 61,908 edges. Tables 19 MB; memos with reverse indexes 49 MB.
- Overlays ≈ 52 KB per divergent path; deltas ≈ 29 KB per path. Without clusters: 1,443 MB (≈ 88 KB per path).
- Per edit: re-runs 0 / 7 / 17; rows touched 276 / 2,163 / 4,443; probes 1 / 14 / 28.
- Latency 0.73 / 2.40 / 3.94 ms wall. Commits and bulk events 45 / 205 ms, max 1.5 s.

**Open items:**

- The CSR arena has not landed (est. 1.5–2×: A).
- No demand GC.
- I8 violated: query bodies are hand-written per SQL stage.

## 14. Y2 and Y3: reached set with level witnesses; the wired path with M2 and D1

### 14.1 Y2 (E:docs/facts/reports/explore-y2-local.md; code `spikes/facts-b/src/y2/{dc,ef}.rs`)

- **Product graph.** States are (node, layer): `flow(node, NULL)`, `flow(node, d)` (a dispatch search for method `d`) and `annotated(node)`. Every edge is "a local query at the source node (`succ`)" (§ 1.1, lines 60–66).
- **Level-witness invariant**, verbatim (lines 68–72):
  - "Each reached state carries a level (its BFS depth at scratch). The invariant: every reached non-root state has a reached predecessor of strictly lower level whose edge exists."
  - "Insertions propagate forward."
  - "Deletions take a deleted edge's target, or a deleted root, in increasing level. The state is removed only if no lower-level predecessor with an edge is left. Predecessors come from `pred`, a superset; each candidate is verified against `succ`."
  - "Removed states with any reached predecessor are then re-derived and propagated."
- **DRed vs level witness** (lines 74–83, M):

  | | DRed | Level witness |
  |---|---|---|
  | States over-deleted, max | 388,040 of ≈ 400K reached | 80 |
  | Edits above 50 ms | 96 of 298 | 0 |
  | Scoped p95 | 394 ms | 1.5 ms |
  | Scoped max | 426 ms | 8.8 ms |

  Reason, verbatim: "every definition has an edge to its module node and module nodes reach module-level names, so the forward closure of almost anything is half the graph."
- **Code** (`y2/dc.rs`):
  - `Dc { reached: LMap<St,u32> /* levels */, disp_by_node, nodes_by_disp, imported, findings }`.
  - `finish()` deletes using the level witness via `support`: a heap in increasing level builds the over-deletion set `unsup`; those states are re-derived and propagated; toggled states drive findings.
- **Size** (line 9, M):
  - 95K–400K states per replay worktree; 896K on the oracle tree, 761K of them dispatch.
  - 12 B/state packed.
  - 41 worktrees: 15.2M states as copies (182 MB); 1.97M as base + delta (24 MB).
- **Latency, dead code** (M): scoped 0.07 / 1.5 / 3.1 / 8.8 ms; unscoped 150–177 ms. Scoping uses inverse indexes over the old state, with proof by read set; fuzzing ran 800 steps.
- **exported-families:** group G = (scope, owner, base_name); partition F = (scope, base_name, implementations). Scoped 0.005 / 0.04 / 0.09 ms (M).
- **Exact** (M): 4,931 = 4,931; 0 = 0; 340 replay checks with 0 differences; 800 random deletion steps with 0 differences.
- **Rule-shape list for Jeremy** (§ ~line 183):
  1. "Declare reachability as reachability."
  2. Dispatch keyed by class.
  3. Definitions keyed by id. Without the "moved only" refinement, dirty p95 was 19,692 and scoped p95 34 ms.
  4. No computed join keys.
  5. The module hub: "The module hub is the rule's meaning, and it is coarse."
  6. Probe facts.
- **Not covered:** symbol-table maintenance. The replay ran only on the 300-event prefix because of a machine swap.
- **Bulk:** 281 ms scoped vs 151 ms unscoped. A fallback when about a quarter of the state changes is suggested [INFERENCE].
- **E's Y1-wired addition:** graph + both rules 0.35 / 2.64 / 4.53 / 9.36 ms (M).

### 14.2 Y3 (E:docs/facts/reports/explore-y3-wire.md; code `spikes/facts-b/src/y3/{wire,sym,m2,world,input}.rs`)

- **Path per edit**: exports → Y1 apply → `refs_delta` (via the reverse index target def → source defs) → `SymState::apply` → Y2 prepare / `Facts::apply` / finish → M2 (D1 or `batch::eval`) → `FindingsDelta`.
- "One path, no differential dataflow" (DECISION-CROSSFILE.md:37).
- **Symbol table:** the same keyed-table design. Its queries are decl, imp, res, bt, pb, be, hier, pub and roots, and they replace the nine `proj_*` inputs. It holds 1,098,978 rows, equal to the dataflow's (M).
- **Deviation:** Y2 does not read `GraphView`.
- **CrossFile trait** (`y3/world.rs`): `apply(worktree, members, delta: &[PathChange{path, old, new}]) -> FindingsDelta{introduced, removed, failures, work}`, plus `forget` and `held_findings`.
- **Exact:** 1,151 from-scratch checks over the full replay, 0 differences (M).
- **Memory** (DECISION-CROSSFILE.md:41, M):
  - Heap 1.94 GB at load, 2.09 GB at edit 1,000. Maintained state ≈ 950 MB:
    - graph 72 + 193 + 32→54 MB;
    - symbol tables 60 + 139 + 32→54;
    - Y2 input indexes 47 + 167 + 92→143;
    - reached sets 3 + 92 + 16→27.
  - Per-file inputs 253 MB and strings 285 MB, both "store-servable"; ≈ 390 MB unattributed.
  - Dc overlays re-add 2.29M states (62 MB) against 94,739 in the base.
- **Latency** (line 42–45, M):
  - Single-file 0.53 / 3.76 / 8.62 ms (Y1 0.18/1.07, symbol table 0.11/0.67, Y2 0.10/2.19, M2 0.02/1.08).
  - 200-file bulk 387 / 550 ms; real commit 63 / 470 / 760 ms.
  - New worktree 11–13 ms, ≈ 2.9 ms and 156 KB per diverging path.
  - Floor: identical content 0.008 ms; content changed with nothing downstream 0.26 / 1.94 ms.
  - M2 ran 105 of 1,111 events. One 202 ms outlier.
- **Verdict:** "latency met by two orders of magnitude; memory 2.1 GB against 250 MB, with ≈ 540 MB store-servable and the levers above unapplied; against today's 42 GB, 20×" (line 48).
- **Levers not taken** (line 46): the overlay re-adds; Y2 keyed by position instead of DefId; three interners; frozen-layer arenas.
- **Verification owed** (P:SCRATCHPAD.md:111): "I8 hand-written query bodies, gate checks `symcheck`/`replay --check-every 1`, refusal withholding, the linker deviations' counters". Mutants for the symbol table are missing. Compaction is not done.

### 14.3 M2 batch executor and D1 skip (code)

E:spikes/facts-b/src/gen/hybrid.rs:

- Lines 4–11, resident state: per (export, path, content) one `Block`, "once in 32 bits a value when every value's word fits", hash-consed across exports, programs and paths, refcounted by the worktrees holding the (path, content).
- Lines 13–18, D1 verbatim: "a plan is a function of the multisets of its exported leaves over the worktree's members (I6, I7; the generator's semantics are order-free, an order dependence is a refusal), so when every block the plan reads is the same block before and after the event at every changed path, its result is unchanged and is not recomputed. Otherwise the plan runs from scratch over the worktree."
- `--no-skip` recomputes every plan.
- `d1_affected(reads, changed, block)` (line 430). Mutant 60 site at line 436 (`a.is_none() || b.is_none()`). Test `d1_affected_iff_a_read_block_differs` (line 997).
- Lines 20–23, exactness: `--check batch|engine`. `engine` also runs the generator's from-scratch dataflow.

## 15. Why SQLite was replaced, and why statistics-chosen plans were forbidden, in Moti's words

**Replacing SQLite as executor:**

1. PLAN.md:16 (measured basis): "267 s = 198 s SQLite stage builds (164,742) + 45 s input re-extraction (81,532 calls) + 13.5 s setup + 1.8 s producer".
2. PLAN.md:23: "SQL set-composition engine (done, parked) | exact; ~3× native cold; never faster on Jeremy's rules; loss was SQLite per-statement and B-tree cost".
3. PLAN.md:82: "SQL set-composition engine retired as an executor: exact, but ~3× native on our rules and never faster on Jeremy's 60 (dl60). Native stays the production path until Phase 5."
4. PHASE-1.md:59: "No SQLite, no per-statement planner. Keep each rule's cost measurable on its own."
5. spike-a.md:242: "There is no SQLite and no per-statement planner at run time."
6. P:SCRATCHPAD.md:82, the re-steer: "the goal is SQL's strong suit on native's raw speed through a strong architecture around invariants, not ad-hoc escape hatches and per-operator patching." This is attributed to the user.
7. PLAN.md:72, Phase 5: "retire the SQLite global phase and the SQL executor".
8. SQLite remains the semantic oracle (I1) and the differential target: "random differentials against the bundled SQLite" (INVARIANTS.md:23).
9. jb-progress-2026-10-23 (P:SCRATCHPAD.md:131) on Jeremy's compiler: "SQLite in the hot path, nothing on cross-file". Conclusion: "his path reaches native parity per file at best and has no answer for cross-file or fleet memory".

**Plans fixed by text, not chosen at run time:**

1. I5 (INVARIANTS.md:27): "Plans are compiled once per rule at pack load, deterministic, printed by `EXPLAIN`, identical for every file". Forbids "Per-statement planning at run time … a plan that differs between files".
2. I1 (INVARIANTS.md:23): forbids "tie-breaking that depends on the plan; emulating SQLite's scan order". Plan-dependent results become "a named, retractable refusal, never a plan-dependent guess".
3. spike-a.md:234: "The plan is fixed by the text"; spike-a.md:281: "a fixed, text-determined rule, not a cost-based planner".
4. phys.rs header (E:facts/src/phys.rs:1–3): "plans are compiled once per rule, printed stably, and a plan that scans a relation per outer row (an inner scan) is refused."
5. LOOP.md:17–23: work counts must be deterministic and "do not move with load". Budgets are defined in algebraic units, "a fact about the plan on that content, reproducible on every engine".
6. **The word "statistics" (or ANALYZE, cardinality-driven join ordering, `sqlite_stat`) does not appear** as a stated reason in any facts doc: not found. Moti's stated reasons are:
   - determinism and identical plans across files (I5);
   - exactness without plan-dependent tie-breaking (I1);
   - per-rule cost measurable on its own (PHASE-1:59);
   - stable EXPLAIN for the gate (`explain-stable`).
7. Related, from Tim's sql-set-composition design on `sql-dl60`, not Moti's facts docs: "`xBestIndex` cost hints are not relied on."

## 16. Contradictions and inconsistencies

1. **Memory target.** PLAN.md:7 says "under 1 GiB resident" for the whole system. PHASE-2 G2.4, PHASE-3 G3a.1 and DECISION-CROSSFILE.md:3 say ≤ 250 MB at load for cross-file over 40 worktrees.
2. **I7 vs the memo key.** I7 forbids "Caches keyed by path" (INVARIANTS.md:29). The implemented memo keys path-dependent clauses by path (facts_memo.rs:7–10; S10 decision "memo key keeps the path for now", P:SCRATCHPAD.md:101).
3. **Plan digest is not purely algebraic.** DECLARED-FACTS.md:53 says the fact digest "covers the definition's algebra after lowering, not its text", and I7 says "expression digest". The implemented clause digest (memo.rs:19–28) is blake3 over the resolved clause JSON plus physical plan and EXPLAIN text. The S1 sharing census also shows text identity (60 groups) differs from IR-digest identity (15 groups).
4. **I8 vs Y1/Y2/Y3/X5/M2.** I8 forbids "Hand-written delta rules in production". Y1's query bodies are hand-written per SQL stage (DECISION-CROSSFILE.md:30). X5 is hand-derived (line 15). M2 and X3 "break I8 for recomputed plans" (lines 9, 13). Y3, the "ship candidate", still owes this (SCRATCHPAD.md:111).
5. **I6/I7 vs per-content facts.** DECLARED-FACTS treats per-content facts as content-keyed. X6 found "per-content facts carry `__path`; `py_all` reads the file path" (explore-x6-rules.md:9). SCHEMA § 3.2's plan-level export namespace also includes the path in the key.
6. **Spike B's prediction.** Spike B: 195 MB for 40 worktrees. Real plans on the generator: 29–31 GB at load. The SCRATCHPAD says "Spike B's 195 MB did not predict it" (P:SCRATCHPAD.md:99).
7. **M1 vs M0 latency.** M1's compacted latency (39 / 131 / 442 ms) is worse than M0's (17 / 62 / 190 ms, locked) (DECISION-CROSSFILE.md:7–8). The table does not explain why; I found no explanation, but M1's run conditions may differ.
8. **The DECISION-CROSSFILE conclusion is stale against its own body.** Lines 55 and 58 say the dedicated maintained graph "has not been built", while lines 26–48 report Y1, Y2 and Y3 as built.
9. **Refusal semantics, Y3 vs P3a.** The Y3 report records a refusal disagreement with P3a: w50 is live at the end. The report does not say what P3a's side shows for w50 beyond the disagreement. Refusal withholding is listed as owed verification (P:SCRATCHPAD.md:111).
10. **Gate time vs I10.** I10 says ≤ 10 min. Measured: 295–389 s warm, but 828 s, 946 s and 1,292 s on later trees with builds or under load (S5 § 9.1, S8 § 10.1, S3 § 13.2, S10). Lanes were trimmed (a 300-event prefix, shard 0/8) to fit, which S5 says "would weaken G2.4's exactness evidence" (§ 6).
11. **Plan-order dependence against SQLite remains.** S8c § 10.3, 1: ties of the lone extremum across join rows differ from SQLite on 32 of 150 random DBs. The reference and the engine use text-order nested loops, so I1's "never a plan-dependent guess" is not fully closed. A ruling is pending.
12. **Ledger correction.** S7 § 1: A3's dump groups cost ≈ 59 s, not the ledger's 10 s.
13. **Declared-facts cost claim vs measurement.** DECLARED-FACTS.md:63 expects most of 34.9 s back. S4 measured the views at 66.5 s (rows) or 42.0 s (columnar), more than the 34.9 s they would replace (S4 § 5.1, W2.3). S7b measured selection sharing's upper bound at ≤ 2.7 s (§ 7.4). S9 never ran.
14. **SCRATCHPAD versions.** E's SCRATCHPAD lacks the exploration-wave, Y and G3b.1 rows; P's is newer. The P3b and P3c reports each have their continuation on a different ref.
15. **P3c timing.** The new-worktree number differs between passes: p50 2.2 s to complete vs first findings at 44 ms. These measure different things across the first pass and the continuation.
16. **Kill criterion overridden.** The Spike A kill criterion was triggered (< 4× win: 1.85× / 1.94×), but the phase went ahead on a GO justified by the latency and memory targets (spike-a.md:80; P:SCRATCHPAD.md:80).

---

## Synthesis (only what the sources establish)

**Q1. Declared facts.**

- **Core facts are implemented.** SCHEMA v1, `SCHEMA_VERSION 1`, path-independent:
  - `node` over every tree-sitter node (named and anonymous), with u32 preorder ids, `subtree_end`, interval FDs and parent < child;
  - plus `child`, `field`, `identifier`, `line`, `syntax_error` and parser facts.
- **Produced** per content from native's tape by facts-a's `Facts::build` at p50 0.53–0.77 ms (M).
- **Stored in memory** as `Vec<u32>` columns, with `flags: u8`, `field: u16`, CSR children and kind postings. Kind and field names are interned process-wide as u32.
- **Stored on disk** as `RRFACTS\0` files keyed by blake3 hash: `kind` u16, `field` u8, derived columns rebuilt on read, zstd 3, 186.4 MB = 1.35 B per source byte = 6.6 B per node (M).
- **Python-derived relations** are normative SQL views (0 differences vs stage texts over 236.6M rows, M).
- **The `facts` block** (`over`, `columns`, `key`, `fd`, `define.sql|match`, `version`; identity = digest of lowered algebra + schema version; memo `(content hash, fact digest, schema version)`) is a proposal, not scheduled. It was exercised only by hand in X6. Its storage width and interning are not specified beyond an open question favouring ids.

**Q2. The content-hash memo.**

- **Key:** `(blake3 content → store source, fact schema version, path or '' for content-pure clauses)`, plus the clause's plan digest.
- **Plan digest:** blake3 over the resolved clause, physical plan texts, rewrites flag, schema, `ENGINE_VERSION` and budget A.
- **Granularity:** one clause × one content[, path].
- **Cached:** findings, failures, counts, the repository clause's per-file rows and budget rows. Empty results are completion bits per plan set; plan sets are sha256 of ordered digests.
- **Invalidation:**
  - digest change, including an integrator-bumped `ENGINE_VERSION` for operator changes under unchanged text;
  - `--no-memo` in the gate and under mutants;
  - cascade on source reclamation after the daemon unpins.
- **Measured:** warm corpus 19.6 s vs 51.2 s cold; a rewrite re-ran 9/105 digests; 77 MB of memo.

**Q3. Cross-file maintenance.**

- **M0:** S3's generic generator over differential dataflow, exact. 29–32 GB at load, 42 GB at 1,000 edits, 17 / 62 / 190 ms (M).
- **M2:** hash-consed 32-bit export blocks plus membership plus last results. Plans are recomputed from scratch by `batch::eval` unless D1 shows every read block unchanged at every changed path. 325 MB heap, exact, 38.8 / 48.8 / 80.6 s.
- **Y1:** linker as 12 keyed `[u32;4]` tables plus `Box<[u32]>` read-set memos (probed keys hashed, absent ones included). Readers re-run in static level order with early cutoff and backdating. Layering through `LMap` / `Rev`: frozen `Rc` layers, a top holding only differences, tombstones, append-only reverse indexes.
- **Y2:** dead code's reachability as a reached set with per-state BFS levels. Deletion uses a "strictly lower-level supporting predecessor" witness, which beat DRed (max over-deletion 80 vs 388,040; p95 1.5 vs 394 ms). 12 B/state; 24 MB for 41 worktrees as base + delta.
- **Y3:** wires exports → Y1 → symbol table (same keyed design) → Y2 → M2 + D1 for the light plans. No dataflow. Exact on 1,151 checks; 0.53 / 3.76 / 8.62 ms; 1.94 → 2.09 GB heap.

**Q4. Alternatives.**

| Design | Why it failed (M unless marked) |
|---|---|
| M1 | 15 GB floor (D) |
| X1 | 123M records remain |
| X2 | 53 B/record floor; 630 MB of strings live |
| X1 + X2 | ≈ 6.7 GB (D) |
| X3 | 88% of edits change inputs; 38–43 s |
| X4 | no plan passes demand scoping; ≈ 8.5 s adjusted |
| X5 | global fixpoints and windows can't be scoped; ≥ 7.6 s; 0.9–1.6 GB (A) |
| X6 | 19 GB on the dataflow; 32–45 light rules fill 250 MB; alive only as the per-file side |
| M2 | latency only (48.8 s p95) |

Every design that ran was exact.

**Q5. Memory and latency.**

- Measured: M0 28–42 GB; M2 325 MB heap; Y1 869 / 956 MB heap; Y3 1.94 / 2.09 GB heap, of which ≈ 950 MB maintained, ≈ 540 MB store-servable and ≈ 390 MB unattributed.
- Per-edit wall p95: M0 62 ms; Y1 2.4 ms; Y2 1.5 ms; Y3 3.76 ms; M2 48.8 s.
- Derived floors: M1 ≈ 15 GB; full maintenance ≈ 6–7 GB at 50 B/record; M2 ≈ 85 MB with store-served exports (D/A).
- Estimated: CSR arena 1.5–2×; X5 0.9–1.6 GB.
- The 250 MB target was not met by any measured design that also met latency. The closest was Y3 at 2.1 GB.

**Q6. Gate.**

- `just facts-gate --mutants`, one command, I10's target ≤ 10 min.
- Lanes: corpus parity, engine vs reference, generator vs reference, leak and audit tests, EXPLAIN stability, per-batch harness, ties.
- Mutants come from a generic list plus one per REWRITES row; a rewrite without a mutant fails the gate. "A crash … is `error`, never a kill."
- Grew from 13/13 to 41/41 mutants and 29 checks.
- Known gaps:
  - several mutants are killed only by unit tests;
  - the from-scratch check cannot catch lowering bugs that affect both sides;
  - timing exceeded 10 min on later trees;
  - verification was dropped for exploration designs by user mode change.
- Loop tiers: T0 ≤ 60 s, T1 ≤ 2 min, T2 gate. Count regressions above 5% need a registry row.

**Q7. Why SQLite was replaced and plans fixed by text.**

- **Replaced:** SQLite per-statement and B-tree cost (set-composition engine ~3× native, never faster); 198 s of SQLite stage builds in batch 6's global phase. "No SQLite, no per-statement planner." SQLite is kept as the semantic definition and differential oracle (I1, pinned to 3.53.2).
- **Plans fixed:** once per rule, deterministic, identical for every file, EXPLAIN-stable and audited (I5). Plan-dependent tie-breaking is forbidden and becomes runtime refusals (I1).
- No source names statistics or cost-based optimisation as the thing being forbidden, beyond "not a cost-based planner" (spike-a.md:281) and "per-statement planning at run time" (I5). An explicit statistics rationale was not found.
