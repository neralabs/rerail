# T14 notes: architecture audit of the tree against 03

Audited tree: `jeremybarnes/final-rerail-shape-facts-crossfile` @ `eea42d24f` (his tip `dd7964bdc` merged with T12's port), 2026-11-02. Judged against `03-architecture.md`, `00-requirements.md`, `05-implementation-plan.md` and decisions 0008–0026, not against facts-engine's design. Read by three read-only passes (front end, execution, state) and spot-checked by the decider; paths are relative to `native/src/code_rule_dsl/` unless they start with `native/` or `docs/`. Line counts are estimates from file sizes [INFERENCE] unless cited. Nothing here was benchmarked; T13 has the cost side.

## Verdict

The slice (`CodeRuleSlice`, `slice/`) is a real, working vertical path for file-local Python matcher clauses: Git/directory push → ingest derivations with cutoff → statistics-chosen prefilter → lowered packed programs on CPU JIT and Metal → a SQL-scheduled wave reading the `demand` vtab → a findings statement. Its decisions are recorded and its numbers measured (S1–S8).

Around it, three of 03's load-bearing claims are not yet true in the code, and each blocks the representability work more than any performance item does:

1. **Rules do not go through the one AST.** The slice compiles policies with W's `compile/*` into `AstMatcher`/`GrammarMatcher` (`slice/napi.rs:45-52` → `slice.rs:75-92`). `logic::front::yaml` carries each matcher as opaque text for W's loader (`logic/front/yaml.rs:10-12`), and `logic::plan` refuses the `ast(...)` atom (`logic/plan.rs:645`: "no executor for … yet"). `logic` runs on the slice only as two fixed Datalog programs (the match projection `waves/matches.rs:39-57` and `FINDINGS`). 03 §0 "matcher rules lower into generic rules plus transducers" and §12 "one AST" have no rule-level implementation.
2. **The findings layer depends on W's matcher.** `waves/findings.rs` imports W's `GrammarMatcher`, `begin_clause`, `binding_range`, `node_at`, gates and suppressions (lines 41-56), re-parses each document (`AstGrep::doc`, :499) and re-runs `grammar.match_rule` per matched node (:294) to recover metavariable spans, because lowered programs do not carry captures. W's matcher cannot be retired even at 100% lowering, and cross-file rules (no per-document loop) have no place in this layer as written (0013: "what SQL cannot compute is written as facts first" is not yet done).
3. **Invalidation is four mechanisms, not one (§2, §14).** Derivations + `propagate` (decide only the ingest families); the slice's kept tables `slice_match/_span/_document/_rules` plus a core-digest comparison (`slice.rs:158-166`, which never consults `derivation_valid`); the content memo as a blob store (`slice/memo.rs`); and W's `calls::stamp`, `module_graph::Stamp`, `completed_*`. The wave's family derivations are recorded and then read by nothing.

## Conformance with 03, by section

| § | Claim | State | Evidence |
|---|---|---|---|
| §0, §12 | One Datalog AST; matcher rules lower into it; rules carry no SQL | **Not built** (AST); holds in production for "no SQL", violated in test-only D/P relational clauses | above; `compile/mod.rs:541-583` (`cfg(test)` relational arm); `waves/relational/batch/tests.rs:15` |
| §1 | Facts in SQLite as the system of record | Partly: core facts are digests only (0008, 0026), dead-code rows stay in memory (`facts/deadcode.rs:23-27`) | |
| §2 | Push path, early cutoff, no level order, no DRed / no level witness | Cutoff holds for ingest (`facts/propagate.rs:42-52`); O38 double re-runs; T12's `maintained` mode is a level witness (O37) | |
| §2 step 4, §4 | Write-back in the background; overlay ∪ SQLite; staleness detectable | Partial: real writer thread with back-pressure (`facts/overlay.rs:259`, :298-300), but `end_push` flushes (:523-526), tapes/core facts and wave rows are written synchronously, kept `slice_match` rows carry no input versions; O4–O6 open | |
| §3 | Combine and optimize across rules (hash-consed graph, 1.4–3.1×) | **Not built**: `LoweredWave::new` concatenates programs (`transducer/packed/compile/matcher.rs:484-497`) | |
| §3 | Every op on CPU and Metal; no CPU fallback | Violated in practice: roots on `CpuJit` under Metal (`waves/matches.rs` ~206), host-walked non-device roots (`demand/vtab.rs:177-178`), sweep fallback (`compile/matcher.rs:1567-1571`), batch split on capacity; parity enforced only by an opcode-set test behind `metal-tests` | O28 open |
| §3, §4 | All transducers of a wave run as one batch through `demand(batch)`; SQL controls execution | Partial: `demand/vtab.rs` is a real vtab and the wave reads it, but candidates, label passes and the batch are built in Rust before SQL runs (`waves/matches.rs:155-235`); 0024's `candidates` vtab not built | |
| §5 | Pinned GPU cache, slabs, drop after 2 min; pools sized at plan time in 500 MB | **Not built** (Phase 9): `residency/metal.rs` is a one-line re-export; budgets are unrelated constants (working set 96 MiB, overlay 64 MiB, read cache 32 MiB) | |
| §6 | Stat-chosen anchors; nothing counts rows at run time | Holds at plan time (`stats::inputs`, `compiled_plans`, `plan_feedback`); exceptions: `waves/matches.rs:275` `count(*)`, `count(DISTINCT)` per wave; `slice::remember` re-inserts all rows | |
| §7 | Recursion through facts is waves; file-local recursion in-dispatch | Wave scheduler exists (`waves/schedule.rs`), slice runs `max_waves = 1`; matcher-level recursive labels run as device rounds; generic per-file rules/lattices not built | |
| §8 | One findings layer of SQL after the fixpoint | Structurally yes; substantively Rust writing 17 temp tables per run with DDL in the hot path (`waves/findings.rs:440-640`) | |
| §10.6 | One interner, u32 atoms | Holds in the store (`content_store/atoms.rs`); violated beside it: `stats/inputs.rs:66` joins terms by string, transducer capture labels are byte strings, and separate interners in `typing/rounds.rs`, `calls/names.rs`, `facts/ygraph/intern.rs`, `facts/y2/facts.rs` (thread-local), `facts_a` caches | |
| §13 | Typing | **Not built**: a spike with its own interner (`typing/rounds.rs`) | |
| §15 | Git trees as facts; no directory walk, reads or hashing of unchanged files | Holds for `git_root` (`content_store/tree_diff.rs`); the default napi observer is `"directory"`, which reads and hashes every file on every apply (`facts/observe/roots.rs:61-105`) | |
| §16 | Regex through term vocabulary and `tid` | Partial: vocabulary and probes in `prefilter`; in the transducer `Regex` runs on node text (`packed/compile/matcher.rs:1835-1841`) | |

## Duplication and dormant code

Every copy below is named by an open item (O17–O21); none has an owner or a removal date (O19: "per pair, decide which survives").

| Concept | Live (slice) | Also in the tree |
|---|---|---|
| Rule representation | W's `AstMatcher` via `compile/*` | `logic` AST (two fixed programs only) |
| Matcher lowering | `match_tree` → `LoweredMatcher` | W tree walk; `AnchorPlan::compile` (W/P/D); N's `relational_ir` (test, 05 "not copied"); P/D `waves::relational::compile`; `prefilter/lower.rs` SQL form; `packed::compile::scm` |
| Anchor engines | W `matcher::anchors` + `prefilter` | `p_anchors`, `d_anchors` (`cfg(test)`, ~13K lines each [INFERENCE]; 05:282 lists them as not copied) |
| Executors | CPU interpreter, CPU JIT, Metal interpreter/JIT, micro walks | host sweeps (env fallback); G's grid executor and its Metal stack (~100 files, tests only) |
| Match vtabs | `demand/vtab.rs` | N's `demand/driver/catalog/sql_backend.rs` (~3.3K lines), G's `packed/sql_backend.rs` (tests) |
| Relational executors | — | D's `waves/relational`, P's `residency/admission/relational` (tests) |
| Fact representations | `core::Facts`, vtab relations, overlay runs | `facts_a` (dead), `d_projection`/`p_table` (tests), and T12's `y2`/`ygraph`/`y3`/`lattice` state |
| Layering | `overlay` | `lmap`, `ygraph::world`, `lattice::maint::MutLayer`, `y3::sym` |
| Page stores | — | `tree_pages` (tests via `facts::pages`), `tree_result_segments` (tests) |
| Tape formats | v9np format 10 (`tape.rs`) | `tape/v9np.rs` spike (dead) |

Blanket `allow(dead_code)` on `transducer/packed.rs:1`, `transducer.rs:15`, `demand.rs:5-8` and `logic.rs` means the compiler cannot say which of the ~210 `packed/` files the slice reaches. By file count the dormant G/N/P/D copies are about as large as the slice path [INFERENCE]. The Python references `logic/front/rdl.py` (124 KB) and `stats/ingest.py` are in the tree.

**W and the slice are two engines over one store file.** W touches none of the new machinery (no `intern_atoms`, `bind_names`, `record_derivations`, `facts_memo`, `tree_pages`, working set in `workspace/`). 05 has no cutover plan for W.

## Seams

- **Sound:** `facts::Graph` (two impls); `Families::push`/`recompute`; the `Document` contract with `fresh`; `prefilter`/`stats` consuming `AnchorPlan`.
- **Leaky:** `MatchTree` embeds ast-grep `Pattern` and `regex::Regex`, and the packed compiler calls `Program::compile_ast_grep_pattern` (`packed/compile/matcher.rs:1774`), so the transducer compiler depends on W's matcher types. Anchors and lowered programs are derived independently from one `AstMatcher` (`slice.rs:93`); only a test checks that anchors are a superset (`waves/matches/tests.rs:222`) — an operator updated in one and not the other drops candidates silently.
- **Stringly typed:** the wave writes the vtab name and column order as SQL text (`Wave::emit`); the scheduler finds `:batch` by `contains` on the SQL; `PackedDemand::new` takes positional parallel vectors.
- **Cycles:** `stats` ↔ `prefilter`; `waves::matches` ↔ `demand::vtab`; `transducer` ↔ `prefilter`.
- **Environment-variable control flow** selects executors (`RERAIL_ROOTS_JIT`, `RERAIL_ROOTS_CHECK`, `RERAIL_HOST_SWEEPS`, `RERAIL_S1_*`).
- **Test-only fields in production types:** `ClauseKind::Relational`, `CompiledClause::data_layer` exist only under `cfg(test)` (`model.rs:686,713`).
- **T12's port:** `deadcode::Pending` hands deltas between families through producer memory, not recorded inputs (O39); `#path:` digests are blake3 in `deadcode` and Git SHA-1 in `families::blob_source_id`, so the two cannot share one observer.

## Platform

`facts/ygraph/mod.rs` (his copy at `dd7964bdc`) calls `libc::proc_pid_rusage`, which libc 0.2.191 defines only for Apple targets (`src/unix/bsd/apple/mod.rs`), and `ygraph` was compiled unconditionally; T12 added more of the same (`lattice/cols.rs`) plus `fork`/`mmap`. So `rerail-engine` could not compile for Linux or Windows on either tip, while CI runs on `ubuntu-24.04` [INFERENCE for the build: cross-checking from macOS stops at the C build scripts of `tree-sitter` and `esaxx-rs`]. Fixed on this branch by compiling `ygraph`, `lattice`, `y2`, `y3` and `deadcode` on macOS only (nothing outside them refers to them); macOS build and `facts::` tests unchanged (94 passed).

## Risks for the representability work, ranked

1. **No rule-level path through `logic`.** Every refused clause (199 Python), the data-layer rules and every cross-file rule need a decision first: is the AST `logic` (03 §0, §12) or W's `AstMatcher` (what the slice runs)? Until then each lowering lands in `MatchTree`, which 03 does not describe.
2. **Captures are not carried by lowered programs**, so the findings layer re-runs W's matcher; shared bindings (18 clauses) and semantic leaves (`resolves` 94, `parse`, `arguments`, `binds`; O24) also live only in W's matcher (`matcher/resolve.rs`). The clauses still to lower are exactly the ones that depend on W.
3. **Four invalidation mechanisms.** A cross-file or data-layer family has to pick one. 03 §14's uniformity holds for ingest only.
4. **Cross-file rules have no representable form yet.** The only one in the tree (T12's dead code) is Rust over imported facts-engine exports with a level witness and in-memory state (O36–O40). It proves the derivation seam and the costs, not the language.
5. **Copies without removal dates** keep compile times and change cost high and hide reachability behind `allow(dead_code)`.
6. **Per-run work grows with scope, not edit** (directory observer, `matches::documents`, `remember`, the findings layer): T13 measured it; it is implementation-level, but cross-file rules enlarge the scope.

## Decisions to raise (none blocks the representability work; 1–3 are prerequisites for it)

1. The AST: make `logic` the rule representation and give the `ast(...)` atom an executor that lowers through `match_tree`, or record that `AstMatcher` is the AST and amend 03 §0/§12.
2. Captures in programs: lowered programs emit capture rows, so the findings layer stops re-entering W's matcher; W's gates and suppressions become facts (0013).
3. One invalidation mechanism: wave rows and findings go through derivations and `propagate` (the kept tables become their storage), and W's stamps are listed for migration with the cutover.
4. A dated removal plan for O17–O21 copies and the blanket `allow(dead_code)`s.
5. A cutover plan for W (what retires when).
6. For T12: express dead code in Rerail logic over this crate's facts; keep the ported Rust as the producer and oracle until then.
