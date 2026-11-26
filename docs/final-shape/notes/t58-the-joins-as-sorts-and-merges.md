# t58: the joins as sorts and merges

The user's direction, 2026-11-24: bring the sort-and-merge shape into the join program and the store, "relations
sorted in place in their blocks, the join as a merge, and the head written as a sorted run", replacing the hash
index, the dedup tables and the host's sort. This note is the design being built to that; its increments are mine.

## What the spike measured

`native/crates/ukl_rust/src/sort_merge_spike.rs` (commits 1f8f89619e, 46837f2e49): the timed test's two strata on
its own rows, one thread on a GB10 big core, opt-level 3, best of 20.

| | Join program today | Sort, merge, neighbour dedup |
|---|---|---|
| Keyed join, 100 K x 100 K | 21.2 ms (b's index already built) | 2.34 ms (b's sort, 0.28 ms, once) |
| Closure, 400 nodes, 50 rounds | 24.1 ms | 5.12 ms with h0 as sorted runs; 1.06 ms with h0 as a bitmap |

The program's run also leaves the host to sort its head's rows (`set_delta`, about 19 ms of the keyed join's
41.4 ms); the spike's output is sorted already.

## What the store already does

- A relation made on the CPU is a sequence of physically sorted runs: each wave's rows are sorted by the relation's
  first order (`Fresh::settle`) before they are pushed, and noted as sorted (`Sorted`, decision 0180), so its
  orders take them as runs in place.
- The first order is on every column, *tagged*: by a 16-bit hash tag of the leading column, then the columns in
  turn (`runs::cmp_with`). Equal leading values are adjacent; a merge on the leading column compares `(tag, value)`.
- Rows the device writes (`Placed`) are in claim order, so the host sorts them into runs afterwards.
- The device program reads relations through a hashed index only (`find_index_from` takes `ordered == 0`).

## Increments

**A. The head written as a sorted run.**
- The program sorts a head's rows by its first order, as packed keys at the columns' widths (their statistics),
  with the radix phases it already has, and drops each row equal to the one before it.
- A recursive head's rows it already holds are removed by a bitmap of every possible row where the packed width is
  small enough (a bounded number of bits), else by a merge against its runs.
- These replace the dedup's claim, mark and scatter phases, its places table and its held map.
- The rows are placed in the head's block in that order, and the lot says so: the host notes the range as sorted in
  the first order (`Sorted::note`), and `set_delta` takes it as a run without sorting.

**B. A join on a relation's leading column as a merge.**
- A probed relation in first-order runs (every relation, once A is in): the stage's bindings are sorted by
  `(tag, key)`, and each run is merged with them, its candidates read in order.
- A key that is not the leading column keeps the hashed index, as today.

**C. Relations read from outside written in their first order.** A relation loaded into a block (a kept relation, a
source table) is written as sorted runs as it is loaded, so B applies to it.

## Invariants kept

- Blocks are never copied (0353): rows are written in their order where they are made; no second layout.
- Row ids are assigned in the order rows are written, so dead bits and levels are unchanged. Merging physical runs
  would renumber rows, so it is not part of this design.
- One program for every backend: `join_stages.uglykl`, its Rust copy (`ukl_rust::join_run`, digest-checked) and the
  encoder change together. Metal is not run until the user lifts the order of 2026-11-24; CUDA and the CPU are.

## How each increment is checked

The `setfix::device` tests on the CPU backend and on CUDA; the timed test on one big core at opt-level 3, the program
and the whole run against the scalar joins; the spike's figures as the bar.
