# 0110: A relation's indexes are sorted orders for the keys its probes bind, 4.5–8 bytes a row

Status: being built (2026-11-10).

## Intro

**The problem.** The native engine keeps, for each relation it derives:

- a hash index on every column, probed or not;
- a composite index for each key set a join probes;
- a separate hash set for the dedup test ("is this row already here?").

A hash index stores a 4-byte link per row and a 16-byte record per key. At O125 that is 46–65 bytes a
row. Copying it is most of a maintained step's fixed cost (11 ms of a 17 ms delta path).

**The target.** 4.5–8 bytes a row of index per relation, about a 90% cut. A variant that misses it is
reported as missing it. It is not re-targeted.

**The rule.** A relation keeps an *order* only for the key sets its probes bind:

- An order is the relation's row ids sorted by its columns in turn: 4 bytes a row.
- The first order holds every column, so it is also the dedup test. No hash set is kept.
- Each column's planner statistics come from a fixed-size sketch, not from an index.

## How an order looks

```text
// r(a, b, c), probed by (a, b) and by (b): two orders
order [a, b, c]: answers (a), (a, b), and the dedup test (a, b, c)
order [b, a, c]: answers (b)
// each is a list of runs, one a wave, newest last
run (wave 1): ids [7, 2, 9, …]   fences [a of id 7, a of the 64th id, …]
run (wave 2): ids [12, 15]
```

- **A key set an order answers** is the order's leading columns, in any order. Key sets that nest
  (`(a)` inside `(a, b)`) share one order.
- **A probe** binary-searches the run's fences (the leading value every 64 ids, 0.125 bytes a row).
  It then searches at most two blocks of ids.
- **A delta probe** reads the current wave's run only.
- **A probe whose key no order answers** uses the order leading with the most of its columns. Every
  key is checked on the candidates. With none, it reads every row of the delta or of the relation.

## Where an order comes from

- **Within a component:** the engine knows each probe's key set when the spec opens or a probe is
  planned again (`Rel::want`).
  - A key set that nests with an order's key sets reorders that order. That is free before its
    first wave.
  - Otherwise the key set gets a new order.
- **A later stratum** probing a frozen relation uses an order that answers its key, or one leading
  with a selective key column. Otherwise it builds an index on demand, as before (`Csr::build`).
- **A maintained relation** started from a kept one shares the kept one's runs (`Arc`). It copies
  nothing but its rows.

## When runs merge

A probe that reads more than one run counts the runs past the first (the rent). At a wave's start,
the older runs merge into one when the rent since the last merge passes the merge's cost:

```text
rent × 8 > rows of the runs merged
```

This is the rent-or-buy rule, so it costs at most twice what the best choice in hindsight would. A
frozen relation cannot merge on its probes, so its runs are merged two smallest at a time, down to
four.

## The dedup test

A pushed row is looked up in the first order. A row pushed since the last wave is not in an order
yet, so it is looked up in a hash set of those rows only. That set is emptied when the wave's runs
are made.

## Statistics

Each column has a sketch of about 1.3 KB, whatever the rows:

- **Distinct values:** a HyperLogLog of 256 registers, about 6.5% off.
- **The sum of each value's rows squared** (the planner's fan): a count sketch of 256 counters, about
  9% off.
- **Each value's exact rows**, while the column holds at most 64 values. Then the two figures above
  are exact too.

Exact per-column statistics for a frozen relation (`posted`) are read from an order leading with that
column. Without one, they are measured from the rows, as before.

## Evidence

From the index spikes at O125, about 2.1M kept rows. "Same as full" held on every step.

| Shape | Bytes a row | Probes vs today |
|---|---|---|
| Today (hash indexes + dedup set) | 46–65 | 1× |
| Sorted orders for probed key sets, dedup by the full-row order | 7.3–7.9 | about 1.6× on most steps |
| Mixed (hash tables for dedup and hot keys) | 8.4–11 | 0.7× |

- **Rejected:** the mixed design misses the target.
- **Rejected:** 16 hash bits beside each id (an estimated 14–17 bytes a row) also misses it.

## What the build must show

Measured at O125 against the engine before this decision, with "same as full" on every step:

1. **Bytes a row** of each kept relation's index (orders, fences, sketches): 4.5–8.
2. **A maintained step's delta path**: no longer dominated by copying the indexes.
3. **A whole run's CPU**: no worse than about 1.1× today. If probes cost more, that is reported, not
   traded for bytes.
