# 0051: All rules run together over cached tapes; a cross-file rule's file-local part runs in the chunks

Status: accepted (2026-11-06; built and measured)

## Intro

A corpus run has a fixed cost before any rule runs: read and parse each file, build its facts, and
write the batch. Over 200 files that is about 4.9e9 instructions. Running the 100 converted rules one
at a time pays it 100 times: 88% of an s200 sweep (notes `t10-corpus-profile-opportunities.md`, §2a).

There were two causes:

- each rule was its own run;
- each run deleted its store and parsed every file again, so nothing was ever cached.

Running all the rules at once exposed a third problem: memory. A corpus run is split into a part
run per chunk of files and a part run once over all of them (the cross-file part). The split is
decided per relation. When the rules are combined, `violation` is a single relation, so one
cross-file rule made every rule's violation cross-file. Each of those rules then read its base
relations whole, and every chunk's `within`, `node` and `child` rows were kept until the end. On
1,000 files that was 629 MB of values and a 1.5 GB peak. The full corpus didn't finish in 280 s.

## Decision

### 1. One run for every rule

`corpus::combine` parses each program and merges them:

- a program's own relations get the prefix `p<i>_`;
- the inputs and `violation` are shared;
- a kind set is named by its kinds (`ks<j>`), so two programs' sets with the same kinds are one.

The simplifier then merges relations that came out identical. `corpus::run_all` runs the result;
`RERAIL_MEASURE_ALL` measures every pack rule this way. A finding's first column still names its
rule, so per-rule counts come from one `violation.csv`.

### 2. Tapes kept between runs

`core::build_cached` keeps each file's tape in a content store of its own. The key is the source's
content (SHA-256), the grammar's version and the tape format version. On a hit, it decodes the
stored tape and restores the document. On a miss, it parses, stores the tape, then restores.

The corpus runner opens this store once per run:

- by default, in the temporary directory;
- `RERAIL_TAPE_CACHE` names another path;
- `RERAIL_TAPE_CACHE=off` parses every file.

The run's own store is still made fresh each time. Only tapes, which depend on nothing but the file
and the grammar, are kept.

### 3. A cross-file rule's file-local part is pushed into the chunks

`file_local::push_local_joins` runs on the lowered program before the split. For each rule whose
head is not file-local, it takes:

- the rule's positive reads of base and file-local relations, grouped into components by shared
  variables;
- the comparisons whose variables a component binds;
- the negated reads of base and file-local relations whose variables a component binds.

A component that reads a base relation becomes a new relation, `fl<k>_<head>`. Its columns are the
component's variables that the rest of the rule uses, and the rule reads it in place of the
component:

```text
// Before: runs once over all files and holds within, node and text for every file.
m($n) :- within($a, $n), node(_, $a, 1025, _, _), text($a, 522), !cross_file_test($a).
// After: each chunk derives fl_m and exports the few rows that pass.
fl_m($a, $n) :- within($a, $n), node(_, $a, 1025, _, _), text($a, 522).
m($n) :- fl_m($a, $n), !cross_file_test($a).
```

Some components are left as they are:

- one atom of distinct variables, since it filters nothing;
- one that reads no base relation;
- one whose variables the rest of the rule doesn't use.

The split checks each new relation. If it isn't file-local (a negation read through no location
column, for example), the rule first retries with only its negated base reads, then is left as it
was.

### 4. The tape encoder asks the cursor once per node

`encode_tree` used to call `ts_tree_cursor_current_field_id` and then
`ts_tree_cursor_current_status` for every node, and both walk the cursor's stack. The status call's
field is the same except at an extra node: `field_id` stops there, while status walks past it. So
the encoder takes the field from status and treats an extra node's field as none.

## What the prior work says

- "The shared relation was scoped to the union of candidate files, i.e. the whole corpus." The
  spike rejected batch sharing for that reason, and later found the rejection wrong because the
  scope was inflated 7.27x (design index, D S149 L6264-6320; S176). Holding base relations for
  the cross-file part had the same inflation: rows needed for a few files were kept for every
  file. §3 scopes them to the chunk that makes them.
- "Holding the trees in memory costs 23.7x the source, so at 1.25 GB the warm matcher cannot
  exist" (history index, S184, DH L3705-3721). Keeping whole base relations was a slow version of
  the same wall. The 1 GB budget holds only if what survives a chunk is small.
- A comparable fact system's findings include "per-file fixed cost, repeated processing"
  (design index §17, LA 6 L252-319, F3). §1 and §2 remove both for corpus runs: one run, and a
  parse per file per grammar version.
- "Restore is 20-26x faster than parse" (history index, TAG). That is why a cached tape is worth
  keeping: on the full corpus, warm parse is 4.3 s against 26.9 s cold.
- The existence semi-join of the spike's outlier pass (design index, D S054 L3349-3382) is the
  same idea as §3 and decision 0048: the cheap, local part of a join runs first, and only its
  survivors meet the expensive part.

## Measured (2026-11-06)

Instructions retired; CPU is user + sys. Machine load 17–55.

| Corpus, 100 rules | Instructions | CPU | Peak RSS | Findings |
|---|---|---|---|---|
| s200, 100 separate runs | 553e9 | | 182 MB (the largest) | 4 |
| s200, one run, before §3 | 57.3e9 | 4.7 s | 472 MB | 4 |
| s1000, one run, before §3 (warm) | 165e9 | 14.5 s | 1,529 MB | 24 |
| s1000, one run (warm) | 115e9 | 9.7 s | 582 MB | 24 |
| full (9,010 files), cold | 970e9 | 93 s | 695 MB | 252 |
| full, warm | 797e9 | 60 s | 739 MB | 252 |

- On the full corpus, 288 rules are rewritten by §3. The cross-file part holds no base relation and
  takes 0.01 s.
- The findings match: callable-constructors alone finds 252 on the full corpus. The replay's totals
  are unchanged (3988 273 266 0 with hand-written rules, 3988 175 169 0 converted).
- The full corpus's tapes take 103 MB.
- §4: parity with native tree-sitter holds on s1000's 997 Python files (3.1M nodes).
