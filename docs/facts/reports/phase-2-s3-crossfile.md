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

## 9. Wave 3 (S3CrossFile-3)

Status: **partial** (run budget reached). The linker and `file.module` compile through the front end; with `narrow.py`'s policies all four narrowed rules compile with the leaf executor (0 refused constructs, census `/tmp`-free copy below); the linker plan normalises on the generator with 0 refusals; its generator parity run and every G2.4 protocol row are not finished.

### 9.1 Results

| Gate / item | Target | Measured | Result | Run directory |
|---|---|---|---|---|
| Linker SQL in IR form (per-content inputs, scope as a relation, module stages split out, no scalar subqueries / non-equality correlations / `digit` joins) | = native | SQLite harness over the per-content form: **145,431 = 145,431, 0 differences**, 0 refusals, after each rewrite step | pass (SQL) | `linker/20261022T225909-ir-form-…`, `…T231744-ir-form-scope-…`, `…T232415-views-split-…`, `…T235216-genform-2b1504adc` |
| Linker through `rerail_facts::compile` | compiles | `references` lowers to `views/*` stages (module_path.sql + reference_edges.sql) over `FileSource::Summary` and `file(path)`; classifier cuts 14 exports; generator: **0 refusals** after the rewrites (before: scalar-subquery shapes ×2, non-equality correlations ×5, subquery in ON) | pass (compile) | probe rule `exports/w3/probe/linker-probe.json`, exports `exports/w3/oracle-linker-probe.jsonl` (9,010 jobs, 0 failures, 11.5 s) |
| Linker on the generator, parity 2b1504adc | 145,431 edges | **did not finish**: the base load of one 9,010-file worktree ran 3 h 8 min without reaching `loaded` and was terminated (exit 143). The generator's fixpoint per touched view is the suspect, with `link_reach` (264,716 rows; 54 s even in SQLite) as the largest recursion; needs profiling (per-stage arrangement sizes) before any rerun | **open, too slow** | `runs/20261023-002532-linker-parity` |
| Narrowed clauses compile (leaf executor, after S6's `binds` merge `d47f859c4`) | 8/8 | `unreachable-internal-symbols`, `exported-families`, `quality-conditional-1`, `state-flow-project-1`: compile, 0 refused constructs; `required-contracts`, `quality-project-taxonomy` as wave 1 | compile pass | census of `narrow.py` output (re-run: `facts-a census --rules <narrow.py OUT>`) |
| Narrowed clauses: exports | 0 failures | `exported-families` oracle 9,010 / replay 19,801 jobs, 0 failures; `state-flow-project-1` 9,010 / 19,801, 0 failures; `quality-conditional-1` 460 s per 200 jobs through the reference evaluator (too slow for 28,811 jobs) | partial | `exports/w3/{oracle,replay}-{exported-families,state-flow-project-1}.jsonl` |
| Generator memory, R1+R3 vectors at edit 1,000 (3 locked pairs, generator then hand-written back to back) | ≤ 300 MB (≤ 1.5× of hand) | generator 283.4 / 281.1 / 273.6 MB vs hand-written 199.7 / 200.7 / 199.9 MB = **1.42× / 1.40× / 1.37×** (load 8–24); r1/r3/rt exactness 0 differences, 280/280 refusals; full 1,000-event r1/r3 oracle run 0/1,576; mutants 4/5/6/9 caught (`e8384e84b`: 8-byte `Val`, 24-byte `Row`, one string table) | **pass** | `runs/20261023-003548-mem-vec-r1r3-tip-p1` + `…-004057-mem-hand-clusters-tip-p1`, `…-004101`/`…-004111` (p2), `…-004115`/`…-004125` (p3); `…-003443-exact-vec-r1r3rt-tip`; `…-002638-exactness-vectors-packed`; `…-0043xx/0044xx/0045xx mutant-*-tip` |
| G2.4 protocol rows (3× locked) | rows | not run | open | — |

### 9.2 Interface changes

- `ir::FileSource::Summary { rel: SummaryRel, links }` / `AnySource::Summary`, `AnySource::Core`; `ir::SummaryRel` (`cgn_module`, `cgn_def`, `cgn_edge`, `cgn_alias`, `cgn_reference_alias`, `cgn_opaque`, `cgn_open`). `AnySource::ReferenceEdges` removed: `reference_edges` is a view.
- `ir::Func::Refuse` (`rerail_refuse(construct)`): a named refusal raised by evaluation; `reference_edges` unions it over `linker_refusal` and `reference_graph_incomplete`.
- `facts/src/front/module_path.sql` (`module_of(path, module, package)` over member paths and the rule's scope) and `facts/src/front/reference_edges.sql` (moved from `spikes/facts-b/linker/`), compiled ahead of a clause's stages by `front::rule::Views`; relational inputs projecting `file.module`/`file.package` become `inputs/<name>` stages = leaf without those fields ⟕ `module_of` (membership side).
- `xexport`: serves `file(path)` for every member (in or out of scope) and `cgn_*` via `cgnative::{summary, links_key, compacted}`.
- `linker_check.py`: per-content inputs, `rule_scope`, `--sql` list.
- Generator (`norm.rs`): EXISTS anywhere in an expression as `count(*) > 0` / `= 0` over the correlated body; a refused stage gets an empty stand-in so all refusals are named.

### 9.3 Decisions received (decider)

- Budget-evidence `closure`: per-content export `<clause>/closure_cost(path, incomplete, steps)`; repository aggregate `any(incomplete) ∨ sum(steps) > maxVisitedNodes` (2,000,000, `capabilities.json`) → clause-level retractable refusal through the runtime-refusal record. S3 owns the IR export and the aggregate (S6: `spikes/facts-a/src/closure.rs` `Closure::derive` counts steps, must return them; native `query/name_closure.rs:69-157`, `eval/mod.rs:581-594`, `workspace/global.rs:157-170` — a syntax-error file is incomplete on the workspace path).
- `constructor-arg`, `unsafe-parameter`: repository plans outright.

## 10. Resume notes (wave 3)

- Commits: `c697a1721` (views, summary source, Refuse, generator EXISTS), merge of integration `d47f859c4`, this report. GenMemory commits `gen/{val,lower,drive,real,vectors}.rs` separately (its working copies are in this worktree, uncommitted at hand-off). `just facts-gate` **not run** on wave 3.
- Next, in order:
  1. Read `runs/*-linker-parity/results.json`; compare `parity_rows` with `linker/native-ground-truth/reference_edges.tsv`. If the base load is too slow, the cost is `link_reach` (264,716 rows, 54 s in SQLite): candidate is the per-view fixpoint over the 3,110 in-scope files.
  2. `calls{graph: references, at: definition}` atom (`imperative-private-functions/caller`): native `calls/mod.rs:1380-1400` answers at a definition by `def_at(path, node.start)`, then `verdict_of([def])`: reach = backward BFS from seeds over callers whose flags `through` allows (`closure.rs:1152 reach`, `close` 2579); `open` (a callee unknown or file open) → Unknown (no finding). Lowering: split the input at its top-level `calls` conjunct into residual + seed inputs, repository plan = residual ⋉ (reflexive closure over `reference_edges` restricted to allowed defs, reaching a seed) minus open. Not started.
  3. Run the 6 narrowed/compiled clauses on greplay (oracle + replay specs, exports in `exports/w3/`); `unreachable-internal-symbols` exports need the merged binary; `quality-conditional-1` needs a faster per-file executor than the reference evaluator.
  4. Closure-cost export + aggregate (§ 9.3).
  5. G2.4 rows 3× locked; message S5Gate-3 the command and `results.json` fields.

## 11. Wave 4 (S3CrossFile-4)

Status: **partial**. The linker now runs on the generator in usable time and exactly: base load 50.1 s, 145,431 = native. Main's I6 fix is committed and gated. 6 of the 8 clauses are still blocked on generator decorrelation gaps (§ 11.3). The G2.4 protocol rows were not run.

### 11.1 Results

| Gate / item | Target | Measured | Result | Run directory |
|---|---|---|---|---|
| Cause of the 3 h "load" | with numbers | Not a load at all. It was **dataflow construction**: `sample` of the stuck process showed 100 % of the time in `lower::reads_var`, an unmemoised recursive walk of the hash-consed plan DAG. The walk is called from `boundary`, `boundary_arrs` and `ViewLower::is_view` for every fixpoint. Calls, as a lower bound per fixpoint (931 plan nodes): `link_result` **1.06e17**, `plain_prefix` 1.7e14, `fq_class` 6.3e12, `class_of_walk` 3.1e12, `order_dist` 1.2e12, `link_reach` 6.0e10. The 9,010-file run never reached `loaded`, and a 514-file subset ran the same way for 7 min | cause found | `runs/20261023-034130-linker-sub500-base` (killed), `…-034723-linker-sub500-prof` (killed), notes in `…-042347-linker-full-memo-prof/profile.json` |
| Generic fix | linear | `var_readers`: one pass in node-id order, because ids are topological (asserted). `boundary` visits each node once. Dataflow construction now takes **0.01–0.02 s** | pass | same |
| Base load = plain semi-naive | no delta expansion without deltas | The fixpoint is now **delta-mode** (`IterLower`): differential `iterate` over tagged records, with the same bilinear and presence expansions as outside, and each round's distinct a count-based presence. With base records only, every product is base and presence is a threshold. The per-touched-view materialisation (touched set, chain seeds, `result-by-view` R_v − R_pv) is deleted, so per-worktree fixpoint state is proportional to divergence, not to view size. b-generated (r3 recursion) 0 differences; mutants 4/5/6 killed | pass | gate `s5-gate/runs/20261023T043646-gate` |
| Linker base load, full worktree (9,010 files, 3,110 in scope) | ≤ 10 min | **69.0 s** locked (load 5.1) after the memo fix and delta fixpoint; **50.1 s** locked (load 2.9) with R15. RSS 9.66 → **7.43 GB**. With profiling: 143 s | **pass** | `runs/20261023-045237-linker-full-7fa1`, `runs/20261023-045802-linker-full-r15` |
| Linker exactness on the generator | 145,431 = native | **145,431 = 145,431, 0 missing, 0 extra**, 0 runtime refusals (`scripts/edges_parity.py`), in all three runs above | pass | `edges-parity.json` in each run dir |
| Where the load goes (profile, `FACTS_PROFILE=1`, before R15) | per stage | busy 112 s: `link_reach` 68.1 s (420 iterations; 2.66M rows; 3.45M derivations; the bulk in iterations 1–20, then a tail of ~100 rows per iteration), `name_sub` 15.2 s (3 iterations, 1.27M rows), all others ≤ 3.2 s. The top operators are key-extraction `FlatMap`s of `link_reach`'s join arms (19.1, 11.2, 10.6 s): regex keys evaluated on every recursive row because the arms' single-side `WHERE` conjuncts ran after the join. Arranged records after R15: 58.8M (presence in+out of `link_reach` 5.3M, `iter-join-l` arms 2.3M/2.0M/1.1M/1.1M); 4.26M interned strings (721 MB) | measured | `runs/20261023-042347-linker-full-memo-prof/profile.json`, `…-045802-linker-full-r15/results.json` → `arrangements` |
| R15 filter pushdown (generic; REWRITES.md, mutant 24) | registered | `norm::push_filters`: a single-side `WHERE`/`ON` conjunct filters that join input. Restrictions: right side only under inner joins for `WHERE`; right side only under LEFT joins for `ON`. Never applied to subqueries, correlations or `rerail_refuse`. Differential test `norm::tests::rewrite_filter_pushdown_equals_unpushed` (reference evaluator before vs after, 200 random table pairs, ≥ 1,000 firings) and `…_side_conditions`. `FACTS_MUTANT=24` fails both. Gate: new check `facts-b-unit`, family `facts-b` in `facts-gate.py` | pass (gate run in § 11.2) | — |
| I6 (Main's priority) | leak tests green, mutant 7 live | `AnySource::Members(CoreRel)` / `AnySource::Summaries{rel,links}` replace `Core`/`Summary`, which only the views produced. `classify::file_source` refuses both in a per-file plan, naming `file.module` / `reference_edges`. `export_to_file` maps them to the per-content leaf on the file side of a repository clause. Mutant 7 also fires on them. A refused repository clause no longer yields a plan (its `Rel::Unit` stand-ins could reach an executor through `compile_report`). New test `repository_clause_over_reference_edges_exports_its_membership_leaves`. Gate **27/27, 23/23 mutants** | pass | commit `7fa1067ec`; gate `20261023T043646-gate` |
| Clauses on the engine, oracle parity | see target | `required-contracts` 0 = 0 and `quality-project-taxonomy` 0 = 0 (exports regenerated with the merged binary; reference check 0 differences). `unreachable-internal-symbols`, `exported-families` and `state-flow-project-1` are refused by the generator (§ 11.3). `quality-conditional-1`: per-file export through the reference evaluator still running at hand-off. `state-flow-1`, `state-projection-recovery-2`: no oracle (native exceeds its budget, wave 2) | 2 of 8 | `runs/20261023-050620-parity-light-w4`, `…-050657-parity-dead-code-w4` |
| G2.4 protocol rows (3× locked) | rows | not run | open | — |
| Closure-cost aggregate (`captured-return`), `calls` atom | implemented | not started | open | — |

### 11.2 Commits and gate

- `7fa1067ec` (s3): I6 typed membership leaves, no plan for refused clauses, linear `var_readers`, delta-mode fixpoint, `FACTS_PROFILE` profiler (`gen/prof.rs`), `scripts/edges_parity.py`. Gate 27/27, 23/23.
- R15, the gate registration (`facts-b-unit`, family `facts-b`, mutant 24) and this report: gate **28/28 checks, 24/24 mutants** (mutant 24 killed by `facts-b-unit`; `b-generated` does not reach it), `s5-gate/runs/20261023T045924-gate`. The release binary in that run was built before the `expr_any`/`rel_any` refactor of `expr_free`/`rel_free`, which is semantics-preserving; the facts-b test binary includes it.

### 11.3 Generator refusals left (generic decorrelation gaps; exports ready in `exports/w4/`)

| Clause / stage | Construct | Generic rewrite needed |
|---|---|---|
| unreachable-internal-symbols, exported-families `base_edges` | scalar subqueries (`coalesce` of three) in an inner JOIN's `ON`, correlated with the left side only | compute them as columns of a `Project` over the left input (their depth-1 refs are left columns, the same indices), then key the join on the new column; check with `expr_any` (added) that no subquery reads a right column |
| unreachable-internal-symbols `high_confidence` | correlation `m.name = d.name` inside an inner join's `ON` within an EXISTS body | lift correlated conjuncts out of inner-join `ON`s and inner-join inputs to the body's top `WHERE` before `decorrelate` (exact for inner joins) |
| exported-families `traits`, state-flow-project-1 `direct_events` | disjunctive correlation `(a1 ∧ b1) ∨ (a2 ∧ b2)` in EXISTS | EXISTS(σ_{(d1∨d2)∧r}) = EXISTS(σ_{d1∧r}) ∨ EXISTS(σ_{d2∧r}) |
| exported-families `family_groups` | correlated filter under a derived table inside an aggregate (`json_group_array` over an ordered subquery) | pull the correlated filter through Project/Sort to the aggregate input |
| state-flow-project-1 `taints` | uncorrelated EXISTS under OR in a JOIN `ON` | constant subquery → cross join with its count |
| state-flow-project-1 `direct_sinks` | correlated count in a window `ORDER BY` | hoist the scalar into the window input |

### 11.4 Interface changes

- `ir::AnySource::{Members, Summaries}` (replacing `Core`, `Summary`). `classify::{export_to_file, member_leaf}`; `stage_to_file` takes the mapping.
- `norm::Graph.labels` (stage per node). `norm::push_filters` (R15). `expr_any`/`rel_any` generalise `expr_free`/`rel_free`.
- `lower`: `IterLower` replaces `ViewLower`; `var_readers`, `op_inputs`.
- `greplay`: `results.json` → `load.dataflow_build_seconds`; `FACTS_PROFILE=1` writes `profile.json` (per stage/node: busy seconds, records sent, arranged records; per fixpoint arm: records per iteration; notes).
- `facts-gate.py`: check `facts-b-unit`; rewrite family `facts-b` (`mutant (facts-b)` column); build step for the facts-b test binary.

## 12. Resume notes (wave 4)

- Exports regenerated with the merged binary, all with 0 failures: `exports/w4/{oracle,replay}-{dead-code,exported-families,state-flow-project-1,required-contracts,quality-project-taxonomy}.jsonl`. Specs: `w4/spec-{oracle,replay}-{light,dead-code}.json`. 514-file subset: `w4/sub500-jobs.tsv`.
- Linker check: `greplay --plans none --real exports/w3/spec-oracle-linker.json --parity exports/oracle-b6-jobs.tsv --no-oracle`, then `scripts/edges_parity.py RUN_DIR`. The reference-evaluator check of the linker is infeasible: more than 50 min on 514 files (`runs/…-043012-linker-sub500-memo-prof`, killed at 3,000 s). Incremental ≡ from-scratch for linker clauses therefore needs an engine from-scratch side: a second dataflow per checked worktree, dropped after the check (`Worker::drop_dataflow`). Not written yet.
- Next, in order:
  1. The four decorrelation rewrites of § 11.3, each registered in REWRITES.md with a test and a mutant (facts-b family); then dead-code parity (4,931 expected) and the light clauses.
  2. Engine from-scratch checks in greplay, then the 1,000-edit replay for every clause that runs.
  3. Memory: the linker base is 7.4 GB (58.8M arranged records, about 110 B each). G2.4's 250 MB for 40 worktrees cannot hold with the linker as is. Levers: presence keeps both input and output arrangements; packed rows; fewer intermediate interned strings.
  4. G2.4 rows, 3× locked; closure-cost aggregate; `calls` atom.

## 13. Wave 5 (S3CrossFile-5)

Status: **partial**. All six named decorrelation gaps are closed by generic, registered rewrites; three more generic changes were needed on the way (R34, ordered aggregates, a LEFT JOIN antijoin on keys), each with a test and a mutant. 5 of the 8 clauses now normalise and run. 4 of them have oracle parity 0 = 0. `unreachable-internal-symbols` runs but is **not exact** (11,560 vs 4,931). The cause is traced to its per-file `imports` export (§ 13.3). `xexport` runs a clause's stages on the facts-a engine: `quality-conditional-1` exports in 59 s (9,010 jobs) and 315 s (19,801 jobs), against a 1,500 s timeout before. The generator still refuses that clause by name (non-equality correlations). Neither incremental ≡ from-scratch nor the protocol rows finished (§ 13.4). Run dirs are under `~/rerail-bench/facts/phase-2/s3-crossfile/{runs,exports/w5}/`.

### 13.1 Results

| Gate / item | Target | Measured | Result | Run directory |
|---|---|---|---|---|
| Six decorrelation gaps (§ 11.3) | generic, registered, test + mutant | R20 (ON subqueries over the left row → left columns), R22 (correlations lifted to the body's WHERE), R24 (disjunctive EXISTS split), R26 (correlated filter pulled through Project/Sort into an aggregate), R28 (uncorrelated EXISTS in ON → cross join with its count), R30 (window subqueries → input columns). Two more were needed: R32 (nested EXISTS joined into its EXISTS body, for `direct_events`) and R34 (an `IN`-list ON → union of keyed joins: `reach_edges` was a 28M-pair cross join on 514 files, 45 s and 11 GB, now 6.5 s and 0.9 GB). Each rewrite has a differential test: the reference before ≡ after on 150 random table sets with NULLs, plus the generated dataflow ≡ the reference on the unrewritten query. Mutants 28–40 and 44 are each killed by `facts-b-unit` | pass | gate `s5-gate/runs/20261023T070721-gate` |
| Generator: ordered aggregates | `group_concat`/`json_group_array` exact | folded in the order of the Sort below (`norm::rel_keyed`, the reference's `keyed`) through `reference::fold_aggregate`. A tie between distinct rows is a runtime refusal exactly where the reference raises `tie:`. Empty-group values come from the same fold (`json_group_array` → `'[]'`; the old code returned NULL). Test `generated_ordered_aggregates`; mutant 42 | pass | same |
| Generator: LEFT JOIN on equalities | stratified in a recursion | the unmatched rows are an antijoin on the join's own keys (before: an identity antijoin against the matched rows, refused inside a recursion). This fixed `views/name_sub`, whose recursive step was silently empty. Test `generated_left_join_equi_antijoin`; mutant 46 | pass | same |
| I9 hole closed | no partial plans | a construct the recursion lowering refused (non-stratified antijoin, aggregate over the recursion, nested recursion) used to drop that subtree and still report findings. Such a refusal is now a base refusal record of the plan, so its findings are withheld in every worktree and the record names the stage | pass | — |
| Clauses normalised (`facts-b gnorm`) | 8 | `required-contracts`, `quality-project-taxonomy`, `exported-families`, `state-flow-project-1`, `unreachable-internal-symbols`: 0 refusals (also after merging S8b `093977aa2`). `quality-conditional-1`: refused, `block_results` and `conditional_findings_0`: *correlated subquery with a non-equality correlation* (`stmt.__start <= coalesce(…)`, `x.__start > outer.__start`), inside a recursive fold whose string state the subqueries read. `state-flow-1`, `state-projection-recovery-2`: no oracle (native exceeds its budget, wave 2) | 5 of 8 | — |
| Oracle parity on `2b1504adc` (suite options) | findings = oracle | `exported-families` 0 = 0, `state-flow-project-1` 0 = 0, `required-contracts` 0 = 0, `quality-project-taxonomy` 0 = 0, all with 0 static and 0 runtime refusals (9.5 s, 2.2 GB) | pass (4) | `runs/20261023-*-parity-light-w5c` |
| `unreachable-internal-symbols` parity | 4,931 = 4,931 | generator **11,560**: 0 missing, 6,629 extra, spread over all four categories; 0 refusals; 79 s; 11.2 GB. **Not exact** | **fail** | `runs/20261023-*-parity-dead-code-w5c` (`findings-parity-*.json`), stages `runs/*-dead-code-stages-w5` |
| Per-file exports on the engine (`xexport --stages engine`) | qc1 exports complete | `quality-conditional-1`: oracle 9,010 jobs in 59.1 s, replay 19,801 jobs in 315.0 s, **0 failures**. Every export of 3,383 (oracle) and 5,138 (replay) contents of ≤ 4 KB was re-derived by the reference: equal | pass | `exports/w5/{oracle,replay}-quality-conditional-1.jsonl{,.summary.json}` |
| Incremental ≡ from-scratch, replay | 0 differences | **Not measured.** The 5 runnable clauses together, 300 events: the load took 48.6 s at 8.6 GB RSS. The first from-scratch check on the reference evaluator (`exported-families`, `state-flow-project-1` over a whole worktree) did not finish within 59 min, and the run was killed at the 1 h cap | open | `runs/20261023-072602-exactness-light-w5` |
| G2.4 protocol rows (3× locked) | ≤ 250 MB for 40 worktrees, latency, state vs divergence | **not run** (run budget). The only measurement: 8.6 GB RSS after loading 40 worktrees with the 5 plans (unlocked), far over 250 MB. The dead-code plan alone is 11.2 GB for one 9,010-file worktree (the linker base 7.4 GB, wave 4; 721 MB interner) → Phase 3 memory item | open | same |
| `captured-return` aggregate, `calls` atom | implemented | not started | open | — |
| Bare `GROUP BY` columns (decider's question) | — | none of the 8 clauses is refused for a bare non-key column (no such refusal in `gnorm` for the 6 that reach the generator). A runtime-checked form would unblock nothing today | — | — |

### 13.2 Commits and gate

- `bcadf44eb` (s3): R20–R34, ordered aggregates, the LEFT JOIN key antijoin, build-time refusals withheld, `xexport --stages engine` with the reference check, `facts-b gnorm`, `scripts/findings_parity.py`. Gate **28/28 checks, 36/36 mutants**, 828 s (`20261023T070721-gate`). That is over the 10 min target, and the timing includes the gate's build.
- `96dada3f0`: `FACTS_STAGE_OUTPUTS=1` (each stage also an output, for diagnosis).
- `02d88aba2`: merge of S8b `093977aa2` (SQLite 3.53.2 typing). The only conflict was REWRITES.md's numbering list (both sides kept). `norm::tests` 13/13 and `gnorm` unchanged after the merge. Gate on the merged tree: see the final message.
- Gate changes: `REWRITE_FAMILIES["facts-b"]` is now `facts-b-unit` only, because `b-generated`'s vectors hold none of the normalisation shapes (R15's mutant 24 already survived it). `MUTANTS` gains 42 and 46.

### 13.3 Dead-code: where it goes wrong

Per-stage sizes on the generator (`FACTS_STAGE_OUTPUTS=1`) against the SQLite harness of NARROWINGS:

| Stage | Generator | SQLite |
|---|---|---|
| `declarations` | 74,579 | 74,579 |
| `class_names` | 38,771 | 38,771 |
| `views/reference_edges` | 145,431 | 145,431 |
| `imports` | **0** | 95,684 |
| `base_edges` | 5,483 | 5,534 |
| `hierarchy` | 27,381 | 27,528 |
| `public_methods` | 417,836 | 418,068 |

`imports` reads export 18, the import statements: 52,201 rows, the same with the merged binary and with the w4 exports. Its rows are wrong: `['_internal/__init__.py', 348, 348, None, '', ',', 0]` gives `top` = the start byte, `base` = NULL and `items` = ''. `imports` is therefore empty, `reachable` is too small, and symbols come out "unreachable" (all extras, no misses). The defect is in the per-file producer's input projection for that input: the IR leaf inputs as `xexport` builds them (`projection_json` over `InputDef.fields`), or the cut's column map. It is not in the generator: every stage that reads correct exports matches SQLite.

### 13.4 Interface changes

- `norm`: `hoist_on_subqueries` (R20), `lift_correlations` (R22), `split_exists` (R24), `pull_correlation` (R26), `count_on_exists` (R28), `hoist_window_subqueries` (R30), `flatten_exists` (R32), `split_in_list_join` (R34); `remap_expr`/`remap_rel` (depth-aware column maps through subqueries); `Op::Group.order`; `Ctx::rel_keyed`.
- `lower`: `agg_logic(…, order)`. Refusals raised while building a plan are base refusal records (`Built.refusals`), named `<stage>: <construct>`.
- `reference::fold_aggregate` (pub; S5's evaluator state reused by the generator).
- `facts-a xexport --stages engine|reference --check-below BYTES`; the summary gains `stages`, `reference_checked`, `check_below_bytes`. Stages are computed once per producer, not once per export (that repetition was most of the old 460 s per 200 jobs).
- `facts-b gnorm --real SPEC [--dump all|TEXT]`; `scripts/findings_parity.py RUN --clause PLAN:CLAUSE --check CHECK --jobs JOBS [--code-col N]`.

## 14. Resume notes (wave 5)

- Next, in order:
  1. Dead-code `imports` export (§ 13.3). Compare export 18's IR producer columns with the clause's `inputs` declaration (fields `top`, `base`, `items`). Run `facts-a xexport --stages reference` on one file and evaluate the IR producer by hand. Then rerun `greplay … spec-oracle-dead-code.json --parity oracle-b6-jobs.tsv` and `findings_parity.py … --check internal_dead_code --code-col 4` (expected 4,931).
  2. `quality-conditional-1`: a generic θ-semijoin. An EXISTS/scalar with a non-equality correlation becomes `n ⋉_identity π_n(σ_θ(n ⋈_eq B))` (and per-identity groups for scalars). Inside its recursive fold (`block_results`), the body reads the recursive row's `state`, so the identity semijoin is over the recursion: it needs the stratification check or a narrowing. Exports are ready (`exports/w5/*-quality-conditional-1.jsonl`; spec: rule `exports/w3/policies/data-layer-serverplan-quality.json`).
  3. Incremental ≡ from-scratch: the reference cannot evaluate `exported-families`/`state-flow-project-1` over a worktree in usable time (> 59 min for one check). Implement the engine from-scratch side of § 12: a second dataflow per checked worktree with only base records, dropped after the check. Then replay with `--real exports/w4/spec-replay-light.json`.
  4. Protocol rows, 3× under `bench-lock`: `greplay --plans none --real …spec-replay-light.json --no-oracle --track`. Expect ~8.6 GB at load. Memory is the blocker for 250 MB: arranged rows of the wide exported relations (`state-flow` exports carry about 60 columns of `nodes`), the linker base 7.4 GB, the interner.
  5. `captured-return` aggregate and `calls` atom (scratchpad decision; § 9.3, § 10 item 2).
- Run dirs: parity `runs/*-parity-{light,dead-code}-w5c`, stage sizes `runs/*-dead-code-stages-w5`, profiles `runs/*-dead-code-sub500-prof{,2}`, exactness attempt `runs/20261023-072602-exactness-light-w5`, exports `exports/w5/`.

## 15. Wave 6 (S3CrossFile-6)

Status: **items 1–4 done; item 5 partial** (R36 done; `quality-conditional-1`, `captured-return` and `calls` still open).

- Dead code is exact: 4,931 = 4,931, with 0 refusals.
- Incremental ≡ from-scratch holds for all 5 running clauses: 0 differences. The comparison covers every stage, not only the findings. It ran over the 300-event prefix with sampled checks; the reason is below.
- Memory is attributed per plan, per stage and per arrangement role.
- The protocol rows were run 3 times, locked. Each run took about 17 min, over the 15 min guideline.
- R36 (θ-correlations, mutant 48) is generic and registered. `quality-conditional-1` still does not run: its next refusal is an `IN (subquery)` under `OR`.
- `captured-return` and the `calls` atom were not started.
- Per Main's LOOP.md rule (received mid-wave), the full gate was not run. T0 and T1 were run instead (§ 15.5).

Runs are in `~/rerail-bench/facts/phase-2/s3-crossfile/runs/` and exports in `exports/w6/`.

### 15.1 Results

| Gate / item | Target | Measured | Result | Run directory |
|---|---|---|---|---|
| `imports` export defect: cause | root cause | The cause was not column order. Regex named groups (`capture.BASE`, `capture.ITEMS` of a same-node `regex` in an AST rule) were looked up in a thread-local `GROUP_NAMES`. Only `eval_file` filled it, per rule, before running a rule. `xexport` calls `Input::rows` directly, so the table was empty: every group read NULL (projections) or `''` (text columns). On the oracle side, export 18 (`imports`' `tokens` seed) had `base` NULL in 52,201 of 52,201 rows and `items` `''` in all of them. `top` was right (`348 = 348` is a top-level statement's own start). The same bug changed `exported-families`' export 4 (its `imports`) in 2,990 oracle / 7,996 replay contents, and dead-code export 25 (`raw_dynamic_imports`, regex `each`) in 33 / 147 | found | — |
| Fix (general) | no implicit binding | `eval::Input.groups`: each input carries its regex groups' capture ids from compile time, and `GROUP_NAMES` is deleted, so no caller can forget to bind. `xexport` also applies `evidence.paths` (it ignored them) and builds every relation it hands on from its IR schema: leaf inputs from `InputDef.schema`, engine stages from `Stage.schema`, by column name (`column_map`, never by position). `conform` checks every row of every leaf, summary, `file`, engine stage and export: the width must match, and each value must be stored as its column's affinity stores it (`apply_affinity` leaves it unchanged). A mismatch fails the content. Tests: `xexport::tests::{regex_groups_read_outside_eval_file, columns_by_name, rows_conform_to_the_schema}` | pass | all 10 export sets regenerated with 0 failures, i.e. 0 width/affinity mismatches over 9,010 + 19,801 contents × 5 clauses (`exports/w6/*.summary.json`) |
| `unreachable-internal-symbols` oracle parity | 4,931 = 4,931, 0 refusals | **4,931 = 4,931, 0 missing, 0 extra**, 0 static and 0 runtime refusals (wave 5: 11,560) | **pass** | `runs/20261023-085817-parity-dead-code-w6` |
| The other 4 clauses after the fix | still match | `exported-families`, `state-flow-project-1`, `required-contracts`, `quality-project-taxonomy`: 0 = 0 each, 0 refusals | pass | `runs/20261023-090138-parity-light-w6` |
| Engine from-scratch side | a second dataflow per check, dropped after it | `drive::scratch`: the same real plans lowered into a new dataflow. Its only records are the checked worktree's members tagged base, so every product is base and presence is a threshold. It runs to completion, its findings and live refusals are read, then `Worker::drop_dataflow`. Refusals it raises are not counted in the run (`lower::uncounted`). A check compares findings and, when the from-scratch side refuses, the refusal sets (`refusals_differ`). The vectors keep Spike B's oracle, and `--scratch reference` keeps the reference evaluator. One check costs 3.3 s for the 4 light plans and 32 s for dead code (one 9,010-file worktree) | implemented | smoke `runs/20261023-090245-scratch-smoke-light`, `…-090427-scratch-smoke-dead-code` |
| Negative control | the check catches delta-maintenance bugs | With mutants built in (`--features facts-b/mutants`) and `FACTS_STAGE_OUTPUTS=1`: mutant 4 (ΔR⋈ΔS dropped) and mutant 5 (count retractions ignored) each give 36 differences at the initial check. **Mutant 6 (fixpoint stops early) is not caught**, because it changes the from-scratch dataflow the same way. The from-scratch side checks delta maintenance, not the lowering's semantics; that is the reference's job on the vectors (`b-generated`, T1) | as designed | `runs/20261023-094240-scratch-control-mutant-4-light`, `…-094345-…-5-light`, `…-094442-…-6-light` |
| Incremental ≡ from-scratch, light (4 plans), findings | 0 differences | **300-event prefix** (288 edits, 10 new-worktree edits, 1 bulk, 1 new worktree), every event's worktree checked plus 5 worktrees at load and at edits 100/200: 301 checks, 315 worktree checks, **0 differences**, 0 runtime refusals | pass | `runs/20261023-091058-exactness-light-w6` |
| Same, every stage compared (`FACTS_STAGE_OUTPUTS=1`: 4 findings + 29 stages, 670K rows in worktree 1) | 0 differences | 300-event prefix, every 2nd event plus sweeps of 5: 151 checks, 165 worktree checks × 33 outputs, **0 differences**, 0 refusals | pass | `runs/20261023-094024-exactness-light-stages-w6` |
| Incremental ≡ from-scratch, dead code, findings (2,570 in worktree 1) | 0 differences | 300-event prefix, every 25th event plus 2 worktrees at load and at edits 100/200: 13 checks, 18 worktree checks, **0 differences**, 0 refusals | pass | `runs/20261023-091058-exactness-dead-code-w6` |
| Same, every stage compared (1 finding output + 96 stages incl. the linker views, 5.0M rows in worktree 1) | 0 differences | 300-event prefix, every 50th event plus 1 worktree at load and at edits 100/200: 7 checks, 9 worktree checks × 97 outputs, **0 differences**, 0 refusals | pass | `runs/20261023-094914-exactness-dead-code-stages-w6` |
| Why the prefix and sampled checks | ≤ 15 min per run | The full replay has 1,111 events. Checking every event from scratch would take ≥ 1,111 × 3.3 s ≈ 61 min (light) and ≈ 10 h (dead code). The runs above took 24, 12, 19 and 10 min. Note: the 7 contents of the replay without retained source (§ 6) first appear after event 300: in the full replay they are 7 runtime refusals, all retracted by the end (protocol runs, `runtime_refusals_live` = []) | — | — |
| R36 θ-correlations (Main's item 5) | generic rewrite, mutant 48, or a named refusal | `norm::Ctx::theta_body`: a subquery whose WHERE has non-equality correlations (`<=`, `>`, functions of both rows, nested subqueries reading the enclosing row) is joined with `distinct(n)` and filtered by θ, then correlated with `n` on all of `n`'s columns by grouping identity. This covers `[NOT] EXISTS`, `[NOT] IN`, scalar aggregates and the scalar `ORDER BY … LIMIT 1`. Registered in REWRITES.md (facts-b table, mutant 48 drops θ). Test `norm::tests::generated_theta_correlations`: 10 shapes, dataflow ≡ reference over 150 random table sets with NULLs and duplicates (> 1,000 rows), plus a θ NOT EXISTS over a recursion refused by name ("not stratified"). Mutant 48 fails it. Plans of the 5 running clauses: unchanged (`gnorm` node counts 386/428/30/15/1327) | pass | — |
| `quality-conditional-1` after R36 | runs, or refused by name | Normalisation now passes `block_results` (whose θ-correlations were wave 5's refusal) and the θ-correlations of `conditional_findings_0`. It refuses `conditional_findings_0` for a different construct: `IN (subquery) under OR, NOT or CASE` (`n.qualified_name NOT IN (SELECT name FROM derived_conditionals) OR NOT EXISTS(…)`, the `physical-contract` arm). [INFERENCE] Past that, lowering `block_results` will refuse by name: its NOT EXISTS reads `fold.state` (the recursive row), so R36's identity antijoin is against the recursion. The unit test shows that shape refused as "antijoin by a recursive relation inside the recursion (not stratified)". It was not observed on qc1 itself, because the plan is refused before lowering | refused (named) | `facts-b gnorm --real` over `exports/w5/oracle-quality-conditional-1.jsonl` |
| `captured-return` aggregate, `calls` atom | — | not started | open | — |

### 15.2 Memory attribution (unlocked loads of the 40 worktrees, 0 events, `FACTS_PROFILE=1`; arranged records from differential's logging, per stage via the profiler's operator→node map)

Correction to wave 5: its "8.6 GB for the 5 plans" was `spec-replay-light.json`, the **4** light plans. With dead code, all 5 plans load at **29.3 GB**.

| Run | RSS loaded | Arranged records | Records sent at load | Interned strings | Exported-row cache | B / record (RSS − interner − rows − store) | Run directory |
|---|---|---|---|---|---|---|---|
| store only (no plan) | 38 MB | 0 | — | 11.6K / 1 MB | 0 | — | `runs/20261023-091125-mem-store-only-w6` |
| `required-contracts` | 59 MB | 20,490 | 0.16M | 20.5K / 2 MB | 2 MB | — | `…-091129-mem-only-required-contracts-w6` |
| `quality-project-taxonomy` | 98 MB | 5 | 17 | 11.6K / 1 MB | 0 | — | `…-091134-mem-only-quality-project-taxonomy-w6` |
| `state-flow-project-1` | 166 MB | 3 | 13 | 11.6K / 1 MB | 6 MB | — | `…-091246-mem-only-state-flow-project-1-w6` |
| `exported-families` | 9,697 MB | 28.33M | 282.7M | 316K / 43 MB | 72 MB | 353 | `…-091139-mem-only-exported-families-w6` |
| `unreachable-internal-symbols` | 23,578 MB | 117.55M | 944.0M | 3.28M / 600 MB | 533 MB | 200 | `…-091254-mem-only-unreachable-internal-symbols-w6` |
| **all 5** | **29,260 MB** | **145.91M** | **1,226.9M** | 3.29M / 600 MB | 613 MB | 201 | `…-092005-mem-all5-w6` (`profile.json` → `stages`) |

Within the 145.91M arranged records of all 5 plans:

| Split | Records | Share |
|---|---|---|
| Linker base (dead code's `views/*` stages: `link_reach` 12.4M, `linked_edges` 6.9M, `s_edge` 6.5M, `name_sub` 3.7M, `spelling` 3.5M, …) | 61.38M | 42 % |
| Dead code outside the linker (`reachable` 17.3M, `reach_edges` 14.5M, `public_methods` 11.9M, `dead_traits` 2.2M, …) | 56.17M | 38 % |
| `exported-families` (`public_methods` 11.9M, `traits` 6.7M, `annotation_names` 2.1M, `imports` 2.0M, …) | 28.33M | 19 % |
| the 3 other plans | 20.5K | 0 % |
| *of which* stages computed twice, identical in `exported-families` and dead code (`public_methods`, `annotation_names`, `imports`, `declarations`, `parameters`, `hierarchy`) | 17.81M | 12 % |
| *by role:* keyed arrangements of node outputs (`n…:by`) | 64.51M | 44 % |
| *by role:* presence input + output (`distinct`, `pres`, fixpoint `iterate-distinct`) | 21.53M + 21.23M | 29 % |
| *by role:* fixpoint join arms (`iter-join-l`) | 24.41M | 17 % |
| *by role:* group input + output | 13.28M | 9 % |

Outside arrangements: the interner holds 600 MB at load (3.28M strings, almost all computed by the dead-code plan; the store's own strings are 11.6K) and 688 MB / 5.77M strings after the replay. It never frees. The exported-row cache holds 613 MB at load and 1,048 MB after the replay.

Per-worktree deltas come from a `--track` run (live records of every operator output per tag, over the full replay). They are not arranged records, and the run's RSS includes the tracking map (40.1 GB at load, 62.3 GB at the end). At the end of the 1,111 events the base tag held 57.8M records (5.4 GB as rows), the 8 cluster tags 81.5M (13.6 GB) and the 46 worktree tags **239.2M (29.9 GB, 63 %)**. Per worktree, state grows linearly with divergence: records ≈ 8.2 × delta rows − 0.47M (R² 0.92), median 3.9 records per delta row (1.1–19.1). The largest worktree (523 delta entries, 1.85M delta rows) holds 10.2M records (`runs/20261023-092718-track-all5-w6`).

What a Phase 3 design must change, with these numbers (nothing was optimised in this wave):

1. **Arrangement pruning to what delta maintenance reads.**
   - Presence keeps both its input and its output arrangement (42.8M, 29 %). One of the two is enough: about −21M.
   - Keyed arrangements of node outputs (64.5M, 44 %) and fixpoint join arms (24.4M) are built per (node, key) for every consumer. Delta maintenance needs only the arrangements a bilinear term actually reads.
2. **One plan graph across clauses.** Identical stages over the same exports are lowered once per plan today: 17.8M (12 %) are duplicates. A plan digest (REWRITES.md "CSE by expression digest", planned) removes them.
3. **The linker as a shared base, not a per-plan view.** At 61.4M records (42 %, about 12 GB at 201 B/record) the linker is the largest single item. It is base state shared by every clause that reads `reference_edges`/`calls`, and is maintained once per repository.
4. **One interner with compaction.** Computed strings (qualified names, link keys) are interned and never freed: 600 → 688 MB over the replay. Strings built for the linker should be ids (path id, def ordinal), not text.
5. **Record size.** At about 200 B per arranged record (dead code; 353 B for `exported-families`' wide rows) against 40 B by hand (§ 8.2), even items 1 and 2 leave about 107M records, about 21 GB. The 250 MB target for 40 worktrees cannot be met by pruning. It needs a columnar arrangement of u32 ids, and the linker outside the per-plan dataflow.
6. **Compaction and divergence.**
   - Arranged records grow from 145.9M at load to 252.8M at edit 1,000 and 349.7M at the end of the replay (deterministic: equal in all 3 protocol runs). Over the same span the live per-worktree state reaches 239M records.
   - [INFERENCE] Most of that growth is divergence (6 new worktrees, delta entries per worktree from a median of 26 up to 523). The rest is merge slack in differential's spines. This run does not separate the two. That needs a from-scratch load of the end memberships (the § 15.1 dataflow, without a replay), which was not run.
   - Re-clustering (moving shared worktree deltas into cluster tags) is the lever for the divergence part.

### 15.3 Protocol rows (3 runs under `bench-lock`; load 1.9 / 10.2 / 14.0 at start; 5 plans, 40 worktrees, the full replay: 1,111 events with 1,000 edits)

Each run took 1,003–1,030 s, over the 15 min guideline (load 268–291 s, membership model 47–53 s). Counts are deterministic across runs; times are calibration.

| Row | p1 | p2 | p3 | Target |
|---|---|---|---|---|
| RSS after loading 40 worktrees | 31,552 MB | 31,664 MB | 31,988 MB | ≤ 250 MB |
| RSS at edit 1,000 | 42,462 MB | 42,541 MB | 42,768 MB | — |
| RSS / footprint at end (46 worktrees) | 67.9 / 43.3 GB | 67.8 / 45.1 GB | 67.3 / 42.9 GB | — |
| Peak RSS (`time -l`) | 71.3 GB | 71.2 GB | 70.6 GB | — |
| Arranged records at edit 1,000 / end (count) | 252,821,796 / 349,745,074 | same | same | — |
| Single-file edit latency p50 / p95 / p99 / max (n = 1,050) | 17.0 / 62.5 / 194 / 1,320 ms | 15.8 / 59.9 / 184 / 1,315 ms | 17.1 / 63.6 / 193 / 1,330 ms | calibration |
| Bulk p50 / max (n = 5) | 7.1 / 19.2 s | 7.0 / 18.3 s | 7.1 / 18.7 s | — |
| Commit p50 / max (n = 50) | 2.2 / 25.3 s | 1.7 / 24.8 s | 1.9 / 24.5 s | — |
| New worktree p50 (n = 5); commit init | 24.4 s; 77.6 s | 23.7 s; 77.0 s | 24.4 s; 78.4 s | — |
| Runtime refusals | 7 contents without retained source, 0 live at end | same | same | 0 live |
| Per-worktree state vs divergence | § 15.2 (track run): ≈ 8.2 records per delta row, R² 0.92 | | | proportional |

Run directories: `runs/20261023-100048-protocol-all5-p1`, `…-101758-protocol-all5-p2`, `…-103441-protocol-all5-p3`.

### 15.4 Commits

- `3f24e1a35`: `Input.groups` replaces `GROUP_NAMES`; `xexport` builds relations from the IR schemas and adds `conform` and the `paths` filter; 3 xexport unit tests.
- `863732d91`: the greplay from-scratch dataflow (`drive::scratch`, `Want`, `--scratch`, `--check-every`, `--sweep`) and `lower::uncounted`.
- `703e5b932`: R36 (`norm::Ctx::theta_body`; correlations carry `KeyMode` on both sides, type `Corr`), the REWRITES.md row and mutant 48.
- Plus this report.

### 15.5 Checks run (LOOP.md T0/T1; the full gate is the integrator's)

- T0:
  - `cargo test --release -p facts-b --features facts-b/mutants`: 14/14.
  - `cargo test --release -p facts-a --features facts-a/mutants --bin facts-a`: 13/13 (the new xexport tests included).
  - `FACTS_MUTANT=48 … norm::tests`: `generated_theta_correlations` fails (killed).
- T1, the 100-event replay prefix on the vectors with the final binary:
  - r1+r3: 101 checks, 180 worktree checks, 0 differences (`runs/20261023-103124-t1-vectors-r1r3-w6`).
  - rt: 180/180 expected refusals refused, 0 differences (`…-103155-t1-vectors-rt-w6`).
- The gate's registry reads REWRITES.md, so mutant 48 joins family `facts-b-unit` with no edit to `facts-gate.py`.
- `just facts-gate --mutants` was **not** run: Main's LOOP.md ruling arrived mid-wave.

### 15.6 Interface changes

- `facts-a`:
  - `eval::Input.groups: Vec<(String, Name)>` (regex group → capture id). `eval::GROUP_NAMES` is deleted.
  - `xexport`: `column_map`, `table_rows`, `conform`. A relation that does not fit its schema now fails the content as `"<relation>: …"`.
- `facts-b greplay`:
  - New flags: `--scratch engine|reference` (default `engine`), `--check-every N` (default 1), `--sweep K` (worktrees per sweep, default all).
  - `results.json` → `exactness` gains `scratch`, `scratch_dataflows`, `scratch_seconds`, `check_every`, `sweep`.
  - New difference kind: `refusals_differ` (`incremental` vs `scratch`).
- `lower::uncounted`.
- `norm`: `Corr = (X, X, (KeyMode, KeyMode))` replaces the `(Aff, Aff)` correlation pairs; `Ctx::theta_body`. `decorrelate` and `rows_of` return the θ conjuncts.

## 16. Resume notes (wave 6)

- Exports: `exports/w6/{oracle,replay}-{dead-code,exported-families,state-flow-project-1,required-contracts,quality-project-taxonomy}.jsonl` (fixed exporter). Specs: `w6/spec-{oracle,replay}-{light,dead-code,all5}.json` and `w6/spec-replay-only-<clause>.json`.
- Commands:
  - Parity: `greplay --plans none --real w6/spec-oracle-dead-code.json --parity oracle-b6-jobs.tsv --no-oracle`, then `findings_parity.py RUN --clause unreachable-internal-symbols:unreachable-internal-symbols --check internal_dead_code --jobs oracle-b6-jobs.tsv --code-col 4`.
  - Exactness: `FACTS_STAGE_OUTPUTS=1 greplay --plans none --real w6/spec-replay-<light|dead-code>.json --events …/seed-5eedb/events.jsonl --limit 300 --check-every N --sweep K`.
  - The `qc1` exports in `exports/w5/` predate the fix. Its inputs are `syntax` relations (no regex groups); regenerate anyway before its first parity.
- Next, in order:
  1. `quality-conditional-1`: a generic rewrite for `IN (subquery)` under OR/NOT/CASE. In three-valued logic, `e IN S` = `CASE WHEN count(S) = 0 THEN 0 WHEN e IS NULL THEN NULL WHEN count(S.f = e) > 0 THEN 1 WHEN count(S.f IS NULL) > 0 THEN NULL ELSE 0 END`: three correlated counts, hoisted like EXISTS. Then lower `block_results`. If its θ NOT EXISTS over `fold.state` is refused as "not stratified" as inferred, it needs a row-local antijoin: an antijoin whose right side is a function of the left row and of non-recursive relations only, i.e. a filter, which is monotone. Otherwise, a narrowing.
  2. Separate divergence from merge slack (§ 15.2 item 6): load the end-of-replay memberships from scratch and compare arranged records with the 349.7M after the replay.
  3. Phase 3 memory items (§ 15.2): presence with one arrangement, cross-plan CSE, the linker as shared base state, id-valued linker strings, columnar records.
  4. `captured-return` aggregate (§ 9.3; needs S6's step counts from `Closure::derive`) and the `calls` atom (§ 10 item 2).
