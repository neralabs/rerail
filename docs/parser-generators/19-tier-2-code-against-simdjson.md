# Parser generators: tier 2's machine code against simdjson's

## Intro

Tier 2 is meant to be simdjson's stage 2. Both are a loop over a token stream:
take the next token, switch on its first byte, do a little work, and write the
tape. This page sets the two side by side as machine code (aarch64, an M-series
Mac). It names each difference, says why it is there, and gives what it costs,
measured.

The data is the 436 tracked JSON files of a data-layer checkout: 15.7 MB.
Times are CPU time (thread user + system), best of 20. The machine had other
builds running.

## Method

- **simdjson** 5.0.5, built from its single-header release at `-O3` with line
  information. A small driver times `stage1` and `stage2` apart, through the
  parser's `implementation`, with the tape allocated first.
- **Ours.** The test `json_corpus_cost_against_simdjson` times:
  - tier 1's compiled block function;
  - tier 1 with its extract pass;
  - tier 2's compiled function, its buffers made first.

  `json_tier2_cost_per_path` times tier 2 on files of one kind of value each.
  `dump_json_tier2_code` writes the machine code.

## The numbers

| | simdjson | ours, before | ours, now |
|---|---|---|---|
| stage 1 / tier 1, the function | 1.46 ms | 3.1 ms | 3.1 ms |
| tier 1's extract pass (host) | in stage 1 | 5.5 ms | 5.5 ms |
| stage 2 / tier 2 | 1.57 ms | 19.0 ms | 5.7 ms |
| tier 2 without `CHECK` | | 13.9 ms | 2.6 ms |
| per token (structural) | 3.0 ns | 25.6 ns | 7.7 ns |

Per token, by path (arrays of 100,000 values, before and now):

| path | before | now |
|---|---|---|
| `true` | 16.6 ns | 3.3 ns |
| `"a"` | 17.2 ns | 3.6 ns |
| `[]` | 18.2 ns | 4.6 ns |
| `{"a":1}` | 24.9 ns | 8.0 ns |
| `1` | 35.8 ns | 16.9 ns |

## One path, side by side

The value `true` in an array, then the dispatch on the next token.

simdjson, about 22 instructions:

```
cmp  w8, #0x66 ; b.eq ...          // switch (*value): 'f', 'n', 't'
cmp  w8, #0x6e ; b.eq ...
cmp  w8, #0x74 ; b.ne error
ldr  w8, [x20]                     // the 4 bytes, one load
ldrb w10, [x20, #4]                // and the byte after:
ldrb w9, [table, x10]              //   structural or space?
orr  w8, w8, w9
mov  w9, #0x7274 ; movk ...        // "true"
cmp  w8, w9 ; b.ne error
mov  x8, #0x7400000000000000       // the tape word
str  x8, [x26], #8                 // one store, the cursor bumped
ldr  w8, [x25], #4                 // the next structural index
ldrb w8, [x24, x8]                 // its byte
cmp  w8, #0x5d ; b.ne ...          // ']' or ','
```

Ours, about 85 instructions for the arm, then the next state's `TOKEN` and
`SWITCH` (excerpt):

```
ldr  x6, [x28, #32]                // lasts, from the context
movz x8, #4 ; madd x8, x4, x8, xzr // lasts[taken]: a multiply
ldr  w6, [x6, x8]
subs xzr, x6, x22 ; b.lo refuse    // last byte before the token?
add  x19, x6, #1
subs xzr, x19, x1 ; b.hi refuse    // past the next token?
sub  x10, x19, x22
subs xzr, x10, #4 ; b.ne refuse    // WORD: the length
ldr  x14, [x28] ; ldr x14, [x14, x22]
and  x14, x14, #0xffffffff
movz w15, #0x7274 ; movk ...
subs xzr, x14, x15 ; b.ne refuse   // WORD: one load and compare
ldr  x2, [x28, #88]
subs xzr, x3, x2 ; b.hs full       // room for a node?
and  x6, x3, #15 ; cbnz ...        // a new block of 16: its base
...                                // off, code, size, tail: four stores,
...                                // each index times its width (madd),
...                                // the code's first-use table, three ORs
b    label299                      // and blocks that only jump
```

## The differences

1. **The dispatch's speculation barrier.** Fixed.
   - *Why it was there:* `SWITCH` was a jump table. On aarch64, Cranelift
     guards every jump table with `csel` + `csdb`, and `csdb` stops
     speculation at every token. clang lowers simdjson's small switches to
     compares, which have no barrier.
   - *Effect:* 13 ns a token on every path, the same for `true` as for `[]`.
   - *Now:* `SWITCH` is a binary search over the byte's runs of one target,
     and `RETURN` a search over the states that `PUSH` names. Tier 2 went from
     19.0 ms to 5.7 ms.
2. **The number check.** 3.1 ms of the 5.7.
   - *Why:* `CHECK` calls the check's block program, the language's own check
     from its Bitlex file. That program is built for any length, 64 bytes at
     a time. For a 1- to 20-byte number it pays for a call, the row's four
     stores, a whole 64-byte block with its edge masks, and a loop over the
     output. simdjson's `parse_number` is an inline scalar loop over the
     digits.
   - *Effect:* about 13 ns a number.
   - *To do:* inline the check's one-block body at the `CHECK` for a token
     under 64 bytes, and call only for longer ones.
3. **More tokens.** Necessary.
   - *Why:* the tape is tree-sitter's tree. A string is three nodes (`"`, its
     contents, `"`), and each is a token. simdjson's string is one
     structural.
   - *Effect:* 748,000 tokens against 524,000 structurals, 1.43 times the
     loop's work.
4. **The tape's columns.** Necessary in kind.
   - *Why:* v9np stores narrow columns (code, size, offset, tail, and a base
     per 16 nodes), and codes numbered by first use. That is four stores, a
     table probe and three ORs a node (the ORs pick the widths). simdjson
     writes one 8-byte word.
   - *Effect:* measured before the fix, at 1.5 ms of the 19.
   - *To do:* keep a cursor per column in a register, bumped as simdjson's
     tape pointer is, in place of an index times a width.
5. **Checks simdjson does not make.** Most are not necessary.
   - *Why:*
     - `TOKEN` and `SWITCH` both test for the stream's end.
     - `TOKEN` tests that a token does not start inside the one before.
     - `SPAN` tests that its last byte lies inside the token.
     - Each node and each `PUSH` tests for room.
   - simdjson needs none of these:
     - its stream is well formed by construction;
     - its tape is sized from the structural count;
     - its end is a sentinel index whose byte refuses.
   - *To do:*
     - Prove the stream's order at compile time from tier 1's extracts.
     - Size the buffers from the stream: at most two nodes a token, and the
       root.
     - Give the end a sentinel byte that no token starts with.
   - *Effect:* about ten instructions a token.
6. **Cranelift's code.** Partly a tool difference.
   - *Why:* unoptimised Cranelift keeps the blocks that only jump, scales an
     index by a constant with `madd`, and loads each column's pointer from
     the context. That load is cheap, and was kept because holding all
     nineteen pointers live cost 8,000 spills.
   - *What was tried:* `opt_level=speed`. It helps the synthetic paths by
     about 5% and makes the corpus slower (6.6 ms). It is not used.
7. **Tier 1's extract pass runs on the host.** 5.5 ms, the largest cost left.
   - *Why:* tier 1's compiled function writes the masks, and Rust turns them
     into positions after it. simdjson writes its indexes inside stage 1,
     with the masks still in registers.
   - *To do:* generate the flatten in tier 1's loop.

## Where it stands

Both tiers now take 14.3 ms, against simdjson's 3.0 ms. In order of what is
left:

| item | cost now |
|---|---|
| tier 1's extract pass (7) | 5.5 ms |
| the number check (2) | 3.1 ms |
| tier 1's function | 1.7 ms over stage 1 |
| tier 2's checks and code quality (5, 6) | part of the 2.6 ms |

Items 3 and 4 are the format's, and stay.
