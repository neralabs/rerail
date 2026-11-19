# 0124: Lattice relations fold by key, on the native set engine and on Metal

Status: built (2026-11-12).

## Intro

**The problem.** A rule file can declare a relation as a lattice's: its last column is a value,
and two rows with the same key join into one, by the lattice's join (`max`, `min`, `flat`,
`shortlex_min`, ...). Only SQLite's evaluation folded them. The native set engine and the Metal
device declined every stratum with a lattice relation, so those strata ran as SQL.

**The decision.** Both engines hold a lattice relation as one row per key, folded as SQLite's
kernel folds it:

- a missing key is bottom, and the top absorbs every other value;
- after `widen` changes to a key, its next value is the lattice's widening;
- when a wave's values for a key join to a new value, the old row dies and the new one is the
  next wave's delta, so readers and later probes see only the folded value.

## On the native set engine

A spec line `lattice <rel> <keys> <name> <widen>` marks a relation as a lattice's
(`setfix::Lat`). The planner still declines a lattice stratum in these cases:

- it has gates, or requests to answer: their SQL seeds and probes insert by key, not by join;
- its lattice has unbounded height and no `widen`;
- it starts from kept rows (a maintained session's survivors): those are a set there.

A lattice stratum is not maintained from deltas (`maintain::unmaintainable`). A session derives
it again on each run.

## On Metal

The device folds each round of a recursive component by key (`metal_join`'s
`Batch::lattice_fold`, kernels `lat_claim`, `lat_best`, `lat_merge`, `lat_rehash`):

- a round's rows are grouped by key and the join picks each key's value (min or max; a `flat`
  conflict goes to the top);
- the result is merged with the held row through a key table kept on the device from round to
  round, which grows and rehashes past half full;
- a held key's row is updated in place and a new key's row is appended; each changed row is the
  next round's delta;
- held rows carry a change count as their last column (for `widen`), which no join reads.

Every loop is bounded. The device still refuses a recursive component over a lattice it has no
fold for: `shortlex_min` compares texts. Seeds alone fold as the store takes their rows.

## Checked

The coverage gate's lattice probes cover `flat`, `max`, `shortlex_min` over symbols, a later
stratum reading a lattice, a key improved after its first value, and `max` over a cycle widened
to its top. Each one runs natively and on the device, and each gives SQLite's rows.

| public_api_triples, strict | CPU | wall | peak |
|---|---|---|---|
| native | 42.6 s | 16.0 s | 1.63 GB |
| Metal (its `shortlex_min` hierarchy on the CPU) | 49.4 s | 24.7 s | 1.73 GB |

Load was 36–69. The hierarchy has 26,527 rows, each the shortlex-least route over its in-edges.
