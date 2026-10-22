# Phase 2 S5 report: gate harness

Status: **G2.6 measured and passing** on this branch, with every slice's latest work merged: 13 of 13 seeded mutants killed, gate median 295 s in one command. `--engine facts` reproduces Spike A's batch-6 numbers through Jeremy's harness. Branch `Moti-Velz/facts-engine-s5-gate`, worktree `~/orca/workspaces/rerail/facts-engine/.worktrees/s5-gate`. Raw runs are under `~/rerail-bench/facts/phase-2/s5-gate/runs/` and are cited relative to it.

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

The command is `just facts-gate [--mutants] [--only <ids>] [--no-build] [--run-dir <dir>]`. Exit 0 means passed, 1 failed, 2 usage. Four lanes run concurrently:

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

## 7. Resume notes

- **State:** every commit on `Moti-Velz/facts-engine-s5-gate`, nothing half-done in the gate. The integrator merging the slice branches needs the `Db.regexes` fix (in `5f6246912`'s merge) unless S4 lands it first.
- **Successor's next steps:**
  1. When S2's backend executes S1's per-file plans, re-point `a-subset`/`a3-check`/`a-reference`: rewritten physical plan vs unrewritten plan vs `reference::evaluate_plan`. Ask S2 for the switch names replacing `FACTS_A3_*`, and drop mutants 1–3 with `subst.rs`.
  2. When S1's physical `EXPLAIN` takes a file, make `explain-stable` compare per file over the subset.
  3. When S3 registers the 8 real repository plans (`--real`), confirm `b-generated` runs them, and add `--plans` names if needed.
