# 0604: The generated Rust in the lean parser's shape

Status: done by the implementer. The goal is the user's (2026-11-22): "continually compare the rust code and address
the differences until the shapes are essentially identical". Which differences there were, and how each is done, are
the implementer's reading and choices. This follows 0602.

## What was there

After 0602, a reading of `json_aot`'s `tier1.rs` and `tier2.rs` next to `rerail-json-lean`'s `index` and `walk`
found these differences:

- **Tier 2 against `walk`:**
  - **The `\u` pair.** Tier 2 read two digits of a high surrogate's next escape. It tested a lone low surrogate at its
    own escape, with `m3 && after2 != y`. `escaped_string` reads all four digits as a number and skips the checks of
    the escape it already read (`low_checked`).
  - **The cursor's bounds.** The cursor tested each node's bounds (the starts in order, the ends within the source),
    and sent a node that failed to the state's op. `Walk` has no such test.
  - **The returns.** Each closer went back through the run's `match st`. `walk`'s `ScopeEnd` is a loop of its own,
    and a run of closers stays in it.
  - **A separator's end.** At a `,`, a `:` and an opener, tier 2 wrote `let s = starts[i] as usize; i += 1; e = s + 1;`.
    `walk` writes `take!()`. No close reads that end.
- **Tier 1 against `index`:**
  - **The records.** They were `((q | op) | nonquote_scalar & ~follows) & ~string_tail`. `index`'s are
    `structural_start = (op | scalar & ~follows) & ~string_tail`.
  - **The file's bits.** Every mask was ANDed with `lv`, the block's bits inside the file, and with the byte order mark
    taken out of block 0. `index` has no such mask: it reads its last block padded with spaces, and starts after the
    mark.
  - **The toggle's name.** It was `v_m_f_str`, where `index` has `in_string`.
  - **A string open at the end.** Tier 1 read the toggle's bit at the file's last byte. `index` tests the carry
    (`finish`: `prev_in_string != 0`).
  - **The records' write.** Tier 1 wrote each block's records inline, in the block. `index` writes the block before's
    (`indexer.write(idx - 64, prev_structurals)`), and the last block's after the loop, through one `write`.

## The decision

1. **The `\u` pair as `escaped_string` reads it** (`bitlex_piece_checks`).
   - A run of digit classes right after a piece's first byte and the gate is read as the next piece's whole number,
     where the bytes before it are `cp`'s digits. So that number has `cp`'s width: `n7`, four digits.
   - Where every error at that next piece is false given what the check proved (`cp`'s range, the pair's position),
     its checks are skipped: `if after2 != y { … }`, which is `low_checked`. A three-valued evaluation (`eval3`)
     proves this, with the proved facts substituted in (`assume`).
   - Elsewhere, the lone-low error is `m3` alone.
2. **The cursor stores with no bound test** (`writer16::Cursor`), as `Walk` does.
   - `Cursor::begin`'s safety contract states the walk's order: each start at or past the one before, each end at or
     past its start, all within the source.
   - A debug build checks the order (`debug_assert!`).
   - The state's op writes only a block's first node and a close of a node in a block before.
3. **The places a level returns to are one arm, a loop** (`one_loop`), as `walk`'s `ScopeEnd`.
   - JSON's arm is `3 | 6 | 7 => 'ret: loop { match st { … } }`: an object's rest, an array's rest and the document's
     end.
   - A closer in it returns with `continue 'ret`, not through the run's `match`.
4. **A token without a node, followed in its node by a token, is taken as `i += 1` alone**, as `walk`'s `take!()`.
   - These are a list's separator, an opener and a pair's `:`. No close reads their end.
   - A closer's end is kept: it is its node's end.
5. **Tier 1's records are `structural_start`** (the grammar, `json-simdjson.bitlex`).
   - The grammar names `structural_start = (op | scalar & ~follows) & ~string_tail`.
   - Punctuation and numbers start at `structural_start & C`. The string's quote keeps the region's opener as its
     start, because the JIT's reader needs it.
   - The grammar adds the error `q & follows & ~string_tail`: a string's quote right after a scalar (`[1"a"]`).
     simdjson refuses that file in stage 2, at the number's or the atom's end. With the error, `structural_start`'s
     form refuses the same files as before.
   - `bitlex_lex_rust` writes the records as a named mask where the tokens' starts differ from it only at bytes an
     error of tier 1 refuses (`refused_equal`). The proof enumerates each byte and each value of the masks the byte
     does not decide; an escaped byte's backslash is before it.
6. **No mask is ANDed with the file's bits where nothing holds at the padding** (`bitlex_lex_rust`, `false_at`).
   - The conditions:
     - every block is the file's or its padding (no stage behind, the source padded);
     - neither the records nor an error ORed into `errors` holds at the padding's byte (a space), whatever the
       carried masks hold, which is proved by enumeration.
   - It holds because a bit past the end reaches no byte before it: shifts, toggles and carries all go up, and no
     block follows the last.
   - A `prefix` is then skipped as `index` skips a byte order mark. The blocks start after it (`off`), and each
     record's base is `off + 64 * q`.
7. **The toggle has the grammar's name.** A region's toggle takes the name of the mask that is it alone
   (`mask in_string = f_str`), unless something else has that name.
8. **A string open at the end is the toggle's carry.** It is `refused |= c_m_in_string != 0`, as `finish`. This is
   used where the masks are not the file's bits and the toggle's mask holds at no padding byte: then its carry out
   of the last block is its value at the file's last byte.
9. **The records are written through one `write_indexes`, a block behind**, as `index`.
   - At each block, the block before's bits are written (`prev_structurals`), and after the pass the last block's.
   - The function is simdjson's `bit_indexer::write`: nothing where the bits are none; then four; then two at a time,
     each test inside the one before, to 24; then one at a time.

Later, the same day, a second and a third reading found five more differences, done as follows:

10. **`peek` returns the record's byte** (`u8`), as `walk` reads `*v`. Every arm it feeds is a byte pattern.
11. **A string's escapes are read as `escaped_string` reads them.**
    - **No end bound.** Where tier 1 refuses a file whose region is open at its end (`refuses_open_at_end`: an error
      of the region's fill at the file's last byte), the read tests no bound at the file's end. Before, it tested
      `x >= len` after each 32 bytes and each escape. `parse` runs no tier 2 on a refused file.
    - **The second byte decided once.** The pieces' words are two bytes with the lead's first, and the checks' gate
      is among their second bytes. So the second byte is one `match src[y]`, as `escaped_string`'s `c`: the gate's arm
      holds the `\u` checks, the other words' arm does nothing, and any other byte is refused. Before, the two bytes
      were matched against every word, and the `u` was then tested again. `piece_checks` now returns the gate and the
      code inside it.
12. **A number's arm is its check's first bytes** (`0x2d | 0x30..=0x39`), as `walk`'s `*v - '0' < 10 || *v == '-'`.
    Any other byte is the `_`'s refusal, at the same byte where the check refused it before. The rule is: a split
    kind's own arm, where it is a checked run whose check matches no empty run.
13. **A check reads its bytes as `number_end` does.**
    - Where none of a check's bytes is the padding's (a space), its match stops at the padding at the latest. So it
      reads `b[j]` (still a slice index), not an `Option` at each byte.
    - Alternatives that are each one byte are one test: the exponent's sign, as in `parse_exponent`.
14. **The root's calls test no depth.** No level is open there (`depth` is 0), as at `walk`'s document. Each list's
    calls keep their test. JSON's `if depth == DEPTH` is then at the four calls inside the two lists' loops.

Implementer's choices:
- **The string's start.** It stays `q & ~string_tail`, not `structural_start & q`: the JIT's split of a region read
  in parts requires the opener's own mask. The records are still `structural_start`, by the proof in 5.
- **The new error.** It is the grammar's, so the JIT refuses `[1"a"]` in tier 1 too. simdjson refuses it later, in
  stage 2. The refused files are the same.
- **The write's stepping** follows the lean parser's `write` (two at a time to 24), with `trailing_zeros` in place of
  `reverse_bits` and `leading_zeros`.
- **The number's value.** `number_end` also computes the number's value, and refuses an integer past 64 bits
  (`BigintError`). The grammar's check states no value and no bound on the digits, so tier 2 computes none. It accepts
  those integers, as the JIT does.
- **Generic over the grammar.** The proofs, the returns' loop and the separator rule are generic over a grammar's
  statements. Only JSON's grammar meets their conditions here: Kotlin's and `.rerail`'s Rust is unchanged (their
  tier 1s have stages behind).

## Checks

- **JSON's tests** (`parser_tests`, `json_tests::`, 14) pass. They include:
  - `json_rusts_tapes_are_the_jits`, with new inputs, each compared in 16 and 16z:
    - `\u` pairs: a non-hex digit in the low half, a cut low half, a pair then a lone low, two pairs, a high then a
      high;
    - a string after a scalar: `[1"a"]`, `[true"a"]`, `{"a":1"b"}`, `[1 "a"]`, `["a""b"]`;
    - files after a byte order mark: the mark alone, a space, `[1]`, an unclosed string, a scalar, the mark inside a
      string and after the mark, and three forms at 17 lengths across a block's edge;
    - unclosed and closed strings at 17 lengths across a block's edge, without the mark.
  - `json_tier2_is_walks_shape`, which now also checks:
    - one returns' loop, with an object's and an array's closer staying in it;
    - each `,` and the `:` taken alone;
    - `e = s + 1` only at the four closers.
  - `json_escape_checks_are_tier2s`, which now checks `if after2 != y {`, `if m3 {`, `n7`'s four digits and its
    range.
  - `json_tier1_masks_are_block_locals`, which now checks:
    - no `lv`, and the blocks after the mark (`off`);
    - `structural_start`'s expression, as `index` writes it;
    - the new error;
    - `in_string`'s word local to its block, and the end's test `c_m_in_string != 0`;
    - one `write_indexes`, called at each block with the block before's bits and once after the pass.
  - The JIT's `simdjson_forms_starts_are_simdjsons_indexes` and `simdjson_forms_sixteen_run_is_direct`, on the new
    grammar.
- **After items 10 to 13:**
  - JSON's 14 tests pass, with escapes of a byte that is no word (`\x`, `\'`, `\U`), a backslash at the file's end,
    and every word in one string.
  - `json_escape_checks_are_tier2s` checks the one `match src[y]`, and that the string has no word match and no
    `x >= len`.
  - `json_tier2_is_walks_shape` checks the number's arm and the check's `b[j]` reads and one sign test.
  - A unit test checks that `refuses_open_at_end` holds for JSON's grammar and fails without its error.
  - 82 of 82 pass on an aarch64 Linux host.
- **New unit tests** in `bitlex_lex_rust`:
  - the records proof holds for JSON's grammar, and fails without the new error;
  - `without_lv`'s rewrites;
  - `false_at` for the records, the control error, a class holding the space, and a carried mask.
- **The writer test** (`the_ops_at_constant_widths_write_the_state_widths_bytes`) passes under the debug checks of the
  cursor's order.
- **On an aarch64 Linux host**, after each change, 79 to 81 of 79 to 81 pass: `rerail-bitlex`, `rerail-compiler`,
  JSON's tests, `rr_parser`'s Rust tests, and Kotlin's and `.rerail`'s `rust_is_current`.
  - After the structurals change, with all of `rerail-parser-tests`, 143 of 163 passed. The 20 that failed are
    device-executor tests: `block_tests`, and two `compile_tests`. Each failed with `BackendCapacity` or
    `UnsupportedBackend` from the host's device, as in 0602.
- **Other checks:**
  - Kotlin's and `.rerail`'s `rust_is_current` pass, their Rust unchanged.
  - Clippy is clean on the changed crates.
  - I read the new `tier1.rs` and `tier2.rs`.

## Not checked, not done

- Throughput was not measured.
- x86 was not run for these changes. No `rerail_simd8` code changed.
- The device tests' failures on the Linux hosts were not looked into.
- Only JSON's grammar meets the conditions of the proofs (5, 6, 8) and of the separator rule.
