# S0: cross-track synthesis (regenerated context)

Purpose: answer the nine research questions in `RESEARCH.md` from the track notes alone, so the design can be written from this file and the notes without re-reading sources or relying on memory.

Rules:
- Every claim cites a note as `tN §section` (or `tN:line` where the note has no section).
- The source's M/D/A mark is kept on each number.
- Where tracks disagree, the disagreement is listed under "Contradictions", not resolved silently.
- Decisions that belong to the user are listed under "Open decisions". Measurements that settle a question are listed under "Open measurements".

Tracks: t1 PIAT spec, t2 PIAT research core, t3 PIAT costs/ledger, t4 ast-index-spike, t5 Tempo residency, t6 Tempo transducer/Metal, t7 main store/invalidation, t8 PIAT branch docs, t9 Typing-support, t10 Moti facts, t11 DLRE/N1562/SSC since the review.

---

## 0. The governing shape, as the sources state it

These come before the nine answers, because every answer depends on them.

1. **The execution unit is a batch: many documents × many program groups.** It is never one file.
   - Tempo's execution unit was already cross-file batched: one Metal invocation covers every (file, program, root) task (t2 transducer-ir §1).
   - The spec defines a batch as documents × the program groups assigned to them. Tasks are (group, document, root), and dispatches cover all documents of the batch (t1 spec-fusion "what batch means").
   - Audit batches are cut by resident bytes, never by file count (t1 §11, §18).
   - The PIAT GPU producer contract: "Resident input and result capacity determine execution partitions. SQL insertion batches, publication pages, file boundaries, and exact signatures cannot determine dispatch boundaries." Also: "100× more roots/files must keep the same stage count; only true dependencies add waves" (t8 §4).
2. **Per-edit is the batch that arrives: the edited document × all rules, with the same plan class and groups** (t1 §14 note; t1 spec-fusion §6.11).
   - Repository facts come from the snapshot (t1 §14).
   - fusion.md's "per edit: fused dispatch without an index, for one file" is CPU advice that predates "never per-file" (t2 fusion §cost model, note).
3. **The win comes from shared evaluation across rules and files, not from cheaper single evaluations.**
   - M2: 0.26–0.40 ns per program-node at 13–17 ns per evaluation; the pass comes from 20–40× fewer evaluations (M) (t2 m2-m7).
   - SSC/DLRE: moving one operator native is not automatically a CPU win (DLRE native child 105.8 vs SQL 106.8 ms; SSC native waves ~0% whole-job) (M) (t11 S.1).
   - Shared sub-rule evaluation is the primary lever: 535 queries become one hash-consed graph, and text scanned falls from 2.59 TB to 3.1 GB (t1 §7).
4. **Tempo's authors reached the same conclusion from the Metal side:**
   - "The executor shape is the primary constraint. Input packing, Program lowering, continuation, and publication must conform to this shape."
   - "The measured difference is not query count or corpus size. The difference is execution shape." (t6 S6)
5. **Completeness of an implementation is not evidence about shape** (user instruction, turn 2).
   - Tempo's Metal executor refusing at 600 dispatches (t6 S8) is evidence about *exact SCM semantics on a path-enumerating model* (t2 transducer-ir "inherent vs implementation"; t6 S2.3). It is not evidence against Metal or against batching.

---

## Q1. Transducers

### The IR (base)
- **Encoding:** Tempo's 32-bit word `opcode:u8 | mode:u8<<8 | operand:u16<<16`; ≤ 65,536 words and ≤ 65,536 entries per table; tables hold data, never hidden instruction sequences (t6 S1.1).
- **Identity:** a canonical digest over the format, grammar, words and tables; machine code is never hashed (t6 S1.1). The spec adds SHA-256 over the normalised serialisation, which keys the plan cache (t1 spec-transducer §3).
- **Opcodes** 0x00–0x40: tests, navigation, state, control (t6 S1.2). The spec reserves new ranges for tests, axes, calls, set stage, documents/side facts, patterns and line documents (t1 spec-transducer §2).
- **Side programs:**
  - value: Equal, Prefix, Regex byte NFA, Lua;
  - capture: Equal, Text, AnyOf, Kind, Contains, relations, SameLine;
  - output: #set!, #offset!, …
  
  (t6 S1.4; t1 spec-transducer §4.)
- **Tempo's machine model:** path-enumerating DFS from roots, with an ordered capture list, cursor slots and repeat counters; FORK defers; only EMIT outputs (t2 transducer-ir §1; t6 S1.3).

### What the final shape changes (PIAT spec-transducer)
- **Set mode.** Output is a set over terminal events, independent of path order, task order and run multiplicity. Executor freedoms follow: stop a KEY label at its first emit, prune paths whose reachable labels have all fired, lazy enumeration (t1 spec-transducer §1).
  - This rests on the findings contract (findings are a set; memory `findings-contract-loose`).
- **Barriers.**
  - EXISTS/NOT_EXISTS: snapshot scoping, nothing flows out, nesting ≤ 8.
  - On Metal, shallow barriers (≤ 2 levels, no unbounded axis) run inline. **Deep ones are promoted to the set stage as IN_SET, so a thread never holds an unbounded stack** (t1 spec-transducer §2.3).
- **Bounded COUNT** and COUNT_PREFIX (t1 spec-transducer §2.5).
- **CALL/RETURN.** Recursion only with monotone single-direction progress. **Recursive groups never recurse per thread; they lower to set-stage FIX waves** (t1 spec-transducer §2.4).
- **The set stage (E14)** produces per-document node sets S ⊆ [0, n) before the programs that read them:
  - definitions: LEAF (candidates from kind postings or a kind scan), HAS, INSIDE, PRECEDES/FOLLOWS, AND/OR/ANDNOT, COUNT_PREFIX, FIX, FACT;
  - representation: bitset or sorted postings, chosen from the bound;
  - multi-target packing: up to 32 or 64 sets per pass as bit-planes;
  - limits: ≤ 16 strata.
  
  (t1 spec-transducer §2.6.)
- **Lazy enumeration** is normative: one resumable cursor per enumeration; pushed paths fell 5.26M → 96k (M) (t1 spec-transducer §2.7; t2 fusion).
- **SIDE columns and FACT sets** carry fact reads into programs (t1 spec-transducer §2.8, §2.6). E15 KEYS was withdrawn for L5 and E17 FOLD_UP for the native `py.flow_exit` (t1 spec-transducer §2.11; t2 derived-facts).
- **The lowering principle** sends each condition to one of three places (t1 spec-transducer §0):
  - transducer program: local structure;
  - named derived fact: relational analyses;
  - native producer: resolver or evaluator facts.
  
  "A transducer program should read like a structural pattern; anything that reads like a query belongs in a derived fact."
- **Verifier:** ten rules, including stratified recursion and **"every backend implements every op."** Every program must be Metal-capable; a non-Metal program is a compiler bug, not something routed to the CPU (t1 spec-transducer §9; t1 spec-tooling §2.3).
- **Backends** (t1 spec-transducer §10; t1 spec-fusion constraints):
  - the Rust interpreter is the reference;
  - CPU JIT (arm64);
  - Metal: interpreter, then MSL codegen from the shared `native_ir`.
  
  The same abstract work units are charged on every backend. There is no Metal → CPU fallback.

### Why Tempo's Metal run failed, and why the set formulation does not
- **Tempo's constraints** were exact tree-sitter query semantics:
  - backtracking quantifiers;
  - longest match as ordered capture containment;
  - raw candidate ordinals as a public contract;
  - an observable directive order.
  
  These forced path enumeration with deferred branches, containment reduction and per-generation exact dedup (t6 S1.6).
- **Dispatch growth** (t6 S2.3):
  - 44 launches per generation, later 18–20;
  - deep captured repetition: 19 generations × 20 = 380 (M);
  - 594 dispatches completed at the 600 cap; tracked memory 1.98 GB of 2e9 (M) (t6 S8).
- **Memory growth** came from worst-case per-task/per-lane arenas, one shared `states` estimate (13.65 GB logical for a 92 KB file) and capture-prefix copies (t6 S2.4).
- **The PIAT reading (t2 transducer-ir):** these costs come from exact SCM semantics plus per-batch repacking. The inherent issue is path enumeration with no existence or aggregation barrier. Set mode, EXISTS and COUNT remove the cause.
- **Measured set-formulation evidence on Metal** (M5 Max; ast-based tape, 5.4M nodes):
  - posting-owner operators: 0.005–0.05 ms GPU per whole-corpus query;
  - SIMD interval scan: ≤ 0.86 ms (t2 metal-recursion);
  - GPU-scheduled hybrid waves: 1.2–1.6 µs per wave against 386–890 µs for a CPU round trip; a 27-wave wide fixpoint takes 1.57 ms GPU (t2 metal-enqueue).
  
  The set stage is stratified Datalog over node sets, with any axis in either direction; only unstratifiable programs are rejected (t2 metal-enqueue §3).
- **Tempo's own grid** spike met full production volumes in 34 invocations and 40 ms (M), but it does not prove semantics (t6 S2.2; t2 transducer-ir §3).

### Fusion
- **Normalisation** canonicalises programs and deduplicates equal digests into one program with several labels (t1 spec-fusion §4.1).
- **Grouping** uses (scope_digest, grammar, backend_profile) with **identical scope only**: 1.45× against 0.86× for a union (t1 spec-fusion §4.2). All three streams agree on exact-scope sharing (t11 S.1).
- **Recursive prefix tries; boolean fusion; relation lowering by cost** (cursor, kind scan or set) (t1 spec-fusion §4.3–4.6).
- **Priority order revised after M2** (t1 spec-fusion §4.11):
  1. interned text tests and L5 joins;
  2. recursive tries with conjunct ordering (2.95 → 0.26 ns/pn, M);
  3. hash-consed sub-rules;
  4. memo columns only for expensive non-text sub-rules;
  5. masks last.
- **Legality:** an exported capture pins a path and is the main legality constraint (t1 spec-fusion §5).
- **Cross-document work is never fused.** Cross-file composition is SQL over t_export, fact_* and graph vtabs (t1 spec-fusion §6.7).
- **On exhaustion** the planner resubmits on Metal with the group split, never on the CPU (t1 spec-fusion §6.10).

### Defects and gaps to carry into the design
- **FLAGS.** The spec copied Tempo's generic interpreter path, which never tests the required bits. The production path at `transducer.rs:2679-2682` is correct: bits named=1, extra=2, missing=4, error=8, with required/forbidden masks (t6 S1.2, S7.2; RESEARCH log 2026-10-26).
- **Current state of each codebase:**
  - **Main runs no transducer in production:** it uses the ast-grep matcher per file with a kind-presence filter (t7 T7 synthesis).
  - N1562's packed IR lowers none of TP/ASD/SAS (t2 m2-m7).
  - N1562 lowered tree relations to SQL over navigation-edge rows, the spike's dead end (t2 transducer-ir §0.2).
  - Its navigation producer still emits every parent→child edge of a file, which is in tension with SSC and DLRE (t11 S.2 #11).

---

## Q2. Tape and disk: why there is no layout expansion

- **Final format v9np, structure only** (t1 §3; t2 tape-v8):
  - columns: `kf` u16 (10-bit folded kind + 6-bit field; named, extra, missing, error and aliases as distinct codes), `size` u16/u32 per document, `base` u32 per 16 nodes, `off` u16, `tail` u16;
  - `par` is not stored.
- **Size** (M, 24.2M nodes):
  - logical 8.25–8.55 B/node = 0.93–1.01× v7;
  - zstd-3 1.77–2.53 B/node = 0.94–1.06× v7;
  - source zstd 0.96–1.23 B/node.
  
  At rest, source and tape are about 1 + 1.8–2 B/node (t2 size tables).
- **The expansion candidates, measured and rejected** (t2 tape-v8; t2 tape-regularity; t5 §15):
  - v8 (absolute start/len): 1.24–1.59× logical and 1.47–1.90× zstd;
  - stored `par`: +2 B/node (10.25–10.84 logical);
  - interleaved layouts compress 6–21% worse;
  - Tempo's 44 B/node `MetalNode` and ~40 B/node `TapeDoc` exist only as expansions;
  - DLRE's `decoded_*` caches were 211 B/node on SQLite, 88% of the store.
- **Derived on load, kept as resident sidecars, never stored:**
  - `par`, built by a stack pass at 4.1–5.1 ns/node (M); +32–46 MB for the gate corpora (D);
  - `cidx`, line starts, text hashes, `tid`.
  
  (t1 §20.6; t2 tempo-residency footprint.)
- **Spike disk rules carried as constraints** (t4 §30):
  - #1: no relation stores one row per node; at most 0.25 rows/node; enforced by test;
  - #2: any node-level relation is charged against the budget;
  - #12: budgets are tape 6 B/node and all secondary indexes ~5 B/node; a new index must fit or replace one at least its own size;
  - #14: costs add across features.
  
  The spike's own tape stored `par` and still missed 6 B at 8.66 (t2 E34). v9np drops `par` and matches v7.
- **Index budget in the spec:** ≤ 4.1 B/node (kind 2.6 + term 1.3 + edge 0.15 + sets 0.01) (t1 spec-indexes §2.14).
  - Tempo's Schema 12 FTS measured 2.998 B/node in total, kind 1.172 B/node (M) (t5 §14 missed-7).
- **What the disk budget rules out for facts.** These copies must not become a second persisted copy of the tree beside the tape:
  - Moti's core facts on disk: `RRFACTS` 6.6 B/node, 186.4 MB (M) (t10 Q1);
  - Typing's exported `facts.sqlite`: 175 MB for 1.2M named nodes, a spike export (M) (t9 §5.2).
  
  Typing's own conclusion: "construction should come from the tape and anchor index"; "tree structure … is never Datalog recursion" (t9 §5.3). The base structural relations are the tape plus postings.
- **Machine disk limits** (t7 CAWS):
  - store < 10 GB (7 soft / 9 hard / 10 absolute) for about 100 workspaces;
  - content addressing measured at 0.87 GiB against path addressing at ~3.0 GiB (M/D).

## Q3. Residency and caching

- **What Tempo has** (t5 §15 Q1–Q2):
  - zstd-3 frames in a machine-wide CAS;
  - a bounded 32 MiB / 30 s decompressed cache keyed by row id;
  - per-evaluation repacking into 44 B MetalNode heap Vecs;
  - a per-call private MTLBuffer upload, rebuilt every time.
  
  **A pinned uncompressed tape store does not exist in Tempo;** the tape cache by identity is designed, not implemented.
- **Tempo evidence** (M) (t5 §15):
  - first-use mapping 213.3 ms against 6.1 ms GPU;
  - buffer reuse 230.5 → 3.8 ms and 110.6 → 2.4 ms;
  - mapped no-copy warm 1.47 ms against cold 39.12 ms;
  - **demand-zero no-copy: kernel time 407.2 ms against 26.7 ms**, i.e. first-touch faults inside a kernel are expensive;
  - the 32 MiB cache had 0 hits on a full scan, which reads each tape once;
  - warm re-scan: 37.6 worker-seconds decompressing against 0.003 s matching;
  - heap decompression left 3.3 GiB of malloc high-water.
- **Correction to the PIAT reading (t5 §14):** Tempo's own conclusion for warm workspaces was **result reuse, not a bigger tape cache**.
  - With result reuse, 47 warm workspaces decompressed no tape, at a peak RSS of 499 MB (M).
  - Tempo also concluded that a process boundary is required to return allocator high-water.
- **Final form (spec §20; t1 §20):**
  - at rest: zstd-3 frames;
  - resident: decompressed **directly** into anonymous-mmap 64 MiB slabs with 16 B-aligned columns, byte-identical to the at-rest blob;
  - device view: one no-copy shared `MTLBuffer` per slab; **pinned means retained across dispatches and evaluations**;
  - validation (SHA-256, length, codec) once per admission;
  - batches **reference** documents through a batch document table `(slab, tape_offset, node_count, width, source_offset, …)`; nothing is concatenated or copied;
  - batches are width-homogeneous;
  - classes: `snapshot` (pinned), `edit` (pinned, newest overlay), `transient` (LRU);
  - audits over budget run in residency waves;
  - per edit, the tape is encoded straight into a pinned edit slab, so there is zero upload;
  - forbidden: repacking, concatenation, heap decompression, copy on hit, re-hash, per-object mmap, small TTL caches, partial DONTNEED, mlock.
- **Footprint** (D): CPython ≈ 99 MB, data-layer ≈ 158 MB resident (+32–46 MB `par`). **The whole gate corpus stays resident** (t2 tempo-residency).
- **How the two warm mechanisms combine:**
  - result reuse (main's keys, Q6) means unchanged files need no tape at all;
  - pinned residency serves whatever executes: cold audit, a rule-set change, cross-file re-evaluation and the edit slab.
  
  Both are needed; neither replaces the other (t5 §14 missed-1; t7 T7 synthesis).
- **Open measurements:** Q-R1 private vs shared storage (no Tempo comparison exists; t5 §15 Q2 and contradictions 3), Q-R2 ResidencySet, Q-R3 binding limit, Q-R7 pin cost per GB, and Q-R8 cold admission throughput (t1 §20.12).
  - Pre-touching pinned slabs (`residency.pin`) answers the first-touch evidence above.

## Q4. Fact storage

- **Producers (spec §21; t1 §21):** exactly one producer per fact:
  - fact program: a transducer program producing base facts from tree structure;
  - native producer: resolver or evaluator facts;
  - derived fact: closed algebra (from, join, anti_join, union, group/aggregate, project, stratified recursion), no SQL, no UDFs.
  
  "Shared does not imply SQL."
- **Admission:** concept-named, ≥ 2 rules, rows per document ≤ node count, no rule seeds (t1 §21.3). This guards against PR 2539's "Process Failure" (t2 derived-facts).
- **Identity:** `fact_digest = H(lowered def ‖ schema ‖ input digests ‖ compiler version)` (t1 §21.4).
  - Moti's equivalent: memo keyed by (content hash, plan digest, schema version[, path]), with content-pure clauses path-free (t10 Q2).
- **Storage:**
  - narrow typed tables with atoms interned as u32; no EAV and no unions;
  - one physical owner per (doc or snapshot, digest);
  - base facts preferably by vtab over native columns (M3);
  - cache tables `derived_facts` / `derived_fact_parts(digest, doc_key, complete, reason, rows)`.
  
  (t1 §21.6–21.7.) The PIAT branch docs state the same ownership rule: "Reading a relation several times is valid. Reconstructing its unchanged covered rows is not." (t8 §1)
- **Inventory on the gate corpus:** 9,010 files, 28.23M nodes; all derived facts < 0.7M rows (A); repository facts estimated at 1.5–2.5 s cold (A) (t2 derived-facts).
- **The cost that decides storage:** loading 7.9M fact rows as real SQLite tables takes 16.4 s (M), against an estimated ~1 s by vtab (A) (t3 §1 HRP).
  - Query wording swings 22× and 34× (M); the algebra exists so the compiler chooses the physical form (t2 derived-facts B2).
- **Measured native alternatives:**
  - **Typing native program mode** (t9 §3.3–3.5, Synthesis):
    - set-valued strata run on a native set engine; relations stay native between strata; SQLite holds only fact loads, non-native strata and outputs, 6–11% of samples (M);
    - arena store: flat rows, row-id hash dedup, postings as chains through one u32 `next` slot per row, deltas as chain prefixes, CSR for immutable inputs;
    - one global symbol interner;
    - Python 1.48–1.67 s exec / 640 MB; Rust 1.13–1.20 s / 450 MB (M).
  - **Moti** replaced SQLite because of per-statement and B-tree cost (set-composition ~3× native; 198 s of stage builds in batch 6). SQLite stays as the semantic definition and differential oracle (t10 Q7).
- **Interners in the sources:**
  - main's CAS `strings` (domain, content hash, collision ordinal) (t7 CAWS);
  - spec `text_vocab(id, bytes)` with a per-token `tid` sidecar (interning 0.22 s, M) (t1 spec-transducer §5.2; t2 regex);
  - §21 u32 atoms;
  - Typing's global interner (t9 §3.4);
  - Moti's process-wide kind/field interning (t10 Q1).
  
  No source relates them (t2 derived-facts "open"; t7 tensions 3). → Open decision D4.

## Q5. Lattices and inference

- **Strategies on the whole corpus, one thread** (M) (t9 §4.3):
  - semi-naive frontier waves: 10–11 ms for exact types, 6–8 ms for `cv`;
  - RPO worklist: 13 ms;
  - SCC condensation: 9–27 ms;
  - naive rounds: 150–550× the visits, "never an option".
- **Lattice types:** flat `cv` (height 2), exact type union, and union capped at K=4 (height 5). The cap changed nothing on this corpus (M) (t9 §4.2–4.3).
- **Declared absorbing elements** on transfer functions halve-plus the rounds (2.5×, M), and need a native evaluator because "a SQL join can't fire on one input" (t9 §4.8).
- **Parallelism should come from documents and from the modules of a layer, not from within a document** (frontiers ≤ 1.3k nodes) (t9 §4.4).
- **Cross-file layering with an interface-digest cutoff** (M) (t9 §4.5):
  - 13 layers; 46–71 ms total;
  - a body edit re-derives 1 module (vs 408); an interface edit re-derives 123 (vs 408).
- **Metal estimate** (A): ~24 waves × 1.2–1.6 µs + ~150k node evaluations, < 1 ms. It needs 32-bit lattice words and declared widening, with arenas ≤ h × n (t9 §4.6).
  - "The same 15 waves cover the cross-file edges if the corpus is one graph. Layering exists for caching, not for the GPU."
- **Tree structure is never Datalog recursion.** Scopes are a tape column: one preorder pass takes 4 ms for 1.2M nodes, against 4.5 s recursive in SQLite (M) (t9 §4.1, §5.3).
- **Demand:**
  - pays when the demanded keys are few relative to the relation (SQL 16.3 → 4.2 s, M);
  - a one-file magic-set query was 13× slower (168 s, M) because of first-column-only adornment and hand-written demand rules;
  - the native resolver is per file, from summaries (t9 §6.3–6.5).
- **Relation to the transducer set stage:**
  - both are stratified Datalog evaluated in semi-naive waves (t2 metal-enqueue §3; t9 §4.7);
  - the set stage's verifier **rejects recursion through facts from other documents**: cross-document closure belongs to SQL/graph producers (t2 metal-enqueue §5);
  - so node-local lattices (per document, R0) can share the wave machinery and batch shape, while cross-file lattice layers belong to the facts engine.
  
  → Open decision D2.
- **Typing requirements** (t9 §8):
  - no embedded type checker;
  - progressive approximate inference;
  - both "provably true" and "not provably false";
  - user-supplied types;
  - first-class generics.
  
  The spike has no evidence yet on the last three.

## Q6. Invalidation

- **Main today, by key and never by deletion** (t7 Q6):
  - objects are content-addressed;
  - per-file results are keyed by (source, file_context = language + path + module + package, check policy within catalog);
  - directory reuse keys on (Git tree, prefix, package chain, catalog);
  - cross-file evidence uses partitions plus a global re-answer with a per-path diff;
  - `calls` uses per-file summaries, sparse backward closure and **per-consumer stamps**: a consumer re-runs only if what it reads of the closure changed. This is a shipped read-set memo with early cutoff.
  - **Cross-file rules are excluded from tree-level result reuse** (t7 CAWS :68).
- **Tempo:** a changed source or language fully invalidates that file's state; results are keyed by canonical source + catalog (t5 §15 Q4).
- **PIAT spec §21.8** (t1):
  - the dependency DAG comes from the definitions;
  - per-document facts are recomputed for the edited document;
  - repository facts update by semi-naive Δ, with DRed for recursive components;
  - only rules whose recorded read set intersects a changed (fact, key) re-run.
  - Per-edit target M8: ≤ 1 s p95 (t2 derived-facts B6).
- **Moti** (t10 Q3, Q5):
  - Y1: keyed tables with `Box<[u32]>` read-set memos (probed keys hashed, **absent keys included**), level-order re-run, early cutoff and backdating;
  - **Y2: a level witness beat DRed (max over-deletion 80 vs 388,040; p95 1.5 vs 394 ms, M)**;
  - Y3: exact on 1,151 checks at 0.53 / 3.76 ms per edit, but 1.94–2.09 GB heap against a 250 MB target that no design met.
- **Typing:** interface digests per module, with cutoff measured at 1 vs 408 and 123 vs 408 (t9 §4.5). Per-document Ascent: body edit 3 ms; interface edit 111 ms for 123 documents (M) (t9 §6.2).
- **PIAT branch docs: non-local cases that must invalidate unchanged files** (t8 §2):
  - aggregate count gates;
  - gate-eligibility changes;
  - callee, import inventory or class-name uniqueness changes;
  - "a small edit can cause repository-wide row work".
- **All of these are one mechanism, with three granularities of read set:**
  - main's stamps (per consumer, over the closure);
  - Moti's Y1 memos (per reader, over keys including absent ones);
  - Typing's interface digests (per module, over exports).
  
  Each is a derivation keyed by (own inputs, digest of what was read), with early cutoff when that digest is unchanged. → Open decision D3 is the deletion strategy: DRed or a level witness.

## Q7. Indexes and statistics

- **Main** (t7 schema):
  - `search_kind_fts` contentless, detail=full, offset = preorder ordinal, token `k{kind:04x}` (**local codes, mixing languages**);
  - used for presence only; no vocab, edge or size statistics; no plan cache.
- **Spec-indexes** (t1 spec-indexes Part 2):
  - one store keyed by `tape_id`; DLRE's private store retired;
  - global codes `lang·1024 + local` in the index only;
  - `search_kind_vocab`, `edge_freq` (≤ 0.15 B/node), `kind_size`, `search_term_fts` (≤ 1.3 B/node), `backend_costs`, `plan_feedback`, `compiled_plans` with decisions and validity intervals;
  - statistics read at plan time only; no run-time `count(*)`.
- **Spike constraints** (t4 §30):
  - #15: indexes may over-produce, never under-produce;
  - #16: absence proves empty;
  - #17–18: a mandatory case-exact value index over a deduplicated vocabulary;
  - #20–22: no corpus-wide statistic; persisted tiny aggregates;
  - #23: no node-level edge index;
  - #26: anchor on the rarest mandatory concrete variable;
  - #61–62: postings spliced, O(inserted); two loads equal one load.
- **Uses of postings in the final shape:**
  - set-stage LEAF candidates (t1 spec-transducer §2.6, §8; spec-tooling's rir shows `S0 = postings(kind=yield)`);
  - M2's dispatch through a kind-postings index (t2 m2-m7);
  - Metal posting-owner operators at 0.005–0.05 ms, "the same postings as the kind index" (t2 metal-recursion).
- **DLRE's anchor SQL** was the largest execution cost: 537 s of anchor selection against 58 s of matching, with run-time `count(*)` (trap T18). Later lab fixes brought it down sharply but it is still the top line (t3 §14–16).
  - In the final shape, in-document anchor work is transducer/set-stage work fed by postings, not SQL lowering (t2 sql-vs-transducer §2).
- **Typing statistics:** 0.73 B/node, built at ingestion, merged per batch, fingerprinted, read in one statement (M) (t9 Synthesis).
- → Open decision D1: positional anchor postings.

## Q8. The vtab boundary and the outer SQL

- **Spec §9 interface:**
  - `demand(program_group, doc[, root][, exports])`, one xFilter per (group, batch);
  - capture-bearing demand goes through a single-use demand handle;
  - outputs: `t_match`, `t_export_<schema>`, `t_count`, `t_outcome`, `fact_*`, `d_*`;
  - no rows per node or edge, no wide unions, no per-node callbacks.
  - **Measured** (t1 §9): the boundary costs 4.9% (4.06 of 83.56 s), ~55 ns per root and 1.2 ms per xFilter (M).
- **All three streams agree** (t11 S.1):
  - whole-demand batch vtab (`sqlite3_vtab_in` or an equivalent frame);
  - xNext emits completed rows only;
  - empty demand means no work;
  - incomplete is never negative;
  - statistics are read once at plan time.
- **The PIAT GPU contract** (t8 §4):
  - "vtab callbacks only read prepared data; never dispatch producers or wait on a device";
  - collect demands across ready files and clauses, run one producer wave, then resume SQL.
  
  This is what keeps the SQL layer from forcing per-file execution.
- **SQL owns:** cross-file composition (join/anti-join/count/closure), allowlists, budgets, incremental invalidation, consolidation and the pure-SQL fallback; the fallback lowers each program to specialised SQL over a tape TVF, and a generic interpreter is forbidden (t1 §10, §13; t2 sql-vs-transducer §4).
- **Disagreements among the streams** (t11 S.2):
  - a hard 100-statement gate vs diagnostic counts;
  - exact logical-effect ledgers vs diagnostic;
  - ordered regions kept native vs lowered to algebra;
  - vtab output ordering.
  
  Under the loose findings contract, exact work ledgers and memo-replay accounting are excluded (t1 §6 "Excluded").

## Q9. Tooling

- **Memory pools sized at plan time** (t1 spec-tooling §2.1):
  - `tape.resident`, `source.resident`, `tape.derived`, `batch.doctable`, `set.<stage>`, `output.*`, `queue.*`, `facts.*`, `sql.temp`;
  - device cap 50% of the recommended working set; host 25% of RAM;
  - exceeding a pool gives `Unknown(resource)` and never reroutes.
- **Debugging:** `rir` dumps and an assembler; `explain` over `compiled_plans` decisions; `--trace`; Metal debug mode; repro bundles; `rerail engine replay` (t1 spec-tooling §2.2).
- **Testing:** differential finding-key sets across interp/JIT/Metal, fused/unfused, vtab/pure-SQL, engine/ast-grep and engine/oracle; proptest/fuzz; v7 ↔ v9np round trip (t1 spec-tooling §2.4).
  - Moti adds the mutant gate: one mutant per rewrite, and a crash is an error, never a kill (t10 Q6).
- **Profiling:** a fixed span taxonomy, always-on counters, one JSONL profile; the gate reads only end-to-end CPU, wall, RSS, device bytes, parity, Unknown rate and compile time (t1 spec-tooling §2.5–2.7).
  - Memory rules: user+sys CPU only; construction separate from execution (`measure-cpu-time`, `anchor-index-mandatory`; t3 §11).
- **Spike measurement rules** (t4 §30 #64–71): oracle parity on node sets; positive controls; never catch "unsupported"; interleaved A/B; decisions require measurement.

---

## Contradictions across tracks (to resolve in the design)

| # | Between | Statement A | Statement B | Proposed reading |
|---|---|---|---|---|
| C1 | spec-indexes vs decision 0006 | per-node positions in SQL serve only the fallback; inside a document the transducer scans the kind column (t1 spec-indexes §2.0.5) | the positional anchor index is always built (t1 OPEN CONFLICT; memory `anchor-index-mandatory`) | postings always built, in the one content store, feeding set-stage LEAF and anchor choice; what is retired is DLRE's private copies → **D1** |
| C2 | spec §21.8 vs Moti Y2 | DRed for recursive components | level witness: 80 vs 388,040 over-deleted (M) (t10 Q3) | **D3** |
| C3 | spec §21 vs Typing / Moti | derived facts compiled to SQL and run in SQLite (t1 §21.1) | recursive and set strata run natively, with SQLite as the system of record and oracle; Typing's native mode beats Ascent; Moti left SQLite over per-statement cost (t9 §3.3, Synthesis 5; t10 Q7) | **D2** |
| C4 | CAWS vs Tempo Metal vs Moti Y3 | < 1 GiB steady RSS (t7 CAWS) | Tempo Metal observed 3.09–3.59 GB (t5 contradictions 1); Y3 1.94–2.09 GB (t10 Q5) | resident tapes fit (≈ 0.1–0.2 GB); the overruns are path arenas and maintained cross-file state → **D5** |
| C5 | sql-vs-transducer ext. 9 vs spec-tooling | non-regular programs routed to CPU | every program Metal-capable; no routing (t2 sql-vs-transducer) | spec-tooling wins; resolved |
| C6 | tape-regularity vs §20 | concatenate the columns of a batch | reference through a document table (t2 tape-regularity) | §20 wins; resolved |
| C7 | metal-recursion vs v9np | resident `parent` | `par` not stored (t2 metal-recursion) | resident sidecar keyed by tape digest; resolved |
| C8 | spec-transducer vs Tempo production | FLAGS generic packing | production masks (t6 S7.2) | fix the spec; resolved |
| C9 | Tempo docs | no-copy shared over mmap (E40–42) | private + staging (grid cutover), with no recorded reason (t5 contradictions 3) | measure Q-R1; §20 defaults to shared no-copy |
| C10 | t5 vs PIAT tempo-residency | Tempo's lesson was "result reuse" | PIAT: "bigger/pinned tape cache" | both are needed (Q3) |
| C11 | t8 | PIAT corpus DB is per process, not across restarts | the content store persists across processes (t8 §contradictions) | the one content store holds the indexes and fact caches (spec-indexes §2.0.1) |
| C12 | t11 | SSC: hard 100-statement gate | DLRE: counts are diagnostic | the spec's gate reads CPU, wall, RSS, parity and Unknown rate; counts are diagnostic (t1 spec-tooling §2.6) |

## Open decisions (the user's to make; recommendations given)

- **D1. Anchor postings.** Recommendation: the reading in C1.
  - per-node kind/value postings (kidx/vidx form: columnar fid + preorder deltas; spike 1.4 + 0.9 B/node) are always built in the content store;
  - main's `search_kind_fts` is re-tokenised to `lang·1024`;
  - they feed set-stage LEAF, M2-style dispatch and anchor choice;
  - the kind-column scan is a per-plan alternative chosen by cost.
  
  Basis: t1 OPEN CONFLICT; t7 schema (main already keeps positions in the one store); t2 metal-recursion; t4 #17, #26.
- **D2. Where derived facts execute.** Recommendation:
  - the derived-fact algebra stays the definition (no SQL in rules);
  - the executor is Typing's native set/lattice engine (arena store, semi-naive waves, declared transfer properties) for recursive and set strata;
  - SQLite holds persisted fact caches, cross-file composition over small outputs, and the oracle.
  
  Basis: C3; t3 §1 (16.4 s table load); t9 §3.3–3.5.
- **D3. Deletion under maintenance.** Recommendation: a level witness (Y2) for recursive components, and read-set memos with early cutoff generalised from main's stamps / Moti Y1 / Typing digests. Basis: C2; Q6.
- **D4. One interner.** Recommendation: a single u32 atom space for token text (`text_vocab`/`tid`), fact atoms and engine symbols, homed in the CAS `strings` table (a new domain) or a sibling append-only table.
  - Basis: t2 regex; t7 tension 3; t9 §3.4.
  - Unresolved: CAS `strings` is machine-wide and collision-ordinal-keyed, while `tid` wants dense, append-only ids. This needs a measured choice.
- **D5. Memory envelope.** Recommendation:
  - keep < 1 GiB steady for the daemon;
  - pinned `snapshot` residency for the active workspace's gate-sized corpus (≈ 0.1–0.2 GB);
  - cross-file maintained state bounded by design (Typing per-document R0 + interface layers, not Y3's whole-program maintained state);
  - audits as residency waves above the cap.
  
  Basis: C4.
- **D6. Cross-file result reuse.** Main excludes cross-file rules from tree reuse. Recommendation: allow reuse when each consumer's read-set digest matches (Q6). Basis: t7 tension 2.

## Open measurements (settle by measurement, not argument; t4 #71)

- Q-R1 shared vs private Metal storage; Q-R7 pin cost per GB; Q-R8 cold admission throughput (t1 §20.12).
- M3: base facts by vtab vs real tables (t3 §1).
- M10: L5 joins within 1.5× of keyed sets (t2 derived-facts).
- The repository-fact cold cost: 1.5–2.5 s estimated (A) vs the 1–2 s audit target. Measure it on Typing's native engine (t2 derived-facts; t9).
- Q2 in spec-indexes: whether positions are needed beyond LEAF and the fallback (t1 spec-indexes §2.2).
- The stack-carried v9np walker for sequential scans (v9 8.8–24 ns/node vs v7 3.8–4.3) (t1 §3).
- Metal lattice waves on 32-bit words: the < 1 ms estimate (A) (t9 §4.6).
