# 0493: The newline index is made from tier 1's bits

Status: decided (the user, 2026-11-20: "go ahead in that order", the second of page 40 step 34's three changes).
Follows 0480.

## Intro

**What the newline index is.** Every format 16 tape holds an index of its source's newlines (`tape_format::newlines`),
so that a reader turns a byte offset into a line and back. It is the offsets of the `\n` bytes in Elias–Fano form:
high bits, low bits and samples.

**The problem.** The writer made it in `Writer::begin` by scanning the whole source with `memchr`, twice: once to count
the newlines and once to place them. Tier 1 has already read every byte, as simdjson's stage 1 does. So the scan was a
second pass over the source. In a profile of `twitter.json` it was about 13% of the parse's time.

## The decision

1. **Tier 1 writes the stream's newlines as an output** (`BlockProgram::newlines`). Every program with a tree section
   gets one more output: the class of the byte `\n`, at its own delay. Like `extracts`, `totals` and `masks`, the field
   names a value that is also an output. It is carried through `live_of`, the passes' split, the program's text
   (`block N newlines %v`), its canonical bytes and its verification.

2. **The index is made from those words** (`newlines::index_of`, `newlines::bits_of`). Its bytes are the scan's, byte
   for byte. `index_of` takes the newlines' count and their offsets in order, and writes the index in one pass over
   them, into a buffer it keeps.

3. **The writer takes what writes the index** (`Writer::begin_with`, with the document in a `Document`). `begin` is
   `begin_with` with the scan, for every producer without tier 1's bits. The index is written into the LINES part's own
   buffer, kept from one document to the next (0480).

4. **Tier 2's format 16 run passes tier 1's bits** (`Parser::image16_in`). After `Batch::call` the output words are in
   the file's coordinates: the host puts a stripped prefix (the byte order mark) back, and a prefix holds no newline.
   So no offset is added. In a debug build the run checks the index against the scan.

5. **Amended 2026-11-20 (0499): only where tier 1's body stores them.** A token loop's tier 1 (`.rr`'s, 0177) has a
   body that makes no mask, as the lean lexer makes none: the loop reads each mask from the bytes where it needs it,
   and no row is stored for an output the record pass makes (`cpu_unstored`). The newline output shares its value
   with a line comment's end rule, which the loop reads from the bytes, so its rows were never stored. The index
   made from those words was empty, and the debug check caught it on the AOT test's `.rr` grammar (`"\n"`).
   `CompiledBlock::newlines` is now the output only where the CPU's body stores its rows. A token loop's tier 1
   offers none, and its index is the scan's. `tier2_tests::the_newline_output_is_every_newline` checks both: JSON's
   forms store it, the `.rr` grammar does not.

## Measured

`json_tests::simdjson_form_against_simdjson`, `--profile profiling`, arm64, one thread, load about 44:

| File | Ours, instr/byte, before | After | simdjson |
|---|---:|---:|---:|
| `twitter.json` | 48.7 | 48.7 | 5.8 |
| `numbers.json` | 44.6 | 45.0 | 14.2 |

The instructions don't change. `memchr` over the source took few instructions per byte; its cost was the time of a
second pass over 631 KB. In a profile of `twitter.json` (`sample`, idle threads left out), the index is now about 4% of
the parse's samples (`index_of`), down from about 13% (`newlines::index`, mostly `memchr`). Tier 1 is 4.58
instructions a byte, up from 4.35, for the one output more. The time ratio against simdjson read 8.4× (9.4× before),
but at load 44 the times are not a measurement.

## Checks

- `newlines::tests::an_index_from_bits_is_the_scans`: from a bitmap at delays 0, 1, 5 and 63, with bits outside the
  source set, the index is the scan's.
- `json_tests::simdjson_forms_rust_trees_are_the_jits` compares every format 16 tape against the writer's own, made
  from format 15 with the scan. New cases: newlines after a byte order mark, and newlines at the start and the end.
  A newline offset moved by one byte fails it.
- The JSON and probe AOT ladders, the compiler's tier 2 tests, and the quick `.rr` tests pass.

## Later: a word at a time

`index_of` first wrote each newline's high bit and low bits as reads and writes of the index's bytes, then made the
two sample lists in two more passes over the high words. It now keeps the high word and the low bits in registers and
stores each once it is whole (the offsets ascend), and makes both sample lists in one pass. `twitter.json`: 39.04 →
38.92 instructions a byte. Its share of a profile was time more than instructions. The test above
(`an_index_from_bits_is_the_scans`) is unchanged and passes.

## What is left

`bits_of` reads the newline words twice, once to count and once to place. A popcount kept by tier 1 (a running count,
as its extracts have) would take the first read away.
