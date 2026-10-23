# Narrowings of order-dependent repository clauses (for Jeremy)

Owner: S3. Status: **ready to send**. Evidence was measured on `2b1504adc` (the suite recordings `suite-b5.json` and `suite-b6.json`). Runs: `~/rerail-bench/facts/phase-2/s3-crossfile/narrowings/<timestamp>-<label>/`. Each run directory holds the `policies/` that were run, `suite.json` and `stderr.txt` (`/usr/bin/time -l`). To reproduce, run `spikes/facts-b/scripts/narrow.py OUT [--diag] [NAME=VALUE]` and then `conformance/data-layer/suite.ts --policies OUT --engine native --cold-only`.

## 1. What the engine cannot accept today, and why

The facts engine's semantics are SQLite's (INVARIANTS I1). Where a query does not fully specify an order, SQLite breaks ties by the order in which it reads its input. That input order is not part of the query, and the incremental engine does not have it: a worktree's relation is maintained as a set of differences, and no order of arrival exists. Each construct below therefore has two possible answers on some input. The engine cannot pick one (I1 forbids tie-breaking that depends on the plan) and must not guess (I9: exact or refuse).

| Construct | Example | Problem |
|---|---|---|
| `row_number() OVER (PARTITION BY p ORDER BY o)`, then `= 1`, where `p, o` is not a key | `declarations`: `PARTITION BY id ORDER BY __start DESC` | Two rows with the same `(p, o)` that differ elsewhere: the one kept depends on input order |
| `LIMIT 1` without `ORDER BY`, or with an `ORDER BY` that ties | `base_edges`: `(SELECT c.id … LIMIT 1)` | Same as above |
| `json_group_array`/`group_concat` over a sort that ties | `traits.params`: `ORDER BY p.__start` | The array's element order depends on input order |
| Recursive CTE with `UNION ALL` (a bag) | `hierarchy.walk`, `imports.tokens` | The dataflow maintains recursion as a set fixpoint. Under a bag, the result's multiplicities count derivations, which a set fixpoint does not compute |

**Engine decision (decider, 2026-10-22).** Order constructs are compiled in two ways.

- *Proven total*: S1's classifier proves the order total from declared keys and FDs (rules K1–K6).
- *Runtime-checked*: the order is not proven total. The dataflow then detects a tie between distinct rows when one occurs. It emits the tie as a named, retractable refusal for that (clause, worktree) and withholds the clause's findings there. The refusal disappears when an edit removes the tie.

The classifier no longer refuses these constructs (S1 `0ea181a84`). The narrowings below are still needed for two reasons:

1. They make the order total by the rule's own data. With them, the corpus has no tie and therefore no runtime refusal (§ 3).
2. They state the author's intent: which row wins. Today that choice is SQLite's input order.

The bag recursions do need the edit. The engine refuses `UNION ALL` recursion at compile time.

## 2. The edits

Each edit is an exact text replacement inside one stage's `query`. The replacement occurs exactly once in that stage. `narrow.py` asserts this. In the tables, `⏎` marks a newline in the stage text. `…` abbreviates text that the replacement leaves unchanged. The byte-exact `(old, new)` pairs are the constants at the top of `spikes/facts-b/scripts/narrow.py`.

### 2.1 `data-layer-internal-dead-code` / `unreachable-internal-symbols`, and `data-layer-public-api-triples` / `exported-families` (shared stages)

| Stage | Replace | With | Why the order was open |
|---|---|---|---|
| `declarations` | `row_number() OVER (PARTITION BY id ORDER BY __start DESC) AS selected FROM inventory` | `row_number() OVER (PARTITION BY id ORDER BY __start DESC,__path) AS selected FROM inventory` | `id` = module + qualname. Two files can map to one module (`pkg.py` and `pkg/__init__.py`). Their definitions can then tie on `__start` |
| `imports` (tokens) | `tokens(path,ordinal,top,base,item,rest) AS (` | `tokens(path,ordinal,top,base,item,rest,position) AS (` | Adds the token's position within its import statement |
| `imports` (tokens) | `SELECT __path,__start,top,base,'',rx_replace(items,'#[^\n]*','') \|\| ',' FROM raw_imports`⏎` UNION ALL SELECT path,ordinal,top,base,trim(substr(rest,1,instr(rest,',')-1),' ()' \|\| char(10) \|\| char(13)),substr(rest,instr(rest,',')+1) FROM tokens WHERE rest<>''` | the same with `,0` appended to the first select list, `UNION` for `UNION ALL`, and `,position+1` appended to the second select list | Bag recursion becomes set recursion. Each row is derived once, because `rest` strictly shrinks |
| `imports` (relative) | `relative(path,ordinal,top,base,original,alias,module,prefix,remaining) AS (` | `relative(path,ordinal,top,base,original,alias,module,prefix,remaining,position) AS (` | Carries `position` |
| `imports` (relative) | `coalesce(length(base)-length(ltrim(base,'.'))-1,-1) FROM parts`⏎` UNION ALL SELECT path,ordinal,top,base,original,alias,module,rx_replace(prefix,'\.?[^.]+$',''),remaining-1 FROM relative WHERE remaining>0)` | `…-1,-1),position FROM parts`⏎` UNION SELECT …,remaining-1,position FROM relative WHERE remaining>0)` | Bag recursion becomes set recursion. `remaining` strictly decreases |
| `imports` (selection) | `PARTITION BY module,name,base IS NULL ORDER BY ordinal DESC)` | `PARTITION BY module,name,base IS NULL ORDER BY ordinal DESC,position DESC,path)` | `from m import a, b as a` binds `a` twice in one statement. Python's later binding wins, hence `position DESC`. `path` separates two files of one module |
| `base_edges` (1st subquery) | `i.name=trim(rx_replace(rx_replace(r.name,'(?s)\[.*$',''),'^.*\.','')) LIMIT 1)` | `… ORDER BY c.id LIMIT 1)` | `LIMIT 1` without `ORDER BY` |
| `base_edges` (2nd subquery) | `c.name=trim(rx_replace(rx_replace(r.name,'(?s)\[.*$',''),'^.*\.','')) LIMIT 1)` | `… ORDER BY c.id LIMIT 1)` | Same |
| `hierarchy` | `SELECT id,id,0,'',',' \|\| id \|\| ',' FROM declarations WHERE kind='class' UNION ALL` | `… WHERE kind='class' UNION` | The walk enumerates simple inheritance paths. As a set, each path appears once: given the owner, the route (the sequence of base ordinals) determines the path. The stage keeps the lexicographic shortest path per (owner, base): the `row_number()` by `(depth, route)`, unchanged |
| `public_methods` | `PARTITION BY h.owner,d.name ORDER BY h.depth,h.route) AS selected` | `PARTITION BY h.owner,d.name ORDER BY h.depth,h.route,d.id) AS selected` | Same-named classes in one module (nested `Meta`) put two methods at one `(depth, route)` |
| `class_names` (both subqueries) | `… AND i.name=r.name LIMIT 1)` and `… c.name=r.name LIMIT 1)` | `… ORDER BY c.id LIMIT 1)` each | `LIMIT 1` without `ORDER BY`. `resolved_names` can map one fqname to several symbols |

### 2.2 `data-layer-public-api-triples` / `exported-families` only

| Stage | Replace | With | Why |
|---|---|---|---|
| `traits` | `FROM parameters p WHERE p.symbol=d.id ORDER BY p.__start)` | `FROM parameters p WHERE p.symbol=d.id ORDER BY p.__start,p.__path)` | `json_group_array` order. One symbol can collect the parameters of two files of one module |
| `slots` | `PARTITION BY scope,owner,base_name,slot ORDER BY ordinal DESC) AS selected` | `PARTITION BY scope,owner,base_name,slot ORDER BY ordinal DESC,symbol) AS selected` | Method scope: `owner` is a class *name*. Same-named classes in different modules tie. These ties occur on the corpus (§ 3) |
| `family_groups` (exemplar) | `ORDER BY x.first_ordinal LIMIT 1) AS exemplar` | `ORDER BY x.first_ordinal,x.slot LIMIT 1) AS exemplar` | Two slots of one family can share `first_ordinal` (a `legacy_async` symbol named like its sync sibling) |
| `triple_violations` | `PARTITION BY f.family ORDER BY d.__path,d.__start) AS selected` | `PARTITION BY f.family ORDER BY d.__path,d.__start,s.symbol) AS selected` | Completes the order with the selected symbol |
| `families` | no change | — | `family_groups` groups by `(scope, owner, base_name)`, so `owner` is already total within `(scope, base_name, implementations)` |

### 2.3 `data-layer-serverplan-quality` / `quality-conditional-1`

| Stage | Replace | With | Why |
|---|---|---|---|
| `block_results` | `SELECT DISTINCT __path,0,'{}' FROM contract_blocks UNION ALL SELECT` | `SELECT DISTINCT __path,0,'{}' FROM contract_blocks UNION SELECT` | The fold advances along `sequence+1` and gives one state per (path, sequence). Set and bag are equal; the engine needs the set |
| `conditional_findings_0` | `a.__start<n.__start ORDER BY a.__start DESC LIMIT 1)` | `a.__start<n.__start ORDER BY a.__start DESC,a.origin LIMIT 1)` | The alias lookup. Ending on the projected column makes a tie harmless by construction |

### 2.4 `data-layer-serverplan-state-tripwires` / `state-flow-1` and `state-flow-project-1`

These are repository clauses only under `entryPoint=scan_project_sources` (`state-flow-1`), or always (`state-flow-project-1`, whose inputs the default `entryPoint=all` gates off).

| Stage | Replace | With | Why |
|---|---|---|---|
| `direct_sinks` (both clauses) | `n.__start) AS chosen FROM direct_events d JOIN nodes n` | `n.__start,d.node) AS chosen FROM direct_events d JOIN nodes n` | Nested nodes start at one byte (`a.b()` and `a.b`) |
| `transitive_sinks` (`state-flow-1`) | `FROM direct_sinks s JOIN reachable r ON r.__path=s.__path AND r.fn=s.fn UNION ALL SELECT` | `… UNION SELECT` | The route makes each derivation a distinct row. Duplicates come only from equal call-graph rows and are collapsed by the selection anyway |
| `transitive_sinks` | `ORDER BY length(s.route),n.__start) AS chosen` | `ORDER BY length(s.route),n.__start,s.node,s.route) AS chosen` | Equal-length routes from different origins tie |
| `project_sinks` (`state-flow-project-1`) | `FROM project_seeds UNION ALL SELECT` | `FROM project_seeds UNION SELECT` | As `transitive_sinks` |
| `project_sinks` | `ORDER BY length(s.route),s.priority,n.__start) AS chosen` | `ORDER BY length(s.route),s.priority,n.__start,s.node,s.route) AS chosen` | As `transitive_sinks` |

## 3. Evidence on `2b1504adc`

**Method.** The edited rules ran through native's relational path (SQLite, native addon `e6d2ac73c`; native is unchanged since except visibility). For each edited stage, `narrow.py --diag` adds a clause that computes three things over the same inputs:

1. The original stage and its narrowed copy (`diff` = rows in exactly one of the two).
2. `rank()` copies of both. `ties` = rows tied at the top of a partition under the original order, beyond the first, counted between distinct output rows: the cases SQLite resolved by input order. `narrowed_ties` = the same under the narrowed order: the cases the dataflow would refuse at run time.
3. The rule's own findings with the narrowed text, compared with the oracle (`suite-b5.json`/`suite-b6.json`, the Python analyzer at `2b1504adc`).

| Clause | Findings, narrowed text vs oracle | Stages diff | Ties under the original order | Ties under the narrowed order | Run |
|---|---|---|---|---|---|
| `unreachable-internal-symbols` | **4,931 = 4,931**, 0 FP, 0 FN, 0 failures | 0 rows in each of 6 stages (`declarations` 74,579 rows, `imports` 95,684, `base_edges` 5,534, `hierarchy` 27,528, `public_methods` 418,068, `class_names` 38,771) | 0 | 0 | `20261022-211541-b5-final` |
| `exported-families` | 0 = 0, 0 failures | 0 rows in each of 9 stages (shared 5 as above; `traits` 74,579, `slots` 10,850, `family_groups` 9,169, `triple_violations` 0) | **11 in `slots`** (SQLite happened to choose the narrowed row each time); 0 elsewhere | 0 | `20261022-211541-b5-final`, `traits`: `20261022-205531-b5-traits` |
| `quality-conditional-1` | 0 = 0 | 0 rows (`block_results` 2, `conditional_findings_0` 0) | — (no window) | — | `20261022-205542-b6-diag` |
| `state-flow-1` (per file, default `entryPoint`) | rule `serverplan_state_tripwires` 0 = 0 | 0 rows: `direct_sinks` 78 rows over 33 files, `transitive_sinks` 5 rows (one file's narrowed copy hit native's relational budget; the original and the narrowed clause itself completed) | 0 | not measured in this run | `20261022-210545-b6-trip-diag` |

Measured on `2b1504adc`, every narrowed order is free of ties between distinct rows. With these edits the corpus therefore yields **zero runtime refusals** on the engine. For `traits`, `base_edges`, `class_names`, `family_groups` and `conditional_findings_0`, this holds by construction: the order ends on the column the subquery projects, or it is total by the stage's grouping.

**Not yet run on the facts engine.** These numbers come from SQLite on native's path, not from the engine. The engine needs two more inputs before it can run them:

- S1's runtime-checked order mode (landed in `0ea181a84`, being merged).
- The per-file exports of these clauses. The leaf executor still lacks `precedes` (`raw_parameters`, `raw_parameter_annotations`) and `binds` (`raw_module_names`), which S6 is adding.

The engine replay of the narrowed clauses will be recorded in `reports/phase-2-s3-crossfile.md` when it runs.

**`scan_project_sources`: no oracle.** Under `entryPoint=scan_project_sources`, native itself cannot evaluate the repository variants on the 9,010-file corpus. `state-flow-1`, `state-flow-project-1` and `state-projection-recovery-2` all fail with native's relational work budget exceeded (`20261022-211324-b6-trip-orig-sps`, unmodified rules with the option's default switched). So no oracle exists for those variants at `2b1504adc`.

## 4. Not covered by a narrowing

`unreachable-internal-symbols/reach_edges` reads `reference_edges`, and `imperative-private-functions/caller` uses the `calls{graph:references}` atom. Both depend on native's whole-workspace reference graph, which is not order-dependent text. They wait on the call graph expressed as IR over S4's per-content summary exports (S3, `reports/phase-2-s3-crossfile.md`).
