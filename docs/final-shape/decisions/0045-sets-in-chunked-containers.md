# 0045: Node sets are chunked containers: arrays, bitmaps and runs

Status: accepted (user agreed, 2026-11-06); steps 1-2 done (`space::NodeSet`), step 3 done for kind postings

## Intro

Most of the engine's work is set operations over nodes: "the calls", "the nodes inside a
function", "the candidates in these files", "the handlers with no raise". 0024 made every such
set one value over the wave's node space: a dense bitmap, a sorted list, or ranges, picked once
for the whole set.

Roaring bitmaps do better by choosing per **chunk** instead. The id space is cut into chunks of
2^16 ids. Each chunk that holds any member gets the container that suits it:

```
set "calls" over 3 documents
  chunk (d=7, 0)   array  [12, 40, 41, 977]       // few members: sorted 16-bit values
  chunk (d=7, 1)   bitmap 8 KB                    // many members: one bit per id
  chunk (d=9, 0)   runs   [(100, 2400)]           // a subtree: one interval
  (d=8 holds none: no chunk, no work)
```

Three things make this a good fit here:

- **Our ids are already split this way.** A node is `d << 32 | preorder`: the document is the high
  half. A chunk is a (document, 2^16 slice) pair.
- **A subtree is a run.** `within(n, a)` is the interval `a+1 … a+size`, so "below any of these
  nodes" is a union of runs, and intersecting it with a kind's members is a range filter, not a
  join.
- **Work follows the members.** An operation visits only chunks present in its inputs, so a rare
  kind over a large wave costs its few chunks, not the whole space.

## What there is today

- `space::NodeSet` (`space.rs:1-153`): a dense `u64` bitmap over the wave's concatenated space,
  with union, intersect, difference, `within_files`, `fill` and `clear`. Sparse sets and
  per-file ranges are planned in 0024 §1 and not built (`space.rs:8-9`).
- Candidates (`prefilter/candidates.rs:4-24`): `All`, `Sparse(Vec<u32>)` or `Bitmap(Vec<u64>)`,
  chosen once per whole set (sparse when `4·len ≤ 8·words`).
- Postings (`content_store/postings.rs:17-23`): delta-varint preorder lists per (kind, document).
- Metal set steps (`metal_set_steps.rs`, its `.metal`): clear, fill, copy, or, and, and-not,
  within, one dispatch, one threadgroup per file over dense words.
- Label passes (`transducer/packed/metal_label_passes.rs`): one bitset per class over the nodes.
- In SQL, unary node relations are tables or vtabs, and difference is `NOT EXISTS`/`EXCEPT`.

## Decision

### 1. One set type, three containers

- **Directory:** sorted `chunk → (kind, offset)`, one entry per non-empty chunk. A chunk is 2^16
  nodes of the space, across files, as in Roaring (it was one file's 2^16 slice; see "Chunks of
  the space"). Files are ranges of the space: per-file questions take the space.
- **Containers** (chunk = 2^16 ids, values are the low 16 bits):
  - **array:** sorted `u16`, while the count is at most 4,096 (it is then no bigger than a
    bitmap);
  - **bitmap:** 1,024 `u64` words (8 KB);
  - **runs:** sorted `(start, length)` pairs, when they are smaller than either.
- The container is chosen when the chunk is made, by size (Roaring's rule, re-checked by the
  measurement below), and may change after an operation.
- A whole document is one directory flag, not 2^16-bit chunks: "every node of d".
- This replaces `NodeSet`, `Roots` and 0024 §1's three forms with one type. The global ids are
  unchanged (0044 §2).

### 2. Operations by container pair

| Pair | Intersection | Union / difference |
|---|---|---|
| array, array | galloping merge | merge |
| array, bitmap | probe each value | set or clear bits |
| bitmap, bitmap | word `&` (1,024 words) | word `|` / `&~` |
| runs, array | binary search per value | merge |
| runs, bitmap | mask the words the runs cover | fill or clear ranges |
| runs, runs | interval merge | interval merge |

Only chunks present in the inputs are visited: an intersection walks the shorter directory.

### 3. Tree axes produce runs

- `within(n, a)` for a set of `a`s is the union of their subtree runs, built from `tape`'s
  `(pre, size)`. Nested subtrees merge.
- `inside_<set>` and `nearest` results, label passes and kind masks come out as containers per
  chunk: a label pass writes a bitmap for a dense chunk and an array for a sparse one.
- Text tests (0042) give sets of symbol ids. The same containers hold them, over symbol ids.

### 4. On the device

- One threadgroup per present chunk (a bitmap chunk is 8 KB: it fits threadgroup memory). The
  directory is the work list, so empty chunks cost nothing.
- Arrays and runs are expanded to a bitmap in threadgroup memory when combined with one, and
  compacted back by a prefix scan when the result is small.
- The dispatch shape stays 0024's and 0027's: one persistent dispatch per step, a bounded work
  queue, no unbounded waits.

### 5. In SQL and in the store

- **Unary node relations** (kind sets, candidate roots, label results) are stored as container
  blobs per document, and read through the vtab as rows when SQL needs rows.
- **Binary relations grouped by their first column** become posting lists of containers. This is
  what the kind postings are, with containers instead of delta-varint lists.
- **Stored containers are per document**: a document's segment holds its own 2^16 slices, so
  0044's per-document derivations replace a document's segment and nothing else. In memory a
  set's chunks are the space's; a segment is moved to the document's base when read (arrays and
  runs by adding it, bitmaps by a shift) and cut out of the space's chunks when written.

## Evidence: the census (step 1)

`facts::set_census` (ignored test), on the 9,010-file repository at `2b1504ad`: 18,423,804 nodes in
the engine's id space. 120 sets: every kind, and "inside a node of kind K" for `function_definition`,
`class_definition`, `call`, `with_statement`, `except_clause` and `lambda`. Sizes, not timings
(23.4 s CPU with parsing, 232 MB peak; load 108, which does not affect sizes).

| Form | All 120 sets |
|---|---|
| dense bitmap over the whole space (0024 §1 dense) | 276 MB |
| dense bitmap per document, documents with members only | 154 MB |
| sorted u32 lists | 201 MB |
| **containers (array, bitmap or runs per chunk)** | **42 MB** |

- **A chunk is almost always a document.** The largest file has 145,555 nodes; 5 files of 9,010
  are over one chunk.
- **Kind sets are arrays.** 393,314 array chunks, 242 bitmap chunks (mostly `identifier`), and
  almost no runs: a kind's members are rarely next to each other.
- **Inside-sets are runs.** "Inside a function" is 15.6 M members in 135 K runs: 0.6 MB as runs,
  62 MB as a u32 list (100x). "Inside a class": 7.3 M members, 30 K runs, 0.18 MB.
- **Most (set, document) pairs are empty: 61%.** A chunked operation skips them; a dense one does
  not.
- So the common operation, a kind set inside a scope set, is an array against runs: a binary
  search per member over a few runs.

## Evidence: operations and memory (step 2)

`facts::set_census::set_operations` and `population` (ignored tests), on the same repository. Both
forms were tuned before comparing:

- **dense:** `clear` had allocated a mask the size of the whole space for every file, so `within`
  cost 1.25 s. It now clears in place: 0.29 ms.
- **containers:** one directory over two shared buffers, not one allocation per chunk. Bitmaps
  hold the words up to their last member. Bitmap against bitmap or runs is one fused pass that
  writes the result once, and result buffers are reserved up front.

Every result is checked equal between the forms. Thread CPU per operation over the whole
repository (18.4 M nodes), mean of 5. The load was 80-128, so treat the times as ratios, not
absolutes.

| Operation | Result | Dense ms | Containers ms |
|---|---|---|---|
| call ∩ inside function | 999,607 | 0.12 | 1.56 |
| the same, the inside-set as bitmaps | 999,607 | 0.13 | 0.38 |
| identifier ∩ inside class | 2,461,311 | 0.14 | 0.83 |
| the same, the inside-set as bitmaps | 2,461,311 | 0.13 | 0.40 |
| raise ∩ inside except | 2,002 | 0.12 | 0.08 |
| await ∩ inside function | 24,312 | 0.13 | 0.19 |
| decorator ∩ call | 0 | 0.12 | 0.19 |
| call within files of raise | 675,330 | 0.20 | 0.57 |
| clone call | 1,056,492 | 0.04 | 0.05 |

**Memory.** 117 sets (every kind and three inside-sets):

| Scope | Dense | Containers |
|---|---|---|
| whole repository | 269.4 MB | 48.3 MB |
| largest wave of 2^18 nodes | 3.8 MB | 0.7 MB |

The prefilter's set programs on the push path, measured on a 219-file subtree with all 59
policies that lower (`data_layer_bench`, `sets.*` counters):

| | Sets held | Containers | Dense would be |
|---|---|---|---|
| before: every value kept to the program's end | 2,648 | 60 MB | 235 MB |
| now: each value freed after its last read | 378 at most | 6.2 MB | 33.6 MB |

What this shows:

- **Memory is where containers pay.** Five to six times smaller for the same sets. With values
  freed after their last read, the set programs' peak goes from 235 MB (dense, every value kept)
  to 6.2 MB, and the footprint rise at set evaluation is gone. The run's peak is now elsewhere:
  labels (823 MB high-water on that subtree) over a 305 MB base after lowering 407 clauses.
- **Runs are the CPU cost.** "Inside a function" is 135 K runs, and an operation pays about 9 ns
  per run. As bitmaps the same intersection takes 0.38 ms instead of 1.56 ms, but the set is three
  times larger (2.3 MB against 0.74 MB). Runs stay, because memory comes first.
- **Bitmap chunks cost 2-3 times dense**, a fixed cost of about 30 ns per chunk. Sparse sets
  tie or win. No operation is over 2 ms on the whole repository.
- **Choices kept from the measurement:** a bitmap priced at the chunk's extent, not Roaring's 8 KB
  (at 8 KB, `identifier` was 10.9 MB); a bitmap once a chunk has more members than words; runs
  when they are smallest.

## Evidence: where containers lose time (profile)

The expectation was that set operations are bound by memory bandwidth, so containers, which move
fewer bytes, would win even with more instructions. `facts::set_census::set_profile` tests that.
It counts instructions and cycles (`proc_pid_rusage`), and runs each operation hot (in cache) and
cold (after touching 256 MB). Xcode's Time Profiler attributed the cost to source lines.

**How to tell which bound applies.** If an operation is bandwidth-bound, cold runs are much
slower than hot ones. If it is compute-bound, cold and hot cost the same and the time follows
the instruction count.

Load 50-80, so read the times as ratios:

| call ∩ inside function | µs hot | µs cold | instructions | instr/byte | bytes touched |
|---|---|---|---|---|---|
| dense | 126 | 217-361 | 0.88 M | 0.13 | 6.9 MB |
| containers, runs: first version | 1,518 | 1,511 | 9.2 M | 1.6 | 5.7 MB |
| containers, runs: now | 837 | 942 | 9.8 M | 1.7 | 5.7 MB |
| containers, the inside-set as bitmaps | 433 | 511 | 3.9 M | 0.5 | 7.4 MB |
| the same, 2^16-node chunks across files | 232 | 298 | 1.9 M | 0.3 | 6.9 MB |
| raise ∩ inside except (sparse), containers | 81 | 83 | 0.5 M | 2.4 | 0.2 MB |
| the same, 2^16-node chunks across files | 30 | 32 | 0.4 M | 4.2 | 0.1 MB |

What it shows:

- **Dense is bandwidth-bound.** Cold runs are 1.7-2.9 times hot ones, at 0.13 instructions per
  byte (NEON, two words per instruction, no count).
- **Containers are compute-bound, not bandwidth-bound.** Cold and hot runs cost the same. They
  spend 1.7 instructions per byte, about 13 times dense. At about 3 instructions per cycle,
  anything over about 0.3 instructions per byte cannot reach memory speed.
- **Dense-ish sets save no bytes.** `call` has about 3.5 members per 64-bit word, so nearly every
  word is non-zero, and its container is a bitmap anyway. The intersection touches 5.7 MB against
  dense's 6.9 MB, only 18% less. There is no bandwidth to save, so the extra instructions are
  pure loss. Containers win only where they shrink the bytes: sparse sets, 0.2 MB against 6.9 MB,
  which runs 1.5 times faster than dense (4 times with 2^16-node chunks).
- **The runs are short.** "Inside a function" is 135 K runs over about 250 K words: 1.85 words
  per run. The first version did a `memcpy` and an edge test per run, and the system `memmove`
  alone was 38% of the samples. Doing the work per word instead removed the calls, but the
  `ws == wl` branch mispredicts about half the time. The current version is branch-free: each run
  toggles two bits, and one pass turns the toggles into the mask with a prefix xor (a carry-less
  multiply, PMULL), ands it with the bitmap and counts. That pass runs at IPC 3.9 and halves the
  cycles, but it is still about 15 instructions per word plus about 8 per run.
- **Per-chunk overhead.** A chunk is one file's 2^16-node slice, and a file averages 2,045 nodes
  (32 words). About 300 instructions per chunk (directory merge, dispatch, sealing) are spent over
  32 words of payload. Chunking the space Roaring's way, 2^16 nodes across files, means 282
  chunks instead of 8,129. That halves the bitmap cases (433 → 232 µs) and makes sparse ones 2.5
  times faster. The result must be the same set: the profile checks both chunkings agree.

Possible next levers:

1. **Chunk the space, not the file** (taken: "Chunks of the space", below).
2. **Pick containers for speed where an operation is hot.** As bitmaps, "inside a function"
   intersects twice as fast but is three times larger.
3. **Count lazily.** Today every result counts its members to choose its container. A pass with
   no count could run at dense's instruction rate.

## Evidence: chunks of the space

A chunk is now 2^16 nodes of the space, across files: 282 chunks over the repository instead of
8,129. Sets are built in node order by `space::Builder`. A chunk's single nodes gather as an
array (up to 1,024 of them), and anything else in a 1,024-word bitmap of the chunk. The chunk is
sealed in the form that suits it when the next chunk starts. Per-file questions use the space:
- `within_files` takes the first test's files, keeps those where every other test has a node,
  and intersects with those files' ranges as runs;
- `file_nodes` reads the chunks over the file's range;
- a stored segment is cut from them.
A bitmap less runs now uses the same toggle-and-prefix-xor mask as a bitmap within runs.

Same repository, results checked against dense and against an ordered-set model (files not
aligned to words or chunks). Thread CPU per operation; load 34-57:

| Operation | Dense ms | Per-file chunks ms | Space chunks ms |
|---|---|---|---|
| call ∩ inside function | 0.10 | 1.24 | 0.63 |
| call − inside function | 0.15 | 2.25 | 0.89 |
| identifier ∩ inside class | 0.12 | 0.75 | 0.45 |
| identifier − inside class | 0.15 | 1.16 | 0.41 |
| raise ∩ inside except | 0.11 | 0.13 | 0.04 |
| decorator ∩ call | 0.14 | 0.24 | 0.04 |
| call ∪ await | 0.12 | 0.42 | 0.26 |
| inside function ∪ inside class | 0.11 | 0.63 | 0.52 |
| call within files of raise | 0.21 | 0.51 | 0.97, then 0.22 (below) |
| identifier within files of await | 0.27 | 0.39 | 0.65, then 0.14 (below) |
| clone call | 0.08 | 0.07 | 0.05 |

The per-file column is from the step 2 run (load 78), with that step's code. `set_profile`, with
the runs/bitmap work of the previous section on both sides:

| | Per-file chunks µs | Space chunks µs | Instructions, space |
|---|---|---|---|
| call ∩ inside function (runs) | 837 | 607 | 7.9 M |
| the same, the inside-set as bitmaps | 433 | 208 | 2.0 M |
| raise ∩ inside except (sparse) | 81 | 39 | 0.4 M |
| await ∩ call (sparse ∩ dense-ish) | 76 | 20 | 0.3 M |

Other measures:
- **Memory:** 117 sets, 48.3 MB → 40.1 MB; the largest wave 0.7 → 0.6 MB. Fewer directory
  entries, and a chunk's bitmap is shared by its files.
- **Reading kind postings into sets** (`posting_forms`, every kind over the repository):
  135 ms, against 66 ms when segments were copied as they are. Delta-varint read through the
  same builder: 188 ms. The cost is the shift into the space's chunks, and sealing.
- **Push path, 219-file subtree** (`data_layer_bench`, load 66-71): 1,806 operations, evaluation
  23.4 ms (17.8 ms per file, load 44), peak set bytes 5.0 MB (5.1), `sets.read` 150 ms.
- Replay unchanged: 3988 273 266 0; converted 3988 175 169 0.

What it shows:
- **Intersections, differences and unions gain 1.2 to 6 times**, because the per-chunk cost is
  paid 282 times, not 8,129. Sparse operations now beat dense by 2.6 to 3.8 times.
- **Large sets are still 2 to 6 times slower than dense.** They are compute-bound, as the
  previous section shows. What is left is the runs mask (about 15 instructions per word) and the
  count.
- **`within` was slower at first: 0.97 ms against 0.51 ms per file.** A profile of `call within
  files of raise` (648 µs, 12.4 M instructions; dense 121 µs, 1.7 M; the benchmark's clone of
  `call` included on both sides) showed three costs:
  - 41%: building the kept files' ranges through `Builder`. Its seal scanned all 1,024 words of
    each chunk to count members and find runs that were already known.
  - 32%: intersecting with those ranges by the toggle-and-prefix-xor mask, about 15
    instructions on each of `call`'s 287 K words, though a kept range is whole files (32 words
    or more).
  - 12%: finding the test's files. That was a `Vec` per chunk and one range query per file of the
    space, even for a sparse test.

  Now:
  - the ranges are written directly as runs containers;
  - a bitmap within runs averaging 8 words or more copies the inner words and masks only the
    edges, then counts in one vectorized pass;
  - a sparse chunk's files are found by walking its members with a cursor over the files.

  `call within files of raise`: 199 µs and 3.7 M instructions (dense 128 µs). In
  `set_operations` at load 18:
  - call within files of raise: 0.22 ms (dense 0.14, per-file chunks 0.51 at load 78);
  - identifier within files of await: 0.14 ms (dense 0.17, per-file 0.39).

  What is left is mostly the benchmark's clone, the copy and the count, and turning sparse
  results into arrays.
- **Reading postings was twice as slow: 135 ms against 66 ms.** A profile of reading every
  kind's set (`posting_forms`, `RERAIL_POSTING_SPIN`) found:
  - ~45%: decoding bytes element by element (`chunks_exact` over `u8`, a new `Vec<u64>` per
    bitmap);
  - ~15%: `words_at`, word by word;
  - ~13%: `node()` per array member;
  - ~8%: sealing.

  Now:
  - containers are read with one copy into reused buffers;
  - an array is offset and appended a chunk at a time;
  - a bitmap is shifted into its chunk in one loop;
  - a sealed chunk's array is extracted in one tight loop.

  The bench now also times a floor: each segment's containers parsed and copied as they are,
  which is what per-file chunks did. At load 37, twice:

  | Read | ms |
  |---|---|
  | delta-varint | 196-199 |
  | segments into the space's chunks | 107-113 |
  | segments copied as they are (the floor) | 46-48 |

  Moving segments into the space's chunks costs about 60 ms over the whole repository, all
  393,756 (document, kind) postings. That is the price of the layout, paid once when a batch
  reads its leaves. Of what is left, about half the samples are the first touch of each
  segment: 40 MB in 393 K separate buffers, more than the 16 MB L2. The floor pays that too.
  The rest is the shift and offset, and choosing each chunk's form.

## Steps

1. **Measure first** (done, Evidence) (the 9,010-file repository, core facts only, no rule run). For every kind
   set and every `inside_<set>` result the data-layer programs use, per document chunk: the
   member count, the run count, and the size as array, bitmap and runs. Report the distribution
   and what Roaring's thresholds would choose.
2. Host type and operations (§1-2), replacing `NodeSet`; set steps from SQL's plan run on it (done:
   `space/chunked.rs`; the prefilter's set programs run on it; Metal reads and writes dense words
   through `to_words`/`from_words` until step 4).
3. Postings as containers (§5); compare size and read cost with delta-varint (kind postings done:
   `tape_kind_counts.postings` are container segments, prefilter version 6; SQL reads them through
   `nera_segment`, the prefilter's sets copy their containers. Over the 9,010-file repository
   (`posting_forms`): 40.6 MB against delta-varint's 19.5 MB on disk, read into every kind's set
   in 66 ms against 201 ms. In memory the sets are the same. Disk has room under its 10 GB budget,
   so the copy-in read wins. Term postings and coverage stay delta-varint: they are read through
   SQL rows, not as sets).
4. Device containers (§4) for the Metal set steps and label passes.

Each step: the replay and the converted gate unchanged; CPU time against the current set code.

## Prior work consulted

- **Spike history index 2.4, "Per-key array-or-bitmap encoding" (DH L4679-4762, S198a/b):** choosing
  array or bitmap per key saved 29-35% of the payload on slices and 12.2% on the budget corpus.
  *Agrees, and goes further:* the choice is per chunk, and runs are added.
- **History index Part 0, item 10, "A set stored as a string is a hidden quarter of execution"
  (S227, DH L6213-6264):** a 128-byte kind bitmap per file was 24% faster and smaller than a
  string set. *Agrees.*
- **History index T41 (DH L174-176, L2493-2496):** a per-language bitmap plateaued at 84%
  because "a bitmap does not reduce what the scan reads". *Heeded:* containers pay only where
  operations visit present chunks only; the measurement must show what is skipped.
- **"Descendant axis = interval containment on (pre, size)" (DH L3766-3773).** *Agrees* (§3).
- **0024 §1** (sets as bitmaps over one node space; sparse when `bound·32 < n`). *Refined:* the
  same three forms per chunk instead of per set; its device rules kept.
- **0026** (no per-node structure beside a tape). *Kept:* containers are sets, not per-node
  attributes.
- **GPU** is the spike's known gap: no prior evidence on device containers.
