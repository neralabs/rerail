# Parser generators: simdjson's techniques, generalized for the compiler

Status: 2026-11-09, plan. The catalog and its measurements; transformations in progress (§4).

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
- **Status: partial.** The compiler has gated regions, but they found none in `.rr`:
  - a byte-local value on the vectors weighs 0 in the planner's cost unless it is made a
    mask, so the `\u` check, all vector work, weighs nearly nothing;
  - a gate must be `and(mask, byte class)`;
  - at most 4 regions, each at least 24 units.

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
- **Status: missing** across passes.

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
| 1 | Gated regions: an accurate cost (vector work counted), any sparse gate, more regions | §1.1 |
| 2 | Values remade in each pass, never stored: constants, `within`, positions | §1.10 |
| 3 | Delayed masks read at an offset, not shifted and held | §1.4 |
| 4 | The fallback's totals folded into one word | §1.5 |
| 5 | Sparse outputs as positions | §1.6, §1.7 |
| 6 | Local values minimized from their truth tables | §1.9 |
| 7 | Cuts where few values cross | §1.11 |
| 8 | Token-level checks to tier 2, by the cost model | §1.15 |
