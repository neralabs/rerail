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

## W2. Wave 2

Base: integration `caceb656f`, plus S1's `0ea181a84` (pattern front end) merged locally. Machine: shared by six agents. The 1-minute load was 20–71 during every run below, so all times are **untrusted**. Rows, bytes and exactness do not depend on load. Run dirs are under `~/rerail-bench/facts/phase-2/s4-schema/runs/`.

### W2.1 Results

| Id | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| Unresolved call-graph views (SCHEMA § 3.3: `cg_scope`, `cg_def`, `cg_call`, `cg_import`, `cg_refusal`), content-keyed, agreed with S3 | 0 differences against native | **0 differences** over 9,010 files:<br>• `cg_def`, 194,281 rows, against native `Summary::extract` defs (name, kind, span);<br>• `cg_import`, 235,250 rows, against native scope-table import bindings;<br>• `cg_call`, 1,056,492 rows, and `cg_import.scope`, against native's context rule.<br>`cg_refusal`: 0 rows; a crafted file shows both refusal constructs firing exactly where the raw text differs from native. | **pass** | `20261022T211538-cg-differential` |
| Native call summary per content (SCHEMA § 3.4, `cgn_*`, S3's ask): symbolic module path, real one substituted on the membership side | 0 differences after substitution | **0 differences, 0 refusals** over 9,010 files (variants: module 6,695 / init 482 / unpackaged 1,833). Covers 13 relations, both `summary` and `as_references` (no links, and batch 6's links). Export: 8,957 contents × 3 variants, 64.6 MB zstd JSONL. | **pass** | `20261022T212451-cg-native` |
| `string_value` as a core fact; § 2.4 views runnable (`string_values`, `special_attributes`, `return_kinds`, `direct_events`) | 0 differences | See W2.2 | see W2.2 | `…-derived-check-w2-shard{0,1}of2` |
| Costs, A/B, 3 runs each, back-to-back under `bench-lock` | reported | Table W2.3 | reported | `20261022T2125…–2128…` |
| `inspect` reads v0, v1 and `.xfacts` | 0 failures | v1 store: 8,957 `.facts`. Old store: 8,957 v0 `.facts` and 19,772 `.xfacts`. 0 failures. | pass | `20261022T213340-inspect` |
| `just facts-gate` (after milestone 1: native visibility, views, derived-check) | 13/13 | **13/13 pass**, 671 s, load 16.6. `views-derived`: bag 0, including the § 2.4 compositions. | pass | `~/rerail-bench/facts/phase-2/s5-gate/runs/20261022T205508-gate` |

### W2.2 § 2.4 compositions and `string_value`

**`string_value`.**

- `string_value(id, value)` and `string_value_unknown(id, named_escape)` are core relations (SCHEMA § 1.2/1.3), built by `views::string_values` from `pyvalue::decode`.
- `views::core_rows` provides them, with the rest of the core, as `rerail_facts::reference` reads them. S1's `pattern-check` now calls it instead of its own copy.
- S1's `value` and `DECODED` lowering reads them.

**Composition differential (`facts-a derived-check`).**

- Reference side: the clause's own stages, with the gate forced `true` and the matcher inputs gate-free (`gate_free`) from the leaf executor.
- View side: the same stage texts, where each stage a § 2.2 view stands for is replaced by the rendered view, and the matcher inputs come from S1's lowered algebra evaluated over `core_rows`.
- The inputs themselves are also compared, as rows `input <name>`.

**Bound.** S1's reference evaluator is naive and clones a core relation per scan. A first unbounded shard (1,126 files) hit the 900 s cap (`20261022T213335-derived-check-shard0of8`). On a 113-file sample the inputs cost 969 s and every other check under 15 s. So `--input-max-nodes 3000` evaluates the lowered inputs for files of at most 3,000 nodes (p50 is 1,455). Larger files take the leaf inputs on both sides and are counted as `inputs unchecked`. For those files the composition is still checked; the inputs' own differential is S1's `pattern-check`. Full-corpus results: W2.4.

### W2.3 Costs (median of 3, untrusted load 20–45)

- A is `facts-a-rows-bd01951bc`: the pre-columnar engine at integration's S4 merge, 15 views.
- B is `facts-a-s4-145cd8f96`: S2's columnar layer at `caceb656f` plus this wave, 20 views.
- Both run `derived-check --no-check --typedefs --threads 4`.

| measure | A rows | B columnar |
|---|--:|--:|
| parse + extract p50 / p99 ms | 0.77 / 14.7 | 0.75 / 14.6 |
| `string_type_definition` p50 / p99 ms; sum s | 0.061 / 1.26; 1.28 | 0.061 / 1.27; 1.25 |
| core tables p50 / p99 ms (B only) | — | 0.20 / 3.0 (3.3 s summed) |
| `string_value` + `string_value_unknown` p50 / p99 ms (B only) | — | 0.012 / 0.20 (0.23 s summed) |
| the 15 wave-1 views, summed s | 68.7 | 42.0 |
| all views per file p50 / p99 ms | 2.67 / 81.8 (15 views) | 2.51 / 75.4 (20 views) |
| process user s / peak RSS MB | 88.2 / 625 | 90.1 / 925 |

Per view, s (A → B): `py_node` 6.4 → 4.3, `py_field` 4.0 → 2.5, `py_owner` 6.8 → 5.7, `py_named` 6.1 → 2.9, `py_owner_named` 7.8 → 6.3, `py_function` 1.3 → 1.3, `py_class` 2.4 → 0.8, `py_method` 3.3 → 1.8, `py_attribute` 1.4 → 0.2, `py_call` 2.7 → 0.4, `py_parameter` 4.5 → 1.6, `py_assignment` 3.6 → 0.4, `py_decorator` 4.8 → 1.3, `py_import_identifier` 8.7 → **10.3 (slower)**, `py_context` 4.9 → 2.2.

The new views cost 19.5 s summed: `cg_scope` 2.8, `cg_def` 2.7, `cg_call` 4.8 and `cg_import` 5.7 (both correlated `max` subqueries per row: an interval-containment probe, I5), and `cg_refusal` 3.5 (`rx_matches` on every identifier).

- Runs, A then B, interleaved: `212540-rows`/`212604-columnar`, `212628`/`212651`, `212715`/`212737`.
- S2 was told these run dirs, so the per-view columnar costs are not measured twice.

**Fact files v1** (`factfile-bench`, 3 runs `212800`, `212812`, `212825`): the bytes are identical to wave 1 (zstd 3: 186.4 MB, 1.35 B per source byte, p50 9,574 B). Timings, write p50/p99 and read p50/p99: v1+zstd3 0.122/2.12 and 0.091/2.02 ms; v0 0.047/0.62 and 0.052/1.00 ms. Round-trip failures: 0. `string_value` is not stored. It is recomputed on read from `node` and the source, at p50 0.012 ms.

### W2.4 Full-corpus differential (wave 2)

Filled from `…-derived-check-w2-shard{0,1}of2` below.

## 7. Resume notes

- **Commits** (local, not pushed):
  - wave 1: `04c3c957b`, `ed0459c71`, `5f6b7ec12`, `275b016d1`, `c9b341b10` and the final wave-1 commit;
  - wave 2: `85548d354` (string_value, § 3.3 views, `--cg`, § 2.4 compositions), `145cd8f96` (`cg-native`, § 3.4), and the last commit on the branch (merge of S1's `0ea181a84`, S1's lowered inputs in the composition, `core_rows`, this report).
- **Interfaces:**
  - SCHEMA § 1.2/1.3 (`string_value`, `string_value_unknown`), § 3.3 (`cg_*` views), § 3.4 (`cgn_*` export and its substitution rule);
  - `views::core_rows`;
  - `cgnative::{compact, substitute}`, the reference S3 mirrors as IR;
  - the export file `~/rerail-bench/facts/phase-2/s4-schema/cgn-export.jsonl.zst`.
- **Native:** visibility only: `calls`, `calls::summary` (Summary/Def fields, `extract`, `as_references`), `calls::contracts::{Key, compile}`, `calls::summarize`, the `scope` import bindings, `NodeRef`, `ModulePath` (`segments`, `names`, `dotted`, `stem`, `qualified`, `last`, `in_package`) and `model::python_module_path_on_disk`.
- **Merge notes.**
  - Integration `8960a5db7` (S6) is not merged yet. Merge it next: it touches `main.rs`, `leaf.rs` and `eval.rs`.
  - `derived_check.rs` still depends on `subst::DumpGroup`. When S2 deletes `subst.rs`, the dump with a forced gate needs another source, such as S1's plans.
- **Next, in order:**
  1. Re-run W2.3 and the wave-1 cost rows on a quiet machine (load < 8).
  2. The lowered inputs for files above 3,000 nodes, once a non-naive evaluator for `Plan<FileSource>` exists (S2), or S1's reference evaluator stops cloning a core relation per scan.
  3. `cg_call`/`cg_import` as interval probes (S2's operator) and `cg_refusal` without a regex per identifier.
  4. `py_import_identifier` regressed under the columnar layer (8.7 → 10.3 s): give S2 the plan.
  5. Debug-build structural checks for § 1.2.1.
  6. `.xfacts` to v1 when S3 settles its export set.
