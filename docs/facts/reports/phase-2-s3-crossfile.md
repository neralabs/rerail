# Phase 2, slice S3: cross-file backend (delta-plan generator)

Branch `Moti-Velz/facts-engine-s3-crossfile`, crate `spikes/facts-b` (module `gen`, `membership`), plus `spikes/facts-a/src/xexport.rs`. Raw runs: `~/rerail-bench/facts/phase-2/s3-crossfile/runs/<timestamp>-<label>/`. Status: **partial**: generator, membership model and mutants done and exact on every plan that compiles; 2 of the 8 repository clauses compile and run (the other 6 are refused, by S1's classifier or by the generator, constructs named in § 5.3); `references`/`calls` wait on S4's unresolved call-graph exports. Timing rows are single unlocked runs (not yet 3× under `bench-lock`).

Scope after the decider's re-steer (Main, 2026-10-22): no hand-compiled plan ships. The deliverable is the **generic delta-plan generator**: an IR repository plan (`rerail_facts::ir::Plan<RepoSource>`, S1) is normalised to delta operators and lowered to one differential dataflow over the membership model. Spike B's hand expansions are the generator's test vectors. Constructs with no exact incremental form are refused with the construct named (INVARIANTS I8, I9).

## 1. Setup

| Item | Value |
|---|---|
| Code | `spikes/facts-b/src/gen/{norm,lower,tags,val,scalar,vectors,drive}.rs`, `spikes/facts-b/src/membership.rs`, `spikes/facts-b/src/mutant.rs` |
| IR | `rerail_facts::ir` and `rerail_facts::compile::compile_report` (S1 `e2aece9c0`); scalar semantics and constant relations through `rerail_facts::reference` (S5 `94e31685f`; my branch carries a 2-hunk stopgap of `reference.rs` for S1's new `StageDef`, superseded by S5 `3f973bae5`), so the dataflow and the reference share one definition of SQLite's expressions |
| Per-file exports (interim) | `facts-a xexport`: S1's `Program.per_file[producer]` run per (path, content), pattern inputs through Spike A's leaf executor, stages and `Sink::Export` relations through the reference evaluator; output JSON lines under `~/rerail-bench/facts/phase-2/s3-crossfile/exports/` (replay: 19,801 jobs, oracle tree: 9,010 jobs, 0 failures) |
| Engine | differential-dataflow 0.25.1, timely 0.31.0, one worker, `u32` epochs, `i32` diffs |
| Corpus | as Spike B: `~/tempo-qe/workspaces47`, worktrees 01–40, store `~/rerail-bench/facts/store`, memberships `~/rerail-bench/facts/spike-b/membership/` |
| Replay | Spike B's `~/rerail-bench/facts/spike-b/edits/seed-5eedb/events.jsonl` (1,000 single-file edits, 5 bulk, 5 new worktrees × 10 edits, 51 real commits) |
| Build | `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target cargo build --release -p facts-b` (gate mutants: `--features mutants`, `FACTS_MUTANT=4|5|6`) |

## 2. Results

| Gate | Target | Measured | Result | Run directory |
|---|---|---|---|---|
| G2.4 oracle parity, `2b1504adc` (suite options) | findings = oracle | `required-contracts` 0 = oracle 0 (`serverplan_target_architecture`); `quality-project-taxonomy` 0 = oracle 0 (`serverplan_quality`); generator vs reference 0 differences; 0 runtime refusals. The other 6 clauses refused (§ 5.3), so no parity claimed for them | 2 of 8 | `20261022-155748-parity-real` |
| G2.4 incremental ≡ from-scratch, 1,000-edit replay, real plans | 0 differences | `required-contracts`: **0** differences over 1,112 checks / 1,576 worktree checks (30 findings per initial worktree, so the plan is exercised); both real plans together: 0 over 121 checks / 200 worktree checks (`--limit 120`) | pass (for the compiled plans) | `20261022-153809-exactness-real`, `20261022-155750-exactness-real-2` |
| Generator on Spike B's R1/R3 written as IR (test vectors) | 0 differences | **0** differences over 1,112 checks / 1,576 worktree checks vs Spike B's oracle | pass | `20261022-151755-exactness-vectors` |
| G2.6 mutants in the delta rules | each caught | mutant 4 (join drops ΔR⋈ΔS) 65 differences; 5 (presence ignores retractions) 65; 6 (recursion one step) 41; unmutated build 0 | pass | `…-152110-mutant-0`, `…-152148-mutant-4`, `…-152202-mutant-5`, `…-152215-mutant-6` |
| G2.4 resident, 40 worktrees, real plans | ≤ 250 MB | 87.4 MB RSS after 1,000 edits (69.9 MB footprint), required-contracts only, `--track` on (single run, unlocked) | pass (as measured; needs 3 locked runs) | `20261022-153809-exactness-real` |
| Resident, vectors R1+R3 through the generator | (B3: 195 MB hand-written) | 702 MB RSS after 1,000 edits in an oracle run (v0 store 88 MB + generic `Vec<Val>` rows + boundary re-arrangement inside the fixpoint); not optimised | open | `20261022-151755-exactness-vectors` |
| G2.4 per-edit latency (single-file edits, n = 1,050) | reported | real plans p50 0.106 / p95 0.153 / p99 0.350 / max 3.0 ms; vectors p50 0.44 / p95 0.73 / p99 1.32 ms; real commits (vectors) p50 11.0 / p95 48.0 / max 133.8 ms | reported (unlocked) | same runs |
| G2.4 per-worktree state vs divergence | reported | `results.json` → `state` (live records per tag over every operator output) and `divergence_initial` (delta entries and exported rows per worktree) in `20261022-153809-exactness-real`; the 8 cluster bases cut weighted divergence 269,295 → 76,795 rows | reported | same |

## 3. How measured

`spikes/facts-b/scripts/run.sh <label> [--lock] -- greplay [--events $E] [--plans r1,r3|none] [--real SPEC] [--parity JOBS] [--flat] [--track] [--fail-fast] [--no-oracle]` with `FACTS_RUN_ROOT=~/rerail-bench/facts/phase-2/s3-crossfile/runs`. Exports: `python3 spikes/facts-b/scripts/jobs.py replay|oracle …`, then `target/release/facts-a xexport --rule R --clauses C [--config J] --jobs JOBS --out OUT.jsonl`. Specs used: `exports/spec-replay.json`, `exports/spec-oracle.json`. Mutants: `CARGO_TARGET_DIR=$PWD/target-mutants cargo build --release -p facts-b --features mutants`, `FACTS_B_BIN=$PWD/target-mutants/release/facts-b FACTS_MUTANT=n run.sh mutant-n -- greplay --events $E --limit 120 --fail-fast`.

## 4. Exactness artifacts

`results.json` → `exactness` (`checks`, `worktree_checks`, `differences`, `first_differences`), `runtime_refusals`, `refused_static`.

## 5. Design notes

### 5.1 The generator

`norm.rs` maps the IR to delta operators (table in its header): `Exported`, `Const` (constant subtrees evaluated once by the reference evaluator), `Map`, `Filter`, `Join` (equality conjuncts of `on` as keys, the rest a residual filter), `Semi` (EXISTS / IN, decorrelated on equality correlations; NOT IN with SQLite's NULL rules), `Distinct`, `Concat`, `Group` (GROUP BY; bare non-key columns refused), `Window` (one function per operator), `DistinctPrefix`, `Fix` (set recursion). Scalar subqueries over aggregates and `ORDER BY … LIMIT 1` become a group or window on the correlation keys plus a left join with the empty-group value. LEFT JOIN = inner join ∪ NULL-extended antijoin of the left side by row identity.

`lower.rs` gives each operator its base + delta expansion over tagged records `(tag, row)` (tag tree in `tags.rs`: base 0, cluster bases, worktrees):

| Operator | Expansion |
|---|---|
| map, filter | per record, tag kept |
| join | one `join_core` over two arrangements; pair (t1, t2) ↦ `product(t1, t2)` = the deeper tag of one chain, nothing across chains (A_B⋈B_B, ΔA⋈B_B, A_B⋈ΔB, ΔA⋈ΔB) |
| presence (DISTINCT; right side of semi/antijoins) | per key, counts per tag; base emits [c_0 > 0]; tag t emits [below + d_t > 0] − [below > 0], below = Σ counts of t's ancestors |
| semijoin / antijoin | left ⋈ presence(right keys); antijoin = left − semijoin |
| GROUP BY, window, DISTINCT prefix | group recompute: per key, out(t) = logic(view(t)) − logic(view(parent(t))) for tags with members at the key; base emits logic(base); keyless aggregates carry a base sentinel so the empty group exists |
| set recursion | per view: the base view and every view whose tag has a nonzero delta record in the fixpoint's inputs ("touched"); semi-naive `iterate` over view-tagged records, joins with delta-mode relations filtered by tag visibility (Σ of visible tags = the view's multiplicity); back to delta mode as R_v − R_parent-view |

Runtime refusals (counted in `results.json` → `runtime_refusals`, findings withheld, never guessed): `row_number()` ties between distinct rows, LIMIT boundary ties, DISTINCT-prefix groups whose trailing columns differ, `sum`/`total` over several non-integer values (input-order rounding), GROUP BY keys with several representations (1 vs 1.0), scalar errors.

### 5.2 Membership model

`membership.rs`: per-path base (each path's most frequent content among the paths held by a majority); cluster bases chosen by an exact dynamic program over contiguous runs of worktrees (in name order = commit date), minimising the exported rows the deltas carry; worktree deltas relative to their cluster; new worktrees attach to the level they differ least from. Resident exported rows are reference-counted by the (tag, path, content) records and evicted when no tag holds the pair.

### 5.3 Repository clauses: exported relations and constructs (sent to S4 and S1)

The 8 repository relational clauses (default partition, or `partitionBy` by `$option`): `internal-dead-code/unreachable-internal-symbols`, `public-api-triples/exported-families`, `serverplan-quality/quality-conditional-1`, `/quality-project-taxonomy`, `serverplan-state-tripwires/state-flow-project-1`, `/state-flow-1` and `/state-projection-recovery-2` (repository only under `entryPoint=scan_project_sources`), `serverplan-target-architecture/required-contracts`. Cross-file pattern-DSL terms: `imperative-private-functions/caller` (`calls{graph:references}`), `grammar-callable-constructors/constructor-arg` and `sql-grammar-escape-hatches/unsafe-parameter` (`closure` without `partition_by`), `grammar-allowlist-burndown/*` (`budget` without `group_by`).

Under the repository suite's options (no option values: declared defaults) the oracle at `2b1504adc` has: internal_dead_code 4,931 findings, imperative_private_functions 18,771, public_api_triples 0, serverplan_quality 0, serverplan_state_tripwires 0, serverplan_target_architecture 0 (`/tmp/dl60/suite-b5.json`, `suite-b6.json`). `state-flow-project-1`'s inputs are gated by a constant-false file filter under `entryPoint=all`; `state-flow-1` and `state-projection-recovery-2` are file-partitioned under it.

Refusals as written (no exact incremental form: input-order dependent), each with the narrowing proposed to Jeremy:

| # | Clause / stage | Construct | Narrowing |
|---|---|---|---|
| 1 | exported-families `slots` | `row_number() OVER (PARTITION BY scope,owner,base_name,slot ORDER BY ordinal DESC)`: ties across files | `, symbol` |
| 2 | exported-families `family_groups` | exemplar `ORDER BY x.first_ordinal LIMIT 1`: ties | `, x.slot` |
| 3 | dead-code, exported-families `declarations` | `row_number() OVER (PARTITION BY id ORDER BY __start DESC)`: ties across files on module collision (`pkg.py` vs `pkg/__init__.py`) | `, __path` |
| 4 | dead-code, exported-families `imports` | `row_number() … ORDER BY ordinal DESC`: ties inside one statement | add the token index |
| 5 | dead-code `base_edges` (×2), `class_names` (×2) | `LIMIT 1` without `ORDER BY` | `ORDER BY c.id` |
| 6 | dead-code, exported-families `hierarchy` | recursive CTE `UNION ALL` (bag) enumerating inheritance paths, then `row_number()` by (depth, route) | lexicographic shortest path (argmin of (depth, route) per (owner, base)) as a set recursion |
| 7 | state-flow `transitive_sinks` / `project_sinks` (per file) | `row_number() ORDER BY length(route), n.__start`: ties with different origin | `, s.route, n.__end` |
| 8 | dead-code `reference_edges`, caller `calls` | native whole-workspace reference graph (`calls/closure.rs` linker) | needs the graph's per-file summaries as exported relations (S4, schema § 3 (A)) and its linking rules as IR; until then refused |

The generator enforces 1–7 at run time (ties counted as refusals) or at compile time (6: bag recursion; `json_group_array`/`group_concat` in input order).

Compile status per repository clause (S1 `e2aece9c0`, suite options `{}` unless noted):

| Clause | Status | Refused construct(s) |
|---|---|---|
| required-contracts | compiles; runs on the dataflow; parity 0 = 0; replay 0 differences | — |
| quality-project-taxonomy | compiles; runs; parity 0 = 0; replay 0 differences (120 events) | — |
| quality-conditional-1 | refused (S1) | `conditional_findings_0`: LIMIT under a non-key ORDER BY |
| state-flow-project-1 | refused (S1) | `row_number()` in `direct_sinks` / `project_sinks` (repository side, downstream of the cross-file `taints` join): order not provably total (#7) |
| exported-families | refused (S1) | #1–6: `row_number()` ×6 stages, LIMIT without ORDER BY, `json_group_array` order |
| unreachable-internal-symbols | refused (S1) | #3–6 + `reference_edges` (#8) |
| state-flow-1, state-projection-recovery-2 | file-partitioned under `entryPoint=all`; repository under `scan_project_sources` (compile clean there per S1; exports not completed: the per-file side through the naive reference evaluator was too slow for the 19,801-job replay set) | — |
| imperative-private-functions/caller (`calls`) | refused (S1): `calls` atom | #8 |

## 6. Risks found

- 6 of the 8 clauses are refused as written: the narrowing list (§ 5.3 #1–8) goes to Jeremy. The generator accepts the narrowed forms (argmin with a total order, set recursion), so each narrowing turns into a compiled plan without generator work, except `reference_edges`/`calls` (#8), which need the call graph as IR.
- Memory of generic rows: `Val` is 16 bytes and rows are `Vec<Val>`; the R1/R3 vectors through the generator hold 702 MB vs Spike B's hand-written 195 MB. Levers, in size order: drop the v0 store when no vector runs (done for real plans: 87 MB); share outer arrangements with the fixpoint instead of re-arranging the boundary inside `iterate`; pack `Val` to 8 bytes and rows to `Box<[Val]>`; arrange each exported relation once per key.
- Reference-evaluator cost: the per-worktree checks on real plans took 962 s for 1,576 checks (required-contracts); per-file producers through the naive evaluator are too slow for the state-flow clauses' replay set.
- 7 (path, content) pairs of the replay set have no retained source (Spike B's edit script); their exports are absent and counted as runtime refusals (`no exports for …`) in `exactness-real`, on both sides of the comparison.
- Module paths (S4's finding, decider's design): native's summaries qualify names by module path, which depends on the worktree's `__init__.py` set. Exports must stay unresolved and content-keyed; `module_of(path)` becomes a membership-dependent relation computed in the dataflow (algebra over the path set: a path's package chain = its ancestor directories holding an `__init__.py` member), and qualified names a join `module_of ⋈ def`.

## 8. Wave 2 (S3CrossFile-2)

Status: **partial**.

- NARROWINGS.md is done and committed. The decider ruled on order constructs: runtime-checked order, retractable runtime refusals and mutant 9. That ruling is implemented and exact on the vectors.
- Generator memory improved from 539 to 354 MB, but that is still 1.76× the hand-written version. The 1.5× target is not met.
- Call-graph exports (S4) have landed, and the port of the native linker to SQL is in progress (a subagent).
- The state-flow clauses under `scan_project_sources` have no oracle: native itself exceeds its budget on them.
- The G2.4 rows were not re-run under the 3-run protocol.

Runs are in `~/rerail-bench/facts/phase-2/s3-crossfile/{runs,narrowings}/`.

### 8.1 Results

| Gate / item | Target | Measured | Result | Run directory |
|---|---|---|---|---|
| NARROWINGS.md committed | doc + evidence | `docs/facts/NARROWINGS.md`, `spikes/facts-b/scripts/narrow.py` (`324da60e3`). Through native SQLite with the narrowed text: `unreachable-internal-symbols` 4,931 = 4,931 (0 FP, 0 FN, 0 failures); `exported-families` 0 = 0; `quality-conditional-1` 0 = 0; `serverplan_state_tripwires` 0 = 0. All 15 edited stages are identical to the originals (0 rows differ). Ties under the original order: 11, all in `slots`. Ties under the narrowed order: 0 | done | `narrowings/20261022-211541-b5-final`, `…-205531-b5-traits`, `…-205542-b6-diag`, `…-210545-b6-trip-diag` |
| Order constructs (decider: option B) | named and retractable, never guessed | Classifier marks `Proven`/`Checked` (S1 `0ea181a84`, merged). Generator: every refusal raised inside an operator is a dataflow record `(t \| REFUSED, [construct])`, maintained per view like the findings, captured per (plan, worktree), and withholds that plan's findings. Constructed-tie vector `rt`: 280 of 280 expected refusals refused, 0 missing, 0 differences, 201 checks (r1, r3 and rt, 200 events). Mutant 9 disables tie detection. The reference side (`tie:` errors) is S5 `c176eefc7` | implemented (the replay does not exercise retraction: the corpus ties `rt` in every worktree) | `runs/20261022-212907-exact-vec-r1r3rt` |
| Generator memory: R1+R3 vectors vs hand-written | ≤ 1.5× hand | Generator **354.4 MB** RSS at edit 1,000 vs hand-written (`replay --clusters`, R1–R3) **201.0 MB**: **1.76×**. Both runs locked and back to back; load 15.7 and 14.1. Before wave 2: 539 MB (no row cache), 669 MB (row cache). Arranged records: 3.19M → 1.66M (hand: 3.13M) | **fail (open)** | `runs/20261022-213008-mem-vec-r1r3-final`, `runs/20261022-213022-mem-hand-clusters-final` |
| Exactness after the memory changes | 0 differences | 0 differences over 201 checks / 280 worktree checks (r1, r3, rt; 200 events), with and without the row cache | pass | `runs/20261022-210546-exact-vec-shared`, `…-210634-exact-vec-shared-nocache`, `…-212907-exact-vec-r1r3rt` |
| `state-flow-1`, `state-projection-recovery-2` under `entryPoint=scan_project_sources` | run if feasible | Native itself fails all three repository variants on the 9,010-file corpus: relational work budget exceeded in `state-flow-1`, `state-flow-project-1` and `state-projection-recovery-2`, so no oracle exists. `state-flow-1` also stays refused by S1 for bag recursion until the narrowing is applied. Not run on the engine | blocked (no oracle) | `narrowings/20261022-211324-b6-trip-orig-sps` |
| G2.4 rows, 3× locked | rows with run dirs | Not re-run in wave 2. The rows of § 2 (wave 1, unlocked) still stand | open | — |
| Call graph as algebra | `reference_edges` and `calls` compile | Native's references-mode graph is written as 79 SQL stages (`spikes/facts-b/linker/reference_edges.sql`, each mirroring a native `file:line`). They read S4's content-only exports of native's summary (`cgn_*`, `145cd8f96`) and the member path set: `module_of` via the ancestor `__init__.py` chain, module substitution, `link_python` branches 1–12 with first-match precedence, MAX_HOPS as a hop range, base-class BFS order from the `<base-slot>` ordinals, then `resolve_edges`. On `2b1504adc` (Python sqlite3 harness `linker_check.py`) native has **145,431 edges and the SQL 145,431, with 0 differences**. `reference_graph_incomplete` has 0 rows and every refusal relation is empty. The refusals are row-producing relations for the slot-order cases (first of several alias targets, bases from several files, `own_name`), module and substitution cases, and stage bounds; none occurs on the corpus, so they are untested. Native links only the files in the installed rules' scope (3,110 for the dead-code rule); the stage `in_scope` models one rule and must become the union with several. **Not yet** compiled through S1's front end or run on the generator: several recursive arms in one `WITH RECURSIVE` (`link_reach`, `class_of_walk`) may exceed the engine's subset | SQL exact; IR/generator pending | `linker/20261022T215523-corpus-2b1504adc` (native dump: `linker/native-ground-truth/`) |

### 8.2 Where the memory goes (attribution)

Arrangement records come from differential's logging, per named arrangement (`results.json` → `arrangements`).

Removed:

- **Per-view `touched` threshold over full boundary rows**: 908K records. Touched views are now the tags with file deltas. That is a superset, and exact, because R_v − R_pv = 0 for an untouched view.
- **Duplicate arrangements**: 686K records. The fixpoint body now joins the delta-mode arrangements (entered), shared by (node, key), and hash-consed nodes share collections and arrangements.
- **The row cache of exported rows**: 107–191 MB (`FACTS_ROW_CACHE=0`; rows are recomputed on retraction).

Remaining: generator 1.66M records at ~150 B per record, against 40 B per record for the hand-written version. The levers, in size order:

1. `Val` packed to 8 bytes, and rows as inline small vectors (`Vec<Val>` costs about 24 + 16·k + allocator overhead per row, against u32 tuples by hand).
2. A second string interner duplicating the v0 store's symbols (23.6 MB).
3. RSS growth during edits (285 → 354 MB, against 197 → 201 for the hand version), probably allocator retention in batch merges.

### 8.3 Interface changes

- `lower::Built.refusals: Vec<(plan, Out)>`; `lower::REFUSED`.
- `results.json` gains `runtime_refusals_live`, `runtime_refusals_seen`, `expected_refusals` and `missing_expected_refusals`, as agreed with S5. Mutant 9 is tie detection off.
- `greplay --plans rt` runs the constructed-tie vector.
- `norm.rs`: hash-consing (`Ctx.seen`). `group_eq` no longer patches nodes in place. `NOT IN` keeps a row with a NULL operand when the correlated set is empty.

## 7. Resume notes

- Commits:
  - wave 1: `ed0c9add9`, `bf3685133`.
  - wave 2: `324da60e3` (NARROWINGS), `806231a84` (shared arrangements, hash-consing, touched from files, NOT IN NULL, named arrangements), `7be88047a` (merge of S1 `0ea181a84`), `e7d1e5c70` (retractable refusals, mutant 9, `rt`), `5d0bf0e18` (report), `3eb776377` (merge of integration `8960a5db7`), `c0ef939fb` (linker SQL + harness), plus this update.
- `just facts-gate` was **not** run on wave 2's engine changes. Run it, and `--mutants` with S5's mutant-9 entry, before merging.
- Next, in order:
  1. Memory to ≤ 1.5× the hand-written version: pack `Val` to 8 bytes (low two tag bits; out-of-range ints and reals interned) and make `Row` an inline small vector; reuse the v0 store's symbols for the vectors. Then measure locked against `replay --clusters`.
  2. G2.4 rows, 3× under `bench-lock`, for the compiled plans (`greplay --plans none --real exports/spec-replay.json --no-oracle --track`), plus the oracle-mode exactness run.
  3. Run the linker on the generator: compile `spikes/facts-b/linker/reference_edges.sql` through S1's front end as a repository plan over the `cgn_*` exports and a member-path export (split multi-arm recursions if the subset refuses them), run it on greplay, and compare with `linker/native-ground-truth/reference_edges.tsv`. Then `reach_edges` and the `calls` atom read it (the atom is the reflexive backward closure over non-class defs, spec § 5).
  4. Ordered `group_concat`/`json_group_array` in the generator: the order comes from the Sort below, through Project and Filter, with tie = refusal, matching S5's reference. `exported-families` needs it.
  5. Narrowed clauses on the engine once S6's `precedes`/`binds` leaves exist.
  6. Switch `norm.rs` decorrelation to S1's `ir::semi_join`/`equi_keys`, as S1 asked.
- Generator code map: `gen/norm.rs` (IR → delta operators, decorrelation, refusals, hash-consing), `gen/lower.rs` (delta expansions, shared arrangements, fixpoint per touched view with entered boundary arrangements, retractable refusals), `gen/tags.rs`, `membership.rs`, `gen/real.rs`, `gen/vectors.rs` (R1, R3, RT as IR), `gen/drive.rs` (greplay).
