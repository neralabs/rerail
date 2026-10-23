# Phase 2 S5 report: gate harness

Status: **G2.6 measured and passing** on this branch, with every slice's latest work merged: 13 of 13 seeded mutants killed, gate median 295 s in one command. `--engine facts` reproduces Spike A's batch-6 numbers through Jeremy's harness. Wave 2 (§ 7): batches 1–5 run through the harness in the gate (lane h); 6 of their 50 checks are exact and gated, 40 refuse by name, 4 differ without refusing; with lane h the gate passes in 314 s. Branch `Moti-Velz/facts-engine-s5-gate`, worktree `~/orca/workspaces/rerail/facts-engine/.worktrees/s5-gate`. Raw runs are under `~/rerail-bench/facts/phase-2/s5-gate/runs/` and are cited relative to it.

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

Base: integration `caceb656f` (wave 1 merged). Commits on `Moti-Velz/facts-engine-s5-gate`: `7d23ab4a7` per-batch checks and refusals; `3f6c5fe6d` the `GATED` ratchet and replay coverage.

### 7.1 Results

| Gate row | Target | Measured | Pass/fail | Run dir |
|---|---|---|---|---|
| G2.6 every seeded mutant caught, at `caceb656f` | all | 13/13 killed, 13/13 checks pass. Mutants 1 and 3 again survive `a-reference` and are killed by `a3-check` alone (§ 6) | **pass** | `20261022T203358-gate` |
| G2.6 every seeded mutant caught, lane h added | all | 13/13 killed, 23/23 checks pass | **pass** | `20261022T204421-gate` |
| G2.6 gate time, one command | ≤ 10 min | `--mutants --no-build` under `bench-lock`, three runs back to back: 314 / 335 / 369 s, median **335 s** (loads at start 5.5 / 7.2 / 13.2; only the first below 8, so untrusted). At `caceb656f` without lane h: 465 s with a 169 s build, so ≈ 296 s of checks (load 13). Lane h adds 24 s of work in its own lane; the critical path is still lane b/g (274–328 s, growing with load) | **pass** (untrusted; re-run on a quiet machine) | `20261022T204421-gate`, `…205339-gate`, `…205914-gate`; `20261022T203358-gate` |
| Per-batch `--engine facts`, batches 1–5 | reported; refusals count as failures | 6 of 50 checks exact in both replay and suite, now in `GATED`; 4 differ without refusing (I9 violations, § 7.3); 40 refused. Table § 7.2 | measured | `20261022T204421-gate/harness-b*`, `…/suite-b*` |
| Per-batch suite, incremental phases | from-scratch runs equal to the cold run | `suite.ts --engine facts` without `--cold-only`: for every batch the unchanged and trailing-newline phases give the cold run's FP, FN and failures | measured | `20261022T205039-harness-suites` |

### 7.2 Per-batch parity through the harness (`20261022T204421-gate`)

Replay: `replay.ts --engine facts` over the batch's recordings and probes (as `/tmp/dl60/replay-all.sh` combines them), 317 / 391 / 361 / 573 / 312 calls; batch 4's 3 host-signature-guard calls are evaluated by the host, not the engine. Suite: `suite.ts --engine facts --cold-only` over the oracle `data_layer_suite_record.py` recorded at `2b1504adc` (`/tmp/dl60/suite-b<n>.json`, copied with SHA256SUMS to `~/rerail-bench/facts/phase-2/s5-gate/suites/`): 9,010 / 5,912 / 6,982 / 3,870 / 5,914 files. A suite oracle with 0 findings only shows that the engine adds none. Status: exact, differs (FP or FN, no failure) or refused (any failure: a compile error naming the construct, a harness refusal, a Rust source). The refusal column is the first message.

| batch | check | replay exact calls | replay FP / FN / failures | suite oracle findings | suite FP / FN / failures | status | first refusal |
|---|---|--:|---|--:|---|---|---|
| 1 | `removed_compat_surfaces` | 4/4 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `versioned_lifecycle_legacy_surfaces` | 3/3 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `no_api_root_imports` | 4/4 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `backend_type_strings` | 0/60 | 53 / 48 / 60 | 0 | 0 / 0 / 0 | refused (replay) / exact (suite) | harness: findings carry no column; backend_type_strings compares columns |
| 1 | `hasattr_async` | 26/33 | 7 / 0 / 0 | 0 | 1 / 0 / 0 | **differs** |  |
| 1 | `no_source_inspection` | 69/72 | 3 / 0 / 0 | 0 | 0 / 0 / 0 | **differs** (replay) |  |
| 1 | `serverplan_name_inference` | 8/8 | 0 / 0 / 0 | 0 | 456 / 0 / 0 | **differs** (suite) |  |
| 1 | `no_blanket_grammar_bypass` | 29/121 | 3 / 117 / 89 | 0 | 0 / 0 / 1 | refused | public-methods: unsupported ast key file |
| 1 | `serverplan_grammar_surface` | 7/7 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 1 | `permission_error` | 0/5 | 0 / 3 / 5 | 0 | 0 / 0 / 1 | refused | raise: rule/all/0/not: unsupported rule key `layout` |
| 2 | `dynamic_module_loading` | 0/66 | 0 / 47 / 66 | 0 | 0 / 0 / 1 | refused | pattern-0: regex content gate unsupported |
| 2 | `forwardref` | 0/15 | 0 / 10 / 15 | 0 | 0 / 0 / 1 | refused | fwdref001: pattern `isinstance(annotation, type)` unsupported (only metavariables and single names) |
| 2 | `unresolved_forward_refs` | 0/73 | 0 / 45 / 73 | 0 | 0 / 0 / 1 | refused | inspection: unsupported rule key `arguments` |
| 2 | `classvar_check` | 0/16 | 0 / 11 / 16 | 0 | 0 / 0 / 1 | refused | comparison: unsupported rule key `follows` |
| 2 | `type_safety_patterns` | 0/56 | 0 / 40 / 56 | 0 | 0 / 0 / 1 | refused | text-str_union_expression: regex content gate unsupported |
| 2 | `conn_or_txn_union` | 15/15 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 2 | `async_executor` | 0/47 | 0 / 39 / 47 | 0 | 0 / 0 / 1 | refused | executor: terminators |
| 2 | `async_patterns` | 0/38 | 0 / 29 / 38 | 0 | 0 / 0 / 1 | refused | to-thread-analyzer: unsupported ast key when |
| 2 | `async_sync_boundary` | 0/17 | 0 / 9 / 17 | 0 | 0 / 0 / 1 | refused | sync-wraps-analyzer: unsupported ast key when |
| 2 | `admin_async_purity` | 32/48 | 21 / 0 / 0 | 0 | 0 / 0 / 0 | **differs** (replay) |  |
| 3 | `plan_scope_side_effects` | 0/29 | 0 / 18 / 29 | 0 | 0 / 0 / 1 | refused | eager-value: unsupported rule key `layout` |
| 3 | `plan_scope_ownership` | 0/30 | 0 / 28 / 30 | 0 | 0 / 0 / 1 | refused | runner-no-parameter: unsupported rule key `precedes` |
| 3 | `transaction_closures` | 0/104 | 0 / 48 / 104 | 0 | 0 / 0 / 1 | refused | inline-mutation: unsupported rule key `precedes` |
| 3 | `transaction_callback_wrappers` | 0/20 | 0 / 7 / 20 | 0 | 0 / 0 / 1 | refused | callback-helper: unsupported ast key file |
| 3 | `transaction_patterns` | 0/50 | 0 / 33 / 50 | 0 | 0 / 0 / 1 | refused | category-1: unsupported ast key when |
| 3 | `lazy_result_usage` | 0/17 | 0 / 4 / 17 | 0 | 0 / 0 / 1 | refused | execute: unsupported ast key file |
| 3 | `swallowed_exceptions` | 0/25 | 0 / 23 / 25 | 807 | 0 / 807 / 1 | refused | except-warning: pattern `queue.Empty` unsupported |
| 3 | `error_handling` | 0/13 | 0 / 8 / 13 | 0 | 0 / 0 / 1 | refused | log-reraise: unsupported ast key file |
| 3 | `module_level_caches` | 0/23 | 0 / 21 / 23 | 0 | 0 / 0 / 1 | refused | raw-dict_cache: unsupported ast key file |
| 3 | `connection_caches` | 0/50 | 0 / 34 / 50 | 0 | 0 / 0 / 1 | refused | pattern-0: regex content gate unsupported |
| 4 | `render_fstrings` | 37/37 | 0 / 0 / 0 | 0 | 0 / 0 / 0 | exact, gated |  |
| 4 | `render_raw_strings` | 0/53 | 0 / 50 / 53 | 178 | 0 / 178 / 1 | refused | literal: value key type unsupported |
| 4 | `regex_sql_parsing` | 0/97 | 0 / 68 / 97 | 0 | 0 / 0 / 1 | refused | category-0: value key type unsupported |
| 4 | `sql_grammar_escape_hatches` | 3/163 | 0 / 464 / 160 | 0 | 0 / 0 / 1 | refused | forbidden-class: unsupported rule key `layout` |
| 4 | `grammar_render_methods` | 0/75 | 0 / 58 / 75 | 0 | 0 / 0 / 1 | refused | fstring: unsupported rule key `layout` |
| 4 | `grammar_str_params` | 0/19 | 0 / 25 / 19 | 0 | 0 / 0 / 1 | refused | parameter: unsupported rule key `follows` |
| 4 | `grammar_callable_constructors` | 0/27 | 0 / 30 / 27 | 0 | 0 / 0 / 1 | refused | namespace-render: unsupported rule key `params` |
| 4 | `grammar_fragment_usage` | 0/80 | 0 / 61 / 80 | 0 | 0 / 0 / 1 | refused | forbidden-class: unsupported rule key `params` |
| 4 | `grammar_allowlist_burndown` | 0/8 | 0 / 9 / 8 | 0 | 0 / 0 / 1 | refused | allowlist-allowlisted_static_classes: budget shape unsupported |
| 4 | `rust_sql_grammar_boundary` | 0/14 | 0 / 26 / 14 | 0 | 0 / 0 / 11 | refused | Rust sources are not supported by the facts engine (language rust) |
| 5 | `missing_async_pairs` | 0/15 | 0 / 6 / 15 | 0 | 0 / 0 / 1 | refused | missing-async: unsupported rule key `params` |
| 5 | `sync_async_signatures` | 0/10 | 0 / 4 / 10 | 0 | 0 / 0 / 1 | refused | transaction-connection: parameterised matches unsupported |
| 5 | `async_sync_delegation` | 0/37 | 0 / 16 / 37 | 0 | 0 / 0 / 1 | refused | sync-delegation: value key type unsupported |
| 5 | `public_api_triples` | 0/38 | 0 / 32 / 38 | 0 | 0 / 0 / 1 | refused | exported-families/raw_all: unsupported ast key file |
| 5 | `data_layer_facade_purity` | 0/12 | 0 / 9 / 12 | 0 | 0 / 0 / 1 | refused | expression: value key type unsupported |
| 5 | `data_layer_compat_aliases` | 0/11 | 0 / 6 / 11 | 0 | 0 / 0 / 1 | refused | import-alias: transform LEAF: unsupported shape |
| 5 | `layer_violations` | 0/11 | 0 / 9 / 11 | 133 | 0 / 133 / 1 | refused | layer-0: budget shape unsupported |
| 5 | `backend_coupling` | 0/62 | 0 / 36 / 62 | 0 | 0 / 0 / 1 | refused | backend-import: unsupported ast key file |
| 5 | `internal_dead_code` | 0/43 | 0 / 55 / 43 | 4,931 | 0 / 4,931 / 1 | refused | unreachable-internal-symbols/raw_all: unsupported ast key file |
| 5 | `imperative_private_functions` | 0/73 | 0 / 150 / 73 | 18,771 | 0 / 18,771 / 1 | refused | direct-execution: unsupported rule key `binds` |

Per batch, replay exact calls / checks exact (replay; suite): b1 150/317, 5/10; 6/10. b2 47/391, 1/10; 2/10. b3 0/361, 0/10; 0/10. b4 40/573, 1/10; 1/10. b5 0/312, 0/10; 0/10. Six checks are exact in both, with positive and clean replay coverage: `GATED`.

### 7.3 Findings

- **I9 violation: regex `lines` is accepted and ignored.** `forbid.regex.lines` (`exclude`, `include`, `first`, the `state` machine) compiles in `eval.rs` but is not evaluated, so comment and docstring lines match: `hasattr_async` (7 FP replay, 1 FP suite), `no_source_inspection` (3 FP), `admin_async_purity` (21 FP, the `state` machine), `serverplan_name_inference` (456 FP in the suite, `lines.include`). Reported to S6, who confirms the gap (`leaf.rs` refuses the key, `eval.rs` ignores it) and is porting native's `lines`/`content` evaluation. `no_blanket_grammar_bypass`'s 3 replay FP sit in calls without a refused clause; they are listed under refused because other calls refuse.
- **Rust sources were skipped silently.** `facts-a calls` dropped `.rs` inputs and `facts-a repo` walked `.py` only, so batch 4's `rust_sql_grammar_boundary` would have passed its 0-finding suite oracle. Both now refuse a Rust file a rule's scope selects, by name.
- **`facts-a repo` ran batch 6 only.** `suite.ts --engine facts` evaluated batch 6's rules whatever the recording; it now passes the recording's checks (`--checks`).

### 7.4 Interface changes

- `facts-a repo --checks c1,c2,…` (snake-case check names, default batch 6). A policy that does not compile, or panics compiling, is one failure `{check, clause: "", path: "", message: "compile: …"}` and is not evaluated; before, `repo` panicked.
- `facts-a repo` and `facts-a calls`: a `.rs` file a compiled rule's scope selects yields a failure "Rust sources are not supported by the facts engine (language rust)".
- `facts-a calls`: a panic while compiling a policy is that call's `compile_error`; a panic while evaluating is a failure `engine panic: …` of that call (its findings discarded). `facts-a repo` does not catch evaluation panics: `a3-check` relies on the `FACTS_A3_CHECK` panic failing the run.
- `conformance/data-layer/facts-engine.ts`: `runFactsRepository(files, policies, checks, threads = 4)`.
- Gate: lane h (`harness-b1`…`b5`, `suite-b1`…`b5`); `GATED` in `facts-gate.py` is the ratchet of batch 1–5 checks held exact.

## 8. Resume notes

- **State:** `3f6c5fe6d` on `Moti-Velz/facts-engine-s5-gate`; the gate passes with lane h (`20261022T204421-gate`). S2 agreed to the `repo.rs` call-site edits and will merge them.
- **Successor's next steps:**
  1. When S6 lands a commit (it messages hash and rules): merge it, run `just facts-gate --only harness-b1,…,suite-b5`, add every check exact in both to `GATED` (replay needs a positive and a clean call), and refresh § 7.2.
  2. When S2 deletes `subst.rs`, it messages the switches that replace `FACTS_A3_CHECK`/`FACTS_A3_OFF`. Re-point `a-subset`/`a3-check`/`a-reference` to rewritten vs unrewritten vs `reference::evaluate_plan`, drop mutants 1–3, and let rewrite mutants also run the corpus differentials.
  3. When S1's physical EXPLAIN takes a file (`facts-a explain --rules R [--rule id] --file <path>` or `--root <dir>`, agreed with S1), make `explain-stable` compare per file over the subset. S1 numbers new registry rows' mutants from 16.
  4. When S3 registers the 8 real repository plans (`--real`), confirm `b-generated` runs them, and add `--plans` names if needed.
