# Implementation plan: copy, then mutate

Status: plan, 2026-10-26.

- Target: wolfeel, `jeremybarnes/final-rerail-shape`, based on `origin/main` 5cf4246e.
- Requirements: `00-requirements.md` (locked).
- Architecture: `03-architecture.md`.
- Sources: the code inventories of 2026-10-26.

## Rules for every copy (M10, M11)

1. **Copy first, verbatim, in its own commit.**
   - Message: `COPY: <worktree>@<sha>:<path> -> <dst>`.
   - The copied files go in, unchanged, at their destination.
   - They stay outside the module tree (no `mod` line) until the next commit, so the build stays green.
2. **Mutate in later commits**, one adaptation per commit. Each commit is checked with `cargo check` plus the copied unit's own tests, kept under 5 min (M9).
3. **Provenance.** Each copy gets a row in `docs/final-shape/provenance.md`: source, SHA, destination, adaptations applied.
4. **Diverged files** that main also changed (`tape.rs`, `transducer.rs`, `matcher/*`, `compile/*`, `workspace/*`, `content_store/*`, `Cargo.toml`) are never copied wholesale. Their branch deltas are ported onto main's version. New trees (`packed/`, `relational/`, `query/plan/`, `sql_backend.rs`, `anchors/sql/`) are copied whole.
5. **Python sources** (Typing `rdl.py`, `stats.py`) have no Rust to copy. They are **ported function by function**, with each Rust function named after its Python one, and diffed against the Python on the same goldens. This is the closest thing to a copy available, and it is flagged as such.
6. **Where no source exists**, the plan says "write new", with the nearest reference.

## Pinned sources

| Key | Worktree / ref | SHA |
|---|---|---|
| N | `rerail/ner-1562-scale-test-the-re-railer` | 17991359 |
| D | `rerail/Data-layer-rule-expressivity` | fe4a6707 |
| P | `rerail/Pulling-it-all-together` | cbee7a03 |
| S | `rerail/Shared-structural-execution` | e1ffeaf2 |
| T | `rerail/Typing-support` (`spikes/datalog-engines/`) | 7ebcffe0 |
| G | `nera-landing-2/ner-1562-scale-test-the-re-railer/tempo` (Tempo) | 4c94cf77e (committed HEAD only) |
| M | `origin/Moti-Velz/facts-engine` | 5b0d63a7 |
| W | wolfeel itself (main) | 5cf4246e |

`cr` = `native/src/code_rule_dsl`.

## Target layout (new modules under `cr/`)

| Module | Covers (03) | Base |
|---|---|---|
| `facts/` | fact families, derivations, overlay, writer, propagation (§1, §2, §4, §14) | M Y1 + `lmap`, M `facts_memo`, D `projection`, W `content_store` |
| `facts/core.rs` | core node facts from the tape | M facts-a |
| `prefilter/` | anchor postings, vocabulary, regex, probes (§6, §16) | P `matcher/anchors/sql` |
| `stats/` | ingestion statistics, `compiled_plans`, `plan_feedback` (§6) | T `stats.py`, D `statistics/`, N `search_kind_vocab` |
| `transducer/packed/` | IR, verifier, CPU and Metal interpreter and JIT (§3, §5, §7) | N `packed/`, plus G deltas |
| `demand/` | the `demand(batch)` vtab and output relations (§3) | N `transducer/sql_backend.rs` |
| `logic/` | Datalog-fragment front end, magic sets, planner, lowering, native fixpoint (§12) | T `rdl.py` (port) + `sqlite-ext` (copy) |
| `waves/` | SQL-driven wave control, SQL owner, schema lifecycle (§4, §7) | D `relational/` |
| `findings/` | facts by vtab, findings layer, coverage (§8) | S `query/plan/*`, `contract.rs`; N output rows |
| `residency/` | slabs, pinned Metal cache, 2-min drop (§5) | G `metal_backend.rs` `MappedRegion` + new |
| `typing/` | type terms, R0/R1..Rk programs, dependency summaries (§13) | T `.rdl` programs, `lattice/` |

## Phases

The order follows the dependencies:
- 0 → 1 → 2 → 3 is the store, facts and prefilter.
- 4 → 5 is the executor.
- 6 is the language.
- 7 → 8 is control and output.
- 9 is the GPU cache.
- 10 is typing.
- 11 runs throughout.

Each phase ends with a short targeted check, not a corpus run.

### Phase 0: build prerequisites

- **Port** the dependency deltas into W `native/Cargo.toml`:
  - rusqlite features `vtab`, `functions`, `hooks`, `modern_sqlite`, `series`, `column_decltype` (from N and D);
  - cranelift 0.125.4 (`codegen`/`frontend`/`jit`/`module`), `regex-automata` 0.4.18, `regex-syntax`, `metal` 0.33 non-optional on macOS, `block` (vendored `block-0.1.6` from N), `hashbrown`, `rustc-hash`, `mimalloc`.
- **Adapt.** Drop the `bench-backends` gate on Metal. Keep W's `bundled` and `limits`.
- **Interner (C12).** Decide where the single u32 atom table lives (§10.6) before Phase 1 writes atoms. Write new: one `atoms` table in the content store, append-only and dense, shared by `text_vocab`/`tid`, fact atoms, paths and type terms. The CAS `strings` table maps into it.

**Status (2026-10-27): done except `mimalloc`: W already runs a tuned jemalloc as its global allocator, so the swap is a decision (O16). `block` added with Phase 4 row 3 (`bb4c9605`).**

### Phase 1: fact store, derivations, overlay, push path (C1–C19)

| Copy | From | To | Adaptations |
|---|---|---|---|
| Content-hash memo | M `native/src/content_store/facts_memo.rs` (+ `facts_memo/tests.rs`) | `content_store/facts_memo.rs` | Generalise the key (content, plan digest, schema version, path) into `derivation`/`derivation_input` rows (C14). Keep the cascade from `source_objects` and the completion bitwords. |
| Read-set memo + cutoff | M `spikes/facts-b/src/ygraph/{engine.rs, intern.rs}` (core only) | `facts/propagate.rs` | Remove link-specific query kinds (`K_LINK`, `link::exec`) in favour of generic fact families. Read sets go into `derivation_input`. `propagate` becomes a batch wave over the dirty frontier instead of one query at a time. Keep the early cutoff on an equal `output_hash` (C5) and backdating. The interner moves to the Phase 0 atoms. |
| Layered map | M `spikes/facts-b/src/lmap.rs` | `facts/overlay.rs` | Becomes the in-memory overlay over SQLite (C7). Add byte accounting against the 500 MB pool, a cap with back-pressure (C8), spill to runs (next row), and tombstones for retracted facts. |
| Columnar runs | D `cr/violations/projection.rs` (+ tests) | `facts/runs.rs` | Columnar batches of u32 atoms per relation (C18, X). Spill file format, bulk merge into SQLite, background writer thread. |
| Tree result pages | G `git show 8fe9ffed4` (`tree_result_segments`, schema 10 writer and attach) | `content_store/tree_pages.rs` | Re-key by `derivation` (tree hash, prefix, `context_chain`, plan identity) (C16). W already has `git_trees`, `git_tree_entries`, `snapshots`, `completed_directory_reuse`, `put_git_trees_bulk`, `reusable_directory_prefixes`: keep them. |
| Write new | — | `content_store/derivation.rs`, schema | `derivation`, `derivation_input`, `name_binding`. The validity join is the staleness check (§4). Observers (W `git_workspace.rs`, the filesystem watcher, buffers) rebind names. A Git root-tree diff seeds the push batch (§15). Add the over-invalidation counter (C19). |

Check: unit tests for an edit to one blob → exact frontier, cutoff on unchanged output, overlay cap → spill → merge.

**Status (2026-10-26): done, check met.**

- Gate: full `cargo test --lib`, run once: 1504 passed, 12 failed, 22 ignored; 141 s user + 11 s sys CPU. All 12 failures are environmental, not from this phase: 3 need pack fixtures that are not in git (`npm run fixtures:pull`), and 9 `git_workspace` tests build temp repos with `git add .`, which the agent git hook blocks.

- Modules: `content_store/{atoms, derivation, facts_memo, tree_diff, tree_pages}.rs` and `code_rule_dsl/facts/{observe, propagate, memo, pages, runs, overlay}.rs`; provenance in `provenance.md`.
- Open items carried forward: O1–O6 in `open-items.md`. O1 (stale-fact detection after a crash) must be validated before the overlay is used outside tests.
- 2026-10-27: O1 closed. The push marker, `invalid_derivations` and `observe::recover` are in, and crash points through the store and the overlay are validated (`facts/observe/tests/recovery.rs`). O2–O6 remain open.

### Phase 2: core facts from the tape

| Copy | From | To | Adaptations |
|---|---|---|---|
| `Facts::build` | M `spikes/facts-a/src/facts.rs` (+ tests) | `facts/core.rs` | Change the visibility of W `tape::TapeDoc`/`Grammar`/`LineTable`, since it lives inside `native/`. Remove the hard-coded `"python"`. Outputs become tape sidecars (`par`, `cidx`, line starts, `tid`) and narrow fact families with derivation rows. No per-node SQL rows. |
| Supertypes | N `cr/tape.rs` TMPTAPE5 delta (da080ae1, cd981e90) | W `cr/tape.rs` | Port the delta onto W's v7 as a new format version with abstract node classes. Lock v9np later (G8, X) using P `experiments/tape-v8/src/{main.rs, sql.rs}` (copy into `cr/tape/v9np.rs` when its 4 measurements pass). |

**Status (2026-10-27): done, except v9np (O8, conditional).**

- Tape v8 (`c3dd82e4`): N's TMPTAPE5 supertypes delta ported onto W's v7.
- Core facts (`f5ccdfd8` copy, `0417cd7e`, `baff0c35`): facts-a's `Facts::build` reads any dialect's tape; kinds are atoms `tag·1024 + code`; supertype postings; sidecars `par`, `cidx`, `cidx_named`, line starts; a content digest.
- Families (`3295791f`, `2ce1fbe2`): `#core:` and `#kinds:` derivations. A rename re-runs core and cuts off at kinds; a new kind reaches its reader. Same results over the overlay.
- Gate: full `cargo test --lib`, run once: 1512 passed, 12 failed, 22 ignored; 143 s user + 9 s sys CPU. The 12 failures are the same environmental ones as Phase 1 (3 pack fixtures not in git, 9 `git_workspace` temp repos blocked by the agent git hook).
- Open items: O2 update, O7–O10 in `open-items.md`.
- 2026-10-27: v9np copied verbatim (`96ec0387`, P's `docs/rule-engine-direction/experiments/tape-v8/src/{main.rs, sql.rs}` → `tape/{v9np.rs, v9np/sql.rs}`), outside the module tree until O8's measurements pass.

### Phase 3: prefilter, vocabulary, regex, statistics (§6, §16; H1–H4, E11, E13, E15)

| Copy | From | To | Adaptations |
|---|---|---|---|
| Anchor store | P `cr/matcher/anchors/sql/{schema.sql, store.rs, store/kinds.rs, store/publication.rs, values.rs, postings.rs, reader.rs, session.rs}` (+ `tests.rs`, `preparation_tests.rs`) | `prefilter/` | Drop `Store::temporary` and the private `file` table. Key by W `tape_objects.tape_id` in the content store. Postings, `vlex` and `vcov` become versioned fact families. Fold `ftsk` into W `search_kind_fts`, re-tokenised to `lang·1024 + local`. Keep `file_kind_counts` as the access path per file. |
| Prefilter lowering | P `cr/matcher/anchors/sql/lower.rs` (set algebra, `combine`/`union`/`factored`/`kinds`/`plan`/`pattern`, about l.170–583, 766–846), `access.rs`, `graph.rs`, `graph/program.rs`, `plans.rs` | `prefilter/lower.rs` | Keep only the candidate-set part: (file, preorder) sets from kinds, terms and probes. **Remove** `related`/`subtree_relation`/`frontier`/`neighbours`/`intervals`, `lower/children.rs`, `lower/waves.rs` and `decode.rs`; they move into transducers (§7). Keep `tree.rs`/`nera_tree` only for the pure-SQL fallback (§17). Replace the run-time `count(*)` in `pattern()` with statistics lookups (D10). |
| Vocabulary + regex | P `cr/matcher/anchors/sql/functions.rs` (`nera_terms_matching`, `nera_literal_set`), `vocabulary.sql`, `vocabulary-fill.sql`, `access.rs` text fallback (b839fa05) | `prefilter/vocabulary.rs` | One vocabulary per language, keyed in the atoms table. Expose per-term results as `R[tid]` for the transducer. Record matched-term counts in `plan_feedback` (H4, X). |
| Probes | W/P `cr/matcher/anchors/text.rs` (`TextProbe`) | `prefilter/probes.rs` | Move into production from `#[cfg(test)]`. Add suffix literals, first/last byte sets, anchored `=`/LIKE, and the non-word-boundary rule for required literals (ast-index-spike S199). |
| Kind vocabulary | N `content_store/{schema.rs, search_index.rs}` delta (`search_kind_vocab`, frequency-ordered probes) | W same files | Port the +71 lines as-is. |
| Plan-input statistics | D `cr/matcher/anchors/sql/statistics/mod.rs` (+ tests), `corpus/cache.rs` | `stats/inputs.rs` | One batched statement reads `kind_size`/`search_kind_vocab`/`search_term_vocab` and `edge_freq` from the content store, not from D's own store. |
| Ingestion statistics | T `stats.py` (port, 193 lines) | `stats/ingest.rs` | `stat_kind`, `stat_edge`, `stat_val`, `stat_col`, `stat_docrange`, renamed to 03's `kind_size`, `edge_freq`, `fact_stats`. Signed per-batch merge inside the push path. Keep the `check` mode as a test. |
| Write new | — | `stats/plans.rs` | `compiled_plans` (key = N `query/plan/identity.rs` `SemanticId` + P `graph.rs` identity), decisions with the statistics read and validity intervals, `plan_feedback`, and a `sqlite_stat1` writer. No source has these. |
| Lab | P `cr/matcher/anchors/sql/{lab.rs, replay.rs, capture.rs, lower/audit.rs}` | `prefilter/lab/` | Point it at the new store. Copy otherwise as-is. |

Check: P's prefilter tests on captured databases; `anchor_lab` steps no worse than P on the same captures.

**Status (2026-10-27): done (parts of rows 2–3 deferred, below); the check waits on O2.**

- Anchor store (`8165bd54` copy, `d24a2231`, `21028281`): rows in the content store keyed by `tape_id` (`prefilter_sets`, `tape_kind_counts`, `kind_freq`, `term_postings`, `term_freq`, `term_coverage`), cascading from `tape_objects`; `nera_postings` on every store connection; `ftsk` folded into `search_kind_fts` with kind-atom tokens (`POSITIONAL_INDEX_VERSION` 3). Five store tests ported. Nothing publishes the rows from the scan yet (with O2). `reader.rs`, `session.rs`, `preparation_tests.rs` and the lowering tests were dropped here and return with the lowering row.
- Lowering + vocabulary (`0afc80d4`, `45fa3c1f` copies; next commit): `prefilter::select(store, lang, plans, tapes)` lowers a batch of plans to one scheduled SQL program over the store's tape-keyed rows. Decoded steps (relations, pattern parent hops, uncovered-text confirmation) became inexact supersets narrowed per tape; transducers confirm (§7). Vocabulary tables in the store schema, regex functions on every connection. Deferred: `nera_tree` for the §17 SQL fallback, atom-keyed vocabulary, `R[tid]` (Phase 4 (g)), `plan_feedback` (row 8). Nothing calls `select` yet (O2).
- Probes (`daebe5db`): `matcher/anchors/text.rs` → `prefilter/probes.rs`, with suffixes, required literals and prefix ranges; the vocabulary fill seeks anchored predicates by range and probes before every regex.
- Kind vocabulary (`19980059`): N's `search_kind_vocab` and frequency-ordered kind probes, as is.
- Plan-input statistics (`ac32af4e` copy; next commit): D's batched `Statistics::read`/`choose` over `kind_freq` (now `kind_size`), `term_node_freq`, `term_kind_freq` (`PREFILTER_VERSION` 3); `pattern()` orders requirements by estimated rows instead of a run-time `count(*)`. `edge_freq` and `kind_size` come with row 7.
- Ingestion statistics (`67200d8d` copy; next commit): `kind_size` and `edge_freq` from each tape at publication, subtracted by triggers (edges from a per-set `nera_edges` segment) (`PREFILTER_VERSION` 4); T's signed `fact_stats` merge for fact relations held as SQL tables. No reader of `edge_freq`/`kind_size` until transducer plans (Phase 4).
- Compiled plans (write new, next commit): `stats/plans.rs` with `compiled_plans` keyed by a 32-byte plan identity (N's `SemanticId` comes with Phase 5's `stats/identity.rs`), each statistic read stored with its validity interval, the log2 bucket of the value (PIAT spec §4.9), and re-checked for many plans in one statement against the caller's batched read (`Statistics::reads`); D's bounded cache holds them in memory. `plan_feedback` is an EWMA of selectivity and ns per root per (plan, backend) (H4's matched-term counts go here when the vocabulary fill reports them). `write_stat1` writes `sqlite_stat1` from `fact_stats` (smallest postings-weighted fanout per index prefix) and reloads it with `ANALYZE sqlite_schema`, which scans no table. Nothing compiles through `Plans` yet (O2).
- Lab (`42cbf6cc` copy; next commit): captures are slim content stores of one batch's prefilter rows, written by `select` under `RERAIL_SQL_PROFILE_DIR`; `anchor_lab` re-lowers a policy over them with their statistics and counts VM steps; `candidate_replay` replays the captured statement. The check (lab steps no worse than P on the same captures) needs production captures, so it waits on O2 (`open-items.md`).
- Gate: full `cargo test --lib`, run once: 1570 passed, 12 failed, 24 ignored; 179 s user + 16 s sys CPU (load average 28). The 12 failures are the same environmental ones as Phases 1–2 (3 pack fixtures not in git, 9 `git_workspace` temp repos blocked by the agent git hook). A non-test `cargo check --lib` is also clean (it caught `20d6db08`).

### Phase 4: transducer runtime (§3, §5, §7; D2, D3, D9, E5–E9, F1, F2)

**Base: N `cr/packed.rs` + `cr/packed/*`.** This is about 24k lines, already ported into Rerail's crate and tape (3cbf7487). It is the only source with all four backends, including **`MetalInterpreter`** (`metal_program_backend::compile_interpreter`, `metal_batch_runtime.metal`), which D9's "interpreted first" needs. Tempo HEAD removed its opcode interpreter.

| Copy | From | To | Adaptations |
|---|---|---|---|
| Packed runtime | N `cr/packed.rs`, `cr/packed/` (all files), `cr/native_ir.rs` | `cr/transducer/packed/` | Replaces W `transducer/{jit.rs, metal_backend.rs}`, which are test- or bench-only. Keep W `persist_search_index_batch` and decide `POSITIONAL_INDEX_VERSION` (W 2 vs G 3). |
| Newer verifier proofs | G `tempo/native/src/policy_dsl/transducer/{cycle_progress.rs, greedy_contract.rs, closure_proof.rs, terminal_repeat_proof.rs, bounded_repeat_proof.rs, execution_shape.rs}` (+ `*_tests.rs`) | `transducer/packed/verify/` | Rebind to the packed `Program`. Add the rule that every op exists on CPU and Metal (F1). |
| Newer Metal pieces | G `T/{metal_launch.rs, metal_launch_budget.rs, metal_output_pages.rs, metal_typed_output.rs, metal_dynamic_execution.rs, metal_parallel.rs}`; regex G `T/{metal_regex_code.rs, metal_regex_frontier.rs, metal_regex_nfa.metal, value.rs}`, value pooling / NFA scratch (6ae6d7591, f1cd315d1) | `transducer/packed/metal/` | Diff-port against the N base. The 2 GB `METAL_BUFFER_LIMIT` and the 600-launch cap become plan-sized pools within 500 MB (K1, K3). |
| Front ends | G `T/scm.rs` (tree-sitter query → IR); N `Program::compile_{kind,supertype,regex,ast_grep_pattern}` | `transducer/packed/compile/` | These lower matcher atoms (§12 `ast(...)`) to IR. |
| Write new (on the copied base) | — | `transducer/packed/` | (a) **Set mode**: KEY labels stop at their first emit, a path is dropped once its labels have all fired, lazy cursors; the oracle becomes finding keys plus Complete/Unknown, not capture parity. (b) **EXISTS/NOT_EXISTS and COUNT barriers.** (c) **Combined hash-consed graph across rules** (fusion, P `spec-fusion.md`). (d) **One plan identity for both bindings**: interpreted runs at once, JIT is compiled in the background and swapped in. (e) **Device-side enqueue** for in-file loops whose bound is not fixed (E9): from G indirect dispatch (`metal_launch.rs:200/241`, `metal_frontier_reduce.rs`) to indirect command buffers. (f) **File-local lattice FIX waves** (I10, R0). (g) **`R[tid]` reads** for regex (§16). (h) **Text dump and assembler** (L2). |

**Status (2026-10-27): rows 1, 2 and 4 adapted; row 3 placed (`bb4c9605`), its adaptations and the write-new items outstanding.**

- Packed runtime (`5bf45ff0` copy, `a36418d0`): in the module tree under `transducer`; W's bench-only JIT and Metal backends and the `bench-backends` feature removed.
- Verifier proofs and query front end (`98e96170`, `692ff6ba` copies; next commit): Tempo's cycle-progress and greedy-contract proofs replace N's backward-goto heuristic in `validate`; `scm.rs` compiles tree-sitter queries to programs; F1 is a test that the verifier's opcodes are exactly the Metal interpreter's.
- Metal pieces (`cf19c0d9` copy, and G's grid executor `b90a7b0a`, `e92c5958`, `faf1c92b`, `be5197b5`, `6dd8ae15`): placed (`bb4c9605`). G's grid executor is `ExecutionBackend::Metal` (`metal_grid_backend`) beside N's `MetalInterpreter`/`MetalJit`, which stay; shared files are 3-way merges over N's base Tempo@176ea093d. The adaptations (plan-sized pools within 500 MB for the 2 GB `METAL_BUFFER_LIMIT` and the 600-launch cap, K1, K3) are outstanding.

Check: N's `production_contract_tests.rs` and `policy_lowering_tests.rs` on the copied base, then differential finding keys for interpreter vs JIT vs Metal on a 100–300-file sample.

### Phase 5: the `demand(batch)` vtab (§3, E6)

| Copy | From | To | Adaptations |
|---|---|---|---|
| Batch vtab | N `cr/transducer/sql_backend.rs` (3,323 lines) | `demand/vtab.rs` | Execute once per (group, batch) in `xFilter`; N's model is right, D's "callbacks cannot dispatch" is wrong. **Remove** the CPU `ast_grep` post-pass for facts (l.717–830), drop `rerail_navigation` rows, and replace the fixed `rerail_relation_statistics` seeds with `stats/`. Inputs: the candidate sets from `prefilter/`, tapes from the slab cache (Phase 9). |
| Driver | N `cr/query/mod.rs` (`SqlPlans::compile`, `drive_sql`, `SqlViolationRow`/`SqlFailureRow`) | `demand/driver.rs` | Run on the content-store connection, not a private in-memory one. Rows become match facts in the overlay. |
| Plan identity | N `cr/query/plan/identity.rs` | `stats/identity.rs` | Key for `compiled_plans`. |
| Reusable from N's SQL lowering | N `query/plan/sql_ast.rs` (typed SQL AST), `typed_boundary.rs`, `physical.rs` `FactFamilyContract` | `logic/sql/` | Copy these. **Do not copy** `relational_ir.rs`/`relational.rs` (tree axes in SQL). |

**Status (2026-10-27): copies done (`b132cef3`, `cd149f23`, `4d6f724c`, `65183b55`, `1fa55934`); rows 1 (`1e951cea`), 3 (`4c54f94d`) and 4 (`5feaaddd`) adapted; row 2 adapted (`aa9d1f9a`).**

- Batch vtab: `demand::vtab` on the packed runtime. The CPU `ast_grep` fact post-pass, the navigation producer and their relations are gone, which leaves batch status, producer status, terminal matches, captures and document maps. Row estimates come from `fact_stats` for each relation, which the driver fills when it merges match facts (row 2). Capture rows still compute N's `semanticPlain` structural key with the CPU value adapter; that moves to the findings layer's equality (Phase 8). Inputs from `prefilter/` candidate sets and the slab cache wait on Phases 7 and 9.
- Driver: `demand::driver::drive(store, batch, statements, files)` runs lowered statements over a registered batch in one transaction on the content store and decodes violation and failure rows. N's catalog lowering (`SqlPlans::compile` over `relational_ir`) is not copied, per the table, so the driver takes statements from the logic planner (Phase 6). For the same reason N's two contract suites (`production_contract_tests.rs`, `policy_lowering_tests.rs`), which Phase 4's check names, cannot run until Phase 6 lowers rule catalogs; they stay outside the tree until then.
- Gate: full `cargo test --lib`, run once: 1601 passed, 12 failed, 27 ignored; 139 s user + 12 s sys CPU (load average 21–28). The 12 failures are the environmental ones of Phases 1–3 (3 pack fixtures not in git, 9 `git_workspace` temp repos blocked by the agent git hook).

### Phase 6: the logic language (§12; B1, B3, B4, B10–B14, I9)

| Copy | From | To | Adaptations |
|---|---|---|---|
| Native fixpoint | T `rdl/sqlite-ext/src/{setfix.rs, lib.rs}` | `logic/native/` | Read from fact families through the overlay, not per-run arenas. Deltas per wave. Use the atoms interner. Generalise `lattice_fixpoint` from `flat` to a declared join plus `widen`, kept as the CPU binding and reference semantics. File-local lattices run in the transducer (Phase 4f). Gate at 500 MB. |
| Front end | T `rdl.py` `tokens`, `parse`, AST, `strata`/`tarjan`, `prune` (**port**) | `logic/front/` | Add the named errors from §12 (aggregation in recursion, unbounded lattice, unbounded value cycle, non-interned cross-document join, cost gate), `min`/`max`, general `.lattice`, `ast(...)`. |
| YAML/JSON syntax | W `cr/compile/*` (existing rule loader, `ast_rule.rs`) | `logic/front/yaml.rs` | Round-trip rule documents into the same AST (B12). |
| Magic sets | T `rdl.py` `magic_sets`, `magic_adorn*`, `gate_negations` (**port**) | `logic/magic.rs` | Change whole-program demand to install per wave: magic relations become each wave's demand batch (C10). Also drives the dependency universe (I18). |
| Planner + lowering | T `rdl.py` `Stats`, `plan_rule` (Selinger DP), `make_plan`, `sql_select`, `semi_naive_round`, `merge_lattice`, `native_stratum`, `join_spec`, `observe` (**port**) | `logic/plan.rs` | Plans go to `compiled_plans`. The emitted SQL goes through N `sql_ast.rs`. Per-document partition recognition marks rules file-local, which sends them to transducer groups (Phase 4). |
| References only | M `facts/src/{phys.rs, explain.rs}` | — | Plan-audit rule (refuse a scan per outer row) and the EXPLAIN format, for L2. |

**Status (2026-10-27): copies done (`8934168d` native fixpoint; `091b65fb` `rdl.py` and its programs, the port's source); adaptations outstanding. The YAML row is W's own `compile/*`, so it has no copy.**

**Status (2026-10-27): rows 1 (steps 1–3), 2 and 4 adapted: `logic::native` (lattices, `widen`, store atoms), `logic::front`, and `logic::magic` (T's `magic_sets`, `magic_adorn` and `gate_negations`, matching T's transformed programs on its five programs in all three modes). Row 5 step 1: `logic::plan`, T's planner and SQL evaluation, choosing T's plans and deriving T's outputs on three of its programs over a nine-module fixture. Row 3: `logic::front::yaml`, every pack's `rerail.policy/v1` document round-tripping through the logic AST and Datalog text, forbid matchers as `violation(…) :- ast($D, $N, {…})` rules and the rest as `.policy {…}`. Outstanding: row 1 step 4 (after Phases 7–8), row 5's native strata (after O13, symbols as atoms in the planner's SQL), `sql_ast` lowering and `compiled_plans`; O11, O12, O13.**

Check: T's goldens (`score_calls.py`, `score_rust.py`) reproduced by the Rust port on T's 574/331-file workloads; CPU and RSS ≤ T's numbers.

### Phase 7: SQL-driven waves (§4, §7; E1, E12, E14)

| Copy | From | To | Adaptations |
|---|---|---|---|
| SQL owner | D `cr/relational/{pool.rs, database.rs}`, `cr/execution/mod.rs` | `waves/owner.rs` | One owner on the content store. `trim`/`statistics_boundary` become statistics refreshes into `compiled_plans`. |
| Schema lifecycle | D `cr/relational/storage.rs`, `storage/contexts.rs` (+ tests) | `waves/storage.rs` | Backing tables become fact families. Dirty-cut `ANALYZE` becomes a statistics write. |
| Fact-access vtab | D `cr/relational/{table.rs, schema.rs, schema/cache.rs}`; P `relational/table.rs` | `facts/vtab.rs` | Access over overlay ∪ SQLite with `best_index` on EQ/IN/range, with a version check for staleness (§4). |
| Frontier loop | D `cr/relational/batch.rs` (frontier ordering only) | `waves/schedule.rs` | Replace the Rust `advance`/`Step` loop with SQL statements that call `demand(batch)` and loop until no delta. Batch admission by resident bytes becomes 500 MB pools. Overlap cache preparation for chunk k+1 with chunk k (E12). |

**Status (2026-10-27): copies done (`6770e5fe`, `2f3913ae`, `13605f8d`, `2c92eb35`, `554b89c2`); adaptations outstanding. Row 3 step 1: the generic integer vtab with S's `Relation` merged in (decision 0008); step 2: T's base relations over core facts, equal to T's exporter rows on the fixture corpus; step 3: the planner over these inputs in atoms (O13's SQL side, decision 0009), T's plans and outputs on the fixture. Rows 1, 2 and 4 (decision 0010): one owner on the content store with leases and recycling, D's schema lifecycle with counted rows in `temp.wave_stats` instead of `ANALYZE`, and the frontier loop selected by SQL with `demand(batch)` per chunk, chunks of 125 MB built ahead on a worker thread, and derived relations recorded as fact families. The planner emitting wave steps is O14.**

### Phase 8: facts by vtab and the findings layer (§8; E10, J1)

| Copy | From | To | Adaptations |
|---|---|---|---|
| Producers | S `cr/query/plan/{python_structure.rs, call_batch.rs, import_screen.rs}`, `cr/matcher/python_facts.rs` (+ tests) | `facts/producers/python/` | Out of `#[cfg(test)]`. Fed from `prefilter/` candidate sets, not full tape scans. Write `py.*` fact families with derivation rows. Keep demand pruning (a55956e3). |
| Relation vtab | S `cr/query/plan/relation.rs` | merge into `facts/vtab.rs` | Add constraint handling; the overlay is the backing. |
| Typed SQL shape | S `cr/query/plan/sql_shape.rs`, `async_reference/generated_shape.rs` | `findings/shape.rs` | Becomes the lowering target for the findings layer instead of a hand-built plan. |
| Contract | S `cr/query/plan/contract.rs` | `findings/contract.rs` | Copy as-is (`FindingKey`, Complete/Unknown, `Truth`). Move consolidation (S `async_reference.rs` ~711–748) into the one SQL findings layer. |
| Oracle | S `async_reference.rs`, `native_oracle.rs`, `tests.rs` | `findings/tests/` | Kept as test oracle only. |

**Status (2026-10-27): copies done (`69371b1c`, `0689eb25`, `6c0e860e`, `8190c8fa`, `0e4a64c3`); adaptations outstanding. S's oracle lives in `query/plan/async_reference/{native_oracle.rs, tests.rs}`, not beside `async_reference.rs`. Placed in the module tree with path changes only: rows 1 (`40ef4d49`, `facts::producers::python`), 3 (`4514b3f4`, `findings::shape`) and 4 (`be038523`, `findings::contract`). Row 5 (oracle) is blocked on S's matcher deltas, which no row copies (O15).**

### Phase 9: residency and the pinned GPU cache (§5; F3–F8, K1)

| Copy | From | To | Adaptations |
|---|---|---|---|
| No-copy buffers | G `T/metal_backend.rs` (`MappedRegion`, `MappedBuffer`, `new_buffer_with_bytes_no_copy`, `StorageModeShared`, guard, pipeline cache) | `residency/metal.rs` | One retained buffer per 64 MiB slab. Remove `PrivateInputUpload` unless Q-R1 (F8) shows private storage wins. |
| Batch admission | P `cr/relational/batch.rs` (resident capacity), `workspace/memory/census.rs`, `profile/memory.rs` | `residency/admission.rs` | Byte budget of 500 MB in total, split into pools. |
| Write new | — | `residency/slabs.rs` | zstd frames decompressed straight into anonymous-mmap slabs, a document table per batch (no copies), sidecars, a pipeline/program cache keyed by plan identity, wave preparation, a drop timer 2 min after each full execution. No source implements this; reference P `spec.md` §20. |

**Status (2026-10-27): copies done (`b558a528`, `b90fe9af`); adaptations and the write-new row outstanding. Placed: `residency::admission::memory` (`602c2ef6`). Outside the tree: `metal.rs` (Tempo's `policy_dsl` Metal modules, with Phase 4 row 3), `census.rs` (P's typed projections) and `batch.rs` (D's sessions).**

### Phase 10: typing (§13; I1–I18)

| Copy | From | To | Adaptations |
|---|---|---|---|
| Programs | T `rdl/{calls.rdl, calls_rust.rdl, workload*.rdl}` | `typing/programs/` | Split file-local strata (`walk`/`look`/`defn`/`bound_in`, R0) from cross-file ones (`pv`/`found`/`qv`, R1..Rk). Remove hand-written demand (magic sets). |
| Wavefront + digest cutoff | T `lattice/src/main.rs` (cross-file wavefront, interface-digest cutoff) | `typing/rounds.rs` | Bounded R1..Rk, with k = 2 (I11, X). |
| Write new | — | `typing/terms.rs`, `typing/declare.rs`, `typing/deps.rs` | Interned type terms (I12), instantiation of generics (I13), `proven`/`possible` (I14), `rerail.types/v1` that fills gaps only (I4, I17), dependency summaries per (package version, module file) with magic-set demand (I18). There is no code in any branch; the design is in the Typing transcript, recorded in 03 §13. |

**Status (2026-10-27): copies done (programs with Phase 6's `091b65fb`, wavefront `59cb33bf`); adaptations and the write-new row outstanding. Placed: `typing::rounds` (`11d2d5ee`), every strategy equal to T's `workload` goldens on the planner fixture.**

### Phase 11: gate and tooling (throughout; M1–M4, L1, L2)

| Copy | From | To | Adaptations |
|---|---|---|---|
| Data-layer gate | D `packs/data-layer/candidates/rules/*.json`, `conformance/data-layer/{suite.ts, replay.ts}`, `scripts/dsl/data_layer_suite_record.py`, `fixtures/host/data-layer-port/batch-0{1..6}-recording.json`, `scripts/profiling/process.py` | same paths | 10 of 60 rules carry SQL (`relational` stages). Rewrite them as named facts plus logic (B2) before they enter the gate. Gate on CPU (M1), RSS ≤ 500 MB, and finding-key sets. |
| Profiling | D or P `cr/profile/{sql.rs, memory.rs, trace.rs}`, P `phase_clock.rs`, `budget.rs` | `cr/profile/` | Fixed stage names from 03. Construction is reported separately (M2). |

**Status (2026-10-27): copies done (`8cb32ac5`, moved under `gate/data-layer/` by `d0c47184`; profiling `8dcf82a1`, from P, under `profile/copied/`); adaptations outstanding. Profiling placed (`16bb6e48` copy of the rest of the unit, `5c77b049`; `8637fba8` `working_set`), tests passing with `code-rule-profile`.**

## Explicitly not copied

- N `query/plan/render/relational_ir.rs`, `relational.rs` and `plan/*` lowering (tree axes in SQL).
- D and P `anchors/sql/lower*` relation evaluation.
- G `admission_*`, `packed_runtime_spike.rs`, `metal_cutover_architecture.rs` (test spikes).
- T Ascent backend.
- M differential-dataflow (M0), M2 executor, Y2 level witness, `factfile.rs` (reference only), `facts/src/front/*` (rules carry SQL).
- W `transducer/{jit.rs, metal_backend.rs}`: replaced by the copied packed runtime.

## Write-new list (no source anywhere)

1. `derivation`, `derivation_input` and `name_binding`.
2. `compiled_plans`, `plan_feedback`, `fact_stats`, `edge_freq` and the `sqlite_stat1` writer.
3. Set mode, EXISTS/COUNT barriers, the combined cross-rule graph, and the interpreted-then-JIT swap.
4. Device-side enqueue.
5. The slab cache and the 2-min drop.
6. The text dump and assembler.
7. YAML ↔ Datalog round trip and `ast(...)`.
8. Type terms, declarations and dependency summaries.

## First steps

1. Phase 0 dependency port and the atoms decision.
2. `COPY` the N packed runtime and the M `facts_memo` + `lmap` + Y1 core, each verbatim in its own commit.
3. Write `derivation` tables.
4. Adapt `facts_memo` onto them. This is the first end-to-end slice: a blob edit invalidates exactly its derivations, and an unchanged output cuts off.
