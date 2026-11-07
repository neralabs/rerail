# Parser generators: tier 2, designed before it is built

Status: design, for review (2026-11-07). Nothing here is built. It replaces the CPU tape
writer, which interpreted rows in Rust and was deleted.

## Intro

A Bitlex file compiles to two tiers (decision 0049):

- **tier 1** is bit operations on every byte: classes, strings, each token's first and last
  byte, and per-byte errors;
- **tier 2** is the per-token rules of the tree section: each token's kind, which nodes open
  and close, nesting, fields, checks, and the tape.

Both tiers are generated code, compiled from the Bitlex file. No executor interprets rows.
Tree-sitter's tapes are the reference.

This page designs tier 2, and how both tiers run on Metal and on the CPU.

```
Metal: one kernel, a threadgroup per run of files, in rounds split by barriers
  1. per block, in parallel: bit operations; small accumulators to threadgroup memory
  2. barrier
  3. scans over the accumulators: carries, offsets, the open-node stack at each block
  4. barrier
  5. per output, in parallel: each block's tokens, then each node's record
CPU: one generated loop per file; blocks in order, tokens in order, a stack in registers
```

## 1. What tier 2 needs from the rest of the file

Each tier-2 rule reads something that may lie in an earlier block. Each is one kind of
accumulator, with an associative combine, so a scan resolves it:

| rule in the tree section | what crosses a block edge | accumulator | combine | JSON size |
|---|---|---|---|---:|
| tier 1's carries (`escaped`, `toggle`, `fill`) | the carry | borrow or parity | carry-lookahead, XOR | 3 bits |
| `prev(x, k)` | the top `k` bits of `x` one block back | none: the lane before, by a SIMD shuffle | | 0 |
| a node's place in preorder (`ord`) | nodes before the block | count | add | 12 bits |
| `after K` | the last token's kind before the block | last present | right if present | 5 bits |
| a node without nesting that spans blocks (a string) | its open start's `ord` | last present | right if present | 32 bits |
| `nested max D`, `parent K`, `end with F` | the open nodes at the block's start | stack effect: pops, then pushes | cancel the middle | see §2 |
| errors, checks | any error | flag | OR | 1 bit |
| codes numbered by first use | codes seen | bitset | OR | 64 bits |

All of it comes from the tree section's rows. Nothing in it is specific to JSON.

## 2. The open-node stack

`parent K`, `end with F` and `nested max D` all ask which nodes are open at a token. That is a
stack. Its effect over a block is a pair of lists: the nodes it closes that opened before it,
then the nodes it opens and leaves open. Two such effects combine into one. The closes of the
second cancel against the opens of the first.

The bound comes from the Bitlex file. JSON's nested kinds are `object` and `array`, at most 13
deep. A `pair` sits under each `object`, and `document` is at the bottom. So the stack holds at
most 28 entries of 5 kinds, 3 bits each:

| part of the stack effect | JSON |
|---|---:|
| closes: a count and the kinds they close | 5 + 84 bits |
| opens: a count and their kinds | 5 + 84 bits |
| total | about 23 bytes |

Most blocks open or close a few nodes, so the compiler may pack the common case smaller. A
stack effect that overflows its bound is a file for tree-sitter, as `nested max D` already
says.

**Matching a close to its open.** A node's `size` is written where it closes. It needs the
`ord` of its open, which may be in an earlier block:

- **in the same block:** the lane's walk keeps the open `ord`s in registers;
- **in an earlier block of the same pass:** each lane publishes its unmatched opens, by depth,
  to threadgroup memory. A close at depth `d` takes the latest earlier open at depth `d`. That
  is one prefix max per depth, so `D` scans of one pass's lanes (`06-block-parallel.md` §4.3);
- **in an earlier pass:** the stack of open `ord`s at the pass's start, at most 28 words in
  threadgroup memory. The threadgroup updates it at the end of each pass.

## 3. The kernel

One threadgroup owns a run of files. Its 1,024 lanes each take a 64-byte block, so a pass
covers 64 KB, and the threadgroup loops passes to the run's end. Scans reset at each file's
first block. The corpus's median file is 3.7 KB, so most threadgroups hold many files.

One threadgroup owns all its blocks, so every round ends with a threadgroup barrier. Nothing
waits on another threadgroup, so there is no look-back and no polling.

### Round 1: tier 1's carries

- **Per block:** the bytes, the classes, and the summary of each carry. Where one carry
  depends on another at the block's first byte, the summary holds both candidates. For example,
  the string toggle depends on whether the first byte is escaped (`06` §3.1).
- **Per SIMD group:** the 32 lanes' summaries combined with SIMD prefix operations. One entry
  per 2 KB goes to threadgroup memory.
- **Barrier, then a scan** over the entries.

### Round 2: tier 2's accumulators

- **Per block:** the bytes are read again and tier 1's masks are recomputed with their
  carry-ins. Recomputing is cheaper than storing a dozen masks per block (`06` §2). The lane
  takes its node count, last token kind, stack effect, errors and code bits. The stack effect
  comes from a walk over the block's bracket-like tokens only, a few per block.
- **Per SIMD group and threadgroup:** as in round 1.
- **Barrier, then a scan.** Each block now knows its `ord` offset, the kind of the token before
  it, and the stack at its start. Each file knows its node total, so the `size` column's width.

### Round 3: the tokens

- **Per block:** the masks once more. Then a loop over the block's token starts, at most 64,
  in order. This is the generated per-token code, one straight-line case per kind:
  - the kind, from the first byte, or a keyword compare of up to 8 bytes;
  - the checks, such as `json_number`, over the token's bytes in 8-byte chunks;
  - the nodes that open here, by `after`, `parent` and rank, each with its `ord`, code and
    start;
  - the nodes that close here: `size` and `end` into the open's record;
  - unmatched opens and closes, kept for §2's match.
- **Records go to a scratch area** in device memory: code, start, end and size, 4 bytes each.
- **An `atomic_min` of `ord` per code** gives the first-use order. An `atomic_max` per column
  gives its width.
- **Barrier.**

### Round 4: the tape

- **Per node, in parallel:**
  - its code, renumbered by first use (a few dozen codes, sorted by one thread);
  - `base` for each 16 nodes, and `off = start − base`;
  - `tail`, from the start of the last node inside it;
  - every column at its final width.
- **No guesses.** Round 2's scan gives the counts, and round 3 gives the widths and the
  numbering. Only the host's buffer capacity is guessed: from the file's length at the
  language's measured density. A file that overflows it goes back with the true size.

### Threadgroup memory

Apple GPUs give a threadgroup 32 KB. JSON's entries per 2 KB SIMD group:

| | bytes |
|---|---:|
| tier 1's carries | 1 |
| node count, last kind, the open string's `ord`, errors | 8 |
| the stack effect | 23 |
| the code bits | 8 |
| total | 40 |

So 32 KB holds 800 entries, which is files up to 1.6 MB. Of the 774 corpus files, 5 are
larger, up to 8.7 MB. For those, the entries go to device memory, still owned by one
threadgroup.

## 4. The CPU

One generated function per program, as tier 1 already is. It loops over a file's blocks in
order, and the carries stay in registers:

- for each block, tier 1's masks, as now;
- for each token start in the block, the same per-token cases as round 3, with the open-node
  stack in registers, bounded by the Bitlex file's `max`;
- records written directly at their final widths. The CPU knows a file's total only at its
  end, so it writes 4-byte columns and narrows them in place if they fit.

## 5. The compiled form

Tier 2 compiles to rows of the transducer's `Program`, beside the block program, as its other
side programs are (`12-what-is-wanted.md` §3). They are printed and assembled by `rir`, and
covered by the digest and `validate`:

- the accumulators: each one's kind (count, last present, stack effect, flag, bitset) and its
  width;
- the per-kind cases: what each token kind opens and closes, and under which conditions;
- the bounds: the stack's depth and kinds, and the scratch record's layout.

Cranelift and the MSL generator each turn these rows into code. Neither interprets them at run
time.

## 6. How it is checked

- **Tapes equal tree-sitter's,** byte for byte, on the corpus. Every file goes either to a tape
  or to tree-sitter.
- **Metal's tapes equal the CPU's,** on every test input.
- **Shape guards on the generated code:**
  - round 3's loop has one case per token kind, and no loop over rows;
  - nothing is allocated per token;
  - the threadgroup memory per entry is within budget;
  - no wait on another threadgroup.
- **CPU time:** tier 2 next to simdjson's stage 2, about 12 ns per 64 bytes.

## 7. Open questions

1. **Fusing rounds 1 and 2.** JSON's tier-2 accumulators depend on tier 1 only through
   in-string state. With two candidates per entry, round 2's summaries could be made in round 1,
   saving a pass over the bytes. That doubles the entry size.
2. **Very large files.** One threadgroup owns a file, so an 8.7 MB file runs on one GPU core.
   Such a file could be split across threadgroups with one extra dispatch between rounds, the
   dispatch boundary being the barrier. Is that worth building now, or only when a large file
   shows up in the timings?
3. **The stack effect's size.** 23 bytes is the worst case. A compact form for the common case
   (a few opens and closes) with an overflow flag might keep it to 8 bytes. Overflowing blocks
   would then take the device-memory path.
4. **Keeping a mask between rounds instead of recomputing it.** This is chosen per mask by
   cost (`13` §5.3). JSON's tier 1 is cheap enough to recompute.
