# 0045: Node sets are chunked containers: arrays, bitmaps and runs

Status: proposed (user agreed to the draft, 2026-11-06); step 1 measured (Evidence)

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

- **Directory:** sorted `(document, chunk) → (kind, offset)`, one entry per non-empty chunk.
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
- **Containers are per document**, so 0044's per-document derivations replace a document's
  chunks and nothing else.

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

Not measured yet: operation time, and the sets the data-layer programs actually build (label
results, candidate roots, text slots). Step 2 measures both against `NodeSet`.

## Steps

1. **Measure first** (done, Evidence) (the 9,010-file repository, core facts only, no rule run). For every kind
   set and every `inside_<set>` result the data-layer programs use, per document chunk: the
   member count, the run count, and the size as array, bitmap and runs. Report the distribution
   and what Roaring's thresholds would choose.
2. Host type and operations (§1-2), replacing `NodeSet`; set steps from SQL's plan run on it.
3. Postings as containers (§5); compare size and read cost with delta-varint.
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
