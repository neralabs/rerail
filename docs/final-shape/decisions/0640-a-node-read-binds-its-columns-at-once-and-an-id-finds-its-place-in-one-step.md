# 0640. A node read binds its columns at once, and an id finds its place in one step

Status: designed, and its first lever measured (2026-11-23); nothing built. Numbered past the highest record in any
worktree (0639).

## Intro

t53 §4 measured the gap to W: `all`'s per-file rules take about 69 s for 62 M rows, against W's Rust parser at a
fifth of the CPU. A sample of the per-file phase (t53 §4) puts its busiest frames, about 30,600 samples together, in three groups:

- the join's own loop: `cands::candidates`, `run_join_in_each::enter`, `RowRef::at`, `Cands::any`, `Rows::put`,
  `Sketch::add`, `push_fresh`, with `IntCol::get` (sealed chunks, 0622) under them: about 11,700 samples;
- an id's way to its place on the tape: `Projected::node`, `Projected::local_of`, `Numbering::granule`,
  `Ranks::select`: 4,473;
- the tape read at the place: `Tape::code`, `f16::side`: 1,433.

W holds a node as a pointer to its struct. Here a node is a 64-bit id (`document << 32 | low`, 0494 step 7a), and each
view read (`node`, `child`, `field`, `line`, `docof`, …) turns it into its place again.

## How an id becomes a place today

A probe of a view by its node column (`Viewed::read_matches`, `Batch::node`):

1. `Batch::doc`: the document from the high 32 bits (a one-entry hint, else a search).
2. `Projected::local_of` → `Numbering::number_of`: the low bits are a granule's slot (a hash, so an edit elsewhere
   leaves the id alone) and an index in the granule. The slot's entry is found near its share of the entries (up to 8
   steps, else a bisection), then `number_in` bisects the granule's child granules to give the node's number (its
   rank among the projection's nodes).
3. `Projected::node` → `Numbering::select`: the number's tape node, from a sample every 64 numbers and the groups'
   bit counts.
4. The columns read at the tape node (`value_of`: `Tape::code`, `f16::side`, …).

A row a view emits carries its id. Computing it from the number is `Numbering::low_of`, which bisects the granules
(`Numbering::granule`), and it is computed again whenever the emitted node is read by another view.

## The two changes

**A. One read binds a node's columns.** A rule that reads several per-node facts of one node (`child(p, c, _),
kind(c, K), text(c, s)`) translates `c` once per atom today. Of `all`'s rules (the program and the Python library it
includes), 724 atoms read a view by a node variable. 381 of them sit beside another such atom on the same variable,
in 172 (rule, variable) pairs. The commonest pair is `child` and `kind` (50), then `field` and `text` (28) and `field`
and `kind` (17). The shape: the views that bind a node (`node`, `child`, `field`, `next_sibling`, `core_node`) carry
the functional per-node columns (`kind`, `docof`, `line`, `node_order`, `ast_depth`) as columns of their own. A
preparation pass folds such an atom into the atom that binds its variable. Each of those views keeps the by-kind
lookup the `node` view has (`Batch::by_kind`, one row a node), so a plan that entered by kind still can.

**B. An id finds its place in one step.** The id's granule entry gives the node's number and tape node together.
Steps 2 and 3 above become one table read where the granule's own nodes run unbroken, and the bisection of child
granules only where they don't. The number is still needed, since the views' rows are laid out by it (`Batch::row`).
This is a change to the numbering (`rerail_clif::ids`), which every reader shares (decision 0310: the host, CLIF, the
JIT and the kernels' `ids.h`). It is written once there.

## Measured: what A can save

A temporary counter on `Batch::node` (m1max, `all`, this head) counted every id translation, and those whose id was
the one the same thread had just translated or emitted from a view read:

| | |
|---|---|
| translations, per-file phase and after | over 40 M |
| of the id just found or emitted | 14% |

A one-entry hint or a fold of the pairs saves at most those 14%. The other 86% of ids reach a view read from a stored
relation's column: a derived fact holds the node, and a later rule reads its kind or text. A fused read doesn't touch
them; only B does.

## What follows

- A is built only with B: alone it saves at most a seventh of the translations, so a seventh of the id path's 4,473
  samples.
- B is the id path's whole lever: most of those 4,473 samples, about 15% of the busiest frames.
- The join's own loop is 2.6 times the id path. Its per-candidate cost in `cands` and `run_join_in_each`, measured per
  step, is the larger lever and comes first.
