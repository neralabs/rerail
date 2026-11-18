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

**Now (2026-11-18, after 0421).** The table above is the page's starting point; the sections
below take it to these:

| Loop | Instructions | Stack loads or stores |
|---|---:|---:|
| simdjson's stage 1, by clang | **344** | 0 |
| `stage1.rir`, by our compiler | **332** | 1 |
| JSON's tier 1 (`14-json`) | **470** (a block with no `\u`: about 390 run) | 1 |
| JSONC's tier 1 (`15-jsonc`) | 632 | 7 |

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

## After 0398 and 0399 (2026-11-18)

Items 1 to 3 above were taken up in order.

| | Hot loop | Stack accesses | Tier 1's operations |
|---|---:|---:|---:|
| At this page | 1,078 | 31 | 186 |
| Numbers per token (0398) | 802 | 17 | 138 |
| `\u` escapes per token (0399) | **772** | 20 | 122 |

Retired instructions, both tiers, against the build before 0398:

| File | Change |
|---|---:|
| `twitter.json` | −3.7% |
| `twitterescaped.json` (31,818 `\u`) | +17% |
| `numbers.json` + 60,000 synthetic numbers | +21% |

**Item 1 moves cost, by density.** Tier 1's region cost was per block, and skipped where a
block had no number or escape. Tier 2's is per token. A number's check costs about 116
instructions, against about 255 saved a block, so a file with more than about 2 numbers a
block costs more. simdjson's stage 2 pays the same per token, but converts the value there too.

**Item 2 does not apply as stated.** Its premise was that the UTF-8 errors are masks reduced to
words only to be tested for "any". In `14-json` and `json_parser/json.bitlex`, the totals'
values are word operations over values the lane plan makes as words: the UTF-8 check's
`prev(…, 1)` of carried values, and a toggle's. ORing the totals on the vectors needs vector
leaves, and there are none. The work that differs from simdjson's 72 instructions is those
values being words: simdjson's `prev1` is a vector `ext` with the block before's bytes. The
mechanism, the totals made one flag at the first, its edge part making them exactly, is kept
as a patch outside the tree. It fires on no grammar here.

**Item 3: token ends from the gaps (0400).** A token ends at the next start, less the
whitespace between them, as this page put it; Bitlex proves it for every row of `14-json`. The
CPU's tier 1 makes no lasts, and its record pass makes each from the next start and the skip's
mask: one word and a count of leading zeros.

| | Hot loop | Stack accesses | Tier 1's operations |
|---|---:|---:|---:|
| Token ends from the gaps (0400) | **616** | **5** | 123 |

JSONC kept its extract at first: the proof did not reach its comments (0400). With the claim
as its own induction hypothesis, and lemmas that two regions are never open at one byte in a
file not refused, it does (0401):

| `15-jsonc` | Hot loop | Stack accesses |
|---|---:|---:|
| After 0400 | 988 | 33 |
| Token ends from the gaps (0401) | **814** | **27** |

## Item 2 again: the UTF-8 rules' form (0402)

The premise above, that the lane plan makes the UTF-8 check's previous-byte values as words,
came from the program as Bitlex writes it, where `~prefix` cuts every class that holds a byte
of the byte order mark. The CPU compiles the program the host strips (0389), and there the
check is on the vectors, its `prev`s as `ext`s. The gap was the rules' form: four two-byte
rules, each a compare, an `ext`, a test and an `and` a chunk. They are now one `pair` lookup,
as simdjson's `check_special_cases`.

| | Hot loop | Stack accesses | UTF-8 stretch |
|---|---:|---:|---:|
| `14-json` after 0400 | 616 | 5 | 195 |
| `14-json`, rules as one pair (0402) | **574** | **1** | **153** |
| `15-jsonc` after 0401 | 814 | 27 | |
| `15-jsonc`, 0402 | **784** | **11** | |

simdjson's UTF-8 check is 72. What is left of it:

- **The block's first chunk** (about 25 instructions). Its previous bytes are loaded again at
  −1, −2 and −3 and looked up again; simdjson carries the previous block's input vector.
  Carrying it here would make a vector a carry between blocks, which the lane plan avoids so
  that a gated region can skip a `prev`.
- **The error's reduction** (12). Page 39's item 2 as written, errors as flags, needs totals no
  one reads by their rows (0347), and `14-json`'s tier 1 runs no CPU token loop, so its totals
  have rows.
- **The other rules' form.** The three-byte "a continuation is needed" test, and the bytes that
  never start a sequence, are tests of the class bits apart, where simdjson's lookup folds
  them into one table.

### Spike: simdjson's carried previous block (2026-11-18)

The user: "spike on it; in general, anything that SIMDJSON does we need to be able to do when
it's profitable". The spike ran in two steps:

1. **The bytes before the block as a chunk of their own.** Chunk 1 made each `prev(a, k)` again
   from its own load at −k, with its own lookups. Made over the 16 bytes before the block
   instead, `a` is made once, and every `prev` of chunk 1 is an `ext`, as in chunks 2 to 4. No
   carry, so the gated regions are untouched.

   | | `14-json` | `15-jsonc` |
   |---|---:|---:|
   | Chunk 1 by lags (as is) | 574 / 1 | 784 / 11 |
   | Chunk 0, every `prev` | 568 / 1 | 792 / 13 |
   | Chunk 0, a `prev` of a lookup only | 568 / 1 | 790 / 12 |

   (Hot loop instructions / stack accesses.) It is a gain only where the `prev`s share a
   table's lookup. For a `prev` of one byte's compare, a load and a compare at −k cost what an
   `ext` does, and hold no register.
2. **simdjson's carry.** Chunk 4's vectors handed to the next block would leave out what step 1
   makes over the bytes before the block: one load and one lookup (6 instructions), and its
   three tests (3). That is 9 instructions a block at most, about 1.5%. The price is a vector
   carried between blocks, kept right on the ASCII path, the edge part, the general body and
   the regions, and 1 to 3 registers held across the loop.

Neither is profitable here on its own count. The user rejected that criterion: a step that
puts simdjson's structure in place is kept even at a flat count. Step 1 is now 0403, and the
carry is the step it prepares.

**Where the profit is.** A steady chunk of the check is 29 instructions against simdjson's
17, about 48 a block:

- simdjson's one lookup holds all of its two-byte rules. Ours, since 0402, holds the four
  overlong and surrogate rules, while "a continuation is needed, by lag 1" and "never a lead
  byte" are separate class tests, each with its `ext`.
- The current byte is looked up in two tables, since its classes need more than 8 bits.
- The lags 2 and 3 are tests of class bits, where simdjson's are two saturating subtracts of the
  raw bytes.

Folding the lag-1 rules into the pair's table is the next simdjson step that pays.

### simdjson's lookup form (0404)

All three are done: the grammars state the check as simdjson's `lookup4` rules, every rule of
two bytes is a bit of one pair of tables, this byte's table is one `tbl` of high nibbles, and
lags 2 and 3 are `prev2 ≥ 0xe0 | prev3 ≥ 0xf0` against `TWO_CONTS`.

| `14-json` | Hot loop | Stack accesses | UTF-8 stretch | Steady chunk |
|---|---:|---:|---:|---:|
| After 0402 | 574 | 1 | 153 | 29 |
| Chunk 0 (0403) and the lookup form (0404) | **549** | **1** | **128** | **25** |

JSONC: 784 / 11 → 735 / 8. simdjson's stretch is 72 and its chunk about 17. Left:

- the bytes that never start a sequence, two products no pair bit holds, on a lookup of their
  own;
- each rule tested to a 0 or 1 lane, where simdjson keeps the ANDed bytes;
- the carried previous block.

### simdjson's structure (0405)

All three are done: the bytes that never start a sequence are pair rules, merged into
simdjson's bits; the rules are made as bytes and tested once a chunk; and the hot loop carries
its last chunk to the next block.

| `14-json` | Hot loop | Stack accesses | UTF-8 stretch | Steady chunk |
|---|---:|---:|---:|---:|
| After 0404 | 549 | 1 | 128 | 25 |
| 0405 | **507** | **1** | **85** | **17** |
| simdjson | | | 72 | 17 |

JSONC: 735 / 8 → 693 / 9. The steady chunk is simdjson's. What is left of the stretch is the
error's mask, about 9 instructions a block, where simdjson ORs error bytes into one vector for
the file: that is item 2's flag (0402).

### The error vector (0406)

Item 2's flag now fires. A total no mask, mark or token-loop read names has no row without a
token loop too, and the UTF-8 error's bytes are ORed into one vector carried round the hot
loop, reduced where the loop leaves, as simdjson's `error`.

| `14-json` | Hot loop | Stack accesses | UTF-8 stretch |
|---|---:|---:|---:|
| After 0405 | 507 | 1 | 85 |
| 0406 | **485** | **1** | **76** |
| simdjson | | | 72 |

JSONC: 693 / 9 → 667 / 11.

### simdjson's registers (0417)

The four ORs a block were simdjson's four `error |=` already. What differed was the ASCII
way's copy of the error vector, lags 2 and 3 compared before their `ext` (and chunk 0 compared
again each block), and six constants built in the loop. Now lags 2 and 3 are `ext`s of the
bytes, then compares, as simdjson's `prev<2>(input)`, and the vector passes no join. Each
rowless total, and each word leaf of the flag, is ORed into the running word where made, an
edge block remaking them.

| `14-json` | Hot loop | Stack accesses | UTF-8 stretch |
|---|---:|---:|---:|
| After 0406 | 485 | 1 | 76 |
| 0417 | **481** | **1** | **74** |
| simdjson | | | 72 |

JSONC: 667 / 11 → 651 / 7. A chunk of the stretch is simdjson's 16 instructions. The gap is the
six constants (`movi` of `0xe0`, `0xf0`, `0x80`, three table loads) built in the loop, where
simdjson holds them in registers. The mid-end's code motion would hoist every constant and
spill them (0069), so hoisting needs to choose which constants to hold, against the vector
pressure of the number and string checks (peak 32).

### The constants in registers (0418)

The six constants are parameters of the hot body, made where the loop is entered and round
the back edge, and made again on the way out of the cold edge part, which holds 37 vectors.
Made at the function's entry, they were spilled there. The slow way also makes its error
bytes first, so each chunk's `ext`s are made where read.

| `14-json` | Hot loop | Stack accesses | UTF-8 stretch |
|---|---:|---:|---:|
| After 0417 | 481 | 1 | 74 |
| 0418 | **475** | **1** | **68** |
| simdjson | | | 72 |

JSONC: 651 / 7 → 647 / 8. The stretch is the four chunks' 64 and four ORs. simdjson's 72
includes its incomplete-sequence check, folded away here in a full block.

### Masks taken out where read (0420)

Each mask leaves its vector (`extractlane`) just before its first reader, so it is not held in
an integer register meanwhile. JSONC: 647 / 8 → 644–645 / 6–7; `14-json` 475 / 1. Parking idle
loop values in vector registers was tried and rejected: it costs about what a spill costs.

### Stage 1 at clang's size (0421)

The trace of `stage1.rir`'s loop against clang's (389 against 344):

| Where | Ours | clang |
|---|---:|---:|
| Word `and`s in the position writer | 16 | 0 |
| Masks reduced to words | 7 | 5 |
| Vector `and`s besides the reductions | 24 | 16 |

- **The writer** (the compiler): `ctz` and `w & (w - 1)` a position, where clang reverses the
  word once and takes `clz`, clearing the bit with `^ (1 << 63) >> z`. Now clang's: −13 in
  every loop.
- **The two masks** (the program): the UTF-8 errors and the open sequence were asked as masks.
  They are now totals, as simdjson keeps its error: the open sequence is tested at the byte past
  the stream, which a full block folds away (−33). The compiler still reduced the error, a
  total that is one vector value: 0406's flag refused a root that is its own leaf. Fixed (−11).
- **The last 8 vector `and`s** are the program's classify, the older simdjson's two-nibble
  table for all five classes. The simdjson clang builds here uses `cmeq` for quote and
  backslash, a whitespace lookup and a `tbx` for operators. That is the program's statement,
  left as it is.

| | `stage1.rir` | `14-json` | `15-jsonc` |
|---|---:|---:|---:|
| After 0420 | 389 / 1 | 475 / 1 | 644–645 / 6–7 |
| 0421 | **332 / 1** | **462 / 1** | **632 / 7** |
| clang's simdjson | 344 / 0 | | |

`14-json` is 118 over clang's stage 1. Its extra checks (control characters, the first bytes
of numbers and escapes) are 130 over `stage1.rir`: the next part to trace.

### What `14-json` adds over stage 1 (0423)

Nine masks a block against stage 1's five. The four more:

| Part | Instructions | Runs |
|---|---:|---|
| The surrogate check (`d`, `8 9 a b`, `c`–`f`: a second table, 3 masks) | 79 | behind its gate since 0423 |
| The `u` of an escape | 13 | in the string-body region |
| The token kinds' word logic (content, escapes, punctuation, words, the skip) | about 46 | every block |

The surrogate check's masks were made every block, before its gate (a `\u` in the block): the
compiler made a region's own vectors outside its branch when its members read them as words.
Now they are made inside, and the compiler refuses a function that does otherwise. The code is
8 longer; a block with no `\u` runs about 60 fewer, and tier 1 retires 5.4% fewer instructions
over 17 corpus files.

What is left over stage 1 is the grammar's split of the work. Tier 2 reads the token kinds
simdjson's stage 2 finds by rescanning. The surrogate check is stage 2's in simdjson, done per
`\u` when it converts one; moved to tier 2 as 0399 moved the hex digits, it would take the
region, its table and the `u` mask out of tier 1.

## Reproducing

```
scripts/dev/simdjson-stage1-loop [--print]
cd native && RERAIL_DUMP_LOOP=/some/dir ../target/debug/deps/rerail_parser_tests-* \
  --exact json_tests::json_shapes --include-ignored --nocapture
RERAIL_RIR=$PWD/../fixtures/simdjson-stage1/stage1.rir RERAIL_ASM=1 \
  ../target/debug/deps/rerail_parser_tests-* --exact block_tests::rir_loop_size --include-ignored --nocapture
```
