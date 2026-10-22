# Phase 2, S4: fact schema v1, extractor, store format

Status: **acceptance items delivered; S3's schema-level export (§ 3.1 of SCHEMA) not started.** Branch `Moti-Velz/facts-engine-s4-schema`, worktree `~/orca/workspaces/rerail/facts-engine/.worktrees/s4-schema`. Raw runs: `~/rerail-bench/facts/phase-2/s4-schema/runs/<timestamp>-<label>/`. Each holds `command.txt` (command and commit), `uptime-{before,after}.txt`, `stdout.txt`, `stderr.txt` (`/usr/bin/time -l`) and `results.json`.

## 1. Setup

| Item | Value |
|---|---|
| Base | integration `Moti-Velz/facts-engine` merged at `694bd698f` (S1's crate move) |
| Build | `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target cargo build --release -p facts-a` |
| Corpus | `~/rerail-bench/nera-data-layer-2b1504ad` (9,010 `.py` files, 137.9 MB, 28,230,245 nodes, of which 18,505,286 named; no file has syntax errors); recordings `fixtures/host/data-layer-port/batch-06-{recording,probes}.json` |
| Policies | `packs/data-layer/candidates/rules/*.json`, unmodified |
| Production change | Visibility only, in `native/src/code_rule_dsl`. Modules `embed`, `embed::segments`, `literal`, `module_path`, `origin` and `scope` are now `pub`. Items now `pub`: `ScopeTable`, `ScopeTable::annotations`, `ScopeTable::build_in`, `Annotation` (`start`, `end`, `tree`), `Reading`, `BuildError`, `SubTree`, `SubDoc` (`tree`), `SubSource`, `SubLang`, `SubNode`, `Origin`, `FileRanges` (`primary`) and `ModulePath`. No behaviour change. |
| Machine | darwin 25.6 arm64, 4 workers. **The 1-minute load was 14–21 during every timing run**: other slices were running. Under PHASE-1 § Measurement these timings are **untrusted**. Byte counts and exactness do not depend on load. |

## 2. Results

| Id | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| SCHEMA v1 committed first; S2 and S3 notified | first commit | `04c3c957b` (v1). Re-steered to views at `ed0459c71` (user direction). Normative view SQL from `275b016d1`. S1, S2 and S3 messaged. S1 confirms that all 15 view texts parse and lower through `rerail_facts::front::sql`. | pass | — |
| Views ≡ policy stage texts, whole corpus | 0 differences | **0 multiset differences** over 9,010 files: 11 stage texts × the 15 clauses holding them, 699,620 file-checks, 236,564,067 rows. Row order is also identical, except `ownership`, whose order no stage specifies (fixpoint level order). 0 `kind:start:end` collisions, 0 node-set mismatches, 0 failures. | **pass** | `20261022T153458-derived-check-shard0`, `…153628-…shard1`, `…153810-…shard2`, `…153944-…shard3` |
| Per-file cost: views and parser facts vs A4 (parse + extract p50 0.53 ms) | reported | Untrusted load; median of 3 runs. Parse + extract: p50 0.68 ms, p99 13.2 ms. **Views via Spike A's interpreter**: p50 2.72 ms, p95 23.7 ms, p99 74.5 ms, 66.5 s summed. `string_type_definition`: p50 0.055 ms, p99 1.18 ms, 1.14 s summed, 5,521 definitions. | reported | `20261022T154339-views-cost`, `…154400-…` (median), `…154422-…` |
| G2.2, 10th rule (`quality-contracts-1`) | recordings and repository exact | Replay: **1,259/1,259 calls, 0 FP, 0 FN, 0 failures** (was 6 failures). Repository: **173/173, 0 FP, 0 FN, 0 failures** (was 38). | **pass** | `20261022T152906-replay`; `20261022T152941-repo/parity.json` |
| A3 differential after the change | no panic, parity unchanged | `FACTS_A3_CHECK=1 repo`: exit 0, 173/173, 0 FP, 0 FN, 0 failures | pass | `20261022T153104-a3check-repo` |
| Fact files v1: bytes, write and read ms | measured decision | v1 (zstd 3): 186.4 MB (v0: 1,048.4 MB, 5.6× smaller). 1.35 bytes per source byte, 6.6 B per node. p50 9,574 B per file, ratio to source p50 1.58 (v0: 8.29). Write p50 0.109 ms, p99 1.8 ms; read p50 0.080 ms, p99 1.7 ms. All 9,010 files round-trip exactly (8,957 distinct contents). | **decided** (§ 5.2) | `20261022T154504/154516/154527-factfile-bench`, `20261022T154821-store-v1` |
| Cross-spike reader | `inspect` reads `.facts` and `.xfacts` | v1 store: 8,957 `.facts` read, 0 failures. Old store: 8,957 v0 `.facts` plus 19,772 Spike B `.xfacts` read, 0 failures. | pass | `20261022T154829-inspect` |

## 3. How measured

- **Differential.** `facts-a derived-check --root <corpus> --threads 4 --shard k/4 --out-dir <run>` (`derived_check.rs`).
  - Per file, it evaluates the SCHEMA views with Spike A's SQL engine (`views.rs` reads the `-- view:` lines of `SCHEMA.md`).
  - For every batch-6 clause that holds a target stage's shared text (the text most clauses hold under that stage name, listed in `results.json` → `shared_texts`), it runs the reference. The clause's dump inputs are built with its gate and ancestor predicate replaced by `{"all": []}` (or the clause's syntax-spec tables are used), and then only the stages the targets read, by the SQL engine.
  - Each view row is rendered into the stage's columns: `n.*` from the clause's `nodes` row, ids as `kind:start:end` or integers, text from the source. Rows are stored with the stage's CTAS affinities and compared as multisets of typed values. Order is compared after sorting by the clause's `nodes` rank of the key node.
  - Pass condition: `bag_mismatch_total = 0`, `nodes_mismatch = []`, `failures = []`.
  - One `--shard 0/4` run (2,253 files) takes about 95 s with 4 threads.
- **Cost.** `FACTS_RUNS=… FACTS_BIN=./target/release/facts-a spikes/facts-a/tools/run-repo.sh views-cost derived-check --root <corpus> --threads 4 --no-check --typedefs`, three runs.
  - `run-repo.sh` is extended, not forked: `run-repo.sh <label> <facts-a args>` runs any subcommand under the protocol, and no arguments keeps the old gate run.
  - Per-file `parse_ns + extract_ns` (A4's measure), `views_ns` and `typedefs_ns` are in `per-file.tsv`; per-view seconds are in `results.json` → `views`.
- **Parity.** Replay: `./target/release/facts-a replay --recording …recording.json --recording …probes.json --output <run>/results.json`. Repository: `run-repo.sh` with no arguments, which also runs `repo-compare`. A3: `FACTS_A3_CHECK=1 facts-a repo --threads 4`.
- **Fact files.** `run-repo.sh factfile-bench factfile-bench --threads 4`, three runs.
  - Per file and layout, in memory: bytes, write ns (encode plus compress) and read ns (decompress, decode, derive `parent`/lines, index). Each is round-trip checked against the built facts.
  - Named-only layouts cannot be decoded without a re-parse, so their `read_ns` is the header and column read only.
  - Then `facts-a store --threads 4 --store ~/rerail-bench/facts/phase-2/s4-schema/store-v1` (round trip through disk) and `facts-a inspect` over both stores.

## 4. Exactness artifacts

- `…-derived-check-shard{0..3}/results.json`: per (stage, clause) files, rows, `bag_mismatch`, `order_mismatch`, examples; `extra_nodes_beyond_n` lists the `step-contracts-1..6` clauses, whose gate-true node set is every named node (§ 5.1).
- `20261022T152941-repo/parity.json` and `20261022T152906-replay/results.json` (empty `mismatches`).

## 5. Design notes

### 5.1 Schema v1 (`docs/facts/SCHEMA.md`)

- **§ 1, core.** `node`, `child`, `field` (with `rank`), `identifier`, `line` as SQL relations, with the declared structure INVARIANTS I2 asks for (keys, functional dependencies, preorder and interval, cardinality; § 1.2.1).
- **§ 1.3, parser facts.** `string_value`, f-string parts and `string_type_definition`.
- **§ 2, Python-derived relations: 15 views.**
  - Generic: `py_node` (= `N`, the named nodes minus 6 kinds), `py_field`, `py_owner`, `py_named` and `py_owner_named` (the `step-contracts` clauses' node set is all named nodes, and their `ownership` chains through `string_content`), `py_function`, `py_class`, `py_method`, `py_attribute`, `py_call`, `py_parameter`, `py_assignment`, `py_decorator`, `py_import_identifier`.
  - Rule-private: `py_context`. Its constants are state-flow's. The path conjunct `provider_helper` is left to the binder, because facts are path-free.
- **Found by the differential, and folded in.**
  - Ownership chains follow `N`-parents: an `escape_interpolation` under `string_content` is a root.
  - The syntax spec's ownership chains through all named nodes.
  - The `step-contracts` raw dumps do not exclude the six kinds.
- **§ 2.4, `special_attributes` and `return_kinds`.** They are views composed of the § 2.2 views, the stage texts as written, and state-flow's three `raw_strings_*` matcher inputs, compiled by S1's pattern front end over `py_node` and `string_value`. This schema declares the leaf and the composition and adds no hand translation of a matcher input (I3). Binding note for the rewrite: both stages join **by name across the node set**, so selection sharing by semijoin does not apply to them.
- **§ 3.** S3's requests are recorded: § 3.1 schema-level from native's call summary, § 3.2 plan-level namespace `x.<rule>.<clause>.<rel>` keyed by plan digest. The native call summary is keyed by (content, module path), not by content alone (§ 7).

**Cost of the views as interpreted today.** In the median run they take 66.5 s summed over the corpus, p50 2.7 ms per file. That is more than the 34.9 s of shared stage text they replace in A3 (§ 8.5). By view (s): `py_import_identifier` 7.7 (imports × identifiers range join), `py_owner_named` 7.3, `py_context` 6.9, `py_owner` 6.3, `py_node` 5.8 (window), `py_named` 5.4, `py_decorator` 4.3, `py_parameter` 4.1, `py_field` 3.7, `py_assignment` 3.4, `py_method` 3.0, `py_call` 2.5, `py_class` 2.1, `py_attribute` 1.3, `py_function` 1.2. This is Spike A's row-materialising interpreter running the views over all of `N`, under untrusted load.
- The G2.3 lever needs S2's columnar operators and S1's rewrites:
  - `py_owner` is an interval/parent-chain probe, not a fixpoint;
  - `py_import_identifier` is an interval probe on the `identifier` postings;
  - `py_node.ordinal` is a sibling rank.
- If these are not lowered, binding the views per clause would cost more than the stage texts do today. S2 should measure before switching.

### 5.2 Fact files v1

Median of 3 runs (untrusted load; bytes exact).

| layout | total MB | B/source B | B/node | p50 B | write p50/p99 ms | read p50/p99 ms |
|---|--:|--:|--:|--:|--:|--:|
| v0 (Phase 1) | 1,048.4 | 7.60 | 37.1 | 50,984 | 0.043 / 0.53 | 0.047 / 0.82 |
| v1 raw | 532.3 | 3.86 | 18.9 | 26,422 | 0.034 / 0.48 | 0.058 / 1.29 |
| v1 + zstd 1 | 193.0 | 1.40 | 6.8 | 9,628 | 0.093 / 1.47 | 0.084 / 1.70 |
| **v1 + zstd 3 (chosen)** | **186.4** | **1.35** | **6.6** | **9,574** | **0.109 / 1.81** | **0.081 / 1.71** |
| v1 + zstd 9 | 178.0 | 1.29 | 6.3 | 9,268 | 0.306 / 5.32 | 0.083 / 1.74 |
| v1 named-only | 450.8 | 3.27 | 16.0 | 22,702 | 0.032 / 0.50 | needs a re-parse |
| v1 named-only + zstd 3 | 176.6 | 1.28 | 6.3 | 8,936 | 0.097 / 1.52 | needs a re-parse |

- **v1 columns.**
  - `kind` is a u16 and `field` a u8 file-string index.
  - `parent`, `start_line`, `end_line` and `line` are derived on read: `parent` as the innermost open subtree in preorder, lines from the source.
  - Saved: 49% raw.
- **Anonymous nodes: stored.**
  - They are 34% of nodes, but dropping them saves only 15% raw and 5.3% compressed.
  - Every read would then become a parse: parse + extract p50 0.70 ms against 0.08 ms.
  - `py_field`'s rank-0 test and the matcher read them.
- **Compression: zstd level 3.**
  - Against v1 raw it is 2.9× smaller for +0.022 ms read at p50.
  - Level 9 gains 4.5% for 2.8× the write time.
  - Read stays about 8.6× cheaper than re-parsing.

## 6. `string_type_definition`

Decision: **schedule it as a fact, extracted by calling native**. It is neither a port nor a narrowing.

Evidence:
- Native already contains Ruff 0.16.9's classifier: `native/src/code_rule_dsl/scope/python/typedefs.rs`, 1,132 lines.
- `ScopeTable::annotations` holds every string type definition with its parsed tree, and native's matcher answers `parse: {grammar: annotation}` from it (`matcher/parse.rs::tree_at`).
- Calling it needs visibility only (§ 1). `typedefs.rs` builds the table with the default reading and an unbounded budget, then copies each tree in preorder with its native host ranges (`Origin::of_node(..).primary()`). `Facts::typedefs` computes this lazily per content.
- `ast.rs` answers `ParseGrammar::Annotation` from this fact, which replaces the refusal. S2 agreed to the edit.
- Result: the 6 replay calls and 38 repository failures go to 0, with every other finding unchanged (§ 2).
- Cost: p50 0.055 ms per file, 1.1 s over the corpus.

Per-content claim and its condition:
- The table reads one file and never crosses files.
- It is built with no module path. Native qualifies `Relative`/`Definition` import bindings through the module path, and such a binding can only resolve to `typing`, `_typeshed` or `typing_extensions` when the file's module path starts with one of those names. The corpus has none. A binder must refuse otherwise (SCHEMA § 1.3).

## 7. Resume notes

- **Commits** (local, not pushed):
  - `04c3c957b` SCHEMA v1;
  - `ed0459c71` views re-steer;
  - `5f6b7ec12` merge of integration;
  - `275b016d1` differential, `string_type_definition`, fact files v1, `run-repo.sh`;
  - `c9b341b10` typedefs timing and § 2.4;
  - final commit: zstd 3 as v1, SCHEMA § 5, this report.
- **Merge note.** S2's `4b14dfe7e` removes `Db.regexes`. When integration carries it, drop the `regexes:` initialiser in `derived_check.rs` (reference `Db`) and `views.rs::evaluate` (S5 did the same in its merge).
- **Next, in order.**
  1. Re-run the three cost rows and the fact-file bench on an idle machine (load < 8).
  2. S3's § 3.1: extract native's python call `Summary` by calling `calls::summary::Summary::extract(root, Some(&ScopeTable::build_in(…, module)), module, has_errors)` (visibility: `calls::summary`, `calls::names::Name`, a `ModulePath` constructor). Key it by (content, module path). The module path depends on package structure (`__init__.py` presence), so it is membership-dependent: tell S3.
  3. Move `.xfacts` (Spike B's writer, S3-owned) to v1 when S3 extends the exported relations.
  4. Add `string_value(id, value)` as a core SQL relation in `views.rs` once S1's pattern front end compiles state-flow's `raw_strings_*` (SCHEMA § 2.4).
  5. Debug-build structural checks for § 1.2.1's declarations.
- **Not done.** Views for `special_attributes`, `return_kinds` and `direct_events` are specified as a composition (SCHEMA § 2.4) but not executed. They need S1's pattern front end for the three `raw_strings_*` inputs.
