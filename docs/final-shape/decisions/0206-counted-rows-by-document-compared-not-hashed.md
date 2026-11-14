# 0206: A session's counted rows are kept by the document they name, compared, not hashed a row at a time

Status: built (2026-11-14), `plan/corpus/exports.rs` (`Counted::patch_own`, `CountedRel::own`, `rows_once`) and
`plan/corpus/keptstats.rs` (`by_document`). It narrows decision 0096 step 3: what the counted exports and base
columns answer is unchanged, but how they hold the rows is new. Switch: `RERAIL_COUNT_OWN=off` counts every row
by hash, as before.

## Intro

A session counts its exports and its base relations' held columns by file (0096 step 3). A row is held while one
file holds it, so the rows form a multiset: each row kept once, with how many files hold it. Every row went through
a hash table: found (counted once more) or inserted. In O125's cold run that cost 0.088 s (0.051 s base columns,
0.037 s exports), after decision 0205 had cut the statistics beside it.

Most of those rows can't need the table. A file's entry holds rows of the document they name (their location
column, or a base row's first column):

- **its own document's rows,** which no other file's entry holds as its own;
- **a guest's rows** (decision 0055), which every file holding that guest gives alike.

Measured over the replay: 0.98 M rows of the file's own document against 2.66 M of other documents, in the same
ratio for nearly every relation. So a lever for the own rows alone did not resolve (13.43 against 13.44 G); the
guests' rows were the bulk.

## What changed

A file's rows are grouped by the document they name (`by_document`; a document's rows mostly come together, so its
group is looked up once a run of them). Each group is counted as one (`patch_own`):

| | the group's document has no rows kept | has the same rows kept | has other rows kept |
|---|---|---|---|
| counted in | kept as they come | counted once more (`times`) | counted as shared rows (by hash) |
| counted out | counted as shared rows | counted once less; dropped at none | counted as shared rows |

A comparison of the group with the rows kept replaces a hash probe a row. Rows that don't match are counted as
before, so nothing assumes a guest's rows are alike, only that they mostly are.

What it answers:

- `source` (an export's rows, each once): the shared rows, less those that are also a document's kept rows (only
  shared rows naming a document with kept rows are checked), then each document's rows.
- `columns_each` (a base relation's rows, as many times as files hold them): the shared rows by their counts, then
  each document's rows `times` times.
- `source_without` (the cone of requests, rare): both sides with every row in the hash table, as before.

```text
file a.py (doc 3) holds guest 9;  file b.py (doc 4) holds guest 9
own[3] = a's rows of doc 3, times 1
own[4] = b's rows of doc 4, times 1
own[9] = guest 9's rows, times 2     (b's equal to a's: one comparison, no hashing)
edit a.py: own[3] replaced, own[9] times 2 -> 1 -> 2 (a's guest rows equal again)
```

## Measured

O125 replay, `--profile profiling` build, steps 06–08, `RERAIL_COUNT_OWN=off` against on, three alternating
rounds; "same as full" in every run. Load averaged 90.

**Instructions retired, cold local part** (G): off 13.39, 13.59, 13.65; on 12.90, 13.27, 12.95. Mean 13.54 →
13.04 G (−0.50 G, −3.7%). The cross part is unchanged (9.29 against 9.28 G).

**The counted rows** (profiled CPU):

| | cold: exports | cold: base columns | step 08: exports | step 08: base columns |
|---|---|---|---|---|
| by hash | 0.037 s | 0.051 s | 0.012 s | 0.020 s |
| by document | 0.007 s | 0.004 s | 0.003 s | 0.002 s |

Peak RSS stayed within its noise (784–892 MB in both modes). The rows are held flat in their groups, as many
values as the columns held them.

## The second question: count an edited file's rows that changed only

Measured over the replay's edits (`RERAIL_STATS_KEEP_TRACE`): of 1.01 M new rows in the changed relations of
edited files, 84% were also among the old rows, but only 40% at a shared prefix or suffix. The rest moved: `base:node`
keeps 15 k of 113 k rows, others more than 90%.

So a count of the changed rows alone would first have to find them, by hashing or sorting both sides, which costs
about what counting them does. And the statistics are sums within documents (the squared counts of each value in a
document, its distinct values): taking out some of a document's rows needs each touched value's count before the
edit, which is one more pass over the document's rows. On step 08 the relations' count is now 0.026 s of a 0.7 s
step. At best it would save part of that, so it is not built. The counted rows above, the edit's other half, are
now a comparison and a copy.

## Tests

- `a_files_own_rows_count_as_one_and_answer_as_rows`: own rows, shared rows and a shared row that is also a
  document's, against the same rows counted by hash, through `source`, `columns_each` and `source_without`. Rows
  that do not match are counted as shared, in either order. It fails without the dedup in `source`, or when a count
  out does not compare.
- `a_guests_rows_given_alike_by_two_files_count_twice`: held while either file holds them. It fails when `times`
  is not counted.
- logic: exports 8, statkeep 4, keptstats 2, corpus tests 5, memo 6. Engine: plan, session, corpus, waves.
