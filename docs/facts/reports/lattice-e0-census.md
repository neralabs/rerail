# Lattice E0: census of the recursion in the 60 rules, and what it costs

Branch `Moti-Velz/facts-engine-lattice-census` from integration `c6c33c491`. Experiment E0 of `docs/facts/LATTICE.md` (H1, H2, H3, open question 2). Every number below is copied from a cited run directory or a generated table in it; `[INFERENCE]` marks reasoning that was not measured. Run root: `~/rerail-bench/facts/explore/lattice-e0/` (below, `E0/`).

## Verdicts

| Id | Verdict | Deciding numbers | Where |
|---|---|---|---|
| H1 (catalogue fit) | **holds**, conditionally | 9 of the 69 recursive CTEs (13.0%; 7 of the 68 `WITH RECURSIVE`, 10.3%) need something outside `set`/`min_level`/`best_witness`; 0 are non-monotone. But only 42 of 69 are a catalogue lattice *as written*; 18 more fit only through a registered rewrite with an I2 side condition (three rewrite families, § 2.2). As written, 27 of 69 (39%) are not catalogue lattices | `E0/census/h1-tally.txt`, § 2 |
| H2 (recursive share of work) | **killed as the lever**; the stated criterion depends on the unit | Recursive stages' share of the 7 facts-a rules: 69.6% of relational algebraic rows, 22.3% of physical counts, 3.4% of all algebraic counts (matcher node visits included), 10.3% of thread time. 66.0 of the 69.6 points are syntax/scope-tree closures (an interval probe answers them with no fixpoint). Graph fixpoints are 3.5% of algebraic rows and 4.7% of physical counts. Dead code (M2): the 68's own fixpoints are 9.4% of rows + derivations (6.9% of time); the engine's own views (the `references` linker and `file.module`) add 26.6% (56.8% of time) | `E0/runs/20261029T133958-corpus-8rules`, `E0/runs/20261029T134023-m2-3of8-w1`, § 3–4 |
| H3 (few rounds, wide frontiers) | **killed** as stated | Per content, the median frontier of every heavy fact is ≤ 1,114 rows (state-signatures-1 `descendants`); levels reach 80–82. In lockstep across all contents the median round width is still < 10K for every heavy fact but one (29,312), with 15–82 levels; the dead-code reachability (membership) has median 1,663 over 24 levels on worktree 1 (11,433 over 25 rounds on E1's oracle-tree product graph); the heaviest recursion dead code runs, the linker's `link_reach`, takes 421 levels. What survives: weighted by derivations, 96–100% of the nine heaviest per-file facts' lockstep work and 95% of `reachable`'s new rows fall in rounds ≥ 10K wide | § 3.3, § 4 |
| Open question 2 (type-join lattice) | **no rule needs one** | The only carried labels are taint kinds (`taints`, a finite set never joined: readers partition by kind) and name sets; `set` over (node, kind) covers them | § 2.3 |

## 1. Method

**Static census.** `E0/census/census.py` walks the 8 rules of `sql-dl60` (`e6d2ac73c`, read-only; byte-identical to this branch's `packs/`) and splits each `WITH RECURSIVE` into its CTEs, the compound operators between arms and the self-references (`E0/census/occurrences.json`, the 41 distinct texts in `texts.txt`, every stage of the 18 clauses in `stages.txt`). Each recursive CTE was then read by hand, with the stage's use of it (windows after it, readers) and the declared structure it relies on; the reading is `E0/census/classify.py`, which generates the table of § 2 (`census.md`) and the tallies (`h1-tally.txt`). Operators are from `facts-a explain --physical` (`E0/runs/explain/`).

**Dynamic counts.** Two engines run these rules today, so two runs:

- **facts-a** (per-file plans and the repository phase), the 8 rules over the 9,010-file data-layer tree (`~/rerail-bench/nera-data-layer-2b1504ad`, the batch-6 corpus), `--no-memo` (stage counts are not memoised), 8 workers: `E0/runs/20261029T133958-corpus-8rules` (204 s wall, load 22.5 → 27.0). `internal_dead_code` is refused at compile (`unreachable-internal-symbols/raw_modules: projection file.module`), so the facts-a totals cover 7 rules and 79 clauses. The run found 0 findings and 1 failure (that refusal); parity was not the question here.
- **M2** (`facts-b hreplay`, the adopted cross-file model, PHASE-3 P3a), the three repository plans of the 8 rules (`unreachable-internal-symbols`, `exported-families`, `state-flow-project-1`) on replay worktree 1 (5,725 contents), from scratch, one worker: `E0/runs/20261029T134023-m2-3of8-w1` (batch run 34.9 s, load 19.9 → 14.3). It runs the narrowed `w3` texts (NARROWINGS.md: the generator refuses bag recursion), with the `references` linker and `file.module` as engine views.

**Instrumentation (count-only, commit `865d79e3a`, T0 green: `~/rerail-bench/facts/loop/runs/20261029T133833-t0`).** Per-round admitted rows were already counted per stage, but summed over files (`StageCounts::fix`); per content they were not kept, and M2 kept only totals.

- facts-a: `repo --file-counts` rows now carry `fix`: per stage with a top-level fixpoint, its admitted rows per round on that content (round 0 the seeds, through the first empty round; `eval::Output::file_fix`). Test `repo::tests::file_counts_report_fixpoint_rounds`: a set chain reports `[1, 1, 1, 1, 0]`, a bag fan-out `[1, 2, 4, 0]`.
- facts-b M2: `batch::FixRounds` per fixpoint (frontier = the variable's new rows per round, derivations per round), carried into `hreplay`'s `results.json` (`runs[].fixes`). Test `gen::batch::tests::fix_rounds_report_frontier_per_round`: frontier `[1, 2, 1, 0]`, derivations `[5, 6, 1, 0]`.

**Units.** facts-a: `alg` = stage_rows + fix_rows (LOOP.md's relational algebraic counts; fix_rows are the rows a fixpoint admits per round: new rows of a set fixpoint, every derived row of a bag one); `alg+matcher` adds leaf_rows, match_nodes, input_rows (the full algebraic count; match_nodes are node visits, 3.06 billion of the 3.26 billion); `phys` = index_rows + range_rows + probes + rows_scanned + rows_joined + rows_out. M2: rows_out of every operator plus derivations (candidate rows of every body operator before the round's distinct). The two engines' units differ, so their shares are reported apart. A *level* is a non-empty round. *Lockstep* is the per-round sum over all contents: the frontier one round would have if every content ran its round k together (the across-contents axis of L7).

## 2. The census

### 2.1 Tallies (`E0/census/h1-tally.txt`)

- 68 `WITH RECURSIVE`, 41 distinct texts. One (`quality-conditional-1/descendant_roots`, #24) holds no recursive CTE: its arms are plain `UNION`s. Two (`imports`, #2 and #11) hold two recursive CTEs each. Hence **69 recursive CTEs**.
- Recursion family: syntax/scope tree 31, in-file graph (call, dataflow, alias, expression) 25, cross-file graph 7, strings 5, sequence fold 1.
- Set 43, bag 26. Of the 26 bags, 22 derive each row once (17 forests by FD child → parent, 3 counters, 2 shrinking texts), so bag = set; 4 enumerate every simple path and a window then keeps one (`hierarchy` ×2, `transitive_sinks`, `project_sinks`).
- I6: 61 per-content (14 of them evaluated inside a repository plan, each keyed by `__path`), 8 membership (the step itself joins across contents: `resolved_names` ×2, `hierarchy` ×2, `imported_modules`, `reachable`, `annotation_reachable`, `derived_conditionals`).
- Monotone: all 69. Negation and aggregates in the steps read non-recursive relations only; windows and `GROUP BY` over a recursive result run after its fixpoint, in the same stage (stratified). `block_results` reads its own row's JSON state, a per-row function.
- Lattice: `set` 43, `best_witness` 16 (`±depth` ×12, `(depth, route)` ×2, `(length(route), route)` ×2), `min_level` 1, **none 9**.

### 2.2 How the 60 that fit, fit

- **As written (42):** set-semantics CTEs that are plain reachability or keyed reachability (a seed's key carried unchanged: `root`, `fn`, `symbol`, `token`, `kind`).
- **Through a registered rewrite (18), three families, each with an I2 side condition:**
  1. *Bag over a forest is a set* (FD child → parent): `scoped_nodes` (#19) → `set`; `scope_chain` (#22) → `min_level` (the unique distance is the minimum).
  2. *Override propagation down a forest is a nearest-ancestor witness*: `ownership` ×12 (`fn`, `cls`: nearest function/class; `top_fn`: farthest) → three `best_witness(key = ±depth)` over ancestors-or-self, or, without any fixpoint, an interval probe. REWRITES.md notes that `ownership`'s readers observe its level order; a lattice fact has no order, so those readers become I1 runtime-checked ties.
  3. *Path enumeration + `row_number() = 1` by an extension-monotone total key is a best witness*: `hierarchy` ×2 → `best_witness(key = (depth, route))` (a least key is a shortest path, hence simple, so the `seen` guard never drops it; route is total given the owner, NARROWINGS § 2.1). `transitive_sinks`, `project_sinks` → `best_witness(key = (length(route), route))` per (fn, kind), then a non-recursive pick by `(length, [priority,] start(node), node, route)`: the pick's tie-break on the last call site is replaced at every extension, so the propagated key must drop it. `length(route)` is the route *text's* length, a positive integer weight per hop (id text + 1), so this is (min, +) with weights, not unit levels. Only the narrowed order is total; the original ties are I1 refusals.
- **Removing the fixpoint instead:** the same declared structure (FD child → parent, preorder intervals) turns every tree closure (`descendants` ×10, `demanded`, and the forest members of families 1–2) into an interval probe with no rounds, R9's form; today R9 fires only on the `reachability` syntax stage [INFERENCE: its side condition, core `node` ids, is what the rules' text-id `edges` stages miss].
- **The 9 that do not fit** need no join at all: each row has one derivation. They are value folds along a unique path or an iterated function:
  - string path-folds on a forest: `scopes` (#1, qualname), `class_names` (#34, #44);
  - a path-text carrier: `call_paths` (#20, dotted names along attribute/call chains; the P3b classifier calls it a simple-path enumeration);
  - string unfolds: `tokens`, `relative` in `imports` (#2, #11, ×2 each);
  - a sequential fold with a JSON accumulator: `block_results` (#27). As a lattice it would be a bottom-up boolean over the block tree with a ∀-children test: monotone, but it needs counting or negation over the recursive relation (outside L3), stratifiable only by tree depth.
  Their work: 2.5% of the recursive algebraic rows on facts-a (`contexts` 332,218, `call_paths` 2,027,270, `imports` 367,304, `block_results` 6; state-flow-1 `contexts` 43,638), `E0/runs/20261029T133958-corpus-8rules/e0-tables.md`.

### 2.3 Open question 2 (type join)

No rule computes a least upper bound of types. `taints` carries a taint kind from a finite set, but kinds never combine: `direct_sinks`, `transitive_sinks` and the findings partition by kind. `executor_types`, `bad_configs`, `return_kinds`, `derived_conditionals` compute sets of names. `set` over (node, kind) is exact for all of them. The only interval-valued recursion in what runs is outside the 68: the linker's `link_result` carries a hop range `[lo, hi]` (`facts/src/front/reference_edges.sql`).

### 2.4 What the engine does with them today

- **Rewritten away:** R9 turns `quality-contracts-1/descendants` (a `reachability` syntax stage, not one of the 68) into an interval probe (EXPLAIN `IntervalProbe`; 15,260,454 rows, no rounds). None of the 68 is rewritten into an interval probe: the ten `descendants` walks and `quality-conditional-1`'s Reach run as the closure operator (EXPLAIN `Fixpoint semi-naive set closure`, the R18 tree walk when its per-execution checks hold). `ownership`, `scoped_nodes`, `scope_chain` plan as `bag levels` (R19 reference walk when its shape check holds). Which runtime path ran was not instrumented.
- **Refused:** facts-a refuses all of `internal-dead-code` at compile (`file.module`). The generator and M2 refuse bag recursion in repository plans (`gen/norm.rs`: "recursive CTE with UNION ALL (bag fixpoint) in a repository plan"), so `imports`, `hierarchy`, `project_sinks`, `transitive_sinks`, `block_results` run there only as NARROWINGS' set rewrites. Nothing in the 68 is refused for non-monotonicity.
- **Classes (P3b):** every clause with a recursive CTE classifies `unbounded` or `O(n²)`; `call_paths` and `contexts` are named "simple-path enumeration (it extends a path value)".

### 2.5 The table (one row per recursive CTE; `#` = occurrence)

| # | rule | clause / stage | CTE | recurses over | set/bag (derivations) | value carried beyond membership | monotone | I6 | catalogue lattice | fit (side condition / alternative) | runs today |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | execution-authority-bypass | authority-sites / contexts (stage 2) | `scopes` | scope tree (`parents`: nearest enclosing def/class, one parent per frame) | bag; one derivation per row: forest FD | qualname (text concatenated along the root path), depth (function-nesting counter) | yes | per-content | none: string path-fold | **none**: non-recursive: ancestor frames by interval containment + ordered `group_concat`; depth = count of function ancestors | facts-a, per file; `fixpoint bag levels` |
| 2 | internal-dead-code | unreachable-internal-symbols / imports (stage 5) | `tokens` | strings (comma split of an import list; `rest` strictly shrinks) | bag; one derivation per row: shrinking text | item, rest (text) | yes | per-content (in a repository plan) | none: string unfold (iterated text function) | **none**: a split table function (non-recursive) | facts-a refuses (compile: `file.module`); M2/generator run the w3 text (its bag stages narrowed to sets) |
| 2 | internal-dead-code | unreachable-internal-symbols / imports (stage 5) | `relative` | strings (drop `remaining` trailing package components) | bag; one derivation per row: counter | prefix (text), remaining (counter) | yes | per-content (in a repository plan) | none: counter-bounded string unfold | **none**: a non-recursive string function (drop k components) | facts-a refuses (compile: `file.module`); M2/generator run the w3 text (its bag stages narrowed to sets) |
| 3 | internal-dead-code | unreachable-internal-symbols / resolved_names (stage 6) | `names` | re-export graph (`imports` ⋈ `declarations` by fqname) | set | symbol (origin declaration, a key) | yes | membership | set, keyed by symbol (multi-source closure) | fits: NOT EXISTS reads `declarations` only (stratified) | facts-a refuses (compile: `file.module`); M2/generator run the w3 text (its bag stages narrowed to sets) |
| 4 | internal-dead-code | unreachable-internal-symbols / hierarchy (stage 8) | `walk` | class hierarchy (`base_edges`) | bag; many derivations: every simple path | depth (hops), route (base ordinals as text), seen (visited list, cycle guard) | yes | membership | best_witness(key=(depth, route)) per (owner, base) | fits: the stage keeps row_number()=1 by (depth, route): a least key is a shortest, hence simple, path, so `seen` never removes it; route total given the owner (NARROWINGS § 2.1) | facts-a refuses (compile: `file.module`); M2/generator run the w3 text (its bag stages narrowed to sets) |
| 5 | internal-dead-code | unreachable-internal-symbols / imported_modules (stage 19) | `modules` | strings (module name → parent package) | set | — | yes | membership | set over interned module names, edge m → parent(m) computed per node | fits: or non-recursive: every dotted prefix of each target | facts-a refuses (compile: `file.module`); M2/generator run the w3 text (its bag stages narrowed to sets) |
| 6 | internal-dead-code | unreachable-internal-symbols / reachable (stage 22) | `flow` | reference graph × dispatch (`reach_edges`, `base_edges`, `dead_traits`) | set | dispatch (method being dispatched; part of the state) | yes | membership | set over (node, dispatch) states | fits: Y2: one product-graph reachability, maintained by level witness | facts-a refuses (compile: `file.module`); M2/generator run the w3 text (its bag stages narrowed to sets) |
| 7 | internal-dead-code | unreachable-internal-symbols / annotation_reachable (stage 23) | `annotated` | annotation references (symbol → unique class by name) | set | — | yes | membership | set | fits | facts-a refuses (compile: `file.module`); M2/generator run the w3 text (its bag stages narrowed to sets) |
| 8 | legacy-execution-surfaces | legacy-execution-graphs / ownership (stage 5) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a, per file; `fixpoint bag levels` |
| 9 | legacy-execution-surfaces | legacy-execution-graphs / receiver_tokens (stage 16) | `tokens` | receiver graph in-file (`receiver_edges`) | set | token (identifier text of the seed, a key) | yes | per-content | set, keyed by token | fits | facts-a, per file; `fixpoint set closure` |
| 10 | legacy-execution-surfaces | legacy-execution-graphs / eager_reach (stage 21) | `reaches` | in-file call graph (callers, non-legacy) | set | — | yes | per-content | set | fits | facts-a, per file; `fixpoint set levels` |
| 11 | public-api-triples | exported-families / imports (stage 5) | `tokens` | strings (comma split of an import list; `rest` strictly shrinks) | bag; one derivation per row: shrinking text | item, rest (text) | yes | per-content (in a repository plan) | none: string unfold (iterated text function) | **none**: a split table function (non-recursive) | facts-a repository phase (as written); M2/generator: the w3 text (its bag stages narrowed to sets); `fixpoint bag levels` |
| 11 | public-api-triples | exported-families / imports (stage 5) | `relative` | strings (drop `remaining` trailing package components) | bag; one derivation per row: counter | prefix (text), remaining (counter) | yes | per-content (in a repository plan) | none: counter-bounded string unfold | **none**: a non-recursive string function (drop k components) | facts-a repository phase (as written); M2/generator: the w3 text (its bag stages narrowed to sets); `fixpoint bag levels` |
| 12 | public-api-triples | exported-families / resolved_names (stage 6) | `names` | re-export graph (`imports` ⋈ `declarations` by fqname) | set | symbol (origin declaration, a key) | yes | membership | set, keyed by symbol (multi-source closure) | fits: NOT EXISTS reads `declarations` only (stratified) | facts-a repository phase (as written); M2/generator: the w3 text (its bag stages narrowed to sets); `fixpoint set levels` |
| 13 | public-api-triples | exported-families / hierarchy (stage 8) | `walk` | class hierarchy (`base_edges`) | bag; many derivations: every simple path | depth (hops), route (base ordinals as text), seen (visited list, cycle guard) | yes | membership | best_witness(key=(depth, route)) per (owner, base) | fits: the stage keeps row_number()=1 by (depth, route): a least key is a shortest, hence simple, path, so `seen` never removes it; route total given the owner (NARROWINGS § 2.1) | facts-a repository phase (as written); M2/generator: the w3 text (its bag stages narrowed to sets); `fixpoint bag levels` |
| 14 | serverplan-dispatch-architecture | executor-provenance / ownership (stage 6) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a, per file; `fixpoint bag levels` |
| 15 | serverplan-dispatch-architecture | executor-provenance / descendants (stage 10) | `walk` | syntax tree (`edges`) | set | root (a key) | yes | per-content | set, keyed by root (reflexive transitive closure) | fits: non-recursive as an interval probe; EXPLAIN shows the closure operator here, not R9 [INFERENCE: R9's side condition wants core `node` ids, `edges` is a text-id stage] | facts-a, per file; `fixpoint set closure` |
| 16 | serverplan-dispatch-architecture | executor-provenance / executor_types (stage 11) | `types` | name aliases in-file (assignments `l = r`, class headers naming a type) | set | name (text) | yes | per-content | set over interned names | fits: the header arm is a regex built from the recursive value: edges must be materialised as (word in header → class) | facts-a, per file; `fixpoint set levels` |
| 17 | serverplan-dispatch-architecture | executor-provenance / annotations (stage 13) | `accepted` | annotation syntax (`annotation_edges`: child → enclosing type node) | set | — | yes | per-content | set | fits | facts-a, per file; `fixpoint set closure` |
| 18 | serverplan-dispatch-architecture | executor-provenance / owners (stage 21) | `owners` | in-file dataflow (`propagation` ∪ `carrier_edges`) | set | — | yes | per-content | set | fits | facts-a, per file; `fixpoint set closure` |
| 19 | serverplan-quality | quality-contracts-1 / scoped_nodes (stage 26) | `walk` | syntax tree below each function, stopping at nested def/class/lambda | bag; one derivation per row: forest FD | fn (a key) | yes | per-content | set, keyed by fn | fits: bag = set under FD child → parent | facts-a, per file; `fixpoint bag levels` |
| 20 | serverplan-quality | quality-contracts-1 / call_paths (stage 30) | `names` | attribute/call chains in-file (`path_edges`) | set | path (dotted text, concatenated along the chain) | yes | per-content | none: path-text carrier | **none**: classifier: simple-path enumeration; on a forest (one object per attribute) a path-fold, non-recursive as an ordered chain aggregate | facts-a, per file; `fixpoint set levels` |
| 21 | serverplan-quality | quality-contracts-1 / preop_functions (stage 36) | `imperative` | in-file call graph by name | set | — | yes | per-content | set | fits | facts-a, per file; `fixpoint set levels` |
| 22 | serverplan-quality | quality-contracts-1 / scope_chain (stage 39) | `chain` | scope tree (`scopes.enclosing`) | bag; one derivation per row: forest FD | depth (hops to the ancestor) | yes | per-content | min_level, keyed by scope | fits: on a forest the unique distance is the minimum | facts-a, per file; `fixpoint bag levels` |
| 23 | serverplan-quality | quality-conditional-1 / ownership (stage 6) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content (in a repository plan) | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a repository phase; generator: narrowed (bag refused); `fixpoint bag levels` |
| 24 | serverplan-quality | quality-conditional-1 / descendant_roots (stage 10) | — | no recursive CTE (`WITH RECURSIVE` over plain `UNION` arms) | — | — | — | — | — | n/a | facts-a repository phase; generator: narrowed (bag refused) |
| 25 | serverplan-quality | quality-conditional-1 / descendants (stage 11) | `demanded` | syntax tree below `descendant_roots` (edges query of a Reach stage) | set | — | yes | per-content (in a repository plan) | set | fits: followed by the Reach stage's own closure (set) | facts-a repository phase; generator: narrowed (bag refused); `fixpoint set levels + fixpoint set closure` |
| 26 | serverplan-quality | quality-conditional-1 / derived_conditionals (stage 19) | `derived` | class hierarchy by name (`class_bases`) | set | — | yes | membership | set | fits | facts-a repository phase; generator: narrowed (bag refused); `fixpoint set levels` |
| 27 | serverplan-quality | quality-conditional-1 / block_results (stage 26) | `fold` | sequence counter (`contract_blocks.sequence + 1`, per file) | bag; one derivation per row: counter | state (JSON map block → raises, rebuilt each step) | yes | per-content (in a repository plan) | none: sequential fold with a JSON accumulator | **none**: as a lattice: a bottom-up boolean over the block tree with a ∀-children test: monotone, but counting/negation over the recursive relation (outside L3); stratified by tree depth | facts-a repository phase; generator: narrowed (bag refused); `fixpoint bag levels` |
| 28 | serverplan-state-tripwires | state-signatures-1 / ownership (stage 5) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a, per file; `fixpoint bag levels` |
| 29 | serverplan-state-tripwires | state-signatures-1 / descendants (stage 9) | `walk` | syntax tree (`edges`) | set | root (a key) | yes | per-content | set, keyed by root (reflexive transitive closure) | fits: non-recursive as an interval probe; EXPLAIN shows the closure operator here, not R9 [INFERENCE: R9's side condition wants core `node` ids, `edges` is a text-id stage] | facts-a, per file; `fixpoint set closure` |
| 30 | serverplan-state-tripwires | state-signatures-1 / bad_configs (stage 18) | `bad` | dataclass field-type graph in-file | set | — | yes | per-content | set | fits: NOT EXISTS reads non-recursive relations only | facts-a, per file; `fixpoint set levels` |
| 31 | serverplan-state-tripwires | state-signatures-1 / shape_values (stage 21) | `shape` | syntax tree upward (`descendants`) + intra-function assignment flow | set | fn (a key) | yes | per-content | set, keyed by fn | fits | facts-a, per file; `fixpoint set levels` |
| 32 | serverplan-state-tripwires | state-flow-1 / ownership (stage 5) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a, per file (default entryPoint); `fixpoint bag levels` |
| 33 | serverplan-state-tripwires | state-flow-1 / descendants (stage 10) | `walk` | syntax tree (`edges`) | set | root (a key), expand (flag fixed at the root) | yes | per-content | set, keyed by root | fits: expand only gates which roots step | facts-a, per file (default entryPoint); `fixpoint set closure` |
| 34 | serverplan-state-tripwires | state-flow-1 / contexts (stage 13) | `class_names` | class nesting tree (`class_parents` via `ownership.cls`) | bag; one derivation per row: forest FD | name (qualified class name, concatenated) | yes | per-content | none: string path-fold | **none**: non-recursive: ordered ancestor aggregate | facts-a, per file (default entryPoint); `fixpoint bag levels` |
| 35 | serverplan-state-tripwires | state-flow-1 / return_kinds (stage 19) | `runtime` | in-file call graph by name (callers passing a carrier) | set | — | yes | per-content | set | fits | facts-a, per file (default entryPoint); `fixpoint set levels` |
| 36 | serverplan-state-tripwires | state-flow-1 / taints (stage 22) | `taint` | in-file expression graph + intra-function assignment flow | set | kind (finite taint label), fn (a key) | yes | per-content | set over (fn, node, kind): a finite powerset of labels, union join | fits: kinds never combine (readers partition by kind): no type join needed | facts-a, per file (default entryPoint); `fixpoint set levels` |
| 37 | serverplan-state-tripwires | state-flow-1 / semantic_values (stage 23) | `semantic` | syntax tree upward + intra-function assignment flow | set | fn (a key) | yes | per-content | set, keyed by fn | fits | facts-a, per file (default entryPoint); `fixpoint set levels` |
| 38 | serverplan-state-tripwires | state-flow-1 / reachable (stage 27) | `reachable` | in-file call graph (`call_graph`) | set | — | yes | per-content | set | fits | facts-a, per file (default entryPoint); `fixpoint set closure` |
| 39 | serverplan-state-tripwires | state-flow-1 / transitive_sinks (stage 28) | `sink` | in-file call graph reversed, within `reachable`, stopping at direct sinks of the kind | bag; many derivations: every simple path | node (last call site), kind, origin, route ('|fn|…|' text) | yes | per-content | best_witness(key=(length(route), route)) per (fn, kind), then a non-recursive pick by (length, start(node), node, route) | fits: length(route) is a positive weighted length (id text + 1): (min, +) with integer weights; only the narrowed order is total (original ties → I1 refusal) | facts-a, per file (default entryPoint); `fixpoint bag levels` |
| 40 | serverplan-state-tripwires | state-projection-recovery-2 / ownership (stage 5) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a, per file (default entryPoint); `fixpoint bag levels` |
| 41 | serverplan-state-tripwires | state-projection-recovery-2 / descendants (stage 8) | `walk` | syntax tree (`edges`) | set | root (a key), expand (flag fixed at the root) | yes | per-content | set, keyed by root | fits: expand only gates which roots step | facts-a, per file (default entryPoint); `fixpoint set closure` |
| 42 | serverplan-state-tripwires | state-flow-project-1 / ownership (stage 5) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content (in a repository plan) | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a repository phase (inputs gated off by default entryPoint: 0 rows); M2: narrowed; `fixpoint bag levels` |
| 43 | serverplan-state-tripwires | state-flow-project-1 / descendants (stage 10) | `walk` | syntax tree (`edges`) | set | root (a key), expand (flag fixed at the root) | yes | per-content (in a repository plan) | set, keyed by root | fits: expand only gates which roots step | facts-a repository phase (inputs gated off by default entryPoint: 0 rows); M2: narrowed; `fixpoint set closure` |
| 44 | serverplan-state-tripwires | state-flow-project-1 / contexts (stage 13) | `class_names` | class nesting tree (`class_parents` via `ownership.cls`) | bag; one derivation per row: forest FD | name (qualified class name, concatenated) | yes | per-content (in a repository plan) | none: string path-fold | **none**: non-recursive: ordered ancestor aggregate | facts-a repository phase (inputs gated off by default entryPoint: 0 rows); M2: narrowed; `fixpoint bag levels` |
| 45 | serverplan-state-tripwires | state-flow-project-1 / return_kinds (stage 19) | `runtime` | in-file call graph by name (callers passing a carrier) | set | — | yes | per-content (in a repository plan) | set | fits | facts-a repository phase (inputs gated off by default entryPoint: 0 rows); M2: narrowed; `fixpoint set levels` |
| 46 | serverplan-state-tripwires | state-flow-project-1 / taints (stage 22) | `taint` | in-file expression graph + intra-function assignment flow | set | kind (finite taint label), fn (a key) | yes | per-content (in a repository plan) | set over (fn, node, kind): a finite powerset of labels, union join | fits: kinds never combine (readers partition by kind): no type join needed | facts-a repository phase (inputs gated off by default entryPoint: 0 rows); M2: narrowed; `fixpoint set levels` |
| 47 | serverplan-state-tripwires | state-flow-project-1 / semantic_values (stage 23) | `semantic` | syntax tree upward + intra-function assignment flow | set | fn (a key) | yes | per-content (in a repository plan) | set, keyed by fn | fits | facts-a repository phase (inputs gated off by default entryPoint: 0 rows); M2: narrowed; `fixpoint set levels` |
| 48 | serverplan-state-tripwires | state-flow-project-1 / project_sinks (stage 31) | `sink` | in-file call graph reversed (project mode) | bag; many derivations: every simple path | node, kind, priority, route | yes | per-content (in a repository plan) | best_witness(key=(length(route), route)) per (fn, kind), then a pick by (length, priority, start(node), node, route) | fits: as transitive_sinks | facts-a repository phase (inputs gated off by default entryPoint: 0 rows); M2: narrowed; `fixpoint bag levels` |
| 49 | serverplan-steps | step-contracts-1 / ownership (stage 3) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a, per file; `fixpoint bag levels` |
| 50 | serverplan-steps | step-contracts-1 / descendants (stage 10) | `walk` | syntax tree (`edges`) | set | root (a key) | yes | per-content | set, keyed by root (reflexive transitive closure) | fits: non-recursive as an interval probe; EXPLAIN shows the closure operator here, not R9 [INFERENCE: R9's side condition wants core `node` ids, `edges` is a text-id stage] | facts-a, per file; `fixpoint set closure` |
| 51 | serverplan-steps | step-contracts-1 / column_derived (stage 28) | `derived` | syntax tree upward + local binding flow | set | — | yes | per-content | set | fits | facts-a, per file; `fixpoint set levels` |
| 52 | serverplan-steps | step-contracts-2 / ownership (stage 3) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a, per file; `fixpoint bag levels` |
| 53 | serverplan-steps | step-contracts-2 / descendants (stage 10) | `walk` | syntax tree (`edges`) | set | root (a key) | yes | per-content | set, keyed by root (reflexive transitive closure) | fits: non-recursive as an interval probe; EXPLAIN shows the closure operator here, not R9 [INFERENCE: R9's side condition wants core `node` ids, `edges` is a text-id stage] | facts-a, per file; `fixpoint set closure` |
| 54 | serverplan-steps | step-contracts-3 / descendants (stage 10) | `walk` | syntax tree (`edges`) | set | root (a key) | yes | per-content | set, keyed by root (reflexive transitive closure) | fits: non-recursive as an interval probe; EXPLAIN shows the closure operator here, not R9 [INFERENCE: R9's side condition wants core `node` ids, `edges` is a text-id stage] | facts-a, per file; `fixpoint set closure` |
| 55 | serverplan-steps | step-contracts-3 / raw_shapers (stage 13) | `shaping` | in-file call graph by name | set | — | yes | per-content | set | fits | facts-a, per file; `fixpoint set levels` |
| 56 | serverplan-steps | step-contracts-3 / payload_values (stage 17) | `values_in_payload` | syntax tree downward (containers, call keywords/arguments, pair values) | set | root (a key) | yes | per-content | set, keyed by root | fits | facts-a, per file; `fixpoint set levels` |
| 57 | serverplan-steps | step-contracts-4 / ownership (stage 3) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a, per file; `fixpoint bag levels` |
| 58 | serverplan-steps | step-contracts-4 / user_callable (stage 14) | `shaped` | in-file dataflow (`callable_edges`) | set | — | yes | per-content | set | fits | facts-a, per file; `fixpoint set closure` |
| 59 | serverplan-steps | step-contracts-4 / direct_callable (stage 15) | `direct` | in-file expression edges into non-identifiers | set | — | yes | per-content | set | fits | facts-a, per file; `fixpoint set levels` |
| 60 | serverplan-steps | step-contracts-4 / mutable_values (stage 16) | `mutable` | syntax tree upward through tuple parents | set | — | yes | per-content | set | fits | facts-a, per file; `fixpoint set levels` |
| 61 | serverplan-steps | step-contracts-5 / ownership (stage 3) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a, per file; `fixpoint bag levels` |
| 62 | serverplan-steps | step-contracts-5 / descendants (stage 8) | `walk` | syntax tree (`edges`) | set | root (a key) | yes | per-content | set, keyed by root (reflexive transitive closure) | fits: non-recursive as an interval probe; EXPLAIN shows the closure operator here, not R9 [INFERENCE: R9's side condition wants core `node` ids, `edges` is a text-id stage] | facts-a, per file; `fixpoint set closure` |
| 63 | serverplan-steps | step-contracts-5 / static_exception_functions (stage 14) | `bad` | in-file call graph by name (returned calls) | set | — | yes | per-content | set | fits | facts-a, per file; `fixpoint set levels` |
| 64 | serverplan-steps | step-contracts-6 / ownership (stage 3) | `owned` | syntax tree (`edges` over `nodes`) | bag; one derivation per row: forest FD | fn, cls (nearest enclosing function/class id), top_fn (outermost function id) | yes | per-content | best_witness(key=±depth) over ancestors-or-self, ×3 (nearest fn, nearest cls, farthest fn) | fits: needs FD child → parent (I2); non-recursive as an interval probe on ancestors; REWRITES.md: readers observe its level order | facts-a, per file; `fixpoint bag levels` |
| 65 | serverplan-steps | step-contracts-6 / descendants (stage 8) | `walk` | syntax tree (`edges`) | set | root (a key) | yes | per-content | set, keyed by root (reflexive transitive closure) | fits: non-recursive as an interval probe; EXPLAIN shows the closure operator here, not R9 [INFERENCE: R9's side condition wants core `node` ids, `edges` is a text-id stage] | facts-a, per file; `fixpoint set closure` |
| 66 | serverplan-steps | step-contracts-6 / user_callable (stage 13) | `shaped` | in-file dataflow (`callable_edges`) | set | — | yes | per-content | set | fits | facts-a, per file; `fixpoint set closure` |
| 67 | serverplan-steps | runtime-reference-summaries / consumers (stage 5) | `reaching` | intra-function flows + return → call sites in-file | set | fn (context function) | yes | per-content | set over (fn, id) | fits | facts-a, per file; `fixpoint set levels` |
| 68 | serverplan-steps | runtime-reference-summaries / refless (stage 6) | `reaching` | as `consumers`, minus calls to `runtime_refs_post_op` | set | fn (context function) | yes | per-content | set over (fn, id) | fits: NOT EXISTS reads `calls`, `locals` (non-recursive) | facts-a, per file; `fixpoint set levels` |

## 3. Counts on facts-a (7 rules, 9,010 files): `E0/runs/20261029T133958-corpus-8rules`

### 3.1 Share of the counted work (H2)

Totals over the 8 rules on facts-a (79 clauses; internal_dead_code refused at compile):

| measure | 8 rules | recursive stages (incl. R9 interval) | share | stages that run a fixpoint | share |
|---|---|---|---|---|---|
| alg = stage_rows + fix_rows | 156,676,167 | 109,086,111 | 69.6% | 93,825,657 | 59.9% |
| alg + matcher (leaf_rows, match_nodes, input_rows) | 3,255,199,336 | 109,086,111 | 3.4% | 93,825,657 | 2.9% |
| phys (index, range, probes, scanned, joined, out) | 2,071,650,397 | 461,160,405 | 22.3% | 414,003,300 | 20.0% |
| thread time, ns (secondary) | 296,097,945,111 | 30,378,054,783 | 10.3% | 27,811,817,112 | 9.4% |

By recursion family (share of the 8 rules' alg / phys):

| family | alg | share | phys | share |
|---|---|---|---|---|
| fold | 6 | 0.0% | 60 | 0.0% |
| igraph | 4,929,552 | 3.1% | 93,890,419 | 4.5% |
| string | 367,304 | 0.2% | 2,372,543 | 0.1% |
| tree | 103,446,844 | 66.0% | 362,968,062 | 17.5% |
| xgraph | 342,405 | 0.2% | 1,929,321 | 0.1% |

Per rule (all its clauses):

| rule | clauses | alg | recursive alg | share | alg+matcher share | phys share | ns share |
|---|---|---|---|---|---|---|---|
| execution_authority_bypass | 1 | 768,278 | 332,218 | 43.2% | 0.3% | 12.2% | 7.6% |
| legacy_execution_surfaces | 8 | 15,289,701 | 7,295,242 | 47.7% | 5.4% | 33.2% | 32.0% |
| public_api_triples | 1 | 1,702,739 | 709,703 | 41.7% | 0.5% | 0.5% | 0.2% |
| serverplan_dispatch_architecture | 9 | 413,476 | 327,980 | 79.3% | 0.3% | 35.5% | 5.6% |
| serverplan_quality | 7 | 39,911,860 | 25,296,990 | 63.4% | 2.7% | 24.7% | 17.2% |
| serverplan_state_tripwires | 32 | 92,248,986 | 70,491,902 | 76.4% | 4.1% | 48.8% | 35.7% |
| serverplan_steps | 21 | 6,341,127 | 4,632,076 | 73.0% | 5.2% | 22.9% | 9.9% |

Per clause with a recursive stage:

| clause | alg | recursive alg | share | alg+matcher share | phys | recursive phys | share | ns share |
|---|---|---|---|---|---|---|---|---|
| execution_authority_bypass/authority-sites | 768,278 | 332,218 | 43.2% | 0.3% | 15,097,617 | 1,836,701 | 12.2% | 7.6% |
| legacy_execution_surfaces/legacy-execution-graphs | 15,289,701 | 7,295,242 | 47.7% | 5.4% | 143,338,992 | 47,562,644 | 33.2% | 32.1% |
| public_api_triples/exported-families | 1,702,739 | 709,703 | 41.7% | 0.5% | 795,294,895 | 4,103,035 | 0.5% | 0.2% |
| serverplan_dispatch_architecture/executor-provenance | 413,476 | 327,980 | 79.3% | 0.4% | 2,822,633 | 1,000,830 | 35.5% | 7.8% |
| serverplan_quality/quality-conditional-1 | 9,497,116 | 3,057,820 | 32.2% | 0.5% | 89,489,797 | 25,016,322 | 28.0% | 17.1% |
| serverplan_quality/quality-contracts-1 | 28,532,298 | 22,239,170 | 77.9% | 9.5% | 366,834,304 | 98,232,545 | 26.8% | 21.7% |
| serverplan_state_tripwires/state-flow-1 | 66,382,036 | 48,431,998 | 73.0% | 11.7% | 405,163,238 | 214,073,431 | 52.8% | 45.7% |
| serverplan_state_tripwires/state-flow-project-1 | 0 | 0 | — | 0.0% | 23 | 3 | 13.0% | 0.0% |
| serverplan_state_tripwires/state-projection-recovery-2 | 4,307,150 | 3,446,602 | 80.0% | 1.0% | 62,514,750 | 9,781,194 | 15.6% | 10.2% |
| serverplan_state_tripwires/state-signatures-1 | 21,559,800 | 18,613,302 | 86.3% | 5.9% | 81,735,495 | 44,523,052 | 54.5% | 25.9% |
| serverplan_steps/runtime-reference-summaries | 650,691 | 290 | 0.0% | 0.0% | 33,332,694 | 265,207 | 0.8% | 0.5% |
| serverplan_steps/step-contracts-1 | 1,621,002 | 1,334,872 | 82.3% | 20.5% | 9,803,836 | 3,970,607 | 40.5% | 16.6% |
| serverplan_steps/step-contracts-2 | 863,651 | 691,212 | 80.0% | 12.2% | 5,523,279 | 2,167,074 | 39.2% | 11.6% |
| serverplan_steps/step-contracts-3 | 859,327 | 697,642 | 81.2% | 12.3% | 6,442,729 | 1,663,920 | 25.8% | 5.9% |
| serverplan_steps/step-contracts-4 | 144,905 | 62,580 | 43.2% | 1.3% | 1,353,694 | 565,522 | 41.8% | 25.3% |
| serverplan_steps/step-contracts-5 | 2,195,525 | 1,840,558 | 83.8% | 25.7% | 9,248,518 | 6,383,595 | 69.0% | 44.0% |
| serverplan_steps/step-contracts-6 | 6,026 | 4,922 | 81.7% | 0.1% | 25,839 | 14,723 | 57.0% | 1.6% |

The eight heaviest recursive stages hold 87.5% of the recursive alg; families: tree. Per-content stage evaluations with non-empty seeds: 16,173 of 96,422.

Reading: the eight heaviest recursive stages hold 87.5% of the recursive algebraic rows (`e0-tables.md`), and all eight are syntax-tree closures: `descendants` (state-flow-1 25.8% of the 7 rules' rows, state-signatures-1 10.7%, quality-contracts-1's interval probe 9.7%, state-projection-recovery-2 1.9%) and `ownership`/`scoped_nodes`. The heaviest graph fixpoints are `receiver_tokens` (1.7%) and `call_paths` (1.3%).

### 3.2 Rounds and frontier per stage (H3)

Per recursive stage (per content: active files only; lockstep: corpus total by round):

| clause / stage | # | family | rows | derivations (fix_rows) | alg share of its rule | alg share of 8 rules | per content: active files | levels p50/p95/max | width p50/p95/max | derivations in rounds ≥ 10K | lockstep levels | lockstep width median / max | lockstep derivations in rounds ≥ 10K | lockstep rounds after the last ≥ 10K round |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| state-flow-1 / descendants | 33 | tree | 20,219,892 | 20,219,892 | 43.8% | 25.8% | 737 of 3095 | 18/28/80 | 275/6,694/88,613 | 37% | 80 | 1,485 / 3,862,590 | 100% | 57 |
| state-signatures-1 / descendants | 29 | tree | 8,398,110 | 8,398,110 | 18.2% | 10.7% | 74 of 3095 | 24/34/35 | 1,114/20,566/88,613 | 66% | 35 | 29,312 / 908,372 | 100% | 15 |
| quality-contracts-1 / descendants | Reach | tree | 15,260,454 | 0 | 38.2% | 9.7% | interval probe | — | — | — | 0 | 0 / 0 | 0% | 0 |
| state-flow-1 / ownership | 32 | tree | 3,862,590 | 3,862,590 | 8.4% | 4.9% | 737 of 3095 | 20/29/82 | 72/1,146/9,708 | 0% | 82 | 131 / 447,840 | 99% | 61 |
| quality-contracts-1 / scoped_nodes | 19 | tree | 2,407,265 | 2,407,265 | 12.1% | 3.1% | 871 of 9010 | 16/24/54 | 36/753/9,791 | 0% | 54 | 229 / 315,385 | 99% | 37 |
| legacy-execution-graphs / ownership | 8 | tree | 2,308,813 | 2,308,813 | 30.2% | 2.9% | 430 of 702 | 20/29/82 | 69/1,195/9,708 | 0% | 82 | 41 / 272,918 | 99% | 62 |
| state-projection-recovery-2 / descendants | 41 | tree | 1,464,236 | 1,464,236 | 3.2% | 1.9% | 22 of 3095 | 21/30/80 | 560/13,084/50,707 | 53% | 80 | 302 / 259,065 | 97% | 65 |
| quality-conditional-1 / ownership | 23 | tree | 1,419,253 | 1,419,253 | 7.1% | 1.8% | repository run | — | — | — | 21 | 3,403 / 518,888 | 98% | 13 |
| legacy-execution-graphs / receiver_tokens | 9 | igraph | 1,338,806 | 1,338,806 | 17.5% | 1.7% | 430 of 702 | 5/9/15 | 58/3,031/36,456 | 21% | 15 | 221 / 910,969 | 100% | 11 |
| quality-contracts-1 / call_paths | 20 | igraph | 1,013,635 | 1,013,635 | 5.1% | 1.3% | 888 of 9010 | 4/8/28 | 22/1,015/24,764 | 14% | 28 | 6 / 693,813 | 99% | 25 |
| state-signatures-1 / ownership | 28 | tree | 908,372 | 908,372 | 2.0% | 1.2% | 74 of 3095 | 24/34/35 | 152/2,222/9,708 | 0% | 35 | 6,607 / 108,915 | 96% | 18 |
| step-contracts-5 / descendants | 62 | tree | 783,086 | 783,086 | 24.7% | 1.0% | 264 of 264 | 10/17/24 | 65/1,357/8,364 | 0% | 24 | 1,241 / 137,184 | 97% | 16 |
| step-contracts-1 / descendants | 50 | tree | 558,576 | 558,576 | 17.6% | 0.7% | 264 of 264 | 8/13/19 | 81/1,241/7,530 | 0% | 19 | 2,370 / 108,849 | 98% | 11 |
| step-contracts-3 / descendants | 54 | tree | 348,704 | 348,704 | 11.0% | 0.4% | 264 of 264 | 4/16/25 | 40/1,139/5,066 | 0% | 25 | 3,429 / 60,855 | 94% | 15 |
| step-contracts-2 / descendants | 53 | tree | 282,769 | 282,769 | 8.9% | 0.4% | 264 of 264 | 7/10/20 | 54/805/3,923 | 0% | 20 | 1,251 / 62,837 | 96% | 13 |
| state-projection-recovery-2 / ownership | 40 | tree | 259,065 | 259,065 | 0.6% | 0.3% | 22 of 3095 | 22/31/82 | 124/2,179/5,624 | 0% | 82 | 21 / 27,450 | 81% | 68 |
| exported-families / imports | 11,11 | string | 95,684 | 271,620 | 21.6% | 0.2% | repository run | — | — | — | 347 | 6 / 157,408 | 88% | 343 |
| authority-sites / contexts | 1 | tree | 166,109 | 166,109 | 43.2% | 0.2% | 6769 of 7416 | 2/3/5 | 6/44/973 | 0% | 5 | 12,224 / 79,632 | 99% | 2 |
| executor-provenance / descendants | 15 | tree | 148,464 | 148,464 | 71.8% | 0.2% | 1 of 450 | 22/22/22 | 4,171/15,502/15,503 | 77% | 22 | 4,171 / 15,503 | 77% | 14 |
| exported-families / resolved_names | 12 | xgraph | 143,023 | 143,023 | 16.8% | 0.2% | repository run | — | — | — | 6 | 1,218 / 74,579 | 93% | 4 |
| step-contracts-5 / ownership | 61 | tree | 137,184 | 137,184 | 4.3% | 0.2% | 264 of 264 | 10/17/24 | 11/253/1,522 | 0% | 24 | 318 / 24,128 | 89% | 16 |
| quality-conditional-1 / descendants | 25,Reach | tree | 130,677 | 88,625 | 0.5% | 0.1% | repository run | — | — | — | 17 | 119 / 46,469 | 94% | 14 |
| step-contracts-1 / ownership | 49 | tree | 108,849 | 108,849 | 3.4% | 0.1% | 264 of 264 | 8/13/19 | 16/230/1,544 | 0% | 19 | 630 / 21,594 | 82% | 12 |
| quality-contracts-1 / scope_chain | 22 | tree | 68,241 | 68,241 | 0.3% | 0.1% | 889 of 9010 | 2/3/5 | 9/142/1,225 | 0% | 5 | 2,309 / 33,368 | 96% | 3 |
| step-contracts-2 / ownership | 52 | tree | 62,837 | 62,837 | 2.0% | 0.1% | 264 of 264 | 7/10/20 | 12/179/1,152 | 0% | 20 | 264 / 17,352 | 67% | 15 |
| state-flow-1 / return_kinds | 35 | igraph | 104,687 | 33 | 0.1% | 0.1% | 9 of 3095 | 1/4/4 | 2/7/7 | 0% | 4 | 5 / 19 | 0% | 4 |
| step-contracts-4 / ownership | 57 | tree | 31,290 | 31,290 | 1.0% | 0.0% | 264 of 264 | 4/5/6 | 5/145/1,152 | 0% | 6 | 264 / 17,348 | 55% | 3 |
| exported-families / hierarchy | 13 | xgraph | 27,528 | 28,825 | 3.3% | 0.0% | repository run | — | — | — | 9 | 1,536 / 13,218 | 46% | 8 |
| state-flow-1 / taints | 36 | igraph | 27,334 | 27,334 | 0.1% | 0.0% | 469 of 3095 | 3/7/10 | 8/58/336 | 0% | 10 | 476 / 12,775 | 47% | 9 |
| state-flow-1 / semantic_values | 37 | igraph | 22,160 | 22,160 | 0.0% | 0.0% | 171 of 3095 | 4/10/14 | 14/117/501 | 0% | 14 | 489 / 8,251 | 0% | 14 |
| state-flow-1 / contexts | 34 | tree | 38,388 | 5,250 | 0.0% | 0.0% | 631 of 3095 | 1/1/2 | 4/28/129 | 0% | 2 | 37 / 5,213 | 0% | 2 |
| executor-provenance / ownership | 14 | tree | 15,503 | 15,503 | 7.5% | 0.0% | 1 of 450 | 22/22/22 | 447/1,807/1,900 | 0% | 22 | 447 / 1,900 | 0% | 22 |
| state-flow-1 / reachable | 38 | igraph | 9,813 | 9,813 | 0.0% | 0.0% | 676 of 3095 | 1/4/6 | 4/30/282 | 0% | 6 | 181 / 6,995 | 0% | 6 |
| step-contracts-6 / descendants | 65 | tree | 2,096 | 2,096 | 0.1% | 0.0% | 2 of 264 | 9/9/9 | 44/320/320 | 0% | 9 | 255 / 364 | 0% | 9 |
| step-contracts-6 / ownership | 64 | tree | 364 | 364 | 0.0% | 0.0% | 2 of 264 | 9/9/9 | 9/56/56 | 0% | 9 | 48 / 63 | 0% | 9 |
| quality-contracts-1 / preop_functions | 21 | igraph | 217 | 217 | 0.0% | 0.0% | 45 of 9010 | 1/3/5 | 2/10/20 | 0% | 5 | 14 / 141 | 0% | 5 |
| state-signatures-1 / bad_configs | 30 | igraph | 169 | 169 | 0.0% | 0.0% | 42 of 3095 | 1/1/2 | 3/10/15 | 0% | 2 | 2 / 167 | 0% | 2 |
| runtime-reference-summaries / consumers | 67 | igraph | 145 | 145 | 0.0% | 0.0% | 21 of 264 | 2/6/7 | 2/9/25 | 0% | 7 | 12 / 42 | 0% | 7 |
| step-contracts-3 / raw_shapers | 55 | igraph | 71 | 71 | 0.0% | 0.0% | 24 of 264 | 1/3/3 | 1/5/11 | 0% | 3 | 13 / 52 | 0% | 3 |
| step-contracts-3 / payload_values | 56 | tree | 46 | 46 | 0.0% | 0.0% | 5 of 264 | 3/3/3 | 1/12/12 | 0% | 3 | 16 / 16 | 0% | 3 |
| state-flow-1 / transitive_sinks | 39 | igraph | 25 | 37 | 0.0% | 0.0% | 7 of 3095 | 2/5/5 | 2/6/6 | 0% | 5 | 9 / 12 | 0% | 5 |
| executor-provenance / annotations | 17 | tree | 11 | 11 | 0.0% | 0.0% | 1 of 450 | 3/3/3 | 4/6/6 | 0% | 3 | 4 / 6 | 0% | 3 |
| executor-provenance / owners | 18 | igraph | 11 | 11 | 0.0% | 0.0% | 1 of 450 | 4/4/4 | 2/4/4 | 0% | 4 | 2 / 4 | 0% | 4 |
| step-contracts-1 / column_derived | 51 | igraph | 11 | 11 | 0.0% | 0.0% | 1 of 264 | 7/7/7 | 1/3/3 | 0% | 7 | 1 / 3 | 0% | 7 |
| step-contracts-5 / static_exception_functions | 63 | igraph | 9 | 9 | 0.0% | 0.0% | 5 of 264 | 1/2/2 | 1/4/4 | 0% | 2 | 1 / 8 | 0% | 2 |
| quality-conditional-1 / block_results | 27 | fold | 2 | 4 | 0.0% | 0.0% | repository run | — | — | — | 2 | 2 / 2 | 0% | 2 |
| quality-conditional-1 / derived_conditionals | 26 | xgraph | 3 | 3 | 0.0% | 0.0% | repository run | — | — | — | 2 | 1 / 2 | 0% | 2 |
| legacy-execution-graphs / eager_reach | 10 | igraph | 2 | 2 | 0.0% | 0.0% | 2 of 702 | 1/1/1 | 1/1/1 | 0% | 1 | 2 / 2 | 0% | 1 |
| executor-provenance / executor_types | 16 | igraph | 1 | 1 | 0.0% | 0.0% | 1 of 450 | 1/1/1 | 1/1/1 | 0% | 1 | 1 / 1 | 0% | 1 |
| step-contracts-6 / user_callable | 66 | igraph | 1 | 1 | 0.0% | 0.0% | 1 of 264 | 1/1/1 | 1/1/1 | 0% | 1 | 1 / 1 | 0% | 1 |
| state-flow-project-1 / contexts | 44 | tree | 0 | 0 | 0.0% | 0.0% | repository run | — | — | — | 0 | 0 / 0 | 0% | 0 |
| state-flow-project-1 / descendants | 43 | tree | 0 | 0 | 0.0% | 0.0% | repository run | — | — | — | 0 | 0 / 0 | 0% | 0 |
| state-flow-project-1 / ownership | 42 | tree | 0 | 0 | 0.0% | 0.0% | repository run | — | — | — | 0 | 0 / 0 | 0% | 0 |
| state-flow-project-1 / project_sinks | 48 | igraph | 0 | 0 | 0.0% | 0.0% | repository run | — | — | — | 0 | 0 / 0 | 0% | 0 |
| state-flow-project-1 / return_kinds | 45 | igraph | 0 | 0 | 0.0% | 0.0% | repository run | — | — | — | 0 | 0 / 0 | 0% | 0 |
| state-flow-project-1 / semantic_values | 47 | igraph | 0 | 0 | 0.0% | 0.0% | repository run | — | — | — | 0 | 0 / 0 | 0% | 0 |
| state-flow-project-1 / taints | 46 | igraph | 0 | 0 | 0.0% | 0.0% | repository run | — | — | — | 0 | 0 / 0 | 0% | 0 |
| state-signatures-1 / shape_values | 31 | igraph | 0 | 0 | 0.0% | 0.0% | 0 of 3095 | — | — | — | 0 | 0 / 0 | 0% | 0 |
| runtime-reference-summaries / refless | 68 | igraph | 0 | 0 | 0.0% | 0.0% | 0 of 264 | — | — | — | 0 | 0 / 0 | 0% | 0 |
| step-contracts-4 / direct_callable | 59 | igraph | 0 | 0 | 0.0% | 0.0% | 0 of 264 | — | — | — | 0 | 0 / 0 | 0% | 0 |
| step-contracts-4 / mutable_values | 60 | tree | 0 | 0 | 0.0% | 0.0% | 0 of 264 | — | — | — | 0 | 0 / 0 | 0% | 0 |
| step-contracts-4 / user_callable | 58 | igraph | 0 | 0 | 0.0% | 0.0% | 0 of 264 | — | — | — | 0 | 0 / 0 | 0% | 0 |

16,173 of 96,422 per-content stage evaluations had non-empty seeds (`e0-tables.md`): the per-file recursion is many small fixpoints, most of them empty.

### 3.3 Reading for H3

- **Per content** (one file's fixpoint, the within-a-round axis): median width ≤ 1,114 rows for every heavy stage (the largest median of any stage is 4,171: executor-provenance `descendants`, one active file); p95 ≤ 20,566; max 88,613 (one file's `descendants` round). Levels p95 ≤ 34, max 80–82 (`ownership`, `descendants`). Derivations in rounds ≥ 10K: 0% for every stage except four `descendants` walks (state-flow-1 37%, state-signatures-1 66%, state-projection-recovery-2 53%, executor-provenance 77%), `receiver_tokens` (21%) and `call_paths` (14%).
- **Lockstep across contents:** round 0 is wide (3,862,590 for state-flow-1 `descendants`), the tail is long and thin: median 1,485 over 80 levels for state-flow-1 `descendants`, 131 over 82 for its `ownership`, 229 over 54 for `scoped_nodes`; only state-signatures-1 `descendants` has a median above 10K (29,312 over 35). Weighted by derivations, 96–100% of the nine heaviest per-file stages' work is in rounds ≥ 10K; after the last such round, 11–65 thin rounds remain (`rounds after the last ≥ 10K round`).
- Repository runs on facts-a are already one fixpoint over all contents: `quality-conditional-1/ownership` 21 levels, median 3,403, max 518,888; `exported-families/imports` 347 levels, median 6.

## 4. Counts on M2 (repository plans): `E0/runs/20261029T134023-m2-3of8-w1`

M2 run `exported-families+state-flow-project-1+unreachable-internal-symbols`: rows_out 56,374,072, derivations 18,904,712, fix_rounds 865; stages with a fixpoint: rows 8,212,964 + derivations 18,904,712 = 36.0% of rows_out + derivations; exclusive seconds 22.2 of 34.9 (63.7%)

| fixpoint (plan/stage) | in the 68? | levels | derivations | frontier median / max | new rows in rounds ≥ 10K | rounds after the last ≥ 10K round | frontier by round (first 12) | stage rows | stage secs | share of the run's rows + derivations |
|---|---|---|---|---|---|---|---|---|---|---|
| exported-families/imports | yes | 328 | 256,161 | 2 / 34,185 | 75% | 325 | 34,185, 34,185, 13,001, 7,131, 4,153, 2,803, 1,970, 1,404, 1,004, 773, 613, 498 … | 938,991 | 0.67 | 1.6% |
| exported-families/imports | yes | 1 | 67,939 | 67,939 / 67,939 | 100% | 0 | 67,939, 0 | 938,991 | 0.67 | 0.1% |
| exported-families/resolved_names | yes | 6 | 225,211 | 338 / 53,674 | 94% | 4 | 53,674, 37,498, 5,031, 338, 14, 2, 0 | 306,455 | 0.14 | 0.7% |
| exported-families/hierarchy | yes | 7 | 35,398 | 920 / 7,734 | 0% | 7 | 7,734, 3,215, 2,460, 920, 214, 97, 10, 0 | 80,458 | 0.04 | 0.2% |
| state-flow-project-1/taints | yes | 0 | 0 | 0 / 0 | 0% | 0 | 0, 0 | 2 | 0.00 | 0.0% |
| state-flow-project-1/semantic_values | yes | 0 | 0 | 0 / 0 | 0% | 0 | 0, 0 | 0 | 0.00 | 0.0% |
| state-flow-project-1/project_sinks | yes | 0 | 0 | 0 / 0 | 0% | 0 | 0, 0 | 0 | 0.00 | 0.0% |
| unreachable-internal-symbols/views/package_climb | no: engine view (`facts/src/front/{reference_edges,module_path}.sql`) | 5 | 15,918 | 999 / 2,247 | 0% | 5 | 2,247, 1,228, 999, 464, 165, 0 | 24,341 | 0.01 | 0.1% |
| unreachable-internal-symbols/views/module_name_part | no: engine view (`facts/src/front/{reference_edges,module_path}.sql`) | 6 | 22,659 | 999 / 2,247 | 0% | 6 | 2,247, 2,247, 1,228, 999, 464, 165, 0 | 26,544 | 0.01 | 0.1% |
| unreachable-internal-symbols/views/module_prefix | no: engine view (`facts/src/front/{reference_edges,module_path}.sql`) | 7 | 31,757 | 1,228 / 2,269 | 0% | 7 | 2,269, 2,269, 2,247, 1,228, 999, 464, 165, 0 | 31,192 | 0.02 | 0.1% |
| unreachable-internal-symbols/views/name_sub | no: engine view (`facts/src/front/{reference_edges,module_path}.sql`) | 4 | 3,115,432 | 56,565 / 389,860 | 99% | 1 | 389,860, 389,860, 56,565, 7,837, 0 | 2,403,562 | 3.69 | 7.3% |
| unreachable-internal-symbols/views/order_dist | no: engine view (`facts/src/front/{reference_edges,module_path}.sql`) | 7 | 42,714 | 859 / 7,837 | 0% | 7 | 7,837, 3,235, 2,446, 859, 128, 33, 2, 0 | 51,457 | 0.02 | 0.1% |
| unreachable-internal-symbols/views/reference_order | no: engine view (`facts/src/front/{reference_edges,module_path}.sql`) | 7 | 35,701 | 930 / 7,837 | 0% | 7 | 7,837, 3,235, 2,472, 930, 220, 99, 10, 0 | 51,720 | 0.03 | 0.1% |
| unreachable-internal-symbols/views/class_of_walk | no: engine view (`facts/src/front/{reference_edges,module_path}.sql`) | 1 | 6 | 1 / 1 | 0% | 1 | 1, 0 | 142 | 0.11 | 0.0% |
| unreachable-internal-symbols/views/fq_class | no: engine view (`facts/src/front/{reference_edges,module_path}.sql`) | 6 | 160,465 | 285 / 25,371 | 69% | 4 | 7,837, 25,371, 3,287, 285, 13, 2, 0 | 118,222 | 0.08 | 0.4% |
| unreachable-internal-symbols/views/link_reach | no: engine view (`facts/src/front/{reference_edges,module_path}.sql`) | 421 | 10,123,137 | 147 / 211,413 | 96% | 402 | 127,333, 193,465, 211,413, 210,626, 188,708, 162,891, 135,845, 106,974, 76,771, 64,968, 54,220, 50,352 … | 2,822,200 | 15.60 | 17.2% |
| unreachable-internal-symbols/views/plain_prefix | no: engine view (`facts/src/front/{reference_edges,module_path}.sql`) | 22 | 40,933 | 198 / 4,012 | 0% | 22 | 4,012, 3,410, 3,089, 1,464, 1,138, 781, 384, 282, 261, 240, 219, 198 … | 56,981 | 0.03 | 0.1% |
| unreachable-internal-symbols/views/link_result | no: engine view (`facts/src/front/{reference_edges,module_path}.sql`) | 5 | 285,735 | 25,270 / 37,044 | 99% | 2 | 32,943, 37,044, 25,270, 866, 18, 0 | 546,591 | 0.22 | 1.1% |
| unreachable-internal-symbols/imported_modules | yes | 3 | 6,424 | 129 / 2,155 | 0% | 3 | 2,155, 129, 4, 0 | 70,895 | 0.03 | 0.1% |
| unreachable-internal-symbols/reachable | yes | 24 | 4,186,684 | 1,663 / 96,601 | 95% | 13 | 9,749, 12,439, 18,447, 37,814, 96,601, 72,140, 29,225, 27,553, 14,804, 12,393, 10,235, 2,858 … | 510,052 | 1.47 | 6.2% |
| unreachable-internal-symbols/annotation_reachable | yes | 2 | 252,438 | 1 / 50,239 | 100% | 1 | 50,239, 1, 0 | 173,159 | 0.06 | 0.6% |

the 68: stage rows 2,080,012 + derivations 5,030,255 = 9.4% of rows_out + derivations; exclusive seconds 2.41 (6.9%)

engine views: stage rows 6,132,952 + derivations 13,874,457 = 26.6% of rows_out + derivations; exclusive seconds 19.82 (56.8%)

Reading:

- Dead code's own recursion (`reachable`, 4,186,684 derivations over 24 levels, frontier median 1,663, max 96,601; 95% of new rows in rounds ≥ 10K) is the one membership fixpoint in the 68 with a wide frontier. It is 1.47 s of the 34.9 s run. On the full oracle tree (`2b1504adc`) and Y2's product graph (flow + dispatch + annotation states), E1 measured the same reachability at 1,038,978 states, 2,951,163 CSR edges, 25 rounds, frontier per round 13,655 (seeds), then 49,188, 254,994, 196,779, … down to 19: median 11,433, max 254,994 (LatticeE1Cpu's runs `~/rerail-bench/facts/explore/lattice-e1/runs/20261029-134656-build-oracle`, `20261029-134710-sweep-oracle`, `counts.changed`; not re-measured here). The membership fixpoint's median is near the 10K line: below it on worktree 1 with the rule's own states (1,663), just above it on the oracle tree with dispatch states (11,433).
- The heaviest recursion in the dead-code plan is not in the 68: the engine's views (`views/*`: the `references` linker and `file.module`, from `facts/src/front/reference_edges.sql` and `module_path.sql`) are 26.6% of rows + derivations and 56.8% of the run's exclusive seconds. `link_reach` alone: 10,123,137 derivations, **421 levels**, frontier median 147, max 211,413, 15.6 s. It is a counter-bounded string rewrite (`d < 19`), `none` in the catalogue; `name_sub` (3,115,432 derivations, 4 levels, 3.7 s) is a string substitution unfold, also `none`. Y1 already replaced `link_reach` with maintained graph code (`explore-y1-graph.md`).
- `imports`' `tokens` (narrowed to a set) takes 328 levels on M2 and 347 (summed with `relative`) on facts-a, at a median of 2 rows: one level per comma-separated name of the longest import list.

## 5. Verdicts, argued

**H1 holds, conditionally.** By the kill line (> 20% outside the catalogue or non-monotone): 9 of 69 CTEs (13.0%), 7 of 68 occurrences (10.3%), 6 of 41 distinct (text, CTE) pairs (14.6%), 0 non-monotone. The condition is L1's: 18 of the 60 that fit are legacy text that becomes a lattice fact only through one of the three rewrite families of § 2.2, each needing a declared FD or a total key. Without those rewrites, 27 of 69 (39%) are not catalogue lattices as written and H1 would be killed. What does not fit is not a missing lattice: all 9 are unique-derivation folds (string path-folds, string unfolds, one JSON fold) that need no join, so an open catalogue (open question 1) would not help them; non-recursive operators would (an ordered aggregate over ancestors, a split table function, an ordered fold).

**H2 is killed as the lever.** The stated line (recursive stages < 20% of the 8 rules' counted work) gives four answers on facts-a depending on the unit: 69.6% of relational rows (holds), 22.3% of physical counts (holds by 2.3 points), 3.4% of the full algebraic count with matcher node visits (killed), 10.3% of thread time (killed, secondary). The relational figure is carried by tree closures: 66.0 of its 69.6 points are syntax/scope-tree recursion, which an interval probe answers without a fixpoint (R9 already does it for quality-contracts-1's `descendants`). The fixpoints that need a lattice evaluator, all non-tree recursion (graphs, strings, the fold), are 3.5% of relational rows and 4.7% of physical counts (in-file graph 3.1% / 4.5%, cross-file 0.2% / 0.1%, strings 0.2% / 0.1%). On the heavy membership rule, dead code, the 68's fixpoints are 9.4% of M2's rows + derivations (6.9% of time); the rest of its recursion is the engine's views, mostly the linker (26.6%, 56.8% of time), whose two heaviest fixpoints are outside the catalogue.

**H3 is killed as stated.** "Median frontier width < 10K ids, or rounds > 100 on the heavy facts": per content, every heavy fact's median is ≤ 1,114 rows; in lockstep across contents, every heavy per-file fact's median is < 10K except state-signatures-1 `descendants` (29,312); dead code `reachable` 1,663 on M2 worktree 1, 11,433 on E1's oracle-tree product graph (25 rounds; the one heavy fact just above the line); levels reach 80–82 (per-file trees), 328–347 (`imports`) and 421 (`link_reach`, dead code's heaviest recursion). What the medians hide: the work is front-loaded. Weighted by derivations, 96–100% of the heavy per-file stages' lockstep work and 95% of `reachable`'s new rows are in rounds ≥ 10K. So per-round parallelism inside one content cannot pay (per-content rounds are tens to hundreds of rows); batching all contents' round k together can, for the first few rounds, with 20–80 thin rounds after them.

## 6. What contradicts LATTICE.md

1. **The heavy recursion is tree closure, not graph fixpoints.** LATTICE.md's thesis and E1 plan centre on reachability over the reference graph. On facts-a, 66.0% of the rows are syntax/scope-tree recursion; the right operator for it is the interval probe (no rounds), which the engine has (R9) but does not apply to the rules' own `edges`-stage walks. A lattice evaluator would make these faster as fixpoints; removing the fixpoint is the larger lever [INFERENCE: not measured].
2. **"Recursion is a monotone fixpoint over a lattice" leaves out 13% of the CTEs and the heaviest recursion in dead code.** The 9 misfits and the linker's `link_reach`/`name_sub` are unique-derivation folds over strings, not joins. The catalogue (open or closed) is the wrong tool for them.
3. **`best_witness` keys are not fixed-width integers.** `hierarchy` orders by a route (a sequence of ordinals), the sinks by a route text's length plus the route. L2's "integer carriers" holds only with per-level ranking of routes (a sort per round), which is not L5's single semiring pass.
4. **H3's axis.** L7 lists three parallel axes; the measured frontiers say only one has width: across contents, in lockstep, for the first rounds. Within a content the frontier median is ≤ 1,114.
5. **Bag recursion.** LATTICE.md says a legacy `WITH RECURSIVE` runs on `Fix` unless a rewrite proves it a lattice fact. Today the repository engines (generator, M2) cannot run bag recursion at all, so NARROWINGS rewrote 5 bag recursions to sets by hand for them (`imports`, `hierarchy`, `transitive_sinks`, `project_sinks`, `block_results`), outside any rewrite registry.
6. **E0's input.** LATTICE.md frames E0 as "from the existing counts on batch 6". The counts existed only summed over files; per content and for M2 they needed the instrumentation of § 1. Also 2 of the 8 rules are batch 5, and dead code does not run on facts-a.

## 7. Proposed amendments to LATTICE.md

1. **H1:** define "fits" as "is a catalogue lattice as written, or through a registered rewrite with its I2 side condition", and register the three families of § 2.2 plus the interval form (forest bag = set; forest override = nearest-ancestor witness; path enumeration + `row_number() = 1` by an extension-monotone total key = `best_witness` + a non-recursive pick; syntax-tree closure over a derived `edges` stage = interval probe, i.e. R9 extended to text-id edge stages). Report the as-written count (42/69) next to the rewritten one (60/69).
2. **Catalogue entry `best_witness`:** state the key as (integer weight sum, lexicographic sequence) with optimal substructure ("extension-monotone"), its per-round ranking cost, and the side condition that the stage's final pick may add tie-breaks that are dropped during propagation (the sinks).
3. **Outside the catalogue, not a user lattice:** add a "unique-derivation fold" non-lattice construct, or require non-recursive operators for it (ordered ancestor aggregate; split table function; ordered fold). These cover the 9 misfits and the linker's two heaviest recursions. Answer to open question 1 from the census: no rule needs a user-defined join.
4. **H2:** name the unit (LOOP.md has three) and split tree closures from graph fixpoints. Measured: graph fixpoints ≤ 4.7% of every facts-a measure; the 68's fixpoints 9.4% of dead code on M2.
5. **H3:** name the axis and weight by work: "≥ X% of a heavy fact's derivations fall in rounds ≥ 10K wide when contents are batched, and the tail after the last wide round is ≤ k rounds". Add the round count of the tail as the synchronisation cost (measured, rounds after the last ≥ 10K round in lockstep: 11–65 for the heavy per-file facts, 13 for `reachable`, 325 for `imports`, 402 for `link_reach`).
6. **E1's input:** the heaviest fixpoint E0 found is state-flow-1 `descendants`, a tree closure; E1 should time it against an interval probe as well as against a semiring round. The heaviest graph fixpoint is dead code `reachable` on M2 (4,186,684 derivations, 24 levels); the heaviest per-file graph fixpoints are `receiver_tokens` (1,338,806 rows) and `call_paths` (1,013,635).
7. **Open question 2:** closed; no rule needs a type join (§ 2.3).

## 8. Not done

- No new operator, no lattice implementation, no rule edits (non-goals).
- Which runtime path (R18 tree walk, R19 walk, generic) each `closure`/`levels` plan took was not instrumented; § 2.4 gives the planned operator only.
- facts-a's fix_rows count admitted rows, not candidate derivations before the distinct; M2 counts both. Set fixpoints' duplicate derivations on facts-a are visible only as physical `rows_joined`.
- Dead code was not run on facts-a (refused) or on the 9,010-file corpus (M2 runs on replay worktrees; worktree 1 has 5,725 contents). Its oracle-tree reached set (896K states) is Y2's (`explore-y2-local.md`), not re-measured.
- `state-flow-project-1` has 0 rows under the default `entryPoint` (its inputs are gated off), on both engines; `scan_project_sources` was not run (native has no oracle for it either, NARROWINGS § 3).
- `imports` sums its two CTEs by round on facts-a (one stage); M2 reports them apart.
- T1 at `865d79e3a` passed (`~/rerail-bench/facts/loop/runs/20261029T135524-t1`: sub200 90/90 findings equal to the reference, ref50 0 mismatches, both 100-event replays 0 differences); T2 is the integrator's.

## Reproduce

```
just facts-t0
target/loop/facts-a repo --threads 8 --no-memo --root ~/rerail-bench/nera-data-layer-2b1504ad --out-dir D --file-counts D/file-counts.jsonl \
  --checks execution_authority_bypass,internal_dead_code,legacy_execution_surfaces,public_api_triples,serverplan_dispatch_architecture,serverplan_quality,serverplan_state_tripwires,serverplan_steps
target/loop/facts-b hreplay --real ~/rerail-bench/facts/explore/lattice-e0/specs/spec-replay-3of8.json --check none --worktrees 1 --workers 1 --run-dir D2
target/loop/facts-a explain --physical --file impl/x.py --checks …        # and --file impl/common/l2070_server_plan/steps/x.py
python3 ~/rerail-bench/facts/explore/lattice-e0/census/census.py && python3 ~/rerail-bench/facts/explore/lattice-e0/census/classify.py
python3 ~/rerail-bench/facts/explore/lattice-e0/analyze.py facts-a D && python3 ~/rerail-bench/facts/explore/lattice-e0/analyze.py m2 D2
python3 ~/rerail-bench/facts/explore/lattice-e0/report.py > docs/facts/reports/lattice-e0-census.md
```
