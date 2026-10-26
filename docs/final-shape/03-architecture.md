# Rerail target architecture

Status: **the target architecture, as set by the user on 2026-10-26.** It supersedes `02-shape.md` where they differ (listed in §11). Section numbers §0–§9 follow the user's points. Each section gives the decision, then what the research notes contribute to it (`notes/`, index `notes/s0-synthesis.md`).

Numbers carry M (measured), D (derived) or A (estimated).

Budgets: 500 MB RAM in total, 10 GB disk.

```
  push: edit / fs / git event
        │  invalidate a batch of base facts (path, blob hash, tree hash, mtime, source, language, …)
        ▼
  ┌──────────── FACT LATTICE (system of record: SQLite; fast path: in-memory overlay) ────────────┐
  │                                                                                               │
  │  wave k:  SQL (calls the shots)                                                               │
  │     ├─ pre-filter by SQLite indexes: files · anchor kinds · identifier text   (stat-chosen)   │
  │     ├─ SELECT … FROM demand(batch)      ── vtab ──► combined transducers                      │
  │     │                                              (Metal/CPU interpreted → JIT in bg)        │
  │     │                                              over pinned tapes + programs (GPU cache)   │
  │     ├─ datalog rules as SQL relations + native functions / vtabs                              │
  │     └─ new facts → compare with cached → only CHANGED facts form the delta for wave k+1       │
  │                                                                                               │
  │  repeat waves until no delta (recursion = waves)                                              │
  └───────────────────────────────────────────────────────────────────────────────────────────────┘
        │                                         ▲
        │                                         └─ background writer: overlay → SQLite
        ▼
  findings layer: one layer of full SQL over facts + match sets → findings
```

## 0. Model

- The model is Datalog-like, with two kinds of rule:
  - **generic rules** over facts;
  - **matcher rules**, which are structural patterns over syntax.
- Both are rules of the one program, and matcher rules lower into generic rules plus transducers (§3).
- **Research basis.**
  - The `rerail.facts/v1` algebra (t1 §21; t9) gives the generic rules. Rules carry no SQL.
  - The transducer set mode and the stat-driven indexes in PIAT give the matcher half (t1 spec-transducer, spec-indexes).
  - The output contract is loose: a set of finding keys plus Complete/Unknown. That makes set semantics, fusion and early stop legal.

## 1. A lattice over facts, all stored in SQLite

- Every input is a fact. This includes:
  - Git tree hashes, content hashes, paths, timestamps, language;
  - file source and tapes;
  - derived facts and matches.
- There is no separate "file layer" or "cache layer"; a cache entry is a fact with a version.
- Each fact has a value and the versions of the facts it was derived from. The order is by version, and invalidation moves facts down the lattice and recomputation moves them back up.
- SQLite on disk is the system of record. It can grow to 10 GB, while little has to be resident.
- **Research basis.** Main's content store (CAS) already keys source, tape and results by content (t7). These become fact families of the one store, so content addressing supplies the leaves: a changed byte means a new blob hash and a new tape fact.
- **Disk.** The figures from 02 §5 still hold:
  - tape: 1.77–2.53 B/node compressed (M);
  - source: 0.96–1.23 B/node (M);
  - kind postings: ~2.6 B/node;
  - term postings: ≤ 1.3 B/node;
  - derived facts: under 0.7M rows on the gate corpus (A).

## 2. The push path

1. An event invalidates a **batch** of facts.
2. Their dependents are recomputed in waves (§7).
3. Each new fact is compared with the cached value. **Only changed facts are updated and propagate.** Unchanged facts stay cached, and their dependents are not re-run (early cutoff).
4. Changes are written back to SQLite **eventually, not in the fast path.**
- **Research basis.** This mechanism already exists in several streams:
  - main's `calls` per-consumer stamps;
  - Moti's Y1 read-set memos with cutoff and backdating;
  - Typing's interface digests: a body edit re-derives 1 module instead of 408, and an interface edit 123 instead of 408 (M);
  - PR 2539's dependency cone.

  Here it is uniform over all facts, not special-cased per stream.
- **No DRed and no level witness.** An invalidated fact is recomputed from its inputs.

## 3. Lowering matcher rules

```
matcher rule ──► datalog rules (relations to other facts)
             ├─► SQL relations (pre-filters, joins, anchors)
             └─► transducers ──► combine + optimize across all rules
                                   ├─► Metal / CPU interpreted (available at once)
                                   └─► Metal / CPU JIT (built in background, swapped in)
```

- **Combine and optimize.**
  - Shared sub-rule evaluation is the primary win: 535 queries collapse into one hash-consed graph of 6,821 ops, 1.4–3.1× faster (M, t1).
  - Also: fusion by shared root kind or target, set mode, lazy cursors, and existence and count barriers (t1 spec-fusion, spec-transducer).
- **Interpreted, then JIT.** The interpreted and JIT forms are two bindings of **one plan**, with the same plan identity and the same outputs. Tempo supplies the IR, the verifier and the `native_ir` codegen (t6).
- **Execution.** All transducers of a wave run as **one batch through a virtual table**: `demand(batch)`, one xFilter per (group, batch). The boundary costs 4.9% (M). This is the DLRE/N1562 whole-demand batch contract (t11).
- **Metal and CPU.** Every op must exist on both, as the verifier requires. There is no CPU fallback for Metal.

## 4. SQL controls execution; state is in SQLite and may be cached in memory

- SQL statements drive the waves. Virtual tables and native functions take over the parts SQL does badly:
  - the transducer batch;
  - set and lattice operations;
  - sparse closure walks;
  - regex through `tid`.
- **State.** SQLite is authoritative, and hot state may also be cached in memory behind a native function or vtab. Every fact read goes through an access path that sees **overlay ∪ SQLite**.
- **Staleness.**
  - Out-of-date facts must be detectable: a fact row carries the input versions it was derived from, and the access path compares them with the current versions.
  - Rewriting stale rows happens only in the background.
- **When the writer lags,** evaluation still runs. Its results stay in the in-memory overlay until written back. The overlay is part of the 500 MB budget, so the writer needs back-pressure: if the overlay reaches its cap, flushing takes priority over the next wave.
- **Research basis.**
  - 16.4 s of SQLite table loads for base facts (M, t3) is why facts must be readable through a native access path (M3, facts by vtab).
  - Main's ≤ 64-object publication transaction already sits outside evaluation (t7).

## 5. GPU memory: pinned programs and tapes, prepared per wave

- The GPU cache holds pinned compiled programs (pipelines, interpreted and JIT) and decompressed tapes with their sidecars (`par`, `cidx`, line starts, `tid`).
- **Wave preparation** computes the batch's needs from SQL (documents × groups), loads what is missing, and only then dispatches.
- **All pinned GPU memory is dropped two minutes after each full execution.**
- **Research basis.**
  - PIAT §20 residency: zstd frames are decompressed directly into 64 MiB slabs, with one retained no-copy shared `MTLBuffer` per slab. Batches reference documents through a document table, with no copies or repacking (t1 §20).
  - The edited document's tape goes straight into a pinned edit slab, so nothing is uploaded.
  - Tempo designed the identity-keyed tape cache but never implemented it (t5, t6).
- **Budget.** On Apple unified memory, pinned GPU buffers count against the 500 MB. At 8.25–8.55 B/node, the whole gate corpus (28.23M nodes) is about 240 MB of tape alone (D), plus about 56 MB for `par` and the source. So the cache is bounded, and what it pins is the **pre-filtered** working set (§6), not the corpus.

## 6. Statistics-based lowering and pre-filtering by SQLite indexes

- Pre-filtering uses SQLite indexes, so that **only the best anchors are used and each batch carries as little work as possible**:
  - files: path, language and changed-set facts;
  - anchor kinds: the positional kind FTS (`search_kind_fts`), always built;
  - identifier text: value and term postings.
- **Decisions taken from statistics** (`search_kind_vocab`/fts5vocab, `edge_freq`, `kind_size`, `fact_stats`, `plan_feedback`, all in SQL):
  - the anchor: the rarest mandatory concrete variable, value before kind;
  - relation form: cursor, kind scan or set pass;
  - climb vs descent;
  - bitset vs postings;
  - fusion groups;
  - conjunct order;
  - SQL join order, written to `sqlite_stat1`;
  - batch size.
- Decisions are recorded in `compiled_plans` with the statistic intervals over which they stay valid. Nothing counts rows at run time (DLRE trap T18).
- **Research basis.**
  - ast-index-spike: a positional kind index took an identifier anchor from 1.8 s to 64 ms (M).
  - t4 #20–28 and decisions 0006 cover the rest.

## 7. Everything is set-based; recursion is waves

- Every operator consumes and produces sets.
- **Recursion through facts is waves.** Each wave consumes the previous wave's delta (semi-naive), and the program reaches its fixpoint when a wave produces no changed fact. Cross-file recursion therefore takes several waves; examples are call closure, interface layers and type propagation across modules.
- **Bounded recursion entirely inside a matcher** is resolved inside the GPU program. Examples are the tree axes (`inside`, `has`, `stopBy`), bounded utility calls and existence/count barriers. Tree structure is tape arithmetic, never Datalog recursion: a scope is one preorder pass, 4 ms for 1.2M nodes against 4.5 s recursive in SQLite (M, t9).
- **Research basis.** The Metal hybrid waves cost 1.2–1.6 µs each (M) when the GPU schedules them. A wave here is one SQL step plus one vtab batch, so what matters is the number of waves, not the cost of each. Typing's condensation has 13 layers (M). A wave always covers every document in the frontier, never one per file.

## 8. Findings layer

- After the Datalog fixpoint, **one layer of full SQL** turns facts and match sets into findings.
- It sees whole sets and may do non-Datalog computation: aggregation, windows, ranking, formatting.
- It is not part of the fixpoint and feeds nothing back.
- It may later run after each wave, so that facts can be more complex, but not now.
- **Research basis.** This is where PR 2539's per-rule SQL over facts and the loose findings contract meet. It also keeps non-monotone SQL out of the lattice, so §2's cutoff stays sound.

## 9. Native functions anywhere; SQL calls the shots

- Native functions and virtual tables may appear at any point to improve speed.
- Control flow, scheduling and the order of joins and waves belong to SQL.
- **Research basis.** This matches the earlier positions: no SQL-vs-Rust dichotomy, push slow constructs down into native code, and let the SQLite optimiser see the relational shape (memory: rule-engine-direction).

## 10. Consequences that need decisions or measurement

1. **Pinned cache vs 500 MB** (§5). The cache is bounded, and pre-filtering decides what it holds. Measure:
   - Q-R1, shared vs private storage;
   - Q-R7, pin cost per GB;
   - the candidate working set after pre-filtering on the gate corpus.
2. **Overlay size when the writer lags** (§4). A cold audit produces every fact at once, so set an overlay cap and flush policy inside 500 MB.
3. **Per-document lattices** (Typing R0, constant values) are recursion through facts, so by §7 they are waves. The alternative is to declare them bounded and in-matcher, which would run them as GPU FIX waves inside one dispatch. This decides how many SQL-level waves a per-edit check needs.
4. **Fact granularity for cutoff** (§2). Compare per fact row or per (relation, partition) digest. Per-partition digests are cheaper and match Typing's interface digests; per-row is finer.
5. **Repository-fact cold cost.** Estimated at 1.5–2.5 s (A), against the 1–2 s audit target. Measure it on this path.
6. **One interner** for `tid`, fact atoms and paths (s0 D4). It needs a measured choice between CAS `strings` and a dense append-only table.

### Answers (user, 2026-10-26)

1. **Pinned cache.**
   - Pre-filtering and memoised facts mean incremental work needs very few files.
   - A full scan runs in chunks, preparing the cache for chunk k+1 while chunk k computes.
2. **Interim facts.**
   - Write-back cannot fall arbitrarily far behind.
   - Interim facts need an efficient in-memory format.
   - Proposed format, to be confirmed:
     - append-only columnar runs per relation, using u32 atoms;
     - spill to a flat file when the overlay reaches its cap;
     - the writer merges the runs into SQLite in bulk.
3. **Per-file lattices.** Re-asked in a concrete form; still open.
4. **Cutoff granularity.**
   - Coarse is acceptable, as long as spurious invalidations do not greatly increase the work. This is a practical question and should be checked with a counter of re-runs against actual changes.
   - A recomputed fact (or partition) that is exactly the same as last time stops further evaluation.
5. **Repository-fact cost.** Explained in detail; still open: is the 1–2 s target for a cold audit or a warm one?
6. **One interner.** Yes.

## 11. What changes from `02-shape.md`

| 02 | Now |
|---|---|
| Invalidation as a derivation graph one level above the facts | The lattice is over facts themselves; files, hashes, tapes and source are facts; the push path invalidates and replaces batches of facts |
| Synchronous publish per edit | Write-back is in the background; results stay in an in-memory overlay until written; staleness is detectable |
| Document-local strata, including lattices, inside the transducer | Only bounded in-matcher recursion is in GPU programs; recursion through facts is waves |
| Interpreter or JIT as alternatives | Interpreted first for reactivity, JIT built in the background and swapped in |
| Residency waves, with pinning undefined | GPU cache of pinned programs and tapes, prepared per wave and dropped 2 min after a full execution |
| Findings as derivations within the program | One findings layer of full SQL after the fixpoint |
