# 0678: One bistate read, built beside the engine and shared by it

Status: in progress. Step 1 done. Step 2 cut down by the user's constraints (below) to the reads of a column and of a
chunk. The user, 2026-11-24: "I want to work on this in parallel with the current
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
