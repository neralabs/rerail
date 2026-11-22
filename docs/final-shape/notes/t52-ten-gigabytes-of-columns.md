# t52. Ten gigabytes of columns: where they come from, what exists, and the plan

## Intro

On `all` (3,089 Python files), t50's run of the reference graph lets go of 10.1 GB of relation columns. The user,
2026-11-22: "don't you find 10GB to be excessive? there ARE vastly more efficient ways of storing all this,
including by destructuring 64 bit things that have been mushed together into their actual fields. mldb has many
ideas"; then "where all of the strangely named things are coming from", "what is the reason for the copies and why
we are making them", "what is the rr_narrow_ mechanism", "what mechanisms we have in place to record aliases/copies
and rewrite them to the original facts", "where the 298M missing ones come from"; then "document the
investigation, survey the sibling worktrees for existing fixes / work, and turn it into a plan".

It is two problems, not one:

- **Volume.** About a third of the bytes are rows written twice: an export's rows copied out of the chunks into a
  relation of their own (2.3 GB), and the narrow twins of the relations annotations reach, each evaluated a second
  time beside its original (1.3 GB or more).
- **Encoding.** Every column is 8 bytes a value. The blocks' own encoder, per range of 4,096 rows, takes the same
  10.1 GB to 4.2 GB as written and 2.15 GB with each range sorted. Node ids split into their fields would go further.

Most of the volume fix already exists on sibling branches (decision 0575 and `3d8f990a26`), and so do the first
compact columns (0494 step 7a-7b, key runs). The export copy has no fix anywhere. The plan (last section) merges
what exists, removes the export copy, and then makes the compact layout the blocks' one layout.

## How it was measured

`RERAIL_WIDTH_CENSUS` (commits 39de0fce60, 5e4a01c8f7, 322a93c3bd), a measuring switch, off by default:

- **Each block** (`facts::rows::Held`) is read once as it is let go: each column's bytes as written, its least and
  greatest value, and the span it was made in.
- **Each relation** (`InRel`, not a view) is counted by its table name as it is let go. A relation with no name is
  counted by the span of its first block.
- **`=ranged`** also encodes each column per range of 4,096 rows with `rerail_blocks::col::choose`, as written and
  sorted, whole or split at the granule. That doubles the run's time.

Runs: the stage harness (`reference_stages_timed`) on `all`, a profiling build, on an M1 Max farm host. Rows and
findings were unchanged throughout.

## What the 10.1 GB is

| Bytes | Made in | What |
|---|---|---|
| 2,264 MB | `export to cross/hold the export` (102 exports) | each chunk's export rows written again into the export's relation |
| 833 MB | `an input's frames` | base relations loaded from the store: not a copy |
| 494 MB | `rr_narrow_look`/`rr_narrow_walk` and its axis | narrow twin |
| 189 MB | `fl…_rf_bind`/`walk` | |
| 4 × 187 MB | the four `xd_…` exports, in the chunks | demand exports (two of them narrow twins' pushed joins) |
| 100 + 100, 91 + 91, 4 × 75, 96, 89, 66 + 66 MB | `rf_named`, `rf_bind`, `nameref`, `wstart`, `rf_in`, `rf_fcond`, `rf_expr` | an original beside its narrow twin, each pair the same size |
| the rest | the chunks' strata | |

By width: of the 10.1 GB, 3.9 GB of columns would fit 4 bytes a value and 5.9 GB need 8. Narrowed whole, a column
to the bytes it needs, as an offset from its least value, or split at bit 32, the total is 7.9 GB each time.
Whole-column narrowing is not the lever; per-range encoding is (4.2 GB, 2.15 GB sorted).

## The five questions

### 1. The names

Each prefix is one program pass's:

| Name | Pass | What it holds |
|---|---|---|
| `fl<h>_<head>` | `PushLocalJoins` (`typing/file_local/passes/push.rs:348`) | a cross-file rule's file-local part, joined in each chunk; `<h>` is FNV-1a of the helper's rule text (`helper_name`, `file_local.rs:362`) |
| `fz<h>_…` | `FactorizeExports` (`passes/factorize.rs:433`, decision 0099) | an export handed on as its factors, where its join meets at a projected-away variable |
| `xd_…` | `DemandExports` (`export_demand/on_version.rs:665`) | an export cut to its cross-file readers' demand |
| `rr_narrow_<rel>` | hiding's `keep_narrow` (decision 0165, 2026-11-20) | question 3 |
| `main.<rel>` | `input_of` (`native/setfix.rs:6361`) | the cross part's name for an export: the same blocks as the export's relation (`rows.clone()`, shared) |

`fl1727681243_rf_bind` is `rf_bind`'s `rf_head(i), look(i, x, s), !rf_forward(i, _, _)` pushed into the chunks.
`fl328227323_rr_narrow_rf_bind` and `fl2977704093_rr_narrow_rf_bind` are the narrow twin's pushes: a push is named
from its head and its whole body, so the twin's bound comparison gives it another name and another relation.

### 2. Why the copies are made

- **The export's second write** (`Exports::add`, `logic/plan/corpus/exports.rs:144`). A chunk derives an export's
  rows in its own relation. Each live row is then written again into the export's relation, kept once across chunks
  by a hash set of the rows written, and the chunk's relation is let go. The reason is the dedup: two files can make
  one row (a value document's). By decision 0353 this is a copy, and it is not in 0353's list.
- **The narrow twins** are a second evaluation of an original's rules, not a copy of its rows (question 3).
- **The `fl` variants** follow the twins: each of an original and its twin pushes its own join.

### 3. The `rr_narrow_` mechanism

Python's annotations (string type expressions) are guest documents keyed `ANNOTATION_KEYS..HIDDEN_KEYS` (0165 §2). A
rule is bounded below `ANNOTATION_KEYS`, so it never reads an annotation as code. A relation an annotation reaches is
copied for it (`rr_in_annotation_…`, 0058 §2). `FoldOwn` (`preparation/combining.rs:522`) folds those copies back
into their originals and widens the originals' bounds to `HIDDEN_KEYS`. A pack's programs share the library's
relations (0089), so the widening reached other programs' rules: RUF060 and C408 found what Ruff does not. Since
2026-11-20, every reader but an original's own reads `rr_narrow_<orig>`: the original's rules again under the narrow
bound, each reading the others' narrow copies. 0165 measured 88 such copies, wall 72-74 s to 83-85 s, peak 5.8 to
6.4 GB.

A twin is made of rules, not of a filter of its original, because a wide row about a code node can be derived
through an annotation's node, so the wide rows cut by document key are not the narrow rows in general. On data with
no annotation documents the two are the same, row for row; that is the doubling the user named ("check for them and
read the original when there are none").

### 4. What records aliases and copies today

- `MergeIdentical` and `DropRepeated` (`simplify`'s congruence), in `same_relations` (`combining.rs:469`) after
  hiding and combining, and after `FoldOwn`: relations whose rules are the same up to renaming become one. A twin
  differs from its original by a bound, and so do their pushed joins; neither merges.
- `PushLocalJoins` reads again a helper of the same rule text within one round.
- `compose`'s `aliases` maps logical SQL table names to physical ones: not a dedup.
- Nothing says two relations are equal on the data (a twin on a chunk with no annotation documents), and nothing
  reads an export where its chunks' rows lie.

### 5. The 298 M unnamed values

All 296.8 M (108.8 M rows) are the 102 export relations `Exports::source` makes (`exports.rs:323`, an
`InRel::held` with no name): the export's second write. `main.<export>` shares their blocks, so the by-name census
counts those rows twice; the 1,277 M values by name overstate what is held. The per-block total (10.1 GB) counts
each block once.

## What the sibling branches already have

From every branch's commits not on this one since 2026-11-08, and the worktrees' uncommitted changes to the same
files:

| Work | Where | On the hub (`final-shape-serverplan-quality`) |
|---|---|---|
| 0165: a narrow copy within one document is its original's rows under the bound, a filter (`10050170bb`, `15e8c73191`) | hub, cost-classifier, bend, benchmarks | yes |
| 0575 step 1: a narrow copy is its original less the annotations' change, rechecked (`combining::Diffed`, `2ea0e8170d`, `c82460adf4`) | hub and the three below | yes |
| 0575 steps 5, 8: a pushed join named by its join, not its head; a bound's variable handed on, so a twin and its original push one join (`3681d927ed`) | `investigate-bend-vs-datalog`, `benchmarks-on-integration`, `cost-classifier-metal-rf` | no |
| 0575 step 6: a diffed copy read in place, not built (`combining::Unnarrow`, `dbdc4038d6`, `3c3a041c9b`) | same three | no |
| 0575 step 7: a rule beside its annotation twin reads only the copies' deltas (`2db477b6bc`, `cef0165efd`) | same three | no |
| 0165: a filter's copy holds none of an annotation's rows (`eef396c894`) | benchmarks, cost-classifier | no |
| 0165 §3: a local original read bounded is read in place, not copied (`3d8f990a26`; `walk`, `nameref`, `attr_of`) | `Native-and-GPU-strata` only | no |
| 0597 step 2, rung 2.4: what a context holds is never copied, widened or narrowed (`4798383568`) | `cost-classifier-metal-rf` only | no |
| 0494 step 7a: a 4-byte column dictionary-coded, or under key runs (a document's high bits a run, each row the low 32), read through `ColDir::get`, CLIF `col_get` and the kernels' `relation.h` (`4265ccd1d0`, `78cecd774f`) | hub and all three | yes |
| 0494 step 7b, 7c: an axis's solve as two 4-byte columns under key runs, 24 MB where 49 MB; a batch's axes share their solves (`fe43e36029`, `1c87fb5754`) | hub and all three | yes |
| The export's sets in block maps (`IdTable`, `BlockHash`, `Vector`), not heap sets | `final-rerail-shape-phased-facts`, uncommitted (532 files) | no |
| The export's rows read where they lie in the chunks | nowhere | no |
| A per-range encoding of every relation's columns, node ids as their fields | nowhere (0371's packed exports were a second layout, superseded) | no |

0575 measured its steps on 901 files: 832 G to 761 G instructions, unprofiled; 174 of 229 copies diffed, 87 of those
read in place. The mixed points-to components stay re-derived. The reference program's twins (`look`, `walk`,
`rf_*`) are not points-to, so they should be among the diffed or the filtered.

`cost-classifier-metal-rf` holds every 0575 commit and lacks 5 of this branch's commits. The hub holds 0575 step 1
and 0494 step 7. `3d8f990a26` is on `Native-and-GPU-strata` alone.

## The plan

In order. Each step is measured with the census on `all`, against this note's numbers, with rows and findings
unchanged and CPU noted.

1. **Merge `cost-classifier-metal-rf`.** It brings the hub's 0575 step 1 and 0494 step 7, and 0575's steps 5-8, the
   filter copies and 0597's contexts. Merged by docs/merging.md: both sides' decisions read first, resolved by
   invariant. The likely conflict is this branch's kept-facts and block work (`kept_facts.rs`, `retain.rs`,
   `facts/rows.rs`) against theirs in the same files. Then the conflicted files' tests and one census run.
2. **Port `3d8f990a26`** (a local original read bounded, in place) from `Native-and-GPU-strata`, beside 0575's
   one-document filter, which it extends: an original whose every row is one document's is read under the narrow
   bound wherever its reader bounds a node narrow. Measured: the twins of `walk`, `nameref` and `look` gone from the
   census.
3. **Read the reference program's remaining twins.** With `RERAIL_NARROW_DIFF_DEBUG`, each twin left in the census
   says why it is not diffed or read in place. Each reason is fixed in the pass, not by a constant: the goal is no
   `rr_narrow_` relation in the census of a run with no annotation documents, and on `all` only the annotations'
   change computed.
4. **An export is read where its chunks' rows lie.** `Exports::add` appends a chunk's part to the export's rows
   (`Rows::append_part`) under the export's life, in place of writing its rows again. A row given twice is marked
   dead in the export's dead bits, found by a set of row ids (an `IdTable` in a block, as `phased-facts` has it), not
   by a copy. `append_own` likewise takes the memo's rows as a part. 0353 lists this copy as found, with this step as
   its removal. Measured: `hold the export` at 0 MB; the cross part's peak and CPU noted.
5. **Decision 0605: one compact layout of a relation's columns.** Written before it is built, citing MLDB's frozen
   columns (`plugins/tabular/frozen_column.cc`: per chunk, the smallest of direct, table, sparse table and integer
   table, by exact size; `FrozenIntegerTable`'s slope, offset and bit-packed residual), `rerail_blocks::col` and
   0494 step 7a. The shape: a block's column is sealed per range of rows as the smallest of constant, runs, line,
   bit-packed offsets and dictionary. A node id column is held as its fields: the document a run (0494's key runs),
   the slot a few bits, the index a line or bit-packed. A column may be stored relative to another (column 0's
   document). It is the blocks' one layout, read where it lies by `ColDir::get`, CLIF's `col_get`/`rows_get` and the
   kernels' `relation.h`, written once; no second, packed layout (0371's mistake).
6. **Build 0605 in its steps:** key runs for every column whose rows lie in one document a run (every chunk
   relation), then per-range bit-packing, then dictionaries for atoms and kinds. Each step's reads are tested through
   every reader (host, JIT, kernels' generated header), and measured by the census.

Steps 1-3 cut the twins' 1.3 GB or more and their CPU. Step 4 cuts 2.3 GB. Step 5-6's target is the per-range
figures: 4.2 GB as written, 2.15 GB sorted, before fields are split.
