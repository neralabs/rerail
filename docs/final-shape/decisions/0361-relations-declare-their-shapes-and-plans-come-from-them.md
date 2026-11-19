# 0361. Relations declare their shapes, and plans come from them

Status: decided (2026-11-17), being built: steps 1, 2, 3, 4, 5 and 7 built (2026-11-18). The user: "a directive which allows us to add enough data to the
relations in the .rr files that we can provide good default shapes for them (or at least name an algorithm or
something) so that we don't need to pay for planning, or only in exceptional circumstances."

## Intro

Today the planner learns each relation's shape at run time. It counts statistics (distinct values, squares of
counts, heavy values, exact pair statistics), prices orders from them, and searches. At 300 files this is a visible
share of a cold run: planning strata 3.2 %, live statistics and bookkeeping 1.8 %, exact pair statistics about 0.2 s
in each stratum that asks for them, and the join-order search (cold13).

Most of what it learns is known when the rules are written. Some examples:

- A node has one parent, so `child(p, c, i)` has at most one row for each `c`.
- A node's children are numbered 0, 1, 2, … in order, so `i` is dense within each `p`.
- A node has one kind, and every node lies in one document.

When the planner has to rediscover these, it can get them wrong. `rr_agg56` pairs every child with every earlier
sibling, 80 K rows to 1.6 M, because nothing tells the planner that "earlier sibling" over a dense order is a running
scan.

The idea: the `.rr` files state these facts once, mostly in `core.rr` for the base relations. The engine infers the
same facts for derived relations. Plans are made from the facts when the program is prepared, kept in its image, and
not searched again at run time. Run time only checks that the facts still hold. Planning from statistics remains for
the exceptions: a relation with no declared or inferred shape, or one whose rows broke what it declared.

## What a relation can declare

Each item is a fact about the rows, not about a plan. So it holds for every form made from the relation's rules:
deltas, maintenance forms (`__ins`, `__del`, `__redo`, `__supN`), and the hiding copies. 0137 found that a fixed
order would be wrong for most of these.

| Fact | Example | What the planner gets from it |
|---|---|---|
| **Key** (a functional dependency) | `child(p, c, i)`: key `c`, key `(p, i)` | A probe on the key returns at most one row. Joins on a key don't multiply rows. A projection that keeps the key needs no dedup (0329 is the special case for `node`). |
| **Fan-out class** | `child`: `p -> c` few | The rows a probe returns, without counting: one, few (16 or fewer), or many. |
| **Size** | `node`: one row per node. `kind`: one row per node. `language_table`: small | Relation sizes from counts the batch already has exactly (nodes, documents, per-kind counts, `Batch::kinds`). |
| **Local** | every column of `child` is a node of one document | Per-document joins: no rows across documents, and work partitions by document (chunks, the device). |
| **Order** | `child`: `i` is dense from 0 within each `p` | "Earlier" or "later" sibling becomes a walk along `i`. An aggregate over earlier siblings becomes a running aggregate: `rr_agg56` without the 1.6 M pairs. |
| **Tree** | `child`: `p -> c` is a forest | Reach over it is a walk up or down, with its depth bounded by the tree's. |
| **Algorithm** | `chain_below`: by reach. `owner_code`: by earlier (exists: `ordered by earlier`) | Names the native operator that makes the relation, where the rule's form alone doesn't show it. |

## Inferred for derived relations

Declarations go where inference can't reach: base relations, aggregates, and some recursion. Every other relation
gets its facts by propagating them through its rules, as types are propagated:

- A projection keeps a key if it keeps the key's columns.
- A join along a key keeps the other side's key and fan-out.
- A filter keeps everything, and can shrink the size class.
- A union keeps a fact only if every branch has it.
- `local` holds if every literal is local and they share a document variable.

Recursion keeps a fact only when it is declared, or proved by the base case and the step.

## Plans from shapes

- When the program is prepared, each rule form is planned once from the facts. Estimates come from keys and fan-out
  classes, sizes from the batch's exact counts. The plans go into the program's image (0246) with the rest of what
  preparing makes, keyed by the facts. A run reads them and does not plan.
- No statistics are gathered for a relation whose shape is declared or inferred.
- **The exceptions.** A relation with no shape is planned from statistics, as today. A relation whose rows break what
  it declares is replanned from statistics, and the profile names it. The executors already count every step's
  rows (0203), so a fan-out class broken by large factors is seen with no extra pass. A broken key is seen where the
  rows are written: dedup already probes the whole row, and a key needs one more probe.
- 0137's `.shape B after A` and `.shape B bound x` stay as the per-rule escape hatch. The test that tells you to
  remove an unneeded directive then also covers declared facts the planner no longer needs.

## Decided with the user (2026-11-17)

1. **A line per relation, `.layout`.** `.decl` stays as Soufflé reads it:

   ```
   .decl child(p: node, c: node, i: number)
   .layout child key(c) key(p, i)
           fanout(p -> c, few)
           order(i dense in p) tree(p -> c) local
   ```

   `.layout` names a declared relation. A relation may have several `.layout` lines, which add up. A fact naming a
   column the relation lacks is a syntax error with the line. `combine` carries the lines with the program and
   renames them as it renames relations, as it does 0137's `.shape`.
2. **Classes, numbers optional.** Fan-out is `one`, `few` (16 or fewer) or `many`. Size is `per n` (at most one row
   per value of column `n`), `per document`, `per kind` or `small`. A measured number may be given instead, for
   example `fanout(p -> c, ~4)` or `size(<= 2 per n)`. Classes are what plans are keyed by, so a number that changes
   does not invalidate a kept plan.
3. **A broken fact replans and is reported.** The relation is planned from statistics for the rest of the run, and
   the profile names the relation and the fact (a strict run's report too). The run goes on.
4. **Algorithms are found from the facts.** Two examples:
   - order plus an aggregate over the earlier rows of a group gives a running aggregate;
   - a tree plus recursion along it gives a walk.

   An explicit override is allowed for rare cases: `.layout r by reach`, `by walk(child)`, `by running(i in p)`.
   Like 0137's directives, a profiled run says when the override is no longer needed.

5. **A column's values, and statistics only where no fact answers** (decided 2026-11-18, after step 4's first
   measurement found no relation answered whole). Two parts:
   - A `values(c, …)` fact states how many distinct values column `c` holds: `documents` or `kinds` (the run's
     exact counts of them), `within document` (numbered in each document: about a document's rows), `many` (about
     one a row, without a key's guarantee), `~N` (at most about N) or `rows / N` (one every N rows). `core.rr`
     declares them for its base relations' other columns (`node`'s `d`, `kind`, `start`, `end`; `kind`'s `k`;
     `field`'s `f`; `text`'s and `name`'s `s`; `docof`'s `d`; `line`'s `l`; `visit_index`'s `i`), so each base
     relation is answered whole. A bound (`documents`, `kinds`, `~N`) carries to a rule's head column that copies
     it; a share of the rows does not.
   - Every other relation's statistics are made for the columns no fact answers alone; an answered column's
     statistic is the facts' estimate, not counted.

## Build steps

1. The `.layout` line: parse, check, carry through lowering, `combine`, the image and the plan-choices digest.
2. `core.rr`'s base relations declared: `node`, `kind`, `child`, `field`, `node_order`, `docof`, `text`, `name`.
3. Shape inference over the rules, as above, kept with the program's version; EXPLAIN shows each relation's facts and
   where they came from (declared or which rule).
4. The planner prices from facts: key and fan-out estimates, sizes from the batch's counts; no statistics requested
   for a relation with a shape.
5. Plans made at preparation and kept in the image; a run plans none of the relations that have shapes.
6. The checks: keys where the rows are written, fan-out from the executors' counts (0203); a broken fact replans
   and is reported.
7. Algorithms from the facts: the running aggregate along an order first. This is the `rr_agg56` planner fix: 80 K
   children, no 1.6 M sibling pairs.

Done when a cold 300-file run plans nothing at run time outside the exceptions, its findings are the reference's,
and the planning share of its profile (cold13: 3.2 % plus statistics) is gone.

## Step 7: a count below a bound, by form (2026-11-17, Q164)

**No rewrite is keyed on a relation's name** (the user, 2026-11-17). A first form (Q163) took `count : { child(g, y,
_), F(y), y < x }` by `child`'s name and its dense index, and lowered it to a running count. It is removed. It
stopped applying when a rule was written another way, and it relied on facts the program never states.

`logic::rank_form` finds the form by the rules' structure alone. A reader `n = count : H(…)` qualifies when:
- `H`'s one rule holds a comparison `ky OP kx` (`<`, `<=`, `>`, `>=`);
- without it, `H`'s body falls in two parts by the variables its literals share, the grouped ones aside:
  - the **counted part** `Y`: the literals tied to the counted columns (the reader's `_`) and to `ky`;
  - the **asked part** `X`: the rest, which binds every grouped column and `kx`.

The **group** is the grouped variables `Y` reads. The two parts are then relations of their own, and a native
operator counts:

```
rr_rankq(G…, A…, kx) :- X.                 the asked rows, each with its bound
rr_rankg(G…) :- rr_rankq(G…, _…, _).       their groups
rr_rankb(G…, C…, ky) :- rr_rankg(G…), Y.   the counted rows of those groups
rr_rank(G…, A…, n)                          Native::Rank; the reader's count becomes `n = sum m : rr_rank(…, m)`
```

`Native::Rank` (`logic::native::rank`) sorts each group's keys once and answers each asked tuple by binary search.
That is O((Q + C) log C) work, where `H` held Q × C pairs in each group. The count stays exact:
- a counted tuple of several keys counts by its least key (for `<` and `<=`) or its greatest (for `>` and `>=`);
- an asked tuple of several bounds is counted at its greatest bound (for `<` and `<=`) or its least.

The sum is 0 where nothing is asked, as the count was.

The rank's kept rule joins its two relations on the group, as `H` did. So the file-local split judges it as it judges
any rule: it is local where its groups lie in one document. It is placed after demand (magic sets), and the
relations it replaced are pruned. `RERAIL_RANK_FORM=off` finds none, a measurement aid, and `.decl H(…) no_rank`
keeps `H` as rows.

Measured at 300 files (cold58 against cold56, load 70–110):

| | CPU | Work |
|---|---|---|
| The aggregate helpers (`rr_agg*`), before | 2.06 s | 20.7 M |
| The aggregate helpers, after | 0.44 s | 3.6 M |
| The rank's relations | ≈0.5 s | |
| Every head together, before | | 437.8 M |
| Every head together, after | | 413.9 M |

- **Where it applied:** 30 counts across the program, python/ast's `body_stmt` among them.
- **Findings:** the reference's.
- **Peak:** 2.29 GB.
- **Strict 10-file:** outputs byte-identical.

When `.layout` lands (steps 1–2), a declared `order(i dense in p)` may let the engine choose a running walk over
the sort. The form itself needs no declared fact.

## Steps 1 and 2: the `.layout` line, and `core.rr`'s base relations (2026-11-18)

**The line.** `.layout r fact…` names a declared relation, then its facts. It is one line, as every `.rr` setting
is (a setting runs to the end of its line, or on past a trailing comma). So the example above is written as several
lines, which add up:

```
.decl child(p: node, c: node, i: number)
.layout child key(c) key(p, i) fanout(p -> c, few)
.layout child order(i dense in p) tree(p -> c) local
```

The facts, as `front::layout_facts` reads them:

| Fact | Written |
|---|---|
| a key | `key(c)`, `key(p, i)` |
| a fan-out | `fanout(p -> c, one)`, `few`, `many`, or a measured `~4` |
| a size | `size(per n)`, `size(<= 2 per n)`, `size(per document)`, `size(per kind)`, `size(small)` |
| local | `local` |
| an order | `order(i dense in p)`, `order(i in p)` |
| a tree | `tree(p -> c)` |
| an algorithm | `by reach`, `by walk(child)`, `by running(i in p)` |

The line is checked where it is lowered (`rr::lower::layout`): the relation must be declared, in the module or the
modules it includes, and each column a fact names must be one of its columns. Either error names the line.

**Carried.** A relation's facts are `Program::layouts`, by relation, each fact once in the order first written, its
columns by place. Every carrier of 0137's `.shape` carries them too:
- a module's merge (`rr::lower::merge`);
- `combine`'s renaming (`rr::lower::rename`, `corpus::link`), which renames the relation and a walk's relation;
- a part's header (`preparation::parts`);
- the program's facts and its image: one about row a fact, `layout(p, rel, k, fact)`, the fact with its columns as
  `$k` (`LayoutFact::placed`), read back by `program_facts` and by `About::of` (`About::layouts`).

The plan choices' key is the version's digest, which sums every about relation, so a changed layout makes new
choices with no change there.

**`core.rr`.** The base relations that the tree makes, declared from what the tree guarantees:

| Relation | Facts |
|---|---|
| `node(d, n, kind, start, end)` | key(n), size(per n), local |
| `kind(n, k)` | key(n), size(per n), local |
| `child(p, c, i)` | key(c), key(p, i), fanout(p -> c, few), order(i dense in p), tree(p -> c), local |
| `field(p, f, c)` | key(c), fanout(p -> c, few), tree(p -> c), local |
| `text(n, s)`, `docof(n, d)`, `line(n, l)`, `name(n, s)`, `visit_index(n, i)` | key(n), local |

`node_order`, named in step 2's list, is not a `core.rr` relation (it is the engine's, `facts::core`); it is left
until a rule reads it by name.

Nothing reads the facts yet: steps 1 and 2 change no plan and no finding.

**Tests.** `rr::tests::a_layout_line_states_a_relations_facts_by_its_columns` (every fact form, several lines adding
up, a repeated fact kept once, each fact read back from its placed form), `a_layout_line_names_a_declared_relations_columns`
(an undeclared relation, an unknown column, no fact, a bad class and an unknown fact, each a syntax error with its
line), and `layouts_are_carried_with_the_program` (`core.rr`'s facts on a policy's program, through the image's facts
and back, in `About::layouts`, and through `combine`). With them, 189 tests of the code the steps touch (`rr::tests`,
`program_image`, `program_facts`, `link`, `parts`, `program_text`) and the slow tier's lowering checks over every
pack pass (M1 Max). The slow tier's `rule_syntax_files_pass_check_rr` fails on `c/includes.rr`, as it does without
these steps (Q176).

## Step 3: facts inferred over the rules (2026-11-18)

`program_image::layouts::Layouts` is an analysis of the version (decision 0281's pass manager), so it is computed
once a version and kept with it. Each relation has its declared facts and, in dependency order, what its rules give
from the facts of what they read. Each fact carries its origin: declared, inferred from the relation's one rule, or
inferred from all its rules.

**What a rule gives its head.** Inference works over the body's dependencies:
- each positive atom's keys (the key's variables determine the atom's);
- each equality (`x = f(y)`: `y` determines `x`);
- each `nearest` (a node determines its nearest ancestor).

From them:

| Fact | Holds of the head where |
|---|---|
| key | one atom's key variables are in the head and determine every head variable (a projection keeping the key, a join along a key, a filter) |
| fan-out, tree | their columns are in the head and determine its row (so its rows are at most the atom's; a forest's edges kept are a forest) |
| order | the same; dense only where the rule is the atom renamed (a filter leaves gaps) |
| local | every atom is local, every node the head holds is an atom's (or `nearest`'s), and the atoms holding nodes are joined by them |

A relation of one node column at most is local by its shape.

**Where it stops.** The decision's rule is "a union keeps a fact only if every branch has it". That is sound for
`local`, which the analysis keeps across a union and, supposing its component local, across a recursion (by
induction on the rounds). It is not sound for a key: two branches may give one key two rows. So a union or a recursion
keeps its declared facts and `local` alone. An aggregate's head, a matcher's, a native's, a request's answer and the
program's facts take their declared facts alone.

**EXPLAIN.** A profiled run lists every relation's facts among its decisions (`relation layouts (decision 0361)`),
one line a relation, each fact with its columns named and its origin. For example
`f: key(c) (by rule #0000), tree(p -> c) (by rule #0000)`.

**Measured** on a Python policy's program (`python/ast.rr` and `core.rr`): 273 derived relations, of which 109 take
facts. 18 are keyed, and 108 are local. Before unions, recursions and the one-node shape were counted for `local`, it
was 48 (41 local).

**Tests.**
- `program_image::tests::layouts::`, five tests: a projection, a filter and a rename; a join along a key and a cross
  product; unions and recursions, and a union with one non-local branch; EXPLAIN's text; the Python program's counts.
- `plan::tests::explain_lists_each_relations_layout`: the lines in a planner run's decisions.

With them, 268 tests of the planner, the image, the profile and `rr` pass (M1 Max).

## Step 4: estimates from the facts (2026-11-18), the planner's side

`plan::shapes::Shapes` holds each relation's facts (step 3's `Layouts`) by name, with its declared width, in the
planner's statistics (`Stats::shapes`), shared by a component's model as the kind sets are. A column's distinct values
come from the facts where one answers it:

| Fact | The column's distinct values, of `rows` rows |
|---|---|
| `key(c)` | `rows` |
| `size(<= m per c)` | `rows / m` |
| `fanout(c -> …, one / few / ~N)` | `rows / 1`, `rows / 3`, `rows / N` (`many`: no estimate) |
| `order(c dense in g)`, `g` a fan-out's source | at most its probe's class bound: 1, or `FEW` (16) |

`few` is priced at 3 rows a probe (`FEW_ROWS`): the class bounds it at 16, and the batch estimate it replaces priced a
parent column at a third of its rows, so `child`'s `p` plans as before.

Where they are used:
- **A batch's base relations** (T's SQL waves, `Inputs::batch`): each column from its facts, else by its name as
  before (`d`, `kind`, `f`, `depth`, `s`: the columns no fact answers).
- **An unmeasured column** (`Model::ndv_or_default`): the facts' estimate before the prior (a relation of the
  component being solved: 200 values, else 1).
- **A relation the facts answer whole** (every column): its lazy statistics are not requested (`Model::deferred` is
  false) and not made (`Model::resolve` writes the facts' estimates). `RERAIL_PLAN_CACHE_COUNTS` counts them
  ("answered by layouts").

`RERAIL_LAYOUT_ESTIMATES=off` plans from statistics alone, as before (a measurement aid).

**Measured** (corpus0, 300 files x B-now's 317 rules, cold, a profiling build on the M3 under load 25-70; off, on,
off, on, each with its own store, costs and images):

| | off | on | off | on |
|---|---|---|---|---|
| findings | 463 `d067ec2ebf6f` | 463 `d067ec2ebf6f` | 463 `d067ec2ebf6f` | 463 `d067ec2ebf6f` |
| instructions | 925 G | 930 G | 955 G | 946 G |
| statistics: batches, relations | 258, 357 | 257, 353 | 324, 432 | 315, 425 |
| answered by layouts | 0 | 0 | 0 | 0 |
| orders searched | 13,850 | 13,734 | 14,585 | 14,143 |

The findings are the same. The work is within the runs' spread, which is wider than any change, so this measures
nothing of the estimates' effect on plans.

**What it does not yet do.** No relation was answered whole, so no statistics were saved. The facts answer few
columns:
- A derived relation takes `local` mostly (step 3: 18 of a Python program's 273 are keyed).
- Of `core.rr`'s base relations, only `child` is answered whole. `node`'s `d`, `kind`, `start` and `end`, `field`'s
  `f`, `kind`'s `k` and `text`'s `s` have no fact that gives their distinct values.

In the same run's profile (85.9 s of own CPU), statistics are about 3%: `statistics` (inside `make_plan`) 1.28 s,
`statistics: relations` 0.71 s, live statistics 0.26 s, the base rows measured 0.24 s, exact pair statistics
0.15 s. Planning (search, kept plans) is about 1%. Removing the statistics needs facts that answer those columns, or
statistics made only for the columns no fact answers; either changes what step 4 decided ("no statistics requested
for a relation with a shape"), so it is the user's call.

**Tests.** `plan::tests::a_layout_prices_the_columns_its_facts_answer` covers `child`'s three columns, a relation
with a column no fact answers, and inferred facts pricing a derived relation. 429 tests of the planner, the image,
`rr`, the corpus, sessions, retain and maintain mode pass (M3).

## Step 4, the rest: a column's values, and statistics only where no fact answers (2026-11-18)

Item 5 above, built:
- **The fact.** `values(c, documents | kinds | within document | many | ~N | rows / N)` (`front::ast::Values`), read
  and printed as the others are. `core.rr` declares it for every base relation's column no key, fan-out or order
  answers, so each base relation is answered whole. A bound (`documents`, `kinds`, `~N`) is carried to a rule's
  head column that copies it (`layouts.rs`); a share of the rows is not.
- **Priced by the run's exact counts.** `Stats::counts` gives the documents (`stat_docrange`) and the grammar's kinds
  (`stat_kind`); unknown, a fact of them answers nothing. A batch's `d` and `kind` were priced at 300 and 200 at
  most; they are now the counts.
- **Statistics only where no fact answers.** A chunk's base relations (the main run's and the guests') are measured
  by `corpus::sampled_statistics_unanswered`: the columns no fact answers are sampled and counted as before, an
  answered one takes its facts' estimate (`Shapes::statistic`: even postings, a document's share within documents),
  and a relation answered whole reads no rows. The planner's own (lazy, unmeasured) relations already took the facts
  (the planner's side, above).
- **Shapes made once a version.** `Shapes` is an analysis of the version (`manager::analysis`), shared by every plan
  of it. Built per `make_plan`, the planner's side had raised that span's own CPU from 1.5% to 2.3% of a cold run:
  about 72,000 plans of one program each built it.

**What it saves, measured before building it** (corpus0 x B-now, cold, profiling build, M3 under load 50-120): the
base relations' sampled measurement ("measure …" and "guest statistics") was 0.37 s of 101 s of own CPU (0.4%).
The statistics' larger share (2.3%) is `make_plan`'s own work per plan, not a count. So this step's value is the
shape (no relation's statistics counted where its facts give them, as decided) more than the time. The same four
cold runs with the values facts in (off, on, off, on) gave the same 463 findings (`d067ec2ebf6f`).

**Tests** (under Relax, the engine's live loop): `a_values_fact_prices_a_column_by_the_runs_counts` (each class,
unknown counts, the carry, the line read back), `a_base_relations_answered_columns_are_not_counted` (an answered
column estimated, the others counted as `sampled_statistics` counts them), `a_layout_prices_the_columns_its_facts_answer`
(now also: the shapes are made once a version), and the layout analysis' five.


## Step 5: plans made with the program, kept in its image (2026-11-18)

**What is planned.** A rule is planned from the facts when every literal is an atom (positive or negated) or a
comparison, and every relation it reads has, from its facts alone, a size and the distinct values of each column the
rule reads. Its forms are the rule itself and, for a recursive rule, its delta at each atom of its own component.
Aggregates, `nearest`, AST matches and relations with no such facts stay the exceptions, planned at run time as
before.

**How.** `plan::prepared::Prepared` is an analysis of the program's version (`manager::analysis`): made once a
version, with the version, and stored with its image's analyses (`rows`, read back by a later process, decision 0333
§4). It plans each such form with the planner's own search, over a model of the facts alone at a nominal scale (one
node in `NOMINAL_NODES`, 300 documents, 200 kinds): a relation's size from `size(per n)` (a node column), `per
document`, `per kind`, `small`, a key on a node column or a fan-out from one; its columns from `Shapes::ndv`; the
base relations' indexes as the run's (`vtab_indexes`); 0137's `.shape` directives as the run's. Nothing is measured
and no store is read. A plan is kept by its line (the rule's text and delta, decision 0258's `line_key`), so every
version that holds the rule finds it, with its work and rows per node.

**At run time.** A search (`plan_inner_in`) and a live plan (`plan_rule_live_in`) of a line the version's prepared
plans hold take the kept order, its work and rows scaled to the run's nodes, and search nothing, price nothing and
keep nothing. `RERAIL_PREPARED_PLANS=off` plans as before; `RERAIL_PLAN_CACHE_COUNTS` counts the lines taken
("prepared"). Step 6's checks are what replan a line whose facts the rows break.

**Stored across processes** (Q183). An analysis with rows is read back with a version reopened from a stored pipeline
run, but a stored run keeps its steps' inputs' analyses alone, and the versions a run plans are made after it
(preparation's output is folded, then split into the parts). So the prepared plans are kept where the kept plans are
(decision 0116 step 2): in their entry under the prepared program, by the planned version's identity
(`manager::identity`: its digest, size and rules' hashes in order). A run reads them with the kept plans; a version's
first plan holds those of its identity (`Store::hold`, counted as loaded) and makes only those of a version no
earlier run planned, which are written with the kept plans when the run ends.

**Tests** (Relax, live, here):
- `a_rule_over_shaped_relations_is_planned_with_the_program`: a rule over `child`- and `kind`-shaped relations is
  prepared, its order covering its literals; one reading a relation with no size is not; a declared recursive
  relation's rule is prepared with its delta; made once a version.
- `a_run_takes_a_prepared_plan_without_searching`: a run's plan of a prepared line is the kept order (one no search
  chooses, set for the test), its work scaled to the run's nodes; another line is searched.
- `prepared_plans_are_stored_with_the_image_and_read_back`: the rows read back the same.
- `a_policy_run_takes_its_prepared_plans_and_finds_the_same`: end to end, `run_policies` over three files with a rule
  over `kind` and `child`: its findings exact, and lines taken from the prepared plans.
- `a_later_run_reads_the_prepared_plans_an_earlier_one_made`: a run keeps the plans it made by the version's
  identity; a later run of the same program, built again, reads them through the store's rows (loaded 1, computed 0).

**At scale** (2026-11-18, corpus0 x B-now, profiling build, a new store, one M3 at load 50-70): 463 findings,
d067ec2ebf6f, as before. 2,313 lines took a prepared plan, against 13,279 searched and 4,042 greedy: about one line
in eight. The run's parts made 7 versions' prepared plans and read 39 back from the store (later runs of the
process reading what earlier ones kept, Q183). A second run over the same store answered from its stored results in
2.5 s and planned nothing. So step 5 holds and pays, but run-time planning is still in a cold run's profile: the
lines it plans are those of rules over AST matches, aggregates and relations with no size. Moving them is sizes
inferred for more derived relations (unions and recursion keep only `local`) and prepared aggregate rules. (A debug
build stops before planning at the check after `simplify`: Q180, apart from this step.)

**Wider: the inputs the engine makes, and derived relations sized from their rules** (2026-11-18). Of the 8,968
rules a corpus0 x B-now run did not prepare, the first literal that stopped 7,158 was a derived relation with no
size and 1,726 an input with no layout (`RERAIL_PREPARED_WHY=FILE` lists each; 23 were aggregates). So:

- The inputs: `core.rr` declares `doc`'s, `op`'s and `within`'s facts, and the layouts analysis gives an input the
  transducer makes from a kind set (`nearest`'s, an axis's) the engine's (`front::ast::made_from_a_kind_set`,
  `Origin::Made`): inferred, not written into the program, so neither side's request record moves (a pass that
  wrote them as about rows moved it; the debug run's check after the merges caught it).
- Derived relations: components in dependency order, each relation no fact sizes is sized from its rules' plans,
  their rows summed (a union's only where every branch is planned), a head column's distinct values those of the
  body column that binds it; a recursion's rows its base rules' times a nominal depth (8). `RERAIL_DERIVED_ESTIMATES
  =off` leaves them out.

Measured on corpus0 x B-now (profiling build, a new store each, M3 at load 60-130): taken as they were, the plans
over estimated relations ran the joins 70% longer (`engine run (joins)` 43.9 s -> 74.4 s, 1,053 -> 1,453 billion
instructions): an estimate carried up the strata errs, and an order chosen for it costs what the error does. The
inputs' facts alone were neutral (1,018 billion). So a prepared plan keeps the rows per node it assumed of each
relation only an estimate sized, and a run takes it only while those relations, made by then, are within
`PRICE_DRIFT` of it (`Prepared::take`); else the line is searched (step 6's check, for the sizes a plan assumed).
With it: 1,020 -> 1,024 billion instructions (off -> on), 463 findings, d067ec2ebf6f; 3,193 lines taken, 2,870
refused, 14,565 searched against 15,146. Planning is 1-3% of the run's CPU (search 0.3 s, statistics about 2.7 s
of 94 s); a wrong order costs ten times that. The criterion "a run plans none of the relations that have shapes"
is met for the relations whose facts size them; for the rest it waits on estimates good enough to keep (Q184).

**A later run on the same store** (2026-11-18). The measurement above gave each run a new store. A second run on
the store of a first, over the corpus edited so that nothing is reused, searches about 5,000 lines with prepared plans
or without: the kept plans (decision 0038) answer the rest. Instructions: 963 billion off, 987 with prepared plans
(+2.5%, within the runs' spread), 967 with derived estimates off. Making a version's plans again from the sizes a
run saw (Q184) cost 31% and was dropped. So on a warm store prepared plans save no planning, and on a cold one they
save little (planning is 1-3% of the run). The criterion that a run plans none of the relations with shapes is met
where facts size them; for derived relations it is open (Q184), as is reading plans across stores (Q185).

**Taken by default; a slow one searched where it runs** (2026-11-18, the user's design). A prepared plan is taken
as it is, estimates and all: the run's planning stays lean for the plans that are right, approximations are
accepted, and the drift check that sent a line to the search (above) is gone. A wrong one is found where it runs:

- **Horribly slow:** at each chunk, a line running its prepared order past `RERAIL_JOIN_SLOW` (8) times what the plan
  priced it at, at least a search's work (`SEARCH_WORK`), is searched (`Model::prepared_horribly_slow`). Its price is
  the plan's own, so the check is a lookup. The slow flag of decision 0203 §3 cannot find it: that compares the work
  with the running order priced under the run's statistics, and a worse order is priced right there.
- **Heavy:** a line reaching decision 0203's check (100,000 rows, then each doubling) on its prepared order is
  searched too.
- **Searched once a line, a run** (`Model::prepared_heavy`): from then on `Prepared::take` gives it none and its
  plans are searched, the wider search and trials of 0203 included. A probe is planned again at the next round; a
  seed's new order goes on under the handoff (decisions 0206, 0271), which goes back to the old one where it runs
  slower a row. The searched order is the one kept (`decide`), so a later run starts from it.
- The plan's assumed sizes beside what the run holds go in the profile's decision for the line: estimate and actual,
  for the misprediction classifier (O166) to learn from.

corpus0 x B-now, profiling, M3: joins 38.1 s off, 38.5 s on (cold, a new store); a second run on the store over the
corpus edited, 38.7 s on against about 37.5 s off. 48 lines a run searched as slow. User CPU 88.1 s off, 88.4 s on;
instructions 1,039 billion off, 1,075-1,090 on (+3.5-5%, spread over many spans, none of them planning's).
463 findings, d067ec2ebf6f, in all. Before this check (prepared plans taken and kept with no check) the joins ran
70% longer. Still to come from the design: the background learning that improves the wrongly planned and the
critical path's plans through the classifier (O166, decision 0203 §5).

