# 0322: The JIT lowers a `match` as rustc does

Status: decided (2026-11-16); landed for tier 1 and tier 2 on the CPU.
Part of 0177's token loop.

## Intro

**The problem.** Tier 1 is compiled two ways from one program: Cranelift
compiles it at run time (the JIT), and rustc compiles the same program
written out as Rust ahead of time (the AOT). Over the 1,453 `.rr` files
the grammar parses, the JIT's tier 1 took 67.7M cycles and the AOT's
51.0M. By kind of token, most of that gap was three kinds: a number took
the JIT 75.5 cycles against the AOT's 17.1, a string 107 against 61, a
comment 106 against 63.

**The cause.** Numbers, strings and comments are tier 1's rare tokens.
The loop picks a rare token's rule by its first byte, as the lean lexer's
`match` does. The IR says it as a jump table (`br_table`): 256 entries,
but only 6 different targets. rustc's LLVM turns that `match` into a few
compares on the byte. Cranelift on aarch64 always lowers a `br_table` as:

```text
cmp   w3, #0x100          // the bound
b.hs  default
csel  x5, xzr, x3, hs     // a Spectre guard: the index 0 when out of bounds
csdb                      // a barrier: no value speculated past here
adr   x4, table
ldrsw x5, [x4, w5, uxtw #2]
add   x4, x4, x5
br    x4
```

The `csdb` stops the core from running ahead into the arm. So each rare
token waits until its entry's load has resolved. Sampling the JIT on a
file of numbers put 80% of the samples on the instruction after the
`csdb`. No Cranelift flag removes it.

**The decision.** The IR keeps its tables. The generated Rust writes a
table as a `match`, which is lean's shape, and rustc lowers it well. Only
the JIT's copy is rewritten: `native_cpu_codegen::switch_trees` runs on
each function just before Cranelift compiles it. It turns every
`br_table` of at most 32 runs of one target into a compare tree. The tree
comes from `native_cpu_stream::dispatch`, tier 2's existing lowering,
which is LLVM's switch lowering: a default target, a balanced tree of
compares over the other cases, and bit tests for close cases.

A table whose targets take arguments is left alone. So is a table of more
than 32 runs, which would need a tree more than five compares deep.

## Measured

As information. Cycles a token, `tier1_by_kind`, which now has a JIT
column:

| Kind | JIT before | JIT after | AOT | lean |
|---|---|---|---|---|
| numbers | 75.5 | 22.8 | 17.2 | 22.7 |
| strings | 107.2 | 56.2 | 64.6 | 84.1 |
| comments | 106.3 | 49.5 | 62.6 | 69.7 |
| words | 18.1 | 18.6 | 16.1 | 15.7 |
| punctuation | 10.5 | 10.5 | 8.7 | 8.9 |

Over the corpus (`jit_against_aot`, least of 15 rounds), tier 1 by the
JIT took 55.5M cycles, down from 67.7M. The AOT took 51.0M and the lean
lexer 47.4M. The AOT's code did not change.

## What is left between the JIT and the AOT

Instructions a token on punctuation, the most common kind, counted by
stepping one block of each in lldb. The JIT runs 72.2, the AOT 45.2:

| Extra in the JIT, a token | Instructions | What it is |
|---|---|---|
| Stack loads and stores | 10.4 | Values carried around the loop are spilled and reloaded each token: the block's masks, the entry table's base, the block's index. Some are stored back unchanged. The AOT has none. |
| Other ALU | 8.4 | Mostly `and` (9.6 against 4.1) and `orr`: masking to a byte and joining flags, which LLVM folds into the instruction that uses them. |
| Booleans made as values | about 4 | A two-condition branch is `cset; cmp; cset; and; uxtb; cbnz`. LLVM makes it `ccmp` and one branch. |
| Jumps | 2.0 | `tbnz …; b`: the arm Cranelift laid out does not fall through. |
| `mov` | 1.3 | Moves between registers around the spills. |

The spills are the largest share. Cranelift's register allocator does not
rematerialize a value or keep a loop-carried one in a register across a
long loop the way LLVM does. A fix would cut how many values the loop
keeps live, which is an IR change, or fold the booleans into compares
before Cranelift sees them, as `switch_trees` does for tables.

## Tests

- `rr_parser`'s `the_jits_dispatches_are_trees`: the IR keeps tier 1's
  table; no `csdb` in tier 1's or tier 2's machine code.
- All tapes as before: `aot_tapes_are_the_jits`,
  `corpus_tapes_are_the_engines`, `the_cpus_records_are_the_interpreters`.
