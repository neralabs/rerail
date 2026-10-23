# Phase 2 / S6: leaf executor constructs (wave 2)

Slice: the matcher constructs of Spike A's leaf executor (`spikes/facts-a/src/ast.rs`, new modules under `spikes/facts-a/src/ast/`) and the regex-clause evaluation. Branch `Moti-Velz/facts-engine-s6-leaf`, from `caceb656f`.

- With the leaf executor, **55/60 rules compile clean**: the same number as the IR-level census (S1). The 5 that still refuse are the 5 the IR level refuses. Wave 1 had 20/60.
- `replay.ts --engine facts` is exact (0 FP, 0 FN, 0 failures) for **batches 2, 3 and 6**. Every remaining difference in batches 1, 4 and 5 is named below, and none of them is a leaf construct.

## 1. Setup

| item | value |
|---|---|
| Commits | `39d33188c` (milestone 1), `8bdf55063` (milestone 2), both gated |
| Build | `cargo build --release -p facts-a` in the slice worktree (`target/release`); the gate builds its own `target/facts-gate` |
| Machine | darwin arm64, shared with the other slices. Load 8–31 during the runs (noted per run). No timing row is owned by S6, and nothing below is a time |
| Corpus | data-layer rules `packs/data-layer/candidates/rules` (60). Replay recordings `fixtures/host/data-layer-port/batch-0N-*.json`, the same recording and probe sets as `/tmp/dl60/replay-all.sh` |

| G2.2 batch 6 stays exact | 0 FP/FN/failures | `facts-gate` 13/13 checks pass on both milestone commits (`39d33188c`: load 20.7, 836 s; `8bdf55063`: load 19.5, 498 s; each in a detached worktree at the commit). Replay b6: 1259/1259 calls exact on every build | **pass** | `20261022T211447-gate-39d33188c`, `20261022T212846-gate-8bdf55063`, `20261022T211917-replay-g4` |

| id | target | measured | pass/fail | run dir (`~/rerail-bench/facts/phase-2/s6-leaf/runs/`) |
|---|---|---|---|---|
| G2.1 (leaf executor) | ≥ 55/60 clean, or each refusal named | **55/60**. The 5 refused are named in § 2.2; they are the IR-level 5 | **pass** | `20261022T210119-census-g2`, `20261022T211915-census-g4` |
| G2.2 batch 6 stays exact | 0 FP/FN/failures | `facts-gate`: 13/13 checks pass on the milestone-1 tree. Replay b6: 1259/1259 calls exact on every later build | **pass** | `20261022T210204-gate-m1` (the milestone-1 tree, before `git commit`), `…-gate-39d33188c` (the commit, separate worktree), `20261022T211917-replay-g4` |
| Parity b1–b5 (replay) | per-batch table | § 2.1 | b2, b3 exact; b1, b4, b5 open (named) | `20261022T211917-replay-g4` (latest), `20261022T205537-replay-g1b` (first) |
| Parity b1–b5 (suite) | per-batch table vs the `2b1504adc` oracle | **not run**: `suite.ts --engine facts` runs `facts-a repo`, which compiles only `BATCH6` (`spikes/facts-a/src/repo.rs:38-52`, `compile_batch6`). The oracles exist at `/tmp/dl60/suite-b{1..5}.json` | **blocked** (S2/S5 own `repo.rs` and the harness) | – |

### 2.1 Replay parity, `--engine facts`, per batch (latest build, run `20261022T211917-replay-g4`)

| batch | calls | FP | FN | failures | remaining (all named) |
|---|--:|--:|--:|--:|---|
| 1 | 317 | 48 | 48 | 60 | `backend_type_strings` only. The harness refuses the call ("findings carry no column"), and its FP/FN are the column dimension. The engine now emits `column` for regex findings. S5 owns the harness side (agreed) |
| 2 | 391 | 0 | 0 | 0 | — |
| 3 | 361 | 0 | 0 | 0 | — |
| 4 | 573 | 0 | 542 | 249 | `grammar_callable_constructors`, `grammar_fragment_usage`, `sql_grammar_escape_hatches`: a budget evidence `closure` (native `query/name_closure.rs`, a repository name closure) is refused by name. This is a clause-level construct, not a leaf |
| 5 | 312 | 0 | 237 | 154 | `imperative_private_functions` (`binds` + `calls`), `internal_dead_code` (`binds`, `reference_edges`), `public_api_triples` (SQL `printf`, order-dependent repository plan). All three are IR-level refusals too (S1 § 5.4) |
| 6 | 1259 | 0 | 0 | 0 | — |

Progress within the slice, from the same harness:
- At `caceb656f` (S5's lane, `20261022T204150-gate`), `lines` options were ignored, which gave false positives:
  - `hasattr_async`: 7 false positives.
  - `no_source_inspection`: 3.
  - `serverplan_name_inference`: 456 in the suite.
  - `admin_async_purity`: 21.
- First build (`…-replay-g1b`): b1 had 149 failures and b2 had 73.

### 2.2 Census refusals left (leaf executor, `20261022T211915-census-g4/census.md`)

| construct | rules | why it stays refused |
|---|---|---|
| pattern key `binds` (scope table: name bindings) | `imperative-private-functions` (3), `internal-dead-code` (1) | Both rules are refused at IR level anyway (a `calls` atom; `reference_edges`). Porting `binds` needs the scope table's binding facts (S4) |
| order-dependent repository plans (LIMIT, `row_number()`, bag recursion, JsonGroupArray) | `public-api-triples`, `serverplan-quality`, `serverplan-state-tripwires` | S1/S3 constructs, unchanged by this slice |
| `raw_module_names` table | `internal-dead-code` | cross-file relation, IR-refused |

## 3. How measured

- **Census:** `./target/release/facts-a census --rules packs/data-layer/candidates/rules --out <run>`, from the worktree.
- **Replay:** `~/rerail-bench/facts/phase-2/s6-leaf/replay-all.sh <run> 1 2 3 4 5 6`.
  - For each batch it runs `node --import tsx conformance/data-layer/replay.ts --recording … --engine facts`, with the same recordings and probes as `/tmp/dl60/replay-all.sh`.
  - It then runs `summ.py` per check.
  - Each run dir records `commit.txt`, `status.txt` and `binary.sha256`.
- **Gate:** `just facts-gate --run-dir <run>`.
  - The run against commit `39d33188c` used a detached worktree, `git worktree add --detach /tmp/s6-gate 39d33188c`, so that later edits could not reach its build.
- **Construct inventory:** `~/rerail-bench/facts/phase-2/s6-leaf/inventory.py <rules dir>`.

## 4. Artifacts

- Census JSON and markdown: `<run>/census*.{json,md}`.
- Replay per batch: `<run>/replay-bN.json`, which holds per-call rows with FP/FN lists and failure messages, plus `<run>/replay-bN.log`.

## 5. Design notes

### 5.1 Constructs ported (native source → facts-a)

Ordered by the rules each one unblocked in the census:

| construct | clauses (wave-1 census) | native source | facts-a |
|---|--:|---|---|
| regex `content` gate, `lines.include/exclude/state/indentation/first/terminators` | 58 + 38 + 1 + 2 + 1 | `query/mod.rs:21-120`, `query/line_state.rs`, `compile/mod.rs:589-705`, `compile/line_state.rs`, `eval/query.rs:721-802` | `ast/lines.rs` (subagent LinesPort; integrated and verified by replay). `eval.rs` `regex_findings` |
| `precedes` / `follows` (and in `count`) | 39 + 5 | `matcher/relation.rs:262-281` `find_sibling`; `tape.rs:2111-2228` `next`, `prev`, `next_all`, `prev_all`, `walk_anchor` | `ast.rs` `Rule::Precedes/Follows`, `Eval::sibling_walk` |
| structural pattern text | 42 | ast-grep-core 0.50 `match_tree`, `Pattern`; native `compile/ast_rule.rs:408-440` | `ast/pattern.rs` (subagent PatternPort) |
| `value.type` | 28 | `matcher/value.rs:269-275`, `value/mod.rs:547` | `ValueTest.types`. Only `str`/`bytes` are accepted, because the decoder tells only those apart; any other type is refused at compile |
| parameterised `matches` / `{params, rule}` utilities | 9 + 25 | `compile/ast_rule.rs:215-285`, `compile/utils.rs:54-110`, `matcher/call.rs:36-85`, `matcher/mod.rs:656-730` `unify_capture`, `each::framed` | `Rule::Call`, `Eval::call`, `Eval::unify` |
| `layout` | 21 | `compile/layout.rs`, `matcher/layout.rs` | `ast/layout.rs` (subagent LayoutPort) |
| `transform` (`replace`, chains, `default`), compared through `compare` | hidden behind the first refusal per clause | `matcher/transform.rs`, `matcher/compare.rs:111-148` | `Transform`, `Eval::transform_text`, `replace_all` |
| `count` `as`/`offset`, and `count` under `each::suspended` | 1 | `matcher/count.rs:20-155`, `matcher/mod.rs:562` | `Rule::Count.binding`, `Env.texts` |
| `arguments` (syntactic) | 3 | `compile/arguments.rs`, `matcher/arguments.rs` | `ast/arguments.rs` (subagent ArgsParsePort) |
| `parse` `identifierWords`, `interpolation`, `host`; pythonFString divergence fixes | 2 + 9 + 2 | `matcher/parse.rs`, `literal/python_fstring.rs`, `compile/parse.rs` | `ast/parse_ext.rs`, `fstring.rs` (S4's file; S4 agreed) |
| `compare text: index` | 5 | `matcher/compare.rs:287-337` | `Eval::element_index` |
| `ast.file` (path gate), `ast.when` | harness path only | `compile/mod.rs:856-890`, `eval/query.rs:1432-1440` | `Compiler::query`. `file` uses the file-path-gate rewrite (REWRITES.md); `file` without `syntax` is refused. `when` goes through `rerail_facts::front::rule::when_holds` (made `pub`) |

### 5.2 Changes beyond leaves (needed for parity; owners agreed)

- **`budget` generalised** (`eval.rs`; S2 agreed):
  - It groups by any projected `group_by` fields, or by none, which makes one repository group.
  - Groups keep first-row order across files sorted by path, and each group is reported at its first row (native `query/crossfile.rs:436-509`).
  - Evidence `where` uses native `Predicate`: `eq`/`ne` by JSON text, and UTF-16 order for strings only (`crossfile.rs:97-267`).
  - `closure` and `count.distinct` are refused by name.
- **Regex clauses on non-Python files:** they now run over `.rs` files with a `Newlines` line table (`eval::eval_text_file`, in the `calls` path; native `language/adapter/rust.rs:53`). This fixed `rust_sql_grammar_boundary`.
- **Regex findings carry `column`:** the 1-based code-point column of the anchor's start. Under `lines` that is the scan's in-line offset (native `eval/query.rs:754-762`). This lays the groundwork for `backend_type_strings`.
- **A fix found on the way:** a capture anchor under `lines` is now read in the line's context, not the whole file.

### 5.3 Capture analysis

- The wave-1 analysis did not follow utilities: `binds()` returned nothing for `matches`, so capture ordering silently ignored what a utility binds.
- It is replaced by `Summary`, which ports native `collect_binds`, `binds_and_reads`, `collect_mentions` and `parse_exported`.
  - Utilities are summarised as least fixpoints, iterated globally. That equals native's per-component `UtilitySummary`, because every summary is monotone.
  - A `Call` maps its utility's summary outward through its arguments.
- Utilities compile twice, as native does (`compile/mod.rs:1107-1126`): first against an empty ("unfilled") table, then against that draft. The main rule compiles last, against the final table.

### 5.4 Interface changes

| item | change |
|---|---|
| `ast::Query` | `utils`/`util_capture_free` are `Arc`, plus new `summary` and `transform` fields; S2 takes this side for Send+Sync |
| `ast::Compiler` | `shared: Option<UtilTable>`, plus a new `options` field for `when` |
| `ast::Env` | new fields `multi` (`$$$` lists) and `texts` (text variables) |
| `ast` functions | new `ast::binds_name`; `eval.rs` `Input::query_binds` now calls it |
| `eval::Finding` | gains `column: Option<u32>`, serialised only when set |
| budget types | new `eval::BudgetRows`, used by `main.rs`/`repo.rs`; `Kind::Budget` gains `group` and `filter` |
| `fstring::parse` | takes `marker: &str` |
| `rerail_facts::front::rule::when_holds` | now `pub` (visibility only) |

## 6. Risks found

- **Two compile paths:** the census (rerail-facts front end plus `LeafCheck`) and the harness (`eval::Rule::compile`) are separate. Wave 1 accepted `ast.file`/`when` and `lines` in one path and ignored or refused them in the other, which is S5's I9 finding. Both paths now go through the same `ast.rs`/`lines.rs` code, but `when`/`file` are still applied separately: the front end rewrites them, the harness path evaluates them.
- **Annotation `parse` exports:**
  - They now use `parse_ext::export`, which never overwrites a node binding (native reads the node match first). Before this, `env.set` was used.
  - ArgsParsePort also noted that a later `$X` re-binding against an exported capture is checked for an exact match. Native binds freely there, because exports are stored apart from node matches. Fixing that needs a distinct `Bound` variant. No replay difference shows it.
- **Utility recursion depth:** the executor refuses past depth 256 (native `MAX_UTIL_DEPTH`). Native's 1 MiB stack limit could cut in earlier; that would show as a native-side failure in parity, and none has appeared.
- **`Unsupported` syntax with a false `file` gate:** natively a gated-out file reports nothing. Here the rewrite only applies when `syntax` is set, and is refused otherwise.

## 7. Resume notes

**Branch state**
- `39d33188c` is milestone 1. It holds all the leaf constructs and passed `facts-gate` 13/13 on the same tree (`20261022T210204-gate-m1`).
- `8bdf55063` is milestone 2, gated 13/13 (`20261022T212846-gate-8bdf55063`). It adds:
  - budget `where`;
  - regex `column`;
  - regex clauses on `.rs` files;
  - the `lines` capture-anchor context fix.

  The latest replay (b6 exact; b1–b5 as in § 2.1) is on that tree.

**Next**
1. **S5:** compare `column` in `replay.ts`/`facts-engine.ts` for `backend_type_strings`, as agreed. This should close batch 1.
2. **Batch 4:** port evidence `closure` (native `query/name_closure.rs`, 157 lines, and `compile/evidence.rs:201-321`) into `eval.rs` budget. It is a repository computation: seeds from options or rows, edges per partition, `contains` outputs added as fields before `where`. It closes `grammar_callable_constructors`, `grammar_fragment_usage` and `sql_grammar_escape_hatches`.
3. **Suite parity b1–b5:** `facts-a repo` must compile the recording's checks rather than `BATCH6` (`repo.rs:38-52`), and `facts-engine.ts` must expect non-`.py` files. Then run `suite.ts --engine facts` per batch against `/tmp/dl60/suite-b{1..5}.json`.
4. **Batch 5:** the 3 open rules wait on IR-level constructs (`binds`, `calls`, `reference_edges`, order-dependent plans).
5. **Cleanup:**
   - Dead-code warnings in `ast/*.rs` (unused `sub_rules`/`reads` helpers) and `ast.rs` (`Names::get`, `Unsupported`, `trial`).
   - The census JSON's leaf pass still reports through `leaf.rs`; only compile is checked there.
