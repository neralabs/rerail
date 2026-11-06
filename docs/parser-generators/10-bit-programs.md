# Parser generators: Bitlex as bit programs

Status: architecture and outline, 2026-11-06. Nothing here is built. It replaces the execution
parts of `09-compile-and-lower.md` (§3–§6). Three earlier builds were reverted:

- one walked bytes and events;
- one ran per-byte integer lanes in scalar loops;
- one built a parallel Rust tree beside the transducer pipeline instead of lowering through it.

This document is the shape to build instead.

## Intro

A Bitlex lexer says, for every byte of a file, which classes it belongs to, which tokens start or
continue there, and where errors are. The aim is to compute all of that the way simdjson computes
its structural characters: 64 bytes at a time, as 64-bit masks, with no loop over bytes and no
branch on the data.

The proposal in one paragraph:

- Every `.bitlex` file lowers to a **bit program**. A bit program is a straight-line list of steps.
  Each step reads one or two 64-bit words per 64-byte block and writes one.
- The steps are boolean word operations, plus four that pass a small carry from one block to the
  next: shifts, add-with-carry, prefix XOR, and reading the block before or after.
- Everything that is not a mask in Bitlex becomes masks too:
  - a byte is 8 bit-planes;
  - a count is a few bit-planes, holding its value in binary;
  - a machine's state is a few bit-planes.

  Their arithmetic is boolean networks over those planes.
- The program is a transducer-pipeline program. It runs where the pipeline's node-set programs
  already run: the Metal set program, the CPU interpreter, and the Cranelift JIT. It is extended
  with the carry steps, not rebuilt beside them.
- Only a short list of exceptions is not a bit operation (§6). Each needs approval before it is
  built.

## 1. What a bit program is

### 1.1 Values

A **plane** is one bit per byte of the input: one `u64` per 64-byte block. It is the only value
type. Every file starts at a block boundary, as the Metal set program's files start at a word
boundary.

| Bitlex value | as planes |
|---|---|
| a byte | 8 planes, bit `k` of every byte (`B0` … `B7`) |
| a class, a mask | 1 plane |
| a count with `max D` | `w = ⌈log2(D + 2)⌉` planes: the count in binary, one plane per bit |
| a value mod 3 (`reflect`) | 2 planes |
| a machine with `S` states | `⌈log2 S⌉` planes: the state number in binary |
| a hypothesis lane | as a machine: the lane number in binary |

### 1.2 Steps

A step is `[op, out, a, b]`, the Metal set program's encoding. The set program's steps stay as
they are: clear, all, copy, or, and, and-not, within. These are added:

| step | meaning per block | carry to the next block |
|---|---|---|
| `xor`, `not` | word logic | none |
| `shl k`, `shr k` | bit `i` is bit `i − k` (`i + k`) of `a`, from the block before (after) | `k` bits |
| `add` | `a + b` as one 64-bit number per block | 1 carry bit |
| `prefix_xor` | bit `i` is the XOR of bits `0..=i` | 1 parity bit |
| `first`, `last` | a file's first or last byte | none |

These are enough. A fill, a toggle, `after_odd_run` and every scan below are sequences of these
steps.

- **Backwards scans.** `back_fill` is `add` on the blocks taken last to first. The step carries a
  direction bit, so a file's carry runs from its end.
- **No other kind of step.** There is no loop, no branch and no per-byte access in the step set.
  The verifier refuses any other op, so the shape is checked when a program is made, not by
  reading source code.

### 1.3 The cost of a program

Each step costs a fixed number of word operations per block, whatever the data. A program's cost
per block is the sum over its steps. The lowering prints it for each Bitlex statement, so the cost
of every construct is visible before anything runs.

## 2. Lowering each construct to steps

The counts below are word operations per 64-byte block, per instance, before sharing equal steps.
They are estimates to check when the lowering exists.

| Bitlex | steps | ops per block |
|---|---|---|
| reading the input | the bytes transposed into planes `B0`–`B7` (§4.1) | about 60 SIMD instructions, once per block for all classes |
| `class` | a boolean function of `B0`–`B7`, minimised per class; classes share terms | 5–15 |
| `text("abc")` | each byte's class, shifted and ANDed (shift-and) | about 3 per byte of the text |
| `prev`, `next`, `starts`, `lasts`, `after`, `span`, `aligned` | shifts and logic | 1–5 |
| `fill(s, e)` | `add` with masks around it (one carry bit) | about 5 |
| `back_fill` | the same, backwards | about 5 |
| `toggle` | `prefix_xor` | about 7 |
| `after_odd_run` | simdjson's odd-backslash sequence: `add`, logic | about 10 |
| `count(up, down) max D` | the running count as `w` planes (§2.1) | about 120 with `w = 4` and no clamp; about 400 with clamps |
| comparisons on a count (`c == L`, `c ≥ L`) | a comparator network on the `w` planes | about `2w` |
| `frame(c, m)`, `since(c, m)` | for each level `L ≤ D + 1`: `c == L` on the planes, then one `fill` | about `(2w + 5)(D + 2)` |
| `head(a, C)` | one shift and select per value of `C` up to `D` | about `3(D + 1)` |
| `reflect` | a running sum mod 3 on 2 planes (§2.2) | about 60 |
| `machine` with `S` states | the running state as `⌈log2 S⌉` planes (§2.2) | about `18 S² ⌈log2 S⌉`: 576 for 4 states, about 2,600 for JSON's 7 |
| `hypotheses … split E lanes` | every lane's body; the lane choice as a machine over lanes; then a select per lane | the bodies, plus a machine of `E` states |
| `rounds N until settled` | the body's steps repeated `N` times | `N` times the body (§2.3) |
| keywords, `munch` | shift-and over the keyword texts; longest match by masks | about 3 per keyword byte |
| `error` | the error's mask; whether a file has one is a `within` | 1–2 |

### 2.1 Counts as bit-planes

A count's value at each byte is held in binary, one plane per bit. Its running value across a block
is a **scan of steps**: each byte adds `+1`, `−1` or `0`, or resets.

- **Without clamps.** The running value is a prefix sum. It is computed in six doubling stages of
  a Hillis–Steele scan:
  - stage `d` adds the value `d` bytes before (`shl d` on each plane);
  - each stage is a `w`-bit adder made of word logic, about `5w` operations.

  The carry into the next block is the block's last value: `w` bits, added to every byte by one more
  adder.
- **With clamps** (a floor at 0, a cap at `D + 1`). Each byte's step is the map
  `x ↦ min(max(x + a, lo), hi)`, which is closed under composition. The scan composes these maps
  instead of adding numbers. A map is three `w`-bit values, and composition is adders and
  comparators. It costs about three times the plain sum.
- **When a clamp can be dropped.** The lowering proves where one cannot trigger, for example
  where an error already covers an underflow. There the plain sum is used.

### 2.2 Machines as bit-planes

A machine's step at each byte is a map from states to states, chosen by the byte's classes. The
running state is a scan that composes those maps.

- **Maps as planes.** A map over `S` states is `S` entries of `⌈log2 S⌉` bits each, so
  `S⌈log2 S⌉` planes.
- **Composing two maps** is a fixed network: each entry of the result is the second map's entry
  selected by the first map's. That is a multiplexer tree of word ANDs and ORs. Nothing indexes
  memory.
- **The scan** is the same six doubling stages as for counts. The carry into the next block is the
  block's composed map. That map's entry for the state the block starts in gives each byte's state,
  through one more selection network.
- **Cost grows as `S²`.** The two lexers today:
  - Python has no `machine`. Its scans are `reflect`, `count` and hypothesis lanes.
  - JSON has one machine, `lex`, with 7 states. That is 3 planes and about 2,600 operations per
    block. It runs only for files with a `/` outside strings; other files take `toggle(q)`, as
    simdjson does.

  A machine whose cost is out of line is split into smaller machines, or shown in the cost table.

`reflect` is the same with the group of sums mod 3: 2 planes, and composition is a 2-bit adder
mod 3.

### 2.3 Rounds

A `rounds N` block repeats its steps `N` times, each round reading the last round's `carry`
values. Unrolled, that is `N` copies: straight-line, with no loop.

`until settled` allows the copies after a settled round to be skipped. Skipping is a decision per
chunk, not per byte: one test of "did any carry change in this chunk". Whether that test is
allowed is exception E3 (§6).

## 3. Through the transducer pipeline

The bit program is a transducer-pipeline program, and reuses the pipeline at every level.

| level | what exists | what the bit program adds |
|---|---|---|
| program form | the Metal set program's steps: `[op, out, a, b]` over `u64` slots, files from word boundaries | the steps of §1.2 |
| Metal | one dispatch, a threadgroup per file, every step in order, a barrier between steps | carries between a file's words (§4.3) |
| CPU interpreter | the set program's CPU reference, the node-set operations | the same steps over a chunk of words |
| Cranelift JIT | the transducer JIT, which emits scalar `I64` code | each step as one to a few `I64` instructions; a straight-line function per program |
| plan identity, background build and swap | the pipeline's plan ids and background compilation | the same; a bit program is one more kind of plan |
| residency | source bytes in the device slabs | the bytes as input; planes as slots |
| output | tapes, v9np | the lexer's token and error planes, turned into a tape (§5) |

**To confirm.** The rows "CPU interpreter", "Cranelift JIT" and "plan identity" come from the
earlier survey (`09` §6–§7). A fresh read of the pipeline is under way. That read decides how a bit
program enters plan identity, and whether set-program steps and node-set steps are one list or two
kinds of program sharing an executor. This section is updated with that read before anything is
built.

## 4. Running it

### 4.1 Reading the input

The bytes of a block are transposed into 8 planes once, and every class reads those planes.

- **NEON.** For each bit `k`: shift so bit `k` is the sign bit, compare, and take the mask with
  simdjson's `to_bitmask`. That is about 8 × 7 instructions.
- **Metal.** Each thread holds one block's bytes. `simd_ballot`, or a 64-bit gather of each bit,
  builds the planes.
- **Cranelift.** The JIT emits scalar code, so the transposition is a fixed helper called once per
  block.

The transposition is the only step that reads bytes. It is a fixed data movement, and every
target has a fixed-cost form of it.

### 4.2 Width per architecture

A step is one 64-bit word per block, everywhere. The width per architecture is **how many
independent blocks one instruction carries**. They come from different chunks or files, so no
carry crosses between them:

| target | blocks per instruction | how |
|---|---|---|
| scalar (`I64`, Cranelift today) | 1 | one `u64` register |
| NEON | 2 | `uint64x2_t`: two chunks side by side |
| AVX2 / AVX-512 | 4 / 8 | the same |
| Metal | 32 per SIMD group | a thread per block |

This plays the part of simdjson's chunk size per architecture. The program is the same; only how
many blocks run side by side differs.

### 4.3 Carries

A carry is at most a few bits per step: a borrow, a parity, `k` shifted bits, or a count's or
machine's block summary.

- **CPU.** Blocks run in order within a chunk, and the carry is a register.
- **Metal.** A file's blocks are spread over threads. A carry across them is a SIMD-group prefix
  over the block summaries (`simd_prefix_exclusive_*`), then across SIMD groups through threadgroup
  memory. Every summary combine is the same step network as within a block (§2.1, §2.2). The
  pattern is fixed, with no branch on the data.

## 5. From planes to a tape

The lexer ends with planes: for each token kind, where tokens start and where they continue; for
each error, where it is.

The tape needs positions. Turning a plane into a list of positions is a compaction, and its output
size depends on the data. This is the one place that is not a bit operation (E1, §6). It is done
once, at the end, in simdjson's form: its indexer writes the set bits' positions with a fixed
unroll per block (`bit_indexer::write`). Everything before it stays a bit program.

## 6. Exceptions: what is not a bit operation

Each needs approval before it is built.

| | what | why it is not a bit operation | proposal |
|---|---|---|---|
| E1 | planes to tape positions | the output's length depends on the data | simdjson's `bit_indexer` at the very end, once per token plane |
| E2 | non-ASCII code point classes in `ucode` | a Unicode class over 21-bit code points is too large a boolean function | simdjson's ASCII path per block. A block with a byte ≥ 0x80 reads a two-level table per character start. That is a branch per block and a table read per non-ASCII character |
| E3 | `rounds … until settled` | skipping settled rounds is a branch, taken once per chunk per round | allowed per chunk, never per byte; otherwise all `N` rounds run |
| E4 | transposing bytes into planes | it moves bytes, not bits | a fixed instruction sequence per block on every target (§4.1) |

## 7. Checks

- **The verifier.** A bit program has only the steps of §1.2. Every step's inputs come before it.
  Every carry has a declared width. Because this is checked when a program is made, the "no loops,
  no branches" rule holds by construction, not by inspecting code.
- **The cost table.** The lowering prints operations per block per statement. It is compared with
  §2's estimates, and every construct far off them is investigated before the next one is built.
- **The oracle.** `tools/bitlex`'s whole-file evaluator stays the reference:
  - each construct's lowering is checked against it on small inputs first;
  - then the whole program, on all corpora: errors, token starts and continuations, `munch`,
    `skip`.
- **Block edges.** Each file is also run split at every offset within a window, so every carry
  crosses a block edge at every phase.
- **Backends.** The interpreter, Cranelift and Metal produce equal planes on the corpora.

## 8. Outline of the work

Each step closes only when its check passes, and with a table of what is done, changed or missing.

1. **The pipeline read** (§3). Confirm how a bit program enters plan identity and the executors.
   Update §3. *Check:* §3 has no "to confirm" left, and you agree with it.
2. **The steps.** Add §1.2's steps to the set program's step enum, its verifier and its CPU
   reference. *Check:* each step equal to a scalar reference on random words and at every block
   edge.
3. **The constructs, one at a time,** in this order: classes and the transposition; shifts;
   fills, toggles, odd runs; counts; `frame`, `since`, `head`; `reflect`; machines; hypotheses;
   rounds; keywords and `munch`. *Check:* for each one, the cost table and its equality with the
   evaluator on small inputs.
4. **JSON whole**, then **Python whole**, on the CPU interpreter. *Check:* equal to the evaluator
   on all corpora; block edges; the cost table; CPU time.
5. **Cranelift.** The same steps, as `I64` code. *Check:* equal to the interpreter; CPU time.
6. **Metal.** The set program's kernel with the carry steps. *Check:* equal to the interpreter;
   the launch budget.
7. **Planes to tape** (E1), once approved. *Check:* tapes equal to the current Python and JSON
   tapes.
