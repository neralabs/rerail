# Set-based SQL composition for code rules

Status: experiment on branch `Moti-Velz/sql-set-composition`. The native grammar matcher is the production engine and the parity oracle.

The current compiler investigation targets one SQL execution across all selected files and rules in a call.
The frame scheduler described below does not meet that target.
See [the whole-call compiler spikes](sql-whole-call-compiler-spikes.md) for executable alternatives, data-layer prior art, and measured limits.

## Goal

Lower every code-rule expression to set operations in SQLite. Native code runs only behind virtual tables, as **terminals executed once per batch of files**. It never runs once per joined row. The experiment succeeds when, on the nera-data-layer benchmark (12 policies, 5,725 files, limits lifted):

1. findings and failures are byte-identical to the native engine, and
2. whole-tree wall time is within 10% of native, on the same machine, over alternating runs.

There is one way to add an operator. A combinator is a lowering. A leaf is a native terminal behind a virtual table plus a lowering that reads it. No operator has both a native and a SQL implementation in production.

## Why the previous SQL engine was slow

`query/` in nera-landing at `1bc2d9de2` compiled each (file, check) to one statement over a one-file batch. Every term was a scalar `tq_*` function of `(file, node, env-handle)`, called per candidate:
- relations were correlated `tempo_nodes` subqueries, re-filtered per outer row;
- utilities were nested statements stepped from inside scalar functions, with one prepared copy per depth;
- patterns were matched per node;
- no join or set operation carried the load.

That design measured 1,589 s against native's 34 s for the same rules. This design inverts it. **Rows are the unit of work, and every statement processes all rows of a batch.**

## Data model

### Batch

A batch is a set of file versions evaluated together: the changed files of a workspace apply, or a slice of the whole tree in the benchmark. Each worker thread owns an in-memory SQLite connection, a session made once per worker and kept across batches and applies (a failed batch empties its tables and returns the session to the pool). The program's statement texts are fixed once per policy set and shared by every session. A session makes a stage's or clause's tables when it first runs, and prepares a statement on its first execution, so its statement cache is sized to the distinct texts of the clauses that ran (lesson D31).

### `node` virtual table

Rows come straight from the batch's restored tapes, with no copies. One row per tape node:

| Column | Meaning |
|---|---|
| `file` | batch-local file index |
| `id` | preorder ordinal (tape node id); the root is 0 |
| `last` | `id + size - 1`; the subtree is `[id, last]` |
| `parent` | parent id (NULL for the root) |
| `kind`, `field` | raw symbol id, field id |
| `named`, `leaf`, `extra`, `missing`, `error` | flags as the matcher sees them; `named` honours contextual aliases |
| `named_ix`, `named_n` | 1-based index among the parent's named children, and their count |
| `prev`, `next` | sibling ids under the tape's byte-positioned `prev()`/`next()` rules (zero-width skip) |
| `start`, `end` | byte span |
| `shash`, `thash` | unification keys (below) |

`xBestIndex` serves:
- `(file, id)` equality and `id` ranges;
- `(file, parent)` equality;
- `(file, kind) IN (...)` from per-file kind postings;
- `ORDER BY id` as consumed.

Walk order is preorder order:
- children left to right: `id ASC`;
- descendants in preorder: `id ASC`;
- ancestors nearest first: `id DESC`;
- later siblings nearest first: `id ASC`;
- earlier siblings nearest first: `id DESC`.

**Unification keys.** `thash` = hash of the node's source text. `shash` = `thash` for a named leaf; for a composite node it is `hash(kind, shash of every child)`. Two bound nodes unify iff:
- they have the same id, or
- both are composite and their `shash` values are equal, or
- at least one is a named leaf and their `thash` values are equal.

This reproduces `does_node_match_exactly`. `$$$X` unifies over the named nodes only; it carries a list hash computed by its terminal.

### Frames

A sub-rule lowers to a function from an input frame to an output frame. A frame is a set of rows:

`(k, file, node, c_1 … c_n, unk, wh)`

- `k`: the caller row the result belongs to.
- `c_i`: the bound node id (or exported text/span pair) of a capture; NULL means unbound. A frame holds only the captures read after it: those a later operator reads or binds (`keyed::touched`), and those the statement emits (the clause's captures, a unit's key, which is its answer). A backward liveness pass over the operator tree picks each frame's list as it is emitted: a pipeline's term holds what the later terms read, an `any`'s remainders what the later branches read, and a walk's candidates what the sub-rule reads and the captures of the output that the walk may rebind, which it returns to the row. An operator's input therefore holds every capture its output holds, so a row keeps its other bindings by copying them, and a capture no reader names is never carried. No capture an operator binds is dropped before it: the operator may leave the row's binding as it was. spt-022's statement has 96 captures; its walk candidates (494k rows) carry 0–10.
- `unk`: the three-valued flag. It exists only in clauses the matcher runs three-valued.
- `wh`: the withheld-capture bitmask for three-valued `all`.

The native matcher keeps at most one env per root at every point, because evaluation is first-match without backtracking. Every operator here therefore maps each input row to at most one output row. That is what makes the per-row semantics exactly reproducible as set semantics: an operator is a join followed by an argmin per `k`.

A clause's root frame is the set of candidate roots:
- nodes of the clause's root kinds;
- narrowed by the anchor index when one exists;
- ids below `visits_for(len)`;
- one row each, with every capture unbound.

The selected-root input uses a read-only `roots(run, file, node)` virtual table.
Each batch publishes immutable `Arc<[u32]>` slices through `RootInputs`.
SQLite reads those slices directly, without individual root INSERT calls or a temporary B-tree.
The loader normalizes unordered or duplicate roots to preserve set semantics.
The small `runs` table represents all-root selections, while an empty selected slice remains empty.
Session cleanup releases the published snapshot after both successful and failed batches.
This implementation is unconditional and has no input-path switch.

The implementation and measurements are in [the compiler investigation](sql-set-composition-compiler-spike.md#shared-root-input-slices-21-october-2026).

## Operator lowering

`R(F)` means "rule R evaluated on frame F". Lowering starts from the compiled `AstMatcher` tree, so term order, `order_by_captures`, `Never` folding and grammar gaps stay with the existing front end.

| Operator | Lowering |
|---|---|
| `kind` | filter `node.kind IN (…)` |
| `regex` | the `text_match(term)` terminal: input set = distinct (file, node) of F; returns the nodes whose text matches |
| `pattern` | the `pattern(term)` terminal, an input-set terminal: input set = distinct (file, node) of F, so a clause's root frame passes its roots as already narrowed by anchors and `RequiredText`. It emits the nodes the pattern matches under a fresh env, with their bound captures, then a join unifies them with F's bound captures. Each (term, node) is matched once per batch, whichever frames reach it. A pattern with an ellipsis directly before a metavariable that F may already bind gets the env-input variant instead: the terminal receives F's distinct (node, bound captures) rows, because greedy ellipsis alignment depends on the env. |
| `all` | sequential pipeline over terms in compiled order. Three-valued: a term's unknown rows set `unk` and OR the term's static may-bind mask into `wh`. Later terms whose static mention mask intersects `wh` pass those rows through untouched. A false term drops the row. At the end, `unk` rows are unknown. |
| `any` | branch i runs on the F rows that no earlier branch matched (anti-join on `k`), from the incoming env; results are `UNION ALL`. A single branch is that branch, with no union copying its rows. Where no branch reads or binds a capture, which branch holds a row is unobservable: the plain `kind` branches are one kind test, run first, and the others run on the rows it leaves (the spt depth walks' stop rule has 12 kind branches of 18). Three-valued: rows where a may-bind branch is unknown stop as unknown; rows where a non-binding branch is unknown continue and carry the unknown. |
| `not` | inner rule on F with the inner rule's bindings discarded; output = F rows whose `k` has no inner match (anti-join). Inner unknown makes the row unknown. Captures F has not bound match freely inside the probe, because the inner lowering only unifies against bound columns. `not { not { R } }` is one semi-join: the F rows whose `k` R matches, with F's bindings; three-valued, unknown where R is unknown. |
| `has` / `inside` / `precedes` / `follows` | join F with `node` on the axis predicate (child: `parent = f.node`; descendant: `id BETWEEN f.node+1 AND f.last`; parent; ancestor: `f.node BETWEEN id+1 AND last`; siblings via `prev`/`next` for neighbour, or `parent` plus an id range for end). Where the sub-rule can hold only at nodes of some kinds (its root kinds: `all` intersects its terms', `any` unites its branches', `not { not { R } }` and utilities are their rule's; three-valued, an `all` term that may be withheld does not count), the join also requires `node.kind IN (…)`, which the `node` table reads from its kind postings, so no other candidate row is built. Where it needs more than kinds and no capture decides it (a `has` with `stopBy: end` or a child: a descendant or child of its rule's kinds; combined through `all`, `any`, `not { not { R } }` and utilities as kinds are), each candidate is tested for that shape against the kind postings before its row is built: a recursive unit the walk feeds is demanded only where the sub-rule can hold. A three-valued walk whose stop rule may be unknown keeps every candidate: its unknown stops are placed among them. Then run the sub-rule on the joined candidates, carrying F's env; a leading kind test the candidates' kinds already pass is not run again (a two-valued walk to `kind: call` would copy every candidate row once more to test it: spt-022, 494k rows a run). If the sub-rule may bind: keep the walk-order-first match per `k` (argmin/argmax on `id`, using SQLite bare columns with `MIN`/`MAX` or `ROW_NUMBER`). Otherwise use `EXISTS`. |
| `stopBy: rule` | the stop rule runs in a fresh env, so it is context-free: a per-batch stop set S. `has`: a candidate is visited iff there is no s in S with `f.node < s.id < c.id ≤ s.last`. `inside`: ancestor a is visited iff no s in S is an ancestor strictly between a and the node. A per-batch set (a stop set, a capture-free `ofRule`) whose rule is a capture-free `not R` that cannot be unknown is R's stage read as its complement (`plan::NodeSet`): a negation usually holds at most nodes of the batch, and no stage stores them. So is one whose rule is a capture-free `any` with a `not` branch that cannot be unknown, by De Morgan: `any[not R, X…]` is the complement of `all[R, not X…]`, a stage bounded by R's kinds (the `serverplan_state_tripwires` depth walks stop at `any[not any[block, argument_list, …], decorator, …]`; as an `any` it ran one anti-join per branch over every node of the batch, about 2M rows a statement and 20 statements, and the 9 spt clauses went 7.4× → 5.2× native). Every reader inverts membership: `nearest` takes `complement = 1` with R's rows, the field walk's stop test and `nthChild`'s ranks read `NOT EXISTS` against R's stage, and a statement that reads a complement for its members is not emptied by R's stage being empty. |
| per-batch stages | a stage is lowered in the clause's logic, except that a rule that cannot be unknown is lowered two-valued under a three-valued clause too (the same verdict, with no `unk` carried through its frames), its rows written with `unk = 0` for the clause's three-valued readers. |
| `field` | `has`: the first child with that field (min id), then the stop mode as specified (the rule mode tests the field child and then only its children). `inside`: ancestor steps where the walked-from node is that ancestor's field-F child. |
| `nthChild` | `named_ix` / `named_n - named_ix + 1` against `An+B`. `ofRule`: the rank among the parent's named children matching `ofRule` (a window over `parent`). A capture-free `ofRule` whose stage is bounded (root kinds, or a complement of a stage that has them) is a per-batch set; otherwise it is evaluated on the sibling candidates with F's env, demand-driven: a kind-less one (`category-c`'s `all[not any[keyword_argument, comment, dictionary_splat], nthChild n+1]`) built 710K stage rows a batch for the few siblings its readers rank. Then the node's own `ofRule` match commits its bindings. |
| `count` | relation join with no argmin, `COUNT` per `k`, bounds filter. Three-valued: count the unknown candidates as an interval. `as` binds a transformed variable. |
| `matches` (shared env) | inline the utility's lowering on F where the rule reaches it through one reference. A utility reached through two or more references is a one-pass unit (as a recursive utility is): each reference demands its keys, and the body is emitted once. Context-free utilities (no captures read or bound) included: a `matches` is evaluated on F's keys only, as the native memo is, never as a per-batch stage over every node of the utility's root kinds (G3: on `revision_ancestry_invariants`, eager stages of kind-less utilities such as `module_level` walked from every node of every file, 25–85× native; demand-driven, 2–4×). Only whole-set readers (stop rules, a capture-free `ofRule` whose stage is bounded) are per-batch stages. |
| `matches` (parameterized `Call`) | build a frame holding only the params, unify the bound params with the caller's captures, run the body, and return the params the utility bound under the caller's names, with `unify_capture`'s min-range rule. A call that copies no parameter in runs its body on the caller's frame itself, not on a copy of every row (spt-022: copies of 494k and 321k rows a run). Called through two or more references, the body is a one-pass unit keyed by the params, mapped to and from the caller's captures at each call. The key holds every param the body may hold, in the kinds it holds it, and every param the body only reads (a `compare` against a capture of its own frame), in every kind, since each call passes it in its caller's capture's kinds; a call returns only the params the body may hold. |
| recursive utilities | they recurse only through relations, so the recursion is well-founded on the tree. Evaluate by depth level in a host loop: one statement per level per batch, never per node. A component's loop is emitted once, at its first reference; every later reference runs the same statements again over the keys it demanded. Three-valued: a unit's answer carries the body's `unk`, and a reference returns it, so an unknown verdict reaches `matches` as natively. The loops are accepted, after the statement's operator, where their read graph is acyclic over the reads the matcher makes: each frame that numbers its rows afresh starts a k-space whose rows derive from a parent row (a candidate's `pk`, a key's input rows), and after a unit's body (and after the statement's operator) the rows it needs are computed from its keys (the clause's roots) down, in creation order, as semi-joins; a first-match search's candidates past its first match (first true or, strict, unknown) are cut, a `count`'s, `nthChild` probe's or `redefines.every`'s are not (`sql/emit/lineage.rs`). Every recursive reference is recorded only from a needed row, and the check holds only the keys those reads reach from the clause's own needed rows: a function that calls itself is not refused where the matcher stops before the self-call, nor a loop only a cut candidate reaches. A recursion through a binding's site, value or annotation sub-rule is guarded as the matcher's `binding::FOLLOWING` guards it: such a sub-rule fails where the binding sub-rules being matched already hold its `(node, rule)`. A unit of a cycle that holds such a sub-rule is keyed by its following context, an interned set of those pairs per statement run (tables `fs`/`fm`; adding a pair and testing one are joins); a reference within the cycle passes the context of the guarded sub-rule it lies in, one from outside passes none. A loop through a guarded sub-rule grows its context, so its keys never repeat; a loop through none keeps it and is refused as before (`sql/emit/following.rs`). |
| `constraints` | after the rule, for each constraint in map order, join the captured node with the constraint's lowering on that node (it may bind). Unbound and `$$$` captures pass. |
| `violation.each` | a host loop of exclusion rounds per batch. Round r re-runs the clause on the roots that produced a fresh range in round r−1, with an exclusion table; relations, `any` and `binding.occurrence` skip candidates whose binding of the enumerated name falls in an excluded range. A root stops when the name is unbound or repeats. Three-valued: only a true candidate is excluded, an unknown one stays unknown (a strict search stops at it, an `any` stops at it where it may bind), and a round whose match is unknown ends its root unreported (`sql/emit/each.rs` holds the case analysis). An `nthChild` whose binding `ofRule` may be unknown is refused there (`Unsupported(three-valued nthChild whose excluding commit may be unknown)`): the matcher's excluding re-match leaves its unknown verdict on the `nthChild`'s answer. |
| semantic leaves: `value`, `compare` (semantic modes), `layout`, `arguments`, `resolves`, `binds`, `parse`, `formatArgs`, `calls` | each is an **input-set terminal**. The virtual table receives F's distinct (file, node, read captures) tuples in one `xFilter` call via `sqlite3_vtab_in`. It evaluates them natively against per-batch fact state (scope tables, value decoders, parsed literals, line tables, call graph) and returns (row key, verdict, unknown, bindings or related nodes). `binds`/`resolves.binding` return related site/value/annotation/occurrence nodes, and their sub-rules lower as relations over them; first occurrence = min id. `compare` in `source`/`unquoted`/`node`/`tokens` modes is pure SQL over text and spans where exact. |

**Distinct-key evaluation.** An operator's output row is a function of the row's `file`, `node` and the slots the operator names (reads or may bind); its other slots pass through, and `unk`/`wh` belong to the enclosing `all`. That is the key native's per-file memo uses: node, input bindings restricted to the utility's scope, and the exclusion state. So every operator that walks (relations, `count`, `nthChild` `ofRule` probes, `redefines.every`, and `not`/calls/parsed trees/host rules over a walk, the semantic leaves' walks over a slot's node or a terminal's related nodes included, whose slots and sources' reads and sets join the key) runs on its input's `DISTINCT (file, node, columns of the slots it names)` — the each root and, in a unit's loop, the origin slot included — and each input row joins its key's output on `(file, node, key columns)` with `IS` (`emit/keyed.rs`). An input already unique on a subset of the key (the root frame, a key frame, a unit's demanded keys, children of such a frame) runs directly. A per-batch stage is the zero-read case: its key is `(file, node)`. Per-row results (`count`'s groups, a non-strict search's first match, `redefines.every`'s first failing candidate) are written to tables keyed by `pk INTEGER PRIMARY KEY`, so a `LEFT JOIN` back to the frame seeks instead of rescanning. `differential::emitted_joins_never_scan_a_whole_table_on_the_inner_side` checks every emitted statement's `EXPLAIN QUERY PLAN`.

`select_when`, `require_each` and `budget` are SQL over per-file leaf rows and evidence rows (they were before D36; recover their SQL from `e049dfbd3`). They are implemented in `sql/select_when.rs` and `sql/crossfile.rs`, each on a per-thread in-memory connection whose tables are created once and cleared and refilled per call:
- `select_when`: leaf rows `(leaf, position, failed, partial, witnesses)`. The plan's tree shape drops out. A leaf that holds (a decision leaf with a witness) moves reading on to the next leaf in `Plan::leaves` order, and any other verdict reaches the root. One statement therefore computes each leaf's end, the first leaf that does not hold, the unknown row, the partial rows, and the witness budget as a running sum. `answer_leaves` pulls rows through a `Leaves` reader with exactly the native answer's calls, so lazy evaluation and its errors are unchanged.
- `require_each` / `budget`: each operand field is extracted natively at load time into `cf_value`. `eq`/`ne` operands use an injective canonical text that mirrors `same_text`, and missing fields are NULL compared with `IS`. Group and distinct keys use JSON text. The anti-join is a join plus `NOT IN`. The budget is a `GROUP BY` with `COUNT(DISTINCT …)`. `interrupted` is polled between statements; budget accounting is not emulated (§Budgets).

## What stays native and outside composition

The following stay native, at the boundary, and are never called per row from SQL:
- parsing and tape restore;
- terminal evaluation (patterns, regex, semantic facts);
- per-file fact construction;
- witness rendering after SQL returns `(clause, file, root, env)` rows: `transform`, syntax-error drop, suppressions, the witness cap, anchors, messages and ids.

## Budgets

With limits lifted, parity is exact. Native budgets are order- and memo-dependent: per-clause allowances, util steps, depth, and per-clause charging of file tables. Set evaluation cannot reproduce them. Before native leaves production, budgets are redefined:
- a per-(clause, batch) row budget and deadline enforced through SQLite's progress handler;
- a failing clause is retried per file to attribute the failure.

That redefinition is a product decision, recorded as a D-record. It is not an emulation.

## Execution

For each batch, the driver:
1. restores tapes and builds the `node` table state;
2. computes the shared stages: stop sets and capture-free `ofRule` sets, keyed by digest. A stage only one clause reads is made in that clause's run instead, after the clause's root frame, and not at all where that frame is empty (emission rules below);
3. runs one statement per clause, built from CTEs (`MATERIALIZED` where a stage is read twice);
4. runs the host loops for `each` rounds and recursion levels;
5. hands result rows to the native renderer.

Workers process batches in parallel, with the same thread count as native. Batch size is a tuning parameter.

A batch runs every compiled statement of every clause, most of them over empty frames, so the driver passes over the statements that cannot write a row. The emitter records each statement's effect as it writes it: the frames it reads, the table it writes and an emptiness rule (`EmptyIfAnyInputEmpty` for joins, semi-joins, relations and terminals over an input set; `EmptyIfAllInputsEmpty` for unions of branches; `EmptyIfPrimaryEmpty` for anti-joins, which still run when only their probe is empty; `CopyOf`; `AlwaysRun`). The session counts every frame's rows per batch from `sqlite3_changes()` and reads the rule before each statement. A clause or stage whose root frame is empty (no root of its kinds, or the RequiredText gate and anchors settled every document) skips everything after its root frame; a recursion block skips a run with no new demand rows; `each` rounds end on an empty enumeration. `RERAIL_SQL_SKIP=check` runs everything and panics where a statement the rules would skip changed a row; `RERAIL_SQL_SKIP=0` runs everything. The rules are documented with their dispatch in `skip.rs`.

Frame names are allocated like registers. Clauses run one after another and a clause's tables are emptied when it ends, so an operator's temporary frame is live only from its first mention to its last, extended through every loop that mentions it and to each `RUN` of a block that mentions it. Each temporary takes the lowest free frame name of its declaration's class, and a name taken a second time in one clause is emptied before its new temporary's first mention. A class is a declaration, so the frames that share a name share their column list: a frame keeps its list for its lifetime, and a loop's statements, and a block's at every `RUN`, read the frames they were prepared for. Clauses therefore share frame names, and statements that differed only in them share one text and one prepared statement. A recursion unit's demand, answer and reads tables keep names of their own, and so do a stage's frames, which live until the batch ends.

The plan stores its statements by shape (`sql/shapes.rs`, `sql/texts.rs`). Clauses alike differ only in their own numbers (namespace, run id, terminal ids), their own tables and their recursion blocks' marks, so a stage's or clause's statements are a shape, shared by every stage or clause alike, bound to what is its own: each text is a skeleton of phrases stored once, with holes for its own numbers, and a step's effect is an interned action whose own tables are offsets into its own. A session writes a step's text from its skeleton into a buffer as it runs it; the texts are byte for byte those its own planning drafted (every test build checks each binding). On the data-layer policies, the 2.85M steps of the stages and clauses share 411 shapes, and 175 MiB of distinct texts are stored in 15 MiB.

The engine is selected at runtime (`engine: native|sql` in the evaluation options) so a single build can A/B and run differential tests. That switch is removed when native leaves production; native remains only as a `cfg(test)` oracle.

## Code layout

`native/src/code_rule_dsl/sql/`:
- `nodes.rs`: `node` virtual table;
- `plan.rs`: frame IR;
- `lower.rs`: `AstMatcher` → IR;
- `emit.rs`: IR → SQL;
- `terminals/`: one module per terminal family;
- `exec.rs`: batch driver and host loops;
- `shapes.rs`, `texts.rs`: the plan's stages and clauses stored by shape, and its texts as skeletons of shared phrases;
- `skip.rs`: the per-batch row counts and the emptiness rules the driver skips statements by;
- `tests/`.

## Gates

| Gate | Evidence |
|---|---|
| G0 baseline | Native whole-tree time, CPU and digest on this branch (`benchmark/data-layer`), stable across 3 runs. |
| G1 kernel | `kind`, `regex`, `pattern`, `all`/`any`/`not`, relations with every stop mode and `field`, non-recursive `matches`, `Call`, `nthChild`, `count`, `constraints`. A differential test generates random rules over fixture files in Python, TypeScript and PHP, and the two engines agree on ≥ 20,000 cases. |
| G2 data-layer parity | Semantic terminals, three-valued logic, `each`, recursion and `select_when`. Whole-tree output is byte-identical to native with limits lifted. |
| G3 speed | Whole-tree wall time ≤ 1.10 × native, alternating runs ×3, recorded with load average. Reached through the speed levers below, in their order. Each lever records the profile that motivated it and its before/after numbers. **Memory condition:** peak RSS on the 47-workspace benchmark stays under 1 GiB and close to native's 564 MiB, with a row cap per batch. A lever that raises the peak noticeably is rejected, even if it is faster. |
| G4 production | Workspace and stateless paths on SQL; language conformance, pack and query-parity suites pass; budgets redefined (D-record); native only under `cfg(test)`; docs updated. |

## Speed levers (G3), in order

Correct SQL composition comes first; levers are applied only after G2 parity. Levers 3–5 are Jeremy's execution levers, which G1/G2 don't use yet. Levers 2 and 3 both attack the measured hotspot: in the G2 probe, frame row writes into SQLite B-trees plus index lookups were about 48% of the time.

1. **Measure.** Profile the whole tree on SQL. Attribute the time to statements, frames, terminals and sub-rules, with an operator ledger: rows written per statement against a per-query work bound, so excess relation construction shows up directly.
2. **Frames and nodes in Rust memory.** Frames live in Rust-owned columnar buffers exposed to SQL as virtual tables, instead of TEMP tables. The `node` table reads kind, field, parent and size straight from the tape records and derives the other columns lazily; there is no wide per-node copy. `thash`/`shash` are computed only for the kinds unification can read. Each statement still reads and writes whole frames; only the storage changes, so semantics are unchanged and the differential tests keep checking it.
3. **Transducer pushdown.** Compile the dominant capture-free sub-rules into transducer `Program`s over tapes, run once per batch across many tapes. Jeremy's list of what transducers may take is same-file joins, conjunctions, disjunctions and simple counting. The `Program` interpreter becomes a production terminal behind a virtual table and runs natively, never as recursive SQL. A pushed-down sub-rule keeps its SQL lowering as its definition and as the differential oracle for the program.
4. **Storage inside the virtual tables.** The `node` table and terminals read decompressed in-memory tape images from the content store and its arena caches: decompression, caching and pinning happen inside the tables, not in a restore step in the batch driver.
5. **Selector-driven loads.** Clause selectors decide which files and tapes a batch loads, through batched reads instead of given file lists. The kind prefilter tests that every required kind is present, with a per-file kind bitmap.
6. **Physical tuning.** Batch size and statement caching, plus the emission rules below. `xBestIndex` cost hints are not relied on.

Native shortcuts (moving a combinator into a native terminal) are not a lever: they would create a second way to add an operator. Parallel or GPU transducers across tapes stay deferred; workers keep native's 4 threads. Recursion over stored facts for multi-file analysis (coloring, call graphs) is out of scope for this experiment; `calls` stays a terminal.

### Decisions from the ast-index-spike review

Source: `neralabs/ast-index-spike` (DESIGN.md S070/S071/S089/S287/S306; branch `jeremybarnes/seadragon` S289–S309).

- **One language per tape.** An embedded region (Blade's compiled PHP, Vue scripts, HTML test scripts) is its own document with its own tape and per-grammar kind codes, linked to its host by a source map. The SQL engine runs on `Moti-Velz/embedded-documents`: a batch's documents are its files' documents, and kind postings are keyed by dialect. Walks never cross a language boundary, so the Blade overlap scan is no longer a hard case. `parse` sub-documents are a separate numbering scheme (built lazily per batch from string literals and comments) and stay as they are.
- **No wide node table.** The spike rejected a wide node table at about 69 B/node (S089). The `node` vtab reads the tape records; structural primitives (`nearest_in` and the like) read the tape, not a derived per-node struct.
- **Choose recursive shapes empirically.** The earlier blanket rejection of recursive CTEs was too broad. Current `ast-index-spike/bench/sql_rec.py` uses them for suitable walks. Its rejected generic cursor implementation does not establish that recursive SQL is infeasible. The [new Rerail spikes](sql-generation-shape-spikes.md) compile real recursive operators into SQL and compare their semantics and cost. Native structural primitives remain useful for tape access.
- **Emission rules.** The spike's four rules, each A/B-tested on the loop (lever 6, same grouping, back to back under bench-lock) and kept only where faster:
  - *Single-use CTEs are not `MATERIALIZED`* (spike: 11 ms inlined vs 5,924 ms materialized; a CTE read inside a correlated `EXISTS` stays materialized, 64 ms vs 728 ms). **Not adopted.** The engine's single-use `MATERIALIZED` CTEs (24,161 statements of 573 clauses) are terminal reads, `pattern`/`fact` over the statement's input set. Inlined, SQLite pushes the outer join's key into the virtual table and calls it once per row (`terminals_run_once_per_statement_never_per_row`: 1,401 pattern filters instead of 2), and a `LEFT JOIN` scans `fact` per row: loop SQL CPU +93%, 7 clauses past the 60 s cap (85 left uncompared). A terminal read stays `MATERIALIZED`: it is what makes a terminal run once per statement. No CTE is read inside a correlated `EXISTS`.
  - *Semi-joins are `EXISTS`, not `JOIN`* (spike: 0.46 s vs 24.3 s). **Not adopted.** The engine writes no `JOIN` semi-join; its semi-joins are `IN (SELECT …)` (165,538 statements: `not { not { R } }`, a non-binding relation, `nthChild` commits, the needed-row filters of recursion, three-valued `u` sets) and one `JOIN` + `DISTINCT` (the keyed needed rows, 18,518 statements). As `EXISTS`, the `IN` forms ran no faster (−0.1%) and the plan audit fails: SQLite builds an automatic index for `IN (SELECT …)` but scans a CTE per row under `EXISTS` (807 statements). The `DISTINCT` join as `EXISTS` probes the input per key row: +161%, 12 clauses past the cap (94 left uncompared). Input-set lists into virtual tables (`ek IN`, `nk IN`, `member IN`) are not semi-joins and stay.
  - *Ordinals come from the virtual-table cursor, not `ROW_NUMBER()`* (spike: 58 ms vs 226 ms on wide parents). **Not adopted.** The only window ranks left are `nthChild` with a stage `ofRule` (116 statements of 42 clauses); the `nearest` cursor can rank a parent's named members exactly. On those 42 clauses it ran no faster (+4.6%, +0.2%): ranks are read over the input's parents' children, a few rows, so the window costs nothing measurable. A frame `ofRule` keeps `ROW_NUMBER()`: its siblings are a frame per input row, which no cursor numbers.
  - *A per-batch shared stage is materialized only with its exact closure and only when it has at least 2 consumers* (spike: S287 regressed 50× without exact closure; S306 gained 7.5× at 8 consumers). **Adopted.** Of 43 stages, 5 have one reader (12 two, 26 three or more, up to 438 clauses); a stage already runs over exactly the files of the clauses that read it. A single-consumer stage cannot be inlined as a CTE (it is several statements and frames), so it is made in its consumer's run after the root frame, and not where that frame is empty (`exec::Plan::sole`): its 5 clauses −14.5% and −8.5% SQL CPU, the whole loop unchanged (−0.1%).
- **Not adopted.** Lowering `pattern` to SQL with a sibling NFA (set-semantic, no first-match, 1.22× native with a 14.5 s worst rule): the native pattern terminal stays. Blob handles: tapes are zstd-compressed, so lever 4 uses decompressed in-memory images.
- **Caveat.** The spike's latencies come from anchored queries over a stored index; this engine runs every rule over every file, so anchor gains will carry over less than the spike's numbers suggest.

## Known hard cases

Each gets a dedicated differential test:
- first-match binding across relations whose later terms read the bound capture;
- `any` with a later branch that would satisfy a downstream term;
- `not` over captures that are unbound at that point;
- three-valued withholding;
- zero-width siblings;
- `has` neighbour including anonymous children;
- the field `has` rule-mode one-level quirk;
- the `each` stop condition;
- `$$$` unification over named nodes only.
