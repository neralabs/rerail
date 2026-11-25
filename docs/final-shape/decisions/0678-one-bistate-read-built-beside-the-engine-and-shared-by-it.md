# 0678: One bistate read, built beside the engine and shared by it

Status: in progress. Steps 1, 3, 4, 5, 6 and 7 done. Step 2 cut down by the user's constraints (below) to the reads of a column
and of a chunk. The user, 2026-11-24: "I want to work on this in parallel with the current
engine (modify the current code so that we can build the new structure alongside it and share it, rather than get
rid of the old structure)."

## The user's constraints (2026-11-24)

Given while reviewing step 2's first build, which was a general trace of many versions:

1. **Two versions, never more.** "Nothing reads more than two versions. I don't want a generic MVCC system. It's
   always one version plus one delta. Deadness is: always alive, alive then dead, always dead. That's it." Then:
   "maybe we can allow dead then alive for an optimization to resurrect rather than re-insert items".
2. **Sorted on zero or one column.** "All facts are stored sorted on zero or one column. The column can vary. This
   enables better join shapes."
3. **No write order.** "I don't care about write order. There are no guarantees about that since we're memory
   mapping. We can use other means for publication."
4. **Linear history.** "there should be no forks, it's always linear". And: "we don't need forks, although maybe in
   the future we will want them".
5. **One stored form.** "one stored form: that is better, although under the hood it may use different bits. Take
   a look at the MLDB tabular datasets."
6. **States, not histories.** Asked whether anything reads a kept input's changes incrementally: "no, we only ever
   read a state, not a history".

So a row has four states across a change: always alive, alive then dead (gone), dead then alive (resurrected: added),
and always dead. A row past the version's end is new: alive (added) or dead.

## Intro

The design page *Deltas at the core* (the user's artifact, Parts I to IV) recasts the engine around one value, the
**bistate**: one relation's rows at two states, before and after. Every operator then has one contract. Its
delta form, given its inputs' bistates, writes its outputs' bistates, and it must agree with running the plain
function on both states and diffing the results. The page also names seven abstractions (`Blob`, `Trace`,
`Version`/`Read`, `Bistate`, the open layer, `Cache`, `DeltaOp`) and maps each of today's types onto one of them.

This decision is how that structure is built: **beside the current engine, sharing its storage and its code**, not
as a replacement. Each step adds a piece of the new structure in `rerail-blocks`. It makes an existing type answer
through that piece (an adapter, or the existing method's body moved into the shared one), and it leaves every
existing caller working. Nothing is removed by this decision. Whether an old type goes later is the user's call,
made step by step.

## The rules of the work

1. **New structure lives in `rerail-blocks`,** beside `relation`, `kept` and `facts`, so every crate above can use
   it.
2. **It reads the engine's blocks where they lie** (decision 0353). A `Read` is a state of rows that are already
   stored somewhere. A `Bistate` is two such states. Neither builds rows, columns or an index.
3. **An existing type is shared, not copied.** It implements the new trait, or its method's body becomes a call into
   the new code. Then the old and new paths are one code path, and the old tests test the new code.
4. **Every step comes with the law it must keep, as a test.** `bistate::check` is law L1 at the level of one
   relation: the rows before, less those gone, plus those added, are the rows after. The tests' checks read the
   states where they lie too, and copy no row (the user, 2026-11-24: "remember that all state needs to live in
   blocks").
5. **The tests stay green at each step,** and the old tests are kept.

## Step 1 (done): `blocks::bistate`, shared by `View` and `InRel`'s versions

`rerail-blocks/src/bistate.rs`:

- **Branded ids** (`ids.rs`): `RowId` (a row), `ColumnIndex` (a column) and `WordIndex` (a word of 64 liveness
  bits), each its integer underneath (`repr(transparent)`), so one is never passed where another is meant (the user,
  2026-11-24: "Please brand the id types").
- **`Read`**: a relation at one state, read where it lies. Its methods are `arity`, `end` (the `RowId` past its last
  id), `live_word(w: WordIndex)` (the liveness of 64 rows at a time) and `get(id: RowId, c: ColumnIndex)`. Step 2
  adds a column resolved once and a fill of several columns.
- **`Bistate<S>`**: two states of the same rows, both of type `S: Read`, where ids below `before.end()` are the same
  rows in both. One type serves both states: the after state reads every row the before state reads, so no pair
  needs two types, and a trace's bistate is two versions of it (`Bistate<Version<'b>>`, step 2). It
  reads `added`, `gone`, `counts` and `is_identity` from the two states' liveness words. It is `cold(after)` when
  there is no before.
- **`check`** and **`is_least`** (tests only): the law, and whether the change is the least one by value. Both
  read the states in place, id by id: a row gone or added exactly when its liveness differs, and a row live in
  both the same values in both.

**What a row is.** A row has no type and no value of its own. It is a `RowId` into its relation's storage in a
block, and its cells are read there, one `i64` each (`Read::get(RowId, ColumnIndex)`). The storage is a column per
layer: `rows::Rows` (an open layer's columns, in chunks that double) or `packed::Packed` (a kept layer's bit-packed
columns). Its liveness is the dead-bit words in the same block. A `Bistate`, a `View` and a `Marked` are handles on
the stack: references into blocks, an end, and for `Marked` the function that reads a cell. They hold no state.

**State that is not yet in a block.** A frontier is state. `View`'s two frontiers come from the facts block's root
(its relations' pointers), but `InRel`'s start is a Rust value: `Origin::Based { n, dead }`, its row count in the
struct and its dead bits a `Vector` in the run's room. A trace (step 2) holds its frontiers in its block: its
directory's layer count and each layer's end. `InRel`'s start moves there when its relation is read as a trace.

Today's types that are states already:

| Type | As `Read` / `Bistate` |
|---|---|
| `relation::View` (a base and its delta) | `impl Read`; `View::bistate()`: before the base alone, after the view |
| `kept::Kept` (a kept relation's packed rows) | `impl Read`, every row live |
| setfix's `InRel` with a version (0364 stage 2) | `InRel::version()`: a `Bistate` of two `Marked` states (rows `..n0` with the start's dead bits, then rows `..n` with today's) |

`InRel::version_ids` (`Minus`, `Plus`) now reads `version().gone()` and `version().added()`. Its own bit
arithmetic is gone, so every reader of a version (`version_view`, `version_delta`, `DeltaIds`, the frames' deltas)
goes through the shared code. One answer differs: a row dead at the start and live now is in `Plus`. The old
arithmetic counted only rows past `n0`, so it left such a row out of both `Plus` and `Minus`, and the version broke
L1. No test exercised the case. The user's constraint 1 allows such a row (a resurrection), and it is added.

Tests:

- `bistate::tests`: a base and its delta, a delta of nothing new, a full relation as a cold run, marked states with
  a row brought back, and ends at word boundaries.
- setfix's `a_maintained_relations_versions_are_read_where_its_rows_lie` and
  `a_relation_derived_again_is_a_version_of_its_kept_rows` now `check` their versions. In the first, a row killed and
  pushed again is both gone and added.

Cost: `added`, `gone`, `counts` and `is_identity` read `after.end() / 64` words. That is the arithmetic the code
had before, not yet P1's O(change). Step 3 fixes it.

On the way, the roll-up 116 merge left `rerail-facts` uncompiled. One side passed `&Arc<dyn AtomTable>` where the
other's functions now take `&dyn AtomTable`, and an unsizing coercion does not deref. Six call sites now pass
`&*`.

## Step 2: reads of a column and of a chunk

### What stays: three grains of read

How a reader reads decides `Read`'s shape. A join step reads the few columns its literal binds, for each row it
visits. A probe reads a key's columns. A scan of a kept relation decodes a chunk of rows, a column at a time. Only
the checks read single cells. Every fast path resolves a column once and then reads many rows through it. So `Read`
has three grains:

- **`get(id, c)`**: one cell. Every other read is built from it by default.
- **`column(c) -> Self::Column<'_>`**: the column resolved once, a `ColumnRead` whose `get(id)` is the column's
  own read. `View`'s is the base's and the delta's `Rows::reader`. `Kept`'s is its packed column's `ColDec`.
  `Marked` has no column of its own and reads through its `get` (`Cells`).
- **`fill(cols, from, out)`**: rows `from .. from + n` of several columns into `out`, a column at a time
  (`out[j * n + i]`). The default reads each column through `column`. `Kept`'s decodes each column's parts `FILL`
  rows at a time (`ColDec::fill`), as `Kept::each_row` does. `out` is the reader's chunk, reused as it goes, not a
  second store of the relation.

Tests: `bistate::tests` (a view's columns and fills on both sides of its base's end) and `kept::tests` (a kept
relation's columns and fills across `FILL`'s runs).

### What was removed

The first build of step 2 (commit `6cbc4e3d6e`) was a trace of many versions: a `Trace` trait with frontiers, a
`BlockTrace` with a directory of layers and a column of each row's death, adapters reading `View`, continuation
chains and kept files as traces, and `Upto` for earlier frontiers. Constraints 1, 4 and 6 leave no reader of more
than two versions and no history to read, so all of it was removed (`blocks/src/trace.rs`, `KeptChain::trace`). The
design choices it raised are on the page *Trace design choices*.

## Step 3: the fact table (`blocks::table`)

The user, 2026-11-24: "next, the fact table abstraction. we need to be able to access, join, query, and perform other
operations on the fact tables. please survey and build the abstraction and the operations that it needs to afford".

### What its readers ask today (the survey)

| Reader | Where | What it asks of a relation |
|---|---|---|
| The native join | setfix `steps::Step::Atom`, `Scalar`; `spec::JoinItem` | a scan of a delta or all of it; a probe of bound columns, through an index (`Csr`); tests of a repeated variable; a negated probe (anti-join); `count`, `sum`, `min`, `max` and the bounded extremes (decision 0511) of a key's rows; `strjoin`; a version: `Old`, `New`, `Minus`, `Plus` (decision 0364) |
| SQL | facts_base `vtab::Source` (about 80 methods) | `len`, `row_at` and `for_rows(_in)` (scan, chunks); `lookup` of one column's values, `lookup_columns` and `lookup_all` of several; `range_lookup`; `ordered` and `ordered_range`; `holds`; the planner's `estimated_matches`, `lookup_cost`, `scan_cost`, `statistics`, `value_counts`; `write` for a mutable relation; `frozen`, `revision` |
| Every compiled reader | clif `reads.rs` (the JIT, Metal, SQLite's vtabs) | `rel_get` down a chain, `rel_dead`, `rel_len`, `rel_group` and `rel_group_row` (a hashed index's bucket), `rel_key_mix`, the bloom reads |
| Kept facts | blocks `kept`, `packed` | a key's newest entry (`KeptRead`); `Packed::probe` (a range of the order's leading columns, or a permutation index); `ColDec::equal_range`, `equal_range_from` (a merge join's inner side), `lower_bound` |
| Writers | blocks `relation::RelW`, `freeze`, `packed::pack` | `push`, `extend`, `kill`, `insert_unique`; freeze (set semantics, canonical order, indexes, statistics); pack (a version and its delta rolled into one) |

Under the user's constraints, these reduce to one state's reads (`Read`: cells, columns, chunks), the two states of
a change (`Bistate`), and four ways rows are found: a scan, an equal key, a range of the sort column, and the sort
order.

### What was built

`blocks::table::Table: Read`, three things a state says beyond its cells:

- `sort()`: the column its runs are sorted on, or none (constraint 2).
- `runs()`: its ids as one or two runs, each sorted on that column: the version's, then the delta's (constraint 1).
- `probe(cols, key, each)`, with `access(cols)` saying how: `Sorted` (the sort column's range in each run, the
  rest tested), `Indexed` (an index of its own on exactly those columns), or `Scan`. `equal_range` and
  `lower_bound` default to binary searches through the column; a table with its own search gives it.

The operations, each written once over any `Table`:

- `ordered(t)`, `range(t, bounds)`: live ids in the sort order, the two runs merged as they are walked; any
  `RangeBounds<i64>`.
- `holds(t, row)`, `count(t, cols, key)`.
- `fold(t, cols, key, Agg)`: `Count`, `Sum`, `Min`, `Max`, `MaxBelow` and `MinAbove` (strict or not), the engine's
  aggregates.
- `semi(a, b, on, neg, each)`: the semi-join, and with `neg` the anti-join (a negated atom).
- `join(a, b, on, each) -> Shape`: `Merge` when a pair of `on` is both sides' sort column (`a` in its order, each
  of `b`'s runs searched only from the last key's start). Otherwise `Probe(access)`: `a` scanned, `b` probed. This
  is the join shape the sort column affords.

Two stores are tables:

- `View`: frozen, a base and its delta lie in canonical order, so they are sorted on the order's first column and
  are the two runs. Its probes use a hashed index on exactly the probe's columns in both relations (`probe_iter`).
- `Kept`: one run, sorted when its packing records an order (a granule kept as given records none). Its searches
  are its packed column's (`ColDec::equal_range`, `lower_bound`), and its probes are `Packed::probe`'s where that
  answers the columns.

The change between two states of a table is `Bistate<T>`, as in step 1. No operation holds rows. Each reads ids and
cells where they lie, with at most two cursors a side.

Tests (`table::tests`), each against the same answer read a cell at a time:

- a view's probes by every access;
- its ordered and range reads, with every kind of bound;
- a key's aggregates;
- every join shape (merged on one column or two, probed by index, by sort ranges and by scan), the semi- and
  anti-joins, and a join stopped early;
- a kept relation's probes, ordered reads and merge joins with a view, both ways round.

16 tests pass with bistate's and kept's.

### Gaps left, proposed (Claude's, not agreed)

- **A CLIF read of the sort column's range** (`rel_lower_bound`), so the JIT, Metal and SQLite share `range` and
  the merge join, as they share `rel_group` (decision 0310). The operators here are the CPU's form and the
  reference.
- **Statistics on the table** for the planner (`Relation::stats`' min and max, `Packed::col_max`, a key's
  estimated rows), so `access` can carry a cost.
- **Adapters:** `vtab::Source` read through a `Table` (its lookups, ranges, orders and `holds`), and setfix's `InRel`
  as one (its open rows are unsorted: `sort()` none until it is frozen).
- **Writes:** resurrection is done by freeze (step 4); a writer that brings back a base row by id is not needed yet.
- **Sort on one column:** freeze still sorts on every column in turn. The table relies only on the first.

## Step 4: a change read in its size

The user, 2026-11-24, on the next steps: "please do 1, 2 and 3". Step 1 of that list was to read a change from what
the delta records, not from the version's liveness.

A state now says what it changed since its version (`Read::since`): the version's end, and the version's ids whose
liveness may differ, ascending, each once (a superset is allowed: each id is checked). `Bistate::gone`, `added`,
`counts` and `is_identity` read those ids and the new rows' liveness words only. A state that records nothing is
read by words, as before.

The two engine forms record their change:

- **A frozen delta** (`Relation::changed`, a new field at the end of the descriptor): freeze writes the base ids it
  killed that were live in the base, sorted, once. `View::since` gives them.
- **setfix's `InRel`**: every kill since its start goes through `Rel::kill_row`, which pushes the id onto its
  origin's log (`Origin::killed`, beside the start it is relative to). `InRel::moved` sorts the log and drops
  repeats; `InRel::version` reads its change from it. A debug build checks the log's counts against the liveness
  words on every `version`, so a kill that bypassed the log fails the test that made it.

**Resurrection** (the user: "maybe we can allow dead then alive for an optimization to resurrect rather than
re-insert items"). What was built, Claude's choice:

- In a frozen delta, a row pushed again that equals a base row the delta killed brings that row back. Freeze cancels
  the kill rather than adding a second row, so the id stays and the change is the least one. A version's rows are a
  frozen full relation's and all live, so "dead then alive" across one delta is exactly this case. The dead bits keep
  their meaning (kills of the base), so no reader's liveness rule changed: the host's, the CLIF's or the GPU's
  `v_live`.
- `InRel` still adds a row derived again as a new row. Bringing back the old id would break its semi-naive waves:
  a wave's new rows are one range of ids (`set_delta(lo, hi)`), and an old id brought back lies outside it, so the
  rules that read the wave's delta would not see it.

Tests:

- `bistate::tests::marked_states_read_their_change_by_words`: the same change from the words and from a recorded
  superset of ids.
- `a_recorded_change_costs_its_size`: a version 64 times as large, with the same change, takes the same liveness
  reads. By words it takes at least 32 times as many.
- `a_delta_records_its_kills_and_brings_back_a_row_pushed_again`.
- setfix's version tests and its quick tier (with blocks' and clif's: 253 tests) pass with the debug check on.

## Step 5: the operations in their change form (`blocks::change`)

Step 2 of the user's list. Each operation of step 3 takes its inputs as bistates and gives its output's change. An
input `A` across a change is `A_gone`, `A_added` (both listed, step 4) and `A_kept` (live in both: never listed; a
probe of the after state finds a row, and the before state's liveness says whether it is kept).

- `join(a, b, on)`: gone `A_gone ⋈ B_before ∪ A_kept ⋈ B_gone`, added `A_added ⋈ B_after ∪ A_kept ⋈ B_added`.
  Each output pair is in one term at most, so the change is the least one when the inputs' are. The output is
  pairs of ids, so no count of derivations is needed.
- `semi(a, b, on, neg)`: `A`'s changed rows, tested against `B` before or after, then `A`'s kept rows of each key
  `B`'s change touches whose match came or went.
- `group(a, by, agg)`: each key `A`'s change touches, its value read in both states. The old row goes and the new
  one comes when they differ.

Their cost is the change's rows and their matches, plus, for `semi` and `group`, the rows of the keys the change
touches. The keys are the only thing collected, each once.

`change::check` is the harness: gone rows were in the output before, added rows were not, none is both, and the
output before, less gone, plus added, is the output after (L1).

Tests: chains of 12 changes of two relations (pseudo-random, the same every run), each state a version (a roll-up
of the last state) and its delta. At every state the change keeps L1 against the cold operation of `table`, and the
changes applied in turn from the first cold output equal each state's cold output (L3). The chains cover:

- joins merged, probed by index and probed by range, on one column and on two;
- semi- and anti-joins on the sort column and off it, with kept rows going and coming as their matches do;
- every aggregate, bounded ones included;
- a cold run's change: its whole output added.

## Step 6: the engine's relations read through the table

Step 3 of the user's list, in part.

- **setfix's input relation as a table** (`InState`): an `InRel` at one state, its rows up to `end`, those not in
  `dead` live, read where they lie. `InRel::version` is now a bistate of two of these, and `InRel::state` gives the
  state now. It is a table of one run on no sort column. Its probes are answered by the relation's own index
  (`InRel::probe_ids`, the index its rules' probes read), each candidate checked against this state's liveness and
  key. A view's rows are scanned. The change operations of step 5 run on it unchanged.
- **The CLIF read** `rel_lower_bound(rel, c, key, from, to, far)` (decision 0310): a binary search of a run sorted on
  `c`, each id read by `rel_get`. The JIT and the kernels share it, and `<ukl/reads/relation.h>` is written again
  with it. A view's two runs are each searched in the relation that holds them: its version's in the base's
  descriptor, its delta's in the delta's.

Tests:

- `a_maintained_relations_versions_join_in_their_change_form`: two maintained relations' versions are joined,
  semi- and anti-joined and grouped in their change form, each change against the cold operation (L1), the probes
  answered by the relations' indexes.
- `the_jits_lower_bound_is_the_tables`: the JIT's bound and equal range against the host table's, in a version's
  run, a delta's, and parts of each.
- The quick tiers of blocks, setfix and clif: 260 pass.

Not done, held back by Claude: SQL's `Derived::lookup` through `InState`. It was written and tested, then taken
out when the user asked what SQL had to do with this work. The saved change is in the worktree's
`tmp/planner-stats/sql-derived-lookup-through-table.txt`.

## Step 7: statistics follow a delta in the delta's size

The user, 2026-11-24, after the planner-statistics survey: "the important thing for statistics is that we have a
way of updating them as the delta happens, in a way which is proportional to the size of the delta. It can be an
approximation."

Before this step, an engine relation's statistics followed its added rows that way, but not its killed ones:

- its column sketches (`setfix::sketch`: the exact counts, the join counters, the HyperLogLog, the keys' filter)
  were only ever added to, so they went on counting killed rows;
- its kept statistics (decision 0096) found its kills by comparing every word of its dead bits with a second copy
  of them (`count_dead`, `stats_dead`): the relation's size, not the change's.

What was built (Claude's choices):

- **A kill is taken out as it is made.** `Rel::kill_row`, and `Rel::based` for the rows `gone` kills, take the
  row's values out of each column's sketch and out of the kept statistics (`uncount_row`): O(columns) a kill. A row
  killed before it is counted is never counted: `indexed_to_n` counts live rows only. `count_dead` and `stats_dead`
  are gone.
- **`Sketch::remove`**, O(1) a row:
  - the join counters are linear, so a row is subtracted (`Counts::uncount`);
  - the exact counts are decremented: few values (`Counts::remove_few`), a page, a label set. A value left with no
    rows keeps its entry at 0, so a reader reading the entries never misses a key, and `few_live` counts those
    with rows;
  - the distinct values: once a column keeps no exact counts, each HyperLogLog register keeps its rows at each rank
    (`ranks`, 256 × 32 counts, made from the last exact counts as they end). A register is its highest rank with
    rows, so the registers are exactly a fresh sketch's of the live rows. While a column keeps a page or a label
    set, its distinct values are exact, and a removed value stays in the registers. Only `overlap` reads them
    then, and it may count a key that is gone, as the keys' filter may;
  - the keys' filter keeps a removed key: keys may hold more than the column, never fewer. This is the
    approximation, and it is on the safe side for the screens.

- **Heavy values merge in the change's size.** They follow a relation from its mark (`InRel::mark`,
  `delta_ids_since_mark`), and the change is read through the version (step 4). The mark held a copy of the dead
  bits and compared them whole to tell the same relation from one made later at its address. It now holds the
  relation's `uid`, one a relation in the process; an `InRel` never changes its rows or marks.

Tests:

- `sketch::tests::a_removed_row_leaves_the_sketch_as_if_never_added`: for few values, a page, a label set and
  many values, a third of the rows and one value's every row removed. The rows, distinct values, squares, join
  counters, exact counts and (past the exact forms) registers equal those of a sketch made from the live rows; a
  value removed whole and added again counts again.
- `a_maintained_relations_sketches_are_its_live_rows`: a relation based on a kept one, with rows gone, rows added
  and some killed after they were counted, has each column's distinct values and squares of a relation made from
  its live rows.

## Next steps, proposed (Claude's, not agreed)

The shape the constraints leave, for the user to accept or change:

- **A bistate is a version and its delta, and that is the stored form.** The version is a relation: its columns,
  its dead bits (rows always dead), sorted on one column or none. The delta is the same form over the version's ids:
  its own rows (new ids, sorted on the version's column), the version's rows it kills, and the ones it resurrects.
  `View` is this already. A change reads the delta alone: gone are its kills, added its own live rows and its
  resurrections. That is the change's size (P1), with no scan of the version's liveness.
- **One stored form, encodings varied underneath (constraint 5).** As MLDB's `RawMappedIntTable` does (one
  descriptor whose first bits pick bit-compressed, run-length or factored), a column's directory entry names its
  encoding, and freeze picks the smallest from the column's statistics. Today's `rows::Rows` (an open column, 4 or
  8 bytes) and `packed::Packed` (a kept column, bit-packed) become two encodings of one column, read through one
  CLIF read. A sorted column is nearly linear, so MLDB's predictor-plus-residuals layer (`MappedIntTable`) is the
  encoding to try for it.
- **The sort column (constraint 2).** A relation records its sort column or none. A probe of it is a range in the
  version and one in the delta, and a read in its order is a merge of the two runs. Freeze no longer sorts on every
  column, so duplicates are found by hashing (`insert_unique`'s table) rather than by order.
- **Publication (constraint 3).** A state is read only once published by other means. The code claims no write
  order.
- **Roll-up.** A delta grows until it is folded into a new version, freezing a new state, as MLDB's commit does.
