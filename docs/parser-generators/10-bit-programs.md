# Parser generators: lowering Bitlex to bit operations through the pipeline

Status: architecture and outline, 2026-11-06. Nothing here is built. It gathers what `00`–`09` and
the engine's spike notes already decided, says what the latest direction changes, and lists the
questions still open. It replaces `09-compile-and-lower.md` §3–§6 and §11.

**Corrected by `12-what-is-wanted.md`:** the bit program is a side program of the existing
transducer `Program`, as `value_programs` are, not a second program kind beside it. Where this
page says otherwise (§1 "program kind", §3.2), `12` wins.

## Intro

Three builds were reverted, each because it ignored work already done:

- the first walked bytes and events;
- the second ran per-byte integer lanes in scalar loops;
- the third built a parallel Rust tree beside the pipeline.

This page starts from that earlier work.

The direction, as stated:

- "there should be NO byte-by-byte operations or event-by-event operations";
- "lower into branchless, loopless bit (mostly) operations";
- "lower through the same Metal / JIT pipeline, ideally through transducers";
- "reuse EVERYTHING POSSIBLE in the existing pipeline";
- the code in simdjson's shape: its ergonomic operations, and a chunk size per architecture.

## 1. Already decided

These are settled. Each row cites where.

| topic | decision | where |
|---|---|---|
| program kind | **corrected (`12` §3):** a side program of the transducer `Program`, as `value_programs` are, read by the transducer's own interpreter, native IR, Cranelift and Metal. `01` §4 and `09` §7's "second program kind" was not what was asked | `12`; `01` §4 for its operations |
| what it shares | plan ids, interpreted first with the JIT built in the background, backend selection, the Metal pipeline cache, the launch budget, residency slabs; the set program's slot layout and barrier pattern | `09` §7; decision 0012 |
| executors | as for tape transducers: one verified program is the only semantic input. The CPU and Metal interpreters read it directly. One ephemeral native IR is emitted by both Cranelift and MSL, with one control-flow structure, no host callbacks and one packed ABI. Every op exists on every backend, with no Metal-to-CPU fallback | t6 §6 (`transducer-native-jit-design.md`); `final-shape/03-architecture.md` lines 88–90 |
| what "no branching" allows | loops whose trip count is a size; selects, table lookups, gathers, scatters; one decision per file (the error bit) | `01` §1 |
| masks | `class`: nibble lookups (`vqtbl1q_u8`), up to 8 classes per pair. `toggle`: carry-less multiply (`vmull_p64`); on Metal, the popcount of a ballot below the lane (Metal has no `simd_prefix_exclusive_xor`). `fill`: the borrow form. `back_fill`: `fill` on bit-reversed words. `after_odd_run`: simdjson's add-with-carry. `prev`, `next`: shifts with the neighbouring word | `03` §3.1 |
| carries | each carry has a summary, an associative combine and an apply. Only summaries cross blocks; masks are recomputed, not stored | `06` §2 |
| hardware levels | NEON: a `u64` per mask, carries in registers, a chunk per core. Metal: a thread per 64-byte word, `simd_prefix_exclusive_*` per SIMD group, a threadgroup scan, decoupled look-back with bounded waits between threadgroups | `06` §5 |
| corpus as one stream | files one after another in a slab; every scan segmented at file starts; nothing speculative crosses a file | `06` §5.1 |
| machines | lowered by Krohn–Rhodes decomposition to a cascade of `fill`s (resets) and group scans (`toggle`, prefix sums mod `n`). The table scan is kept only for non-solvable groups, and no lexer has one | `08` §2.1 |
| `reflect` | `toggle` for the sign, then a prefix sum mod 3 over two bit-planes: a Hillis–Steele scan, six rounds per word | `08` §2.2 |
| hypotheses | every lane computed for every line; the segment's lane map composed by a scan of selects | `08` §7; `03` §3.5 |
| rounds | unrolled to a fixed count; an error bit if the last round still changes; `until settled` may stop early | `08` §7; `03` §2.7 |
| `ucode` | decoding gated by an ASCII-fact mask; lead bytes gathered; a branch-free decode; a two-level table | `04` §3.3 |
| the tape | columns from extents by one prefix sum and gathers; the code table numbered by first use with a carried bitset | `01` §2; `06` §4.4 |
| oracle | `tools/bitlex`'s evaluator, whose tokens equal tree-sitter's on all JSON files and 13,042 of 13,042 and 1,990 of 1,993 Python files | `03` §5; `08` §9 |

## 2. What the direction changes

The earlier docs allowed work per event and per token, over compacted lists. The direction now
excludes event-by-event work and asks for bit operations where possible. Each row changes a
decision of §1's sources.

| construct | was | becomes | why |
|---|---|---|---|
| `count` | a prefix sum in bytes; the floor as a composed map | **bit-planes**: `w = ⌈log2(max + 2)⌉` planes, a Hillis–Steele scan of a `w`-bit adder, with clamps as composed maps where needed. Bounded by the declared `max` | the same method as `08` §2.2's mod-3 planes |
| `frame`, `since` | matching by depth, a `last[depth]` vector of positions | one `fill` per depth level up to `max + 1`, from `c == L` on the planes | bit operations, no positions |
| `machine` | compacted events, 16-byte maps composed by `TBL` (`03` §3.2) | Krohn–Rhodes only (`08` §2.1). JSON's `lex` looks like a `Z2` (`q` swaps code and string) with resets (comments), so `toggle`s and `fill`s; the generator confirms it | removes the event list |
| `keywords` | a perfect hash per token (`03` §3.3) | shift-and over the keyword texts: each byte's class, shifted and ANDed | removes the token list |
| `munch` | three fixed applications over candidates | the same, as masks | unchanged |
| rounds over gathered lines and bodies | rounds over a compacted list of the lines and bodies that need them (`08` §7) | rounds over every block, fixed work | removes the gather; costs the round work on every block (Q3) |
| tokens to the tape | compaction of token starts into a list (`03` §3.3) | kept only at the end, as `01` §2's prefix sum and scatter | the only per-token work is the output |

Nothing else changes. The nibble-table `class` stays: it is simdjson's own method, branch-free,
and cheaper than any bit-plane alternative.

## 3. The stream transducer program

### 3.1 Its operations

A program is a straight-line list of operations over 64-bit masks, one word per 64-byte block,
with typed carries. Its operations are `01` §4's, restricted by §2:

| operation | carry | summary, combine |
|---|---|---|
| `class(table)` | none | — |
| `and`, `or`, `xor`, `andnot`, `not`, `select` | none | — |
| `shift(±k)` | `k` bits (a halo) | — |
| `scan_xor` (`toggle`) | 1 bit | parity, XOR |
| `scan_borrow` (`fill`, `back_fill`, `after_odd_run`) | 1 bit | generate and propagate |
| `scan_planes(group)`: prefix sums over bit-planes mod `n`, or of clamp maps | the group element: a few bits | the group operation |
| segmented forms of the scans | as the scan | reset at a flag |
| `reduce_or` per file | 1 bit | OR |
| `ucode` | the 3-byte halo | — (§4, Q1) |
| per-chunk popcounts of the output planes, for the extract pass (§4.1) | a count | add |

There is no operation that loops over bytes, events or tokens, and none that branches on data.
The verifier refuses anything else when a program is made.

### 3.2 Executors

The executors are the transducer's own (§1, "executors"), extended for the side program as they
are for `value_programs` (`12` §3):

1. **The CPU interpreter** reads the program and runs each operation over a chunk of words: one
   dispatch per operation per chunk (`09` §4.1).
2. **The Metal interpreter** is the set program's kernel shape: a threadgroup per stretch of the
   stream, every operation in order, a barrier between them. It is extended with the scans' SIMD-group
   and threadgroup carries (`06` §5).
3. **The native IR** is one straight-line block per program. Cranelift emits it as scalar `I64`
   code, which the transducer JIT already emits. MSL emits it with one thread per word. Both come
   from the same IR, with no host calls, as t6 §6 requires.
4. **Plan identity and background build** as `compilation.rs` does for tape transducers: the
   interpreter first, the JIT swapped in at the next batch.

### 3.3 Width per architecture

The block is 64 bytes on every target. The width is how many blocks one instruction carries.
Blocks from different chunks of the stream run side by side, so no carry crosses between them.

| target | blocks per instruction |
|---|---|
| scalar `I64` | 1 |
| NEON `uint64x2_t` | 2 |
| AVX2 / AVX-512 | 4 / 8 |
| Metal | 32 per SIMD group, one per thread |

This is the role of simdjson's step size per architecture (`09` §1).

## 4. Decided (2026-11-06)

| | question | decision |
|---|---|---|
| Q1 | `ucode`'s per-character work for non-ASCII code | **keep** `04` §3.3's design: leads gathered, decoded without a branch, two table loads per character |
| Q2 | `until settled` | **yes**: an early exit once per chunk per round, never per byte |
| Q3 | rounds over every block, or a gather of the lines and bodies that need them | **every block**, with Q2's exit per chunk. Most chunks settle in one or two rounds (`03` §5), so no event list is needed. The cost table (§5 step 3) shows whether rounds dominate; the gather is reconsidered only if they do |
| Q4 | the output | a final **extract pass**, shaped differently from the mask passes (below) |

### 4.1 The extract pass

The mask passes end with planes: token starts and continuations per kind, node starts and ends,
errors. The tape needs positions and columns. The extract pass turns one into the other.

- **Its sizes come from the passes before it.** Each mask pass also yields, per chunk, the
  popcounts of the planes the tape reads: tokens, nodes, codes used. A scan over those gives every
  chunk's output offset and the file's totals before extraction starts. The tape is allocated once,
  at its final size, and nothing grows.
- **Its shape is per output element, not per block.** Each chunk writes its elements from its known
  offset:
  - on the CPU, simdjson's flattening (`bit_indexer::write`: trailing zeros, clear the lowest bit,
    a fixed unroll per block);
  - on Metal, a prefix count within the threadgroup and a scatter.
- **What it writes** is `01` §2's columns: `ord` from the prefix count, `size` and `tail` by
  gathers at `end`, `base` and `off` per 16 ords, and codes numbered by first use (`06` §4.4).
- **It is the only pass whose work depends on the data,** in proportion to the elements it writes.
  It runs once, at the end.

## 5. Outline of the work

Each step closes only when its check passes, and with a table of what is done, changed or missing.

1. **The program and its verifier,** in the engine beside the tape transducers: the operations of
   §3.1, their carries, and a printer. *Check:* the verifier refuses every other operation; every
   operation equals a scalar reference on random words and at every block edge.
2. **The CPU interpreter**, one dispatch per operation per chunk. *Check:* as step 1, plus segmented
   files.
3. **Lowering from Bitlex,** one construct at a time, in the order of §2's table and `03` §3.1.
   *Check:* each construct equals the evaluator on small inputs, with its cost per block printed.
4. **JSON, then Python, whole,** on the interpreter. *Check:* equal to the evaluator on all corpora;
   every file split at every offset in a window; CPU time.
5. **The native IR, Cranelift, and plan identity.** *Check:* equal to the interpreter; CPU time,
   with compile time reported apart.
6. **Metal:** the interpreter kernel, then MSL from the native IR. *Check:* equal to the
   interpreter; the launch budget; the corpus as one stream.
7. **The extract pass** (§4.1): sizes from the mask passes, then the tape. *Check:* tapes equal to
   today's, byte for byte; no reallocation.
