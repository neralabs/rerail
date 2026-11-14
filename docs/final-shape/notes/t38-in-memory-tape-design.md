# t38: the in-memory tape, a design pass (0156 T9, E10)

## Intro

**What this decides.** A tape is stored compressed on disk and read in place in memory (0156,
"Fixed"). Compression at rest is close to solved: format 16's events under zstd take 0.7–0.9 B a
node (t36). What is open is the form loading produces. Loading means zstd plus at most one
linear pass. The result must answer every tree question in place, with random access, on every
platform the engine runs on, without too much memory and without losing too much speed.

**The tradeoff in one line.** Explicit fields (format 15, simdjson) answer each question with one
load, but take 8.7–9.3 B a node. Succinct trees take about 2 bits a node for the structure, but
their questions go through rank, select and excess searches, whose speed depends heavily on the
instruction set. Bit-packed arrays (mldb's mapped int tables) sit between the two: a shift and a
mask everywhere, with weaker size guarantees.

**What this note is.** A survey of the options against our questions and platforms, a candidate
set, and what E10 has to measure. The bit counts below are estimates until E10 measures them.

## 1. What readers ask

These are the questions, by how often rules ask them (E7: reads a restored node, through
`TapeDoc`), with what each needs:

| question | median rule | 121 rules | heaviest | what it needs |
|---|---|---|---|---|
| kind | 2.06 | 2.49 | 11.47 | a sweep over all nodes, and random reads |
| flags, named | 1.0 each | 0.76, 1.00 | 1.5, 3.6 | swept with kind |
| field | 0.04 | 0.33 | 2.68 | random |
| span (start, end) | 0.01 | 0.96 | 11.12 | random; within a subtree |
| children | 0.05 | 1.17 | 13.18 | first child, next sibling |
| parent | 0.00 | 0.00 | 0.01 (5.0 in one of the 7) | random |
| classes (memberships) | 0.01 | 0.01 | 0.12 | random |
| subtree end, `span_within` | (via children, span) | | | an interval: i to i + size |
| line | (0174's requests) | | | random |
| text | (0174's requests) | | | a leaf's bytes, or its stream id |

Points, previous siblings and sizes are never read through `TapeDoc` today. The engine's other
readers (the prefilter, the search index, facts, Metal) sweep kind and flags, and probe children,
fields and spans.

So, in order:
1. A sweep of kind and flags over every node must be the cheapest thing the format does.
2. First child, next sibling and the subtree interval must be cheap at random.
3. Span must be cheap at random.
4. Field, classes, parent and line can cost more, as long as they stay bounded.

## 2. Platforms

What each operation costs where the engine runs (from the ISA manuals; marked where only
measuring will tell):

| | Apple M5 (here) | Apple M1–M3 | x86-64, BMI2 (Intel Haswell+, AMD Zen 3+) | x86-64, AMD Zen 1–2 | wasm (browser build) | Metal |
|---|---|---|---|---|---|---|
| popcount of a word | scalar `CNT` (FEAT_CSSC, reported by `sysctl`) | vector `CNT` + `ADDV`, a few cycles | `POPCNT`, 1 cycle | `POPCNT` | `i64.popcnt`, host-dependent | `popcount`, 32-bit native |
| select in a word | broadword, about 10–20 instructions | broadword | `PDEP` + `TZCNT`, about 3 | `PDEP` microcoded, very slow: broadword | broadword | broadword, 32-bit |
| unaligned 64-bit load | yes | yes | yes | yes | yes | no: two loads and a funnel shift |
| a byte compare over 16 bytes | NEON, 1 instruction | NEON | SSE2 / AVX2 | SSE2 / AVX2 | SIMD128 | one thread per node |

- **Select is the operation that varies.** It is 3 instructions with `PDEP` and 10–20 without. On
  Zen 1–2, `PDEP` is microcoded, and slower than broadword. SME's `BDEP` exists on M4+ only in
  streaming mode, which is no use for single reads.
- **Popcount is fine everywhere except where it isn't scalar.** On M1–M3 it goes through a vector
  register. That is cheap in bulk and costs latency on a single read.
- **The GPU wants 32-bit words** and no data-dependent loops longer than a few steps.

So a portable design keeps select off the hot paths: sampled answers plus a popcount of a few
words, or no select at all.

## 3. Structure: the families

| | bits a node | first child, next sibling, subtree end | parent | depth | platform sensitivity | order |
|---|---|---|---|---|---|---|
| explicit, fixed width (format 15, simdjson) | 32–64 | one load | one load (if stored) | stored or none | none | preorder |
| explicit, bit-packed (mldb) | ~8–14 (est.) | one extract: i + 1, i + size[i] | one extract: i − back[i] | none | none: a shift and a mask | preorder |
| balanced parentheses (BP) + range min-max tree | ~2.2–2.5 | `findclose`: an excess search, O(log n) in the tree, a few words in practice | `enclose`: the same search | rank | the excess search uses byte tables or popcount; select for node-to-position | preorder |
| DFUDS | ~2.2–2.5 | i-th child in O(1) by select; subtree end by `findclose` | `findopen` | not direct | select on every child step | preorder |
| LOUDS | ~2.1 | children by rank and select, O(1) | rank and select | not direct | select on every step | level order |

- **LOUDS does not fit.** Our questions are preorder questions: a subtree is an interval, starts
  never decrease in preorder, and `span_within` is interval containment. LOUDS numbers nodes by
  level, so every one of those needs a mapping back to preorder. It is listed and not built.
- **DFUDS** is best when children are asked by index (`child(p, i)`). Rules do that, but most
  child reads in E7 are iterations (first child, then next sibling), which BP serves as well.
- **BP and format 16's events are the same sequence.** An open event is `(`, a close is `)`, in
  nesting order. So the BP bit vector comes straight from the events, one bit an event, with no
  stack and no tree. Its range min-max tree is one more linear pass.
- **Explicit bit-packed** (size and back distance) needs a stack pass at load to compute sizes and
  parents (2.4–3.3 ns a node on the CPU, measured for positions; on Metal the scan, 1–4 ns). Its
  reads are the same on every platform.

A hybrid is likely: BP for the structure, plus explicit sizes for nodes whose subtree spans more
than a few words of the BP vector, so `findclose` never searches far.

## 4. Positions

Starts never decrease in preorder. Ends never decrease in postorder, and in BP a node's postorder
number is a rank of its close.

| | bits a node (est.) | a read | platform sensitivity |
|---|---|---|---|
| absolute, 32 or 64 bits | 32–64 | one load | none |
| block base + block-local offset (frame of reference), fixed width a block | ~10–12 | one extract + base | none |
| linear predictor + residual, a range at a time (mldb `MappedIntTable`) | ~4–8, depending on how regular the code is | an extract, a multiply-add | none (one float multiply) |
| Elias–Fano | ~2 + log2(source bytes a node), about 4 | select on the high bits | select |
| deltas from the previous event (format 16's `lead`, `trail`) with an absolute every K | ~4–6 + K's samples | a walk of up to K deltas | none, but a walk |

- **The predictor is mldb's idea, and it fits.** Within a range, a node's start is close to a
  straight line in its preorder number, because code is fairly regular. A residual of a few bits
  corrects it, so a read is an extract and a multiply-add, with no select. `MappedIntTable` also
  picks, a range at a time, among fixed-width, factored (radix packing, for maxima that are not a
  power of two), run-length and predicted forms, by bytes.
- **End** can be stored as a length (end − start). Leaves are short, and inner nodes' lengths grow
  with their size. Or it can be stored in postorder through BP rank, with the same choices as start.

## 5. Kind, field, flags and memberships

These go together as one code a node from a table per language. Format 15 numbered codes per tape,
which broke "the same structure gives the same bits". Format 16 uses the grammar's numbering (0156
1.3).

- **u8 codes with an escape** keep the sweep byte-aligned. NEON compares 16 nodes an instruction,
  and the GPU reads one byte a thread.
- **Bit-packed codes** (6–9 bits) are smaller, but a sweep first spreads them into lanes.
- **Block-local code tables** (each block remaps the few codes it uses to 3–5 bits) are smaller
  still, at the cost of one table lookup a read.

Measurements will say how many distinct codes a language and a block use. Classes and field
names are then lookups in the code table, never stored per node.

## 6. Line and text

- **Line:** the line table (0.07–0.17 B a node, t36) stored in the tape, with the same encodings as
  starts. A node's line is a predecessor search, or a predicted lookup in the table followed by a
  short correction.
- **Text:** a leaf's span into the source, or its id in the names stream (0156 6). Comparisons are
  on bytes, or on stream ids once a constant is found in the pool (E9: 0.55 ns against 2.2).

## 7. One encoding, or a choice a range at a time

mldb chooses an encoding a range at a time, by bytes, and the reader dispatches on a type tag.

- **On the CPU** the dispatch is once a block in a sweep, and once a read at random: a
  well-predicted branch when blocks are mostly the same type.
- **On the GPU** neighbouring threads can take different branches (divergence) where block types
  differ.
- One encoding a field is simpler and uniform, but pays for the worst block in each block.

E10 measures both.

## 8. Loading: the decompression path

Two shapes:

1. **On disk, zstd of the in-memory form.** Loading is zstd alone (GB/s a core). But bit-packed and
   succinct data compress less well under zstd than byte-aligned events do, so disk grows from
   0.7 B a node toward an unmeasured figure.
2. **On disk, zstd of the events (0.7 B a node), then one linear pass.** The pass writes BP bits
   straight from the events, the codes in preorder, and positions by a prefix sum into their
   encoding. It runs on the CPU (SIMD) or on Metal (the scan already exists). Explicit sizes need
   a stack pass instead.

The second keeps the best disk size and is still one pass. It is not a decode into another form,
because what it writes is the tape that is read. The first loads fastest. E10 reports disk bytes
and load ns a node for both.

## 9. Candidates for E10

| | structure | positions | codes | expected bits a node |
|---|---|---|---|---|
| F15 | format 15 as it is, read in place (the baseline) | | | 70–75 (measured 8.7–9.3 B) |
| X | explicit bit-packed size and back distance, fixed width a block | frame of reference | u8 | ~40 |
| M | X, with mldb's choice a range at a time (fixed, factored, run-length, predicted) | predictor + residual | u8 or bit-packed | ~25–35 |
| B | BP + range min-max tree | predictor + residual; end by length | u8 | ~20–28 |
| BE | B, positions by Elias–Fano | Elias–Fano, start in preorder and end in postorder | u8 | ~18–24 |
| BH | B, plus explicit sizes for large subtrees (no long `findclose`) | as B | u8 | ~22–30 |

Each candidate implements the same reader interface in the lab: `kind(i)`, `flags(i)`,
`field(i)`, `classes(i)`, `first_child(i)`, `next_sibling(i)`, `subtree_end(i)`, `parent(i)`,
`start(i)`, `end(i)`, `line(i)`, `text(i)`, and a sweep `nodes_of(kind)`. Every answer is checked
against the tree.

## 10. What E10 measures

Per candidate and corpus (data-layer, rerail, CPython):

- **Memory:** bytes a node, everything included (tables, samples, min-max trees).
- **Each question:** ns and instructions a call, at random and in a sweep, on one CPU thread.
- **The E7 mix:** those costs weighted by the median, 90th-percentile and heaviest rule, giving
  ns a node.
- **Disk and load:** disk bytes and load ns a node, for both loading shapes in section 8.
- **Platforms:**
  - the M5 here, built twice: with scalar popcount, and without it (no `+cssc`, as on M1–M3);
  - select with a lookup table against broadword;
  - Metal kernels for the sweep and the random questions;
  - x86 and wasm need a run there; the reader code is portable Rust, so it is the same code.

What would end a candidate: a question with no bounded in-place answer, or a weighted cost far
off the others for no saving in bytes.

## Prior art read for this

- **mldb's mapped int tables** (`mldbai/mldb`, branch `tabular-string-analysis`,
  `plugins/tabular/`): `BitCompressedIntTable` (fixed width, an extract over one or two words);
  `FactoredIntTable` (values packed in radix `factor` into 32- or 64-bit words); `RunLengthIntTable`;
  `MappedIntTable` (ranges, each with linear predictors chosen by a selector table and bit-packed
  residuals; `getRaw` is a search for the range, a selector, a residual and a multiply-add);
  `MappedSelectorTable` (a cumulative count every 2^n entries, a rank sample); all of it mapped
  and read in place. `RawMappedIntTable` picks fixed, run-length or factored by
  `indirectBytesRequired`.
- **simdjson's tape:** 8-byte words; an open holds the index of its close.
- **Succinct trees:** BP with a range min-max tree (Navarro and Sadakane), DFUDS, LOUDS (Jacobson).
