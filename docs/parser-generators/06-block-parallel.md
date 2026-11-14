# Parser generators: blocks in parallel

Status: design, 2026-11-06. It applies to the whole pipeline of `01-branch-free-design.md` and
`02-python.md`, and to the lexers of `03-bitlex.md`. The Python lexer described here still had the
`strings` and `fstrings` machines. Without them its carries are those of
`08-without-automata.md` §6 and §8: parities, residues mod 3, counts and one map on three lanes.

## Intro

Almost all the work of turning source into a tape depends only on bytes near each position. A
small part depends on everything before it:

- whether a byte is inside a string;
- the state of an automaton;
- the bracket depth;
- a node's preorder number.

This document splits the pipeline along that line. The text is cut into blocks, and each block
does three things:

1. **All its local work, at once and without coordination.** It also computes a *summary*: what
   the block does to each carried quantity, as a function of that quantity's value at its start.
2. **A short scan over the summaries.** This combines them, so each block learns its *carry-in*,
   the carried values at its start. This is the only step that crosses blocks, and it reads one
   summary per block, not the text.
3. **Applying its carry-in** with selects, adds and table lookups, and writing its output.

The carry that passes between blocks is small. For the whole Python lexer it is about 30 bits
plus two counters. Every carry kind combines associatively, so the scan is a parallel prefix
scan. On Metal it runs in one pass with decoupled look-back. The same scheme treats a whole corpus
as one stream of blocks, with file boundaries as segment flags, so a thousand small JSON files fill
the device as well as one large file does.

## 1. Three kinds of work

| kind | examples | across a block edge |
|---|---|---|
| **local** | byte classes; `prev` and `next`; byte-pair events such as `star & next(slash)`; `span`, `starts`, `lasts`; UTF-8 lead bytes; keyword hashes once a token's bytes are known | read the neighbouring bytes, a *halo*. The input is in shared memory, so a halo is a read, not a carry |
| **carried, from the input only** | `after_odd_run` (escapes), `toggle` of a mask of input bytes, `line_has`, the column of a byte in its line | a carry whose summary is a function of the block's bytes alone |
| **carried, from earlier results** | an automaton's state; `fill` and `toggle` over masks the automaton produced; bracket depth over brackets in code; node counts; nearest smaller values | a carry whose summary needs the earlier result, so it waits for that result's scan |

Local work is most of the work: every class lookup, every shift and logic operation, and every
operator in the `munch`. It runs in phase 1 with no carry at all.

**Halos are short.** The widest look in the two lexers is 10 bytes: `next(hexdig, 9)` for `\U`
escapes. The `munch` needs about 9: three rounds of operators up to 3 bytes long. Reading 16 bytes
past each side of a block covers both.

## 2. Carries: summary, combine, apply

Each carried operation has three parts:

- a summary that phase 1 computes per block;
- an associative way to combine two summaries;
- a way to apply a carry-in in phase 3 without redoing phase 1's work.

| carry | summary of a block | combine | apply the carry-in | size |
|---|---|---|---|---|
| parity (`toggle`) | XOR of the block's mask bits | XOR | XOR the block's result with all ones if the carry is 1: a select | 1 bit |
| borrow (`fill`, `back_fill`, `after_odd_run`) | generate: does a run start in the block and reach its end? propagate: does the block have no start and no end? | carry-lookahead: `g = g2 \| (p2 & g1)`, `p = p1 & p2` | OR in the fill from the block's first byte to its first end, if the carry is 1 | 1 bit |
| count (`scan(+)`) | the block's total | add | add the offset | 16–32 bits |
| automaton | the block's map: an end state for each start state | compose: `m2 ∘ m1`, by table lookups | look up the start state; walk the block's events from it | 4–6 bits as a carry; the map is N states × 4–6 bits |
| nearest smaller value | per key, the block's first and last position holding a key at most that key | elementwise max (last) and min (first) | the carried `last` vector seeds the block's own scan | L positions |
| code table | the set of grammar codes the block uses, as a bitset | OR | codes not in the carried set are new; they are numbered after it | one bit per grammar code |

Notes on the table:

- **Backward carries.** `back_fill` passes from later blocks to earlier ones. Its scan runs in the
  other direction. It needs nothing from the forward scans of the same round, so both directions
  run at once.
- **Maps are small.** The `strings` map is 13 states × 4 bits = 52 bits, one `u64`. `fstrings`
  is 25 states × 5 bits, one `u128`.
- **Converged maps are smaller still.** 1 KB blocks of CPython keep at most 3 distinct live runs
  in 99.9% of cases (`05-many-states.md` §3). A map can be sent as 3 candidate states plus a
  2-bit index per start state.
- **Phase 3 walks with one state.** Once the start state is known, the automaton needs only one
  table lookup per event, not a map. At one event per 53 bytes that is a short serial walk, with
  no branch.

**Store summaries, recompute masks.** Phase 3 does not read stored masks from phase 1. It
recomputes them. They cost a few instructions per byte, which is cheaper than writing and reading
back a dozen 64-bit masks per 64 bytes. Between the phases only the summaries persist: tens of
bytes per block.

## 3. Rounds

A carry from earlier results cannot be summarised until those results are known. The pipeline is
therefore a short sequence of *rounds*. Each round is phases 1–3, and each one's local work
starts from the resolved carries of the rounds before it.

### 3.1 JSON

| round | resolves | what its local work produces |
|---|---|---|
| 1 | escapes (borrow), in-string (parity) or the `lex` machine (comments variant), number and word fills (borrow) | every token's start, end and kind; the error bits |
| 2 | bracket depth (count); node count `C` (count); the code table (bitset) | each node's start and rank; pairs and their ends, from the depth |
| 3 | bracket matches (nearest smaller depth) | `size`, `tail`, `base` and `off`, then the columns |

Round 1 is simdjson's stage 1, with its two carries passed by a scan instead of a loop. Its
in-string parity depends on the escape carry, but only at the block's first byte. So the block
keeps two parity summaries, one for each value of the escape carry: §3.3's fusing, with two
candidates.

### 3.2 Python

| round | resolves | what its local work produces |
|---|---|---|
| 1 | escapes; `line_has`; the `strings` machine | strings, comments and f-string opens |
| 2 | `fstr` and `raw` fills; the `f1` machine | interpolation code and nested f-string bodies |
| 3 | `f2_str` fill; the `f2` machine | the code mask, complete |
| 4 | word, number and continuation fills and back-fills (both directions); bracket depth; line column | every token; logical lines and their widths |
| 5 | nearest smaller values: brackets, blocks (key `2w + continues`) and binding powers | node extents and ranks (`02-python.md` §4–§6) |
| 6 | node count `C`; the code table | `ord`, then the columns |

### 3.3 Fusing rounds

Six rounds means six passes over the text, but each pass is cheap. Phase 1 of round `r + 1` can
also run in the same pass as phase 3 of round `r`, since both are local work on the same block.

Two rounds can be fused into one scan when the later carry depends on the earlier one through a
small state. The block summary then holds the later summary for each candidate value of the
earlier carry:

- **`f1` under `strings`:** an `f1` summary for each of the ~3 live `strings` runs. This is the
  cascade summary of `05-many-states.md` §2.3.
- **Bracket depth under `strings`:** a depth total for each live run.

The cost is a few times the later summary, so the summary must stay small. Fusing is worth it on
the device, where each round costs a dispatch. Rounds 1–3 of Python can be fused this way.

## 4. What passes between blocks

These are the carry-ins, after the scan. The summaries are larger, as §2 shows, but only the scan
reads them.

### 4.1 JSON

| carry | bits | round |
|---|---:|---|
| inside an odd backslash run | 1 | 1 |
| inside a string (or the `lex` state: 7 states) | 1 (3) | 1 |
| inside a number or word | 1 | 1 |
| bracket depth | 8 (the deepest file is at depth 42) | 2 |
| nodes before the block, `C` | 32 | 2 |
| codes seen so far | one bit per code: JSON has 19 kinds, and a few dozen codes with fields and memberships | 2 |
| the open brackets' positions, for matching | at most the depth at the block edge | 3 |

The first three are the whole lexer: **3 bits**, or 5 with comments.

### 4.2 Python

| carry | bits | round |
|---|---:|---|
| inside an odd backslash run | 1 | 1 |
| this line has a quote or `#` so far | 1 | 1 |
| `strings` state (13 states) | 4 | 1 |
| inside an f-string; inside a raw string (fills) | 2 | 2 |
| `f1` state (25 states) | 5 | 2 |
| inside a nested f-string (fill) | 1 | 3 |
| `f2` state | 5 | 3 |
| inside a word, a number, a hex-like number, a sign continuation (fills) | 4 | 4 |
| back-fill carries: float, before a string start, odd number (backwards) | 3 | 4 |
| bracket depth | 8 (deepest 14) | 4 |
| column of the block's first byte in its line; has the line had a token yet | 16 + 1 | 4 |
| open blocks' widths and open brackets' positions | at most 13 and 14 entries | 5 |
| `C`, and the codes seen so far | 32, and one bit per grammar code | 6 |

The lexer's carry is the first nine rows: **26 bits**, one 32-bit word. Then come one depth and
one column counter. Structure adds the open blocks and brackets at the block edge, which are
usually a handful, and the node count.

### 4.3 Matching across a block edge

A bracket opened in one block may close many blocks later. Matching does not need a stack carried
across blocks:

- **Depth first.** Round 4's count scan gives every bracket its depth in the whole file.
- **Unmatched brackets are few.** A block's brackets that have no match inside it are its
  unmatched closes, then its unmatched opens, each list at most the file's depth long.
- **Matching them.** A close at depth `d` matches the latest earlier open at depth `d`. Over the
  compacted lists of unmatched brackets, that is a "previous position with this key" query: the
  vector-max scan of `01-branch-free-design.md` §3, keyed by depth.

Blocks (indentation) work the same way. Their keys are logical-line widths, and the unmatched
lines are those whose block ends past the block edge.

### 4.4 The tape's columns

- **`ord = C[start] + rank`:** `C` is the count carry plus the block's own prefix count.
- **`size = C[end] − ord`:** a gather at `end`, which may be in another block. It runs after
  round 6's scan, when every block's `C` offset is known.
- **`tail`:** a gather of the last descendant's start, likewise.
- **`base` and `off`** group ords by 16. Groups cross block edges, so they are computed over the
  dense node list once ords exist, not over byte blocks.
- **The code table:**
  - a code is new in a block when it is not in the carried set;
  - the new codes are numbered in order of their first local use, after the carried set's
    popcount;
  - that is exactly "numbered by first use", with no global sort.
- **Widths:** a max reduction per file of `size`, `off` and `tail`. The writer picks 2 or 4
  bytes per column per file, which is one decision per file, as fallback is.

## 5. Mapping to hardware

| level | CPU (NEON) | Metal |
|---|---|---|
| 64 bytes | one `u64` per mask; carries in registers | one thread per 64-byte word; masks in registers |
| 2 KB | (sequential within a thread) | a simdgroup of 32 threads: `simd_prefix_exclusive_*` combines summaries |
| 16–64 KB | one chunk per core; phase 1 runs over the chunk's 64-byte words in order, so carries inside a chunk pass in a register | a threadgroup: a scan in threadgroup memory |
| file or corpus | a scan over chunk summaries, then phase 3 per chunk; one core per chunk | decoupled look-back (Merrill and Garland, 2016): each threadgroup publishes its summary, then its inclusive prefix, and reads its predecessors' as they appear. One dispatch per round |

- **Bounded waits.** Decoupled look-back waits on flags that earlier threadgroups set. Rerail's
  Metal code allows no unbounded wait, and Metal does not promise that an earlier threadgroup
  makes progress. So the wait is a loop with a fixed count. If a predecessor's summary has not
  appeared by then, the waiting threadgroup computes it from that predecessor's input itself.
  This is the "decoupled fallback" variant, used by GPU renderers that cannot rely on forward
  progress.
- **Within a CPU thread, phase 2 is free.** Carries pass in a register from one 64-byte word to the
  next, as in simdjson. The three-phase form is only needed between threads, and there the
  summaries are per chunk, not per 64 bytes.

### 5.1 The corpus as one stream

Files are placed one after another in a slab (decision 0032). Every scan is *segmented*: a file's
first block has a segment flag, and the combine rule resets at a flag.

- **Parity, borrow and count** reset to 0.
- **An automaton** resets to its start state: the map becomes the constant map to `start`.
- **Nearest smaller values and the code table** reset to empty.

Small files share threadgroups. The data-layer JSON corpus has 14,669 files averaging 39 KB. Many
are far smaller, and per-file dispatch would leave most of the device idle. As one stream they are
about 8,700 threadgroups of 64 KB.

Reductions are segmented too:

- **Error bits:** an OR per file. A file with a set bit is re-parsed by tree-sitter, and its
  output is discarded.
- **Width maxima:** a max per file.

Nothing speculative crosses a file boundary. A file that falls back does not disturb its
neighbours.

## 6. Cost

Per byte, phases 1 and 3 do the local work twice, since phase 3 recomputes the masks. They also
read the carry-ins and apply them with selects and adds. Per block, phase 2 reads one summary.

| | per 64 B | per 64 KB chunk |
|---|---|---|
| local mask work, Python | an estimate of 100–200 instructions, done twice | |
| events, Python | ~1.2 automaton steps | ~1,200 |
| summary, Python lexer | | ~45 bytes: 8 (`strings` map) + 32 (`f1` and `f2` maps) + flags |
| summary, structure | | depth, `C`, a few unmatched brackets and blocks, a code bitset |

So the summaries are well under 0.1% of the input, and phase 2 is negligible beside phases 1 and 3.

What to measure first, on the corpora:

- **The local fraction:** phase 1 and phase 3 time against total time, single-threaded. This shows
  what recomputing costs.
- **Scaling:** the CPU over 1, 2, 4 and 8 cores, chunked, against simdjson's single-threaded
  stage 1.
- **Unmatched brackets and blocks** per edge, at 4 KB, 16 KB and 64 KB blocks.
- **Convergence** of `f1` alongside `strings`, which decides whether rounds 1–3 fuse.
