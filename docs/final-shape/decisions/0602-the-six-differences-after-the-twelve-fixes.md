# 0602: The six differences after the twelve fixes

Status: done by the implementer. The items are the user's (2026-11-22, goal: "address all of these 'still different'
items", page 40's "After the twelve fixes"). How each is done is the implementer's choice. This follows 0599.

## What was there

Page 40's reading after 0599 found these differences from the lean parser (`rerail-json-lean`):

- **Tier 2 against `walk`:**
  - **Writing nodes.** `walk` stores into the tape's columns through pointers it holds. Tier 2 called the writer's ops
    (`w_open_with`, `w_leaf_with`, `w_leaf_in_with`, `w_close_with`) on its state.
  - **`\u` escapes.** `escaped_string` reads the four digits as a number and tests the surrogates on that number. Tier 2
    tested each byte's class: four hex digits, a `D`, then the third byte's range. A low surrogate looked for its high
    one in a 64-bit word of the string's earlier escapes.
  - **String scanning.** `walk` reads 32 bytes at a time, tier 2 read 64.
  - **Repeated code.** With each alternative an arm of the place's match (0599), the arms for true, false, null and the
    number were written out at each of the three value places. `walk` calls `primitive` from each.
- **Tier 1 against `index`:**
  - **String masks.** Tier 1 kept the grammar's names (`o_str`, `hidden`, `body`, `code`, `wordbyte`, `word`), where
    `index` has `scalar`, `follows` and `in_string`.
  - **End of a UTF-8 block.** Tier 1 tested a block's last three bytes with three byte compares, where `index` uses one
    vector compare.

## The decision

1. **The UTF-8 end is one vector compare** (`is_incomplete`).
   - `rerail_simd8` gains `gt_bits`, simdjson's saturating subtract, on arm64 (`vqsubq_u8`), Haswell
     (`_mm256_subs_epu8`), Westmere (`_mm_subs_epu8`) and the fallback.
   - Tier 1 loads a 64-byte `MAX_VALUE` once: 255, then `0xf0 - 1`, `0xe0 - 1` and `0xc0 - 1` at the last three bytes.
     The thresholds are the ones 0591 derives from the grammar.
   - `prev_incomplete` is a vector, `gt_bits(v, max_value)`. It is ORed into the error vector at an ASCII block, and
     after the pass (`check_eof`).
2. **A string is read 32 bytes a step** (`backslash_and_quote`).
   - `rerail_simd8` gains `marks32`, the two compares over 32 bytes, on each backend: arm64 as simdjson's (two loads,
     pairwise adds), Haswell with one load and two `movemask`s, Westmere with two loads, and the fallback.
   - Tier 2's walk and its methods are generic over the backend, chosen once (`run_best`). `marks` is a direct call
     of `S::marks32`, and the string loop steps `x += 32`.
3. **A value's primitives are one method, `value_primitive`, called from each place**, as `walk` calls `primitive`.
   - Each value place's `match` on the byte has the containers' arms (`{`, `[`), and its `_` is the call. The
     method's one `match` has the string's, the atoms' and the number's arms, each written once.
   - In it, a token's leaf is `token(d, &mut i, number, s, end)`, and a checked run's end is `number_end(src, s)`
     (the lean parser's `number_end`: the check's match, with the byte after it outside the row).
   - This is 0582's method, which 0599 had removed so the byte was matched once. For a primitive the byte is now
     matched twice, at the place and in the method, as in `walk` (`match *v`, then `primitive`).
   - Later, the same day: a first version kept every arm at each place and made only their bodies calls. The stop
     hook found the arms still written three times, and this replaced it.
4. **`\u`'s digits are one number** (`bitlex_piece_checks`), as `escaped_string` reads them.
   - The digits the first anchored error requires (one to four hex digits at consecutive bytes) are read as `cp`,
     through `HEX_VALUE` (simdjson's `digit_to_val32`). A byte that is not hex puts bits past the digits, so the error
     is `cp > 0xffff`.
   - Each run of hex-digit classes at consecutive bytes is a range: of `cp` where the run is within its digits (`high`
     is `0xd800..=0xdbff`, `low` is `0xdc00..=0xdfff`), else of a number of its own (the pair's next escape:
     `0xdc..=0xdf` of its first two digits). A run becomes a range only where its classes spell one interval, and
     each class is a set of hex digits that holds both cases of each.
   - A piece before read at one distance `k` alone becomes one position, `after2 = y + 6` (`low_checked`), where no
     piece can start in the `k - 1` bytes from it. The proof uses the classes those bytes hold (the mask's own and
     those the errors before require), none of which is the pieces' first byte. Otherwise the word of 0599 stays.
5. **JSON's string and token masks are `index`'s** (the grammar, `json-simdjson.bitlex`).
   - The masks are `scalar = ~(op | ws)`, `nonquote_scalar = scalar & ~quote`, `follows = prev(nonquote_scalar)`,
     `in_string = f_str` and `string_tail = in_string ^ q`.
   - The control error is `in_string & control` (`non_quote_inside_string`).
   - Each token's start is `… & ~string_tail`.
   - `bitlex_lex_rust` takes a conjunct every start has out of their union once (`factored`). Stage 1's starts are
     then `((q | op) | nonquote_scalar & ~follows) & ~string_tail`, the form of `structural_start`.
   - `code`, `word` and `wordbyte` are gone. `body` keeps its form `f_str & ~o_str`: it feeds only the escapes, and
     the JIT reads that form as the region's inside (with `in_string & ~q` its string loops lose their part scans
     and its direct run).
6. **Tier 2 stores its nodes through a cursor it holds** (`writer16::Cursor`, the lean parser's `Walk`).
   - The cursor is the walk's local in `run`. It holds the block it writes (its room and side area), the next node,
     the start before, the source's length, and the open nodes' frames.
   - An open, a leaf, a node of one leaf and a close store the node's cells through these, at the `Fixed` widths: the
     slot of the node before, the code, the gap, the checkpoint, the length and the size.
   - Four cases go to the state's own op, with the cursor's fields given to it and taken back:
     - a block's first node (`new_block`, and `settle` of the block before);
     - a close of a node in a block before;
     - a node of one leaf whose child begins a block;
     - a node whose bounds fail (the state keeps the failure, as before).
   - The cursor's `Drop` gives its fields back to the state.

Implementer's choices:
- **Exactness over likeness, three times.** In each case the lean parser's form would refuse other files than the
  grammar states, so the generated code keeps the grammar's meaning:
  - **The quote outside `~follows`.** `index`'s `structural_start` holds a quote after a scalar in the scalar
    (`1"a"`), and simdjson refuses it in `parse_number`. Our grammar refuses it in tier 2's tree. So the quote's start
    stays outside `~follows`.
  - **The pair's second escape.** It reads the two digits the grammar's lone-high error names, not all four.
  - **A lone low surrogate.** It is still tested at its own escape (`after2 != y`). The lean parser instead skips the
    checks of the escape it already read.
- **The bound checks stay.** The cursor's bound checks (start order, the source's length) are compares that send a
  node to the state's op. The lean parser has none. The writer never writes a tape that breaks its bounds.
- **Generic over the grammar.** The number reading and the factoring are generic over a grammar's statements; only
  JSON's grammar was tried.

## Checks

- **JSON's tests** (`parser_tests`, `json_tests::`, 14) pass. They include:
  - `json_rusts_tapes_are_the_jits`, with the `\u` inputs of 0599, 16 and 16z, the UTF-8 files and the deep nests,
    some of them several blocks of nodes.
  - `json_tier2_is_walks_shape`, which now also checks:
    - `value_primitive` written once, each of the three places' `_` its call, and each of its arms (the string's,
      `true`, `false`, `null`, the number's) written once in all of `tier2.rs`, with no container arm in it;
    - `number_end` written once, and no leaf written out at a place;
    - `x += 32` with `S::marks32` and `run_best`;
    - one `Cursor::<Cells>::begin(state)`, and no `w_*_with` or `d: *mut u8`.
  - `json_escape_checks_are_tier2s`, which now checks `let cp = HEX_VALUE[…] << 12 | …`, `cp > 0xffff`, the two ranges,
    `after2 = y + 6` and `m3 && after2 != y`, with no `w2` and no digit's class.
  - `json_tier1_masks_are_block_locals`, which now checks the masks `scalar`, `nonquote_scalar`, `follows` and
    `string_tail`, the factored starts, and none of `word`, `code`, `wordbyte`, `hidden` or `o_str`.
  - `json_utf8_is_simdjsons_lookup`, which now checks `gt_bits(v, max_value)` and `MAX_VALUE`'s last bytes.
  - The JIT's `simdjson_forms_starts_are_simdjsons_indexes` and `simdjson_forms_sixteen_run_is_direct`, on the new
    grammar.
- **`rerail_simd8`'s backend test** covers `gt_bits` and `marks32`:
  - On this Mac, arm64 and the fallback agree.
  - On an x86_64 Linux host, the backends the CPU runs (Haswell among them) agree. JSON's tests pass there (67 of 67,
    with `rerail-tape-format`).
- **The new `bitlex_piece_checks` unit test** checks:
  - the digit classes' values;
  - the surrogates' ranges;
  - that a class set that spells no interval is refused.
- **The writer test** (`the_ops_at_constant_widths_write_the_state_widths_bytes`) now compares a third writer, the
  cursor's, at five width sets, 16 and 16z, with nodes of one leaf.
  - Its trees were 23 ops, within one block, though its doc said many blocks.
  - They now pass 4096 ops, with more than 1000 nodes of one leaf and a length past two bytes, both asserted.
- **On an aarch64 Linux host,** after the primitive method: 78 of 78 of `rerail-bitlex`, `rerail-compiler`,
  `rr_parser`'s and Kotlin's Rust tests and JSON's.
- **On an aarch64 Linux host,** before it:
  - 131 of 132 pass: `rerail-bitlex`, `rerail-compiler`, `rerail-tape-format`, `rerail-simd8`, `rr_parser`, and
    Kotlin's and JSON's Rust tests.
  - The one failure is `block_tests::stage1_utf8_equals_std_on_every_executor`: the host's device refused to map host
    memory (`BackendCapacity`). It passed on the same host earlier today with the same device code. No device code
    changed here.
  - With all of `rerail-parser-tests`, 207 of 215 pass. The 8 that fail are device-executor `block_tests`
    (`BackendCapacity`, one `UnsupportedBackend`). They read no grammar.
- **Other checks:**
  - Kotlin's and `.rerail`'s `rust_is_current` pass unchanged: no common conjunct in their starts.
  - Clippy is clean on the changed crates, with one lint in the writer test fixed.
  - I read the new `tier1.rs` and `tier2.rs`.

## Not checked, not done

- Throughput was not measured.
- The device tests' `BackendCapacity` on the Linux hosts was not looked into.
- Westmere's `marks32` and `gt_bits` ran only where the x86 host's CPU runs them; which of the x86 backends ran there
  was not recorded.
- Only JSON's grammar was tried for the number reading, the one-position history and the factored starts.
