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

**The targets** (the user, 2026-11-14):

| tier | CPUs | GPUs | other |
|---|---|---|---|
| most important | Apple M5; AMD Zen 3+; Intel of the last 3 years | Metal | |
| decent | Apple M3 | CUDA (important, not now) | |
| works, not horrible | Apple M1; older Intel | OpenCL (results may vary a lot) | wasm (WebGPU later) |

What follows from it:
- **Every first-tier CPU has a fast scalar popcount**, and the x86 ones have a fast `PDEP`. So
  select is cheap there, except on the M5, which has no `PDEP`: there select is broadword.
- **M1 and M3 have no scalar popcount** (no FEAT_CSSC). Older Intel before Haswell has no BMI2, so
  no `PDEP`. A design that leans on select or single popcounts must still be "decent" there, and
  E10 measures that by building without them.
- **Metal is first-tier**, so every question has a kernel form: 32-bit words, short bounded loops.
- **The same Rust reader** serves every CPU. Where an instruction differs, the choice is made at
  build time or when loading (feature detection), never once a read.

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

## E10, first results: F15, X and B (2026-11-14)

`tape16 inplace <cap>` builds each candidate per file, checks every answer for every node against the
tree (0 wrong on every corpus and candidate), and costs each question on one CPU thread of the M5.
The readers are monomorphized, so each accessor inlines as a real reader's would. Instructions
include the loop around each call. The load average was 30–60.

- **F15:** format 15's image, read as it lies.
- **X:** explicit and bit-packed: code at the language's width; size, back distance, start (frame of
  reference) and length with patched exceptions, a block of 256 nodes at a time.
- **B:** balanced parentheses with rank9 and excess summaries a word and per 32 words, plus X's
  code, start and length by preorder rank. A node is its open's position.

**Bytes a node:**

| | impl/sqlite | data-layer, every 10th | rerail, every 5th |
|---|---|---|---|
| F15 | 8.66 | 9.00 | 9.34 |
| X | **5.49** | **4.88** | **5.06** |
| B | **4.13** | **3.60** | **3.69** |

By part (data-layer; the others are within 0.4):

| | structure | code | start | length |
|---|---|---|---|---|
| X | size 0.68 + back 0.96 | 0.70 | 1.48 | 1.05 |
| B | parentheses 0.25 + summaries 0.12 | 0.70 | 1.48 | 1.05 |

**ns a call** (data-layer; instructions in brackets):

| | F15 | X | B |
|---|---|---|---|
| sweep: code in a set | 1.0 (24) | 1.4 (29) | 2.8 (55) |
| code at random | 0.9 (20) | 1.3 (28) | 2.8 (52) |
| start + end at random | 5.3 (81) | 5.2 (98) | 8.4 (127) |
| subtree end at random | 1.2 (25) | 2.6 (45) | 12.6 (95) |
| children of a node, all of them | 3.8 (50) | 7.9 (98) | 15.2 (193) |
| parent at random | 247 (5,234) | 6.2 (98) | 18.1 (168) |

**E7's mixes, ns a node** (data-layer):

| | F15 | X | B |
|---|---|---|---|
| median rule | 2.3 | 3.3 | 6.7 |
| 121 rules, all | 12.2 | 18.0 | 33.7 |
| schema-introspection (5 parents a node) | 1,269 | 81 | 183 |
| heaviest rule | 125 | 181 | 333 |

On rerail, F15's parent costs 4.9 µs and the tree walk 24.5 µs a node. With no parent stored,
format 15 walks back over every preceding node, so large files make it worse.

What this says:

- **Positions are the lever.** Start and length are 2.5 B a node, 50–70% of X and B. The structure
  is 1.6 B in X and 0.37 B in B; codes are 0.7–1.1 B. Positions are next: mldb's predictor plus
  residual (M), Elias–Fano (BE), and lengths predicted from the subtree.
- **X costs 0.55× F15's bytes for 1.3–2× its per-call time,** and it has a parent: 6 ns, against
  F15's backward walk.
- **B saves 1.2–1.4 B a node more than X, but the structure costs 2–5× more.** `findclose` (subtree
  end, next sibling) takes 12.6 ns against 2.6, and every field read pays a rank (2.8 ns against 1.3
  for a code). Only 1.3 of B's 3.6 B is not shared with X, so B's lead in bytes shrinks as
  positions shrink, while its cost does not.
- **Next:** M and BE (positions), BH (B with explicit sizes for large subtrees); disk bytes and load
  cost for both loading shapes; the M5 built without scalar popcount, standing in for M1 and M3;
  then the questions as Metal kernels.

### F11: format 11, every token stored

Format 11 (06dbef22b) has format 15's columns, but stores every node, anonymous tokens included
(named-only at rest came with format 12), and a 32-byte source digest a tape. `capture-tapes
--all-tokens` encodes the tree-sitter tree with no token filter. JSON goes through tree-sitter-json
rather than the Bitlex parser, and `.rr` keeps its lean parser. `tape16 inplace --f11` reads those
images in place with F15's reader, plus the digest. The same files were captured both ways (rerail
again, as `rerail-5b`, 1,698 files, from the current tree).

| | impl/sqlite | data-layer, every 10th | rerail, every 5th (5b) |
|---|---|---|---|
| nodes, format 15 | 472,556 | 9,539,304 | 5,539,786 |
| nodes, format 11 | 672,712 (1.42×) | 19,192,413 (2.01×) | 8,989,553 (1.62×) |
| F11 bytes a node of its own | 8.79 | 9.29 | 9.27 |
| **F11 bytes a format-15 node** | **12.5** | **18.7** | **15.0** |
| F15 bytes a node | 8.66 | 9.00 | 9.20 |
| X, B bytes a node (format 15's nodes) | 5.49, 4.13 | 4.88, 3.60 | 5.08, 3.71 |

F11's calls cost what F15's do (the same columns; data-layer, ns): sweep 1.1, code 1.2, start and
end 7.2, subtree end 1.5, children 3.9; parent 412 (7.0 µs on rerail), by the same backward walk. But
every sweep and every children walk covers 1.4–2.0× the nodes. So a rule pays F11's sweeps 1.4–2.0×
over, and F11 holds 1.4–2.1× F15's bytes for the nodes rules ask about.

E7's mixes, ns a node of F11's own (data-layer): median 2.6, 121 rules 14.6, schema-introspection
2,099, heaviest 152.

## E10, second round: positions, BH, loading, M1/M3, Metal (2026-11-14)

New candidates, each built per file, every answer checked against the tree (0 wrong on every corpus):

| | structure | start | end |
|---|---|---|---|
| X | explicit size + back distance | frame of reference | length |
| M | as X | mldb's predictor + residual (a line a block, or slope 0 where smaller) | the gap to the start of the node past the subtree |
| XE | as X | Elias–Fano | gap |
| B | balanced parentheses | frame of reference | length |
| BM | as B | predictor + residual | length |
| BE | as B | Elias–Fano (preorder) | Elias–Fano (postorder, through `findclose` and rank) |
| BH | as B, plus stored sizes for subtrees over 64 positions, and a branch-free 64-bit window search | predictor | length |

Elias–Fano's select is Vigna's broadword select in a word, after a four-word window chosen by
comparisons (a sample every 64 ones). Timings below are the best of three runs, on one thread. The
machine was shared (load 50–70), and single runs varied 2–4×.

**Bytes a node:**

| | F15 | X | M | XE | B | BM | BE | BH |
|---|---|---|---|---|---|---|---|---|
| impl/sqlite | 8.66 | 5.49 | 4.69 | 4.27 | 4.13 | 3.76 | **2.86** | 4.21 |
| data-layer | 9.00 | 4.88 | 3.96 | 3.56 | 3.61 | 3.20 | **2.43** | 3.60 |
| rerail (5b) | 9.20 | 5.08 | 4.24 | 3.85 | 3.72 | 3.31 | **2.52** | 3.76 |

By part (data-layer): code 0.70; structure 1.64 (X family) or 0.37 (B family; BH 0.77); start 1.48
(frame of reference), 1.08 (predictor), 0.68 (Elias–Fano); end 1.05 (length), 0.53 (gap), 0.68
(Elias–Fano).

**ns a call** (data-layer, default build; instructions in brackets):

| | F15 | X | M | XE | B | BM | BE | BH |
|---|---|---|---|---|---|---|---|---|
| sweep: code in a set | 0.8 (24) | 1.1 (29) | 1.1 (29) | 1.1 (29) | 2.2 (55) | 2.2 (55) | 2.2 (55) | 2.2 (55) |
| code at random | 0.8 (20) | 1.0 (28) | 1.0 (28) | 1.0 (28) | 2.1 (52) | 2.1 (52) | 2.1 (52) | 2.1 (52) |
| start | 1.1 (29) | 1.6 (43) | 1.9 (46) | 7.8 (130) | 3.0 (69) | 3.3 (75) | 11.3 (162) | 3.3 (75) |
| end | 3.0 (59) | 3.4 (83) | 6.4 (139) | 15.7 (226) | 5.6 (112) | 5.8 (115) | 22.0 (276) | 5.8 (115) |
| subtree end | 0.9 (25) | 2.1 (45) | 2.0 (45) | 2.0 (45) | 7.9 (87) | 8.2 (87) | 8.2 (87) | 19.6 (161) |
| children of a node | 2.5 (51) | 5.9 (104) | 5.9 (104) | 6.0 (104) | 10.8 (191) | 10.6 (191) | 10.6 (191) | 21.7 (344) |
| parent | 186 (5,230) | 3.3 (97) | 3.3 (97) | 3.4 (97) | 8.4 (159) | 7.9 (159) | 7.7 (159) | 7.9 (159) |

**E7's mixes, ns a node** (data-layer; impl/sqlite and rerail in the same order):

| | F15 | X | M | XE | B | BM | BE | BH |
|---|---|---|---|---|---|---|---|---|
| median rule | 1.8 | 2.6 | 2.6 | 2.8 | 5.3 | 5.3 | 5.5 | 5.9 |
| 121 rules | 9.0 | 14.7 | 17.8 | 32.6 | 27.1 | 27.3 | 50.5 | 40.4 |
| schema-introspection | 956 | 57.7 | 68.0 | 117 | 117 | 115 | 190 | 154 |
| heaviest rule | 91 | 149 | 184 | 356 | 268 | 271 | 540 | 418 |

### 1. Positions

- **mldb's predictor saves 0.4 B a node on start (1.48 → 1.08), for 0.3 ns.** The gap form of end
  saves another 0.5 B (1.05 → 0.53), but it reads the next node's start, so end costs 6.4 ns instead
  of 3.4.
- **Elias–Fano is the smallest (0.68 B each for start and end) and the slowest** (start 7.8–11.3 ns,
  end 15.7–22 ns). Before the broadword select and the four-word window, start took 40 ns; the time
  is mispredicted branches in the select, not instructions.
- **BE is the smallest tape measured: 2.43–2.86 B a node,** 0.27–0.33× format 15. It costs 2–3.5×
  X's time in the mixes.

### 2. BH

- **Stored sizes do not help.** BH's subtree end costs 19.6 ns, against B's 7.9. A long search was
  not the cost: most subtrees are small, and B's cost is the search within a word.
- **The branch-free window is slower than the branchy search** it replaces. Its eight table lookups
  are independent (the start of each byte comes from broadword prefix popcounts), and it has no data
  branch. It still runs at about 0.3 instructions a cycle, against F15's 4 or more. The stall has not
  been traced: the M5's counter profiles are refused (memory: xctrace), so it would need another
  route.
- **The marks cost 0.4 B a node** (a bit per position). BH as built is dropped. What it shows is that
  in the B family the price is `findclose` itself, at 8 ns a call.

### 3. Loading

| data-layer | F15 | X | M | XE | B | BM | BE | BH |
|---|---|---|---|---|---|---|---|---|
| shape 1: disk, zstd 3 of the in-memory bytes, B/node | 1.58 | 2.91 | 2.10 | 1.58 | 2.42 | 2.04 | 1.36 | 2.13 |
| shape 1: load (zstd), ns/node | 5.8 | 3.9 | 3.2 | 2.8 | 2.4 | 2.3 | 2.0 | 2.7 |
| shape 2: disk, zstd 3 of the events, B/node | 0.73 | | | | | | | |
| shape 2: zstd + events to a tree, ns/node | 39.3 | | | | | | | |
| shape 2: + the lab's build from the tree, ns/node | | 102 | 105 | 111 | 73 | 77 | 52 | 79 |

On impl/sqlite, shape 1 takes 2.19 (BE) to 4.09 (X) B a node and shape 2 1.56. On rerail, 1.43 to
3.03 and 0.93.

- **Bit-packed tapes barely compress under zstd.** X goes from 4.88 to 2.91, and BE from 2.43 to
  1.36. Format 15's byte columns go from 9.00 to 1.58, so zstd finds what the bit packing already
  took. Shape 1 loads in 2–6 ns a node and costs 1.9–4× shape 2's disk.
- **Shape 2's build here is an upper bound.** It goes through a tree, and the lab's builders allocate
  a vector a field and hash every code. A writer that goes straight from events to the tape is one
  pass: BP bits are the events themselves, codes are the opens' in order, and positions are a prefix
  sum. So the 50–110 ns says nothing yet about shape 2's real cost. E10 needs that writer before
  shape 2 can be judged.

### 4. M1 and M3

The default build of `aarch64-apple-darwin` targets apple-m1: popcount is the vector `CNT` + `ADDV`
(168 sites in the lab). Built with `-C target-cpu=apple-m4 -C target-feature=+cssc`, the same sites
are scalar `cnt x, x` (checked with `llvm-objdump --mattr=+cssc`). Rust lists no CSSC for any Apple
CPU, so it has to be asked for.

- **Scalar popcount changes nothing that measures.** The B family saves about 3 instructions a call
  (rank 52 → 49), and the nanoseconds are within the noise. So popcount is not what B pays for.
- **The default build is M1's instruction set, not M1's microarchitecture.** M1 and M3 numbers need
  those machines. With no select instruction anywhere, and popcount not the cost, nothing here leans
  on an instruction M1 lacks.

### 5. Metal

`tape16 inplace-gpu <cap>` builds each candidate over the corpus as one forest: the files under a
root of kind 0, positions shifted so starts stay monotone, codes from one table for all languages.
The image is one buffer with a table of offsets, and each kernel answers one question a thread, over
every node at random (in order for the sweep). It takes the best of four warm dispatches, and every
answer is checked against the CPU reader's. Kernels exist for X, M, B and BE. Because the forest
merges languages, its codes are wider, so its images are 0.3–0.7 B a node larger than the per-file
sums above.

| GPU ns a question | X | M | B | BE |
|---|---|---|---|---|
| image bytes a node (data-layer) | 5.53 | 4.61 | 4.26 | 3.11 |
| code, in order (a sweep) | 0.05 | 0.05 | 0.03 | 0.06 |
| code at random | 0.12 | 0.12 | 0.12 | 0.38 |
| start at random | 0.30 | 0.18 | 0.20 | 0.32 |
| end at random | 0.42 | 0.36 | 0.29 | 0.35 |
| subtree end at random | 0.12 | 0.07 | 0.25 | 0.23 |
| parent at random | 0.13 | 0.08 | 0.46 | 0.46 |
| children of a node, at random | 0.59 | 0.53 | 1.21 | 1.21 |

Rerail (5.5M nodes) is within 2× of these, except children (4.6–5.0 ns): the forest's root has 1,698
children, and wide nodes keep one thread walking while the rest of its group wait. impl/sqlite (0.47M
nodes) is too small to fill the GPU.

- **Every question runs on Metal at 0.03–1.2 ns a question,** 10–40× the CPU's single thread.
- **On the GPU the B family's structure costs 2–6× X's** (subtree end 0.25 against 0.07–0.12, parent
  0.46 against 0.08–0.13), the same order as on the CPU. Elias–Fano's select costs little there:
  start 0.32 against 0.18–0.30.

### What the second round says

- **X-family positions are the cheap lever.** M saves 0.9 B a node over X (4.88 → 3.96) for 0–3 ns
  a call. XE saves 1.3 B, but its Elias–Fano start and end cost 5–12 ns
  more.
- **B saves 1.3 B a node of structure, and pays 2–4× on every structure question,** on the CPU and on
  Metal. BE is the smallest (2.43–2.86 B), at 2–3.5× X's mixes.
- **Structure is now X's largest part** (1.64 B: size 0.68, back 0.96). The next lever for the X
  family is the back distance, which grows with sibling subtrees.
- **Still to measure:**
  - shape 2's real cost, through a writer from events straight to each tape;
  - M1, M3 and x86 hardware;
  - BE's select on x86 with `PDEP`;
  - the forest's children walk, given wide nodes, as a kernel one group a node.

  The third round (below) takes the first three.

## E10, third round: X's structure, shape 2's writer, other CPUs (2026-11-14)

Three things changed, and every candidate was measured again. The numbers below supersede the second
round's: the new packing (2) makes every candidate 0.1–0.2 B a node smaller. Every answer was checked
against the tree again, with 0 wrong on every corpus, the CPU and Metal alike.

### 1. XF: size and back distance in one slot

X stores two numbers a node, the subtree size and the distance back to the parent. Half of each is
known without storing it:

- a leaf's size is always 1;
- the node after an inner node is that node's first child, so its parent is one back.

XF stores one slot a node, tagged by its low bit. An inner node's slot holds its size (tag 0). A
leaf's slot holds the back distance of the node after it (tag 1): that node follows a leaf, so it is
a later sibling, and its parent is the one thing not known. Every question reads one slot, with no
rank and no search:

```rust
// subtree end: a leaf ends at the next node, an inner node at its size
let v = slot.get(i);
let t = v & 1;
i + (((v >> 1) & t.wrapping_sub(1)) | t) as usize
// parent of i (not the root): read the slot of the node before it
let v = slot.get(i - 1);
let t = v & 1;
i - (((v >> 1) & t.wrapping_neg()) | (t ^ 1)) as usize
```

| structure, B/node | X | XF | B |
|---|---|---|---|
| impl/sqlite | 1.58 | **0.99** | 0.38 |
| data-layer | 1.53 | **1.02** | 0.37 |
| rerail | 1.61 | **1.11** | 0.37 |

- **XF saves 0.5–0.6 B a node.** XF is 4.15 B/node on data-layer, against X's 4.66. MF (XF with the
  predictor and the gap end) is 3.37, against M's 3.88. That puts MF within 0.25 B of B (3.50), at X's
  speed.
- **The cost is 0.3–0.7 ns on subtree end and parent** (data-layer: 2.4 against 1.7, and 3.0 against
  2.7). Leaf or inner is a coin toss, so the reads are branch-free; with a branch, parent cost 5.5 ns.
- **On Metal XF costs the same as X** (subtree end 0.086 ns, parent 0.086; MF 0.060 for both).
- **XF's floor in this family is about 1 B.** The slots carry 4.3 bits of value on average, but a
  block's width is set by its tail. Moving the values that cross a block boundary into per-block
  ancestor tables would gain at most 0.1 B, before paying for the tables (`XF_HIST=1` prints these
  costs). So it was not built.

### 2. PFOR with indexed escapes

Every candidate packs its arrays with PFOR: 256 values a block at one width, with exceptions for the
values that do not fit. The second round's PFOR stored each exception as 40 bits (an index and a
32-bit value), and found an exception by scanning the block's list. Now a block's top `e` codes are
escapes: a slot at or above 2^w − e names its exception directly, and the exceptions follow the
block's slots, packed at the block's own exception width. The header is one word a block. So no
exception needs a search, and none takes more bits than the block's largest value.

- **Every candidate is 0.1–0.2 B a node smaller,** and the questions cost the same.
- **A branch-free exception read is slower:** start went from 2.3 to 5.6 ns, because the second load
  is then always taken. It was dropped.
- **The width comes from a histogram of value widths,** and the escapes are made exact for that
  width. Before, every width from 0 to 40 was tried, each to a fixpoint. That cost 0.01–0.04 B a node
  and cut the build 8×.

### 3. Shape 2: a writer from events straight to each tape

Every builder now reads one set of flat columns (code, start, end, size, parent, in preorder). The
columns come either from the tree, or straight from format 16's events in one pass with a stack of
open nodes. The writer reads them off the byte planes as zstd gives them back, with no events built
between. Each candidate built from the events is checked byte for byte against the one built from the
tree.

| data-layer, ns/node | X | M | B | BE | XF | MF |
|---|---|---|---|---|---|---|
| zstd of the events | 4.0 | 4.0 | 4.0 | 4.0 | 4.0 | 4.0 |
| byte planes to columns | 6.9 | 6.9 | 6.9 | 6.9 | 6.9 | 6.9 |
| columns to the tape | 23.1 | 24.3 | 16.5 | 16.1 | 18.3 | 19.7 |
| **shape 2: load, all** | **33.9** | **35.2** | **27.4** | **27.0** | **29.2** | **30.6** |
| shape 1: load (zstd of the tape) | 2.8 | 2.3 | 1.3 | 1.7 | 2.1 | 1.7 |

| disk, B/node | X | M | B | BE | XF | MF |
|---|---|---|---|---|---|---|
| shape 1 (data-layer / impl/sqlite / rerail) | 2.98 / 4.02 / 3.06 | 2.16 / 3.28 / 2.39 | 2.48 / 3.30 / 2.47 | 1.36 / 2.19 / 1.43 | 2.86 / 3.74 / 2.92 | 2.04 / 3.01 / 2.27 |
| shape 2 (the events) | 0.73 / 1.56 / 0.93 | | | | | |

- **Shape 2 loads in 27–48 ns a node** (29–40 on impl/sqlite, 34–48 on rerail), against the second
  round's upper bound of 90–150. Shape 1 loads in 1.3–4.2 ns, but takes 1.4–4× the disk.
- **The build is now most of shape 2's load:** 16–34 ns a node. X's four PFOR arrays cost the most,
  and the B family's parentheses cost little.
- **So it is a trade:** with shape 2, a 10 GB/s read of 0.73 B a node gives nodes 30–40× faster than
  the writer can build them. Shape 2 wins where disk or the network is the limit (cold loads, remote
  stores), and shape 1 where load time is (a warm local cache).

### 4. x86, M1 and M3

- **x86 with BMI2 takes Elias–Fano's select from PDEP:** `_pdep_u64(1 << r, x).trailing_zeros()`.
  Builds without BMI2 take the broadword select. That includes Zen 1 and 2, which run PDEP in
  microcode, so a build for them has to leave BMI2 off.
- **The lab now builds and counts on Linux** (instructions and cycles by `perf_event_open`), and
  every results file opens with the machine and the target features compiled in.
- **Checked under Rosetta, not timed:** an x86-64 baseline build and an x86-64-v3 build (BMI2, 12
  PDEP sites) give every answer right on impl/sqlite, with the same bytes. Rosetta's timings are the
  M5's running translated code, so they say nothing about x86 CPUs.
- **`scripts/bench-pack.sh <dir> <capture>...`** packs the lab, the format crate, the toolchain pin
  and the captures into one directory. Its `run.sh` builds a baseline build and a
  `target-cpu=native` build on the machine, and runs every question on each capture. M1, M3 and x86
  numbers need those machines. The captures hold source text, so a pack carries the code of the
  repositories it was captured from.

### 5. x86 hardware (Google Cloud, 4 vCPU, data-layer)

Each machine ran the pack: a baseline x86-64 build (no POPCNT) and a `target-cpu=native` build
(POPCNT, BMI2 and PDEP on all four). Every answer is right on every machine, build and corpus.
Results are in `tmp/cap/remote/`.

| ns, native build | M5 | Zen 3 (EPYC 7B13) | Zen 4 (EPYC 9B14) | Sapphire Rapids (8481C) | Haswell-era Xeon (n1) |
|---|---|---|---|---|---|
| MF, median-rule mix | 2.3 | 4.3 | 4.3 | 3.5 | 6.8 |
| BE, median-rule mix | 4.6 | 7.7 | 7.7 | 8.1 | 15.7 |
| X, 121 rules | 12.2 | 23.8 | 23.2 | 23.1 | 43.5 |
| MF, 121 rules | 16.7 | 36.5 | 35.5 | 33.6 | 61.9 |
| BE, 121 rules | 41.1 | 63.7 | 60.2 | 77.3 | 131 |
| BE start (Elias–Fano select) | 9.1 | 11.5 | 11.1 | 12.9 | 21.9 |
| MF end (gap: reads the next start) | 6.9 | 17.0 | 16.6 | 14.5 | 25.4 |
| B subtree end (`findclose`) | 6.5 | 11.8 | 12.2 | 17.0 | 24.7 |
| shape 2 load, MF | 30.6 | 48.4 | 45.2 | 52.0 | 85.8 |

- **The ranking holds on x86:** X ≈ MF on the median rule, and the B family 2–3× slower. The cloud
  cores are about 2× slower than the M5 a call, and the Haswell-era Xeon 3×.
- **The baseline build costs Elias–Fano 2×** (Zen 3: BE start 26.1 → 11.5, 121 rules 98 → 64).
  Without POPCNT the select's counts are done in software. So x86 builds need x86-64-v2 (POPCNT,
  every Intel since Nehalem) at least. With v3 (BMI2), the select is PDEP.
- **MF's gap end is its weak point on x86:** 14.5–17 ns against X's 6.3–6.5, because the gap needs a
  second random read. On the 121-rule mix that makes MF 1.4–1.5× X on x86, against 1.4× on the M5.
  XF with lengths (XF in the tables: 4.15 B/node) avoids it.
- **`findclose` is relatively worse on Sapphire Rapids** (17 ns against Zen's 12).

### What the third round says

- **MF is the X family's best: 3.37 B a node on data-layer at X's speed,** against format 15's 9.00
  and BE's 2.43. Its median-rule mix is 2.3 ns a node (BE 4.6). On all 121 rules it is 16.7 (BE 41.1).
- **The B family still costs 2–3× on structure questions** and saves only 0.65 B against XF's
  structure.
- **Still to measure:** M1 and M3 (the pack is ready); the forest's children walk as a kernel one
  group a node.
