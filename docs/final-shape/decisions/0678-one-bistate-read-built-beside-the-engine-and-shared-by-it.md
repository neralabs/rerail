# 0678: One bistate read, built beside the engine and shared by it

Status: in progress. Steps 1 and 2 done. The user, 2026-11-24: "I want to work on this in parallel with the current
engine (modify the current code so that we can build the new structure alongside it and share it, rather than get
rid of the old structure)."

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
L1. No test exercised the case.

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

## Step 2 (done): traces, their versions, and reads of a column and of a chunk

### Three grains of read

How a reader reads decides `Read`'s shape. A join step reads the few columns its literal binds, for each row it
visits. A probe reads a key's columns. A scan of a kept relation decodes a chunk of rows, a column at a time. Only
the checks read single cells. Every fast path resolves a column once and then reads many rows through it. So `Read`
has three grains:

- **`get(id, c)`**: one cell. Every other read is built from it by default.
- **`column(c) -> Self::Column<'_>`**: the column resolved once, a `ColumnRead` whose `get(id)` is the column's
  own read. `View`'s is the base's and the delta's `Rows::reader`. `Kept`'s is its packed column's `ColDec`.
  `Version`'s is the reader of the layer it last read, found again only when a read leaves that layer. `Marked`
  has no column of its own and reads through its `get` (`Cells`).
- **`fill(cols, from, out)`**: rows `from .. from + n` of several columns into `out`, a column at a time
  (`out[j * n + i]`). The default reads each column through `column`. `Kept`'s decodes each column's parts `FILL`
  rows at a time (`ColDec::fill`), as `Kept::each_row` does. `out` is the reader's chunk, reused as it goes, not a
  second store of the relation.

### `blocks::trace`: the `Trace` trait

A trace is one relation's history: its states at frontiers `EMPTY ..= frontier()`. A **`Frontier`** is a count of
layers. Ids below a frontier's end are the same rows at every later frontier, so any two states are a `Bistate`:

```rust
pub trait Trace {
    type Version<'s>: Read where Self: 's;
    fn frontier(&self) -> Frontier;
    fn at(&self, f: Frontier) -> Self::Version<'_>;
    fn between(&self, a: Option<Frontier>, b: Frontier) -> Bistate<Self::Version<'_>>;  // cold when a is None
}
```

`Upto<S>` reads a state below an end. That is how an adapter reads its earlier frontiers and its empty one from a
store that reads its latest.

Four stores are traces. Each reads what it already holds and writes nothing:

| Store | Its frontiers | Its version |
|---|---|---|
| `BlockTrace` (new) | its committed layers | `BlockVersion`: rows of layers `..f` whose death is 0 or past `f` |
| `relation::View` (also `FrozenFacts::view(r)`) | 1: the base alone; 2: the view | `Upto<View>`: the base's view stopped at `base_rows`, then the view |
| a continuation chain (`trace::chain(top)`) | one per relation, root first | `Upto<ChainRead>`: the relation read down its chain (`Relation::get`), stopped where the next one continues it |
| a kept key's files (`KeptChain::trace(key)`) | one per file, oldest first | `KeptVersion`: each file's rows for the key at ids past the older files'; live, the newest entry's |

How each one's change reads:

- **A view:** what the delta kills is gone and what it adds is added. Its change between frontiers 1 and 2 is
  `View::bistate`'s, and a test checks that they are equal.
- **A chain:** a fork from row 6 of ten reads its own rows at ids 6 on, so the root's frontier stops at 6. A chain
  must lie in one block, down to a relation of its own. A far base is read through another block, so its link is
  not one.
- **A kept key:** a file's entry with rows replaces the relation. Its change kills every older row and adds its own,
  which keeps L1 but is not the least change. A reads-only entry is the identity, and a gone entry kills every row.
  Kept files are snapshots, not deltas, and the trace says so.

### `BlockTrace`

All of a `BlockTrace`'s state lies in one block:

- **Its directory** (`TraceDir`): its arity, the room for layers, the layers committed (its latest frontier,
  written last on a commit), and each layer's record (`LayerRec`: its relation and the id past its last row).
- **Its rows**: each layer is a `relation::Relation` that continues the layer before's (`Relation::try_continue`,
  0384 step 7). A row's id is the same in every layer. The trace adds no storage of rows: a layer is today's
  continuation.
- **Each row's death** (`died`): a 4-byte column by id, 0 while the row is live, else the first frontier at which
  it is dead. It is the layers' kills indexed by id, written once per kill, beside them in the block. Each layer
  also records its kills in its relation's `killed` rows, for step 3.

A committed frontier's rows and deaths are never written again (a kill in the open layer marks the row dead from the
next frontier), so a version reads the same rows however far the trace has gone on. `BlockTrace::open` makes the next
layer (`LayerW`: `push`, `kill` of a row of any layer, `commit`), one at a time. A trace is found again from its
directory's offset in its block (`BlockTrace::at_offset`), which a block's root records.

Tests:

- `trace::tests`:
  - three layers, each frontier reading its layers' rows less their kills, and a kill in an open layer changing no
    committed frontier;
  - a chain of frontiers composing to the cold run (L3), walked id by id in place;
  - a version's columns and fills across layers' boundaries;
  - the trace found again from its block;
  - a view as a two-frontier trace;
  - a continuation chain as a trace, a fork dropping its base's later rows.

  Each trace checks L1 for every pair of its frontiers and every cold run.
- `kept::tests`: a kept key over four files (rows, reads-only, new rows, gone) as a trace, every pair checked; and a
  kept relation's columns and fills.
- `bistate::tests`: a view's columns and fills on both sides of its base's end.

Cost: a version's `live_word` is O(rows) for a `BlockTrace` (64 deaths a word) and O(1) a word for the others. A
kept version finds a row's file by walking the key's files, once per file its column enters. Step 3 reads a change
from the layers' kills and own rows instead of every word.

`InRel`'s start count is still a Rust value. It becomes a frontier when its relation is a `BlockTrace`'s layer, which
the maintain mode's wrapping (step 6) decides.

## The next steps

Each adds to `blocks`, makes an existing type share it, and keeps the old path.

3. **Kills recorded per layer, so a change costs its size (P1).** A delta's `killed` rows and its own new rows
   are its change. `Bistate::gone` and `added` over a trace read those, not the liveness words, and a test doubles
   the relation and checks that the work counters stay the same.
4. **Probes through `Read`.** `Read::probe(index, key)` over the layers' own indexes, as `View::probe_iter` merges
   two today. `InRel`'s probes of a version's `Old` read through it.
5. **The four maintain-mode sources from one bistate.** The planner's `ModeInputs` (old, minus, plus, new) and
   `Delta::Marks` are made from an input's `Bistate`, so the planner holds one value where it holds four sources.
   The old type stays as the form the setfix spec reads.
6. **`DeltaOp` and its harness.** The trait (a delta form and the plain `run`) and a test harness that checks L1
   (the square) and L3 (a chain of deltas equals a cold run). Two runs' outputs have different ids, so the harness
   compares them by value: each run's ids, ordered by their rows' values, in a block, and the two orders walked
   together, reading every cell in place. The first operator is a stratum's maintain mode,
   wrapped, not rewritten.
7. **`Cache` with its header** (the state it was made from, and an open mark), shared first by the plan caches
   that are keyed by version identity today.

The design page's Part IV holds the full mapping from today's types to these abstractions.
