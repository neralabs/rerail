# 0686: A delta executor that loads itself from one tape

Status: in progress (the first cut and its rework, below). Design page: *Bootstrap from one tape*
(https://claude.ai/artifact/7Sc29B3894aE1PBsbhwbqa), which follows *Delta-first engine primitives* and *The program
bistate and the bootstrap*.

## The user's request (2026-11-25)

"I want to begin creating A BRAND NEW EXECUTOR BASED ON THE FOUR DELTA OPERATORS". It:

- "will have ALL compilation / program modification / lowering logic in rerail";
- "will operate ENTIRELY with the operators we have written, no special Rust logic that reaches inside";
- "will have a tiny amount of hardcoded Rust steps, only enough to bootstrap the rest";
- "be very deliberately built a step at a time".

A new `bootstrap-l0.rerail` is parsed to its tape, and the tape is passed to a bootstrap level 1 operation, "written
as a bistate, whose only job is to extract the loads via ast-match operations and turn them into a program, via a
single production input. It will take one tape, no program, one production, and only facts in which are program
globals". Later levels load the core in phases: "Each phase's results will become a program and facts for the next
phase." At the end, a user `.rerail` program is loaded and all its rules compiled and lowered "into the program input
for the actual engine".

On the design page: "I think we may want more levels, but this is broadly correct." On the first level, after a
round on whether it should be a wave with a constant program read by a smaller Rust pass: one fixed Rust pass that
reads the kernel subset and skips that level ("okay, let's do it that way").

## The design

- **K-form, the one program form.** A program is rows of nine relations, each row's first column its module `m`
  (the document it was read from): `k_rule(m, rule, head)`, `k_step(m, rule, k, kind, rel)` (kind `atom`, `not` or
  `cmp`; a `cmp`'s rel is its operator), `k_arg(m, rule, k, i, term)` (`k = -1`: the head), `k_var(m, t, name)`,
  `k_str(m, t, s)`, `k_num(m, t, n)`, `k_wild(m, t)`, `k_fn(m, t, op, left, right)` (what a term is: `k_fn` an
  arithmetic expression, `+ - *`, of two terms), and `k_order(m, head, k)` (a head's stratum; see *Strata*). Rule and term ids are tape
  node ids, local to their module (L9); across modules only symbols cross. Every level writes it, the executor reads
  only it, and the user's program ends as it.
- **What the executor does with it, fixed:** a rule's steps run in `k` order; a variable is bound at its first
  occurrence and tested at every later one; an `atom` step is a join with the relation on its bound columns, a `not`
  step an anti-join, a `cmp` step a filter; a relation is a base input, a tape view, an answer, or the head of rules;
  a module's rules are one stratum, run as a fixpoint when they read their own heads and as one wave otherwise, in
  `k_order`; a rule that reads one document's views runs in that document's single-file wave, any other in the
  cross-file wave. It refuses a `not` or `cmp` with an unbound variable and a head variable no step binds, and never
  rewrites a program.
- **The kernel subset (K0):** clauses with one head; bodies of atoms, negated atoms and comparisons, in order;
  arguments that are variables, strings, numbers, `_` or arithmetic of them; facts. A K0 rule's tape is its K-form, read node by node.
- **L1, the K0 pass:** one fixed Rust pass over `bootstrap-l0.rerail`'s tape, which writes P2's K-form rows and
  refuses anything outside K0. As a delta operator: the tape's bistate in, P2's bistate out, the file its grain; no
  program port.
- **`bootstrap-l0.rerail`:** the pass's meaning written as 13 K0 rules (P2's reader), and the `load` facts of the next
  level's files. Its check: P2 run over `l0`'s own tape gives P2 again, row for row.
- **The chain:** each later level is a phase whose program came from the level before; it loads files written in
  what that program reads and exports the next program. The levels proposed on the page (K0 files that lower plain
  Datalog, then the full compiler, then the user's program) are not fixed: more may be added. The main run is the
  loop's next turn: the user's program over the sources.
- **Built beside the engine:** a crate of its own, `native/crates/delta_exec`, on `blocks` and the rr parser alone.
  No old-engine crate is a dependency or a test oracle.

## The first cut

What it proves: K-form runs on `blocks`' change operations; tape views read in place; reading moves into rules (the
self-reading check); changes flow as deltas (a program delta runs an added rule cold and removes a dropped rule's
rows); loads are answered inside a phase; and a first lowering in rules works (filters placed after their variables
are bound, written in K0).

Its pieces:

1. this record;
2. the crate;
3. a symbol table (strings to ids) in an atoms block: the one program global;
4. K-form as relations in blocks;
5. tape views (`kind`, `field`, `child`, `text`, `value`, `doc`) as reads of a tape;
6. the rule runner: cold, the steps in order with bindings as tuples of row ids; delta, one term per step (that
   step's change joined with the earlier steps' after states and the later steps' before states);
7. strata: one wave when not recursive; semi-naive when recursive and cold, and on a change the reference strategy
   (run both sides, diff), which is always allowed; strata in module order;
8. waves: rules partitioned by document into single-file waves, the rest in a cross-file wave;
9. the `load` answerer: path, bytes, the rr parser, a document;
10. the K0 pass and `bootstrap-l0.rerail`;
11. the loop, as far as L3, which runs the first lowering file.

Left out on purpose: incremental recursion across edits (levels, 0364), the rest of the plain-Datalog lowering, the
full compiler, aggregates, patterns, the passes, join ordering, statistics, pins across processes, devices.

Validation: narrow tests in the crate, run by Relax on this machine (the user, 2026-11-25: "never run any tests
outside of the NARROW new-architecture exploration"; "never do ANYTHING on a farm host").

## Strata: stated by the program, checked by the executor

The user (2026-11-25), on how strata are found: "please add a section in the design for it. I would prefer that
the Rust code stays minimal, even if it means more steps."

The first cut made a module one stratum, so a lowering that negates twice took three files. Stratifying by
dependency in the executor (the components of the dependency graph, ordered) would be Rust that reads the program to
decide what it means: the line this executor is not to cross. So:

- **The program states its strata.** `k_order(m, head, k)`: the rules of module `m` that write `head` run in stratum
  `k`; a head with no row is stratum 0. A file states them as facts, `stratum("head", k).`, which one rule of `l0`
  reads off its tape into `k_order` (a view-only rule: single-file, like the other readers). The K0 pass reads no
  strata: `l0` states none, and every rule of P2 is stratum 0.
- **The executor checks them, and computes none** (`kform::Program::order`). A program is refused when a head is
  given two strata or is written in two; when a rule reads a head of a later stratum; when it negates a head of its
  own stratum; or when it reads views before `load`'s stratum (a view's rows are the documents the loads answer). A
  stratum whose rules read one of its own heads runs as a fixpoint; any other as a wave. That is a check of a
  stated property, not a choice.
- **The cost is steps, not Rust.** Each dependency level is its own stated stratum, since a positive read within a
  stratum makes it a fixpoint, which runs by the reference strategy. The first lowering (`kernel/k1-binding.rerail`)
  states seven strata, one file again. A file that states none runs as one stratum: a user file of facts and rules
  is then a fixpoint (correct, and cold on every change).
- **Later, the strata are computed in rerail.** A stratification level, written as rules, reads a program's K-form
  dependency graph and writes its `k_order` (reachability over the graph, a stratum one past each negated head:
  recursion and a maximum). It runs as one more level of the loop, before the program it stratifies runs. Until it
  exists, a user program states its strata, and the first lowering copies them (`k1_order`).

## Auxiliary information: in the cache or in the program's storage

The user (2026-11-25): "for the slow lookups and other auxiliary information what is needed but can't be stored in
the facts or accessed efficiently from the facts, either it needs to be placed in the cache or we need to improve the
program storage to include extra information."

What a run needs of a program beyond its rows, and where it lives:

- **The cache (L5), per level** (`Db::plans`, a `kform::Program`): each module's plans (its rules' steps in order,
  each as a `change::body` part; variables numbered; the head's arguments and expressions; whether a rule is
  single-file; the columns each relation is probed by), and the strata they make. Made again only for the modules
  whose K-form rows changed (`kform::changed_modules`, read off the relations' changes); the strata are checked again
  only then. A refused run drops the cache, so the next reads the program again.
- **The program's storage**: each K-form relation has a hashed index on each set of columns the executor reads it
  by (`kform::indexes`: its module, and module with rule, step or term), so reading a module is a probe per row it
  reads. Relations are unsorted: a probe of a relation sorted on a column it binds reads that column's equal range
  whatever indexes it has, so every probe has its own index. Each relation a plan probes gets an index on those
  columns (`Db::want`): at once when it has no rows, else from the next `settle`.
- **The tape's storage** lacks one read: `child`'s index (a node's place among its parent's counted children) is
  counted along the siblings at each read. Either the tape gains a sibling index or the views keep one in the cache:
  not done yet (below).

## What was built (2026-11-25)

The crate `native/crates/delta_exec` (`rerail-delta-exec`), on `blocks`, `rr_syntax` and `tape_format` only.

- `sym`: the symbol table, an atoms block. `db`: a level's relations, each a version and at most one delta in one
  block, and its documents, each a text before and after; `settle` rolls deltas into new versions. `tape`: the six
  views, each row read off the tape into its reader's chunk; a view's first column is its document. `kform`: the
  relations and their reading into a run's plans, by probes of the rows where they lie. `k0`: L1. `run`: the
  executor. `lib`: the loop.
- **A level's form is named by a prefix.** The executor reads a program from the eight relations under one prefix:
  `k_` as read, `k1_` as the first lowering writes them. The shape is one; a level's names are its own, so a level
  can read `k_` and write `k1_` in one run.
- **`k_order` is the loads'.** The K0 pass writes the other seven; a module's stratum comes from the `load` that asked
  for it (`k_order(d, k) :- load(p, k), doc(d, p).` in `l0`). A module with none is stratum 0.
- **Bindings are registers.** A rule's variables are registers of values, set at their first occurrence; a step's
  rows are read where they lie and each runs the steps after it before the next is read. Nothing between steps is
  stored.
- **The delta of a rule:** one term per step whose input changed; the earlier steps read after, the later before; a
  `not` step's factor is its after value less its before value; a `cmp` has no term. A head's derivations are counted
  (`{head}#counts`, `change::project`); the head changes where a count crosses 0. A stratum with no counts yet, or with
  a new head, runs by the reference strategy: its derivations added, every kept count gone, and the head set to the
  counts' keys. A program's change is taken at the module grain: a changed module's rules before are run gone and its
  rules after run added.
- **Recursive strata and loads** run semi-naive from cold into open relations of the level's block, each new row
  inserted where it is found; loads are answered between fixpoints until no document comes or goes; each head is then
  diffed with its version by probes.
- **Waves.** A view's change is the change of the documents that changed, so a rule's term over a view does work for
  those documents only: the single-file waves are read off the views' changes, with no partition kept.
- **The first lowering is three files** (`kernel/k1-binding-1..3.rerail`, strata 1 to 3), since a module is a stratum
  and the lowering negates twice (`!bound_before`, `!waits`). A filter moves to just after the atom that binds the last
  of its variables. K0 has no arithmetic, so the new step numbers are facts (`atom_at`, `filter_at`): a body of at
  most 8 steps.
- **The loop** as built: L1 (the K0 pass over `l0`), L2 (P2 over `l0`, loading the kernel), L3 (`l0` and the kernel
  over the user's files: their `k_` and `k1_` forms), L4 (the `k1_` program: the user's rules run). `Boot::turn` runs
  one turn and `settle` starts the next.

Tests (`src/tests.rs`, 8, all passing under Relax): the views over a three-line tape; the K0 pass's rows for a
five-line file against rows by hand; the pass refusing an aggregate and a disjunction; the delta against a cold run over
a 24-step edit chain (a non-recursive module by its terms, a recursive one by the reference strategy); a program delta
over three turns (a rule added to a head, a rule removed, a rule added); the self-reading check, and each loaded kernel
file's rows at L2 against the K0 pass's; the first lowering (a rule with its filters first is refused as read and,
lowered, gives the rows of the rule written in order); the loop's next turn after an edit of the user's file.

Gaps found:

- `k1-binding` drops a filter with no variable, or with a variable no atom binds: the rule then runs without it. The
  executor refuses such a rule as read, but the lowered form no longer shows it.
- Plan state is still made per run: the rule list, each rule's steps and arguments in order (read by probes, then
  sorted). The K0 pass and `Db::write_set` hold a relation's new rows in a set before writing them.
- A relation's one index is on all its columns, so a probe on some columns scans the rows of its column-0 value (a
  view's document, a K-form row's module).

That first build fell short of the request, and the review (2026-11-25) said so: the executor ran rule bodies with a
nested loop of its own and used only `Tally`/`project` from `blocks`; single-file and cross-file waves were not
built, though they were described as built; rows were staged in sets; each relation had one index; the lowering
dropped filters; a module was a stratum; step numbers were fact tables; documents and loads were read again on every
turn; and a file that did not parse was read as absent. The user: "do it right". The rework follows.

## The rework (2026-11-25)

- **Every rule body runs through one operator of `blocks`:** `change::body` (0678 step 5's join from two inputs to
  `k`; committed with its brute-force test, `a_bodys_change_is_the_sum_of_its_terms`). Its parts are joins,
  negations and tests over one table type (`src::Src`: a relation's view, a tape view, or a fixpoint's rounds);
  bindings are tuples of row ids, and a variable's value is read through the row that bound it, where that row lies.
  `All(side)` is a cold run's, `Term(i)` part `i`'s term of the change. The executor (`run`) says which states each
  body reads and what is done with its derivations. It has no join, anti-join or filter of its own.
- **Tape views are tables with a change.** Each text a document has is given a range of view ids when it comes
  (`Db::set_doc`), past every earlier one; a row's id is its text's base plus its node. So a view's states share the
  ids of the documents that did not change, and the after state records its change (`Read::since`): the ids past the
  before state's end, and the old texts' ids, retired. A probe reads a bound node where it lies, walks a bound
  parent's children, or else reads each document's nodes.
- **The single-file wave:** a rule whose joins and negations all read views, of one document (their first terms one
  variable), runs for each changed document over its old text alone (derivations gone) and its new text alone
  (derivations added); the other documents do no work. **The cross-file wave:** any other rule runs one term for
  each part whose input changed. A changed module's rules run gone over the before state and added over the after
  state. Heads change where a derivation count crosses 0 (`change::project`), each change written into the head's
  delta as it is found (`Db::write_pair`).
- **The fixpoint:** semi-naive from cold, each round a frozen relation of the level's block (`src::Rounds` reads the
  rounds as one table, the last round its change); round `n` runs, for each part that joins a head of the stratum,
  that part's term. Each head is then set to its rounds by `write_diff`.
- **Loads:** a stratum that writes `load` runs its `load` rules first, answers each path whose stamp changed
  (`World::stamp`; a path whose stamp did not change is not read), and again until no document comes, goes or
  changes. A loaded file that does not parse is refused, naming it (the parser writes a bad statement as an ERROR
  leaf; `Text::parse` refuses a tape with one).
- **Arithmetic in heads:** `k_fn(m, t, op, left, right)`, read from `binary_expression` nodes by the K0 pass and by
  `l0`'s rule alike, computed where a head row is made. The first lowering numbers steps with it: a filter of no
  variable at its own place (first), atom `k` at 2^20 (k + 1), a filter placed after atom `p` at
  2^20 (p + 1) + 1 + f, a filter with a variable no atom binds at 2^40 + f (last, where the executor refuses it). No
  filter is dropped, and a body has no bound on its steps.
- **The K0 pass at the file grain:** a file whose text did not change is not read; a changed file is read whole
  first (a refused file writes nothing), then its module's rows are killed by a probe of the module index and its new
  rows pushed as the walk finds them.
- **No staging:** rows are written where they are found; the sets that remain hold no block data (a test's expected
  rows, a load's asked paths).
- **One `load` per path:** `load(path)`, arity 1; `l0` loads `kernel/k1-binding.rerail`, one file.

Tests (`src/tests.rs`, 14, all passing under Relax on this machine): the eight above, adapted (`k_order` rows are
`(m, head, k)`; the kernel's stated strata are 1 to 7), and:

- the lowering keeps every filter (one of no variable runs first; one with an unbound variable is refused at L4);
- arithmetic in a head;
- a load that does not parse is refused;
- stated strata are checked (a head negated in its own stratum; a head read before its stratum);
- an edit of one of three documents does the work of that document's old and new texts alone (3 + 4 derivations,
  none for the others);
- a turn with nothing changed reads no file again and runs no rule but the two `load` facts.

Gaps that remain, each with its remedy:

- **The roll-up at `settle`** writes each changed relation's rows into a new version: O(rows) per changed relation
  per turn. `blocks`' `View` is two-level (a version and one delta), so a third state needs either a multi-level
  view or a compaction policy in `blocks`.
- **Recursion runs cold.** A fixpoint stratum runs from nothing on every change (the reference strategy), and a file
  that states no strata is one fixpoint. The remedies: the stratification level (above), and incremental recursion
  (0364's levels) later.
- **Discovery runs its `load` rules cold**, then diffs `load`: O(loads) per turn for `load` facts; a load computed
  from documents would read them again.
- **`child`'s index is counted along siblings** at each read (see *Auxiliary information*).
- **An index a plan asks for on a relation that has rows** is built at the next `settle`; until then that probe scans.
- **A view probed with no node or parent bound** (`kind(d, c, "clause")`) reads each node of its documents: one
  document's nodes in a single-file wave, every document's in a cold run.
- **The fixpoint's dedup** probes each earlier round for a new row: O(rounds) probes a row.
