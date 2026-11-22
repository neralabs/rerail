# 0585: Tier 1 in one pass, its index padded as simdjson's

Status: done by the implementer. The item is the user's (2026-11-21, "do number 1"; then the goal "complete steps
1-5"). These are fix 5 of page 40's step 59 and the reads half of fix 4. How they are done is the implementer's
choice. This follows 0584.

## What was there

JSON's tier 1 (`bitlex_lex_rust`, simdjson's form: no `skip`) ran its stage 1 pass over the blocks and then did more
work after it:

- **A region walk.** A scalar walk of the strings went from each opener's `first_bit` to its closer. It wrote `o_str`,
  `f_str` and `hidden` as the file's words.
- **Further passes.** Each mask made of the strings was one more pass over the file's words: `body`, `esc`, `code`,
  `word` and the escapes' tests.
- **A record loop.** A loop over the starts' words pushed each bit and a code. The code was `KINDS[src[s]]` in all
  three branches, and nothing reads it.
- **A copied block.** Each block was copied into a `[u8; 64]`.

Tier 2 bounds-tested each read of a record (`starts.get(i)`, 256 past the end) and of its byte.

The lean parser (`rerail-json-lean`) does all of this in one pass:

- `prefix_xor` of the unescaped quotes, carried to the next block;
- `bit_indexer::write` at each block;
- an index ending in `len`, `len` and 0;
- a padded source, each block loaded where it is;
- stage 2 reads `*next` and its byte unchecked.

## The decision

1. **A region whose closer is its opener, one byte, with nothing else in its group is a stage-1 mask.**
   - `f = toggle(opener)` (simdjson's `prefix_xor`);
   - `o = opener & f`;
   - `hidden = f ^ opener`.

   Every mask made of these is in the pass too, because the generator already puts a mask there when it reads no
   region walked after the pass.
2. **Where every mask and escape is in the pass, so are the records (`Gen::index`).**
   - The starts' word becomes indexes at its block. As `bit_indexer::write`, it writes four, then four more where the
     block has more, then one at a time. The writes past the count are overwritten.
   - The index ends in `n`, `n` and 0.
   - `Lexed` is `starts`, `count`, `len`, `masks` and `refused`. No codes are written, and no `KINDS` table.
3. **The source is padded, as simdjson's** (`PADDING` = 64 spaces after it).
   - `lex(buf, n)` loads each block of the file where it is, the one past the end excepted.
   - `json_aot::parse(buf, n, w)` takes the padded file. `padded(src)` makes such a buffer for a caller that has none;
     the tests use it.
4. **Tier 2 reads the index unchecked.**
   - A record past the last is the first of the index's last entries. Its byte is a space, and no arm takes a space,
     so nothing reads past it.
   - Tier 2 ends at `count` and closes the root at `len`.
   - A file with no token is refused before the walk, as simdjson's `EMPTY`.
5. **In this form tier 1 writes only the helpers it calls** (`used_helpers`).

Kotlin's and `.rerail`'s tier 1s have more than one region and a `skip`, so they keep the walk and the loop. Their
files are unchanged: their `rust_is_current` tests pass without a bless.

## Checks

- JSON's tests (`parser_tests`, `json`) pass:
  - `json_rusts_tapes_are_the_jits` (16 and 16z, the refusals among them);
  - `simdjson_forms_starts_are_simdjsons_indexes`;
  - `json_rust_is_current`. Its shape test now reads the root's closer as `src[starts[count - 1]]`.
- On a Linux host, 73 tests pass: those of `rerail-bitlex` and `rerail-compiler`, JSON's, `rr_parser`'s `rust_tests`,
  and the `.rerail` and Kotlin `rust_is_current`.
- I read `tier1.rs` after the change. It holds the nibble tables, `stage1` and `lex`, and nothing else.

## Not checked, not done

- Only the arm64 backends ran: on this Mac and on an aarch64 Linux host. The x86 backends did not.
- Fix 6's other half: the masks kept for tier 2 (`q`, `esc`, `rd0_open`, `run_number`), which the lean parser reads
  from the bytes instead. Also fix 7, the UTF-8 check.
- `padded` makes a padded buffer for a caller. An engine caller that reads a file should make its buffer padded when
  it loads it. No engine caller exists yet.
