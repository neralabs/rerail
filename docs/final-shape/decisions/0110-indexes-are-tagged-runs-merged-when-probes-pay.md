# 0110: A relation's indexes are tagged runs, merged when their probes pay

Status: being built (2026-11-10).

## Intro

**The problem.** The native engine keeps, for each relation it derives, an index per column and
one per composite key the joins probe. Each index is a hash table of postings with a "next" link
per row. That is 46–65 bytes a row at O125, and copying it is most of a maintained step's
fixed cost (11 ms of a 17 ms delta path).

**The rule.** An index on a key is a list of *runs*. A run holds row ids sorted by the key's
values, with 16 bits of the hash of the key's leading value beside each id. Each wave's new rows
become one run: that run is the wave's delta. Runs are merged only when the probes since the
last merge would have cost more than the merge.

The dedup set (one per relation, on the whole row) stays a hash table.

## How an index looks

```text
// relation r(a, b), an index on (a, b): one run per wave, newest last
run 0 (wave 1): ids [7, 2, 9, …]   tags [0x01a3, 0x01a3, 0x4410, …]   // sorted by (tag, a, b)
run 1 (wave 2): ids [12, 15]       tags [0x0022, 0x9f01]
```

The sort key is the tag first, then the values. So the rows of one leading value are still
together, and every prefix of the key (here `a` alone, or `a, b`) is a range of each run.

- **Merging** two runs compares tags. It reads the two rows only when the tags are equal.
- **A probe** finds the tag's range from where the tag would sit if tags were spread evenly
  (the tag times the run's length, as mldb's PathIndex places a hash), widened by doubling
  steps. It then binary-searches the rows within that small range.
- **A delta probe** reads the current wave's run only.

## When runs merge

Each index counts its probes since its runs last changed. Its runs are merged into one when

```text
probes × (cost of a probe over its runs − cost over one run) > cost of the merge
```

This is the rent-or-buy rule, so it costs at most twice what the best choice in hindsight would.
A relation that is rarely probed keeps its runs, and costs its waves nothing more than the sort
of their own rows.

## What else an index answers

- **Distinct keys** (a column's `ndv`, read by planning): each run counts the keys it holds that
  no older run holds when it is made. That is one probe per distinct key of the new run.
- **Column statistics** (`posted`): the runs are merged once, then read in order.
- **A short lead column** for a composite key (`chain_lead`): the column's index, as before.

## Evidence

O125 steps 1–3, summed over the large steps. The orders cost alone; "same as full" held on every
step.

| Variant | Total (ms) |
|---|---|
| Merged two at a time as runs come | 1096 |
| Hash bits | 905 |
| Lazy merging and hash bits | 799 |
| Lazy merging, hash bits, interpolated probe | 720 |

- Of the rows the tagged merges read, 21% are tag collisions. The other 79% are rows with the
  same leading value, which no hash of that value can separate.
- Merging more runs at a time (fanout 4 or 8) did not help: the cost is row compares, not copies.

## What the build must show

Measured at O125 against the engine before this decision, with "same as full" on every step:

1. **Bytes a row** of the kept relations' indexes: from 46–65 to under 20.
2. **A maintained step's delta path**: no longer dominated by copying the indexes.
3. **A whole run's CPU**: no worse than about 1.1× today.
