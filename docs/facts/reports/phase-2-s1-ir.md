# Phase 2 / S1: IR, front ends, classifier, census

Status: **G2.1 met as stated: 60/60 rules compile and classify, or every refusal is named; the seeded leak is rejected.**

- With Spike A's leaf executors, 20 rules compile without refusal. The other 40 refuse pattern or regex leaf constructs that the leaf executor (S2's `ast.rs`/`eval.rs`) lacks; every one is named and counted in § 5.3.
- At IR level, with every leaf accepted, 55/60 compile. The 5 that do not refuse cross-file constructs with no exact incremental form (§ 5.4), as PHASE-2's kill criterion requires.

Branch `Moti-Velz/facts-engine-s1-ir` (local, not pushed); worktree `~/orca/workspaces/rerail/facts-engine/.worktrees/s1-ir`.

## 1. Setup

| Item | Value |
|---|---|
| Base | integration `Moti-Velz/facts-engine` @ `a84473f25` (both spike crates merged) |
| Crate | **`facts/`, package `rerail-facts`**, workspace member next to `native/` (decided in `43bd9c048`). Not `native/` (the shipped cdylib, changed by visibility only); not `spikes/` (deleted after Phase 2). `sql/parse.rs`, `sql/value.rs` and `policy.rs` moved from facts-a by `git mv`; facts-a re-exports them, so no other slice's paths changed |
| Build | `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target cargo build --release -p facts-a -p facts-b` (green after the move) and `cargo test -p rerail-facts` (12 unit tests and 1 `compile_fail` doctest) |
| Corpus | 60 policies `packs/data-layer/candidates/rules/*.json` (unmodified); for the volume run, `~/rerail-bench/nera-data-layer-2b1504ad` (9,010 files) |
| Machine | darwin arm64. No timing gate is owned by S1: the census and EXPLAIN runs are deterministic outputs, and the volume run counts rows (load 19.9–20.8 during it: counts are load-independent, its time is not reported) |

## 2. Results

| Gate | Target | Measured | Pass/fail | Run dir (`~/rerail-bench/facts/phase-2/s1-ir/runs/…`) |
|---|---|---|---|---|
| G2.1 compile + classify | 60/60, or each refusal named | **60/60 classified, every refusal named.** Leaf-executor pass: 20/60 compile clean; 282 refusals over 40 rules, all named (§ 5.3), 1 runtime refusal (`annotation`). IR-level pass: 55/60 compile clean; 33 refusals over 5 rules (§ 5.4): 31 order-dependent constructs in repository plans, `reference_edges`, a `calls` atom | **pass** (as stated; see § 6 for what "compile" means at each level) | `20261022T155655-census` (`census.md`, `census-ir.md`, `census-ir-scan-project-sources.md`, JSON beside each) |
| G2.1 leak test | the seeded cross-file leak fails to compile | `cargo test -p rerail-facts leak`: 4 tests. The seeded `partitionBy: file` clause reading `reference_edges` is refused ("cross-file read in a per-file plan: `reference_edges`"); so are a `calls` atom in a `forbid` and a `file.module` projection in a per-file input; the control compiles. Type level: a `compile_fail` doctest (`ir.rs`) shows `Rel<FileSource>` cannot hold `RepoSource::Exported`. Mutant 7 (`classify::file_source` admits a cross-file leaf) fails 3 of the 4 | **pass** | `cargo test` output (no run dir; reproduce with the command) |
| Move: no behaviour change | Spike A replay unchanged | 1,253/1,259; 0 FP; 2 FN; 6 failures; all mismatches `serverplan_quality` (= Phase 1) | **pass** | `20261022T145533-replay` |
| Parity after merging integration (`2b111331b`) | replay and repository unchanged | replay: 1,253/1,259, 0 FP, 6 failures, all `serverplan_quality`. Repository: 173/173, 0 FP, 0 FN, 38 failures, all `quality-contracts-1`; file set 9,010 = oracle's | **pass** | `20261022T154937-replay`, `20261022T155011-repo/parity.json` (bench-lock; load 29–31, so its time is not reported) |
| EXPLAIN stable | byte-identical across runs | logical EXPLAIN of all 60 rules: 10,910 lines; two runs identical (sha256 `02aa1ebc…`) | **pass** | `20261022T152742-explain` |
| Rewrite mutants | each caught | 7, 8, 11, 12, 13, 14, 15: each fails ≥ 1 `rerail-facts` test with `--features mutants FACTS_MUTANT=<n>`; the clean run passes 12/12 | **pass** | command in § 3 |

## 3. How measured

- **Census:** `./target/release/facts-a census --rules packs/data-layer/candidates/rules --out <run>` writes three passes.
  - `census.*`: default options, leaves compiled by facts-a's `leaf.rs` (Spike A's `ast.rs` compiler and the regex features `eval.rs` implements).
  - `census-ir.*`: default options, every leaf accepted (`rerail_facts::census::IrOnly`): what the IR, the front ends and the classifier decide.
  - `census-ir-scan-project-sources.*`: `entryPoint = scan_project_sources`, which makes `state-flow-1` and `state-projection-recovery-2` repository clauses.
- **Leak test:** `cargo test -p rerail-facts leak`.
- **Mutants:** `FACTS_MUTANT=<n> CARGO_TARGET_DIR=$PWD/target cargo test -p rerail-facts --features mutants --lib`, for n in 7 8 11 12 13 14 15.
- **EXPLAIN:** `./target/release/facts-a explain [--rule <id>] [--config '<json>']`, run twice and compared with `cmp`.
- **Export volume:** `/usr/bin/time -l ./target/release/facts-a export-volume --threads 4 --out <run>`.
  - It covers the batch-6 rules with repository clauses (the leaf executor runs their inputs).
  - It counts the witness rows of every input each exported relation scans, over the 9,010 files, with Spike A's `Input::rows` under the policy scope and the input's `paths`.
- **Replay after the move:** `./target/release/facts-a replay --recording fixtures/host/data-layer-port/batch-06-recording.json --recording fixtures/host/data-layer-port/batch-06-probes.json --output <run>/results.json`.

## 4. Artifacts

- Census: `20261022T155655-census/*.json`. The JSON lists every refusal per rule (clause, location, construct), every exported relation's columns and flags, the sharing groups with their stage lists, and the order-dependent constructs per repository plan.
- Logical plans of all 60 rules: `20261022T152742-explain/explain-1.txt`.
- Export volume: `20261022T153856-export-volume/export-volume.json`.

## 5. Design notes

### 5.1 IR (`facts/src/ir.rs`, spec `docs/facts/IR.md`)

- **Algebra.** Relational algebra over typed columns (name and SQLite affinity), with SQLite semantics and row order defined where the rules observe it. Operators: `Scan`, `Local`, `Unit`, `JsonEach` (lateral), `Join` (inner/left, nested-loop order), `Filter`, `Project`, `Aggregate` (representative row then aggregates: SQLite bare columns), `Window` (`row_number()` in the total order partition, keys, input position; partition `min`/`max`), `Distinct {prefix}`, `Union {all}`, `Sort {keys, limit, offset}`, and `With` (plain bindings, or `Fix` with set or bag semantics, level by level).
- **Subqueries.** Semijoin, antijoin and scalar subqueries are expressions (`Exists`, `InRel`, `Scalar`), with correlation as `Col {depth > 0}`.
- **Reachability** is a stage form (`StageDef::Reach`, directed / forest / syntax).
- **Deferred columns** are inlined (R2).
- **Literals** are `ir::Lit`. `Program` is `Send + Sync` (statically asserted), so one `Arc<Program>` serves every worker.
- **Classification is a type parameter.** `Rel<FileSource>` (Input | Syntax | Stage of the same content) and `Rel<RepoSource>` (Exported | Stage); the front ends produce `Rel<AnySource>`.
- **Entry point:** `compile(doc, config, leaves) -> Result<Program, Unsupported>`.
  - `Program` = `{ inputs, per_file: Vec<Plan<FileSource>>, repository: Vec<Plan<RepoSource>>, exported: Vec<ExportedRelation>, report_when, runtime_refusals }`.
  - `compile_report` returns the program of every clause that compiled, plus all refusals (the census, and S3's adapter, read it).

### 5.2 Front ends and classifier

- **SQL** (`front/sql.rs`): lowers Spike A's parse tree. It resolves names (only the top-most scope of each level is visible, so no lateral derived tables), fixes comparison affinities, and lowers `BETWEEN`.
  - `HAVING` without `GROUP BY` is accepted (parser change).
  - New in the IR beyond Spike A's planner: `printf`, `json_array`, `json_group_array`, window `min`/`max`.
  - **Every SQL text of the 60 rules lowers**: 451 stages, 52 final queries, the deferred columns, and the 15 view texts of S4's SCHEMA.md v1.
  - No SQL text survives the front end (I3).
- **Policies** (`front/rule.rs`):
  - `$option` resolution and `when` (`option`, `pythonVersion`) are applied statically; 53 clauses are disabled by `when` under the defaults.
  - `report_when` is read.
  - `forbid.ast`/`forbid.regex`/`budget`/`relational` become unclassified clauses; pattern and regex leaves are checked by the injected `LeafCheck`.
  - Rewrite R1 (`ast.file.path` → `filePath`) runs here.
  - A pattern with a `calls` atom, or a `file.module`/`file.package` projection, marks its input cross-file.
- **Classifier** (`classify.rs`, I6: dependency typing, the declaration checked):
  - A per-file plan goes through `file_source`, and any cross-file leaf is a compile error.
  - A `forbid` is per file; a cross-file leaf there is a refusal.
  - A `budget` is per file iff `group_by` and every closure partition hold a field projected from `file.path`.
  - A repository clause is split by file-locality (R3), with export projection (R4) and distinct (R5), and its repository side is audited for order (below).
- **Order audit (repository plans).** Candidate keys are inferred from exports read as sets, `DISTINCT`, set `UNION`, `GROUP BY` keys, `row_number` partitions and reachability. The audit refuses:
  - `row_number()` whose `PARTITION BY` + `ORDER BY` is not a key;
  - `LIMIT` without a key order;
  - `group_concat`/`json_group_array` without a total sort;
  - aggregate bare columns;
  - bag recursion.

  Per-file plans keep input-order tie-breaking: PHASE-2 accepts it, and it is deterministic per IR § 4.
- **Registry:** `docs/facts/REWRITES.md`: R1–R5 implemented, each with a test and a mutant; the decider's seven further rewrites are listed as planned.
- **EXPLAIN:** `explain.rs` prints the logical plans. `phys.rs` holds the physical node types agreed with S2, their EXPLAIN, and the inner-scan audit (`cargo test -p rerail-facts audit`, mutant 8).

### 5.3 Census, leaf-executor pass (what runs today)

Compiled without refusal: **20 / 60** rules.

| rule | forbid | budget | relational | disabled by `when` | per-file plans | repository plans | exports | refusals | runtime refusals | compiles |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|---|
| `admin-async-purity` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 3 (3 clauses) | 0 | **refused** |
| `async-executor` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 1 (1 clauses) | 0 | **refused** |
| `async-patterns` | 0 | 0 | 0 | 8 | 0 | 0 | 0 | 24 (24 clauses) | 0 | **refused** |
| `async-sync-boundary` | 60 | 0 | 0 | 3 | 60 | 0 | 0 | 0 | 0 | yes |
| `async-sync-delegation` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 13 (13 clauses) | 0 | **refused** |
| `backend-coupling` | 2 | 0 | 0 | 0 | 2 | 0 | 0 | 0 | 0 | yes |
| `backend-type-strings` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 13 (13 clauses) | 0 | **refused** |
| `classvar-check` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 1 (1 clauses) | 0 | **refused** |
| `conn-or-txn-union` | 2 | 0 | 0 | 0 | 2 | 0 | 0 | 0 | 0 | yes |
| `connection-caches` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 48 (48 clauses) | 0 | **refused** |
| `compat-aliases` | 1 | 0 | 0 | 0 | 1 | 0 | 0 | 1 (1 clauses) | 0 | **refused** |
| `facade-purity` | 5 | 0 | 0 | 0 | 5 | 0 | 0 | 1 (1 clauses) | 0 | **refused** |
| `dynamic-module-loading` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 5 (5 clauses) | 0 | **refused** |
| `error-handling` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 2 (2 clauses) | 0 | **refused** |
| `execution-authority-bypass` | 0 | 0 | 1 | 0 | 1 | 0 | 0 | 0 | 0 | yes |
| `forwardref` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 3 (3 clauses) | 0 | **refused** |
| `grammar-allowlist-burndown` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 5 (5 clauses) | 0 | **refused** |
| `grammar-callable-constructors` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 8 (8 clauses) | 0 | **refused** |
| `grammar-fragment-usage` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 5 (5 clauses) | 0 | **refused** |
| `grammar-render-methods` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 6 (6 clauses) | 0 | **refused** |
| `grammar-str-params` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 3 (3 clauses) | 0 | **refused** |
| `hasattr-async` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 2 (2 clauses) | 0 | **refused** |
| `imperative-private-functions` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 3 (3 clauses) | 0 | **refused** |
| `impl-direct-execution` | 0 | 0 | 1 | 0 | 1 | 0 | 0 | 0 | 0 | yes |
| `internal-dead-code` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 4 (1 clauses) | 0 | **refused** |
| `layer-violations` | 4 | 2 | 0 | 0 | 6 | 0 | 0 | 0 | 0 | yes |
| `lazy-result-usage` | 6 | 0 | 0 | 0 | 6 | 0 | 0 | 0 | 0 | yes |
| `legacy-execution-surfaces` | 7 | 0 | 1 | 0 | 8 | 0 | 0 | 0 | 0 | yes |
| `missing-async-pairs` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 3 (3 clauses) | 0 | **refused** |
| `module-level-caches` | 0 | 0 | 0 | 6 | 0 | 0 | 0 | 34 (34 clauses) | 0 | **refused** |
| `no-api-root-imports` | 2 | 0 | 0 | 0 | 2 | 0 | 0 | 0 | 0 | yes |
| `no-blanket-grammar-bypass` | 1 | 0 | 0 | 0 | 1 | 0 | 0 | 2 (2 clauses) | 0 | **refused** |
| `no-source-inspection` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 5 (5 clauses) | 0 | **refused** |
| `permission-error` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 1 (1 clauses) | 0 | **refused** |
| `plan-scope-ownership` | 21 | 0 | 0 | 0 | 21 | 0 | 0 | 5 (5 clauses) | 0 | **refused** |
| `plan-scope-side-effects` | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 5 (5 clauses) | 0 | **refused** |
| `public-api-triples` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 3 (1 clauses) | 0 | **refused** |
| `raw-execute-regression` | 7 | 0 | 0 | 0 | 7 | 0 | 0 | 0 | 0 | yes |
| `regex-sql-parsing` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 9 (9 clauses) | 0 | **refused** |
| `removed-compat-surfaces` | 3 | 0 | 0 | 0 | 3 | 0 | 0 | 0 | 0 | yes |
| `render-fstrings` | 1 | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 0 | yes |
| `render-raw-strings` | 1 | 0 | 0 | 0 | 1 | 0 | 0 | 3 (3 clauses) | 0 | **refused** |
| `rust-sql-grammar-boundary` | 8 | 0 | 0 | 0 | 8 | 0 | 0 | 0 | 0 | yes |
| `serverplan-bypass` | 0 | 3 | 0 | 0 | 3 | 0 | 0 | 0 | 0 | yes |
| `serverplan-dispatch-architecture` | 8 | 0 | 1 | 0 | 9 | 0 | 0 | 0 | 0 | yes |
| `serverplan-grammar-surface` | 7 | 0 | 0 | 0 | 7 | 0 | 0 | 0 | 0 | yes |
| `serverplan-name-inference` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 4 (4 clauses) | 0 | **refused** |
| `serverplan-quality` | 1 | 0 | 6 | 0 | 7 | 2 | 40 | 2 | 1 | **refused** |
| `serverplan-state-tripwires` | 0 | 0 | 32 | 0 | 32 | 1 | 52 | 3 | 0 | **refused** |
| `serverplan-steps` | 14 | 0 | 7 | 0 | 21 | 0 | 0 | 0 | 0 | yes |
| `serverplan-target-architecture` | 15 | 0 | 1 | 0 | 16 | 1 | 8 | 0 | 0 | yes |
| `sql-grammar-escape-hatches` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 11 (11 clauses) | 0 | **refused** |
| `swallowed-exceptions` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 4 (4 clauses) | 0 | **refused** |
| `sync-async-signatures` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 1 (1 clauses) | 0 | **refused** |
| `transaction-callback-wrappers` | 2 | 0 | 0 | 0 | 2 | 0 | 0 | 0 | 0 | yes |
| `transaction-closures` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 6 (6 clauses) | 0 | **refused** |
| `transaction-patterns` | 0 | 0 | 0 | 35 | 0 | 0 | 0 | 23 (23 clauses) | 0 | **refused** |
| `type-safety-patterns` | 3 | 0 | 0 | 0 | 3 | 0 | 0 | 6 (6 clauses) | 0 | **refused** |
| `unresolved-forward-refs` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 1 (1 clauses) | 0 | **refused** |
| `versioned-lifecycle-legacy-surfaces` | 1 | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 0 | yes |
| **total** | 182 | 5 | 50 | 53 | 237 | 4 | 100 | 282 | | |

Refused constructs (count; rules):

| construct | count | rules |
|---|--:|---|
| SQL: no such table: raw_parameters | 2 | `internal-dead-code` 1, `public-api-triples` 1 |
| order-dependent in a repository plan: LIMIT under an ORDER BY that is not a key (ties broken by input order) | 1 | `serverplan-quality` 1 |
| order-dependent in a repository plan: bag (UNION ALL) recursion: no set fixpoint for the dataflow | 2 | `serverplan-quality` 1, `serverplan-state-tripwires` 1 |
| order-dependent in a repository plan: row_number() whose PARTITION BY + ORDER BY is not a key (ties broken by input order) | 2 | `serverplan-state-tripwires` 2 |
| pattern: count binding unsupported | 1 | `missing-async-pairs` 1 |
| pattern: parameterised matches unsupported | 9 | `async-sync-delegation` 4, `grammar-callable-constructors` 1, `grammar-fragment-usage` 1, `grammar-render-methods` 2, `sync-async-signatures` 1 |
| pattern: pattern `callable(annotation)` unsupported (only metavariables and single names) | 1 | `forwardref` 1 |
| pattern: pattern `contextlib.suppress` unsupported (only metavariables and single names) | 2 | `swallowed-exceptions` 2 |
| pattern: pattern `dict()` unsupported (only metavariables and single names) | 34 | `module-level-caches` 34 |
| pattern: pattern `isinstance(annotation, type)` unsupported (only metavariables and single names) | 1 | `forwardref` 1 |
| pattern: pattern `issubclass(annotation)` unsupported (only metavariables and single names) | 1 | `forwardref` 1 |
| pattern: pattern `queue.Empty` unsupported (only metavariables and single names) | 2 | `swallowed-exceptions` 2 |
| pattern: pattern `txn.commit()` unsupported (only metavariables and single names) | 1 | `error-handling` 1 |
| pattern: unsupported rule key `arguments` | 3 | `render-raw-strings` 2, `unresolved-forward-refs` 1 |
| pattern: unsupported rule key `binds` | 4 | `imperative-private-functions` 3, `internal-dead-code` 1 |
| pattern: unsupported rule key `follows` | 5 | `classvar-check` 1, `error-handling` 1, `grammar-str-params` 3 |
| pattern: unsupported rule key `layout` | 21 | `async-patterns` 12, `grammar-render-methods` 1, `permission-error` 1, `plan-scope-side-effects` 5, `sql-grammar-escape-hatches` 1, `type-safety-patterns` 1 |
| pattern: unsupported rule key `params` | 25 | `async-patterns` 12, `grammar-callable-constructors` 7, `grammar-fragment-usage` 4, `missing-async-pairs` 2 |
| pattern: unsupported rule key `precedes` | 39 | `internal-dead-code` 2, `plan-scope-ownership` 5, `public-api-triples` 2, `sql-grammar-escape-hatches` 1, `transaction-closures` 6, `transaction-patterns` 23 |
| pattern: unsupported shape | 1 | `compat-aliases` 1 |
| pattern: value key type unsupported | 28 | `async-sync-delegation` 9, `facade-purity` 1, `grammar-allowlist-burndown` 5, `grammar-render-methods` 3, `regex-sql-parsing` 9, `render-raw-strings` 1 |
| regex leaf: `content` gate | 58 | `connection-caches` 48, `dynamic-module-loading` 5, `type-safety-patterns` 5 |
| regex leaf: `lines.exclude` | 38 | `admin-async-purity` 3, `async-executor` 1, `backend-type-strings` 13, `hasattr-async` 2, `no-blanket-grammar-bypass` 2, `no-source-inspection` 5, `serverplan-name-inference` 3, `sql-grammar-escape-hatches` 9 |
| regex leaf: `lines.include` | 1 | `serverplan-name-inference` 1 |

Runtime refusal: `parse: {grammar: annotation}` in `serverplan-quality/quality-contracts-1`. It compiles; the leaf refuses per content, as in Phase 1.

### 5.4 Census, IR-level pass (every leaf accepted)

55/60 compile. The 5 that do not:

| rule | forbid | budget | relational | disabled by `when` | per-file plans | repository plans | exports | refusals | runtime refusals | compiles |
|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|---|
| `imperative-private-functions` | 3 | 0 | 0 | 0 | 2 | 0 | 0 | 1 | 0 | **refused** |
| `internal-dead-code` | 0 | 0 | 1 | 0 | 1 | 1 | 12 | 12 | 0 | **refused** |
| `public-api-triples` | 0 | 0 | 1 | 0 | 1 | 1 | 11 | 15 | 0 | **refused** |
| `serverplan-quality` | 1 | 0 | 6 | 0 | 7 | 2 | 40 | 2 | 0 | **refused** |
| `serverplan-state-tripwires` | 0 | 0 | 32 | 0 | 32 | 1 | 52 | 3 | 0 | **refused** |
| **total** | 443 | 14 | 52 | 53 | 508 | 13 | 130 | 33 | | |

Refused constructs (count; rules):

| construct | count | rules |
|---|--:|---|
| `reference_edges` (the native reference graph) has no definition over exported facts | 1 | `internal-dead-code` 1 |
| cross-file read in a per-file plan: pattern atom `calls` (the reference graph); a forbid clause is per file | 1 | `imperative-private-functions` 1 |
| order-dependent in a repository plan: JsonGroupArray over an input without a total order | 2 | `public-api-triples` 2 |
| order-dependent in a repository plan: LIMIT under an ORDER BY that is not a key (ties broken by input order) | 2 | `public-api-triples` 1, `serverplan-quality` 1 |
| order-dependent in a repository plan: LIMIT without ORDER BY | 6 | `internal-dead-code` 4, `public-api-triples` 2 |
| order-dependent in a repository plan: bag (UNION ALL) recursion: no set fixpoint for the dataflow | 8 | `internal-dead-code` 3, `public-api-triples` 3, `serverplan-quality` 1, `serverplan-state-tripwires` 1 |
| order-dependent in a repository plan: row_number() whose PARTITION BY + ORDER BY is not a key (ties broken by input order) | 13 | `internal-dead-code` 4, `public-api-triples` 7, `serverplan-state-tripwires` 2 |

Notes:

- The 8 bag-recursion refusals:
  - `imports` and `hierarchy` in dead-code and api-triples: S3's item 6;
  - `block_results` of `quality-conditional-1`: a per-path fold, but its input `contract_blocks` reads `conditional_methods`, which reads the cross-file `derived_conditionals`, so it is not file-local;
  - `project_sinks`.
- `unreachable-internal-symbols` additionally refuses `reference_edges`: the native reference graph has no definition over exported facts yet.
- `imperative-private-functions/caller` refuses the `calls` atom: a forbid is per file, and the atom reads the reference graph.

With `entryPoint = scan_project_sources`, `state-flow-1` becomes a repository clause and adds 2 `row_number` refusals and 1 bag recursion (`transitive_sinks`, `direct_sinks`); `state-projection-recovery-2` becomes a repository clause and compiles clean:

Refused constructs (count; rules):

| construct | count | rules |
|---|--:|---|
| `reference_edges` (the native reference graph) has no definition over exported facts | 1 | `internal-dead-code` 1 |
| cross-file read in a per-file plan: pattern atom `calls` (the reference graph); a forbid clause is per file | 1 | `imperative-private-functions` 1 |
| order-dependent in a repository plan: JsonGroupArray over an input without a total order | 2 | `public-api-triples` 2 |
| order-dependent in a repository plan: LIMIT under an ORDER BY that is not a key (ties broken by input order) | 2 | `public-api-triples` 1, `serverplan-quality` 1 |
| order-dependent in a repository plan: LIMIT without ORDER BY | 6 | `internal-dead-code` 4, `public-api-triples` 2 |
| order-dependent in a repository plan: bag (UNION ALL) recursion: no set fixpoint for the dataflow | 9 | `internal-dead-code` 3, `public-api-triples` 3, `serverplan-quality` 1, `serverplan-state-tripwires` 2 |
| order-dependent in a repository plan: row_number() whose PARTITION BY + ORDER BY is not a key (ties broken by input order) | 15 | `internal-dead-code` 4, `public-api-triples` 7, `serverplan-state-tripwires` 4 |

### 5.5 Classification

Classification (every relational and budget clause, and every clause inferred cross-file):

| rule | clause | form | declared | inferred | per-file stages | repository stages | exports |
|---|---|---|---|---|--:|--:|--:|
| `async-sync-delegation` | `escape-hatch` | budget | implicit | file | 0 | 0 | 0 |
| `execution-authority-bypass` | `authority-sites` | relational | file | file | 7 | 0 | 0 |
| `grammar-allowlist-burndown` | `allowlist-allowlisted_static_classes` | budget | implicit | repository | 0 | 0 | 1 |
| `grammar-allowlist-burndown` | `allowlist-legacy_allowlisted_classes` | budget | implicit | repository | 0 | 0 | 1 |
| `grammar-allowlist-burndown` | `allowlist-allowlisted_classes` | budget | implicit | repository | 0 | 0 | 1 |
| `grammar-allowlist-burndown` | `allowlist-data_value_params` | budget | implicit | repository | 0 | 0 | 1 |
| `grammar-allowlist-burndown` | `allowlist-operator_params` | budget | implicit | repository | 0 | 0 | 1 |
| `grammar-callable-constructors` | `constructor-arg` | budget | implicit | repository | 0 | 0 | 1 |
| `grammar-fragment-usage` | `captured-return` | budget | implicit | file | 0 | 0 | 0 |
| `imperative-private-functions` | `caller` | forbid | implicit | repository | 0 | 0 | 0 |
| `impl-direct-execution` | `direct-execution-lines` | relational | file | file | 3 | 0 | 0 |
| `internal-dead-code` | `unreachable-internal-symbols` | relational | repository | repository | 6 | 19 | 12 |
| `layer-violations` | `layer-0` | budget | implicit | file | 0 | 0 | 0 |
| `layer-violations` | `layer-1` | budget | implicit | file | 0 | 0 | 0 |
| `legacy-execution-surfaces` | `legacy-execution-graphs` | relational | file | file | 23 | 0 | 0 |
| `public-api-triples` | `exported-families` | relational | repository | repository | 5 | 17 | 11 |
| `serverplan-bypass` | `bypass-_execute_raw_sql` | budget | implicit | file | 0 | 0 | 0 |
| `serverplan-bypass` | `bypass-_execute_raw_sql_async` | budget | implicit | file | 0 | 0 | 0 |
| `serverplan-bypass` | `bypass-_query_raw_sql_async` | budget | implicit | file | 0 | 0 | 0 |
| `serverplan-dispatch-architecture` | `executor-provenance` | relational | file | file | 22 | 0 | 0 |
| `serverplan-quality` | `quality-contracts-1` | relational | file | file | 54 | 0 | 0 |
| `serverplan-quality` | `quality-contracts-2` | relational | file | file | 12 | 0 | 0 |
| `serverplan-quality` | `quality-contracts-3` | relational | file | file | 8 | 0 | 0 |
| `serverplan-quality` | `quality-project-taxonomy` | relational | repository | repository | 0 | 0 | 2 |
| `serverplan-quality` | `quality-conditional-1` | relational | repository | repository | 28 | 4 | 38 |
| `serverplan-state-tripwires` | `state-signatures-1` | relational | file | file | 23 | 0 | 0 |
| `serverplan-state-tripwires` | `state-flow-1` | relational | file | file | 30 | 0 | 0 |
| `serverplan-state-tripwires` | `state-projection-recovery-2` | relational | file | file | 12 | 0 | 0 |
| `serverplan-state-tripwires` | `state-flow-project-1` | relational | repository | repository | 25 | 7 | 52 |
| `serverplan-steps` | `step-contracts-1` | relational | file | file | 31 | 0 | 0 |
| `serverplan-steps` | `step-contracts-2` | relational | file | file | 24 | 0 | 0 |
| `serverplan-steps` | `step-contracts-3` | relational | file | file | 18 | 0 | 0 |
| `serverplan-steps` | `step-contracts-4` | relational | file | file | 20 | 0 | 0 |
| `serverplan-steps` | `step-contracts-5` | relational | file | file | 16 | 0 | 0 |
| `serverplan-steps` | `step-contracts-6` | relational | file | file | 16 | 0 | 0 |
| `serverplan-steps` | `runtime-reference-summaries` | relational | file | file | 7 | 0 | 0 |
| `serverplan-target-architecture` | `required-contracts` | relational | repository | repository | 13 | 5 | 8 |
| `sql-grammar-escape-hatches` | `unsafe-parameter` | budget | implicit | repository | 0 | 0 | 1 |

Not listed: 29 stage-less `partitionBy: file` relational clauses, all inferred per file.

The 8 repository clauses are the 7 relational clauses declared repository (`partitionBy` absent or `repository`; `state-flow-1` and `state-projection-recovery-2` only under `scan_project_sources`), plus the `calls` forbid. Budgets: 7 are inferred cross-file (`grammar-allowlist-burndown` ×5 with no `group_by`; `constructor-arg` and `unsafe-parameter` with a name closure that has no path partition). `captured-return` stays per file: its closure partitions by `(path, owner)`.

### 5.6 Exported relations and estimated volume

Exports per repository clause (IR-level pass; ᴰ distinct, ᴼ ordered; first 8 shown, all in `census-ir.json`):

| clause | exports | distinct | ordered | exports (columns) |
|---|--:|--:|--:|---|
| `grammar-allowlist-burndown`/`allowlist-allowlisted_static_classes` | 1 | 0 | 1 | `rows`(__path,__start,__end,path)ᴼ |
| `grammar-allowlist-burndown`/`allowlist-legacy_allowlisted_classes` | 1 | 0 | 1 | `rows`(__path,__start,__end,path)ᴼ |
| `grammar-allowlist-burndown`/`allowlist-allowlisted_classes` | 1 | 0 | 1 | `rows`(__path,__start,__end,path)ᴼ |
| `grammar-allowlist-burndown`/`allowlist-data_value_params` | 1 | 0 | 1 | `rows`(__path,__start,__end,path)ᴼ |
| `grammar-allowlist-burndown`/`allowlist-operator_params` | 1 | 0 | 1 | `rows`(__path,__start,__end,path)ᴼ |
| `grammar-callable-constructors`/`constructor-arg` | 1 | 0 | 1 | `rows`(__path,__start,__end,constructor,path,position,seed)ᴼ |
| `internal-dead-code`/`unreachable-internal-symbols` | 12 | 8 | 9 | `export_0`(__path,__start,__end,name,class_name,return_annotation,class_start,body_start,module,kind,id,fqname)ᴼ, `export_1`(__path,__start,__end,annotation,name,owner)ᴼ, `export_2`(__path,name,owner)ᴰᴼ, `export_3`(__path,name,parameter)ᴰᴼ, `export_4`(__path,__start,top,base,'',rx_replace(items,'#[^\n]*','') || ',')ᴼ, `export_5`(__path,__start,name,owner)ᴼ, `export_6`(__path,name,owner)ᴰ, `export_7`(__path,__start,array,selected)ᴰᴼ … |
| `public-api-triples`/`exported-families` | 11 | 7 | 7 | `export_0`(__path,__start,__end,name,class_name,return_annotation,class_start,body_start,module,kind,id,fqname)ᴼ, `export_1`(__path,__start,__end,annotation,name,owner)ᴼ, `export_2`(__path,name,owner)ᴰᴼ, `export_3`(__path,name,parameter)ᴰᴼ, `export_4`(__path,__start,top,base,'',rx_replace(items,'#[^\n]*','') || ',')ᴼ, `export_5`(__path,__start,name,owner)ᴼ, `export_6`(__path,name,owner)ᴰ, `export_7`(__path,__start,array,selected)ᴰᴼ … |
| `serverplan-quality`/`quality-project-taxonomy` | 2 | 0 | 1 | `export_0`(__path,__start,__end), `export_1`()ᴼ |
| `serverplan-quality`/`quality-conditional-1` | 38 | 25 | 15 | `export_0`(qualified_name,name)ᴰ, `export_1`(__path,0,'{}')ᴼ, `export_2`(__path,__end,id,sequence)ᴼ, `export_3`(__path,__start,__end,kind,text,id,parent)ᴰ, `export_4`(__path,__start,kind,parent)ᴰᴼ, `export_5`(__path,kind,parent)ᴰ, `export_6`(__path,id,parent)ᴰ, `export_7`(__path,kind,id,parent)ᴰ … |
| `serverplan-state-tripwires`/`state-flow-project-1` | 52 | 47 | 27 | `export_0`(__path,fn,id,kind)ᴰ, `export_1`(__path,fn,node,'authority')ᴰ, `export_2`(__path,fn,__path,id,callee)ᴰ, `export_3`(__path,name,kind)ᴰ, `export_4`(1)ᴰ, `export_5`(__path,dst,src)ᴰ, `export_6`(__path,fn,node)ᴰᴼ, `export_7`(__path,id,target_kind,target_text,value)ᴰ … |
| `serverplan-target-architecture`/`required-contracts` | 8 | 4 | 0 | `export_0`(__path,name)ᴰ, `export_1`(path,class,'SPTA013','fields'), `export_2`(path,class,'SPTA013','immutable'), `export_3`(path,class,'SPTA013','enum'), `export_4`(path,alias,'SPTA013','union'), `export_5`(1)ᴰ, `export_6`(1)ᴰ, `export_7`(1)ᴰ |
| `sql-grammar-escape-hatches`/`unsafe-parameter` | 1 | 0 | 1 | `rows`(__path,__start,__end,alias_from,alias_to,base,class,leaf,owner,parameter,path,position)ᴼ |

Volume (`20261022T153856-export-volume`; default options, 9,010 files). An export holds at most its leaf input's rows when it reads one input through filter/project/distinct. Multi-input exports are joins over the per-file stages, so their true row counts need S2's executor running the per-file plan.

| clause | exports | distinct | ordered | leaf inputs | leaf-input witness rows |
|---|--:|--:|--:|--:|--:|
| `quality-project-taxonomy` | 2 | 0 | 1 | 1 | 0 |
| `quality-conditional-1` | 38 | 25 | 15 | 20 | 1,419,253 |
| `state-flow-project-1` | 52 | 47 | 27 | 29 | 0 |
| `required-contracts` | 8 | 4 | 0 | 10 | 103,306 |

With `entryPoint = scan_project_sources` (`20261022T154313-export-volume-sps`, same corpus), the inputs are gated per entry point:

| clause | leaf inputs | leaf-input witness rows |
|---|--:|--:|
| `state-flow-project-1` | 29 | 4,983,106 |
| `state-flow-1` (repository under this option) | 29 | 4,983,106 |
| `state-projection-recovery-2` (repository under this option) | 28 | 266,099 |
| `required-contracts` | 10 | 103,306 |
| `quality-conditional-1`, `quality-project-taxonomy` | 20, 1 | 0 (gated off for this entry point) |

- Each clause's inputs are gated by `entryPoint`, so each variant feeds a different set of repository clauses. `state-flow-project-1` (≈5.0 M dump rows per evaluation) is the largest per-file side; its 52 exports, 47 of them distinct, are what S3 holds resident.
- `quality-conditional-1`'s 38 exports read the 20 node dumps: 1.42 M dump rows feed its per-file stages. 25 of its exports are distinct per content. 15 are ordered: they feed a `row_number`, a `LIMIT` or a bare-column read.

### 5.7 Sharing census (A3 § 8.5, all 60 rules)

Sharing census (stages recurring in more than one clause):

| measure | groups | stages in them | groups spanning rules |
|---|--:|--:|--:|
| byte-identical stage text (whitespace-normalised) | 60 | 224 | 30 |
| equal IR digest (same algebra over the same inputs) | 15 | 30 | 14 |
| equal IR shape (same algebra, inputs abstracted to their columns) | 26 | 57 | 13 |

- The byte-identical groups are mostly the tree-rebuilding stages: `edges` 13, `functions` 13, `nodes` 12, `attributes` 12, `ownership` 12, `calls` 11, `arguments` 10, `descendants` 10, `parameters` 9, `keywords` 8, `assignments` 8, `classes` 7, `nonempty_keywords` 4, `compiled` 4.
- Equal IR digest means the same algebra over the same inputs.
- Equal IR shape means the same algebra over leaves with the same columns, i.e. "one relation over all nodes, filtered per clause" (A3's conclusion). It is CSE's candidate set (planned rewrite).

### 5.8 Order-dependent constructs in repository plans (for S3)

Default options:

Order-dependent constructs left in repository plans (after the cut):

- `grammar-allowlist-burndown`/`allowlist-allowlisted_static_classes`: none
- `grammar-allowlist-burndown`/`allowlist-legacy_allowlisted_classes`: none
- `grammar-allowlist-burndown`/`allowlist-allowlisted_classes`: none
- `grammar-allowlist-burndown`/`allowlist-data_value_params`: none
- `grammar-allowlist-burndown`/`allowlist-operator_params`: none
- `grammar-callable-constructors`/`constructor-arg`: none
- `internal-dead-code`/`unreachable-internal-symbols`: declarations: row_number() (partition 1, order 1); imports: row_number() (partition 3, order 1); base_edges: ORDER BY (0 keys) LIMIT; base_edges: ORDER BY (0 keys) LIMIT; hierarchy: row_number() (partition 2, order 2); public_methods: row_number() (partition 2, order 2); class_names: ORDER BY (0 keys) LIMIT; class_names: ORDER BY (0 keys) LIMIT (refused: 11)
- `public-api-triples`/`exported-families`: declarations: row_number() (partition 1, order 1); imports: row_number() (partition 3, order 1); base_edges: ORDER BY (0 keys) LIMIT; base_edges: ORDER BY (0 keys) LIMIT; hierarchy: row_number() (partition 2, order 2); public_methods: row_number() (partition 2, order 2); traits: JsonGroupArray; slots: row_number() (partition 4, order 1); family_groups: ORDER BY (1 keys) LIMIT; family_groups: JsonGroupArray; families: row_number() (partition 3, order 1); triple_violations: row_number() (partition 1, order 2) (refused: 15)
- `serverplan-quality`/`quality-project-taxonomy`: none
- `serverplan-quality`/`quality-conditional-1`: conditional_findings_0: ORDER BY (1 keys) LIMIT (refused: 2)
- `serverplan-state-tripwires`/`state-flow-project-1`: direct_sinks: row_number() (partition 3, order 3); project_sinks: row_number() (partition 3, order 3) (refused: 3)
- `serverplan-target-architecture`/`required-contracts`: none
- `sql-grammar-escape-hatches`/`unsafe-parameter`: none

`scan_project_sources`:

Order-dependent constructs left in repository plans (after the cut):

- `grammar-allowlist-burndown`/`allowlist-allowlisted_static_classes`: none
- `grammar-allowlist-burndown`/`allowlist-legacy_allowlisted_classes`: none
- `grammar-allowlist-burndown`/`allowlist-allowlisted_classes`: none
- `grammar-allowlist-burndown`/`allowlist-data_value_params`: none
- `grammar-allowlist-burndown`/`allowlist-operator_params`: none
- `grammar-callable-constructors`/`constructor-arg`: none
- `internal-dead-code`/`unreachable-internal-symbols`: declarations: row_number() (partition 1, order 1); imports: row_number() (partition 3, order 1); base_edges: ORDER BY (0 keys) LIMIT; base_edges: ORDER BY (0 keys) LIMIT; hierarchy: row_number() (partition 2, order 2); public_methods: row_number() (partition 2, order 2); class_names: ORDER BY (0 keys) LIMIT; class_names: ORDER BY (0 keys) LIMIT (refused: 11)
- `public-api-triples`/`exported-families`: declarations: row_number() (partition 1, order 1); imports: row_number() (partition 3, order 1); base_edges: ORDER BY (0 keys) LIMIT; base_edges: ORDER BY (0 keys) LIMIT; hierarchy: row_number() (partition 2, order 2); public_methods: row_number() (partition 2, order 2); traits: JsonGroupArray; slots: row_number() (partition 4, order 1); family_groups: ORDER BY (1 keys) LIMIT; family_groups: JsonGroupArray; families: row_number() (partition 3, order 1); triple_violations: row_number() (partition 1, order 2) (refused: 15)
- `serverplan-quality`/`quality-project-taxonomy`: none
- `serverplan-quality`/`quality-conditional-1`: conditional_findings_0: ORDER BY (1 keys) LIMIT (refused: 2)
- `serverplan-state-tripwires`/`state-flow-1`: direct_sinks: row_number() (partition 3, order 3); transitive_sinks: row_number() (partition 3, order 2) (refused: 3)
- `serverplan-state-tripwires`/`state-projection-recovery-2`: none
- `serverplan-state-tripwires`/`state-flow-project-1`: direct_sinks: row_number() (partition 3, order 3); project_sinks: row_number() (partition 3, order 3) (refused: 3)
- `serverplan-target-architecture`/`required-contracts`: none
- `sql-grammar-escape-hatches`/`unsafe-parameter`: none

## 6. Risks found

- **"Compiles" depends on the leaf executor.** 40 rules refuse pattern or regex leaf constructs that Spike A's `ast.rs`/`eval.rs` lack. In clause counts (§ 5.3):
  - pattern: `precedes` 39, `pattern` text beyond a metavariable or name 42 (e.g. `dict()` 34), `value.type` 28, `params` 25, `layout` 21, parameterised `matches` 9, `follows` 5, `binds` 4, `arguments` 3;
  - regex: `content` gate 53, `lines.exclude` 38, `lines.include` 1, `lines.indentation` 5.

  These are S2's (the leaf executor). Under INVARIANTS I3 the end state is the pattern front end lowering matcher rules into the algebra over `node`/`child`/`field`. This slice keeps them as leaves, checked by the executor, as PHASE-2's S1 text says.
- **The order audit is conservative.** Candidate keys come from:
  - set reads, `DISTINCT`, group keys and `row_number` partitions;
  - the syntax tables' declared keys (`id`, `dst`);
  - per-content keys plus a path column for exports.

  An export that is distinct per content but carries no path column has no key across contents; `9e2861b87` fixed an earlier acceptance of that case. Adding the keys changed no refusal count. I2's full declarations (SCHEMA § 1.2.1: FDs, the preorder interval, cardinalities) are not yet attached to sources, so a key the audit cannot see may lift a refusal. S3's narrowing items 1–7 match the refusals listed in § 5.8.
- **File-locality is syntactic.** A stage that is per path in effect, but whose inputs pass through a cross-file filter (e.g. `contract_blocks` → `block_results`), stays on the repository side. That is exact and conservative.
- **Row order of exports.** An `ordered` export is read in path order with rows in content order. That equals the repository evaluation's order for these file-local subtrees because their leftmost leaf is path-ordered (IR.md § 7). S5's reference evaluator checks it.
- `json_each` over objects: keys sorted, as in Phase 1.

## 8. Wave 2

Base: integration `caceb656f`; merged integration `8960a5db7` (S6's leaf constructs) at `5b9c25823`. Runs under `~/rerail-bench/facts/phase-2/s1-ir/runs/`. No timing gate is owned by S1; every number below is a count or an exactness result (load 9–35 during the runs, irrelevant to them).

### 8.1 Results

| Item | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| Rewrites R6–R11 registered (S2 retires `subst.rs` with them) | each with side conditions in code, justification, differential test, mutant | 6 rows in `REWRITES.md`, `rerail_facts::rewrite`; tests `rewrite::tests::rewrite_*` (random preorder trees / relations, reference evaluator); mutants 16–21 each fail exactly their test; clean 26/26 | **pass** | command: `FACTS_MUTANT=<n> cargo test -p rerail-facts --features mutants --lib` |
| R1–R5 vs the new rows | stated | R2 = column-level case of R6 (view inlining); R4 = R8 (projection pruning) at the export boundary; R1, R3, R5 are distinct | — | `REWRITES.md` |
| `ir::equi_keys`, `ir::semi_join` for S3 | merged and used by S3 | `9eba4764f`; differential: 8 semijoin shapes × 300 seeds, equi-key split × 200 seeds; found S3's `NOT IN` NULL bug (fixed by S3); S3 switching after its NARROWINGS commit | **delivered; use by S3 pending** | — |
| Order constructs in repository plans (decider ruling) | `total: proven | checked` on the IR node, in EXPLAIN and census | `ir::Total {InputOrder, Proven, Checked}` on `WindowFn`, `Sort`, `AggCall`; classifier marks instead of refusing; proofs K1, K2, K3/K4 (pins), K6; mutant 10. IR-level census: 23 constructs, **2 proven, 21 checked** (with S6's leaf executor: 15, 2 proven, 13 checked) | **pass** (mark); proofs limited by input keys, § 8.3 | `20261022T214030-census` |
| Physical EXPLAIN per file | per-file text, stable | `facts-a explain --physical --root|--file [--audit]` over subset-8: 1,150 files, 199,291 stage plans, 50 clauses each with exactly one plan text across all files, two runs byte-identical (sha256 `560fc844…`) | **pass** | `20261022T213002-explain-physical` |
| Inner-scan audit over physical plans | report | 1,233 inner scans = 2 stage plans in every file: `quality-conditional-1/block_results` (correlated NOT EXISTS whose item 0 scans `contract_blocks`), `legacy-execution-graphs/function_symbols` (correlated derived table over `classes`). Audit fix: correlation now followed through operators above a subquery's pipeline (wave 1 printed 0 because of it) | **found 2** (S2 informed) | same |
| Pattern front end for S4's `raw_strings_*` | the three inputs lowered over `node` + `string_value` | lowered (`front/pattern.rs`, `InputDef.algebra`); differential `facts-a pattern-check` vs `ast.rs`: raw_strings inputs **0 mismatches** in every run: subset-8 ≤ 600 nodes default config (294 files, 224 rows), ≤ 1,500 nodes `scan_project_sources` (608 files, 15,138 rows), oracle/no-source config (608 files, 6 rows), l2070 sample ≤ 3,000 nodes (502 files, 34 rows) | **pass, weak on value tests**: state-tripwires' `raw_strings_0..2` produced 0 rows wherever the value test is active (gates false on small files) | `20261022T214120-…`, `20261022T214209-…`, `20261022T215716-…`, `20261022T215735-pattern-check-raw-strings-l2070` |
| Census with S6's leaf executor | numbers with run dir | **55/60** rules compile (wave 1: 20/60). Refusals: 5 bag recursions (`public-api-triples`, `serverplan-quality`, `serverplan-state-tripwires`), 4 `binds` (S6), 1 `no such table raw_module_names` (follows from a refused input). IR level: 55/60 (`reference_edges`, `calls`, 8 bag recursions). Pattern front end lowers 65/622 pattern inputs | **pass** (G2.1 as written) | `20261022T214030-census` (`census*.md/json`) |
| Export volume | current | unchanged from wave 1: default 100 exports (76 distinct, 43 ordered), leaf witness rows `quality-conditional-1` 1,419,253, `required-contracts` 103,306; `scan_project_sources` 166 exports, `state-flow-project-1`/`state-flow-1` 4,983,106, `state-projection-recovery-2` 266,099 | — | `20261022T214131-export-volume-default`, `20261022T214149-export-volume-sps` |

### 8.2 Interface changes

- `ir`: `FileSource::Core(CoreRel)` (core relations of one content, SCHEMA § 1.2/1.3 incl. `string_value`, `string_value_unknown`, `file`, `syntax_error`); `InputDef.algebra: Result<Lowered, String>`; `Lowered { rows, refuse_if }`; `Total` on `WindowFn.total`, `Rel::Sort { total }`, `AggCall.total`; `Key`, `equi_keys`, `SemiJoin`, `semi_join`, `conjuncts`, `conjoin`, `cols_read`, `reads_outer`, `rel_reads_outer`, `has_subquery`, `each_subexpr`, `rel_exprs`, `children_mut`.
- `classify::ClauseReport.order: Vec<OrderMark>`; order constructs no longer refused (bag recursion and aggregate bare columns still are).
- `front::rule::LeafCheck::known_field` (default `None`); `front::pattern::lower_input`.
- `reference::Evaluator::with_content(scan, path, source)` (`source_text` over one content; S5's file, small addition).
- `rewrite::{inline_views, fuse_union, prune_columns, reach_to_interval, order_observed, or_probe, OrProbe::matches, share_selection, subtree_closed}`.
- `phys::audit` follows correlation through non-pipeline operators.
- facts-a: `pattern-check`, `explain --physical`; visibility only: `sql/engine.rs` plan structs `pub(crate)` (`b099448a5`), `repo::compile_batch6` `pub(crate)` (S5 replaced it by `compile_checks` on its branch; keep S5's version on merge); facts-b `gen/norm.rs` patched mechanically for `Total` (S3 owns it).

### 8.3 Risks and limits

- **R9/R11 do not fire on the current pack.** Their side conditions need the tree from I2's declared structure (core `node`), but the dump clauses rebuild the tree in SQL from matcher inputs with text ids; the pattern front end refuses the dump inputs' ancestor captures under relations (180 inputs: "metavariable under any/not/relation"), so `subst.rs` cannot be retired by the logical rewrites alone (S2 reports the per-clause gap from the generic executor's speed instead).
- **Pattern front end coverage 65/622.** Next constructs by count: captures under relations (180), stop rules (150), `inside` with `field` (42), `precedes` (31), value tests off the root chain (27).
- **Pattern-check is slow**: the naive reference re-scans the core relations per correlated subquery (O(n²) per relation search); runs are limited to files ≤ 600–3,000 nodes. The value-test/`\N{...}` path is unexercised by rows on the corpus sample.
- **Input keys.** Native guarantees one row per witness node (identity (path, node id), not carried by the row); (`__path,__start,__end`) is not a guaranteed key (S4: 492 same-kind collisions, all anonymous `yield`; 0 for named kinds, measured not guaranteed). No input key is declared, so 21 of 23 order constructs stay `checked`.
- **Not run: `just facts-gate --mutants` on this branch** (budget). Mutants 10 and 16–21 and the lib tests were run individually (above); S5 merged `f1edf7edc` and runs the gate on its branch.

## 7. Resume notes

- **Commits** (local): wave 1 as listed in git (`43bd9c048` … `9e2861b87`); wave 2:
  - `b099448a5` engine plan structs `pub(crate)` (visibility only);
  - `9eba4764f` `ir::equi_keys`, `ir::semi_join`, tests;
  - `0ea181a84` `Total` marks + proofs K1 K2 K3/K4 K6 + mutant 10; `FileSource::Core`; pattern front end; `pattern-check`; reference `source_text`;
  - `a40a2bcbb` rewrites R6–R11, registry, mutants 16–21;
  - `f1edf7edc` physical EXPLAIN per file, audit fix;
  - `dab01fd35` census pattern-front-end counts;
  - `5b9c25823` merge of integration `8960a5db7`.
- **Interfaces:** § 8.2; `docs/facts/REWRITES.md`; `facts/src/{ir,rewrite,phys,front/pattern}.rs`.
- **Temporary:** `.worktrees/s1-census` (detached, s1-ir + s6-leaf) and `target-census/` were used for the census; removed.
- **Next, in order:**
  1. Run `just facts-gate --mutants` on this branch; add mutant 10 to the gate's fixed list (S5 told).
  2. Pattern front end: captures bound under relations (ancestor captures `any[inside{all[X, pattern $A], stopBy: X}, …]`) and stop rules, so the dump inputs lower to algebra over `node`; then origin tracing through text ids (`kind||':'||start||':'||end` is injective given node uniqueness) so R6–R9/R11 fire on dump clauses — the route to deleting `subst.rs` by rewrites rather than by executor speed.
  3. A cheaper check for `pattern-check` (memoised reference or S2's executor over `InputDef.algebra`) and a corpus run that exercises value tests with rows.
  4. Input span keys only with a per-content check that refuses on violation (S4's recommendation), then re-count proven vs checked.
  5. S3: confirm `equi_keys`/`semi_join` adoption; definitions of `reference_edges` and `calls` over exported facts.
