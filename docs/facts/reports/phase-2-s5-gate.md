# Phase 2 S5 report: gate harness

Status: **G2.6 measured and passing** on this branch, with every slice's latest work merged: 13 of 13 seeded mutants killed, gate median 295 s in one command. `--engine facts` reproduces Spike A's batch-6 numbers through Jeremy's harness. Wave 2 (§ 7): batches 1–5 run through the harness in the gate; 44 of their 50 checks are exact and gated, 6 refuse by name; the gate passes with 25 checks and 21 of 21 mutants killed in 389 s (load 15). Branch `Moti-Velz/facts-engine-s5-gate`, worktree `~/orca/workspaces/rerail/facts-engine/.worktrees/s5-gate`. Raw runs are under `~/rerail-bench/facts/phase-2/s5-gate/runs/` and are cited relative to it.

## 1. Setup

| Item | Value |
|---|---|
| Commits | `f208cfd0e` gate interface (first commit); `39b8c0363` mutant hooks + harness bridge; `94e31685f` reference evaluator; `b87eab2e0`, `3f973bae5`, `5f6246912` gate after the re-steer; measured gate runs at `5f6246912` (`…154719-gate`) and its successor (summary wording only) |
| Merged in | integration `2b111331b`; S1 `02b49ed65`; S2 `4b14dfe7e`; S3 `bf3685133`; S4 `275b016d1`. My merge commits carry one fix across slices: S2 removed `Db.regexes`, while S4's `derived_check.rs:381` and `views.rs:151` still initialised it. S4 acknowledged the fix and will apply it. |
| Build | `CARGO_BUILD_JOBS=4`, release. The gate builds in `target/facts-gate` with `facts-a/mutants,facts-b/mutants` and tests `rerail-facts --features mutants`. On first use it is seeded by an APFS clone of `target/release`. |
| Machine | darwin arm64, shared with four other slices: 1-minute load 14–34 during the runs. Every timing here is therefore an upper bound measured under contention, not a trusted (< 8) measurement. |
| Corpora | `~/rerail-bench/nera-data-layer-2b1504ad` (9,010 files); batch-6 recordings `fixtures/host/data-layer-port/batch-06-{recording,probes}.json`; suite oracle `~/rerail-bench/facts/spike-a/work/suite-b6.json`; Spike B's edit script `~/rerail-bench/facts/spike-b/edits/seed-5eedb/events.jsonl` (1,111 events) |

## 2. Results

| Gate row | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| G2.6 every seeded mutant caught | all | 13/13 killed: rewrites R1–R5 (11–15), generic 4–8, interim A3 views 1–3. Kill matrix in § 4. | **pass** | `20261022T154719-gate` (`gate.json`, `mutants/`) |
| G2.6 gate time, one command | ≤ 10 min | `just facts-gate --mutants`: median **295 s** of 389 / 295 / 268 s (§ 2.1). A cold build from a cloned `target/release` adds 235–257 s under load 10–14 (`…151205-gate`, `…153626-gate`). A cold build plus the slowest run (load 34) would be 646 s, over 10 min; with a warm build every measured run is under 6.5 min. | **pass** | `20261022T154719-gate`, `…155725-gate`, `…160221-gate` |
| `--engine facts`, `replay.ts`, before S4 | Spike A's batch-6 numbers | 1,253/1,259 calls, 0 FP, 2 FN, 6 failures, all `quality-contracts-1` `annotation`. Identical to Spike A's `…135635-replay` and to `facts-a replay`. | **pass** | `20261022T150702-harness-replay` |
| `--engine facts`, `suite.ts`, before S4 | Spike A's repository numbers | 173/173 findings, 0 FP, 0 FN, 38 failures, all `quality-contracts-1`. Identical to Spike A's `parity.json`. | **pass** | `20261022T150850-harness-suite` |
| `--engine facts`, `incremental.ts` | 0 differences | 10 checks, 40 transitions, 0 differences | **pass** | `20261022T151155-harness-incremental` |
| `--engine facts` after S4's `string_type_definition` | exact | replay 1,259/1,259, 0 FP/FN/failures (gate `harness-replay`). Suite 173/173, 0 FP, 0 FN, 0 failures, and the incremental phases (unchanged, trailing-newline edit) have 0 differences. Incremental 40/40. | **pass** | `20261022T154719-gate/harness-replay`, `20261022T155358-harness-suite`, `20261022T155721-harness-incremental` |
| Reference evaluator over the IR vs S3's generated dataflow (G2.4 exactness part) | 0 differences | `b-generated`: R1 and R3 as IR plans through S3's generator against `rerail_facts::reference`: 0 differences in 1,112 checkpoints, 1,576 worktree checks, 1,111/1,111 events, no runtime refusals | **pass** | `20261022T154719-gate/b-generated` |

### 2.1 Gate wall time, three runs

`just facts-gate --mutants`; runs 2 and 3 under `~/rerail-bench/bench-lock … --no-build`. Each run: 13/13 checks, 13/13 mutants.

| run | wall (s) | load start → end | lane times (s): a (replay, repo, subset, a3-check, reference) / b / g / c |
|---|--:|---|---|
| `20261022T154719-gate` | 389 (warm build 16 s included) | 34.2 → ~15 (untrusted) | 23, 83, 12, 22, 13 / 327 / 328 / 1, 0, 0, 4, 34, 98 |
| `20261022T155725-gate` | 295 | 12.9 → 5.0 (untrusted) | 20, 76, 11, 20, 12 / 260 / 260 / 0, 0, 0, 4, 31, 90 |
| `20261022T160221-gate` | **268** | 5.0 → 3.0 (trusted) | 15, 61, 9, 17, 10 / 233 / 233 / 0, 0, 0, 3, 23, 76 |

Median **295 s** (spread 268–389). The critical path is lane b or g, the 1,111-event replays with the oracle after every event (233–328 s), plus about 40 s of mutant runs (each fails fast within ~15 s).

## 3. How measured

- **Gate:** `just facts-gate --mutants` = `scripts/check/facts-gate.sh --mutants`, from the worktree root. The run directory holds `gate.json` (format `rerail.facts-gate/v1`) and one directory per check and mutant run. Each has `command.txt` (commit, env, argv), `stdout.txt`, `stderr.txt` (with `/usr/bin/time -l`), `uptime-before.txt` and `uptime-after.txt`. The output format is in § 5.1.
- **Harness:** `node --import tsx conformance/data-layer/{replay,suite,incremental}.ts … --engine facts` with `FACTS_ENGINE_BIN` pointing at a facts-a build (default `target/release/facts-a`). `node_modules` is a symlink to the main checkout's, which is git-excluded. Each run directory has the same files as a gate run.

## 4. Kill matrix (`20261022T154719-gate/mutants/`)

| n | Mutant | Site | Killed by | Evidence |
|--:|---|---|---|---|
| 1 | dropped side condition (interim) | `subst.rs::edges` (A3 view) | `a3-check` (`a-reference` survives: findings unchanged) | `FACTS_A3_CHECK: stage edges at data_layer/lazy_result.py: 540 rows vs 539` |
| 2 | lost union arm (interim) | `subst.rs::nodes` | `a3-check`, `a-reference` | `stage nodes … 425 rows vs 540`; 10 findings only compiled, 3 only reference |
| 3 | wrong interval bound (interim) | `ast.rs::has_end_memo`, the range starts at `id + 2` | `a3-check` (`a-reference` survives) | `has memo at node 2101 in impl/common/l1000_cache/serialization.py` |
| 4 | missing ΔR⋈ΔS cross term | S3 generator, join tags | `b-generated` | 65 differences by the 2nd checkpoint |
| 5 | retraction-blind count | S3 generator, presence/count | `b-generated` | 65 differences |
| 6 | fixpoint stops early (round ≥ 2 dropped) | S3 generator, fixpoint | `b-generated` | 41 differences |
| 7 | per-content plan reaching a membership relation | `classify::file_source` | `classifier-leak`, `rerail-facts-lib` | `leak_calls_atom_in_forbid_is_rejected`, `leak_reference_edges_in_per_file_plan_is_rejected`, `leak_module_projection_in_per_file_plan_is_rejected` |
| 8 | inner scan admitted | `phys.rs` plan audit | `planner-audit`, `rerail-facts-lib` | `phys::tests::audit_refuses_inner_scan` |
| 11 | R1 file-path gate: dropped side condition | `front/rule.rs` | `rerail-facts-lib` | `classify::tests::rewrite_file_path_gate` |
| 12 | R2 deferred-column inlining: wrong column map | `front/rule.rs` | `rerail-facts-lib` | `rewrite_deferred_reads_stored_columns` |
| 13 | R4 export projection: dropped side condition | `classify.rs` | `rerail-facts-lib` | `rewrite_projection_keeps_read_columns` |
| 14 | R5 export distinct: dropped side condition | `classify.rs` | `rerail-facts-lib` | `rewrite_distinct_only_under_set_readers` |
| 15 | R3 file-locality cut: dropped side condition | `classify.rs` | `rerail-facts-lib` | `rewrite_cut_requires_path_equality`, `rewrite_projection_keeps_read_columns` |

Earlier candidates that survived, and were replaced:

- **"Swapped tie order"** in the `nodes` window: no ties occur on the subset, so the mutant is equivalent there.
- **`subtree_end - 1` and `subtree_end + 1`** in `has_end_memo`: the last descendant, and the node after the subtree, never decide a `has` verdict on the subset.

Both runs are in `20261022T145137-gate` and `…145500-gate`. The re-steer made 1–3 interim in any case: they go with `subst.rs`.

## 5. Design notes

### 5.1 Gate (interface; first commit `f208cfd0e`, extended since)

The command is `just facts-gate [--mutants] [--only <ids>] [--no-build] [--run-dir <dir>]`. Exit 0 means passed, 1 failed, 2 usage. Five lanes run concurrently (lane h since wave 2, § 7):

| Lane | Check | Passes when |
|---|---|---|
| a | `a-replay`: `facts-a replay` over both recordings | every recorded call replayed, 0 FP, every miss or failure is accepted residue (`RESIDUE`, now empty) |
| a | `a-repo`: `facts-a repo --threads 4` + `repo-compare` | 0 FP, 0 FN, the oracle's 9,010 files, no unaccepted failure |
| a | `a-subset` / `a3-check` (`FACTS_A3_CHECK=1`) / `a-reference` (`FACTS_A3_OFF=1`) over the subset | no rewritten relation differs from the unrewritten one; findings and failures are multiset-equal to the unrewritten plan's |
| b | `b-exactness`: `facts-b replay --events E` | 0 differences; Spike B's hand-written expansions, the generator's test vectors |
| g | `b-generated`: `facts-b greplay --events E` | 0 differences between S3's generated delta plans and `rerail_facts::reference`; no runtime refusals |
| c | `rerail-facts-lib`: `cargo test -p rerail-facts --lib` | all pass (the rewrites' differential tests, classifier, audit, reference) |
| c | `classifier-leak`: `cargo test … leak` | ≥ 1 test runs, all pass (G2.1's seeded leak is rejected) |
| c | `planner-audit`: `cargo test … phys::tests::audit` | all pass (an inner scan is refused) |
| c | `explain-stable`: `facts-a explain` over the pack, twice, and once per rule in its own process | the three texts are byte-identical. 60 rules, 2.27 MB of plan text. |
| c | `harness-replay`: `replay.ts --engine facts` | as `a-replay` |
| c | `views-derived`: `facts-a derived-check --shard 0/4` (S4) | `bag_mismatch_total` 0, no node mismatches, no failures |
| h | `harness-b1`…`harness-b5`: `replay.ts --engine facts` over batch n's recordings and probes | every check in `GATED` exact (0 FP, 0 FN, 0 failures, ≥ 1 positive and ≥ 1 clean call); the rest reported per check |
| h | `suite-b1`…`suite-b5`: `suite.ts --engine facts --cold-only` over batch n's repository oracle | every check in `GATED` with 0 FP, 0 FN, 0 failures; the rest reported per check |

**`--no-build`.** Without it the gate first runs `cargo build --release` for `facts-a` and `facts-b` with their `mutants` features, then `cargo test --release -p rerail-facts --features mutants --no-run`, both in `target/facts-gate` (`FACTS_GATE_TARGET`). Measured: 16 s with nothing changed, 85 s after an edit to `facts-a`, 169 s after the integration merge `caceb656f` (load 13), 235–257 s cold from a clone of `target/release`. `--no-build` skips both and runs whatever `target/facts-gate` holds, so use it only when nothing has changed since the last gate build (the commit stamp reads `+dirty` for uncommitted edits, but nothing checks that the binaries match them). Timing runs use `--no-build`, so the build is not inside the measured wall time.

**Subset (deterministic).** Every corpus file whose `sha256(path)[0] % 8 == 0`, plus every file with a suite-oracle finding: 1,150 files, hard-linked under `~/rerail-bench/facts/phase-2/s5-gate/subset-8/` (`subset-8.list`). The repository parity runs on the full corpus.

**Mutants.** Two sources:

- the generic list in the driver (`MUTANTS`): 1–3 interim A3 views, 4–6 delta generator, 7 classifier, 8 plan audit;
- one mutant per row of `docs/facts/REWRITES.md`'s registry table (`mutant` column, `<n> <kind>`). A registered rewrite without a well-formed mutant fails the gate.

Each mutant runs the checks of its family with `FACTS_MUTANT=<n>`. A kill is a detection the check names; a crash or a missing tool is `error`, never a kill.

Output, one line per event, each prefixed `facts-gate: `:

```
facts-gate: run <run dir> commit <sha>[+dirty] load <1-min load>
facts-gate: build <ok|FAIL> <s>s <package>[mutants] ...
facts-gate: check <id> <pass|FAIL> <s>s <summary>
facts-gate: registry FAIL <problem>                       (a rewrite without a mutant)
facts-gate: mutant <n> <name> <killed|SURVIVED> <s>s <check>=<killed|survived|error> ...
facts-gate: <passed|FAILED> <s>s: <p>/<c> checks pass, <k>/<m> mutants killed[; failing: <ids>]
```

### 5.2 `--engine facts` in Jeremy's harness

The bridge is the CLI `conformance/data-layer/facts-engine.ts`.

- `replay.ts` and `incremental.ts` resolve each call's policy exactly as they do for `native`/`sql`: options, scope, clause filters, the zod-parsed document. They queue the calls, run one `facts-a calls --input calls.jsonl --output results.jsonl` (compiled policies cached by text), and map the findings back into the result-page shape they compare: path, line, code, category = clause id.
- Columns are refused, not guessed: a `backend_type_strings` call fails with `facts-unsupported`, because facts findings carry lines, not byte spans.
- A compile error or engine failure is a failure of that call. The addon is never loaded under `--engine facts`, so nothing can fall back to native.
- `suite.ts` writes the recording's files under a temporary root and runs `facts-a repo --threads 4`. The facts engine has no resident workspace, so the two incremental phases are from-scratch runs over the unchanged and the edited trees (`incrementalMode: "from-scratch"`).

### 5.3 Reference evaluator (`facts/src/reference.rs`, `rerail_facts::reference`)

A naive evaluation of S1's IR, written from `IR.md` alone:

- nested-loop joins, with subqueries re-evaluated per outer row (correlation through the row stack);
- aggregates with first-seen groups and SQLite's representative row;
- `row_number()` over the total order (partition, keys, input position);
- set and bag unions, stable sort with `LIMIT`/`OFFSET`, `WITH`, and level-by-level fixpoints with the 50 M cap;
- reachability (directed and forest);
- every `Func` (`printf` limited to `%d %i %s %f %%`; any other conversion is refused by name).

Values and affinities come from `sql::value`. It refuses by name: budget sinks, `source_text` and syntax reachability in repository plans.

- **API:** `eval_scalar`, `truth`, `Evaluator::rel`, `evaluate_plan`, `evaluate_repository(plan, exported)`, `multiset`.
- **Users:** S3's `greplay` uses it as the from-scratch side of `member(w) ⋈ exported facts`, and its scalar evaluation inside the dataflow, so the comparison tests the relational and delta machinery. S3's `Plan` trait defaults `reference` to it.
- **Tests:** 5 unit tests (`rerail-facts-lib`).

### 5.4 Edit-sequence replay with real plans

S3 owns the plan registry. `facts-b greplay [--plans …] [--real spec.json]` runs any IR repository plan through the generator and the reference over Spike B's events, with no harness edit. The gate's `b-generated` runs whatever `greplay` registers; today that is R1 and R3 as IR. The `replay` driver (hand-written R1–R3) stays as `b-exactness`, since it supplies the generator's test vectors (I8).

## 6. Risks found

- **Cross-slice semantic conflicts appear only when every branch is merged.** S2's `Db.regexes` removal broke S4's views. S1's IR change (`deferred` removed) broke S3's `norm.rs` until `bf3685133`. Because the gate builds the merged tree, it is where these surface first.
- **`a-reference` is still Spike A's unsubstituted SQL path, not `rerail_facts::reference`.** Interim mutants 1 and 3 change no finding on the subset, so only the row-level `FACTS_A3_CHECK` catches them. Once S2's backend runs S1's per-file plans, `a-reference` must compare against `reference::evaluate_plan` over `FileSource` scans; that also gives I1's per-file differential.
- **Rewrite mutants 11–15 are killed by S1's unit differentials only.** The registry names corpus parity as the second test; it applies once the leaf executor runs the rewritten rules.
- **Timing.** Only the third run started under load 8 (§ 2.1). A cold build plus the slowest run exceeds the limit: 257 s + 389 s = 646 s. Warm builds are the norm after the first run. Shortening the replays (`--limit`) is the lever if a slower machine needs one, but it would weaken G2.4's exactness evidence.

## 7. Wave 2

Base: integration `caceb656f`; merged since: S6 `39d33188c`, `8bdf55063`; S1 `a40a2bcbb`, `f1edf7edc`; S3's tie vector; integration `8960a5db7`. S5 commits on `Moti-Velz/facts-engine-s5-gate`: `7d23ab4a7` per-batch checks and refusals; `3f6c5fe6d` `GATED` ratchet; `2093ccc98`, `6c7cec396` report; `ec37ce73b` mutant 9 reserved; `e2c4d55e7` harness column comparison and mutant 10; `c176eefc7` reference tie refusals; `cc2fce760` 44 checks gated; `4c907e133` explain-physical, b-ties, mutant 9; `3e7bee330` merge of `8960a5db7`.

### 7.1 Results

| Gate row | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| G2.6 every seeded mutant caught, at `caceb656f` | all | 13/13 killed, 13/13 checks | **pass** | `20261022T203358-gate` |
| G2.6 every seeded mutant caught, current tree (`3e7bee330`) | all | **21/21 killed** (1–10, 11–21), **25/25 checks**. New: 9 (S3 tie detection, killed by `b-ties`: 51 differences, 51 of 51 expected refusals missing), 10 (S1 order proofs), 16–21 (S1 rewrites R6–R11, unit differentials). Mutants 1 and 3 still survive `a-reference` and die by `a3-check` only | **pass** | `20261022T214833-gate` |
| G2.6 gate time, one command | ≤ 10 min | `--mutants --no-build`, lane h added, under `bench-lock`: 314 / 335 / 369 s, median **335 s** (loads 5.5 / 7.2 / 13.2). Current tree with 25 checks and 21 mutants: **389 s** (build up to date, load 15.4, untrusted). Critical path: lane b/g (342–343 s at load 15); lanes h and t add 20 s (explain-physical) and 11 s (b-ties) in parallel | **pass** (untrusted; decider to re-run quiet) | `20261022T204421-gate`, `…205339-gate`, `…205914-gate`, `20261022T214833-gate` |
| Per-batch `--engine facts`, batches 1–5 (replay + suite) | reported; refusals are failures | **44/50 checks exact in both and gated**; 6 refuse by name (budget-evidence closure ×3, `binds` ×2, `printf` ×1); 0 checks differ without refusing | measured, gated | `20261022T214833-gate/harness-b*`, `…/suite-b*` |
| Per-batch suite incremental phases (wave-2 start) | from-scratch phases = cold | equal for every batch | measured | `20261022T205039-harness-suites` |
| `explain-physical` per file over the subset | identical runs; one text per clause across files | 1,150/1,150 files, 50 clauses, sha256 `560fc8448638d5a4` in both runs, each clause one text; 1,233 audit inner-scan lines (informational, 2 stage plans) | **pass** | `20261022T214833-gate/explain-physical` |
| `b-ties` (S3's constructed tie, 200 events) | refusals met, 0 differences | 0 differences in 201 checkpoints, expected refusals `rt` 280/280 | **pass** | `20261022T214833-gate/b-ties` |

### 7.2 Per-batch parity through the harness (`20261022T214833-gate`)

Replay: `replay.ts --engine facts` over the batch's recordings and probes (as `/tmp/dl60/replay-all.sh` combines them): 317 / 391 / 361 / 573 / 312 calls; batch 4's 3 host-signature-guard calls are evaluated by the host. Suite: `suite.ts --engine facts --cold-only` over `data_layer_suite_record.py`'s oracle at `2b1504adc` (`/tmp/dl60/suite-b<n>.json`, copied with SHA256SUMS to `~/rerail-bench/facts/phase-2/s5-gate/suites/`): 9,010 / 5,912 / 6,982 / 3,870 / 5,914 files. A 0-finding suite oracle only shows that the engine adds none; the replay carries the coverage (≥ 1 positive and ≥ 1 clean call per gated check). The wave-2 start (`20261022T204421-gate`): 6/50 exact, 4 differing without refusing (regex `lines` ignored, fixed by S6), 40 refused.

| batch | check | replay exact calls | replay FP / FN / failures | suite oracle findings | suite FP / FN / failures | status | first refusal |
|---|---|--:|---|--:|---|---|---|
| 1 | `removed_compat_surfaces` | 4/4 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `versioned_lifecycle_legacy_surfaces` | 3/3 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `no_api_root_imports` | 4/4 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `backend_type_strings` | 60/60 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `hasattr_async` | 33/33 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `no_source_inspection` | 72/72 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `serverplan_name_inference` | 8/8 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `no_blanket_grammar_bypass` | 121/121 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `serverplan_grammar_surface` | 7/7 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `permission_error` | 5/5 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 2 | `dynamic_module_loading` | 66/66 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 2 | `forwardref` | 15/15 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 2 | `unresolved_forward_refs` | 73/73 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 2 | `classvar_check` | 16/16 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 2 | `type_safety_patterns` | 56/56 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 2 | `conn_or_txn_union` | 15/15 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 2 | `async_executor` | 47/47 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 2 | `async_patterns` | 38/38 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 2 | `async_sync_boundary` | 17/17 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 2 | `admin_async_purity` | 48/48 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 3 | `plan_scope_side_effects` | 29/29 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 3 | `plan_scope_ownership` | 30/30 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 3 | `transaction_closures` | 104/104 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 3 | `transaction_callback_wrappers` | 20/20 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 3 | `transaction_patterns` | 50/50 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 3 | `lazy_result_usage` | 17/17 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 3 | `swallowed_exceptions` | 25/25 | 0 / 0 / 0 | 807 | 0 / 0 / 0 | exact, gated |  |
| 3 | `error_handling` | 13/13 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 3 | `module_level_caches` | 23/23 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 3 | `connection_caches` | 50/50 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 4 | `render_fstrings` | 37/37 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 4 | `render_raw_strings` | 53/53 | 0 / 0 / 0 | 178 | 0 / 0 / 0 | exact, gated |  |
| 4 | `regex_sql_parsing` | 97/97 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 4 | `sql_grammar_escape_hatches` | 21/163 | 0 / 451 / 142 | 0 | 0 / 0 / 1 | refused | unsafe-parameter: budget evidence closure unsupported (a name closure over the repository's rows) |
| 4 | `grammar_render_methods` | 75/75 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 4 | `grammar_str_params` | 19/19 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 4 | `grammar_callable_constructors` | 0/27 | 0 / 30 / 27 | 0 | 0 / 0 / 1 | refused | constructor-arg: budget evidence closure unsupported (a name closure over the repository's rows) |
| 4 | `grammar_fragment_usage` | 0/80 | 0 / 61 / 80 | 0 | 0 / 0 / 1 | refused | captured-return: budget evidence closure unsupported (a name closure over the repository's rows) |
| 4 | `grammar_allowlist_burndown` | 8/8 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 4 | `rust_sql_grammar_boundary` | 14/14 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 5 | `missing_async_pairs` | 15/15 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 5 | `sync_async_signatures` | 10/10 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 5 | `async_sync_delegation` | 37/37 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 5 | `public_api_triples` | 0/38 | 0 / 32 / 38 | 0 | 0 / 0 / 1 | refused | exported-families: stage hierarchy: function printf unsupported |
| 5 | `data_layer_facade_purity` | 12/12 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 5 | `data_layer_compat_aliases` | 11/11 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 5 | `layer_violations` | 11/11 | 0 / 0 / 0 | 133 | 0 / 0 / 0 | exact, gated |  |
| 5 | `backend_coupling` | 62/62 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 5 | `internal_dead_code` | 0/43 | 0 / 55 / 43 | 4,931 | 0 / 4,931 / 1 | refused | unreachable-internal-symbols/raw_module_names: rule/all/7/not: unsupported rule key `binds` |
| 5 | `imperative_private_functions` | 0/73 | 0 / 150 / 73 | 18,771 | 0 / 18,771 / 1 | refused | direct-execution: rule/all/0/all/4/any/0/all/1/any/0/has: unsupported rule key `binds` |

### 7.3 Findings

- **I9 violation (fixed): regex `lines` was accepted and ignored** in `eval.rs`, giving FPs in `hasattr_async`, `no_source_inspection`, `admin_async_purity` and 456 suite FPs in `serverplan_name_inference`. Found by lane h at the wave-2 start, reported to S6, fixed in `39d33188c`; all four are exact and gated now.
- **Rust sources were skipped silently** by `facts-a calls` and `facts-a repo`. First refused by name (`7d23ab4a7`); with S6's `eval_text_file` (`8bdf55063`) both now evaluate regex clauses over `.rs` text, and `rust_sql_grammar_boundary` is exact.
- **`facts-a repo` ran batch 6 only**, so `suite.ts --engine facts` silently evaluated the wrong rules for batches 1–5; it now takes `--checks` from the recording.
- **`backend_type_strings`** compares columns: the harness compared nothing and refused; with S6's code-point `column` the harness compares it when every finding carries one (60/60 calls exact).

### 7.4 Interface changes

- `facts-a repo --checks c1,c2,…` (default batch 6); `repo::compile_checks(policies, checks) -> (rules, refused)` and `repo::selected_checks(args)` replace `compile_batch6` (S1's `physplan.rs` uses them). A policy that does not compile, or panics compiling, is a failure `compile: …` of its check.
- `facts-a repo` evaluates `.rs` files' regex clauses via `eval::eval_text_file` (as `facts-a calls`).
- `facts-a calls`: a compile panic is that call's `compile_error`; an evaluation panic is a failure `engine panic: …` of that call. `repo` does not catch evaluation panics (`a3-check` relies on them).
- `conformance/data-layer/facts-engine.ts`: `runFactsRepository(files, policies, checks, threads)`; findings carry an optional `column`; `replay.ts` compares it for column-compared checks and refuses only a finding without one.
- `rerail_facts::reference`: under `Total::Checked`, a tie between distinct rows is `Err("tie: …")` — row_number (equal partition and order keys), `Sort` with LIMIT/OFFSET (any tie, wherever the cut falls), group_concat/json_group_array (keys of the Sort reached through Project/Filter; none: any two distinct rows of a group). Mirrors S3's dataflow; test `reference::tests::checked_orders_refuse_ties`.
- Gate: lanes h (`harness-b1..b5`, `suite-b1..b5`, `explain-physical`) and t (`b-ties`); `GATED` (44 checks) is the ratchet; `check_b` accepts run-time refusals only on plans in `expected_refusals` and counts `missing_expected_refusals` as a detection; fixed mutants 9 and 10; REWRITES § Numbers lists 9 and 10.

## 8. Resume notes

- **State:** `3e7bee330` on `Moti-Velz/facts-engine-s5-gate` (integration `8960a5db7`, S1 `f1edf7edc`, S3 tie vector, S6 `8bdf55063` merged); gate passes 25/25 checks, 21/21 mutants (`20261022T214833-gate`). Nothing half-done.
- **Successor's next steps:**
  1. **S2's `subst.rs` deletion (not yet landed; S2 will message the switch names).** Re-point `a-subset`/`a3-check`/`a-reference` to rewritten vs unrewritten vs `reference::evaluate_plan`, drop mutants 1–3 from `MUTANTS` (and REWRITES § Numbers), and add `a-reference`/the new differential to the rewrite mutants' families (11–21) so they are also killed on the corpus, not only by unit differentials.
  2. When S6 unblocks the remaining 6 checks (closure evidence, `binds`, `printf`), run `just facts-gate --only harness-b4,harness-b5,suite-b4,suite-b5`, add the exact ones to `GATED`, refresh § 7.2.
  3. When S3 lands ordered aggregates in the generator, extend `b-ties` (or the tie vector) to cover them; the reference side is done.
  4. Ask the decider for a quiet-machine timing run (3 × `--mutants --no-build`); every timing above is untrusted.
  5. When S3 registers the 8 real repository plans, confirm `b-generated` runs them (`--plans` if needed); runtime refusals on real plans fail the gate unless the run declares them expected.
