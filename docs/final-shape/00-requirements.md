# Rerail engine: design requirements (locked 2026-10-26)

This is the locked set. Changing a **C** row needs the user. **I** rows may be revised with evidence. **X** rows are to be settled by the listed measurement.

Status marks:

| Mark | Meaning |
|---|---|
| **C** | Confirmed: stated or explicitly agreed by the user |
| **I** | Inferred: adopted by us from the user's direction, a branch's design, or the PIAT spec, but not explicitly confirmed by the user |
| **X** | Exploratory or uncertain: a working choice pending measurement or a decision |

Sources: user statements in the PIAT, Typing-support and wolfeel sessions; memory; PIAT decision records `D000n`. "§" refers to `03-architecture.md`.

| Tag | Session | Date |
|---|---|---|
| P25 / P26 | PIAT | 2026-10-25 / 2026-10-26 |
| T25 / T26 | Typing-support | 2026-10-25 / 2026-10-26 |
| W26 | wolfeel | 2026-10-26 |

## A. Product targets

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| A1 | C | All three targets: fast per-edit checks, a full-repository audit, everything runnable inside an outer SQL query | P25 11:24 | §17 |
| A2 | C | Edits are eventually validated as they are typed; the GPU is a design requirement for that (keep the affordances) | P25 11:24 | §5 |
| A3 | C | Cold full audit: 1–2 s | P25 11:24; W26 19:51 | §10.5 |
| A4 | C | Warm run after a small change: milliseconds | W26 19:51 | §10.5 |
| A5 | C | A pure-SQL fallback so rules can ship before native support; lower priority, SQLite only | P25 11:24, 11:48 | §17 |
| A6 | I | The pure-SQL fallback is the same plan lowered over a tape table-valued function | PIAT spec §13 | §17 |

## B. Model and rule language

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| B1 | C | Datalog is the underlying format, with extensions; generic rules plus special matcher rules | W26 18:32, 21:2x | §0, §12 |
| B2 | C | Rules carry no SQL, code or eval | P25 12:58; D0002 | §0 |
| B3 | C | Not too many incompatible dialects: unify the matcher, transducer, Datalog and PR 2539 forms | T25 23:41 | §12 |
| B4 | C | Be honest about generality: state which Datalog subset runs | T26 01:00 | §12 |
| B5 | C | Named derived facts, defined separately from rules and cached in SQL | P25 19:03; D0005 | §1 |
| B6 | C | Not every single-file condition goes into the transducer; balance simplicity against representability | P25 19:03 | §3 |
| B7 | C | Lower everything possible to the AST; regexes are part of the transducer spec | P25 17:09 | §16 |
| B8 | C | Use trigram indexes for regexes the transducers cannot speed up (to be explored) | P25 17:09 | §16 |
| B9 | C | No tree copies | P25 19:03 | §7 |
| B10 | C | Everything else in the language and typing is resolved as in the Typing-support branch | W26 | §12–§14 |
| B11 | I | Named "Rerail logic: a Datalog fragment"; excluded constructs are named compile errors: unstratified negation, aggregation in recursion except monotone lattices, unbounded lattices, unbounded value-creating builtin cycles, cross-document joins not on an interned key | Typing 01:02 | §12 |
| B12 | I | One AST, two concrete syntaxes: Datalog text for facts and inference, YAML/JSON (matcher style) for lint rules; both embed `ast(...)` | Typing 00:39 | §12 |
| B13 | I | Extensions: `.lattice` (join, height or `widen k`), `count`/`min`/`max`, stratified negation, `$v = @f(...)` builtins, `.demand` magic sets with gated negation, `ast(...)`, tree built-ins (`nearest`, `inside`), a per-document partition, `must_`/`may_`/`*_open` three-valued results | Typing spike | §12 |
| B14 | I | Builtins amend decision 0005's "no UDFs" | Typing 22:30 | §12 |
| B15 | I | Trigram postings are not built unless textual clauses appear or a vocabulary scan nears the latency target; if built, custom u32 postings, never FTS5 | ast-index-spike S159; PIAT regex.md | §16 |

## C. Facts, storage, invalidation

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| C1 | C | A lattice over facts, all stored in SQLite: Git tree hashes, content hashes, paths, timestamps, source, tapes, language, derived facts | W26 18:32 | §1 |
| C2 | C | One generic dependency mechanism: fact → tape → source hash, directory hash → tree hash, … | T25 23:41 | §14 |
| C3 | C | The dependency engine reuses the fact engine; ideally they are the same thing. Work is expressed in SQL and tracked in the database | T25 22:25 | §14 |
| C4 | C | Push path: invalidate a batch of facts and replace them; only changed facts are updated, the rest stay cached | W26 18:32 | §2 |
| C5 | C | A recomputed fact identical to the last one short-circuits further evaluation | W26 18:43 | §2 |
| C6 | C | Coarse invalidation is fine, provided spurious invalidations don't greatly increase work | W26 18:43 | §10.4 |
| C7 | C | Write-back is eventual and off the fast path; stale facts are detectable; if the writer lags, results stay in memory | W26 18:32 | §4 |
| C8 | C | Write-back cannot fall arbitrarily far behind; interim facts need an efficient store | W26 18:43 | §10.2 |
| C9 | C | Invalidation is part of the lattice design | W26 17:01 | §1–§2 |
| C10 | C | Only the facts needed by the running programs are installed | W26 19:51 | §10.5 |
| C11 | C | Caching is key for repository-wide facts | W26 19:51 | §10.5 |
| C12 | C | One interner | W26 18:43 | §10.6 |
| C13 | C | Git tree hashes and their relationships are facts the engine works with | W26 | §15 |
| C14 | I | Derivation tables `derivation(id, producer, rev, key_hash, status, reason, output_hash, cost)` and `derivation_input(id, ord, input_hash, name_id)`; names are mutable bindings to hashes changed only by observers; validity is one SQL join | Typing 23:44 | §14 |
| C15 | I | No DRed and no level witness: an invalidated fact is recomputed from its inputs | ours | §2 |
| C16 | I | Git tree facts: `git_tree`, `git_tree_entry`, `git_blob_alias`, `snapshot`, `overlay`, `clean(prefix)`, `context_chain`; Git events diff root trees to seed the push path; subtree-keyed derivations give a cutoff per directory | Tempo E31–E34; main schemas 9/10 | §15 |
| C17 | X | Cross-file derivations record the tree hashes of the prefixes they read as coarse read-set entries | ours | §15 |
| C18 | X | Interim-fact format: append-only columnar runs of u32 atoms per relation, spilling to a flat file at the cap, merged into SQLite in bulk | ours | §10.2 |
| C19 | I | A counter of re-runs against facts that actually changed measures spurious invalidation | ours | §10.4 |

## D. Lowering and compilation

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| D1 | C | Matcher rules lower into Datalog rules, SQL relations and transducers | W26 18:32 | §3 |
| D2 | C | The transducer is the concrete IR; whatever can run reasonably on Metal or the CPU JIT goes there, and SQL works above it; extend the IR where needed | P25 13:51 | §3 |
| D3 | C | Transducers are combined and optimized, including AND/OR co-execution; fusion gets a detailed spec | W26 18:32; P25 14:43, 16:11 | §3 |
| D4 | C | Lowering is driven by statistics: query shape depends on corpus statistics | P25 11:24; W26 18:32 | §6 |
| D5 | C | Statistics live in SQL, ideally from indexes; Rust-side statistics need a stated reason | P25 11:48 | §6 |
| D6 | C | Planner statistics are built at ingestion from an SQL index, as ast-index-spike does | T26 10:24 | §6 |
| D7 | C | Statistics-based anchoring for walking up the tree | T26 01:56 | §6 |
| D8 | C | Rules and statistics change rarely: compile output is cached, and compile time is recorded separately | P25 11:48 | §6 |
| D9 | C | Interpreted Metal/CPU first, for reactivity; the JIT is built in the background | W26 18:32 | §3 |
| D10 | I | Interpreted and JIT forms are two bindings of one plan with one identity; `compiled_plans` records each decision with the statistics it read and their validity intervals; nothing counts rows at run time | PIAT spec §11; ours | §3, §6 |
| D11 | I | Shared sub-rule evaluation (hash-consed graph) is the primary fusion win | PIAT fusion-model | §3 |
| D12 | I | SSC plan records (`RelationSpec`/`Stage`/`ProgramGroup`/`Plan`) merge into `compiled_plans` | SSC | 04 |

## E. Execution

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| E1 | C | SQL calls the shots; native code is driven through vtabs, not by SQL scheduling granular Rust calls; the optimizer sees the relational shape | P25 11:24; W26 18:32 | §4, §9 |
| E2 | C | No SQL-vs-Rust dichotomy; slow constructs move into native code | P25 11:24 | §9 |
| E3 | C | Native functions anywhere; state may be cached in memory behind a native function | W26 18:32 | §4, §9 |
| E4 | C | Rules run as a batch | P25 11:24 | §3 |
| E5 | C | Never per-file execution, on any executor | W26 16:12 | §3, §7 |
| E6 | C | Transducers run as one batch per wave through a vtab | W26 18:32 | §3 |
| E7 | C | Everything is set-based | W26 18:32 | §7 |
| E8 | C | Recursion runs as waves. Bounded recursion inside one file, guaranteed to finish, runs inside the GPU program, for matchers and generic rules alike (e.g. per-file type inference); waves remain for programs not guaranteed to finish within a file | W26 18:32, 19:51 | §7 |
| E9 | C | Device-side enqueue handles recursion inside one file whose bound is not fixed | W26 | §7 |
| E10 | C | One final layer of full SQL turns facts and matches into findings; per-wave later, not now | W26 18:32 | §8 |
| E11 | C | SQLite indexes prefilter by file, anchor kind and identifier text so only the best anchors are used | W26 18:32 | §6 |
| E12 | C | A full scan overlaps cache preparation with computation, possibly in several chunks | W26 18:43 | §10.1 |
| E13 | C | File filtering may run regexes over the index or vocabulary, guided by ast-index-spike | W26 | §16 |
| E14 | I | A wave is one SQL step plus one `demand(batch)` call over every document in the frontier | ours | §7 |
| E15 | I | Regexes run once per distinct term per language (literal sets by join, patterns in Rust); probes come from the pattern; the vocabulary path never under-produces; the transducer reads `R[tid]`; whole-file text only for genuinely textual clauses | ast-index-spike; PIAT §17 | §16 |
| E16 | I | The findings layer feeds nothing back into the fixpoint | ours | §8 |

## F. GPU (Metal)

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| F1 | C | No Metal → CPU/JIT fallback; Metal runs every program | P25 16:11 | §3 |
| F2 | C | Data-dependent recursion on Metal; device-side enqueue | P25 14:43, 16:11; W26 | §7 |
| F3 | C | Pinned GPU memory caches program source and tapes; wave preparation fills it | W26 18:32 | §5 |
| F4 | C | Pinned GPU memory is dropped two minutes after each full execution | W26 18:32 | §5 |
| F5 | C | Learn from Tempo's cached, pinned, page-allocated uncompressed tapes | P25 17:09 | §5 |
| F6 | C | Prefiltering and memoised facts keep the incremental working set to very few files | W26 18:43 | §10.1 |
| F7 | I | Residency: zstd frames decompress directly into 64 MiB slabs with one retained no-copy shared `MTLBuffer` per slab; batches reference documents and never copy | PIAT spec §20 | §5 |
| F8 | X | Shared vs private Metal storage (Q-R1); pin cost per GB (Q-R7) | PIAT §20.12 | §10.1 |

## G. Tape and data layout

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| G1 | C | One common tape, a minor variant of ast-index-spike's format | P25 11:24 | §17 |
| G2 | C | The tape is completely regular | P25 13:51 | §17 |
| G3 | C | Disk space matters: no layout expansion without clear benefit | P25 14:43; W26 16:22 | §1 |
| G4 | C | The tape holds structure only; Blade stores exactly what tree-sitter-php does | P25 14:43 | §17 |
| G5 | C | ERROR/MISSING spans are stored as tree-sitter emits them; Blade may use a compatible generic form | P25 16:11 | §17 |
| G6 | C | Take ast-index-spike's multiple-trees-per-file-range experiments into account | P25 13:51 | §17 |
| G7 | C | Tapes stay processable in SQL; Rust-friendly changes must show clear benefits | P25 14:43 | §17 |
| G8 | X | Tape v9np (`kf, size, base, off, tail`, no stored `par`), 0.93–1.01× v7, pending 4 measurements before lock | D0003 | §17 |
| G9 | I | Ranges with injected languages are documents with an origin seam | main embedded-program documents | §17 |

## H. Indexes and statistics

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| H1 | C | The anchor index (positional kind and value postings) is mandatory: always built, never optional or switchable | P26 02:28; D0006 | §6 |
| H2 | C | Spec the new indexes exactly, including how they are built | P25 14:43 | §6 |
| H3 | I | Kind postings are re-tokenised `lang·1024 + local` in the one store; term postings are per language and case-exact; plan-time statistics come only from point lookups on `search_kind_vocab`/`search_term_vocab` | PIAT spec-indexes | §6, §16 |
| H4 | X | How a regex's matched-term count feeds the plan-time estimate; the proposal is `plan_feedback` after the first fill | ours | §16 |

## I. Typing

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| I1 | C | Types are collected and rules can refer to them; most mypy/pyright-class rules are supported; generic across languages, Python, JS/TS and Rust first | T25 20:59 | §13 |
| I2 | C | No full type checker; approximate, progressive, per-language inference over the AST, invalidated like any fact; 80/20 | T25 21:23 | §13 |
| I3 | C | Both "provably true" and "not provably false" are supported | T25 21:29 | §13 |
| I4 | C | Users can tell the checker the types; declarations fill gaps only; conflicts are reported | T25 21:29, 21:36 | §13 |
| I5 | C | Parameterized and generic types are first-class | T25 21:29 | §13 |
| I6 | C | Handle installed packages and stubs (orders of magnitude more files) | T25 21:36 | §13 |
| I7 | C | Call-chain analysis and call-target resolution | T26 11:06 | §13 |
| I8 | C | A native lattice with recursion, parallelism, wavefront tracking and set-valued recursion | T26 01:56, 09:51 | §7, §12 |
| I9 | C | Soufflé-style magic sets | T26 15:38 | §12 |
| I10 | I | R0 per file, inside the GPU dispatch; R1..Rk across files over interface summaries, with interface-digest cutoff; bounded rounds, not a global fixpoint | Typing 21:25 | §13 |
| I11 | X | k = 2 cross-file rounds | Typing (open) | §13 |
| I12 | I | Interned type terms: named type with arguments, union with an `open` flag, `⊤`, variable with bounds, callable, tuple, literal, reference; widen at depth about 4; must/may trait bitmasks; `type_term`/`type_arg` | Typing 21:31, 22:30 | §13 |
| I13 | I | Generic instantiation: receiver, then explicit arguments, then one-way matching, then one callback step, then bound fallback; variance declared for libraries, unknown for user types | Typing 21:31 | §13 |
| I14 | I | `proven:` / `possible:` combine with `not` and `some`/`every`; a bare `type:` keeps today's semantics | Typing 21:31, 23:44 | §13 |
| I15 | X | Findings reached only through unknown share the rule's code and carry a tag | Typing (open) | §13 |
| I16 | I | `assert` and type guards narrow for the rest of the block; `cast`/`as` only for the expression | Typing (open; recommended) | §13 |
| I17 | I | `rerail.types/v1` declarations are written in the language's own syntax | Typing 21:31 | §13 |
| I18 | I | Dependencies produce interface summaries only (no findings, no tapes), content-addressed per (package version, module file), machine-wide; read on demand through the magic-set relation; identity from lockfiles and `RECORD`; "pending" ≠ `⊤` | Typing 21:37, 22:30 | §13 |

## J. Output contract

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| J1 | C | Loosest contract: a set of finding keys plus Complete/Unknown per (rule, file); order, ids, metadata, witnesses and messages are not requirements; not a drop-in replacement | P25 12:58; D0001 | §0 |
| J2 | C | Resource exhaustion is reported (Unknown(resource)), not rerouted | P25 16:11 (memory) | §17 |

## K. Resources

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| K1 | C | 500 MB total RAM, 10 GB disk; SQLite on disk so little is resident | W26 17:01 | §5 |
| K2 | C | Memory limits are part of the design | P25 16:11 | §17 |
| K3 | I | Every allocation belongs to a pool sized at plan time; CI gates on peak RSS, device bytes and the Unknown rate | PIAT spec-tooling | §17 |

## L. Tooling

| # | St | Requirement | Source | § |
|---|---|---|---|---|
| L1 | C | Memory limits, debugging, testing tools (e.g. transducer dumps) and profiling are built in | P25 16:11 | §17 |
| L2 | I | Dumps and an assembler, `explain`, replay bundles, differential testing on finding keys across backends, fixed profile stages | PIAT spec-tooling | §17 |

## M. Evidence, measurement and process

All confirmed.

| # | St | Requirement | Source |
|---|---|---|---|
| M1 | C | CPU time (user+sys), never wall; note the load | T26 12:18 |
| M2 | C | Construction costs are excluded from execution measurements and reported separately | P26 02:28 |
| M3 | C | Measure before merging | P25 16:11 |
| M4 | C | Estimates are grounded in the actual data-layer rules and corpus | P25 16:11 |
| M5 | C | Consult ast-index-spike for every decision; never re-decide what is decided | P25 12:58; P26 02:28 |
| M6 | C | Respect PR 2539's lessons | P25 11:24 |
| M7 | C | Judge by shape, not completion; deliver a coherent design | W26 16:12 |
| M8 | C | Deep research with tracked notes | W26 16:22 |
| M9 | C | No test over 5 min; reuse recorded baselines; more than half the time spent editing; fast inner loops | P25 20:54; P26 12:19 |
| M10 | C | **Copy or merge the closest code into place, then mutate it; don't rewrite** | P25 12:11; W26 |
| M11 | C | Commit as you go; never push; no large fixtures in history | P25 23:43; P26 01:03 |
| M12 | C | Other worktrees are read-only from here | T25 21:23; W26 |

## Open measurements (X rows)

| Row | What to measure or decide |
|---|---|
| C17 | Cross-file read sets by tree hash |
| C18 | Interim-fact format |
| F8 | Shared vs private Metal storage; pin cost |
| G8 | v9np lock |
| H4 | Regex count estimate |
| I11 | k |
| I15 | `possible` tag |
| Others | Cold cost of the repository-wide facts (estimated 1.5–2.5 s); facts by vtab (M3); lattice waves on Metal; Tempo subtree-reuse RSS rise (399 → 678 MB) |
