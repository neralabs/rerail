# Explore Y2: dead code and `exported-families` as local queries over the graph

Branch `Moti-Velz/facts-engine-y2-local` from integration `685db3f76`. Code: `spikes/facts-b/src/y2/` (the rules), `spikes/facts-b/src/gen/y2dump.rs` (a greplay stage dump, `FACTS_Y2_DUMP`), `spikes/y2-local/` (probe rule, specs, summary script); X6's pack copied from `Moti-Velz/facts-engine-x6-rules` into `spikes/x6-rules/`. Runs: `~/rerail-bench/facts/explore/y2-local/runs/`. Machine shared with Y1 (load average 4–6); counts are deterministic, milliseconds are one run each.

## Decision-table rows

| Rule | Per-edit cost, scoped (298 single-file edits, p50 / p95 / p99 / max) | Per-edit cost, unscoped (same edits) | State the rule needs | Exactness | What the rule author must change (Jeremy) |
|---|---|---|---|---|---|
| **`unreachable-internal-symbols`** (dead code) | **0.07 / 1.5 / 3.1 / 8.8 ms**; work p95: 727 dirty states, 15 deleted + 2 inserted edges, 6 states unsupported, 6 toggled, 87 findings re-evaluated | **150 / 161 / 171 / 177 ms** (whole reachability + 74K findings from scratch over the graph); 233–310 ms on the oracle tree | **Not local: reachability is maintained.** Per worktree the reached set with a level per state: 95K–400K states (replay worktrees), 896K on the oracle tree (67K flow + 67K annotated + **761K dispatch**), 12 B/state packed, + 2 dispatch indexes (1.5M words on the oracle tree). 41 worktrees: 15.2M states as copies (182 MB), **1.97M as base + delta (24 MB)**. Inputs it reads, not maintained by the rule: Y1's reference graph, the symbol table (9 `proj_*` facts), per-definition traits (per content) | **0 differences**: oracle 4,931 = 4,931 (`suite-b5`), = dataflow row for row, `reachable` 67,286 and `annotation_reachable` 67,288 = the dataflow's stages; replay prefix 340 checks (scoped = from scratch = dataflow, after every event, 41 worktrees); 800 random delete/re-add steps on the oracle tree (up to 22,895 states toggled per step), 0 differences | (1) declare reachability as reachability (roots + edge queries), not a recursive CTE; (2) dispatch keyed by class, not by method: the reached set 896K → 135K states plus a ≥ 51K-pair class closure that does not depend on reachability; (3) definitions keyed by id with position as payload; (4) no computed join keys (string concatenation, regex) without a stored inverse. § 6 |
| **`exported-families`** | **0.005 / 0.04 / 0.09 / 0.6 ms**; work p95: 110 sources, 14 surfaces, 11 groups, 10 family partitions | **10.7 / 11.0 / 11.4 / 13.3 ms** (all surfaces, groups, families); 17 ms on the oracle tree | **Local, nothing global maintained.** Its keyed intermediate results: 10.9K surfaces, 9.2K groups, 0.5K eligible groups, families and findings, 236K–342K words (≈ 1–1.4 MB) per worktree. Inputs as for dead code minus the graph (`proj_exported_roots`, `proj_public_methods`, traits) | **0 differences**: oracle 0 = 0 (`suite-b5`), surfaces 10,870, groups 9,169, eligible 499, families 489 = the dataflow's stages (family JSON text equal); replay prefix: 127–138 findings per worktree, 340 checks; fuzz 800 steps (0–46 findings) | Nothing for locality: its windows are partitioned by keys (group, family), not global. § 6 |

Kill line (neither rule scoped exactly and unscoped > 5 s): **not reached**. `exported-families` is scoped exactly with no maintained property; dead code is scoped exactly given one maintained property (reachability), and even unscoped it costs 150–310 ms over the graph, against 32 s (M2, linker included) and ≥ 7.6 s (X5, rule logic alone) on the set-at-a-time executor.

What the per-edit figures include and exclude:
- Included: the rule's own work after the edit's input deltas are known: scoping (`prepare`), edge diffs, reachability maintenance, findings (`finish`).
- Excluded, and owned elsewhere:
  - Y1's graph update (here the dataflow's `views/reference_edges`, recomputed per edit, as agreed until Y1 lands).
  - The symbol table's update. It is taken from the dataflow's `proj_*` stages; it is not built here (§ 7).
  - Per-content traits (per-file side, X6: 0.27–0.72 ms per content per rule).
- Applying the deltas to Y2's tables costs 0.05 / 0.40 / 0.91 / 1.5 ms per edit.

## 1. What was built

**Boundary.** Y2 computes every rule-private stage of X6's rewrites; everything shared is input.

| | Input (given) | Computed by Y2 |
|---|---|---|
| dead code | reference edges; `proj_annotation_names`, `proj_imports`, `proj_resolved_names`, `proj_base_edges`, `proj_hierarchy`, `proj_public_methods`, `proj_exported_roots`; per definition `dc__dead_traits` (declaration + 7 booleans read at its own position); per content `dc__module_names`, base-name categories and dynamic-import targets (two per-content probe clauses, `spikes/y2-local/y2-probe.json`, because the findings query and `dead_roots` read `py_bases` / `raw_dynamic_imports` directly) | `class_names`, `imported_modules`, `dead_roots`, `reach_edges`, `reachable`, `annotation_reachable`, `high_confidence`, findings |
| `exported-families` | `proj_exported_roots`, `proj_public_methods`, `ef__traits`, `dc__dead_traits.structural_protocol` | `surfaces`, `slots`, `family_groups`, `eligible_groups`, `families`, `triple_violations`, findings |

- **Inputs.** `gen/y2dump.rs` writes selected outputs of greplay's dataflow (X6 pack `symbols-merged` + the probe plan, `FACTS_STAGE_OUTPUTS=1`):
  - a reference snapshot;
  - each worktree's difference from it at its first touch;
  - each event's row deltas for the edited worktree.
  
  `y2/facts.rs` holds them as keyed multisets with the inverse indexes the queries read (the stand-in for Y1's `GraphView` plus the symbol table; one interner).
- **Graph API used.** The rules read the reference graph only as `edges_from(def)` / `edges_to(def)`, keyed by (path, start) at both ends. Against Y1's `GraphRead` the adapter is `edges_from(path, fqname)` → `def(dst).(path, start)`, with the target kind read from `def(dst)` as Y1 does (agreed with Y1Graph; not wired, Y1 had not committed).
- **Driver.** `facts-b y2 --dump F --out D [--scratch-every N | --fuzz N --seed S]`. Per event: classify the deltas into presence changes, `prepare` both rules under the old state, apply, `finish`. Then the checks of § 2.

### 1.1 Dead code (`y2/dc.rs`)

The rule's two recursions are one reachability over a product graph of states (node, layer):
- `flow(node, NULL)`;
- `flow(node, d)`, a dispatch search for method `d`;
- `annotated(node)`.

Every edge is a **local query at the source node** (`succ`). It reads the node's definition, its `edges_from`, and keyed symbol-table rows: the 11 arms of `reach_edges`, the 5 dispatch rules, and the annotation step through `unique_classes`. Roots are a local predicate. `high_confidence` and the categories are a local query per definition, over its definition, its class, its hierarchy rows and the reached bits of itself and its bases.

**Reachability is maintained**:
- Each reached state carries a level (its BFS depth at scratch). The invariant: every reached non-root state has a reached predecessor of strictly lower level whose edge exists.
- Insertions propagate forward.
- Deletions take a deleted edge's target, or a deleted root, in increasing level. The state is removed only if no lower-level predecessor with an edge is left. Predecessors come from `pred`, a superset; each candidate is verified against `succ`. A removal checks the removed state's higher-level successors in turn.
- Removed states with any reached predecessor are then re-derived and propagated.

**First version: plain DRed.** It over-deletes the forward closure of the deleted edges' targets. That was exact but slow:

| | DRed | Level witness |
|---|---|---|
| States over-deleted, max | 388,040 (of ≈ 400K reached) | 80 |
| Edits above 50 ms | 96 of 298 | 0 |
| Scoped p95 | 394 ms | 1.5 ms |
| Scoped max | 426 ms | 8.8 ms |

DRed is slow because every definition has an edge to its module node and module nodes reach module-level names, so the forward closure of almost anything is half the graph. The level witness finds the module node's other supports immediately. Run: `…/y2-dred/`.

### 1.2 `exported-families` (`y2/ef.rs`)

- A surface is a local query at its source: an exported root's definition (function arm), or an exported class with its public methods (method arm).
- Surfaces are stored by source and by group G = (scope, owner, base_name).
- Slots and the group row are a query at G. Eligibility is a query at G and its exemplar.
- Families are a query at the partition F = (scope, base_name, implementations). The `row_number()` over owners is evaluated inside the partition.
- Violations and findings are computed per F, and per source for function-scope mixed names.

## 2. Exactness

| Check | Result | Run |
|---|---|---|
| Oracle tree `2b1504adc`, dead code vs `suite-b5` (`findings_parity.py`) | **4,931 = 4,931**, 0 missing, 0 extra | `runs/20261023-210008-dump-oracle/y2/parity` |
| `exported-families` vs `suite-b5` | **0 = 0** | same |
| Reference graph vs native (`edges_parity.py`) | **145,431 = 145,431**, 0 missing, 0 extra | same |
| Dead code vs the dataflow's stages | findings row for row; `reachable` 67,286 and `annotation_reachable` 67,288 as sets | same, `y2.log` |
| `exported-families` vs the dataflow's stages | surfaces 10,870, groups 9,169, eligible 499 by count; the 489 family JSON strings equal | same |
| **Replay prefix**: S3's events, `--limit 300`, all 41 worktrees touched, `--scratch-every 1` | **340 checks, 0 differences**. At every event: scoped findings = from-scratch findings = the dataflow's `dc__findings` / `ef__findings` (both rules); scoped reached set = scratch, per layer and in dispatch-state count; = the dataflow's `reachable` / `annotation_reachable`. 83 edits changed dead-code findings, 2 changed `exported-families` findings | `runs/20261023-211927-dump-replay-300/y2` |
| Fuzz on the oracle tree: 2 × 400 steps, each removing or restoring 1–16 random rows across all 13 input tables, checked against scratch | **0 differences**. Up to 23,923 states unsupported and 22,895 toggled in a step; dead-code findings 4,931–5,350; `exported-families` findings 0–46 | `runs/20261023-210008-dump-oracle/y2-fuzz-e-s{1,2}` |

The dataflow side of the replay check is M0's incremental engine. X6 checked it against the engine from scratch on this prefix (0 differences).

## 3. Per-edit cost

298 single-file edits of the prefix: delete_fn 77, rename_fn 59, call 52, import 48, whitespace 40, revert 22. Run `…-dump-replay-300/y2/summary.json`.

| ms | p50 | p95 | p99 | max |
|---|---|---|---|---|
| dead code, scoped | 0.072 | 1.52 | 3.09 | 8.78 |
| dead code, unscoped | 149.8 | 160.7 | 171.4 | 177.3 |
| `exported-families`, scoped | 0.005 | 0.042 | 0.093 | 0.62 |
| `exported-families`, unscoped | 10.7 | 11.0 | 11.4 | 13.3 |
| applying the deltas to the inputs | 0.046 | 0.40 | 0.91 | 1.49 |

Work counts per edit (p50 / p95 / max):

| | Counts |
|---|---|
| Input presence changes | 46 / 683 / 2,474. By table, p95: reference edges 394, definitions 104 + ef traits 104, imports 34, hierarchy 16, bases 8, annotations 8, resolved 4, public 3 |
| Dead code | dirty states 22 / 727 / 6,093; edges deleted 0 / 15 / 150, inserted 0 / 2 / 18; unsupported 0 / 6 / 80; re-derived 0 / 0 / 3; states toggled 0 / 6 / 36; findings evaluated 7 / 87 / 302; `succ` calls 45 / 1,428 / 12,196 |
| `exported-families` | sources 10 / 110 / 298; surfaces recomputed 0 / 14 / 482; groups 0 / 11 / 266; partitions 0 / 10 / 218 |

- **Bulk.** The one bulk event (200 files replaced) costs 281 ms scoped against 151 ms unscoped: 279K dirty states. A scheduler should fall back to scratch when the dirty set passes a fraction of the reached set [INFERENCE: e.g. ¼].
- **Fuzz on the oracle tree.** 896K states; random deletions, harsher than edits. Dead code scoped costs 0.8 / 17 / 217 ms.

## 4. The scoping rule and why it is exact

**Rule.**
- An edit's input deltas become presence changes: a row of a table appears or disappears.
- Each change names the keys it occupies in the indexes the rules read, misses included.
- Under the *old* state, the inverse index of each keyed lookup gives the readers of those keys:
  - dead code: the product-graph states whose `succ` or root predicate reads the key, and the definitions whose finding reads it;
  - `exported-families`: the surface sources, and the groups holding a changed definition.
- Derived keys are expanded to the keys they read, or compared by value: `class_target(module, name)` (imports → resolved names → class definitions) and `unique_class(name)` (the class count by name).
- A definition whose row changed only in position (an edit above it in its file) is read only by position readers: its own reference edges and the edges into its old and new positions.
- Dead code recomputes `succ` at the dirty reached states before and after the edit. The differences are the changed edges, and they drive the maintained reachability (§ 1.1). Findings are re-evaluated at the dirty definitions, and at those reading a node whose reached bit toggled (the node itself, the subclasses of a toggled base, and their methods).
- `exported-families` recomputes the dirty sources' surfaces. Their old and new groups are recomputed, and so are the old and new family partitions of those groups.

**Proof.**
- Every local query is a deterministic function of the rows at the keys it reads.
- Suppose none of the keys a query read under the old state changed. Then every lookup returns the same rows, the evaluation takes the same path, and the result is the same (X5 § 2's read-set argument).
- So an edit can change a state's edges, a root, a surface, a group or a finding only if one of its old reads changed. That is exactly the reader set computed above, from old-state inverse indexes.
- For reads of the reached set: a finding that read the reached bit at node n is re-evaluated if n toggled, through the hierarchy index under the new state. If the hierarchy row it used was deleted, the deletion itself made it dirty; otherwise the new state's index still finds it.

**Reachability is exact by its invariant.**
- (⊆) Every reached state has a chain of strictly decreasing levels down to a root.
  - Kept states were verified.
  - Re-derived and propagated states take level = supporter + 1 over a verified edge.
  - Any state whose supporter left or whose edge went was checked, because it is a deleted edge's target or a higher-level successor of a removed state.
- (⊇) Take a new-graph path from a root, and the first state on it not reached. Its predecessor p is reached. Either the edge was old, so the state was reached and was removed, and re-derivation finds p; or the edge is new, so p was dirty, the inserted edge was detected, and it is propagated; or p itself was added by propagation, which follows new edges.
- `pred` must be complete: it is a superset, and every candidate is verified. The fuzz's large deletions exercise it.

The proof is by hand, per rule: the inverse indexes in `dc::prepare` and `ef::prepare`. That is I8's cost: a compiler would have to derive them from the join keys (§ 6).

## 5. The rule-shape item: dead code's reachability is global

"Unreachable from any root" is not local: deleting one edge can un-reach an unbounded chain. Under the replay that is small (≤ 36 toggled states per edit), but the fuzz toggled 22,895 in one step. So the rule needs one maintained property.

The maintained structure is the reached set with levels: a compact set over the graph, not dataflow arrangements.

| | Oracle tree | Replay worktrees (41) |
|---|---|---|
| reached states | 895,971 (flow 67,286; annotated 67,288; dispatch 761,397) | 94,739–399,826 each |
| packed size (state 8 B + level 4 B) | 10.8 MB, + dispatch indexes 6.1 MB | 15.2M states as copies = 182 MB; **base + delta 1.97M states = 24 MB** (deltas against worktree 1: 0 to 307K; 1K–51K for 39 of the 40 others) |
| dispatch states if dispatch were keyed by class | 50,704 instead of 761,397 | 5,149–19,427 instead of 26K–299K |

Against M1's 61M linker + 37M dead-code arrangement records, this is the whole maintained cost of the rule's global part.

**Dispatch dominates: 85 % of the reached states are `flow(class, method)` pairs.**
- The walk the rule's dispatch arms take from method m's class C does not depend on m: down to subclasses, then up to the subclasses' other bases except C, recursively.
- So `flow(x, m)` for all reached methods of C repeats one per-class closure.
- Written as a symbol-table relation `class_walk(C, X)` over `proj_base_edges` (5.5K edges), with edges m → same-named methods of the walked classes, the reached set keeps only flow and annotated nodes. `class_walk` is then a membership fact like `proj_hierarchy`, independent of reachability. Measured: 50,704 distinct (class, node) pairs on the oracle tree.
- The equivalence is [INFERENCE] from the rule text; it was not built.

## 6. The Jeremy list (rule-author changes)

Dead code:
1. **Declare reachability as reachability.** Roots plus edge queries, not a recursive CTE with UNION arms over a product tuple. Edge arms written as keyed lookups at the source are what made `succ`/`pred` local and the maintenance possible. The engine needs to know the recursion is a monotone closure to use a level witness instead of a fixpoint.
2. **Dispatch keyed by class (§ 5).** On the oracle tree the maintained reached set goes from 896K to 135K states (flow + annotated). The class closure that replaces the 761K dispatch states holds ≥ 50,704 (class, class) pairs and is a membership fact, independent of reachability.
3. **Key definitions by id, with position as payload.** `dead_traits` rows carry `__start`/`__end`, so every definition below an edit changes rows: p95 104 per edit. Exact scoping then needs the "moved only" refinement (§ 4). Before it and the two value refinements (structural-protocol filter, `unique_class` by value), the dirty set was 19,692 states at p95 instead of 727, and scoped p95 was 34 ms instead of 1.5 ms (run `…/y2-levels-v1`). Reference edges are keyed by position for the same reason: 394 changed rows at p95.
4. **No computed join keys without a stored inverse.**
   - `s.id IN (d.class_id || '.' || d.base_name || '_lazy', …)` and `base_name = rx_replace(name, '_(?:async|lazy)$', '')` are joins on strings built from the row. The inverse (which `d` reads `s`) had to be written by hand: split `s.id` at each dot, strip the suffixes.
   - Store the variant name as a column of the definition fact, or declare the inverse.
5. **The module hub is the rule's meaning, and it is coarse.** Every definition has an edge to its module node (`d → @module:<m>`), and an imported module reaches every module-level name. "Reachable" is close to "its module is imported and its name is used at module level". That is why plain DRed's closure was half the graph. If that is the intent, keep it: the level witness handles it. If not, it is the place to tighten the rule.
6. **The probe facts.** The findings query and `dead_roots` read `py_bases` and `raw_dynamic_imports` inside cross-file stages. As declared per-definition facts (two booleans, one target), they stay on the per-file side.

`exported-families`:
- Nothing for locality: its windows are keyed.
- Its cost sits in its inputs, `proj_public_methods` (418K rows, inherited methods via `proj_hierarchy`) and `proj_exported_roots`, which are symbol-table facts.

Both rules: the symbol table (§ 7).

## 7. What this does not cover

- **Symbol-table maintenance.** Y1's graph carries the linker's output only (Y1Graph: no bases, hierarchy, public methods, roots, imports, resolved names, annotations, parameters). Those 9 `proj_*` inputs came from the dataflow's stages here, and their per-edit update is not costed.
  - By the SQL, they are keyed lookups and windows partitioned by keys, plus two closures over small graphs: `proj_resolved_names` (re-export chains through imports, 143K rows) and `proj_hierarchy` (transitive bases with routes, 27.5K rows over 5.5K base edges). [INFERENCE: both fit the same maintained-closure treatment as § 1.1.]
  - X5 measured them from scratch on M2's executor: `resolved_names` 0.43 s, `hierarchy` 0.15 s, `imports` 1.72 s.
  - They are the remaining piece between Y1's graph and these rules.
- **Per-content traits** (`dead_traits`, `ef__traits`, module names, probes) came from the dataflow too. They are per-content computations (X6 § 5).
- **Input tables are not compact.** Y2's tables are generic hash maps: the oracle-tree process footprint is 1.14 GB, holding the parsed dump. The replay process is 4.4 GB, holding all 41 worktrees' parsed rows. These are not memory figures for the design: Y1 owns the compact graph. Y2's own state is counted in words above.
- **Replay length.** Only the 300-event prefix was run, not the full replay. With the dataflow needed to produce the deltas (≈ 45–65 GB for 1,111 events), the machine's memory (swap 28 of 29 GB in use) did not allow it next to Y1.

## 8. Reproduce

1. Probe exports:
   ```
   facts-a xexport --rule spikes/y2-local/y2-probe.json --clauses y2-probe,y2-dyn --jobs …/s3-crossfile/exports/{oracle-b6,replay}-jobs.tsv --out …/y2-local/exports/{oracle,replay}-y2-probe.jsonl --stages reference
   ```
2. Stage dumps:
   ```
   FACTS_STAGE_OUTPUTS=1 FACTS_Y2_DUMP=D/dump.tsv FACTS_Y2_OUTPUTS=/stage/dc__dead_traits,/stage/ef__traits,/stage/proj_base_edges,/stage/proj_hierarchy,/stage/proj_public_methods,/stage/proj_exported_roots,/stage/proj_imports,/stage/proj_resolved_names,/stage/proj_annotation_names,/stage/dc__module_names,/stage/views/reference_edges,:y2-probe,:y2-dyn,/stage/dc__findings,/stage/ef__findings,/stage/dc__reachable,/stage/dc__annotation_reachable facts-b greplay --plans none --real spikes/y2-local/specs/replay.json --events …/seed-5eedb/events.jsonl --limit 300 --no-oracle --run-dir D
   ```
   For the oracle tree: `--real …/oracle.json --parity …/oracle-b6-jobs.tsv`, plus the intermediate stages listed in `runs/…-dump-oracle/dump.tsv`'s `O` lines.
3. Rules and checks:
   ```
   facts-b y2 --dump D/dump.tsv --out D/y2 --scratch-every 1
   facts-b y2 --dump D/dump.tsv --out D/y2-fuzz --fuzz 400 --seed S
   ```
4. Oracle parity:
   ```
   scripts/findings_parity.py D/y2/parity --clause y2:dc --check internal_dead_code --jobs … --suite …/s5-gate/suites/suite-b5.json   (y2:ef: public_api_triples)
   scripts/edges_parity.py D/y2/parity --clause y2:edges
   ```
5. Numbers: `spikes/y2-local/y2_summary.py D/y2`.
6. `cargo test -p facts-b`: 17 passed, 3 of them Y2's (the rule's `rx_replace` prefix chain, `imported_modules`, and the sibling-name inverse).

Commits on `Moti-Velz/facts-engine-y2-local` (not pushed): `fbce97df5` (code), and this report.
