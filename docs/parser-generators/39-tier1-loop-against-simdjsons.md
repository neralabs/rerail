# JSON's tier 1 loop against simdjson's stage 1 loop

## Intro

Tier 1 is to have simdjson's stage 1 shape: one loop over 64-byte blocks, each block classified
on the vectors, its masks combined as words, its structural positions written out. This page
sets the two loops side by side, instruction by instruction, and says where ours is longer.

Both are counted the same way: the machine code from the first block the loop's back edge
takes to that back edge, in layout order. Cold blocks are laid out after the loop and are not
counted. An inner loop (writing the positions) is counted once. This is code size, not the
path one block runs; the path is noted where it matters.

- **simdjson's loop**: `scripts/dev/simdjson-stage1-loop` builds
  `fixtures/simdjson-stage1/stage1_loop.cpp` with clang as `rerail-simdjson-native` builds
  simdjson, and counts the loop. The file is `json_structural_indexer::index<64>`'s loop (arm64's
  step size), with `step<64>` and `next()` written out as they are there, in a function of its
  own. In simdjson it is inlined into `dom_parser_implementation::stage1` with the tail and
  `finish()`, which is why it could not be read off before.
- **Ours**: `json_tests::json_shapes` prints each form's "hot loop" (`clif_text::machine_loop`);
  `RERAIL_DUMP_LOOP=DIR` writes it to `DIR/14-json.s`.

Measured 2026-11-18 on arm64, at d169f605b.

## The numbers

| Loop | Instructions | Stack loads or stores |
|---|---:|---:|
| simdjson's stage 1, by clang | **344** | 0 |
| simdjson's stage 1 as our block program (`fixtures/simdjson-stage1/stage1.rir`), by our compiler | 437 | 1 |
| JSON's tier 1 (`bitlex_ladder/14-json.bitlex`, simdjson's acceptance) | **1,078** | 31 |
| JSONC's tier 1 (`15-jsonc.bitlex`) | 1,332 | 58 |
| `lexers/json.bitlex` (`json_loop_keeps_values_in_registers`) | 673 | 7 |

**A correction.** The 673 quoted in 0383 and 0389 as "JSON's hot loop" is the loop of
`docs/parser-generators/lexers/json.bitlex`, the program `compile_tests` measures. That lexer is
laxer than simdjson: it checks no UTF-8, and its only string error is a newline. The program
that accepts what simdjson accepts, and that 0383 and 0389 changed, is the ladder's `14-json`.
Its loop is 1,078.

The same program through the two compilers (344 against 437) gives the compiler's own share:
about 1.3 times. The rest of the gap to 1,078 is work that `14-json` does and simdjson's stage 1
does not.

## Part by part

Each part is a stretch of the loop, read from the two listings. The parts add up to each
loop's total.

| Part | simdjson | `stage1.rir`, ours | `14-json` |
|---|---:|---:|---:|
| Load the block, ASCII test | 11 | 17 | 18 |
| UTF-8 check (a block with a byte ≥ 0x80) | 72 | 160 | 204 |
| Strings: backslashes, quotes, escapes, inside a string | 44 | 29 | 52 |
| Classes, structural starts, control characters | 67 | 98 | 80 |
| `\u` escape check (gated region and its test) | — | — | 76 |
| Number check (gate's own lookup, then the region) | — | — | 318 |
| Token starts and ends combined, outputs stored, edge test | — | 7 | 69 |
| Write the structural positions | 147 | 119 | 121 |
| Write the token ends (the records) | — | — | 123 |
| Loop control | 3 | 7 | 17 |
| **Total** | **344** | **437** | **1,078** |

The `stage1.rir` strings part is smaller than simdjson's because its backslash and quote masks
are in the classes part: one lookup gives them all.

### What our compiler adds on simdjson's own program (+93)

- **The UTF-8 check makes masks simdjson does not** (+88). simdjson keeps its UTF-8 error as a
  vector, ORed across blocks and reduced once at the end. It tests "a sequence is still open"
  on the block's last 16 bytes only, as only the block's end matters. The program asks both as
  64-bit masks (outputs 3 and 4), so each block reduces both to words, and the open-sequence
  mask is made over all 64 bytes (37 instructions where simdjson has 2). The lead-byte tests
  are a subtract and a compare each where simdjson has one saturating subtract.
- **Classes and strings** (+16). Both reduce five masks to words; the difference is not traced
  instruction by instruction.
- **Outputs stored, and the edge test** (+7): two error masks stored to the row, and the slot
  load (from the stack) and compare for the edge part (0383). simdjson's loop has no such test:
  its last block is a copy padded with spaces, run after the loop.
- **The ASCII test** (+6): an OR tree and two moves to a word, where simdjson has one `umaxv`.
- **Loop control** (+4).
- **Writing positions** is 28 smaller than simdjson's: ours unrolls 8 and loops, simdjson's
  unrolls 24.

### What `14-json` adds on top (+641 over our `stage1.rir`)

1. **The number check: 318, 30% of the loop.** A gated region: skipped on a block with no
   number bytes and no number running into it. But it is inline, so it is in the loop's code,
   and its gate needs a second class lookup (98) before it. simdjson checks numbers in stage 2,
   once per number, from the token's text (0375 noted this).
2. **The token ends: 123.** Every token's last byte is a second extract (0087's records),
   written as the starts are. simdjson writes starts only; stage 2 finds where a token ends.
3. **The `\u` escape check: 76.** Also a gated region, also stage 2's work in simdjson.
4. **The UTF-8 check: 204 against simdjson's 72.** It makes the same masks as `stage1.rir`
   does, and 44 instructions more, not traced here.
5. **Stack traffic: 31 accesses.** 14 are the block's first 16 bytes: stored once at the head
   and loaded again in 13 places. The other 17 are 8 values held across the loop (2 to 4
   accesses each) and the edge slot's load. `stage1.rir` has 1, the edge slot.

## What would close it, in order of size

1. **Numbers and `\u` escapes checked per token, as simdjson's stage 2 does** (−394). Tier 2
   already visits every token: the number's and the escape's checks are its work on the token's
   bytes. The loop keeps their first bytes only.
2. **Errors as flags, not masks** (about −60 in UTF-8). An output only tested for "any" can be
   ORed into a carried vector and reduced at the end. An output only read at its last bit
   (an open sequence, an unclosed string) can be made on the last 16 bytes only. A program
   would mark such outputs, or the compiler would see that only `any` or the last bit is read.
3. **Records with no second extract** (−123). A token's end is the next start, less any
   whitespace between them. For one-byte tokens the end is the start. simdjson leaves this to
   stage 2; tier 2 could too, reading the gap's bytes.
4. **The spills** (31 accesses). Most come from the regions' code lying inside the loop: with
   1 and 3 done, the loop is near `stage1.rir`'s size, which has 1.

Subtracting 1 to 3 gives about 500 instructions (an estimate, not built): near `stage1.rir`'s
437. What would be left over is the token records' combining and JSON's control-character test.

**Pre-aligned, pre-padded source.** It would not shrink the hot loop: the edge part is already
cold, and the loop's only cost from the ends is the 3-instruction edge test. It would help the
edge part, which remakes 47 values. With the file padded with spaces to a whole block, as
simdjson's last-block copy is, the end's block would be a full block for every byte test, and
only the end's own checks (an unclosed string, a number cut off) would remain. That is not
needed to match the loop, so it is not done here.

## Reproducing

```
scripts/dev/simdjson-stage1-loop [--print]
cd native && RERAIL_DUMP_LOOP=/some/dir ../target/debug/deps/rerail_parser_tests-* \
  --exact json_tests::json_shapes --include-ignored --nocapture
RERAIL_RIR=$PWD/../fixtures/simdjson-stage1/stage1.rir RERAIL_ASM=1 \
  ../target/debug/deps/rerail_parser_tests-* --exact block_tests::rir_loop_size --include-ignored --nocapture
```
