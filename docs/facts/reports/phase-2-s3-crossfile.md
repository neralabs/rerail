# Phase 2, slice S3: cross-file backend (delta-plan generator)

Branch `Moti-Velz/facts-engine-s3-crossfile`, crate `spikes/facts-b` (module `gen`, `membership`). Raw runs: `~/rerail-bench/facts/phase-2/s3-crossfile/runs/<timestamp>-<label>/`. Status: **in progress**; this file is kept current at every milestone so a fresh agent can resume.

Scope after the decider's re-steer (Main, 2026-10-22): no hand-compiled plan ships. The deliverable is the **generic delta-plan generator**: an IR repository plan (`rerail_facts::ir::Plan<RepoSource>`, S1) is normalised to delta operators and lowered to one differential dataflow over the membership model. Spike B's hand expansions are the generator's test vectors. Constructs with no exact incremental form are refused with the construct named (INVARIANTS I8, I9).

## 1. Setup

| Item | Value |
|---|---|
| Code | `spikes/facts-b/src/gen/{norm,lower,tags,val,scalar,vectors,drive}.rs`, `spikes/facts-b/src/membership.rs`, `spikes/facts-b/src/mutant.rs` |
| IR | `rerail_facts::ir` (S1 `ef5d43adc`); scalar semantics and constant relations through `rerail_facts::reference` (S5 `94e31685f`), so the dataflow and the reference evaluator share one definition of SQLite's expressions |
| Engine | differential-dataflow 0.25.1, timely 0.31.0, one worker, `u32` epochs, `i32` diffs |
| Corpus | as Spike B: `~/tempo-qe/workspaces47`, worktrees 01–40, store `~/rerail-bench/facts/store`, memberships `~/rerail-bench/facts/spike-b/membership/` |
| Replay | Spike B's `~/rerail-bench/facts/spike-b/edits/seed-5eedb/events.jsonl` (1,000 single-file edits, 5 bulk, 5 new worktrees × 10 edits, 51 real commits) |
| Build | `CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target cargo build --release -p facts-b` (gate mutants: `--features mutants`, `FACTS_MUTANT=4|5|6`) |

## 2. Results

| Gate | Target | Measured | Result | Run directory |
|---|---|---|---|---|
| G2.4 oracle parity, repository clauses, `2b1504adc` | parity | pending (needs S1's compiler output for the 8 clauses and S2's per-file exports) | — | — |
| G2.4 incremental ≡ from-scratch, 1,000-edit replay | 0 differences | pending | — | — |
| G2.4 resident, 40 worktrees, real plans | ≤ 250 MB | pending | — | — |
| G2.4 per-edit latency, per-worktree state vs divergence | reported | pending | — | — |

## 3. How measured

`spikes/facts-b/scripts/run.sh <label> [--lock] -- greplay --events $E [--plans r1,r3] [--flat] [--fail-fast] [--no-oracle]` (run directories under `~/rerail-bench/facts/phase-2/s3-crossfile/runs/` once `run.sh` takes `FACTS_RUN_ROOT`).

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

## 6. Risks found

- Real plans depend on S1's compiler (repository side of each clause) and on per-file exports (S2's executor over `Plan<FileSource>` with `Sink::Export`).
- The reference evaluator's naive nested loops make per-worktree checks on large exported relations slow; the replay's 1,576 checks need its correlated-equality subqueries indexed.

## 7. Resume notes

- Done: generator (`norm`, `lower`), tag tree, membership model with eviction, IR test vectors R1/R3, `greplay` driver, mutants 4–6 in the generator's emitters.
- Next: run the vectors on the replay (exactness vs Spike B's oracle), measure; adapter-free intake of S1's `Program` for the 8 clauses; per-file exports; parity on `2b1504adc`.
