# 0678: One bistate read, built beside the engine and shared by it

Status: in progress. Step 1 done. The user, 2026-11-24: "I want to work on this in parallel with the current
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
   relation: the rows before, less those gone, plus those added, are the rows after. `bistate::reference` is the
   least change by definition ("run on both, diff").
5. **The tests stay green at each step,** and the old tests are kept.

## Step 1 (done): `blocks::bistate`, shared by `View` and `InRel`'s versions

`rerail-blocks/src/bistate.rs`:

- **`Read`**: a relation at one state, read where it lies. Its methods are `arity`, `total` (its ids, live or not),
  `live_word(w)` (the liveness of 64 ids at a time) and `get(id, c)`.
- **`Bistate<B, A>`**: two states of the same rows, where ids below `before.total()` are the same rows in both. It
  reads `added`, `gone`, `counts` and `is_identity` from the two states' liveness words. It is `cold(after)` when
  there is no before.
- **`check`** and **`reference`**: the law, and the oracle.

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
  a row brought back, and totals at word boundaries.
- setfix's `a_maintained_relations_versions_are_read_where_its_rows_lie` and
  `a_relation_derived_again_is_a_version_of_its_kept_rows` now `check` their versions. In the first, a row killed and
  pushed again is both gone and added.

Cost: `added`, `gone`, `counts` and `is_identity` read `after.total() / 64` words. That is the arithmetic the code
had before, not yet P1's O(change). Step 3 fixes it.

On the way, the roll-up 116 merge left `rerail-facts` uncompiled. One side passed `&Arc<dyn AtomTable>` where the
other's functions now take `&dyn AtomTable`, and an unsizing coercion does not deref. Six call sites now pass
`&*`.

## The next steps

Each adds to `blocks`, makes an existing type share it, and keeps the old path.

2. **A trace of layers, with frontiers.** A `Trace` holds a relation's layers in order, and a frontier is a count of
   them. `Version` is the trace read at a frontier, and `between(a, b)` is a `Bistate`. Shared by: `View`
   (two layers), a continuation chain (`Relation::try_continue`, 0384 step 7), `facts::FrozenFacts` (a full
   block and its cumulative delta) and `kept::KeptChain` (files newest first). Each gets a `trace()` reading what
   it already holds.
3. **Kills recorded per layer, so a change costs its size (P1).** A delta's `killed` rows and its own new rows
   are its change. `Bistate::gone` and `added` over a trace read those, not the liveness words, and a test doubles
   the relation and checks that the work counters stay the same.
4. **Probes through `Read`.** `Read::probe(index, key)` over the layers' own indexes, as `View::probe_iter` merges
   two today. `InRel`'s probes of a version's `Old` read through it.
5. **The four maintain-mode sources from one bistate.** The planner's `ModeInputs` (old, minus, plus, new) and
   `Delta::Marks` are made from an input's `Bistate`, so the planner holds one value where it holds four sources.
   The old type stays as the form the setfix spec reads.
6. **`DeltaOp` and its harness.** The trait (a delta form and the plain `run`) and a test harness that checks L1
   (the square) and L3 (a chain of deltas equals a cold run). The first operator is a stratum's maintain mode,
   wrapped, not rewritten.
7. **`Cache` with its header** (the state it was made from, and an open mark), shared first by the plan caches
   that are keyed by version identity today.

The design page's Part IV holds the full mapping from today's types to these abstractions.
