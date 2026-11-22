# 0599: The walk's loops, and the checks in the lean parser's places

Status: done by the implementer. The items are the user's (2026-11-21, goal: "address all of these", page 40's
fixes 8 to 12). How each is done is the implementer's choice. This follows 0591.

## What was there

Page 40's reading after 0591 ("After the seven fixes") found these differences from the lean parser
(`rerail-json-lean`):

- **Tier 2 against `walk`:**
  - Each step of a container went through the one `match st`: an object's pair, its close and its `,` or `}`
    were three trips.
  - Taking `{` or `[` was a place of its own.
  - A pair's close was a place of its own.
  - A value's first byte was matched three times: a range, then `"` against the rest, then t/f/n/number.
  - Each string piece was its own leaf.
- **Tier 1 against `index`:**
  - `\u`'s digits and surrogates were checked in tier 1 at every block, over five classes and a second stage.
  - Every mask was written into a ring and read back.
  - `lv` and the byte order mark were worked out in both stages. A string open at the end was tested at every block.
  - It had two pairs of class tables.

## The decision

### Tier 2 (`bitlex_rust::tier2_loop`)

1. **A list is one place, a labeled loop** (fix 8):
   - The element comes first, then the decision on the separator (`continue 'lN`) and on what follows the list.
   - A level's call inside it returns to a place made from the rest of the loop's body: `to_place` turns
     `continue 'lN` into `st = N`. Places made from the same code are one place.
2. **A choice's alternatives fall out of their decision** (`K::Fall`) to one continuation, written once after it.
   - A primitive value's pair close is therefore written where the value ends.
   - Only a level's call needs a place to return to.
3. **A place that takes a token and goes on to a list's loop shares the loop's arm**: `A | B => { if st == A { … } … }`.
   That is the lean parser's `ObjectBegin | ObjectKey`.
4. **A value decided once** (fix 9):
   - `decide` writes one `match` on the token's first byte. A split kind's words are guarded arms of it, and the
     kind's own arm is its row's bytes.
   - Each alternative of a choice is its own arm of the place's decision: the containers', the string's, the atoms'
     and the number's. The byte is matched once, at each of the value's three places.
   - So the `value` method of 0582 is gone, and each primitive's arm is written at each place (`string` stays a
     method). The lean parser's `match *v` takes `{` and `[`, then `primitive` tests the byte again; here one `match`
     takes them all.
   - Later, the same day: a first version kept `value` as the place's `_` arm, which matched the byte a second time.
     It was replaced by the above.
5. **A string with no escape written in one go** (fix 12):
   - The first block's `marks` are read before the loop.
   - If a closer comes before every piece, the string and its content are written as one op, `leaf_in`: one leaf when
     it is empty.
   - `leaf_in` is new in the writer (`writer16::leaf_in_x`, `w_leaf_in_with`). It writes the same slots as `open`,
     `leaf` and `close`.

### `\u`'s checks where the escape is read (fix 10, `bitlex_piece_checks`)

6. **An error anchored at an escape is tier 2's.** The analysis works from the grammar's statements:
   - The region's piece starts at `B & R & ~escaped(B)`.
   - The mask `escaped(B) & R` is the anchor: the piece's second byte.
   - A mask, or an error, is anchored if one of its conjuncts is an anchored mask. Its other conjuncts must be read
     from the bytes: a class at an offset, a mask of classes, or an anchored mask at an earlier byte (`prev`). The
     earlier pieces are kept as a 64-bit word.
7. **Tier 2 tests these at each piece.** The test is gated on a byte that every error needs (JSON's `u`), so it runs at
   a `\u` alone, as the lean parser's `escaped_string` does.
8. **Tier 1 drops what tier 2 covers** (`piece_covers`). With them go the classes and masks that only these errors read.
9. **A class read past the file must not hold the padding's byte (a space).** Tier 2's byte tests then equal tier 1's
   masks there.

### Tier 1 (`bitlex_lex_rust`, every grammar's; fix 11)

10. **Each mask's word is read where its reader reads it, by age**, meaning the number of blocks between the write and
    the read:
    - age 0: a `let` of the block's code;
    - age 1, read in the writer's own stage: a `let` of the block before's word;
    - age 1, read in another stage: a variable of the pass;
    - older reads, and reads two stages behind the writer: a ring, as before. Past the file's last block, the writer's
      stage has stopped by then.
11. **`lv` once a block, at the loop's top**, with the stage behind taking the block before's. **The byte order mark is
    tested once, before the loop.**
12. **The end's errors are tested once, after the pass.**
    - An escape `A & M`, where `M` is the file's last byte, reads `A`'s word at that byte once.
    - `M` is proved to be the last byte by enumeration: every length from 1 to 200, with and without the `prefix`.
    - `last_byte` then has no reader, and neither does the second stage.
13. **With no stage behind, the pass is the file's blocks alone.** There is no zero block past the end. The UTF-8 check's
    end is `prev_incomplete`, tested after the loop, which is what the zero block's ASCII test did.
14. **The errors' bits are ORed into one word, tested once after the pass.** This is `index`'s
    `unescaped_chars_error`. Before, each error was a test at every block.

Implementer's choices:
- `K::Fall`, the shared arm, and the piece analysis are generic over a grammar's statements, not written for JSON.
- The earlier pieces are a 64-bit word per string, not per file. A `prev` that reaches past the string's opener reads
  nothing. For JSON's grammar (`prev(high, 6)`) that never happens: from a `u` back to a string before, there are its
  four bytes, a closer, a byte between, an opener and a backslash.
- `marks` reads 64 bytes, where the lean parser's `backslash_quote` reads 32.

## Checks

- JSON's tests (`parser_tests`, `json_tests::`, 14) pass. They include:
  - `json_rusts_tapes_are_the_jits`: 16 and 16z. Its new `\u` inputs are pairs in both cases, a lone high, a lone low,
    a low before a high, a high before another escape or another high, too few digits, a digit that is not hex, an
    escaped backslash before a `u`, a pair split over two strings, and a pair across a block's edge (at 20 offsets).
  - The new `json_tier2_is_walks_shape`:
    - two labeled loops, each separator going back inside its own;
    - two begin-and-loop arms;
    - no place made of a pair's close alone;
    - no `value` method, and at each of the three value places one `match` on the byte with the containers', the
      string's, `true`'s and the number's arms, and no second match in it;
    - `leaf_in` in `string`.
  - The new `json_escape_checks_are_tier2s`: no `\u` class or mask in tier 1, and the three errors in `string` behind
    `if matches!(src[y], 0x75)`.
  - The new `json_tier1_masks_are_block_locals`:
    - no ring, and the file's blocks alone;
    - the byte order mark tested once, and `lv_0` made once;
    - only `v_m_f_str` and `v_m_word` outside the loop;
    - the end's test after the pass, and no `last_byte`;
    - the errors ORed (`errors |= v_t2`) and tested once after the pass.
- Kotlin's and `.rerail`'s `rust_is_current` were blessed. Their stage 1s take the same local form. Their lagged
  stages remain, and `.rerail` keeps one ring, for `digit`.
- On an aarch64 Linux host, 119 of 119 pass: `rerail-bitlex`, `rerail-compiler`, `rerail-tape-format`, `rr_parser`,
  Kotlin's Rust tests and JSON's.
- Clippy is clean on the changed and generated crates. Three older lints in `native_cpu_stream.rs` are fixed too.
- I read the new `tier1.rs` and `tier2.rs`.

## Not checked, not done

- Throughput was not measured.
- x86 was not run for this change. No `rerail_simd8` code changed.
- The piece analysis takes a region of one part of a fixed length. The end's test takes `A & M`, where `M` is the last
  byte. Only JSON's grammar was tried.
- The `\u` checks test the bytes' classes. The lean parser reads the four digits as a number.
