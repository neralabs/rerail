# Tier 2's jumps, and primitives for keywords

## Intro

Tier 2 compiled as a function a grammar rule (page 33) has the lean parser's structure,
but each function is too big: 13,119 words, 148,533 machine instructions,
against the lean parser's 12,000 or so.
A third of the words are `GOTO`s.

This page finds what makes the jumps, and the shape that needs none.
Most of them are not the grammar's: tier 2 asks, in each state,
what tier 1 could say once a token.
The second half designs the primitives that say it:
a token's keyword, row and marks, from the hardware this runs on.

Numbers: the `.rr` grammar compiled as rules, over the 1,360 corpus files.

## Where the jumps come from

### In the words

4,785 of the 13,119 words are `GOTO`s. By the word before each:

| Cause | `GOTO`s | Target shared |
|---|---|---|
| A test's branch: a test skips the next word, so a branch is a test and a `GOTO` | 1,666 | 894 |
| The test's other way: a second `GOTO` after it | 1,042 | 971 |
| An arm's end: to the next state, or to code merged with another arm's | 1,047 | 839 |
| A call: `PUSH` the return, `GOTO` the callee | 559 | 480 |
| A `GOTO` to a `GOTO` | 406 | 406 |
| A `GOTO` to the next word | 65 | 0 |

The tests behind the first two lines (2,708 jumps, 57%):

| Test | Count | What it asks |
|---|---|---|
| `WORD m=1` | 984 | Is the token this keyword? In 669 chains, 1.5 keywords each |
| `ROW` | 373 | Which row is the token? Rows that share a first byte |
| `MARK` | 308 | Does a mark hold at the token? Mostly to refuse (`STOP`) |

Each of these asks something about the token, not about the parse.
Tier 1 knows each answer when it writes the token's record:
which row took the token, which keyword its bytes are, which marks hold there.
Tier 2 asks again, in each state that can meet the token,
one test and two jumps at a time.

The lean parser's tokens carry one kind that says all of it,
keywords included. Each rule switches on it once.

### In the machine code

At run time the `GOTO`s cost little: 2.7 instructions a token of 288.
In the machine code they are mostly gone already.
Cranelift's text before emission has a block for each word,
and 13,914 of its 33,961 blocks only jump.
Emission folds them: the code as emitted is 590,312 bytes,
148,533 instructions, and 4,424 of them are unconditional branches (3%).

Resolving the jumps in the lowering, before Cranelift, was tried.
It made the code larger (594,132 bytes) and tier 2 slower
(500M instructions, from 484M), so it was dropped.

What the emitted code is made of:

| Instruction | Count | Why |
|---|---|---|
| `ldr`, `ldur`, `ldp` | 42,748 | The state's fields, loaded in each block; spills |
| `str`, `stur` | 28,105 | The fields stored back; a node's columns, one store each |
| `mov` | 23,621 | Moves between them, constants |
| `cmp`, `b.lo`, `b.eq`, `b.ne`, `cbnz` | 21,332 | Compare trees and guards |
| `cset`, `uxtb`, `orr` | 6,020 | Branches through booleans: `x == 0 \|\| y == 0` is seven instructions |
| `b` | 4,424 | Jumps |

Half the code moves the state between memory and registers.
So the jumps matter in the words: each is the trace of a test
that tier 2 should not need to make.

## The shape that needs no jumps

1. **One symbol a token.** The record's code says the token's row,
   and its keyword where it is one.
   Each state is one switch on the symbol: no `ROW`, no `WORD` test, no `SPAN`
   before them, and none of their 2,708 jumps.
   A keyword that the state takes is a target of the switch.
   One it does not take goes where the row's own token goes,
   or to the state's end where the keyword is reserved.
   This is the lean parser's `match` on the token's kind.
2. **Marks in the record.** Tier 1 sets a mark's bit in the token's record.
   A state that refuses marks tests the record's bits against its own mask:
   one `tst` and one branch, not a `MARK` and a `STOP` a mark.
   A mark a state takes is a symbol bit the switch reads first.
3. **Branches, not booleans.** A guard of two tests is two branches,
   not two `cset`s, an `orr` and a `cbnz`.
4. **Layout.** Within a rule, a jump's target with one way in is placed after the jump.
   A state no larger than a call's cost is copied into the arms that reach it.
5. **The hot state in registers.** The token's place and the tape's end
   are a rule function's arguments and results.
   They are not loaded and stored in each block: half the code now.
   Page 33 found that holding all 23 fields this way is worse.
   Two or three will do.

Steps 1 and 2 change what a record holds, and so the program format.
Each executor changes with it: interpreter, Cranelift and Metal.
Steps 3 to 5 are the compiler's.

## Primitives for keywords

Three questions, each asked by a different part:

- **A token's word.** Is this word token a keyword, and which?
  Once a token, in tier 1's record pass.
- **Where the keywords are.** The byte positions where a keyword begins,
  for the marks that read keywords.
  These are the 180 of tier 1's 865 operations a block that look for letters one byte at a time.
- **A state's keyword set.** Is the token's keyword one of this state's?
  In tier 2, once a state.

### What the machine has

This machine is an Apple M5 Max. The cores report:

- **NEON** with `TBL`/`TBX` (a lookup in a table of up to 64 bytes),
  `CMEQ`, `SHRN` (narrows a compare to a 64-bit mask), `UMAXV`, `ADDP`.
- **CRC32C**, one instruction a 64-bit key.
- **PMULL**, a carry-less multiply.
- **SHA3**: `EOR3`, `BCAX` (three-input bit operations).
- **I8MM** and **DotProd**: `SMMLA`, a 2×8 by 8×2 byte matrix multiply into 32 bits.
- **CSSC**: `CNT`, `CTZ`, `ABS` and `SMIN`/`SMAX` on general registers.
- **SME2.1**, 512-bit streaming vectors (`sme_max_svl_b` 64).
  Its outer products include `BMOPA`, which counts the matching bits
  of every pair of 32-bit elements from two vectors (`SME_BI32I32`).
  It also has `LUTI2`/`LUTI4`, lookups from a 512-bit table register (`ZT0`).
  The machine does not report ordinary SVE, so SVE runs only in streaming mode.
  To be checked with a probe: which SVE2 instructions streaming mode allows
  (`MATCH`, the gathers).
- **The GPU**: SIMD groups of 32, `simd_shuffle` and ballots,
  8×8 `simdgroup_matrix` multiplies, and per Apple, a matrix unit in each core.

### A token's word

Now: the token's first eight bytes, masked to its length,
a multiply and a shift to a slot of a perfect hash, then a compare with the slot's key.
About as the lean lexer does it.

Better primitives, in the order to try them:

1. **A hash the compiler picks for the grammar's words.**
   The words are known at compile time: 64 for `.rr`.
   The compiler tries a few hashes and keeps the cheapest that is perfect over them:
   - a CRC32C of the key: one instruction;
   - `(length, first byte, last byte)` as an index, as gperf does;
   - a multiply and a shift.
   A table of 64 slots or fewer has one byte a slot: one `TBL` holds it in registers,
   and the key's check is the only memory read.
2. **No branch on the token's kind.** Every token goes through the same lines.
   A token that is not a word gets the code "none", by a select, not a branch.
3. **Sixteen tokens at once.** Load the sixteen keys (sixteen loads, no gather),
   then hash, look up and compare in vectors.
   NEON's 64-bit lanes are narrow for this;
   with 512-bit streaming vectors it is eight keys an instruction.
   Entering streaming mode costs, so only a pass over a whole file's tokens pays for it.
4. **A matrix of tokens against words.** `BMOPA` compares sixteen tokens
   against sixteen words, four bytes of each, in one instruction:
   a match is 32 matching bits a word of the key.
   The GPU's 8×8 matrix multiplies do the same in a SIMD group.
   For `.rr`'s 64 words a hash beats this.
   It pays where the sets are large: rule bodies that test names
   against hundreds of strings.

### Where the keywords are

Now: for each letter a keyword uses, a mask of the bytes that are that letter,
shifted and combined, a block at a time.

Better: **nibble fingerprints** (Hyperscan's "Teddy").
Each keyword gets a bucket bit, eight buckets to a byte.
Two `TBL` lookups find the buckets each byte can begin, by its low and high nibbles.
Shifted and combined for the second and third bytes,
sixteen bytes give their candidate starts in about ten instructions,
for every keyword at once.
A candidate is checked against the token's word code, which the record pass has.
With `LUTI4` the same lookups run 64 bytes at a time.

Better still, for marks that only need the next token:
**no byte masks at all.** Tier 2 reads the next record's code (page 33, item 3).
Byte masks stay for what only bytes show: strings, comments, escapes.

### A state's keyword set

Number the codes so each state's keywords, by target, are short runs.
Then for each target:

- **One run:** a subtract and an unsigned compare.
- **Up to 64 scattered words:** a bit set.
  `lsl` the code into a mask, `tst` it against the state's set, branch:
  three instructions, however many words.
- **A word with no arm here:** it falls into the row's own arm, or the state's end.
  It is not tested at all.

A jump table on the code was measured and is worse
(page 33: indirect branches mispredict), so compare trees and bit sets it is.

## The opportunities

| Where | Now | With | Gain to measure |
|---|---|---|---|
| Tier 2: keyword tests | `SPAN`, `WORD`, `GOTO` a keyword a state | One switch on the symbol | 984 tests and 2,000 jumps; `WORD` and `SPAN` a token |
| Tier 2: rows | `ROW` and two jumps a shared first byte, a state | The row in the record | 373 tests, about 1,100 words |
| Tier 2: marks | `MARK` (25 instructions) and a jump a mark, a state | The record's bits, one `tst` | 43 instructions a token |
| Tier 2: the state | Loaded and stored in each block | Two or three fields in registers | Half the code's 148,533 instructions |
| Tier 1: marks by keyword | 180 operations a block | Nibble fingerprints, or the next record | Most of the 180 |
| Tier 1: the word code | Multiply, shift, load, compare | A hash the compiler picks, `TBL` in registers | A few instructions a word |
| Rule bodies: name sets | One compare at a time | `BMOPA`, GPU matrices | For large sets only |

## Built: keywords in the switch

A state's switch now names the keywords it takes (`switch N word sK @P`).
A token that is one goes to its arm, which takes it.
A keyword its row reserves goes to the state's end, or refuses.
Any other token switches on its kind, and the row's own arm tests no keyword.
In Cranelift, the switch reads the token's symbol:
its kind, or 256 and the word's index for a word.
One compare tree takes both.

Where two rows begin with one byte, a token of one row could have the bytes
of the other's keyword, and only `ROW` tells them apart.
There the keyword tests stay: 78 `WORD`s.

| | Before | After |
|---|---|---|
| Words | 13,119 | 8,860 |
| `GOTO` | 4,785 | 2,786 |
| `WORD` | 1,792 | 78 |
| `SPAN` | 1,888 | 1,342 |
| Machine code, as emitted | 590,312 bytes | 530,772 bytes |
| Tier 2, instructions | 484.2M | 481.3M |

The run barely changes, as the keyword tests cost 6 instructions a token.
The code is a tenth smaller, and a third of the words are gone.
Both shapes still give the engine's tape on every corpus file they take.

## Built: the code laid out as the lean parser's

These changes move tier 2's code toward what rustc emits for the lean parser.
They are judged by that shape, not by one run's cycles.

- **Marks.** `TOKEN m=1+k` names mask `k`, where any mark holds.
  `TOKEN` sets the marks live there (and at the stream's end), none elsewhere.
  A state's `MARK` tests then fail on a register, with no load.
  This is the lean parser's one compare of the next comment's place.
- **State in registers within a rule's function** (`HELD_IN_RULES`):
  the token's place, record, bytes taken, marks live, the end before it,
  and the nodes written.
  They are stored at a call and loaded after it, as `&mut self`'s fields are.
- **Branches, not booleans.** A guard of two or three tests is a branch each.
  Refusals are cold blocks, after the hot code.
- **Layout** (`lay_out`), on the words:
  - a target goes past places that only jump on;
  - a jump to a lone `STOP`, or to its rule's `RETURN`, is that word;
  - a run entered by one jump alone goes after that jump;
  - a jump to the next place goes.
- **Kinds numbered** so neighbours share targets:
  a switch on the kind has fewer ranges.

| | Before | After |
|---|---|---|
| Words | 8,860 | 8,570 |
| `GOTO`s | 3,430 | 1,953 |
| Machine instructions | 148,533 | 135,201 |
| `cset`, `uxtb`, `orr`, `csel` | 6,020 | 3,453 |
| Loads | 42,748 | 41,928 |
| Stores | 28,105 | 28,709 |
| `TOKEN`, instructions a token | 61 | 50 |
| `SWITCH`, instructions a token | 87 | 78 |
| `PUSH` and `RETURN`, instructions a token | 26 | 52 |
| Tier 2, instructions (cycles) | 481.3M (89M) | 497.5M (105M) |

The held fields cost at calls: each call stores all ten and loads them after.
The lean parser stores only what its callee reads or changes.
That is the next step: a call's arguments and results, or only the fields it touches.

## Built: calls as the lean parser makes them

- **`ctx` is the state's address,** every rule function's first argument,
  kept by the allocator in a callee-saved register, as `&mut self` is.
  No pinned register, so no move each block.
- **Four fields are passed and given back** (`HELD_IN_RULES`):
  the token's code, start and end, and the stream's end.
  Four fit the argument registers after `ctx` and the entry.
  Ten went through the stack and a return area: 706 KB of code, 525M instructions.
- **The rest of the state is in memory,** read and written where used (`Cut::reload`).
  So is each `ctx` field: none lives across the function and spills.
- **A call's result is 0 or 1** for a return (`cbz`, a compare with 1), as a `Result` is.
  The run's end comes back with its top bit flipped.
  Before, each call compared with `1 << 62`: a `mov`, a `movk` and two compares.
- **A refusal's place is set in a cold block,** not on the hot path before each guard.

| | Held across calls | Passed in registers |
|---|---|---|
| Machine instructions | 135,201 | 159,283 |
| `mov` | 19,768 | 21,568 |
| Tier 2, instructions | 497.5M | 441.0M |
| Tier 2, cycles | 104.9M | 87.6M |

What is left at a call: the four passed fields spill between blocks
(21,725 stack accesses), as rule functions are large.
And the level's word and flags are stored around every call, though few rules use them.

## The order

1. One symbol a token: rows and keywords in the record, one switch a state.
   Built for keywords; rows stay `ROW` tests, which are cold.
2. Marks in the record. Built as the marks live from one mask, at `TOKEN`.
3. A cheaper `TOKEN`, and the hot state in registers.
   Built within a function; across calls still to build.
4. Tier 1's primitives: the picked hash, then nibble fingerprints.
