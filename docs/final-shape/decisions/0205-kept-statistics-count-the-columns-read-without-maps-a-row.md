# 0205: The kept statistics count only the columns read, and a column's values without a map entry a row

Status: built (2026-11-14), `plan/statkeep.rs` (`RelStats::rows_of`, `documents_at`) and
`plan/corpus/keptstats.rs` (`Keeper::begin`'s columns, `from`). It narrows decision 0096 §3, statistics kept
by document: what is kept is the same for the columns the planner reads, and a column no rule reads is no longer
kept.

## Intro

A memoised run keeps its planner statistics by file (decision 0096 §3). When a file's content is new to the
keeper, its rows are counted in. When it changes, its old rows are counted out and its new ones in. For each
column, the keeper keeps the rows, the distinct values (exactly while few, else as a sketch), the sum of squared
counts and the same sum within documents.

The O125 replay's profile showed this count as the largest part of the local phase that is not a join. In the cold
run the keeper took 0.234 s to count 4.6 M values, 51 ns and 121 bytes allocated a value, which is half the CPU of
the whole cross-file part. In step 08, an edit of six files, it took 0.119 s, about 40% of that step's local part.
By the rule that planning never costs more than naive execution, statistics are planning, so this was the finding.

Split by part (cold, profiled):

| part | CPU | allocated |
|---|---|---|
| the relations' statistics (`RelStats::rows_of`) | 0.149 s | 363 MB |
| a session's base columns, counted by file (`Counted::patch`) | 0.051 s | 134 MB |
| a session's exports, counted by file | 0.030 s | 52 MB |
| holders, the own counts | under 0.001 s | |

## What changed

**1. A base relation is counted at the columns its rules read.** The keeper used to count every column
`input_columns` names. The run reads a base relation's statistics only at its held columns (`r.statistics(&|c|
held.contains(c))`), and states every other column as a key: `(c, n, n, n, n)`. The keeper now makes a base
relation's statistics with the held columns only, and `from` maps each one to the row's column. A row's document
is still read from the row's first column.

```text
// base relation r(n, kind, start, end, line); the rules read n and kind
before:  RelStats [n, kind, start, end, line]   five columns counted, three never read
after:   RelStats [n, kind], from [0, 1]        two counted; start, end, line read as keys, as they were
```

- **Why it stays right:** the statistics name their columns, and the blob keeps the names. A run whose rules read
  other columns asks for other names, and the keeper counts that relation from scratch (as for a relation new to
  it). `begin` compares the names, so nothing counted at two columns is taken for three.
- **Symbols (decision 0170):** `map_symbols` maps a column's exact map between this process's ids and the store's.
  It reads a base relation's symbol columns by name now (`Syms::columns_named`), since its statistics no longer
  have the row's width.

**2. A column is counted without a map entry a row.** `rows_of` built a map for each document and one for each
value, an entry a row in each, and then merged them. Now each row's (document, value, whether its first column is
a document key) is listed once. If the list is already in order, as node columns mostly are, each
(document, value) is a run and no map is made. Otherwise the list is counted in one map, which is kept across the
columns. Values are summed across documents only when the rows name more than one document, and a counted-in
column with more distinct values than the exact map's bound gives the map up without filling it.

A sort in place of the map was tried first. It was slower on the columns that are not in order (names, kinds):
0.120 s against the map's 0.095 s, cold.

## Measured

O125 replay, `--profile profiling` build, steps 06–08, before against after, three alternating rounds; "same as
full" in every run. The machine's load averaged 80–107, so the seconds are rough; instructions retired are not.

**Instructions retired, cold local part** (G):

| | round 1 | 2 | 3 |
|---|---|---|---|
| before | 14.03 | 13.67 | 13.66 |
| after | 13.30 | 13.25 | 13.27 |

Mean 13.79 → 13.27 G (−0.52 G, −3.8%). The cross part is unchanged (9.22 G both).

**The keeper's parts** (profiled CPU):

| | cold: relations | base columns | exports | step 08: relations |
|---|---|---|---|---|
| before | 0.149 s | 0.051 s | 0.030 s | 0.079 s |
| map kept across columns, no sort when in order | 0.095 s | 0.046 s | 0.026 s | 0.050 s |
| and only the columns read | 0.082 s | 0.041 s | 0.023 s | 0.043 s |

The relations' statistics cost 45% less (cold) and 46% less (step 08). Their allocation did not fall (361 against
363 MB), because the list and the map are allocated for each call. Allocation was not where the time went.

## Tried and dropped

- **The counted exports' table keeping each row's hash beside its id** (so growing it does not gather rows to
  hash again, and a probe compares hashes first), with the file's rows reserved up front. No clear gain (cold
  0.051 → 0.041 s, step 08 0.023 → 0.029 s, under load), and the table is resident across a session's runs, 12
  bytes more a row. Reverted.

## Tests

- `one_pass_counts_as_the_four_passes_did` (`statkeep.rs`) now also covers paired relations (the pairs sketch),
  one document's rows in order (no map), the same reversed (the map), and a column past the exact map's bound. It
  fails when the sum across documents, the order test, or the pairs of document-key rows is broken.
- `a_base_relation_is_counted_at_the_columns_read` (`keptstats.rs`): counted at two of three columns, the
  statistics of those two are the ones counted at all three. Counted at three next, the relation is counted from
  scratch. It fails when `from` is ignored or the reset is removed.
- logic: statkeep 4, exports 6, keptstats 2, corpus tests 5, memo 6. Engine: plan 55, session 25, corpus 114,
  waves 53.

## Open

- The counted exports and base columns (`Counted::patch`, 0.06 s cold together) cost a hash probe a row, which
  is inherent to a multiset of rows held by files.
- An edited file's changed relations are counted out and in whole (`changed_only` works by relation, not by row).
  Counting only the rows that differ would need each (document, value) count from before the edit, which costs
  the same pass.
