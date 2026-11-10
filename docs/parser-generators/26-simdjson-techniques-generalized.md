# Parser generators: simdjson's techniques, generalized for the compiler

Status: 2026-11-10. The catalog and its measurements; transformation 1 done (§5); only the hot
loop inlined (§6); the rest in progress (§4).

## Intro

simdjson's stage 1 is a few dozen instructions per 64 bytes. Its speed is not one trick but
about fifteen, each a choice about what to compute, when, and in what form. Our tier 1 is
compiled from a grammar, not written by hand. So each technique has to become a rule the
compiler applies to any block program: a test that says where the technique applies, and a
transformation that applies it.

This page lists the techniques as simdjson uses them, says how each generalizes, and
measures where each applies in the `.rr` grammar (832 operations, 9 passes, 988 spills). §4
orders the compiler's transformations by measured payoff.

The measurements come from three tools:
- **sparsity** (`rr_sparsity`): for each value, the share of the corpus's 135,889 blocks where
  it has a bit;
- **pressure** (`rr_parser_profile`): the values live in each pass's hot loop, with the
  statement that made each;
- **facts** (`token_local`): which values are local, and which are anchored on token starts.

## 1. The techniques, as simdjson uses them

Each entry gives what simdjson does, the general rule, and where it applies here.

### 1.1 Skip what a block does not have

simdjson's escape scanner returns early when the block has no backslash. Its UTF-8 checker
does nothing for an all-ASCII block but carry one bit:

```
// simdjson, utf8_checker: the common case does no UTF-8 work.
if (simdjson_likely(is_ascii(input))) { this->error |= this->prev_incomplete; }
else { /* the full check */ }
```

- **General rule.** A value that is zero in a block, with its carries in zero, makes zero
  every value it gates: an `and` with it, a `fill` from it, a `prev` of it, and so on. Those
  values, and the values only they read, sit behind one branch on the gate.
- **Where it applies.** Much of `.rr`'s tier 1 is rare:

  | Gate | Blocks with a bit | What it gates |
  |---|---|---|
  | `uesc` (a `\u` in a string) | 0.02% | the whole `\u` check: `h4`, `b_short` … `b_six`, `u_ok` |
  | `tt` (two backticks) | 0.02% | the two- and three-backtick literals |
  | `o_block` (`/*`) | 0% | block comments, inside the regions' fixed point |
  | `id_first & c_f`, `c_s`, `c_o`, `c_m`, `c_j` | under 1% after the letters | the keyword words and their marks |
  | `f_dot`, `sig2` | 0.01%, 0.29% | floats, integers of 19 digits |
  | `at_gap`, `dot_gap & …` | 1.2% | functors named, I/O directives |
- **Status: done** for gates with carried state (§5). Gates of byte classes alone, such as
  `tt`, wait on measured densities.

### 1.2 Classify every byte once, by two table lookups

simdjson's character classes come from one pair of nibble lookups, and each class mask is
one test of the result:

```
// simdjson arm64: whitespace and operators from one lookup pair.
v  = in.bit_and(0xf).lookup_16(table1) & in.shr<4>().lookup_16(table2);
op = v.any_bits_set(0x7);  whitespace = v.any_bits_set(0x18);
```

- **General rule.** Every byte test of a program is a member of some class table. Byte
  sets that are tested together share a table. A mask is made from the shared lookup only
  where a value outside the vectors reads it.
- **Where it applies.** `.rr` has 69 byte tests and position values, many of them single
  letters for the keywords (`c_i`, `c_n`, …), each an `eq`.
- **Status: done.** Class tables, shared products, and values on the vectors until a mask is
  needed (`native_cpu_lanes`).

### 1.3 Carry a few bits between blocks

simdjson's whole state between blocks is three words of one bit each: in a string, escaped,
the last byte a scalar.

- **General rule.** Each carried operation's state is its fewest bits, packed into words.
- **Status: done** (the packed state words).

### 1.4 Look behind, never ahead

simdjson writes every mask at the byte where it is decided from bytes before it:

```
// simdjson: a scalar's start is a scalar not following one.
follows = match << 1 | overflow;  overflow = match >> 63;
potential_scalar_start = scalar & ~follows_nonquote_scalar;
```

- **General rule.** Bitlex's `next(x, k)` is made by delaying the program by `k` bytes. An
  output at delay `d` does not need shifting back in tier 1: its readers can read bit `p + d`.
  An extract already does this (its positions are written less `d`).
- **Where it applies.** The last pass holds one word per delayed output across its loop,
  to shift it back: 13 of its 52 live values.
- **Status: missing** for masks.

### 1.5 One accumulated error, tested once

simdjson ORs every error condition into one word through the whole stage and tests it at
the end:

```
// simdjson: a control byte in a string is an error; noted, not branched on.
error |= strings.non_quote_inside_string(unescaped);
```

- **General rule.** The outputs read only as "any bit in the file" (Bitlex's errors and
  escapes, the fallback) need no alignment and no order. They fold into one accumulated word
  per file, at any delay.
- **Where it applies.** `.rr` has 4 fallback outputs (one per delay), each its own running
  word.
- **Status: partial.** Each total has its own running word.

### 1.6 One stream of structural starts

simdjson's stage 1 writes one index of positions, structural characters and scalar starts
alike. Stage 2 tells them apart by the byte.

- **General rule.** Values tier 2 tells apart by the byte go to one output.
- **Status: done** (the stream of token starts). **Missing** for marks. `.rr` stores 15 mark
  masks a block, most of them almost never set (`options_on`, `over_on`, `no_value` 0%;
  `fix_on` 0.09%).

### 1.7 Write sparse things as positions

simdjson's index is positions, not masks: stage 2 reads only what is there.

- **General rule.** An output with a bit in under a few percent of blocks is written as
  positions, behind a branch, instead of a dense mask word per block.
- **Where it applies.** The 15 marks, the rare ones above.
- **Status: missing.**

### 1.8 The fast carried operations

simdjson's `prefix_xor` is one carry-less multiply. Its escape scanner is an add with carry.

- **Status: done** (`toggle`, `escaped`, `fill`, `back_fill` as one add or multiply a word).

### 1.9 Derive masks from each other

simdjson writes `string_tail = in_string ^ quote` rather than computing it anew. Its masks
are chosen so each is one or two operations from others.

- **General rule.** A local value is a boolean function of few leaves. Its truth table is
  small: the smallest expression for it can be found exactly, and shared across values.
- **Where it applies.** The keyword words are one `and` chain each. `agg_word`'s five
  alternatives (57 operations) share their first letters and their ends.
- **Status: missing** beyond sharing identical expressions.

### 1.10 Recompute, do not keep

simdjson keeps no value from one block to the next but its carries, and recomputes
everything else.

- **General rule.** A value that costs nothing to remake is never stored or held: constants,
  `within`, and the positions.
- **Where it applies.** `within 3` is stored by pass 0 and loaded in pass 2. It is the most
  blamed value of pass 2, and in a full block it is a constant.
- **Status: partial.** Constants and `within` are remade in every pass. A byte check is
  remade in a later pass only where a gate there skips it (§5.2); elsewhere it is stored once
  and loaded at its reader, which measured cheaper than holding it.

### 1.11 Phases with small interfaces

simdjson's stage 1 is a sequence of phases: classify, strings, scalars, structurals, write.
Each phase hands the next a few masks.

- **General rule.** When a body's live values outnumber the registers, cut it where few
  values cross. Each part runs over the blocks of a chunk, and the crossing values are stored
  once and loaded once. A planned store is far cheaper than a spill.
- **Where it applies.** Pass 2 peaks at 85 live integer values, pass 0 at 56 vector ones.
- **Status: partial.** Passes cut only at `back_fill`s. Within a pass, values are made in the
  order they are needed.

### 1.12 Write behind, eight at a time

simdjson writes block `i`'s positions while computing block `i + 1`, eight bits at a time,
and branches only when a block has more than eight.

- **Status: done** for the extract's flattening; the write is in the same block.

### 1.13 Pad, so one body serves every block

simdjson copies the last partial block into a buffer of spaces, so one body runs for every
block.

- **General rule.** A pad byte that changes no output inside the file lets the last block
  run the full body, with outputs cut after.
- **Where it applies.** Each pass has a second body for the first and last blocks. It is cold
  code, and it doubles the code size.
- **Status: not taken.** Bitlex's `~` is a complement inside the stream, so a pad byte is
  neutral only for some programs. It is not a hot-loop cost.

### 1.14 Two blocks a step

simdjson's indexer steps 128 bytes at a time for instruction-level parallelism.

- **Status: not taken.** It doubles the live values, and our bodies are already over the
  registers.

### 1.15 Stage 2 checks what is per token

simdjson checks `true`, `false` and `null` in stage 2, once per atom.

- **Where it applies.** Measured in doc 25 §5: about 11% of `.rr`'s tier 1 now, about 25%
  with checks of the next token.
- **Status:** facts done (doc 25 step 1); the move itself waits on its cost model.

## 2. What the measurements say

The rare work (§1.1) is the largest share. The `\u` check, the backtick literals, the
keyword words, the floats and the long integers are each set in under 1% of blocks. How
many operations they gate is the first transformation's first measurement. Next come the delayed outputs' held
words (§1.4) and the cross-pass constants (§1.10), which are direct register costs.

## 3. Testing

Each transformation is tested three ways:
- **its decision**, on small grammars with a known answer: this region is gated, this one
  is not, this value is recomputed;
- **its soundness**, against the interpreter: the outputs are equal on the corpus and on
  random bytes, on every executor;
- **its payoff**, in the profile: instructions, spills, the hot loop's pressure, and CPU time
  with the load.

## 4. The transformations, in order

| Order | Transformation | Technique |
|---|---|---|
| 1 | Gated regions: an accurate cost (vector work counted), any sparse gate, more regions. Done (§5) | §1.1 |
| 2 | Values remade in each pass, never stored: constants, `within`, positions | §1.10 |
| 3 | Delayed masks not held across the loop. Done (§7) | §1.4 |
| 4 | The fallback's totals folded into one word | §1.5 |
| 5 | Sparse outputs as positions | §1.6, §1.7 |
| 6 | Local values minimized from their truth tables | §1.9 |
| 7 | Cuts where few values cross | §1.11 |
| 8 | Token-level checks to tier 2, by the cost model | §1.15 |

## 5. Transformation 1: gated regions

A gated region is a set of values behind one branch on a gate: in a block where the gate has
no bit, and its carried bits came in zero, the region's values are zero and none of its work
runs. The tests are in `gating_tests`.

### 5.1 What a region is

```
# .rr's `\u` check: set in 0.02% of the corpus's blocks.
mask uesc = f_str & ~o_str & escaped & u
mask h4   = next(hex) & next(hex, 2) & next(hex, 3) & next(hex, 4)
escape "a \\u escape the engine checks" when uesc & ~u_ok
```

The gate is `prev(uesc, k)`. The region holds the values zero with it (`uesc & ~u_ok`), the
values only they read (`u_ok`, `h4`, `b_short` … `b_six`), and the vector work only those
read. Its decisions:

- **The gate.** Two conditions or more, one of them carried state: an escape, a fill, a
  value of a pass before. A byte class, or a run of them, is in most blocks of text. Without
  measured densities nothing better can be known.
- **The cost.** Vector work counts, about four instructions per chunk set.
- **What leaves it.** At most 4 values read after it. Each is held from the region's end to
  its reader: one region of 215 values gave 75 to the body after it, and pass 2's hot loop
  peaked at 110 live integer values.
- **Convexity.** No value outside both reads the region and is read by it. A vector value
  reading another reads its bytes, not its mask, so that edge imposes no order.

### 5.2 Three things a region needs

1. **Its work in its pass.** A byte check is made in the first pass, and its gate is often a
   later pass's: `uesc` reads a string's fill. The passes are planned twice. First, every byte
   check is remade where it is read, to find the regions; then only what a region of a later
   pass skips stays remade, with its operands (`native_cpu_lanes::remade`).
2. **No carried field.** As masks, `next(hex, 2)` is a `prev` of a mask, which carries the
   block's last bits to the next block: a skipped block would leave the carry stale. On the
   vectors, a `prev` reads the bytes before the block and carries nothing. So the values a
   region skips go on the vectors deep, `prev` over `prev`, made at a lag from the bytes that
   far back. Elsewhere logic over a `prev` stays as masks: there one operation does the work
   of four vector ones and the mask (`Plan::of`).
3. **Its place in the order.** A region is a node of the demand order, as a fixed point's
   loop is: made just before the first value that reads what it gives.

### 5.3 Measured

`.rr`, the corpus's 1,346 files, CPU time of tier 1 as one batch, runs alternated at a load of
about 18:

| | Without regions | With regions |
|---|---|---|
| CPU a 64-byte block | 376, 361 ns | 319, 322 ns |
| Pass 0's hot loop | 1,926 points; 77% over the integer registers | 1,379; 24% |
| Instructions, all bodies | 9,938 | 13,318 |

The instruction count rises because a region's code is still there in the blocks that skip
it. CPU time is the measure.

Two attempts lost, and are recorded so they are not tried again:
- remaking every byte check in each pass that reads it: pass 2 went to 109 live integer
  values, 2,484 spills;
- every `prev` chain and `within` on the vectors, gated or not: 436 ns a block against 317,
  without regions in both.

## 6. Only the hot loop is inlined

Code that runs rarely still costs where it sits. Inline, it lies between the hot loop's
instructions and crowds the instruction cache. So each pass is now several functions, all
made from the same program:

- **The pass:** the loops over files and blocks, and the full blocks' body, inlined.
- **The general body:** a file's first block and its last ones, where `within` is not all
  ones. The pass calls it with the loop's parameters in a stack slot, and it writes the next
  block's parameters back.
- **One function per gated region:** its branch calls it when the gate has a bit. It takes the
  carried state, its carried words so far, and the values it reads from outside, through a
  stack slot. It gives back the values read after it and its carried words. It makes its
  vector values again from the block's bytes.

```text
# .rr's pass 2, its hot loop: region 1, behind its gate v242 (Cranelift IR).
block13(v241: i64, v242: i64, v243: i64, v244: i64, v245: i64):
    brif v242, block14, block15        ; the gate: a bit, or not
block14:                               ; taken: the words in, the call, the word out
    stack_store.i64 v11, ss1           ; the state carried in, four words
    stack_store.i64 v12, ss1+8
    stack_store.i64 v13, ss1+16
    stack_store.i64 v14, ss1+24
    stack_store.i64 v242, ss1+32       ; a value the region reads, the gate
    v255 = stack_addr.i64 ss1
    call fn1(v255, v9, v15, v8, v50)   ; words, bytes, row of slots, block, length
    v256 = stack_load.i64 ss1+40       ; the one value read after it
    jump block16(v256)
block15:                               ; skipped: the value is zero
    v257 = iconst.i64 0
    jump block16(v257)
```

The shape check allows a call only to one of the lowering's own functions. Those functions
call nothing and have no loops, so the calls make no cycle. Host Rust and runtime helpers are
still refused. The tests are in `outline_tests`.

Measured on `.rr`, runs alternated at a load of 8 to 23:

| | Inline | Called |
|---|---|---|
| Pass 2's function | 7,951 instructions | 1,614 |
| Pass 2's hot loop | 2,710 points | 1,374 |
| Pass 0's function | 2,840 instructions | 1,397 |
| CPU a 64-byte block | 316, 310, 292 ns | 275, 310, 299 ns |

The CPU time does not change beyond the noise: the general body runs a block or two per
file, and the regions are rarely taken. The gain is in the size of the hot loop.

A region's call is not always free. A call clobbers the caller-saved registers, and aarch64
keeps no whole vector register across one. So a vector value live across a region's call is
stored before the gate's branch and reloaded after it, in every block, taken or not. JSON's
loop did 21 stack loads and stores that way, against 4 inline. So the compiler decides per
region (`worth_calling`). It builds the pass, counts what is live across each region's call,
and inlines a region where a vector, or more integers than the 10 callee-saved registers,
would cross. Then it builds the pass again, until nothing changes. On `.rr`, pass 2's
regions all cross vectors and stay inline.

## 7. Transformation 3: delayed outputs, not held

An output at delay `d` decides bit `p` at bit `p + d`. Its stream word `i - 1` takes bits from
two of the program's words:

```text
stream[i-1] = (w[i-1] >> d) | (w[i] << (64 - d))
```

The loop used to hold `w[i-1] >> d` in a register from one block to the next: one value live
through the whole loop per delayed output, 13 in `.rr`'s last pass. Now each block stores its
`w[i]` as it is, in its own place. The next block loads it back from there (stored one
iteration before, so the load forwards from the store), stores the joined word over it, and
stores its own. The row's end shifts the last one. The readers see the same words as before.

```text
# Block i, output k at delay d (Cranelift IR, `delayed_store`):
v1 = load.i64 v_dst-8*outputs+8k   ; w[i-1], as block i-1 stored it
v2 = ushr_imm v1, d
v3 = ishl_imm v_w, 64-d             ; w[i]
v4 = bor v2, v3
store v4, v_dst-8*outputs+8k        ; the stream's word i-1
store v_w, v_dst+8k                 ; w[i], for block i+1
```

Measured on `.rr`'s last pass (pass 8), statically:

| | Held | Stored |
|---|---|---|
| Instructions | 649 | 482 |
| Stack loads and stores | 249 | 50 |
| Peak live integers in the hot loop | 52 | 33 |

The tests are in `delay_tests`: no word is held across the loop, and outputs at delays 1, 3 and
7 equal the interpreter at every block's edge and in a batch.

The loop-size tests now measure the whole body. They used to find the largest machine block
that branched to itself, which was only the tail after the ASCII branch's merge. Measured
over the whole body, JSON's loop was 564 instructions with 4 stack loads and stores before
these changes (the old budget said 182 and 0), and stage 1's 660 with 11. Their budgets now
start from the true numbers.
