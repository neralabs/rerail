# Rewriting the heavy relational inputs, and lowering them to the AST

Scripts and clause dumps: `../experiments/heavy-inputs/`.

Nothing here was implemented or run end to end. The before figures are measured. The after figures are estimates built from measured unit costs, and every clause was classified by reading its SQL.

### Headline

- **The "top ten ≈ 30%" figure undercounts the problem.** All 191 relational-clause inputs together take **146.6 s of the 171.5 s `sep` total, about 85%**. Six clause groups account for 139.6 s of that.
- **Almost none of that time is the rule's own logic.** It goes on three things:
  - copying the whole syntax tree into SQL rows;
  - running AST queries whose only SQL step is "one finding per line";
  - feeding string literals into SQL so it can compare them with a constant.
- **Rewritten as transducer programs, the six groups cost an estimated 0.40–1.29 s on one core** (same harness unit costs) instead of 139.6 s.
- **Exported rows fall from about 6.7M rows / 163M cells to essentially zero.** Only finding keys remain, plus one shared fact family (22,061 class-base rows).
- **Of the 52 relational clauses, 45 can run entirely as IR programs**, one of them reading a per-document fact column. Three more are IR plus a small cross-file SQL tail. Only 4 are genuinely cross-file.
- **This needs three IR extensions beyond E1–E14 (proposed E15–E17 below), plus one spec reversal.** Spec §7 and spec-fusion §6.6 currently say capture equality across matches stays in SQL. That rule should apply only across documents; within one document those joins should go to the IR.

### 1. What the heavy inputs are, and why they were written that way

Each relational input is an AST query whose captures become a SQL table. The heavy ones come in four shapes:

| Shape | Clauses | `sep` time | Rows exported |
|---|---|---|---|
| **C: syntax tree copied into rows.** Inputs named `syntax_*` or `raw_nodes` capture every node with up to 17 field captures. SQL then rebuilds nodes, edges, child order (via `row_number`), ownership (a recursive CTE) and a full descendant closure | quality-conditional-1, step-contracts-1..6, legacy-execution-graphs, executor-provenance | 64.7 s | 6.41M rows / 162.6M cells |
| **A: dedupe only.** No SQL stages; the AST input does all the matching and SQL only keeps one row per (path, line, scope, terminal) | 28 state-tripwire clauses | 54.9 s | 0 (no violations in this corpus) |
| **Literal feeders.** String values exported only so SQL can test `value IN (...)` | `raw_strings_*` in state-flow-1, state-flow-project-1, state-projection-recovery-2 | 20.0 s | 325,944 |
| **Fact tables in disguise.** Inputs that export every function, class, import or base | internal-dead-code, public-api-triples, required-contracts, authority-sites, runtime-reference-summaries, quality-contracts-1 | ≈5.9 s | ≈1.7M |

**Why the code looks like this.** All of it comes from the DLRE generators in `scripts/dsl/data_layer_six/`:

- **Exact oracle parity.** The port bar was "Model the exact state tracked by the oracle" (`docs/data-layer-rule-port.md`). The Python oracles walk the whole tree imperatively, so the generator reproduced that tree in SQL. That is category C in `authored-sql.md`.
- **Sharding to fit the arena.** `typed_input_partitions()` in `ast_relations.py` splits the every-node input into 20–27 typed shards:
  - identifiers by first letter (`^[a-f]`, `^[g-m]`, …);
  - strings by prefix;
  - one shard per kind group;
  - a "not any of the above" catch-all.
  The reason was to stay under the per-input witness bound and the 64 MiB producer arena. The batch-six table records it: "A large step file exceeded its witness bound | Split independent typed inputs".
- **quality-conditional-1 could not use the native syntax table.** The performance journal (around line 3210) explains why. Its selection includes module declarations, so the whole-file gate admitted every file and hit the 32 MB source guard. It therefore kept "its existing projected AST inputs and ordinary SQLite stages". That is why it alone visits all 28.2M nodes: `syntax_18` has an unconstrained root and takes 14.5 s by itself.
- **Category A exists only to reproduce the oracle's one-finding-per-line identity.**
- **The literal feeders and allowlists (category I) exist because the rules could not test a string value or an exception list in place.**

### 2. The heavy clauses, one by one

Before figures are the fusion harness's `sep` seconds and exported rows (`inputs.json`). After figures are root counts × κ, the measured per-candidate cost of ordinary forbid queries with that root kind (p50–p90: call 134–468 ns, function 0.9–5.4 µs, class 1–8.8 µs, module 8.6–17.5 µs). Set-stage passes are 6.4–17.3 ns/node. Script: `estimate_after.py`.

#### 2.1 quality-conditional-1 (serverplan-quality, whole repository): 29.34 s → 4–14 ms

**What the inputs do.**
- 20 inputs, each `<kind shard> AND NOT leaf-kind AND matches quality-conditional-1-selection`.
- That selection utility is a large `any` describing every node any later stage might read.
- Each shard exports the node plus parent, ancestor and up to 14 field captures as columns.
- In total: 2,093,271 rows, 69.2M cells.

**What the SQL does (32 stages).**
- Unions the shards (`syntax_rows_0/1`), then rebuilds `node_ordinals`, `nodes`, `edges`, `ownership` (recursive) and `descendants` (a closure over a demanded forest, up to ~16.7M rows).
- Builds `functions`, `classes`, `calls`, `attributes`, `returns`, `assignments`, `parameters`, `aliases`.
- Derives module names from paths with regex UDFs.
- `conditional_classes` keeps the latest class definition per name.
- `class_bases` resolves base names through import aliases.
- `derived_conditionals` is a recursive subclass closure of `ConditionalStep`.
- `function_colors` and `block_results` (a JSON fold computing "always raises TypeError"), `integer_returns`, `branch_variables`, `self_stores`, `callable_parameters`.
- Finally 17 `SPQ-029` detectors.

**What it actually checks.**
- **13 detectors** look at 4 named classes and functions in 4 known files. Together those files hold 76,417 nodes, 77 classes and 411 functions. Examples:
  - `ConditionalStep.compile` must raise `TypeError`;
  - `level` must return an integer > 0;
  - `SqlConditionalStep.execution_role` must return `StepExecutionRole.SQL`;
  - `_lower_conditional_step` must call `SqlConditionalStep.from_semantic`.
- **4 "missing" detectors** fire when a file lacks its class or function.
- **5 detectors** apply to subclasses of `ConditionalStep` anywhere: runtime-state, runtime-callback, declared-callback, child-compilation, backend-control. The corpus has **3** such classes (closure measured on `facts.sqlite`).

**Rewrite:**

| Part | Lowering |
|---|---|
| (a) 13 anchored detectors | One program each, gated by `filePath` and rooted at `class_definition`/`function_definition` with a name test: `NOT_EXISTS` method `compile` with `EXISTS raise TypeError`; `COUNT` of integer returns in `level` ≠ 1 (E4); `EXISTS` return text `StepExecutionRole.SQL`; `EXISTS` call matching `compile_sql_conditional_step$` |
| (a) "always raises" | A bottom-up attribute over blocks, with negation: a block raises if a statement before the first return/raise is `raise TypeError…`, or is an `if` all of whose branches raise. Needs **E17** |
| (a) child-compilation | `$X.compile()` where `$X` was earlier bound in the same function from `then_plan.steps…` or `for $X in …`: `EXISTS` from the enclosing function with `TEXT_EQ $X` (allowed by spec-fusion §2.3, since sub-runs read outer captures) plus "earlier than" (**E16**) |
| (a) runtime-callback | `self.x = p` where `p` is a `Callable`-annotated parameter of `__init__`: `TEXT_EQ` |
| (a) compiler-contract | The annotation resolved through an in-file import alias: `EXISTS` import in the module with `TEXT_EQ` alias |
| (b) | `fact_class_base(doc, class, base_name)`: 22,061 rows, shared with internal-dead-code and public-api-triples |
| (c) | A recursive closure from `ConditionalStep` gives 3 names, fed back as a seeded key set (E7 → **E15** `IN_KEYS`) to the 5 subclass detectors, so 32,728 classes each get one cheap key test |
| (c) "missing" guard | `require_each` over 4 `t_match` rows |
| Dropped | "latest definition wins" and module-name regexes; replaced by `filePath` |

**After:** 4–14 ms. Rule-specific exports are 3 names plus findings.

#### 2.2 step-contracts-1..6 (serverplan-steps, 264 files, 2.34M nodes): 26.08 s → 0.08–0.27 s for all six

**What the inputs do.**
- Each clause has one `raw_nodes` input: every node inside a strict module that matches a large `any` demand filter.
- 33–45 columns each; 551,463 rows / 21.3M cells over the six.
- step-contracts-6 exports only 563 rows but costs 8.6 s, because the demand filter, including `inside step-strict-module stopBy end`, is evaluated on all 2.34M nodes.

**What the SQL does.**
- Rebuilds the tree, including `descendants` as a full `WITH RECURSIVE` walk (about 4.4M rows over the six).
- Then: `step_classes` (base text matches `ServerPlanStep`; 268 such classes in 99 files), `first_methods` (first definition per name wins), `method_returns`, `levels`, `roles`, `empty_tables` (`frozenset()` with no arguments), `compiled` (keyword flags on `CompiledStep(...)` calls), `compile_aliases`, `local_bindings`.
- Recursive stages:
  - `column_derived`: flow from `cte_output_columns` through local names;
  - `user_callable`: lambda or callable-named identifiers propagated up attribute, call and argument edges;
  - `raw_shapers`: a function counts as a shaper if it iterates a raw parameter or calls a shaper; an intra-file call graph by name;
  - `payload_values`.
- Findings: STEP-016, 018, 019, 023–026, 028, 033–036, 039, 040, 045–050.

**Rewrite: all (a).** Every check is "class with a ServerPlanStep base, method named N, inspect its body". For example:

```yaml
# STEP-018: raise TypeError/NotImplementedError directly in compile() of a level-0 step
rule:
  kind: raise_statement
  has: {regex: '^(TypeError|NotImplementedError)\b'}
  inside: {kind: block, stopBy: neighbor, field: body,
    inside: {kind: function_definition, has: {field: name, regex: '^compile$'},
      inside: {stopBy: end, matches: level0-step-class}}}
utils:
  step-class: {kind: class_definition, has: {field: superclasses, has: {regex: '^(?:\w+\.)*ServerPlanStep(\[.*\])?$'}}}
  level0-step-class: {all: [{matches: step-class},
    {has: {stopBy: end, all: [{kind: function_definition, has: {field: name, regex: '^level$'}},
                              {has: {stopBy: end, kind: return_statement, has: {kind: integer, regex: '^0$'}}}]}}]}
```

- Joins between captures within a file use `TEXT_EQ` inside `EXISTS` when one side is reached by navigation, and **E15** keyed sets when both sides are free. Examples: keyword `post_op=<name>` against a function with that name (STEP-026/028), `self._x` alias names (STEP-019), materializer class name against an identifier (STEP-039).
- Recursion through names (`raw_shapers`, `column_derived`) becomes a monotone FIX over an E15 key set.
- Recursion along fields (`user_callable`, `payload_values`) is an ordinary E14 FIX in the has-direction.
- "First definition wins" is dropped under the loose findings contract, or kept as `NOT_EXISTS` an earlier sibling definition with `TEXT_EQ` + E16.

**After:** 0 exports; `t_match` = findings. Cost: 2,386 class roots, 94k call roots, 16k function roots, plus three set passes over 2.34M nodes: 0.08–0.27 s for all six. This matches heavy-rule-profiles' independent 0.08 s estimate for AST matching on this rule.

#### 2.3 legacy-execution-graphs (702 files, 4.31M nodes): 7.99 s → 0.10–0.37 s

**What the inputs do.**
- 25 typed syntax shards (`syntax_24`, the catch-all, alone: 3.45 s, 1.64M rows), plus `from_imports` and `direct_imports`.
- 3.75M rows / 71.4M cells.

**What the SQL does.**
- `function_symbols`: qualname by interval containment plus `group_concat`.
- `decorators`, `valid_boundary`.
- `receiver_tokens`: recursive; lowercased identifier tokens propagated up attribute, call, `cast` and subscript edges.
- `call_candidates` → `resolved_calls` (`HAVING count(*)=1`) → `eager_reach`: recursive callers of functions that call `resolve`.
- `eager_anchors`: first call per function.
- 22 detectors, LEG-010 … LEG-085, mostly "call to X under path prefix P, minus path allowlist".

**Rewrite: all (a).**
- Path tests and path allowlists become `filePath` (category I literals become constant tests).
- `receiver_tokens` becomes an E14 FIX utility, "receiver chain has identifier token in T".
- `eager_reach` becomes an E15 keyed FIX over function names within the document. Dropping the unique-name requirement is acceptable under the loose contract; otherwise use `COUNT` on the key.
- LEG-085 (callee equals an earlier raw import alias): `TEXT_EQ` + E16.
- LEG-021 `valid_boundary` derives a `X_lazy` sibling name from `X_async`: needs a capture transform (E5 `transform`) then `TEXT_EQ`.
- The qualname allowlist (three entries) compiles to `inside` chains.

**After:** 0 exports, 0.10–0.37 s; the two set passes over 4.3M nodes dominate.

#### 2.4 The 28 dedupe-only state-tripwire clauses: 54.94 s → 0.08–0.25 s

**What they do.** Each is a single `events` input:

```
inside state-project-source (stopBy: end) AND filePath … AND <call/attribute shape>
```

where `state-project-source` is a module-wide regex. The SQL final is only `row_number() OVER (PARTITION BY path,line,scope,terminal)=1`. No rows are exported on this corpus. The 2–3 s per clause is all matching: in `sep`, every one of 412k calls walks to its module and re-runs the module regex. That is the 2.59 TB rescan in fusion-model §3.

**Rewrite: plain `forbid` with `key: [line, scope, terminal]` (a).** The new part is a lowering rule, not an opcode:

- **L1, document-invariant hoisting.** `filePath` conjuncts, and `inside {module + regex} stopBy end`, depend only on the document. They are evaluated once per file as a gate.
- Option residue such as `filePath: '\b\B'` (never matches) and `'(?s).*'` (always matches) is constant-folded.

**After:** 3,095 module-gate evaluations once, plus one fused call dispatch: 0.08–0.25 s. Memo-mode fusion already recovers much of this, so this group's saving overlaps with fusion.

#### 2.5 String feeders in state-flow-1, state-flow-project-1 and state-projection-recovery-2: 20.01 s → about 0.05 s

**What they do.**
- `raw_strings_0/1/2`: non-interpolated strings in the project-source scope, decoded through a `transform`, sharded by prefix. 325,944 rows.
- The only consumer, `string_values`, uses them to test `s.value IN ('_conn', 'materialized_pipeline', …)` on `getattr` / `.get` / subscript keys.
- It also folds module-level constants: an identifier whose name matches a module assignment of such a string.

**Rewrite (a).**
- The key test moves into the `getattr`/`get`/subscript sub-program as a value DFA on the decoded literal.
- Constant folding becomes `EXISTS` from the module with an assignment whose left side matches `TEXT_EQ $NAME` and whose right side is a matching literal.
- 0 rows exported.

**What remains.** The category-H core of state-flow (`taints`, `semantic_values`, `call_graph`, `reachable`, `transitive_sinks`) still has to be handled. Section 3 shows it fits E15 + FIX within a file.

#### 2.6 executor-provenance (450 files): 1.26 s → 0.09–0.32 s

- 25 typed shards; 24,598 rows / 700k cells.
- SQL computes executor types (recursive over annotations), name bindings, and owners (recursive over attribute, subscript and binding edges).
- **Rewrite (a):** an E14 FIX over field edges plus an E15 keyed FIX over name bindings scoped to functions. 0 exports.

#### 2.7 Smaller clauses with large exports

| Clause | Before | Rewrite |
|---|---|---|
| internal-dead-code (13 inputs, 554k rows, 1.82 s) | Fact tables in disguise; 25 stages including import resolution, hierarchy, reachability | **(b)+(c).** `fact_def`, `fact_import`, `fact_ref`, `fact_class_base` (194k / 227k / 1.18M / 22k rows, shared). SQL reachability is 0.80 s on a proper edge table (heavy-rule-profiles) |
| public-api-triples `exported-families` (11 inputs, 551k rows, 1.15 s) | Same facts plus traits and `__all__` | **(b)+(c).** Per-symbol traits (decorators, suffixes) are IR exports; family grouping across modules via the re-export closure is SQL |
| required-contracts (10 inputs, 103k rows, 0.89 s) | Exports every symbol, member and annotation, then anti-joins JSON lists of required (path, name) | **(a)+(c).** One `NOT_EXISTS` program per requirement, gated by `filePath`; only a missing file needs a 1-row SQL existence check |
| authority-sites (5 inputs, 166k rows, 0.89 s) | Exports every function and class to compute qualnames for three allowlists | **(a).** AUTH001–003 as `forbid`; allowlist qualnames compiled to `not inside` chains, or E8 SIDE qualname |
| runtime-reference-summaries (10 inputs, 266k rows, 0.40 s) | Edges, returns, bindings; recursive `consumers` / `refless` | **(a).** E15 scoped keyed FIX |
| quality-contracts-1 (7 inputs + native syntax, 0.78 s, 54 stages) | Scope chains, scope imports with shadowing, annotation resolution | **(a)+F.** IR programs that read a per-document `fact_scope`/`fact_binding` column (E8 `SIDE_EQ`) |

### 3. IR extensions needed

**Already in E1–E14 and used here:**
- E1 set mode;
- E2 `EXISTS`/`NOT_EXISTS`, with sub-runs reading outer captures, which is what makes navigational joins possible;
- E3 axes;
- E4 `COUNT` (taxonomy, `level` returns, ordered-occurrence limits);
- E5 patterns and transforms (the `_async`→`_lazy` name transform);
- E6 utilities;
- E7 seeded starts and keys from SQL (the derived-conditional names);
- E8 `SIDE` (quality-contracts-1; optional qualname for allowlists);
- E12 lazy enumeration;
- E14 set stage with FIX (`receiver_tokens`, `user_callable`, `payload_values`);
- `TEXT_EQ` (spec-fusion §8).

**New (proposed):**

| # | Extension | Needed by |
|---|---|---|
| **E15** | **Keyed sets.** A set-stage definition `KEYS(S, key = text(x) or a capture, scope = document or nearest ancestor of kind K)` producing per-document sorted (scope, hash) pairs, plus `IN_KEYS k [negate]`. Admissible inside a monotone FIX. Metal: sort/dedupe plus binary search, the same machinery as postings | Every within-document capture join where neither side reaches the other by navigation; recursion through names (`raw_shapers`, `eager_reach`, `bad_configs`, executor owners, local value flow, state-flow taint); SQL-seeded name sets (E7) |
| **E16** | **Order against a capture.** `ORDER cmp, capture` compares preorder (or byte) position with a captured node | "Assigned before use", "earliest anchor", `strategy.line <= call.line`, alias imported earlier |
| **E17** | **Bottom-up fold.** `FOLD_UP` set: `S = {x : φ(x, S ∩ children(x))}`. φ may negate, because recursion is strictly into the subtree and therefore well-founded. One reverse-preorder pass on CPU; deepest-first depth waves on Metal, as for has-direction FIX. Requires amending spec §6.8 / spec-fusion §6.8, which currently reject any negation through recursion | quality-conditional-1 "always raises"; similar "all branches" properties |

**Lowering rules (no new opcodes):**
- **L1:** hoist document-invariant conjuncts to a per-document gate, and constant-fold option residue.
- **L2:** drop `row_number` pick-first and "latest definition" selections (findings contract).
- **L3:** compile allowlist literals into constant tests or `inside` chains.

**Spec changes.** Spec §7's "Capture equality across matches | stays in SQL" and spec-fusion §6.6 should become: within one document, lower to `EXISTS`+`TEXT_EQ` when one side is reached by navigation, otherwise to E15. SQL hash joins remain for cross-document joins only.

**Risks.**
- **Quadratic sub-runs.** A per-candidate `EXISTS` that scans a whole function body can go quadratic. The planner must choose the E15 set form by cost (spec-fusion §4.8).
- **Category H stays a parity risk** (state-flow, runtime-reference-summaries, executor-provenance), even though it now fits the IR as a monotone keyed FIX.

### 4. All 52 relational clauses

| Target | Clauses | Input `sep` time | Which |
|---|---|---|---|
| **(a) fully in the IR** | **44** | 95.7 s | The 28 dedupe clauses, step-contracts-1..6, legacy-execution-graphs, executor-provenance, authority-sites, direct-execution-lines, quality-contracts-2 and -3, quality-local-taxonomy, state-signatures-1, state-projection-recovery-2, runtime-reference-summaries |
| (a) plus a per-document fact column | 1 | 0.8 s | quality-contracts-1 |
| (a) plus a small cross-file SQL tail | 3 | 38.0 s | quality-conditional-1 (class-base closure + guard), required-contracts (missing files), state-flow-1 (cross-file return-kind summaries; this rule currently bakes them in as the `returnKindsHex` option) |
| (b)+(c) genuinely cross-file | 4 | 12.1 s | internal-dead-code, exported-families, state-flow-project-1, quality-project-taxonomy (a repository count ≥ 2, i.e. a `budget`) |

So **48 of 52 clauses have all their per-document work in the IR**, against the fusion model's 11 `EXISTS` + 29 "join" split. The 7 that use the native syntax table and all 29 per-file joins lower to the IR once E15 exists. The SQL that remains is fact-family joins, two cross-file closures, a re-export closure, one repository count, and three small tails.

### Caveats

- The before figures are single-core `sep` runs in a harness that treats fact keys as true; identical runs varied up to 2.2×.
- κ comes from ordinary forbid queries, and root counts are upper bounds over the whole rule scope.
- For the 28 dedupe clauses, much of the saving overlaps with memo-mode fusion.
- I read quality-contracts-1/2/3, state-signatures-1 and runtime-reference-summaries less closely than the heavy ones.
- No findings were compared; this is a design analysis.

Everything is in ``../experiments/heavy-inputs/``; all scripts run with `python3 -I`, except `brief.sh`, which is a bash wrapper:
- `map_inputs.py` maps per-query timings to named inputs and writes `inputs.json`.
- `dump_clause.py`, `specific_stages.py` and `brief.sh` dump a clause's inputs and its rule-specific SQL stages.
- `clauses.py` writes the 52-clause census to `clauses.txt`.
- `scope_counts.py` gives kind counts per rule scope.
- `estimate_after.py` produces the before/after table.
- `classify.py` holds the per-clause classification.
- The raw clause dumps are `qc1.txt`, `sc1.txt`–`sc6.txt`, `leg.txt` and `sf1.txt`.