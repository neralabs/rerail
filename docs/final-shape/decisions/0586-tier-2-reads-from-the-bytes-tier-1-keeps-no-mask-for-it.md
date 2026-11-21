# 0586: Tier 2 reads from the bytes; tier 1 keeps no mask for it

Status: done by the implementer. The item is the user's (2026-11-21, goal: "Fix 6's second half: tier 1 still hands
tier 2 four masks that the lean parser reads from the bytes"). How it is done is the implementer's choice. This follows
0585, which did fix 6's first half (no codes).

## What was there

JSON's tier 1 kept four masks of the file for tier 2: `q`, `esc`, `rd0_open` and `run_number`. Tier 2 read three of
them:

- a string's closer: `first_bit(&m.q)`;
- each escape in a string: `first_bit(&m.esc)`;
- the end of a number or an atom: `run_last(&m.run_number)`. After it, tier 2 matched the word on the slice, and then
  ran `check_number` over the number's bytes again.

`rd0_open` was not read.

The lean parser (`rerail-json-lean`'s `walk`) reads all of these from the bytes:

- `string` loads 32 bytes at a time from the quote and takes the backslashes and quotes (`backslash_quote`). A quote
  before any backslash is the end.
- `primitive` switches on the first byte. An atom is `str4ncmp`, plus a test that the byte after it is not structural
  or whitespace.
- `number_end` reads the number once.

## The decision

1. **A region read in parts reads the bytes a block at a time** (`marks`, on `rerail_simd8`'s backends: the bits of the
   pieces' first byte and of the closer's byte in the 64 bytes from `x`).
   - A closer before every piece (`(lead - 1) & quote`) is the end.
   - A piece's byte first: its piece, then go on from after it.
   - Neither: the next block.
   - At the file's end, the string is refused.

   The closer's byte is the opener's. The pieces' first byte is their words' (JSON's `\`). A region whose closer or
   pieces are not one byte is not written yet.
2. **A run's end comes from the bytes.**
   - A kind with a check (`number`): the check reads the token's bytes once from its start and returns its match's
     end. The byte at that end must not be one of the row's (`in_row`, a table of each byte's row, as simdjson's
     `is_not_structural_or_whitespace`). That is `number_end`.
   - A kind without one: the row's bytes from the start.
3. **A split kind's words are decided by their first byte.** The test is `src[s..s + L] == *b"word"`, and the byte after
   must not be the row's. Its own arm (the number) is the check's end. These are simdjson's atoms.
4. **Tier 1 keeps no mask for tier 2.** `Lexed` is the records, their count, the file's length and the verdict. `Masks`
   is not written. Tier 1 still makes `q`, `esc` and the rest in its pass, for its own escape tests.
5. **Tier 2 writes only the helpers its code calls** (`drop_uncalled`). It has no `first_bit`, `run_last` or `word_*`.

`End`'s `Pair` and `Last` (and a region's token taken outside its read) have no byte form yet. The generator refuses
them by name. JSON has none.

## Checks

- JSON's tests (`parser_tests`, `json`) pass:
  - `json_rusts_tapes_are_the_jits`: 16 and 16z, with the refusals, the escapes, the UTF-8 files and the deep nests;
  - `json_rust_is_current`.
- On an aarch64 Linux host, 73 tests pass:
  - those of `rerail-bitlex` and `rerail-compiler`;
  - JSON's;
  - `rr_parser`'s `rust_tests`;
  - the `.rerail` and Kotlin `rust_is_current`.
- I read `tier2.rs` after the change. It has no mask, and `string` and `value` read the bytes.

## Not checked, not done

- `marks` on x86 (Haswell, Westmere) was not compiled or run. Only arm64 ran.
- The lean parser writes a string with no escape as two nodes at once. Here each piece is still its own leaf.
- Fix 7, the UTF-8 check.
