# 0389: The byte order mark is stripped before the passes

Status: decided (the user, 2026-11-17: "fix this: … the BOM handled before the loop, as
simdjson does"). Follows 0335 and 0383.

## Intro

**The problem.** JSON may start with a UTF-8 byte order mark, `EF BB BF`. Decision 0335 made it
a prefix: where a file starts with it, the stream starts after it. Tier 1 checked this inside
the block body. The check was a mask `P` of the file's first three positions, and every class
that holds one of its bytes was cut by it: `class & ~P`.

In the full body (0383), `P` is zero: a full block is past the file's first bytes. In block 0 it
is not. So everything that reads `P` differs on block 0, through the strings' `fill`s and on.
The edge part had to make 90 of JSON's 186 values again.

simdjson does not check the mark in its loop. Before stage 1, it compares the first three bytes
once and moves the buffer past them (`buf += 3`). Its stage 1 never sees the mark.

## The decision

The host strips the prefix before the passes, as simdjson does (`CompiledBlock::prefix`,
`native_cpu_block::host_stripped`).

- **What the passes run.** The program with the prefix's value a constant zero, and the token
  loop starting at the stream's first byte (`without_prefix`). The CPU build compiles that
  program.
- **The rows.** For a file that starts with the prefix, `Batch::strip` moves the row past it:
  - the row's bytes start 3 later, and it is 3 shorter;
  - the prefix's bytes are zeroed, so the 16 bytes before the stream are zeros, as before any
    file.
- **The results, back in the file's positions** (`Batch::unstrip`). Afterwards the prefix's bytes
  are put back (tier 2 reads the file), and:
  - each output mask's bits move up by 3, across the row's words;
  - each extracted position, and each record's start and end, gains 3;
  - counts and codes stay as they are.
- **When.** Only where the program's values do not depend on where the stream lies in the file.
  A `phase`, a `position`, or a constant other than all zeros or all ones would differ once the
  stream moved. Such a program keeps the prefix in its body (`strip_prefix`).

**The `prefix` op.** The ladder's JSON files now spell the mark `prefix 0xef 0xbb 0xbf`. Bitlex
compiles that to one leaf, `BlockOp::Prefix(n, word)`, in place of a mask per byte. The
interpreter keeps the mark's meaning from it, and a program the host cannot strip computes it
once a file from the first word (`prefix_start`). Metal computes it in the body the
same way (0335, since 2026-11-18); the host's strip is the CPU's.

## What changed

| | Before | Now |
|---|---:|---:|
| JSON's values the edge part makes again | 90 of 186 | 47 of 187 |
| JSONC's | 63 of 248 | 64 of 248 |
| JSON's tier 1 as Rust, lines | 6,341 | 5,381 |
| its pass function (`f0`) | 5,787 lines | 4,827 lines |
| its CLIF (`f0`), instructions not cold / cold | 1,792 / 1,548 | 1,751 / 1,263 |
| the hot loop (`json_loop_keeps_values_in_registers`) | 673 instructions, 7 stack accesses | the same |
| `vqtbl1q_u8` lookups in the file | 86 | 94 |

- **JSONC.** Its count did not fall. Its comments' checks at the stream's ends are most of what
  differs, and the mark was a small part.
- **The lookups.** They are in the cold edge part. Without `~P`, more of the classes are on the
  vectors, and the edge part makes those from the bytes again.

**Rejected: the `prefix` op alone, the mark checked in the body.** The check was then one scalar
compare, as simdjson's, but the stream still started at 3 in block 0 and at 0 in the full body.
98 of 187 values differed, more than the masks' 90. Only moving the stream itself removes the
difference.

## Found on the way: a reader made from words read an unmade operand

The CPU build first got the stripped program wrong on files with no mark at all. JSON's
`within 1 & ~prev(0, 1)` came out zero on every full block.

- **The cause.** The value is on the vectors, and the ASCII split reads it, so it is made from
  words before the split's branch (`Emit::as_words`). The emission order (`demand_order`)
  emitted an operand first only when it was not on the vectors. Its operand `within 1` is on the
  vectors only, so it was never made as a word. The value read its placeholder, zero.
- **The fix.** A reader made from words needs its operands made first, as any word reader does
  (`needs`).
- **Why it was not seen before.** Bitlex folds zeros as it builds, so its programs never held this
  shape.
- **How it was found.** Each value made an output, then the values outside the cone cut down by
  bisection, to 29 values.

## Tests

- `compiler`:
  - `remade_tests::a_word_made_value_reads_made_operands` (new): JSON's program with the mark's
    check a constant zero, compiled against the interpreter. It fails without the fix.
  - every test but `random_needs_are_answered` (102), the ladder's equality included. It now runs
    the stripped programs on files with and without the mark.
- `parser_tests`:
  - `json_edge_values_are_its_ends_checks`: the host strips the mark for both programs, tier 1
    strips it, and a third of the values or fewer differ.
  - the `json`, `loop_keeps_values`, `lowers_to`, `equal_the`, `gated`, `lanes`, `registers`,
    `stage1`, `vectors` and `on_every_executor` guards: 54 passed. Among them,
    `json_tier1_vectors_equal_the_interpreter` covers files that start with the mark (0383).
- `rr_parser`: the JSON equality set, with `RERAIL_JSON_CORPUS` and `RERAIL_JSON_SUITE`. The
  suite includes its byte order mark files.
