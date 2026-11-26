# 0591: Tier 1's UTF-8 check in simdjson's form

Status: done by the implementer. The item is the user's (2026-11-21, goal: "Fix 7: the UTF-8 check in simdjson's
form"). This is fix 7 of page 40's step 59. How it is done is the implementer's choice. It follows 0586.

## What was there

JSON's grammar states UTF-8 as simdjson's rules (0404): `invalid UTF-8` and `UTF-8 cut short at the end`. Tier 1's Rust
(`bitlex_lex_rust`) wrote those statements as written:

- 18 UTF-8 classes as bitmasks, among 26 classes in all (four pairs of nibble tables), each kept in a ring;
- the error as an OR of `prev` shifts of them;
- the end's error in a third stage, two blocks behind, for its `next`s.

Stage 1 also wrote every mask the grammar names, read or not.

simdjson's `utf8_checker`, in the lean parser's `stage1.rs`, works per block:

- three nibble lookups: the byte before's high and low nibbles, and this byte's high nibble (`check_special_cases`);
- two compares for lags 2 and 3 (`must_be_2_3_continuation`), XORed with the two-continuations bit;
- a block of ASCII skipped, an error there only where the block before left a sequence open (`prev_incomplete`).

## The decision

1. **The byte-pair errors take simdjson's form where it is proved theirs** (`bitlex_pair_lookup`). In simdjson's
   stage 1 (no `skip`), the generator reads the error statements over classes:
   - those at lags 1 to 3;
   - those that hold only within three bytes of the file's end (a class and `last_byte`, `next(last_byte, k)`).

   Each term must be a pair rule (`prev(L, 1) & C`, or `C & ~prev(L, 1)`), or the XOR rule
   `(prev(A, 1) & B) ^ (prev(X, 2) | prev(Y, 3))`. The pair rules' boxes are merged as the JIT's are
   (`native_cpu_pairs::merged`, now public; 0404), each a bit of the three tables, and the XOR rule takes bit 0x80. The
   thresholds of a block's last three bytes are derived from the tables.
2. **The tables are proved equal to the statements, by enumeration.** The enumeration runs over each byte class the
   statements tell apart, and over the bytes the check reads that no mask reads: 0x00 before the file, the padding,
   and the `prefix`. It checks:
   - every window of four bytes, at the file's start too, with and without the byte order mark;
   - no error inside the `prefix`'s own bytes;
   - every end of four bytes, with and without the byte order mark, padded with 0x00 and with spaces. Where the check's
     end and the statements' differ there, every fifth byte before them must leave an error at the last two bytes'
     windows. In a longer file those windows are its own, so the two refuse the same files;
   - a block of ASCII: no error past its first three bytes, and an error there exactly where the block before's test
     says so.

   Where any of these fails, tier 1 keeps the grammar's masks, and `lookup` says why.
3. **Tier 1 writes the check in the class pass, at each block, as `check_next_input`**: the ASCII test, `prev` (new
   in `rerail_simd8`, simdjson's `prev<N>`), the three `lookup_16`s, `ge` for lags 2 and 3, `xor` (new), and
   `prev_incomplete` from the block's bytes 61 to 63. The block past the file's end is zeros, so the same test is the
   end's check. The errors are tested once, after the pass (simdjson's `errors()`).
4. **Stage 1 writes only the masks something reads**: an escape, the records, or a later stage, and their operands.
   The class tables are made again, of the classes written. In the pass form, no mask is kept for after the pass: the
   `m_q` vector 0586 left is gone.

Implementer's choices:
- the derivation and the proof are generic over a grammar's pair rules, rather than written for UTF-8;
- the test of a block's last three bytes is three byte compares, where simdjson uses a vector compare against
  `max_value`.

## What changed in the generated Rust

- JSON's tier 1:
  - the 18 UTF-8 classes' masks are gone, and its tables are two pairs, not four;
  - the third stage is gone;
  - the unread `esc` mask is gone;
  - stage 1 holds `BYTE_1_HIGH`, `BYTE_1_LOW`, `BYTE_2_HIGH` and the check.
- Kotlin's and `.rerail`'s tier 1s keep their masks' form, without the masks nothing reads:
  - Kotlin's drops `sq_bt` and `goes_on`, and a pair of tables;
  - `.rerail`'s drops about thirty one-letter classes.

## Checks

- JSON's tests (`parser_tests`, `json`, 11) pass, including:
  - `json_rusts_tapes_are_the_jits`, which has the UTF-8 files and the refusals;
  - `json_rust_is_current`;
  - the new `json_utf8_is_simdjsons_lookup`. It asserts that the lookup is proved for JSON's two statements, that the
    thresholds are simdjson's (0xe0 and 0xf0; 0xc0, 0xe0 and 0xf0), and that tier 1 has the check's parts and none
    of the UTF-8 classes' masks.
- `rerail_simd8`'s backend test now covers `xor` and `prev` 1 to 3. On an x86_64 Linux host, Haswell, Westmere and the
  fallback ran and agree, and JSON's 11 tests pass there on Haswell. That host also ran 0586's `marks` on x86 for the
  first time.
- On an aarch64 Linux host, 119 of 122 pass: `rerail-bitlex`, `rerail-compiler` (`native_cpu_pairs` among them),
  `rr_parser`, Kotlin's Rust tests, and JSON's. The 3 that fail are `rr_parser`'s Metal tests (`BackendCapacity` on
  that host). They pass on a farm Mac.
- Kotlin's and `.rerail`'s `rust_is_current` were blessed. I read the diffs: each drops masks nothing read, and renumbers
  the class tables.
- The proof takes about 90 ms a generation in a debug build.
- I read the new `tier1.rs`.

## Not checked, not done

- Throughput was not measured.
- A grammar whose pair rules do not fit eight bits, or whose byte of a rule is not a set of high nibbles, keeps the
  masks. Only JSON's grammar was tried.
- `\u`'s digits and surrogates are still checked in tier 1, over its masks. simdjson checks them in stage 2.
