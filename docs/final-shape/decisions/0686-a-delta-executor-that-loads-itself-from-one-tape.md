# 0686: A delta executor that loads itself from one tape

Status: in progress (the first cut, below). Design page: *Bootstrap from one tape*
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

- **K-form, the one program form.** A program is rows of eight relations, each row's first column its module `m`
  (the document it was read from): `k_rule(m, rule, head)`, `k_step(m, rule, k, kind, rel)` (kind `atom`, `not` or
  `cmp`; a `cmp`'s rel is its operator), `k_arg(m, rule, k, i, term)` (`k = -1`: the head), `k_var(m, t, name)`,
  `k_str(m, t, s)`, `k_num(m, t, n)`, `k_wild(m, t)` (what a term is), and `k_order(m, k)`. Rule and term ids are tape
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
  arguments that are variables, strings, numbers or `_`; facts. A K0 rule's tape is its K-form, read node by node.
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
