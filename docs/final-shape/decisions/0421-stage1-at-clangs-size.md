# 0421: simdjson's stage 1, compiled by ours, at clang's size

Status: built (audited 2026-11-23). Decided (the user, 2026-11-18: "trace the 35 extra ands and the 2 extra masks on
stage1.rir, fix whatever the compiler does wrong there, and update page 39's numbers"). Follows 0071, 0406, 0417 and
0420.

## Intro

**The question.** simdjson's stage 1 as our block program (`fixtures/simdjson-stage1/stage1.rir`)
compiled to a 389-instruction loop. clang's build of simdjson's own loop is 344. The program
was the same, so the gap was the compiler's or the program's statement. Two counts stood out:

- **`and`: 73 against 38.** 52 on the vectors and 21 on words, against clang's 36 and 2.
- **`addp`: 28 against 20.** Seven masks reduced to words each block, against simdjson's five.

## What the trace found

| Where | Ours | clang | Whose |
|---|---:|---:|---|
| Word `and`s in the position writer | 16 | 0 | the compiler's |
| Masks reduced (4 `and`s, 4 `addp`s each) | 7 | 5 | the program's form, and the compiler's |
| Vector `and`s besides the reductions | 24 | 16 | the program's form |
| A UTF-8 chunk's `and`s | 4 | 4 | the same |

**1. The position writer** (`flatten`). Ours took each position as `ctz(w)`, then cleared the bit
with `w & (w - 1)`: `rbit`, `clz`, `add`, store, `sub`, `and`, six instructions a position.
clang reverses the word once, then takes each position as its leading zeros and clears the bit
with `r ^ (1 << 63) >> z`: `clz`, `add`, store, `lsr`, `eor`, five a position.

**2. The two extra masks** were outputs 3 and 4, the UTF-8 errors and "a sequence still open",
asked of each block as 64-bit masks. simdjson keeps its error as a vector ORed across the
blocks, tested once, and checks an open sequence at the end only. The program said otherwise,
so it was restated. The compiler then still reduced the error, because of a gap in 0406's test:
the error total was a single value on the vectors, so its tree's one leaf was its root, and a
leaf that is an output was refused.

**3. The last 8 vector `and`s** are the classification. `stage1.rir` states the older
simdjson's classify: one table of two nibbles for all five classes, then a test per class. The
simdjson clang builds here classifies by byte. Quote and backslash are a `cmeq` each,
whitespace a 4-instruction lookup, and operators a `cmeq` and a `tbx`. Changing that is a
change to the program's statement, not the compiler, and is not done here.

## The decision

**1. Positions are written as clang writes them** (`flatten`): the word reversed once, each
position its leading zeros, its bit cleared by a shift and an XOR. Past the last set bit `z` is
64, the shift wraps to 0, and the top bit flips. What follows is slack, which the count leaves
out, and the next eight are not entered. This holds in every grammar's loop.

**2. `stage1.rir` states its UTF-8 errors as simdjson keeps them:** two totals, each one flag
for the file.
- Output 3 is the errors.
- Output 4 is a sequence cut short at the end: at the byte past the stream, a lead byte 1, 2
  or 3 bytes before it that needs more (simdjson's `is_incomplete`). The program runs one byte
  past the stream for it (`delay 1`), and the other outputs are not late (`delay 0`). In a full
  block `within` makes it nothing, so the loop has no code for it.

**3. A total that is one value on the vectors is ORed into the error vector** (`flags`): its
root may be its one leaf.

**Two red tests fixed on the way.**
- `the_cpu_executors_take_a_batch` had been red since 0406. It compared rows the CPU no longer
  stores. It now leaves out `cpu_unstored` rows, as `json_compiled_path_equals_the_interpreters`
  does, and compares the totals' flags.
- `only_the_compiler_reads_tier2s_states` matched `.arms` in four files where it is a token
  reader's arms or a count. It now counts `.arms` only in code that reaches `grammar`, whose
  `ArmRow`s the table's arms are. A probe file that does read them still fails it.

## Requirements, a test each

- R1. Stage 1's loop is at most clang's 344, its masks are simdjson's five (20 `addp`), and its
  positions come from one `rbit` (`stage1_loop_is_no_longer_than_clangs`). Undoing the root
  leaf gives 24 `addp`; undoing the writer gives 345 instructions.
- R2. Stage 1 equals simdjson and std on every executor, Metal included:
  - `stage1_equals_simdjson_on_every_executor`;
  - `stage1_utf8_equals_std_on_every_executor`, which reads the compiled code's totals.
- R3. Tier 1 equals the interpreter:
  - the compiler suite, the parser_tests guards, `block_tests` and `compile_tests`;
  - the rr_parser suite (48);
  - the rr JSON set: 789 files, every token the same, and `the_loops_errors_are_the_masks`.

## What changed

| Loop | Before | Now | Stack accesses |
|---|---:|---:|---:|
| simdjson's stage 1, by clang | 344 | 344 | 0 |
| `stage1.rir`, by our compiler | 389 | **332** | 1 |
| `14-json` | 475 | **462** | 1 |
| `15-jsonc` | 644–645 | **632** | 7 |

Of the 57 off `stage1.rir`:
- the writer, 13;
- outputs 3 and 4 as totals, 33;
- the error on the vectors, 11.

The grammars gain only the writer's 13: their UTF-8 errors were already totals.
