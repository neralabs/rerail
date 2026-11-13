# T17: the 60 data-layer rules in Rerail logic — representability

Workstream C, branch `jeremybarnes/final-rerail-shape-dl-census` from `35c010235`. The question is 03 §12's: what of the
60 data-layer rules (`gate/data-layer/packs/data-layer/candidates/rules/`, deferred to Phase 11) Rerail logic can say,
what needs a named §12 extension, what needs facts that do not exist, and what is outside the fragment.

Nothing here runs a rule. Every count comes from one of two checked sources:
- **The census** (`gate/data-layer/census/*.tsv`). `logic::front::data_layer_census_tests` derives it from the rule
  files on every test run and fails if a file drifts. `recursion.tsv` is classified by hand, and the test checks that
  its keys are exactly the recursions the rules contain.
- **The programs** (`cr/logic/programs/data_layer/*.rr`). These are the eight recursive rules written in Rerail
  logic. `logic::front::data_layer_tests` parses and stratifies each one after `prelude.rr` (front end only). It also
  checks that every relation used in a rule or a fact is declared at its arity, and that every matcher atom holds JSON.

`[INFERENCE]` marks reasoning that no run or file shows.

## Verdicts

| Question | Answer | Numbers (source) |
|---|---|---|
| Representable on today's built Rerail logic | **No rule.** 51 rules need the matcher atom, which is proposed and has no executor (T14). The other 9 consist of text-regex clauses, which have no logic form. | `rules.tsv` |
| Representable once the matcher atom runs | **19 rules** need nothing else. **13 more** also need semantic labels (O24). | `rules.tsv` "blocked by" |
| The eight recursive rules | **All written and front-end checked.** 416 SQL stages across 21 programs. **2 stages are outside** (`call_paths`, `block_results`). Two exhibits fail with their named errors: `NotStratifiable` and `UnboundedLattice`. | programs; `data_layer_programs_parse` |
| Lattices (the open "general form" point) | **A closed catalogue is enough.** 70 recursions: set 48, best-witness 12, min-level 1, none 9. Of the 12 best-witnesses, 8 are tree ancestors. 4 are genuine, and all 4 are the same lattice: a shortlex-least route. There is no user lattice and no type join. | `recursion.tsv`; E0 § 2 |
| Smallest set that makes all 60 representable | Ten items, § 7. Five are §12 extensions: the matcher atom's executor, its captures, a builtin catalogue, tree built-ins, one best-witness lattice. Two are facts: O24 labels; `module_of`, reference edges and `calls`. Three are decisions: the text-clause rewrite, LIMIT without ORDER BY, the two outside stages. | § 7 |

## 1. Method

**Unit.** Each clause of each rule: 606 clauses in all.

| Kind | Clauses | Of which |
|---|---|---|
| forbid | 496 | 383 matcher, 109 regex over file text, 4 regex over the path |
| define | 44 | |
| relational | 52 | |
| budget | 14 | |

**What a clause's constructs are:**
- the terms of its matchers (rule objects, utils, the `transform` and `file` filters);
- its evidence projections;
- its relational declarations (`partitionBy`, `syntax`, `references`, `deferred`, `key`);
- the SQL of every stage and of its final query.

SQL is read at each option's default; another option's case is never counted. A recursion is a recursive CTE (one whose body names itself), a `reachability` stage, or an evidence `closure` with edges.

**Categories:**

| Category | Meaning |
|---|---|
| `today` | Built in Rerail logic now: joins, unions, stratified negation, `count`/`min`/`max`, `@cat`, `nearest`, set recursion, per-document partition. |
| `findings` | Non-Datalog SQL in a clause's final query, the violation templates, `key`: the findings layer (03 §8). |
| `ext:matcher-atom` | §12 matcher atom (proposed; no executor). |
| `ext:matcher-captures` | The atom exporting captures. Added to the front end on this branch; no executor. |
| `ext:builtins` | §12 builtins beyond the built `@cat` and `@flat_bin`. |
| `ext:lattice` | §12 lattice general form. |
| `ext:tree` | Tree built-ins beyond `nearest`. |
| `facts:semantic` | O24 labels. |
| `facts:crossfile` | Fact families no producer makes. |
| `outside:*` | Outside the fragment, each with what it is. |

**Inside a stage vs the final query.** A construct that Datalog lacks belongs to the logic when it is inside a stage, and to the findings layer when it is in the final query.

**Limits of the mechanical classification:**
- SQL is classified by keyword after string literals are blanked.
- `sql:arith` is an operand–operator–operand scan.
- `LIMIT` counts as ordered (an argmin) only when an `ORDER BY` precedes it in the same `SELECT`. `rx_matches(...)` in a SQL stage is a builtin, though most of its uses are allowlists that facts would replace.
- The programs (§ 3) are the finer reading. Where the two differ, the programs say which form was used.

## 2. The census

**By category** (`constructs.tsv`, `clauses.tsv`, `rules.tsv`):

| Category | Rules | Clauses | Occurrences | Main constructs |
|---|---|---|---|---|
| `ext:matcher-atom` | 51 | 478 | 51,133 | `kind` 9,605, `all` 8,751, `stopBy` 7,187, `has` 5,370, `field` 4,285, `regex` 3,579; `filePath` 1,113 |
| `findings` | 60 | 562 | 922 | 562 violation templates; the final queries' string building |
| `ext:builtins` | 19 | 134 | 1,655 | `||` 778, `rx_matches` 290, `substr` 77, line projections 112, path globs 76 |
| `outside:text` | 14 | 109 | 293 | regex over file text, line options 108, line state machine 8 |
| `ext:matcher-captures` | 14 | 54 | 1,672 | capture projections 1,660, `each` 12 |
| `facts:semantic` | 23 | 43 | 308 | matcher `value` 292 (20 rules), `binds` 10, `arguments` 6 |
| `ext:tree` | 6 | 17 | 38 | subtree walks 16 (`@within`), ownership 8, path folds 3 (`@qualname`, `@class_path`), `scope_chain` 1, `syntax` 7 |
| `ext:lattice` | 3 | 4 | 4 | the four route best-witnesses |
| `outside:collection` | 3 | 3 | 7 | `group_concat`, `json_group_array`, `json_array`, `json_quote` in a stage |
| `outside:choice` | 3 | 3 | 8 | `LIMIT` with no `ORDER BY` in a stage (8 more are ordered: `today`) |
| `facts:crossfile` | 2 | 2 | 4 | `references`, `reference_edges`, `file.module`, matcher `calls` |
| `outside:value-cycle` | 1 | 1 | 1 | `call_paths` |
| `outside:not-stratifiable` | 1 | 1 | 1 | `block_results` |
| `today` | 18 | — | 3,551 | joins 1,256 + 259, unions 464, negation 216 + 78, `row_number() = 1` 63 + 30, `count`/`min`/`max` 39 |

**By rule: what blocks each one** (`rules.tsv`, last column):

| Blocked by | Rules | Which |
|---|---|---|
| `ext:matcher-atom` | 19 | async-sync-boundary, backend-coupling, classvar-check, data-layer-compat-aliases, error-handling, forwardref, grammar-str-params, lazy-result-usage, missing-async-pairs, module-level-caches, permission-error, plan-scope-side-effects, raw-execute-regression, removed-compat-surfaces, render-fstrings, serverplan-grammar-surface, swallowed-exceptions, transaction-closures, versioned-lifecycle-legacy-surfaces |
| `ext:matcher-atom`, `facts:semantic` | 13 | async-patterns, conn-or-txn-union, data-layer-facade-purity, grammar-allowlist-burndown, grammar-render-methods, no-api-root-imports, plan-scope-ownership, regex-sql-parsing, render-raw-strings, sync-async-signatures, transaction-callback-wrappers, transaction-patterns, unresolved-forward-refs |
| `outside:text` (+ `ext:builtins` for path globs) | 9 | admin-async-purity, async-executor, dynamic-module-loading, hasattr-async, no-source-inspection, rust-sql-grammar-boundary; backend-type-strings, connection-caches, serverplan-name-inference |
| the matcher atom and one or more of captures, builtins, semantic labels, cross-file facts, text clauses | 11 | async-sync-delegation, grammar-callable-constructors, grammar-fragment-usage, imperative-private-functions, impl-direct-execution, layer-violations, no-blanket-grammar-bypass, serverplan-bypass, serverplan-target-architecture, sql-grammar-escape-hatches, type-safety-patterns |
| the eight recursive rules | 8 | § 4 |

Every rule also uses `findings` (its violation template). The per-clause reading is in `clauses.tsv`: one row per clause, with its non-`today` categories and every construct with its count.

## 3. Which §12 extensions, how often

**Matcher atom** (`ast($D, $P, {…})`, proposed): 51 rules, 478 clauses.
- `yaml::add` writes a plain forbid clause as this atom.
- **What `yaml::add` refuses.** A forbid wrapped in `$option` (one clause) and the 113 regex forbids (§ 7 item 8).
- **Options inside matchers.** Matchers carry `$option` in 93 clauses of 8 rules (271 occurrences). These are compile-time choices; the programs either keep them verbatim or write the default case.
- **Execution path.** T14 found the atom has no executor, and rules reach execution through W's `compile/*`.

**Matcher captures:** 14 rules, 54 clauses, 1,672 projections. §12's atom binds the document and the node, but relational inputs and budget evidence read `capture.X` text, `capture.X.startByte`, regex groups and `transform` text.
- **Front-end form** (this branch, `5b80a703d`): `ast($D, $N, {…}, NAME: $v)`.
  - A match that leaves `NAME` unbound gives no row.
  - An optional capture (`any: [X, {pattern: $_}]`) is read by a second atom whose matcher is X alone, or by exact `field`/`child`/`nearest` relations.
- **In the programs:** 71 exported captures.
- **No executor.** The atom still has none; `plan.rs` refuses `Literal::Ast`.

**Builtins:** 19 rules, 134 clauses in the census. In the programs, 30 builtin names appear outside comments; only `@cat` (17 uses) is built.

| Kind | Builtins (uses) | Notes |
|---|---|---|
| SQL scalar | `@rx_matches` 224, `@concat` 66, `@substr` 60, `@instr` 37, `@lower` 27, `@rx_extract` 25, `@length` 21, `@trim` 20, `@sub` 16, `@rx_replace` 14, `@printf` 11, `@ltrim` 8, `@like` 5, `@cast_int` 5, `@replace` 4, `@add` 3, `@upper` 1, `@char` 1 | `@rx_matches` is mostly allowlists over names and paths that facts could replace |
| new, over strings | `@drop_components` 4, `@join_nonempty` 4, `@glob` 2 | |
| tape | `@id` 118, `@within` 64, `@depth` 9, `@line` 8, `@qualname` 1, `@class_path` 2, `@class_prefix` 1, `@substr_bytes` 1 | `@id` 118 is identity text for findings |

- **Filters.** The programs write filters as `@f(…) = c`. The grammar accepts this; §12 writes `$v = @f(…)`.
- **Value-creating builtins in cycles.** Three recursions keep one:
  - `hierarchy` and the two sinks build routes with `@concat` (bounded by `widen`);
  - `call_paths` builds path text (declared outside).

**Lattices:** 3 rules, 4 recursions.
- **Which ones:** `hierarchy` in internal-dead-code and public-api-triples (shortest base path, ties broken by base order), and `transitive_sinks` and `project_sinks` (shortest call route).
- **Form in the programs:** `.decl r(keys; route: sym) merge @shortlex_min widen 64`.
- **Not executable as written.** The native lattices are `i64`-valued `Flat`/`Min`/`Max` (`logic/native.rs:55–103`). A (length, text) witness has no implementation.
- **Widening.** `Min`'s widening jumps to its top once a value has changed `k` times, so a widened route loses its witness [INFERENCE from `native.rs:96–103`].

**Tree built-ins:** 6 rules.
- **In the census:** 28 recursions are `ext:tree`: 16 subtree walks (`descendants`, the `reachability` stages over syntax, `mutable_values`), 8 `ownership`, 3 path folds, `scope_chain`.
- **Covered today.** `nearest` covers `scoped_nodes` and the nearest function and class of every `ownership`.
- **What `@within` covers.** Descendants and the outermost function (`top_fn`). The programs write it as a builtin, `@within($x, $r) = 1` (64 uses), and depth as `@depth` (9).
- **Why not a literal.** A new `Literal` variant would need `logic/plan*` changes (`plan.rs:861` matches literals exhaustively), so the function form keeps the front end alone.
- **Filtered walks.** Two remain Datalog recursion over tree edges: dispatch `annotations` (up through type nodes) and steps `payload_values` (down through containers). A "climb or descend while the kind is in a set" built-in (the matcher's `inside`/`has` with `stopBy`) would cover both.
- **`mutable_values`** (up through tuple parents) became a depth count.
- **`syntax` declarations** (7 clauses): the tape relations plus `nearest`/`@within`.

**Aggregates.**
- **Usage.** The programs use `count` 50, `min` 34 and `max` 33 times. No rule uses `sum` or `avg` in a stage.
- **`row_number()`.** Its 93 windows (63 in stages, 30 in final queries) are first-row picks; the programs write them as min/max chains.
- **One atom per aggregate.** An aggregate reads one atom, so each window needs a helper relation.
- **Ties.** Where order keys tie on distinct nodes, the programs keep every tied row and say so. Most ties cannot occur, since siblings never share a start.

**Stratified negation:** today (`NOT EXISTS`/`NOT IN`/`LEFT JOIN … IS NULL`: 216 in stages).

**Demand, three-valued results:** no program needs them as written.

**Per-document partition:** 46 relational clauses are `partitionBy: file` at the defaults, and 6 are repository-wide. The repository-wide ones join across documents on computed names (`@cat`, `@concat`). §12 allows cross-document joins only on interned keys. `@cat` results are interned when atoms are on (`plan.rs:808`, `sym_cat`); `@concat` results would have to be [INFERENCE].

## 4. The eight recursive rules: what blocks each

| Rule | Programs | Stages written / outside | Needs |
|---|---|---|---|
| internal-dead-code | 1 (+ 1 exhibit) | 25 / 0 | matcher atom and captures, builtins, best-witness `hierarchy`, `binds` (O24), `module_of` and `reference_edge`, LIMIT choice (`base_edges`) |
| execution-authority-bypass | 1 | 7 / 0 | matcher atom and captures, builtins, `@qualname` (tree), `value` (O24) |
| public-api-triples | 1 | 23 / 0 | matcher atom and captures, builtins, best-witness `hierarchy`, LIMIT choice, `json_group_array` (rewritten as rows) |
| legacy-execution-surfaces | 1 | 23 / 0 | matcher atom and captures, builtins, `@within`, `group_concat` (rewritten as `@class_prefix`), 7 text/path-regex forbids |
| serverplan-dispatch-architecture | 1 | 22 / 0 | matcher atom and captures, builtins, `@within`, `value` (O24) |
| serverplan-quality | 3 (+ 1 exhibit) | 106 / 2 | as above, plus `call_paths` (value cycle) and `block_results` (tree form `NotStratifiable`), a text forbid, the `parse` sub-grammar nodes, `file_content` |
| serverplan-state-tripwires | 5 | 78 / 0 | matcher atom and captures, builtins, `@within`, best-witness sinks, `value` (O24) |
| serverplan-steps | 8 | 132 / 0 | matcher atom and captures, builtins, `@within`, `value` (O24) |

**How the programs are written.**
- One relation per SQL stage, and matchers verbatim.
- A SQL row keyed by (`__path`, `__start`) is a node. A nullable column is a side relation, and `CASE`/`COALESCE` are split into ordered rules.
- Each program's header lists its options, builtins, and its OUTSIDE, findings-layer, missing-fact, LIMIT and tie decisions.

**What disappears over the tape.** E0's string recursions in internal-dead-code and public-api-triples go away:
- `tokens` is the statement's `name` children;
- `relative` is `@drop_components`;
- `modules` is `@cat`'s split, which gives every dotted prefix.

**The two outside stages are declared as inputs**, so the stages after them still parse:
- **`call_paths`** (E0 row 20) carries dotted text along attribute chains with `path || suffix`. This is §12's "cycle through value-creating builtins". The front end has no check for it (`front.rs:13–15`). A tape builtin like `@qualname` would replace it [INFERENCE].
- **`block_results`** (E0 row 27) folds a JSON map in block order. Its tree form, in `serverplan_quality.block_results.rdl`, says a block raises when every block of its if/match raises. That is a negation of `raises` in its own component, so the front end reports `NotStratifiable`. It is stratified by tree depth, which §12's check cannot see; a bottom-up tree fold, or a per-file lattice inside the dispatch (03 §7), would carry it.

**Facts no producer makes, by program:**
- `module_of` and `reference_edge` (internal-dead-code, in `prelude.rr`);
- `file_content` (serverplan-quality's line-count forbid);
- the `parse` sub-grammar nodes (quality-contracts-1's f-string and annotation re-parses).

## 5. Facts that do not exist

**Semantic predicates (O24):** 23 rules, 43 clauses: matcher `value` (292 occurrences, 20 rules), `binds` (2 rules) and `arguments` (2 rules). They stay inside the verbatim matchers. The atom's lowering needs them as labels from fact producers (decision 0011).

**Cross-file:**

| Fact | Used by | W's source |
|---|---|---|
| `calls` | imperative-private-functions | matcher term |
| reference edges | internal-dead-code | `references` declaration plus `reference_edges`; W's native linker |
| `module_of` | internal-dead-code | `file.module`, which needs the package roots |

The other repository-wide clauses compute modules from paths, so they need no such fact. facts-a refuses internal-dead-code at compile for `file.module` (E0 § 2.4).

## 6. The lattice catalogue (03 §12 "general form proposed")

**Evidence:**
- **E0** (facts-engine `lattice-e0-census.md` § 2): 69 recursive CTEs over every option case. Set 43, best_witness 16, min_level 1, none 9; 0 non-monotone; no user lattice and no type join (§ 2.3).
- **This census, at the default options:** 70 recursions, including 8 reachability stages and 3 budget closures.

| Lattice | Recursions | Where they go |
|---|---|---|
| set | 48 | 28 set recursions over graphs, 16 `@within`, 2 filtered tree walks, 1 `nearest`, 1 `@cat` split |
| best-witness | 12 | 8 are `ownership`'s nearest or outermost ancestor: tree built-ins, no lattice. 4 are routes (§ 3). |
| min-level | 1 | `scope_chain`, a depth count |
| none | 9 | 7 become builtins or tape forms (`@qualname`, `@class_path` ×2, `@drop_components` ×2, `name` children ×2); 1 value cycle; 1 `NotStratifiable` |

**Answer.** The data-layer needs:
- set recursion;
- the built `i64` lattices (`flat`, `min`, `max`);
- **one value lattice**: the least route under shortlex (length, then text), over a finite graph.

The four route lattices are the same lattice. The general `merge @join` form adds nothing these 60 rules use.

**What is left to decide is how that one lattice is bounded.**
- **The problem.** Its height is bounded by the graph's simple paths, which a declaration cannot state. Without `widen` the front end rejects it (`internal_dead_code.hierarchy_unwidened.rdl`, `UnboundedLattice`); with `widen` it can lose the witness (§ 3).
- **Option 1.** A catalogue lattice whose bound comes from the edge relation.
- **Option 2.** Split the work: a `min` lattice for the length in logic, and the route in the findings layer. `hierarchy` reads the route only to break ties between methods at one depth, and the sinks to pick one witness per kind [INFERENCE].

## 7. The smallest set that makes the 60 rules representable

| # | Item | Rules it unblocks or touches | Kind |
|---|---|---|---|
| 1 | Matcher atom executor (§3 lowering of `ast(…)`; not W's `compile/*`) | 51 (19 need nothing else) | §12 extension |
| 2 | Matcher captures, `ast(…, NAME: $v)`: front end done here | 14 | §12 extension |
| 3 | O24 labels for `value`, `binds`, `arguments` | 23 (13 then complete) | facts |
| 4 | A builtin catalogue: ~20 SQL-like scalars and the tape builtins of § 3 | 19 in the census; every recursive rule | §12 extension |
| 5 | Tree built-ins `within`, `depth`, outermost, and a kind-filtered climb/descent | 6 | §12 extension |
| 6 | One best-witness lattice (shortest route), bounded by the graph, or the route moved to §8 | 3 | §12 extension / decision |
| 7 | `module_of`, reference edges, `calls` | 2 | facts |
| 8 | Rewrite the 113 text/path-regex clauses as matcher atoms: PIAT §5.1 (`t1` :446, `t2` :588) found 107 AST-expressible, 2 needing line positions (`@line` here), 4 path-only, 0 genuinely textual | 14 | decision (adopted in PIAT, not done) |
| 9 | Semantics of `LIMIT` with no `ORDER BY` inside a stage: keep every candidate (the programs do) | 3 | decision |
| 10 | The two outside stages: a tape builtin for attribute-chain text (`call_paths`), and a bottom-up tree fold or depth stratification (`block_results`) | 1 | decision / §12 extension |

**The findings layer (§8)** already exists by decision. It takes every violation template, every identity and JSON text column of the final queries, and the bag multiplicities of `UNION ALL` (the programs are sets).

## 8. Front-end observations

**The front end checks less than these programs need.**
- **Declarations.** It does not check that relations are declared or used at their arity. The data-layer test does, for rules and facts.
- **Value cycles.** It has no check for value-creating cycles (§12's fourth exclusion), so routes and `call_paths` parse.

**Aggregates are verbose.** An aggregate reads one atom, so each first-row window costs two to four helper relations [INFERENCE from the programs, not counted].

**Exported captures** make the matcher atom carry its evidence. A capture that the matcher leaves unbound gives no row, so optional captures need a second atom or a tree relation.

## Reproduction

```
cargo test -p rerail-engine --features metal-tests --lib logic::front         # programs + census + front end (11 tests)
RERAIL_BLESS_CENSUS=1 cargo test -p rerail-engine --features metal-tests --lib data_layer_census   # rewrite the derived TSVs
```

**Files:**
- Census: `gate/data-layer/census/{clauses,constructs,rules,recursion}.tsv`.
- Programs: `native/src/code_rule_dsl/logic/programs/data_layer/`.
- Tests: `native/src/code_rule_dsl/logic/front/{data_layer_tests,data_layer_census_tests}.rs`.
