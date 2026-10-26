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

- **Datalog is the underlying format, with extensions (§12).**
- The model has two kinds of rule:
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
- **Recursion inside one file whose bound is not fixed but which is guaranteed to finish** (data-dependent loops, per-file lattice fixpoints) uses **Metal device-side enqueue** inside the dispatch, not waves (user, 2026-10-26).
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
3. **File-local recursion runs inside the GPU program.**
   - A recursive rule that stays inside one file and is guaranteed to terminate resolves within the batch's dispatch. This holds for matchers and for generic rules alike, for example per-file type inference over a finite lattice.
   - Waves (§7) are used for recursion that crosses files, and for any program that cannot be guaranteed to finish inside one file.
4. **Cutoff granularity.**
   - Coarse is acceptable, as long as spurious invalidations do not greatly increase the work. This is a practical question and should be checked with a counter of re-runs against actual changes.
   - A recomputed fact (or partition) that is exactly the same as last time stops further evaluation.
5. **Repository facts and targets.**
   - **Targets.** A cold audit should take 1–2 s. A warm run after a small change should take milliseconds.
   - **Caching.** Caching repository facts is central.
   - **Demand-driven installation.** Only the facts needed by the programs being run are installed (computed and made resident). This keeps memory bounded and cuts the cold cost.
   - **First measurement.** The cold cost of the cross-file call edges: 0.73–1.08 s on its own (M), within a 1.5–2.5 s total estimate (A). It is the first candidate for native vtab operators.
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

---

# Part II: resolved details (user, 2026-10-26)

Rule: Datalog is the underlying format, with extensions. Everything else is resolved as in the Typing-support branch.

That branch's resolutions mostly live in its session transcript. The committed spike is `spikes/datalog-engines/` (`DE/`). Where the branch made a proposal and left it unanswered, the proposal is adopted here and marked *(adopted)*.

## 12. Rerail logic: a Datalog fragment with extensions

**Name and claim.** The language is "Rerail logic: a Datalog fragment". It does not run arbitrary Datalog, and the compiler rejects each excluded construct with a named error (Typing 2026-10-26 01:02). The excluded constructs are:
- unstratified negation;
- aggregation inside recursion, unless it is a monotone lattice;
- unbounded lattices;
- cycles through value-creating builtins with no bound;
- cross-document joins that are not on an interned key.

Programs are also cost-gated.

**Semantics come from Datalog; the runtime is ours** (`DE/FINDINGS.md:208-210`).

**One AST, two concrete syntaxes** that round-trip losslessly *(adopted)*:
- Datalog text for facts and inference;
- YAML/JSON for lint rules, in the matcher style.

Both embed the same matcher pattern as an `ast(...)` atom. Rules carry no SQL.

**Extensions:**

| Extension | Form | Status in the branch |
|---|---|---|
| Lattices | `.lattice cv(n: node; v: flat)`; declared join, finite height or `widen k`, e.g. `.decl type_of(doc, node; t: approx) merge @approx_join widen 8` | `flat` built; general form proposed |
| Aggregates | `$k = count { child($b, _, _) }`, plus `min` and `max` | `count` built |
| Stratified negation | `!kind($c, "block")`, checked (`NotStratifiable`) | built |
| Builtins / native functions | `$v = @f(...)` only, never inside a body atom. `@cat` (invertible), `@flat_bin`; type builtins `parse_type`, `subst`, `match_args`, `narrow`, `intern`, `traits` | partly built; amends decision 0005's "no UDFs" |
| Demand / magic sets | `.demand rel, …` or `.demand *` (Soufflé-style). This is what the "install only needed facts" requirement uses. Gated negations hold `!N(t)` until `done_N(t)` | built (rounds 8–9) |
| Matcher atom | `ast($D, $P, { kind: typed_parameter, not: { inside: {...} } })`, lowered to transducers (§3) | proposed |
| Tree built-ins | `nearest($r, $f, scope)` with `.kindset`; `inside`/`within` as tape arithmetic | `nearest` built |
| Per-document partition | recognised from `docof` views and executed as id-interval scans; marks a rule file-local (§7) | built (recognition) |
| Three-valued results | `must_`/`may_` relations and `*_open` relations for the unknown side | built (`call_open`) |

## 13. Typing

These are the Typing branch's resolutions, adopted.

**Inference without a checker.**
- Inference is approximate and progressive: language-specific inference programs over the AST, aiming at 80/20. Their results are facts, invalidated like any other fact. No full type checker is ever embedded.
- **R0 runs per file.** It is a lattice fixpoint inside the GPU dispatch (§7).
- **R1..Rk run across files** over interface summaries from the previous round, with an early cutoff on interface digests. These are bounded rounds, k = 2 *(adopted)*, not a global fixpoint.

**Type terms.**
- All terms live in one space of interned terms. A term is one of:
  - a named type with arguments;
  - a union with an `open` flag (the union is the approximation);
  - `⊤`;
  - a type variable with bounds;
  - a callable, tuple, literal or reference.
- Widening applies past depth about 4 or past a member limit.
- Traits are must/may bitmasks per term.
- Stored as `type_term(tid, ctor, name_atom, arity)` and `type_arg(tid, i, arg)`, interned in the single interner (§10 answer 6).

**Generics are first-class.**
- Interface summaries stay generic.
- Instantiation is local, in this order: receiver, then explicit type arguments, then one-way matching (no full unification), then a single callback step, then unbound variables fall back to their bound.
- Bounds are must-traits.
- Variance is declared for library types and unknown for user types, so a check gives unknown rather than a wrong false.

**Rule predicates.**
- `proven: X` means X is true; `possible: X` means X is not false. Both combine with `not`.
- `some`/`every` quantify over union members.
- A bare `type:` keeps today's behaviour, where unknown means no finding.
- Findings reached only through unknown share the rule's code and carry a tag *(adopted)*.

**User declarations, `rerail.types/v1`.**
- Signatures are written in the language's own syntax, for code that cannot be edited.
- They **fill gaps only**; a conflict with an in-source annotation is reported (accepted).

**Narrowing.**
- `assert` and type guards narrow for the rest of the block.
- `cast`/`as` narrow only the expression itself *(adopted)*.

**Dependency universe** (installed packages, stubs, typeshed; orders of magnitude more files).
- Dependencies produce **interface summaries only**: no findings and no tapes.
- Summaries are cached content-addressed per (package version, module file), machine-wide.
- They are read on demand from resolved imports, with re-exports chased and a budget per package. The demand is the magic-set relation of the type program: `demand module(M2) :- module(M), reexport(M, _, M2).`
- Identity comes from lockfiles and `RECORD`; nothing is ever executed.
- "Pending" is a state distinct from `⊤`.
- Measured: 3 hops reached 14% of registry bytes; summaries compress to 4.8% of source (M).

## 14. Facts record their inputs: the generic dependency mechanism

This is the Typing branch's design, adopted. It is how §1, §2 and §4 work: every fact family is a derivation, so the dependency engine and the fact engine are the same thing.

```
derivation(id, producer, rev, key_hash, status, reason, output_hash, cost)
derivation_input(id, ord, input_hash, name_id NULL)
```

- The model follows "Build Systems à la Carte": constructive and verifying traces, with early cutoff.
- **Names** (paths, refs, `HEAD`, workspace roots) are mutable bindings to hashes. They change only through observers: the filesystem watcher, Git, the editor.
- **Validity is one SQL join.** A derivation is valid while every name it recorded still binds to the same hash. That join is the staleness check in §4.
- **Cutoff (§2).** A recomputed derivation whose `output_hash` is unchanged stops propagation.
- **Existing mechanisms map onto it:** the store's `tape_objects`, `search_index_sets`, `completed_directory_reuse` and five more. Outputs stay in narrow typed tables.
- **Granularity.** Coarse derivations (per file, per module, per tree) are the default, with the over-invalidation counter from §10 answer 4.

## 15. Git trees and their relationships are facts

Source: Tempo, `nera-landing-2/ner-1562-scale-test-the-re-railer/tempo/docs/workspace-store-runtime-journal.md` entries 31–34, and `content-addressed-workspace-store.md` "Git tree reuse". The same mechanism shipped in main as schemas 9/10 (`notes/t7`).

**Facts:**

| Fact | Meaning |
|---|---|
| `git_tree(tree_hash)` | a tree object |
| `git_tree_entry(tree, name, mode, kind, target_hash)` | a Merkle edge: name, mode, blob or child tree |
| `git_blob_alias(blob_oid → content_hash)` | a blob is a known source |
| `snapshot(commit → root tree)` | an immutable base |
| `overlay(path, …)` | staged, unstaged, untracked or deleted paths from `git status` / the filesystem monitor |
| `clean(prefix)` | no overlay below this prefix |
| `context_chain(prefix, package_hash)` | e.g. Python package markers, which determine module context |

**A tree hash is a Merkle digest of everything below it.** That gives the lattice:
1. **Push path from Git events.** A checkout, pull or branch switch compares root trees and descends only into subtrees whose hash changed. The invalidated batch is exactly the changed blobs: no directory walk, no reads, no hashing of unchanged files.
2. **Coarse cutoff at directory granularity.** Derivations keyed by (tree hash, prefix, context chain, catalog/plan identity) are valid for a whole clean subtree. Results attach as stored pages, with no per-file state, source read, hash, tape restore, decode, parse or match.
3. **Cross-file derivations** can record the tree hashes of the prefixes they read, as coarse read-set entries in `derivation_input`. This extends subtree reuse to cross-file rules, which Tempo and main exclude today ("policies with global checks continue to use per-file state").

**Measured gains (M, Tempo, 2026-09-26):**

| Measurement | Before | After |
|---|---|---|
| Share of 304,937 tracked file occurrences covered by unchanged subtrees at the same prefix | — | 45.36% |
| `nera-data-layer`, 5,725 files: cold vs reuse of the completed root tree | 11.59 s cold | 610 ms (reads no repository files) |
| Same, result pages (schema 10), median | 668.59 ms, 5,725 file states | **34.11 ms**, p95 36.96 ms, no file states |
| Estimated workspace memory | 9.29 MB | 0.91 MB (−90%) |
| 47 active workspaces (341k files): elapsed | 48.72 s | 13.57 s |
| Same: aggregate index time | 101.05 s | 17.29 s |
| Same: p50 | 976 ms | 111 ms |
| Same: p95 | 5.39 s | 1.29 s; 0.65–0.78 s after bulk restoration of the dirty overlay |
| Same: peak RSS (rise not yet explained) | 399 MB | 678 MB |

**Limits, to carry forward:**
- A tree hash describes committed bytes. Dirty paths are overlays, validated by reading and hashing.
- Content-transforming attributes (filters, end-of-line conversion, `working-tree-encoding`) force a read and hash.
- Reuse requires the same logical prefix.
- Submodules are separate snapshot roots.
- Exact untracked-file detection needs a traversal or a trusted filesystem monitor.

## 16. Regex

Guided by ast-index-spike (`notes/t4` §4–5; PIAT `research/regex.md`, `spec-indexes.md`, shape-fixes §17).

**Where regexes run.**
1. **Filtering files and candidates runs the regex over the vocabulary, not file text.**
   - It runs once per distinct term per language, and incrementally over new terms only.
   - The result is term ids, then postings or document words (`search_term_fts`), then candidate files and nodes.
   - Exact literal sets are filled by join or hash (`acfc934d`).
   - General patterns run in native Rust, compiled once (`5e8cbcea`): `vocabulary_fill` went from 6.0 s to 0.95 s (M).
   - Matching is case-exact. Character classes are rewritten as codepoint ranges.
   - There is one vocabulary per language, persisted.
2. **Probes derived from the pattern** prune before the regex: required literals with non-word boundaries, anchored literals turned into `=`/LIKE, first/last byte sets, length bounds. The spike measured 3.5× for these.
3. **The vocabulary path must never under-produce.** Kinds the vocabulary does not cover fall back to scope-limited probes (`b839fa05`).
4. **Inside the transducer**, the same per-term results are read through the `tid` sidecar, as `R[tid]` cached by (pattern-set digest, vocabulary length).
   - Text that is not a token (strings, comments) uses a DFA on the node's bytes, or probes on spans.
   - Whole-file text is used only for genuinely textual clauses. There are 0 of 113 on the data-layer rules (M).
   - Expected cost: 0.1–0.3 s cold against 9–17 s today (D).
5. **Trigram postings are not built** unless a pack has genuinely textual clauses or a vocabulary scan nears the latency target. Even then they would be custom u32 postings (about 0.127 B/source byte), never FTS5 trigram.
6. **Planning.**
   - Term and document counts come from point lookups on `search_term_vocab`/`search_kind_vocab`, never from a vocabulary scan at plan time.
   - Value seeds take precedence over kind.
   - A word filter and an inexact value seed are alternatives; take whichever leaves fewer nodes.
   - **Open:** how a regex's matched-term count enters the estimate without a plan-time scan. The proposal is to record it in `plan_feedback` after the first fill.

`text_vocab`, the fact atoms and the type terms share the single interner (§10 answer 6).

## 17. Carried from the PIAT spec (unchanged)

These are not reopened:

- **Tape** (D0003):
  - v9np holds structure only;
  - Blade stores exactly what tree-sitter-php stores;
  - ERROR/MISSING spans are kept as tree-sitter emits them;
  - several trees per file range (injections) are documents with an origin seam (main's embedded-program documents).
- **Pure-SQL fallback and outer SQL:**
  - the same plan lowers to SQLite over a tape table-valued function;
  - the transducer appears in SQL as the `demand` vtab;
  - lower priority.
- **Resources and tooling** (`spec-tooling.md`):
  - exhaustion yields `Unknown(resource)` and is never rerouted;
  - memory pools are sized at plan time;
  - dumps and an assembler, `explain`, replay bundles, differential testing on finding keys, and fixed profile stages.
