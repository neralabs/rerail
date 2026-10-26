# Rerail engine: design requirements

Status: the consolidated set as of 2026-10-26. Sources:
- the user's statements in the PIAT, Typing-support and wolfeel sessions (2026-10-25 and 2026-10-26), quoted or closely paraphrased;
- the persistent memory notes;
- PIAT's accepted decisions.

Each requirement gives its source and where `03-architecture.md` covers it. **Gap** means 03 does not cover it yet.

Source tags:

| Tag | Session | Date |
|---|---|---|
| P25 | PIAT | 2026-10-25 |
| P26 | PIAT | 2026-10-26 |
| T25 | Typing-support | 2026-10-25 |
| T26 | Typing-support | 2026-10-26 |
| W26 | wolfeel | 2026-10-26 |
| Dnnnn | PIAT decision record | — |

## A. Product targets

| # | Requirement | Source | 03 |
|---|---|---|---|
| A1 | All three targets are required: fast per-edit checks, a full-repository audit, and everything runnable inside an outer SQL query | P25 11:24 | §10 answer 5 (partly) |
| A2 | Per-edit checks should eventually validate edits as they are typed. That needs the GPU, which is a design requirement: keep the affordances | P25 11:24 | §5 |
| A3 | Full audit, **cold: 1–2 s** | P25 11:24; W26 19:51 | §10 answer 5 |
| A4 | **Warm with a small change: milliseconds** | W26 19:51 | §10 answer 5 |
| A5 | Pure-SQL fallback, so rules can ship before the engine has native support. Lower priority; SQLite only | P25 11:24, 11:48 | **gap** |

## B. Model and rule language

| # | Requirement | Source | 03 |
|---|---|---|---|
| B1 | A Datalog-like model with generic rules and special matcher rules | W26 18:32 (0) | §0 |
| B2 | Rules carry no SQL, code or eval ("an eval statement should never be needed") | P25 12:58; D0002 | §0 |
| B3 | Not too many incompatible dialects. Unify the matcher, transducer, Datalog and PR 2539 forms | T25 23:41 | **gap** |
| B4 | Be honest about generality: which Datalog subset runs, so the system doesn't look more generic than it is | T26 01:00 | **gap** |
| B5 | Shared intermediates and relational analyses are **named derived facts**, defined separately from rules and cached in SQL (PR 2539's design) | P25 19:03; D0005 | §1 |
| B6 | Don't force every single-file condition into the transducer. Balance transducer simplicity against representability | P25 19:03 | §3 |
| B7 | "Lower EVERYTHING POSSIBLE to the AST." Regexes are part of the transducer spec; explain any regex over file text | P25 17:09 | **gap** (regex) |
| B8 | Use trigram indexes for regexes the transducers can't speed up | P25 17:09 | **gap** |
| B9 | No tree copies ("why do we need to copy trees at all?") | P25 19:03 | §7 (tape arithmetic) |

## C. Facts, storage, invalidation

| # | Requirement | Source | 03 |
|---|---|---|---|
| C1 | A lattice over facts, all stored in SQLite. Git tree hashes, content hashes, paths, timestamps, source, tapes, language and derived facts are all facts | W26 18:32 (1) | §1 |
| C2 | One generic dependency mechanism covering every kind: fact → tape, tape → source hash, directory hash → tree hash, … | T25 23:41 | §1–2 |
| C3 | The dependency engine reuses the fact engine; ideally they are the same thing. The work is expressed through SQL and tracked in the database | T25 22:25 | §1–2 |
| C4 | Push path: invalidate a batch of facts and replace them. Only changed facts are updated; the rest stay cached | W26 18:32 (2) | §2 |
| C5 | A recomputed fact that is exactly the same as last time short-circuits further evaluation | W26 18:43 (4) | §2, §10 answer 4 |
| C6 | Coarse invalidation is fine, as long as spurious invalidations don't greatly increase the work (a practical question) | W26 18:43 (4) | §10 answer 4 |
| C7 | Write-back to SQLite is eventual and off the fast path. Stale facts must be detectable. If the writer lags, results stay in memory | W26 18:32 (4) | §4 |
| C8 | Write-back can't fall arbitrarily far behind; interim facts need an efficient store | W26 18:43 (2) | §10 answer 2 |
| C9 | Invalidation is part of the lattice design, not a separate mechanism | W26 17:01 | §1–2 |
| C10 | Install only the facts needed by the programs being run, to keep memory bounded | W26 19:51 | §10 answer 5 |
| C11 | Caching is key for repository-wide facts | W26 19:51 | §10 answer 5 |
| C12 | One interner | W26 18:43 (6) | §10 answer 6 |

## D. Lowering and compilation

| # | Requirement | Source | 03 |
|---|---|---|---|
| D1 | Matcher rules lower into Datalog rules, SQL relations and transducers | W26 18:32 (3) | §3 |
| D2 | The transducer is the concrete IR driver. Anything that can go into a transducer and run reasonably on Metal or the CPU JIT should; SQL works at the level above. AstMatcher fragments are one input to it; extend the IR where needed | P25 13:51 | §3 |
| D3 | Transducers are combined and optimized, including simple co-execution (AND/OR) as one combined transducer. Fusion gets a detailed spec, with its limits and any extra operations | W26 18:32 (3); P25 14:43, 16:11 | §3 |
| D4 | Lowering is driven by statistics: the right query shape depends on corpus statistics (different lowering, conditional CTEs) | P25 11:24; W26 18:32 (6) | §6 |
| D5 | Statistics live in SQL, ideally derived from indexes. Statistics kept in Rust need a stated reason | P25 11:48 | §6 |
| D6 | Planner statistics are computed from the actual corpus at ingestion time, from an SQL index, the way ast-index-spike does it | T26 10:24 | §6 |
| D7 | Walking up the tree uses statistics-based anchoring (fixed in ast-index-spike; don't repeat the slow version) | T26 01:56 | §6 |
| D8 | Rules and statistics change very rarely. Compile output is cached, and compile time is recorded separately | P25 11:48 | §6 (`compiled_plans`) |
| D9 | Interpreted Metal/CPU runs first, for quick reactivity; Metal/CPU JIT is built in the background | W26 18:32 (3) | §3 |

## E. Execution

| # | Requirement | Source | 03 |
|---|---|---|---|
| E1 | SQL calls the shots. Native code inside a file is driven from SQL through virtual tables, not by SQL scheduling granular Rust calls; the SQL optimizer must see the relational shape | P25 11:24; W26 18:32 (4, 9) | §4, §9 |
| E2 | No SQL-vs-Rust dichotomy: slow SQL constructs are pushed down into native code | P25 11:24 | §9 |
| E3 | Native functions are allowed anywhere; state may be cached in memory behind a native function | W26 18:32 (4, 9) | §4, §9 |
| E4 | Rules run as a batch to avoid repeated work | P25 11:24 | §3 |
| E5 | **Never per-file execution**, on any executor. A per-edit check is a small batch of the same plan | W26 16:12 | §3, §7 |
| E6 | Transducers run as one batch per wave through a virtual table | W26 18:32 (3) | §3 |
| E7 | Everything is set-based | W26 18:32 (7) | §7 |
| E8 | Recursion is multiple waves. Bounded recursion inside one file that is guaranteed to finish runs inside the GPU programs, for matchers and generic rules alike (e.g. per-file type inference). Waves remain available for programs not guaranteed to finish within a file | W26 18:32 (7); 19:51 (3) | §7, §10 answer 3 |
| E9 | One final layer of full SQL turns facts and matches into findings and may do non-Datalog computation. Later it may run after each wave, but not now | W26 18:32 (8) | §8 |
| E10 | SQLite indexes pre-filter by file, anchor kind and identifier text, so only the best anchors are used and the batch does as little work as possible | W26 18:32 (6) | §6 |
| E11 | A full scan overlaps cache preparation with computation and may run in several chunks | W26 18:43 (1) | §10 answer 1 |

## F. GPU (Metal)

| # | Requirement | Source | 03 |
|---|---|---|---|
| F1 | No fallback from Metal to CPU/JIT ("an invitation to never finish the Metal support"). Metal runs every program | P25 16:11 | §3 |
| F2 | Data-dependent recursion has a solution in Metal; device-side enqueue is considered for unbounded programs | P25 14:43, 16:11 | §7 (in-file); **reconcile** with E8 waves |
| F3 | GPU memory pins cached program source and tapes; wave preparation makes sure the cache holds what is needed | W26 18:32 (5) | §5 |
| F4 | Pinned GPU memory is dropped two minutes after each full execution | W26 18:32 (5) | §5 |
| F5 | Learn from Tempo's cached, pinned, page-allocated uncompressed tapes and source | P25 17:09 | §5 |

## G. Tape and data layout

| # | Requirement | Source | 03 |
|---|---|---|---|
| G1 | One common tape, with minor changes from ast-index-spike's format | P25 11:24 | §1 (v9np, D0003) |
| G2 | The tape must be completely regular | P25 13:51 | §1 (v9np) |
| G3 | Disk space matters: no layout expansion without clear benefit (8 → 11 B/node, +30%, was rejected) | P25 14:43; W26 16:22 | §1 |
| G4 | Don't overload the tape with things merely associated with source structure. Blade stores nothing more and nothing less than the tree-sitter-php it replaced | P25 14:43 | **gap** (D0003 only) |
| G5 | Store ERROR/MISSING spans as tree-sitter emits them; Blade may store a compatible generic form | P25 16:11 | **gap** |
| G6 | Account for ast-index-spike's experiments with multiple trees for different ranges in one file | P25 13:51 | **gap** |
| G7 | Tapes stay processable in SQL (ast-index-spike techniques). Rust-friendly format changes must show clear benefits | P25 14:43 | §1 (partly) |

## H. Indexes

| # | Requirement | Source | 03 |
|---|---|---|---|
| H1 | The anchor index (positional kind postings, value postings) is **mandatory**: always built, never optional or switchable | P26 02:28; D0006 | §6 |
| H2 | Spec the new indexes exactly, including how they are built | P25 14:43 | §6 (spec-indexes) |

## I. Typing

| # | Requirement | Source | 03 |
|---|---|---|---|
| I1 | Collect types and let rules refer to them. Support most mypy/pyright-class rules. Generic across languages; Python, JS/TS and Rust first | T25 20:59 | **gap** |
| I2 | No full type checker: approximate, progressive, language-specific inference over the AST, invalidated like any other fact. Aim for 80/20 | T25 21:23 | **gap** (E8 covers execution) |
| I3 | Rules can express both "provably true" and "not provably false" | T25 21:29 | **gap** |
| I4 | There are ways to tell the checker the types, so a rule can be made to pass. Declarations fill gaps only; conflicts with in-source annotations are reported | T25 21:29, 21:36 | **gap** |
| I5 | Parameterized and generic types are first-class | T25 21:29 | **gap** |
| I6 | Installed packages and stub files mean a much larger file universe (orders of magnitude). Say how that works | T25 21:36 | **gap** |
| I7 | Call-chain analysis and call-target resolution | T26 11:06 | §10 answer 5 (repository facts) |
| I8 | A native lattice, including recursion, parallelism, wavefront tracking and set-valued recursion | T26 01:56, 09:51 | §7 |
| I9 | Use magic sets (Soufflé-style) for demand | T26 14:32, 15:38 | §10 answer 5 (C10) |

## J. Output contract

| # | Requirement | Source | 03 |
|---|---|---|---|
| J1 | The loosest contract: a set of finding keys (rule, code, path, line [, declared column or tag]) plus Complete/Unknown per (rule, file). Order, indices, metadata, witnesses and messages are not requirements; the integration normalises and consolidates. Not a drop-in replacement for a too-strict interface | P25 12:58; D0001 | §0 |
| J2 | Resource exhaustion is reported as Unknown(resource), never rerouted | P25 16:11 | **gap** |

## K. Resources

| # | Requirement | Source | 03 |
|---|---|---|---|
| K1 | **500 MB total RAM, 10 GB disk.** SQLite on disk means little has to stay resident | W26 17:01 | header, §5 |
| K2 | Memory limits are part of the design | P25 16:11 | §10 answers 1–2 (partly) |

## L. Tooling

| # | Requirement | Source | 03 |
|---|---|---|---|
| L1 | Memory limits, debugging, testing tools (e.g. transducer dumps) and profiling are built into the design | P25 16:11 | **gap** (spec-tooling) |

## M. Evidence, measurement and process

| # | Requirement | Source |
|---|---|---|
| M1 | Measure CPU time (user+sys), never wall time; note the machine load | T26 12:18 |
| M2 | Remove construction costs from execution measurements; report them separately | P26 02:28 |
| M3 | Do measurements before merges | P25 16:11 |
| M4 | Ground estimates in the actual data-layer rules and corpus: fusion opportunity, a performance model, and top-to-bottom profiles of the heaviest rules | P25 16:11 |
| M5 | Consult ast-index-spike's design and journal indexes for every design decision. Don't re-decide what is already decided | P25 12:58; P26 02:28 |
| M6 | Respect PR 2539's lessons; it was the closest to a viable runtime | P25 11:24 |
| M7 | Judge by architectural shape and potential, not by how complete a stream is. Deliver a coherent design, not abstract ideas | W26 16:12 |
| M8 | Do deep research with written, tracked notes; don't fake depth | W26 16:22 |
| M9 | No test over 5 minutes, no re-running recorded baselines, more than half the time spent editing, a fast inner loop (e.g. the anchor lab) | P25 20:54; P26 12:19 |
| M10 | Copy or merge code into place from the subtrees and adapt it; don't rewrite | P25 12:11 |
| M11 | Commit as you go; never push. Keep large fixtures out of history | P25 23:43; P26 01:03 |
| M12 | Typing work stays in the Typing-support worktree; other worktrees are read-only from here | T25 21:23 |

## Gaps in 03 to close

1. **The outer-SQL target and the pure-SQL fallback** (A1, A5): how a rule runs inside a user's SQL query, and how the same plan lowers to SQLite only.
2. **Rule-language unification** (B3) and **the stated Datalog subset** (B4).
3. **Regex** (B7, B8): regexes lowered to transducers, `tid` term matching, and trigram postings for the rest.
4. **Tape content** (G4–G6): structure only; error spans as tree-sitter emits them; multiple trees per file range (injections).
5. **Typing** (I1–I6): a section on type facts, three-valued rule predicates, user type declarations, generics, and the dependency universe (stubs and installed packages as content-addressed facts installed on demand).
6. **Unbounded in-file recursion** (F2 vs E8): device-side enqueue inside a dispatch for data-dependent loops that terminate, and waves only for programs that cannot be bounded to one file.
7. **Unknown(resource) and tooling** (J2, L1): carry PIAT `spec-tooling.md` forward.
