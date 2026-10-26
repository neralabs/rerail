# The final Rerail shape

> Superseded where it differs by `03-architecture.md` (the user-set target architecture).

Status: the overall shape, written from `notes/` (index: `notes/s0-synthesis.md`). Details are given only where they fix the shape. Numbers carry the source's M/D/A mark.

Budgets: **500 MB RAM in total, 10 GB disk.** Targets: per-edit checks, a full-repository audit in 1–2 s, everything runnable inside outer SQL, Metal for every program with no CPU fallback.

---

## 1. The shape in one paragraph

Rules, named facts and type inference are **one stratified Datalog program with lattices**, over two kinds of keys: (document, node) and names. **One compiler**, driven by statistics stored in SQLite, lowers each stratum to one of two executors:
- **Document-local strata** become **transducer program groups**. They run set-at-a-time over **batches of documents × rule groups**, directly over v9np tapes in resident slabs, on Metal, JIT or the interpreter.
- **Cross-document strata** become **SQL in SQLite**, the on-disk system of record, with native vtab operators where SQL is the wrong shape.

**Invalidation is the same fixpoint, one level up.** Each (stratum, document) or (stratum, module) result is a derivation keyed by its inputs and the digests of what it read. An edit seeds a frontier, and semi-naive waves over the derivation graph re-run only derivations whose read digests changed, with early cutoff. A cold audit is the same evaluation with every derivation dirty. A per-edit check is the same evaluation with a frontier of one document.

```
                  rules + rerail.facts/v1 + type inference          (no SQL in rules)
                                   │
                    ONE Datalog program with lattices
                                   │
     stats in SQLite ──►  STAT-DRIVEN COMPILER  ──► compiled_plans (decisions + validity intervals)
   (fts5vocab, edge_freq,          │
    kind_size, fact_stats,         ├── document-local strata ──► TRANSDUCER program groups + set/lattice stage
    plan_feedback)                 └── cross-document strata ──► SQL over fact/interface tables + native vtab ops
                                   │
         ┌──────────────────── derivation graph (invalidation = semi-naive waves + digest cutoff) ───────────────┐
         │  dirty derivations ∩ candidate docs (postings) ─► demand                                              │
         │  batch (docs × groups, cut by resident bytes) ─► transducer: set/lattice strata → programs → outputs  │
         │  bulk publish narrow outputs + interface digests ─► SQLite                                            │
         │  SQL strata (joins, closures, counts, cross-file lattice layers) ─► changed digests ─► next wave       │
         └───────────────────────────────────────────────────────────────────────────────────────────────────────┘
                                   │
                    finding keys + Complete/Unknown ─► consolidation
```

## 2. One semantic model

- **Strata** come from the program: positive recursion within a stratum, negation and aggregation between strata (t1 spec-transducer §9; t2 metal-enqueue §3).
- **Lattices** are declared with a join, a finite height or widening, and transfer functions with declared properties such as absorbing elements (t9 Synthesis 4, 7). The boolean node set is the two-element lattice.
- **Tree structure is not Datalog recursion.** Parent, subtree, ancestors and enclosing scope are tape arithmetic and columns (t9 §5.3; t2 metal-recursion).
- **The output is a set of finding keys plus Complete/Unknown per (rule, file).** That is what makes set mode, early stop, pruning and fusion legal (t1 spec-fusion §5).
- **Three producers per fact** (t1 §21):
  - fact programs: transducer programs over structure;
  - native producers: resolvers and evaluators;
  - derived facts: the algebra.
  
  All three are nodes of the same program and the same derivation graph.

## 3. Stat-driven lowering (the compiler)

Every physical choice is made **at compile time from statistics stored in SQL**:
- statistics are read in one batched statement;
- each choice is recorded in `compiled_plans` with the statistic keys it read and the interval over which it stays valid;
- plans are cached per stat bucket and re-lowered only when a decision would flip;
- nothing counts rows at run time, so DLRE's trap T18 cannot recur.

(t1 §11, spec-indexes §2.9–2.12, spec-fusion §4.8–4.9; t4 #20–22, #26–28.)

| Decision | Chosen from | Alternatives |
|---|---|---|
| Stratum placement | partition (document vs repository), with demand and size bounds | transducer group / SQL table / inline / vtab (t1 §21.6) |
| Anchor (seed) per program | `search_kind_vocab`, term vocab: rarest mandatory concrete variable, value before kind | postings seed / fused dispatch over the kind column (t4 #26; t1 spec-fusion §4.6) |
| Relation form (has/inside/precedes, stopBy) | `edge_freq`, `kind_size`, selectivity | lazy cursor / `*_KIND` scan / set-stage pass (cost formulas, t1 spec-fusion §4.6) |
| Climb vs descent | `edge_freq` walk cost | parent walk / subtree interval (t4 #27–28) |
| Set representation | bound(S) · 32 vs n | postings / bitset (t1 spec-fusion §4.8) |
| Fusion | ≥ 16 programs share a root kind, or ≥ 2 share a target; identical scope only | separate / trie / packed set pass (t1 spec-fusion §4.2–4.5) |
| Conjunct order | cost / (1 − sel), from `plan_feedback` | — |
| Derived-fact materialisation and demand | `fact_stats`; demand only where keys are few (t9 §6.3) | table / inline / vtab; magic or full |
| SQL join order | `fact_stats` written to `sqlite_stat1` (never ANALYZE) | — |
| Batch size | pool budgets (§6) | — |

The same plan has **three bindings**: Metal, CPU (JIT or interpreter, the reference), and pure SQL over a tape TVF (the fallback, lower priority). The bindings differ in executor only, never in plan.

## 4. The transducer: the executor of every document-local stratum

The base is Tempo's IR: 32-bit words, a verifier, and value, capture and output side programs (t6 S1). The final shape changes Tempo's IR in these ways (t1 spec-transducer):
- **Set mode.** Output is a set. KEY labels stop at their first emit. Paths whose reachable labels have all fired are dropped. Enumeration is lazy, one cursor per enumeration (5.26M → 96k paths, M).
- **Barriers.**
  - EXISTS/NOT_EXISTS and bounded COUNT.
  - Shallow barriers run inline. Deep ones, and any containing an unbounded axis, are promoted to the set stage, so no thread holds an unbounded stack.
- **The set stage, generalised to a lattice stage.**
  - Per document in the batch, strata produce node-keyed columns before the programs that read them:
    - LEAF, seeded from kind postings or a kind scan;
    - HAS / INSIDE / PRECEDES;
    - AND / OR / ANDNOT;
    - COUNT_PREFIX;
    - FACT (from the fact cache);
    - FIX, for monotone recursion.
  - Boolean sets are bitsets or postings, with up to 32–64 targets packed per pass.
  - **Document-local lattice strata** (R0 type inference, constant values) are the same FIX waves over 32-bit lattice words with declared height or widening, using semi-naive frontiers (t9 §4.3–4.6).
  - Programs read the results through IN_SET, COUNT_SET, SIDE_TEST and SIDE_EQ.
- **Fact programs** are transducer groups with EXPORT labels. Base facts come out of the same batch as the rule programs.
- **Calls.** Non-recursive utilities are inlined or bounded CALLs. Recursive utilities become FIX plus IN_SET.
- **The verifier** requires every op to exist on every backend. Recursion through facts of other documents is rejected; that belongs to SQL strata.
- **FLAGS** uses Tempo's production masks (named 1, extra 2, missing 4, error 8, with required/forbidden masks). The spec text is fixed to match.

Execution on Metal (t2 metal-recursion, metal-enqueue):
- dispatches per batch are O(strata), not O(tree depth) or O(generations);
- FIX and lattice strata run as GPU-scheduled hybrid waves (1.2–1.6 µs per wave, M) in one command buffer;
- outputs go through count → scan → scatter;
- memory is sized at plan time, with no per-path arenas;
- resource exhaustion gives `Unknown(resource)`, and the planner resubmits with the batch or group split, still on Metal.

## 5. Data at rest (10 GB disk) and SQLite as the system of record

One content store, `content-v*.sqlite`, extending main's CAS (t7):

| Object | Form | Approx. bytes |
|---|---|---|
| Source | zstd-3 frame, content-addressed | 0.96–1.23 B/node (M) |
| Tape v9np | zstd-3 frame of SoA `kf, size, base, off, tail`; no `par` | 1.77–2.53 B/node (M), ≈ v7 |
| Kind postings | `search_kind_fts`, positional, re-tokenised to `lang·1024 + local`; always built (decision 0006) | ~2.6 B/node |
| Value/term postings | case-exact, deduplicated vocabulary; always built | ≤ 1.3 B/node |
| Statistics | `search_kind_vocab`, `edge_freq`, `kind_size`, `fact_stats`, `plan_feedback` | < 0.2 B/node |
| Fact tables | narrow and typed, u32 atoms, ≤ 0.25 rows/node per relation; never a copy of the tree | < 0.7M derived rows on the gate corpus (A) |
| Interface rows | per-module exported names, types and summaries, with digests | small |
| Derivations | key → output digest, read-set digest, completeness | small |
| Results | finding pages per derivation (generalising `local_check_results` and tree reuse) | small |
| Plans | `compiled_plans`; a single interner (`text_vocab` / atoms) | small |

- Snapshots and overlays are shared across workspaces (100 workspaces on one commit share one snapshot).
- GC is by relationships and leases.
- Nothing stores a row per node (t4 #1, #12).

Disk is plentiful and RAM is not. That is why facts, interfaces and results live in SQLite and not in a resident engine heap.

## 6. Memory (500 MB in total)

Resident state is the **working set of the current wave**, never the corpus:

| Pool | Content | Bound (A) |
|---|---|---|
| `tape.resident` / `source.resident` | the current batches' v9np and source, decompressed directly into 64 MiB anonymous-mmap slabs; one no-copy shared `MTLBuffer` per slab, retained; pre-touched | ~128–192 MB, by residency waves |
| `edit` slab | the edited documents' tapes, encoded straight from the parse | a few MB |
| `tape.derived` | `par`, `cidx`, line starts, `tid` per resident tape | ~0.3× tape |
| `set.*`, `lattice.*`, `output.*` | plan-sized, count → scan → scatter | ~32–64 MB |
| SQLite page cache | facts, interfaces, postings and statistics on demand | ~32–64 MB |
| plans, interner hot set, consolidation | — | small |

- Batches reference slabs through a batch document table. Nothing is repacked or concatenated, and nothing is copied on a hit.
- An audit streams residency waves: load, pin, execute, release. Decompression costs 59–77 µs per file p50 (M), runs in parallel, and happens outside SQLite transactions.
- Warm work avoids tapes entirely through result reuse (§7). Tempo's 47 warm workspaces needed no tape at a 499 MB peak (M).
- Whole-program in-memory designs are excluded because they don't fit: Moti Y3 ~2 GB, Typing program mode 450–640 MB, Tempo Metal 3+ GB (M).

## 7. Invalidation is part of the lattice design

**A derivation** is one (stratum, partition) evaluation. The partition is a document for document-local strata, or a module/component for cross-file strata. Its key is:

```
(stratum plan digest, own input digests (tape id, fact digests), digest of every interface/fact row it READ, absent keys included)
```

Its value is its output rows in SQLite plus an output digest.

- **The derivation graph** has edges from read sets, recorded when the derivation ran. Its cyclic components are module components (Typing's condensation: 13 layers, M).
- **Evaluation of that graph is the same semi-naive wave algorithm as the lattices inside it:**
  1. The frontier is the derivations whose keys changed: an edited document, a changed rule, or changed statistics that flip a decision.
  2. Each wave re-runs the frontier's **document-local derivations as one transducer batch** (all dirty documents × their groups). It then re-runs the dirty **cross-file SQL strata**.
  3. A derivation whose output digest is unchanged **cuts off**: its readers are not enqueued (backdating).
  4. Cyclic components iterate until their interface digests are stable.
- **Retraction never goes through DRed.**
  - A dirty derivation is recomputed from its inputs: from scratch for a document (under 0.3 ms for the largest document's lattices, M), or per component for cross-file strata.
  - Monotone propagation happens only inside a derivation, where lattice values only rise.
- **Measured instances of this one mechanism, each a special case:**
  - main's `calls` per-consumer stamps;
  - Moti's Y1 read-set memos with early cutoff and backdating;
  - Typing's interface digests: a body edit re-derives 1 module instead of 408, an interface edit 123 instead of 408 (M);
  - PR 2539's dependency cone.
- **Rule findings are derivations too.** Result pages are keyed the same way. Main's reuse by (source, file context, catalog) and by clean Git tree become special cases. Cross-file rules become reusable because their read sets are explicit, which main cannot do today.
- **Content addressing gives the leaves.** A changed byte means a new source id, a new tape id and new keys.

Repository-wide recursive relations (call closure, reachability) are cross-file SQL strata over interface rows:
- answered by sparse walks from the demanded seeds (main's `calls` shape) or by a component recompute;
- memoised by read-set digest;
- never maintained as whole-program state in RAM.

## 8. How the targets map onto the shape

- **Per edit.**
  1. Parse and encode into the edit slab, then publish the document synchronously.
  2. Wave 1: one batch of the edited document × all groups. This runs base facts, R0 lattices and rule programs in one dispatch sequence, with a cached plan class for small n.
  3. If interface digests changed, wave 2 is one batch of the affected dependents, loaded from the store, plus the cross-file SQL strata.
  4. Typical frontier sizes: 1 derivation for a body edit, 123 for an interface edit (M).
- **Full audit, warm.** The derivation keys match, so result pages attach without loading tapes.
- **Full audit, cold or after a rule change.** Every derivation is dirty, so the audit runs residency waves of large batches (documents × all groups); cross-file layers follow in digest order. The 1–2 s target rests on these levers:
  - fused shared evaluation, at 0.26–0.40 ns per program-node on the CPU (M);
  - regex through `tid` at 0.1–0.3 s instead of 9–17 s (D);
  - facts by vtab instead of SQLite table loads, 16.4 s (M) → ~1 s (A);
  - Metal for the program and set strata.
- **Outer SQL.**
  - The transducer is the `demand(group, doc[, root][, exports])` vtab, one xFilter per (group, batch), with callbacks reading only prepared data.
  - The boundary costs 4.9% (M).
  - Pure-SQL fallback: the same plan over a tape TVF.

## 9. What each stream contributes to this shape

| Stream | Kept | Replaced |
|---|---|---|
| Main | CAS, snapshots and overlays, leases, the ≤ 64-object publication transaction, result keys, `calls` stamps, the positional kind FTS in the one store | the per-file ast-grep matcher (kept as the rehydration oracle); v7 encoding; the 32 MiB/30 s cache; local kind tokens |
| PIAT | the spec: v9np, §20 residency, §21 facts, transducer set mode + set stage, fusion, stat-driven indexes and plans, tooling | §21.8 DRed → derivation waves with digest cutoff; SQLite-only derived facts → the stat-driven placement in §3 |
| Tempo | the IR, verifier, side programs, `native_ir` codegen, no-copy MTLBuffer lifetime, reserve-before-allocate, count/scan/scatter, result reuse | the path-enumerating model, exact SCM semantics, MetalNode repacking, per-call private uploads |
| Typing | lattice declarations, semi-naive waves, interface-digest layering, demand by cost, native set-engine techniques for vtab operators, tape-derived construction | whole-program in-memory evaluation (over budget) |
| Moti | per-clause memo keys, read-set memos, the mutant gate | the SQLite replacement (the 500 MB budget favours SQLite on disk) |
| DLRE / SSC / N1562 | the whole-demand batch vtab contract, demand handles, exact-scope grouping, plan identity (`query/plan`), the SSC gate pattern | anchor SQL lowering inside documents, navigation-edge rows, run-time `count(*)` |

## 10. Decisions this shape fixes, and what is left to measure

Fixed by the shape:
- anchor postings are always built in the one store and feed LEAF and the anchor choice;
- invalidation is derivation waves with digest cutoff, not DRed;
- facts and interfaces live in SQLite;
- the set stage is generalised to lattices;
- one interner.

Left to measure (t4 #71; s0 "Open measurements"):
- Q-R1, shared vs private Metal storage;
- M3, facts by vtab;
- the repository-fact cold cost on the cross-file SQL strata (1.5–2.5 s, A);
- 32-bit lattice waves on Metal (< 1 ms, A);
- the v9np stack-carried walker.
