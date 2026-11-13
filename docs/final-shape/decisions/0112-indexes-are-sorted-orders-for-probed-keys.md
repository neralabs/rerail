# 0112: A relation's indexes are sorted orders for the keys its probes bind, 4.5–8 bytes a row

Status: built and accepted (2026-11-11): 5.9–9.7 bytes a row at O125, about 1.08× today's CPU.

## Intro

**The problem.** The native engine keeps, for each relation it derives:

- a hash index on every column, probed or not;
- a composite index for each key set a join probes;
- a separate hash set for the dedup test ("is this row already here?").

A hash index stores a 4-byte link per row and a 16-byte record per key. At O125 that is 46–65 bytes a
row. Copying it is most of a maintained step's fixed cost (11 ms of a 17 ms delta path).

**The target.** 4.5–8 bytes a row of index per relation, about a 90% cut, and 2 bytes more for 16
bits of hash on the full-row order (approved 2026-11-11; no more). A variant that misses it is
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
- **The full-row order is tagged.** It sorts by 16 bits of the leading value's hash first, then by
  the columns, and keeps each id's tag (2 bytes) instead of fences. A key's span is found among the
  tags, and only its ids' rows are read. Equal leading values share a tag, so every prefix is still
  a range. Its sorts and merges compare tags as integers and read rows only within one tag.

```text
order [a, b, c], tagged: ids sorted by (tag(a), a, b, c), tags [t of id 7, t of id 2, …]
find (a = 7): the span of tag(7) among the tags, then the rows of that span
```

- **Probes come in sorted batches where they can.** Each join step keeps its place in each run it
  probes and searches from there, galloping either way. A step that scans 256 rows or more reads
  them in the next step's key order, so the next step's probes are one pass over each run.

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

A wave at a time. A rule's derived rows are kept unchecked; at the wave's end (or past 65,536 rows)
they are sorted as the full-row order sorts, repeats dropped, and each run of that order is searched
once, in order. The wave's new rows go in already sorted. A gate's relation, whose rows are read
before its wave ends, also keeps them in a hash set of those rows only until its wave's run is made.

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

## Measured (2026-11-10)

O125 steps 1–8, run twice each, alternating with the engine before this decision (c04a18cb2). "Same as
full" held on all 8 steps of all 4 runs. The load average was 24–87.

| | Before | After |
|---|---|---|
| Bytes a row, kept relations' indexes ("other", ~1.07M rows) | 53–66 | 4.7–5.5 |
| Bytes a row, kept relations' indexes ("supports", ~0.78M rows) | 53–65 | 6.8–7.9 |
| Whole run CPU, user + sys (s) | 38.2, 33.5 | 45.7, 38.7 |
| Maintained steps' session CPU, summed (s) | 12.7, 10.9 | 14.7, 13.1 |

- **Target 1 is met:** 4.7–7.9 bytes a row, about a 90% cut.
- **Target 3 is missed:** the whole run's CPU is 1.15–1.20× before, against about 1.1×. The
  maintained steps are 1.16–1.20×.
- **Not yet split:** how much of the extra CPU is probes (binary search against a hash lookup) and how
  much is the dedup test (a lookup in the full-row order for each row derived).
- **Next:** test the dedup once per wave, by sorting the wave's rows and merging them against the
  full-row order, as the first spike proposed. Then split the probe cost.

## Measured (2026-11-11)

O125 steps 1–8, the same program and planner fix on every engine, alternating in each round, "same as
full" on all 8 steps. Today's index was built on the same program by taking the engine's index code
from before this decision. Load averages 10–80, so rounds are compared within themselves.

| | Today's index | This decision |
|---|---|---|
| Bytes a row, "other" (~1.08M rows) | 53–66 | 5.9–7.4 |
| Bytes a row, "supports" (~0.80M rows) | 53–64 | 3.7–9.7 |
| Index at step 8 | 44 + 62 MB | 7.2 + 7.4 MB |
| Peak RSS | 2.0 GB | 1.62 GB |
| User CPU, one round (s) | 31.8, 31.4 | 33.5, 34.5 |

- **Bytes:** within 8 + 2. "Supports" is past 8 only by the full-row order's tags.
- **CPU:** about 1.07–1.10× today (1.13× before the batch dedup's sort, the tags and the batched
  probes). Accepted (2026-11-11).
- **Where the rest is** (share of the run): probes 10.7% against today's 4.3%; within them the index
  search is 5.6%, about half of it the loop over a probe's several runs. The batch dedup and run
  building are 5.6%, against today's hash push 5.6%.
- **What did not pay:** a tag on the second column within the leading value, and freezing kept
  relations to one run an order, were both neutral. Every approach tried, with its numbers, is in
  `../index-shape-exploration.md`.
- **Fixed on the way:** the plan cache held an empty relation "near" a one-row one, so a plan priced
  while a relation was empty was reused once it filled (O125 step 2: 32.5 s instead of 0.2 s). It
  affected every engine.
