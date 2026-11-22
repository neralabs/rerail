# t50: The reference graph's shape: where the rules spend, and the operators that make them fast

Status: assessment and plan (2026-11-20). Follows decision 0483 (the reference graph is rules).

## Intro

Decision 0483 replaced W's Rust reference graph with `.rerail` rules (`lib/python/references.rerail` over `lib/python/names.rerail`).
The rows match W's. The run's CPU went from 38–40 s to 105.5 s on the `all` corpus (3,089 files, 9.6 M nodes). W's
graph cost about 6 s of that 38 s, so the rules spend about 73 s where W's Rust spent 6.

The rules don't lose because a generic executor must lose. They lose because the engine runs every rule as a
general join: the scope walk, the "latest binding before this read" search and the dotted-name prefix walk are each
materialized row by row. W runs each of them as a specialized algorithm. The plan is to recognize those rule shapes
and run them as specialized operators, each regular enough for Metal. That is how `nearest` already works: a tree
axis solved in one preorder pass, or by pointer jumping on the device (decisions 0038, 0040), never by joins.

## 0. The data

### Where the CPU goes, top down (span own time, every thread)

Run `after5`: an unloaded native run with `RERAIL_PROFILE` on, profiling build. The spans' own time sums to 97.0 s of
the process's ~105 s. Nothing large is unaccounted for. The "25–45 s unattributed" in an earlier report came from
summing each span's inclusive CPU on its own thread, which leaves out the jobs it ran on the pool's other threads.

| Part | After (s) | Before (s) |
|---|---|---|
| Per-file chunks (`wait/…`): projection, the matcher wave, the per-file rules | 63.6 | 4.1 |
| … of which the per-file rules' strata | 46.3 | 0.3 |
| Cross-file part (`cross`) | 15.9 | 4.9 |
| … of which its strata | 15.3 | 4.8 |
| `local` (export to cross 5.0, documents built) | 9.8 | 16.3 |
| Phase 1 (the source files, the import fixpoint) | 3.0 | 2.9 |
| Store writer, frames | 2.9 | 0.9 |

Before, W's graph ran inside `local` (`CrossFileRelations::add`, during the documents' build).

### Bottom up (macOS `sample`, every thread, 1 ms)

Run `after7`: the same program and corpus at load ~20, so only the proportions count. Busy samples, by the
innermost engine phase on the stack:

| Phase | After | Before |
|---|---|---|
| Rule joins (`native::setfix`) | 46 % | 17 % |
| Projection: batch build, `Projected` reads | 13 % | 10 % |
| Store writer (kept facts) | 11 % | 19 % |
| Tape reads, the parser | 9 % | 21 % |
| Allocator | 9 % | 10 % |
| Frames, kept facts | 3 % | 1 % |
| AST matcher wave | 2 % | 4 % |

Instructions retired: 1,666 G after, 485 G before.

- **The join engine's own functions lead:**
  - `runs::first_print`, 4.5 %;
  - `run_join_in_each::enter`, 3.8 %;
  - `runs::Index::find_at`, 3.6 %;
  - `enter::candidates`, 1.9 %;
  - `RowRef::at`, 1.6 %.
- **Then the projection:** `Projected::local_of`, 1.9 %, and `Projected::node`, 1.7 %.
- **The store writer is mostly waiting on durability.** Of its busy samples:
  - 47 % are `fcntl` and 24 % are `msync`, from `Block::sync_mapped` under `KeptW::finish`;
  - 163 relations are kept per wave, against 64 before.

W's graph in the before sample (share of busy samples; seconds scaled to the unloaded 38 s):

| Stage | Busy | ~CPU s |
|---|---|---|
| Scope table (`scope::python::Visitor`, the table's build) | 6.6 % | 2.5 |
| Summary walk (`Extractor::walk`) | 5.6 % | 2.1 |
| Linking (`closure::Graph`: link, union, member, MRO) | 3.4 % | 1.3 |
| All of `CrossFileRelations::add` | 15.7 % | 6.0 |

### The per-file rules by the transformation of W's they stand for (46.3 s)

| W's transformation | Rules | CPU s |
|---|---|---|
| (generated) factorized exports, demand, prerequisites | `pre_*`, `dm_rf_qn_0`, `fl*_*` | 10.8 |
| T5: the binding a read sees | `rf_bind`, `rf_fn_seen_at`, `rf_seen_at`, `rf_mlatest`, `rf_names`, … | 7.7 |
| (the dead-code program's own per-file rules, and demand) | `inventory`, `raw_*`, `demand_*`, … | 6.8 |
| T1: the scope table's references | `nameref`, `bind_only`, `tp_*` | 4.7 |
| T4: reference sites and heads | `rf_site`, `rf_expr`, `rf_head`, `rf_annot_head`, … | 4.3 |
| T2: the scope-chain lookup | `wstart`, `walk`, `look` | 4.0 |
| T1: the scope table's bindings | `binding`, `defn`, `asg`, `param`, `tgt`, `ann_*`, … | 3.9 |
| T3: the enclosing definition | `rf_in`, `rf_fn_in`, `rf_top`, `rf_owner` | 2.5 |
| T6: the rest of the summary | `rf_def`, `rf_qn`, imports, bases | 0.9 |

The cross-file strata (15.3 s) are led by:

| Rule | CPU s |
|---|---|
| `rf_hit` (prefix walk) | 3.7 |
| `rf_target` | 2.2 |
| `public_best_len` | 1.1 |
| `flow0` | 0.8 |
| `rf_ref` | 0.7 |
| `rf_method`, `rf_super_method`, `rf_member_fn` | 1.3 |

Tools and runs: `tmp/refs/sample.sh`, `tmp/refs/tools/{st,thr,inc,spans}.py`, runs `tmp/refs/runs/{after5-all-native,
after7-all-sampled,before3-all-sampled}`.

## 1. Why there is a cross-file step

Three things genuinely need other files:

1. **A name's definition lives in another module:** an import, a re-export chain of up to 16 hops, a package's
   `__init__`.
2. **A class's bases live in other files:** its order of bases, the method `self.f` reaches, a field's type.
3. **The dead-code program's reachability is over the whole graph** (`flow0`).

All three are joins on names over each file's summary: its definitions by qualified name, its imports and aliases,
its bases, its field declarations and its references by form. W does all three in about 1.3 s, over small tables.

The step could be avoided only by bringing each file's imported modules into its chunk as guests (the import
fixpoint, 0083), over and over. That repeats the work and still leaves (3) global. **So the cross-file step belongs
in the shape, but small: its input is the summaries, about one row per definition, import and reference.**

Today it is big for three reasons, none of them inherent:

- **Per-file work has leaked into it.**
  - `rf_pydoc` reads the program's setting facts (`references_given`, `references_include`, `references_exclude`),
    and the split doesn't treat them as constants of the file-local part.
  - So `rf_module` is cross-file, and so is everything that reads it: `rf_qn`, `rf_owner_class`, `rf_ref` (13
    rules), `rf_names`, `rf_init_arg` (18 rules), `rf_same_qn`, `rf_redefines`.
  - The split then factorizes their per-file parts into exports: `fl*` and `xd_fl*`, about 75 export relations.
  - It also makes demand for `rf_qn` over every node of every file: `dm_rf_qn_0($f) :- docof($f, $d)`, 34 M rows of
    work.
  - This accounts for about 10.8 s of the generated per-file strata, the 5.0 s export to cross, and about 2 s of
    cross strata.
- **The prefix walk works on strings.** `rf_walk`/`rf_hit` climb a qualified name one dot at a time with `split_dot`,
  `substr` and `strlen`. Each step makes and interns a new symbol and tests `!rf_binder(p)`. W's names carry their
  structure (`Name::new_all`), so a prefix is a lookup, not a string built.
- **"First by an order" is a quadratic self-join.** `rf_method_first` and its kind are written as `X(k, v),
  !later(k, v)` with `later(k, v) :- X(k, v), X(k, w), w < v`, comparing every pair in a group. This is the form I
  introduced in 565dbba022 when `min` over symbols was refused.

## 2. Each transformation: W's way, the rules' way, why slower, the operator

The rule shapes below are recognized structurally (the body's shape, never a relation's name). Each operator is a
plan choice the planner prices against the general join, as an axis already is (0143). Each has a CPU form and a
Metal form, and nothing is precomputed.

### T1, T4: the scope table, reference sites and heads (W ≈ 2.5 s of the table; rules 12.9 s)

- **W:** one recursive visitor over the tree. The current scope, the enclosing statement and the field a node sits
  in are its context, held in stack variables. Each binding goes into its scope's `HashMap<name, Vec<Binding>>` in
  source order, and each reference into a vector sorted by start.
- **Rules:** about 40 relations (`nameref`, `bind_only`, `binding`, `defn`, `rasg`, `param`, `tgt`, `tscope`,
  `rf_site`, `rf_expr`, `rf_head`, …). Each is its own join program over `kind`, `field`, `child`, `text` and
  `nearest`.
- **Why slower:**
  1. About 40 passes over the batch instead of one. Each scans a `kind` index and probes `field`/`child` per row.
  2. Negated parents, `!field(_, "attribute", n)` and `!field(_, "function", i)`, become anti-joins probed per row,
     where the visitor knows its parent for free.
  3. Every intermediate is materialized: sorted, deduplicated, indexed, and kept (the store writer's syncs).
  4. The text of every identifier is interned, and `text(n, x)` is read per row.
- **Recognize:** a rule whose body is a tree pattern around one node: `kind`, `field`, `child`, `text`, a negated
  parent edge, `nearest` to a kind set. The variables it binds are the pattern's captures.
- **Operator: the matcher wave with captures.** Decision 0106 §4 already lowers tree patterns into the transducer's
  one-pass matchers, but only parts with no captures. Its evidence says "most rules read again the nodes their fields
  bind, and those parts wait for captures." Captures are that missing piece:
  - a matcher emits `(root, capture…)` rows in tape order, so they come out sorted;
  - negated parent edges become `inside`/`not field-of` conditions of the matcher;
  - all of T1 and T4 then run in one tape pass with the existing matchers. 0106 measured 22 ms of CPU for two
    matchers over 211 K nodes.
  - Metal: the matcher wave is already a device program (the label passes).

### T2: the scope-chain lookup (W: within the table; rules 4.0 s)

- **W:** per reference, follow the scope's parent pointers. At each scope, probe the scope's bindings of the name,
  skipping class scopes and handling comprehensions and `nonlocal`. Nothing is materialized.
- **Rules:**

  ```text
  walk(n, x, s) :- nameref(n, x), wstart(n, s), …
  walk(n, x, p) :- walk(n, x, s), !bound_in(s, x), nearest(s, p, lfscope)
  look(n, x, s) :- walk(n, x, s), bound_in(s, x)
  ```

- **Why slower:** a row for every (reference, scope) pair until a binder is found, built one semi-naive iteration per
  nesting level. Each iteration does an anti-join with `bound_in`, a `nearest` step and dedup. `wstart` alone
  expands 34 K → 2.3 M rows in one plan.
- **Recognize:** the shape "start `S(n, x, s0)`; step `s → p` by a functional relation (an axis) while `!B(s, x)`;
  answer where `B(s, x)`". That is the nearest ancestor along a chain that satisfies a keyed predicate.
- **Operator: keyed nearest.**
  - CPU: one preorder pass with a stack of scopes, each with its binder set, resolving each reference by probing up
    the stack. This is W's algorithm, but derived from the rule.
  - Metal: binders and references sorted by `(x, preorder)`. Each reference's answer is the nearest enclosing binder
    interval of its key, by a segmented scan per key over the Euler intervals. That is sort, scan, scatter, with no
    serial walk.
  - The skip rules (a class scope seen only by its own body, comprehension first-iterator starts) belong to the step
    relation and stay rules.

### T3: the enclosing definition (W: the extractor's owner context; rules 2.5 s)

- **W:** the summary walk carries "the outermost definition whose body holds this" as it descends.
- **Rules:** `rf_in(n, g) :- rf_site(n), nearest(n, g, fdef), rf_body(g, b), within(n, b)`, then the recursive
  `rf_fn_in`, `rf_top` and `rf_owner`. One plan enumerates `within` from the body: 53 K → 5.1 M rows, kept 1.1 M.
- **Why slower:**
  - `within` with both sides bound is an interval test, but the plan enumerates it;
  - "the outermost along a chain" is built one level per iteration.
- **Operators:**
  - **`within` with both bound is an O(1) interval test, always.** It is never enumerated from the larger side. That
    is a planner rule, and needs no new operator.
  - **The chain root:** `top(g, t)` where `t` is the end of the up-chain by a functional step. CPU: the same
    preorder pass as `nearest`. Metal: pointer jumping, as 0040's `nearest` does.

### T5: the binding a read sees (W: binary search in the scope's bindings; rules 7.7 s)

- **W:** `ScopeTable::visible(scope, name, crossed, at)`: the scope's bindings of the name in source order, and the
  last one before `at`, found by search.
- **Rules:**
  - `rf_fn_seen_at(i, u) :- rf_fn_read(i, s, x, r), …, u = max t : { rf_fn_at(s, x, _, t), t < r }`;
  - the same shape for `rf_seen_at`, `rf_mlatest_at`, `rf_mreset`;
  - counts per group: `rf_mlive_n`, `rf_fn_n`.
- **Why slower:** each read joins every binding of its `(scope, name)` group, filters `t < r` and aggregates.
  `rf_seen_at` makes 2.2 M rows between steps to keep 17 K. The group is joined, not searched.
- **Recognize:** an aggregate `max t : { B(k…, t), t < r }` whose key `k…` and bound `r` come from the probe's row.
- **Operator: the as-of join** (predecessor search).
  - `B` sorted by `(k, t)`, one search per probe, or one merge when the probes are sorted too, as matcher rows come
    out in tape order.
  - Metal: merge-path over the two sorted sides.

### "First by an order" (cross: `rf_method_first`, `rf_super_first`, `rf_field_first`, `rf_compose`; also `type_first`, `public_best_route`)

- **W:** a loop over the order that returns at the first hit (`Graph::lookup` over the MRO).
- **Rules:** `X(k, v), !later(k, v)` with `later` as a self-join on the key, quadratic in the group.
- **Recognize:** exactly that shape, with `later(k, v) :- X(k, v), X(k, w), w < v` (or a lexicographic pair of
  comparisons).
- **Operator: group argmin.** One pass over `X` sorted by `(k, v)`, keeping each group's first. Metal: a segmented
  min.
- This also gives back what refusing `min` over symbols took away. The order is explicit (the text order of `<`), and
  the operator keeps the group's least under it in linear time.

### C1: name to definition (W: hash maps by name, memoized per name; rules: cross `rf_hit` 3.7 s, `rf_target` 2.2 s, …)

- **W:** `by_name` and `aliases` are hash maps on interned `Name`s, which know their segments. `link_name` recurses
  through aliases with a memo, up to 16 hops.
- **Rules:** `rf_want`, `rf_link` and `rf_link_via` recurse over symbols. The prefix walk builds strings.
- **Recognize:** `split_dot`/prefix climbing until `rf_binder(p)` is the T2 shape (keyed nearest) over the forest of
  dotted names: a name's parent is its prefix.
- **Operator: the name forest plus keyed nearest.**
  - Qualified names are interned once into a name table with parent ids. That is a derived relation written once,
    not a copy.
  - The longest bound prefix is keyed nearest over that forest, with the T2 operator.
  - The alias recursion stays a rule.

### C2: bases in order (W: C3 per class, memoized; rules: `rf_order` with string routes)

- **Rules:** the route is a string built with `@printf("%06d.")` and `@concat`, under a shortlex lattice.
- **Operator:**
  - the route as a number pair (depth, then slot path as a packed integer where it fits);
  - "first class in the order with member f" is the group argmin above.
  - A string route stays the fallback past the packed width.

### The overheads around the rules

- **What is kept.**
  - 163 relations are kept as facts per wave, against 64. The store writer spends its time in `fcntl` and `msync`
    syncing them.
  - Only the file-local part's exports and the outputs need to be kept: the per-file memo (0246) replays the part
    from them.
  - Keep the exports; don't keep the intermediates.
- **Projection.** `BROAD_SCALAR_PROJECTION`, 7.4 s: 18 M node visits and 4.0 M strings interned. Intern a node's
  text only where a rule reads it into a variable, and per matcher capture once T1 is in the matcher wave.

## 3. The plan

Each step starts from the test that pins it, keeps the rows equal to W's (`reference_edge_is_ws_reference_graph_on_*`
and the `all` row digest), and is measured on `all`, native and Metal, before the next starts. The expected gains are
estimates from the tables above, not measurements.

| # | Step | Removes (est. CPU s) | Test first |
|---|---|---|---|
| 1 | **Split:** a relation computed from run constants (the setting facts) and one document's facts is file-local. `rf_module`, `rf_qn`, `rf_ref`, … go back to the per-file part. No `dm_rf_qn_0` over every node, no factorized `fl*` exports for them. | 12–18 (generated 10.8, export 5.0, cross ~2) | the split of `references.rerail` puts `rf_module` and `rf_ref` file-local; the exports are the summary relations only |
| 2 | **Group argmin** for `X, !later` (and `min`/`max` under an explicit order) | ~1.5 (cross first/later), and the typing programs' `type_first` | the operator's rows equal the self-join's; linear work on a 10⁴-row group |
| 3 | **As-of join** for `max t : { B(k, t), t < r }` | ~5 of T5's 7.7 | `rf_fn_seen_at` and `rf_seen_at` rows equal; work proportional to probes |
| 4 | **`within` both bound is an interval test**, and **chain root** by the `nearest` pass | ~2 of T3's 2.5 | `rf_in`'s plan never enumerates `within` from the body |
| 5 | **Keyed nearest** (CPU stack pass; Metal segmented scan) for the walk/look shape | ~3.5 of T2's 4.0, plus the `look` reads | `look` rows equal; no `walk` rows materialized |
| 6 | **Matcher captures** (0106 §4 extended): T1 and T4 tree patterns in the one matcher pass | ~10 of 12.9 | the two ways' rows equal on the ruff fixtures, as 0106's test pack does |
| 7 | **Name forest** plus keyed nearest for the prefix walk | ~3.5 (`rf_hit`, part of `rf_target`) | `rf_hit` rows equal; no symbols made per step |
| 8 | **Keep only exports and outputs**; intern only text that is read | store writer, ~3 of projection | kept relations = exports and outputs |
| 9 | **Q413:** derived row blocks past the Metal budget spill to the deferrable tier, so the matcher wave gets its room at 512 MB (not a revert) | Metal at the default budget | the `all` run at 512 MB, rows equal |

Steps 1–4 are planner and split changes over existing machinery. Steps 5–7 add the operators. Each operator is its
own decision record when it is built.

Target after all steps: per-file rules about 10 s, the cross part about 4 s, the rest as before. That puts the run in
the low 40s of CPU s against W's 38–40, with every piece of it in rules, and every operator able to run on the device.
This is an estimate, to be replaced by measurements step by step.

## 4. W's pass and the rules, stage by stage

`logic/convert/tests/reference_stage_tests.rs` sets each stage of W's pass beside the rules that stand for it. Each
side's rows are compared, and each side's CPU is timed:

| Stage | W's code | The rules (heads summed by `stage_of`) | Rows compared |
|---|---|---|---|
| scope | `ScopeTable::build_in` | `names.rerail`'s heads but the walk's | `load`: each name read |
| lookup | `resolve_python_index` per read | `wstart`, `walk`, `look`, `miss`, `in_first_iter`, `builtin_ref` | `look`: the scope each read lands on |
| summary | `calls::summarize` | `references.rerail` above `// ==== across files` | `def`, `ref`: W's spelled names |
| link | `Graph::link_references` | `references.rerail` below it | `link`, `open_name`, `edge`, `open` |

`each_stage_of_ws_reference_graph_is_the_rules_on_the_projects` runs it on the calls oracle's projects. `…_on_the_cases`
runs it on one small case per difference found so far. `reference_stages_timed` (ignored; `RERAIL_STAGE_CORPUS`) times it
on a corpus, and with `RERAIL_STAGE_DIFFS=1` it prints every differing row.

On s1000 every stage's rows are now equal: 341,263 reads and lookups, 21,164 definitions, 89,966 references, 53 links,
101 open names. The differences fixed to get there were all in the rules, each with a case:

- a typed splat's name;
- a type parameter's name, PEP 696's default form included;
- a type parameter read;
- a class reached by its path's spelling;
- a dotted `case` pattern's tail, which is an attribute;
- an annotation or default, which is read in the scope around the definition;
- a `global` name, read after its function binds it;
- `nonlocal` resolved through a chain of `nonlocal` declarations;
- a class body reading a name it binds only later;
- `type(self).x = v`, which tree-sitter parses as a type alias;
- a nested function rebinding its own parameter;
- a union annotation written over several lines (SQLite's `trim` removes spaces only);
- the builtins of Python 3.10;
- a scope's last binding when it is unconditional.

CPU per stage on s1000 (profiling build, one farm host, 2026-11-20). W's times vary 2–7× between runs on the farm's
hosts; this run's W total of 1.2 s matches the earlier quiet runs:

| Stage | W s | rules s | rules/W |
|---|---|---|---|
| scope | 0.215 | 2.848 | 13.3 |
| lookup | 0.023 | 1.237 | 53.6 |
| summary | 0.752 | 6.170 | 8.2 |
| link | 0.182 | 2.459 | 13.5 |
| (engine: the run outside the strata) | — | 16.9 | — |
| (generated strata) | — | 5.7 | — |
| (outputs) | — | 1.9 | — |

Two ratios are the worst. The first is the lookup, at 54×: a scope walk materialized as `walk` rows, which step 5's
keyed nearest replaces. The second is the time outside the strata, which is larger than all the stages together.
Steps 1 and 8 address that part.

### Step 1, done: the setting's relations are stated, empty when unset

A `.references` setting's relations (`REFERENCE_SETTING`) are declared by the program and stated by the lowering
(`references_setting`). With no setting given, nothing was stated, so the split saw relations with no location. It
counted every reader as cross-file, and `rf_module`, `rf_qn`, `rf_ref` and their readers went across files. Now the
lowering states each declared setting relation, empty when unset. The split already counts a stated relation as a run
constant (`fact`), so those relations go back to the per-file part. The split's test is
`a_relation_stated_empty_is_a_constant`. A structural rule ("declared and not derived is a constant") was tried first
and was wrong: the engine's world facts (`dir_entry`, `file_line`, …) are declared non-inputs too, and it made
`lib/c/includes.rerail` unstratifiable.

The move surfaced an engine bug. An input's index was kept by its set of key columns, in the column order of the first
caller (the planner's `probe_size`). A join that probed the same columns in another order read its key in its own order
through that index and found nothing. This happens only when a join is cut and its rows are handed to a new order
(decision 0206). On s1000 in two chunks, 1,433 `<unqualified>` references were lost. The fix puts a step's keys in the
index's order (`index_order_tests`). `each_stage_is_the_same_in_small_chunks` checks that the rows do not depend on
the chunking.

s1000 after step 1, every stage's rows equal (same host, 2026-11-20):

| Stage | W s | rules s | rules/W |
|---|---|---|---|
| scope | 0.182 | 2.408 | 13.2 |
| lookup | 0.016 | 1.020 | 65.5 |
| summary | 0.619 | 4.757 | 7.7 |
| link | 0.193 | 1.373 | 7.1 |
| (engine) | — | 8.5 (was 16.9) | — |
| (generated strata) | — | 3.2 (was 5.7) | — |
| (outputs) | — | 2.9 | — |

The rules' total went from 31.2 s to 24.1 s. The lookup's ratio is now the worst by far (step 5).

### Step 3, done: an aggregate bounded by its probe (decision 0511)

`u = max t : { B(k, t), t < r }`, with `r` bound outside the body, was a context of the probes and a helper row for
each probe and each value under it. It is now one helper of the group's values with the bound on the aggregate's op
(`AggOp::Bounded`), and the native step folds each probe's group with the bound as a filter. The checks are
`setfix::bounded_tests`, on two sides. W's side is the plain search for the latest value before the read. The rules'
side is the lowering and the step, on 10⁴ values and 10⁴ probes. The coverage gate's four bounded probes equal
SQLite's rows. On `all` the as-of strata went from 4.68 s to 3.07 s of CPU, with every stage's rows unchanged (on and
off, `RERAIL_ASOF=off`). `rf_fn_seen_at` went from 1.57 s to 0.73 s. The groups are small, so the fold is enough for
now (0511, Measured).

`all`, with the step, same host, 2026-11-20:

| Stage | W s | rules s | rules/W |
|---|---|---|---|
| scope | 1.542 | 25.822 | 16.7 |
| lookup | 0.140 | 9.841 | 70.2 |
| summary | 5.724 | 45.223 | 7.9 |
| link | 1.267 | 16.477 | 13.0 |

The rows on `all` differ from W's in five stages, the same with the step on or off: `load`/`look` 19 reads
(`get_history.py`, and `look` lands on another scope in `test_staged_large_column_application.py`), `ref` 197, and
`open_name` 1 and `open` 6. They are not this step's. They come next, before step 2.

### Step 2, done: the first of a group by an order (decision 0515)

`X, !later` with `later` a self-join that states an order is read as a lattice relation: `min<>`, `max<>` or
`shortlex_min<>` on the value column, and the self-join is gone. Nothing changes in `.rerail`. On `all`, every stage's
rows are as before, and `rf_method`/`rf_super`/`rf_field`'s first and later strata go from 0.907 s to 0.474 s of CPU.
`rf_compose` (symbols under `<`, text order) and `rf_init_later` (the value is not the head's last column) are left as
written: 0.01 s together. Two things were found on the way: `ShortlexMin` joined a stored atom it did not hold in
memory to the top, and the device folded 0511's bounded aggregate as a plain one. Both are fixed; the second is not
yet run on a device.

### Step 4, first half: `within` from a part is priced at its parent's size

`rf_in`'s plan started from `kind(g, "function_definition")`, read each function's body (`field(g, "body", b)`),
enumerated it with `within(n, b)`, and only then checked `nearest(n, g)` and `rf_site(n)`. A node inside nested
functions was enumerated once per function around it. The price of `within` read from its container was the mean
size of a node with children (decision 0299), 3 to 5 nodes, since `b` has no kind atom. A part's subtree is at most
its parent's, so a container that is a `field` or `child` of a node of known kind is now priced at that kind's size
(`plan_search`'s `part_of`; test `within_read_from_a_part_is_priced_by_its_parents_size`).

The plan now starts from `rf_site(n)`, and `within(n, b)` is a check with both sides bound. On `all` (same host, rows
unchanged): `rf_in` 6.52 → 5.65 s, `rf_fn_in` 2.00 → 1.87 s. What is left is linear in the sites (16.6 M rows of
work in `rf_in`'s main lines, about 160 ns a row and step), and the chain `rf_fn_in`/`rf_top`/`rf_owner`, which the
chain root (the second half) is for.

### Step 4, second part: `within` with both sides bound is a test

With its plan starting from the sites, `rf_in` read `within(n, b)` with both sides bound, and so did the generated
strata of the program's other rules. Each such check was a probe of `n`'s rows: the rows of every ancestor of `n`,
made on the first lookup, or the batch's solve, which makes them for every node. Then the probe filtered them for `b`.
A source can now answer a fully bound row with a test (`Source::holds`). The setfix step asks it when the step reads
no column of the row (`need` 0, every key exact), and the candidate it gives is never read. `within` in tree mode
answers with the projection's preorder interval: `n` lies in `a`'s `size`, the interval its made rows are. Nothing
is made (test `within_with_both_given_is_an_interval_test`, against each node's chain of parents).
`RERAIL_WITHIN_TEST=off` probes as before (a measurement aid).

The first version read the tape's `subtree_end` through a reader opened for each call. It saved 15 s in the strata
above and lost about as much in strata that check the same nodes again, where the probe was a hit on rows already
made (`tscope` 0.21 → 1.62 s, `rf_fcond` 0.10 → 1.42 s). The projection's `size` opens no reader, and that loss is
gone.

On `all` (M1 Max, both sides in one session, rows unchanged), instructions retired went 3.677 T → 3.538 T and the
rules' own CPU 312.9 → 272.0 s. By stratum: `rf_in` 6.56 → 4.32 s, `rf_fn_in` 2.24 → 0.86 s, and the generated
`pre_379` 10.57 → 2.82 s, `pre_380` 6.23 → 1.29 s, `pre_378` 4.81 → 2.67 s. On s1000: 371.1 G → 349.6 G
instructions. The hosts were loaded (load 4 to 31 on 10 cores), so wall and CPU moved by 10–40% from one run to the
next. Instructions are the measure here.

### Step 4, the chain root: measured, left for later (user, 2026-11-20)

On `all`, after the test above, the chain costs `rf_fn_in` 0.91 s (295 runs: 0.58 s joins, 0.22 s input load),
`rf_top` 0.18 s and `rf_owner` 0.44 s. `rf_owner` is a plain join of `rf_in` and `rf_top` (5.5 M work), which a
chain root keeps. Only a function in another function's decorators or defaults takes `rf_fn_in`'s recursion. So a
chain root would remove `rf_top` and that recursion: at most 0.5–1 s of the rules' 272 s.

No existing mechanism fits. Rewritten as a closure (`rf_up(g, g)`, `rf_up(g, z) :- rf_up(g, y), rf_fn_in(y, z)`,
`rf_top(g, t) :- rf_up(g, t), !rf_fn_in(t, _)`), it is the reach index's shape (decision 0119). But the reach
index reads its inputs into pair vectors and builds one bitmap per start, for 700 K functions. It would be a new
operator, with its own decision record. The user chose step 5 next and the chain root for later.

What is left of T3 is `rf_in`'s 4.3 s: 26 M rows of work in its first rule, about 185 ns a row and step. That is the
executor's cost per row, not the chain.

### Step 5, before the operator: what an index probe costs, and what else it could be (2026-11-21)

The guess was that the lookup is slow because each probe is a binary search over a large sorted relation, a cache
miss a step. Measured, it is not the main cost.

**Profile of `all`** (one run, macOS `sample` at 1 ms, waiting threads taken out: 298,754 CPU samples, about
299 s; 238 s wall, 322 s user, 3.57 T instructions, peak 12.3 GB). Shares of the run's CPU, nested:

| Part | Share |
|---|---|
| the joins (`run_join_in_each::enter`) | 32.2% |
| … the loop over each entering row (`Rows::each`) | 22.4% |
| … the `any(…)` check loop inside it | 20.5% |
| … finding candidates (`candidates`) | 15.1% |
| … writing and deduplicating rows (`flush`) | 11.0% |
| index probes (`Index::find_at` 4.4%, `Run::find` 2.0%, `Relation::between` 2.3%, nested in part) | ~7% |
| view probes (`View::find_keyed` 3.1%, `View::find_read` 3.0%) | ~6% |
| lazy derived-input orders, built on first probe | ~1.5% |

**The spike** (`setfix/runs/index_spike.rs`, an ignored test over the `walk`, `wstart` and `look` captures of s1000:
`RERAIL_INDEX_SPIKE=<capture dir>`). Each structure answers the same probes, and the found ids' checksum must
agree. One thread, a million probes or more each; ns per probe:

| Table | Probe | today (`runs`) | `printed` | hash (hashbrown) | sorted hash |
|---|---|---|---|---|---|
| 16 K rows | whole key, shuffled | 132 | 87 | 9 | 29 |
| 56 K rows | lead column, shuffled | 100 | 103 | 16 | 38 |
| 56 K rows | whole key, shuffled | 103 | 91 | 17 | 38 |
| 244 K rows | lead column, shuffled | 264 | 268 | 45 | 68 |
| 244 K rows | whole key, shuffled | 273 | 177 | 36 | 77 |
| 244 K rows | miss, shuffled | 285 | 143 | 9 | 49 |

| Structure | Bytes per row | Build ns per row |
|---|---|---|
| today (`runs`) | 6.1–6.4 | 14–25 |
| `printed` | 7.1–7.5 | 16–34 |
| hash | 46–81 | 25–58 |
| sorted hash (64-bit hashes and 32-bit ids, sorted by hash) | 12 | 36–41 |

- Today's index costs 100–285 ns a probe even in key order: each probe still searches a 64-id block, reading each row
  through its id.
- A hash or sorted-hash order answers one key set, where a sorted order answers every prefix of its columns.
- Ranges by document (`dense`) did not apply: a node id's low half is a tape position, not a dense local, so the
  ids are too sparse for offsets. It would need the projection's locals (`Batch::node`).
- One 3-column tagged order of 79 K rows (`look-3:s1`) built at 1,838 ns a row once, against 34 the next time.

**Decision (user, 2026-11-21): no sorted hash.** Its memory (twice today's, 7–13× for hashbrown) is not wanted, and
faster probes would save at most 5–6% of the run's CPU. The data stays here; the structure is not built. The next
measure is the join's per-row work (`any(…)`, `flush`, the view probes).

**The join, by role** (self time under `run_join_in_each::enter`, the same sample; 32.2% of the run): the
interpreter's own work (`enter`, `candidates`, the row closures, the `any` loops) 24.7% of the join, index probes
15.9%, view reads 8.9%, the within/nearest axis 8.5%, `RowRef` 5.6%, alloc and free 5.4%, plain relation reads 5.3%,
thread-locals and clock reads 4.2% (in part the profiler's own span per scalar-summary probe), the head's write and
dedup 3.5%, lazy index builds 3.2%, strings and scalars 2.9%. The inclusive shares of the row closures (22.4%) and
`flush` (11.0%) nest: they recurse into the next step.

**The join, by rule** (`RERAIL_JOIN_LINES` on `all`, no sample: 180 s wall, 294 s user, 3.38 T instructions): 93 K
lines, 872 M rows of work, 104.8 s of line wall over the threads, about 120 ns a row. No rule is over 4.7%. Most run
at 90–140 ns a row: the cost is the executor's per row, spread over every rule. The outliers by ns a row:

| Rule | Rows of work | Wall s | ns/row |
|---|---|---|---|
| `st_open` (one line) | 23 K | 1.20 | 52,329 |
| `walk` (the recursive step: `!bound_in`, `!gbefore`, `rr_nearest_lfscope`) | 3.8 M | 3.60 | 959 |
| `rf_fn_n` (a count) | 2.0 M | 1.18 | 591 |
| `st_ref_at` | 3.1 M | 1.57 | 512 |
| `rf_crossing`, `rasg` | 3.5 M, 2.9 M | 1.04, 0.84 | 293, 290 |

And by size: `st_pos` (a position for every node of every document, from `node` and `doc`) is 70 M rows of work,
4.97 s, the largest rule.

### The rows on `all`: two rules fixed, the rest W's departures from Python

`RERAIL_STAGE_DIFFS=1` listed every differing row. Each kind was cut down to a case of a few lines
(`each_stage_of_ws_reference_graph_is_the_rules_on_the_cases`). Two were the rules':

- **A type alias's bounds** (`type A[T: B = D] = …`). Every identifier of the alias's left side was a binding, so the
  bounds and defaults, which W reads, were not reads (`tp_alias_read`). This was the 19 `load` rows.
- **A type parameter read late.** Under `from __future__ import annotations`, an annotation in a generic function's
  body that names a type parameter the module also binds (`T = TypeVar("T")` beside `def a[T]`) reads the module's
  `T`, as W's `forward_module` does. The rules made every type parameter's read a local. Now a forward read
  (`rf_forward`) is not one. This was 190 `ref` rows and all six `open` rows.

After them, on `all`: `load`, `def`, `link`, `edge` and `open` equal W's, and `look` has the same count. What is left
reaches no edge and no open definition. It is W's scope model departing from Python's, which the rules keep, since
`python/names.rerail` serves the engine's other programs too:

- `look` (29 rows each way):
  - W binds an assignment to a `nonlocal` name in the nearest enclosing function, not the one that owns the name.
  - In an annotation that names its own generic class (`self: C[T]`), W finds no binding.
- `ref` (9 rows only W's, 2 only the rules'), all reads whose order decides:
  - a read of an `except … as e` name from a lambda in the handler;
  - a read before the function's own later import of the name;
  - a class annotation naming a class the function binds later.
- `open_name` (1 row).

## Open

- The dead-code program's own per-file strata (6.8 s) and its `demand_*` rules have not been examined here.
- `public_best_len` (1.1 s cross) and `reach_edges` belong to the program, not to the graph.
