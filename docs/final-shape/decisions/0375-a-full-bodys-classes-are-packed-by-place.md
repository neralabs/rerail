# 0375: A full body's classes are packed by place

Status: built (audited 2026-11-23). Decided (the user, 2026-11-17: "go ahead", on making tier 1's block work
simdjson's rather than an imitation of it). Follows 0366.

## Intro

**The problem.** simdjson classifies a block's bytes with one lookup: two 16-byte tables, one by
each byte's low nibble and one by its high nibble. Their `and` gives eight class bits at once
(whitespace, the structural characters, and so on). A 64-byte block is four vectors, so that is 8
table lookups (`vqtbl1q_u8`) a block. The UTF-8 check adds 12, but only on a block that is not
all ASCII.

JSON's tier 1 full body made 80 lookups. It took each class from the program's own tables, as
the program's compiler had laid them out. A table's eight bits held classes that are made in
different places, so a place that needed one class looked up the whole table, the others' bits
included:

- the escape region (`\u`'s hex digits) looked up the UTF-8 table for one bit, `CDEFcdef`;
- the number classes' table was looked up in two regions;
- a region tested the structural characters again, from tables of its own.

## The decision

The full body's classes are packed again, by the place where each is made
(`native_cpu_block::full_pack`, `native_cpu_lanes::placed_pack`):

- **The places.**
  - the body's own part (place 0);
  - each gated region (`Region`: its members and the vectors only they read);
  - the inside of the ASCII split (a value known on an all-ASCII block).
- **One pack a place.** Each place's classes go into the fewest nibble tables that can test them
  (`class_pack::pack`), as simdjson's one lookup answers all of its classes. A region's lookups
  then serve its own classes only.
- **Nothing tested twice.** The own part's lookups come before any place inside it, so a class
  there is not packed again in a region. A region's class that the own part's products make up
  is tested from the own part's bits, with no lookup: `by_products`, set or complement. Two
  classes of one byte set, from two of the program's tables, are one test.
- **Not worse than before.** A place whose packing would take more tables than its classes' own
  tables keeps those. A class whose table a `pair` reads keeps that table, since its lookup is
  the pair's.

Where a class is packed changes only what a block costs, never what it computes. The emitter
saves and restores its lookups around each branch (`Emit`'s region code), so a lookup made
inside a branch is never read after it.

## What changed in JSON's tier 1, read as Rust

| The full body | Before | Now |
|---|---:|---:|
| `vqtbl1q_u8` lookups | 80 | 34 |
| the pass function's lines | 3,534 | 3,296 |

By place, one pair of tables each:

| Place | Its classes | Lookups | Runs on |
|---|---|---:|---|
| own part | whitespace, structurals, quote, control bytes | 8 | every block |
| number region | digits, `1-9`, `-`, `+`, `Ee` | 8 | a block with a number |
| escape region | hex digits, `\u` surrogates | 8 | a block with a `\` |
| ASCII split's inside | UTF-8 leads and continuations | 10 | a block not all ASCII |
| structural region | the own part's bits | 0 | — |

An ASCII block with no number makes simdjson's 8 lookups.

## What is still not simdjson's

The number region and the escape region check tokens' insides (a number's syntax, a `\u`'s
digits) with masks over the whole block. simdjson's stage 1 does not: its stage 2 checks each
number and escape where its token starts. Both regions are skipped on a block without their
bytes. The number region runs on most blocks of JSON with numbers in it.

## Tests

- `parser_tests`: `json_classes_are_one_pair_a_place` (new). For JSON and JSONC, every place
  has at most one pair of tables, the own part exactly one. It prints
  `tables by place [(0, 1), (1, 1), (2, 1), (3, 1)]`.
- `parser_tests`: the `json`, `loop_keeps_values`, `lowers_to`, `equal_the`, `gated`, `lanes`,
  `registers`, `stage1` and `vectors` guards.
- `rr_parser`: the JSON equality set, with `RERAIL_JSON_CORPUS` and `RERAIL_JSON_SUITE`.
- `compiler`: `outline`, `every_step_parses_as_the_reference`, `token_local`, `lanes`, and
  `class_pack` but for `random_needs_are_answered`.

## Found on the way

`class_pack::tests::random_needs_are_answered` takes over a minute in a debug build. It joins
the slow tier (`.config/nextest.toml`).
