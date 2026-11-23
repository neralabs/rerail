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

## Why ids are stable, and how they need not be (the user's question, 2026-11-23)

**Why.** 0083 §3 and 0094 §2 made a node id `document | granule slot | index in the granule`, the slot a hash of the
definition's symbol. An edit inside one function then renumbers no other definition's nodes, so an edit's base delta
is the size of the edit, and the per-file facts are maintained by that delta (0083 §2, 0117). 0094 already keeps the
other job apart: **exports and kept findings never carry node ids**; they are keyed by symbol, so a moved definition's
new ids stop at its exports.

**What stable ids cost, measured.** The engine keeps the old numbering, `RERAIL_NODE_IDS=preorder` (`document |
preorder`), whose reads skip the slot search and the granule bisections (`local_of` is the low bits itself). Both
runs on m1max, `all`, one after the other at load 5–7, rows and findings identical (20,046):

| | Stable (head) | Preorder |
|---|---|---|
| CPU | 167.6 s | 157.2 s |
| Instructions | 1,653 G | 1,596 G |
| Wall | 41.1 s | 39.0 s |
| Per-file workers' CPU | 89.4 s | 82.6 s |
| Peak | 2,481 MB | 2,409 MB |

So stable ids cost about 6% of the CPU and 72 MB at the peak. They are not the gap to W; the join's loop is (§
"What follows").

**How unstable ids would work.** The user's rule, that no node id takes part in a cross-file join, holds by
construction: a join on a node variable joins facts about one node, and a node is in one file. A node id crosses
files only as a payload carried in a cross-part relation (`import_candidate(i, …)`'s `i`), never as a key that
matches two files. So:

1. **The id is `document << 32 | tape node`.** An id is its place: the tape node is the low bits, and its number
   (the view's row) is a rank, one group word and a popcount (`Numbering::rank`). Both translations go, and so does
   computing an emitted node's id (`low_of`). This goes past preorder ids, which still select a number's tape node.
2. **The file is the unit an edit rederives.** An edit renumbers the file's nodes after it, so its per-file facts are
   derived again whole. On `all` that is 82.6 s of per-file CPU over 3,089 files: about 27 ms a file on average.
   The granule-sized delta of 0083 §2 is given up for a file-sized one.
3. **What the cross part keeps across runs carries no node id.** 0094 already asks this of exports and kept
   findings. Each cross-part relation that carries a node id as a payload is either keyed by symbol, with its
   place in a side relation that only findings read (0083 §3's `struct.span`), or derived again with its file. The
   next step lists them for `all`'s program.
4. **Findings name places by line and column**, read off the tape at output, never by id.
5. **The readers that use the granules** move to the tape: node order keys (0121) become preorder indexes, as
   `RERAIL_NODE_IDS=preorder` already makes them; the per-file memo (0146) and kept frames (0212) key on the
   file's content, which a renumbering doesn't change.

What it would save, past the 6% measured: the rank-select from a number to its tape node, which preorder ids still
pay (`Projected::node`, `Ranks::select`: 2,252 of the sample's busiest 30,600). What it costs: an edit's work
becomes one file's, about 27 ms on average.
