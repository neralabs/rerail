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
- **Four fields were passed and given back** (`HELD_IN_RULES`):
  the token's code, start and end, and the stream's end.
  Four fit the argument registers after `ctx` and the entry.
  Ten went through the stack and a return area: 706 KB of code, 525M instructions.
  Now none is (below: the tape and the state as the lean parser's).
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

A call keeps the level's word and flags only where its callee's level may change them
(a `REMEMBER`, or a `FLAG` set or cleared, in its function or one it goes on to):
432.5M instructions, 84.0M cycles.

The rules are 55 functions, the smallest 23 words: none is small enough to copy into callers.
The lean parser has 45.

## Built: keyword marks from the records

A keyword mark is written in bytes:

```
mask fix_w  = id_first & c_f & next(c_i) & next(c_x, 2) & ~next(ident, 3)
mark fix_on = fix_w & next(id_ahead, 3)
```

`fix_w` holds where an identifier token's text is `fix`.
The record already says so: its word code, as the lean lexer's `word_of`.
Now tier 2 tests that code, as the lean parser's `is_id(W_FIX)` does,
and the term leaves tier 1.
The mark keeps only its other terms (here `next(id_ahead, 3)`).

The compiler makes the rewrite only where it proves, term by term,
that:

- the term holds only at the grammar's token starts;
- it holds exactly where a row that can begin with the word's first byte starts
  and its token's text is the word.

### The prover

The search prover cannot show the second.
`ident` is `fill(w & alpha, ~w)`, and `w` reads `code`,
which a fixed point computes (`rounds until settled`).
Whether a letter continues an identifier depends on bytes far back.

The new prover (`bitlex_prove::Induction`) writes the question as clauses
for a small SAT solver of its own (`bitlex_sat`, conflict learning, no dependencies):

- **A window of bytes.** Each operation at each lag it is read at.
  Each byte is one of the groups of bytes its readers tell apart.
- **The equations of carried values**, at every lag of the window:
  `toggle`, `escaped`, `fill`, and a fixed point's carry, which equals its next.
- **The stream's two ends.** It begins once and ends once.
  Past the end the bytes are 0x00, as the executors pad them.
  Without these, the solver found "ways" no stream has:
  a byte past the end for one test and in the stream for the next.
- **The window's edge.** Past it, carried values are free.
  A run of bytes that changes nothing can carry a free value to the position,
  however long the window.
  So a way that reaches the edge is checked:
  - First, a stream that begins inside the window.
    A way there is a real stream, and the answer is "no" at once.
  - Otherwise the edge's values become a lemma to prove: "these never hold together".
    The lemma is proved by induction over the stream's positions.
    At the first position where it holds, it held at none before,
    so the proof assumes it fails at every earlier lag.
    A proved lemma is made smaller, then holds at every lag of every later window.

A claim the prover cannot settle within its budget counts as not proved.
That is sound: the compiler then keeps the byte masks.

The first bytes a term can start with come from the same solver.
A real stream gives a byte, its group is set aside, and the solver goes again.
When none is left, one proof shows no other byte can start the term.

### What moved

Five marks now test the record's word: `fix`, `suppress`, `options`, `over`, `policy`.
Tier 1 went from 865 operations a block to 806.
The rest of the 180 letter operations stay, for two reasons:

- other masks read the same letters (`input_w`, `output_w`, the aggregates' words);
- the other marks' terms hold away from token starts too.

Each proof takes 10 to 100 ms in a debug build.

## Built: the tape and the state as the lean parser's

The lean parser's code, read beside tier 2's, showed six differences.
Each is now the lean parser's shape:

- **No load nothing reads.**
  A word loaded each `ctx` field it might read where it started.
  On most paths most were unread, and Cranelift keeps a load that is not read-only.
  A pass before compiling removes each load that can not trap and whose value is unread.
- **Nothing held or passed across calls.**
  The four passed fields were live through every block, and spilled:
  15,133 stack loads, against 161 in the lean parser.
  Now the token and the state are read from `ctx` where used, as `&mut self` is.
- **`TOKEN` branches to its end.**
  The stream's end is a cold block, taken once a file.
  The token's start, end and code are stored as loaded, with no `select`.
- **The kind and the word are bytes of the code.**
  A switch loads the kind's byte (`ldrb`), a keyword test the word's.
  Before, each test masked the code again: 579 `and #0xff`.
- **The marks are in the record.**
  A code is the kind, the word and the token's marks, a byte, a byte and 16 bits.
  The record pass sets a mark's bit where the mark holds at the token's start,
  its word compared for a mark of one word.
  This is on the interpreter, Cranelift and Metal.
  A mark in tier 2 is one bit test.
- **The node table is rows.**
  A node is 24 bytes, as the lean parser's `Node`:
  kind and field (16 bits each), start, end, size, parent, extra flag.
  A word computes its row's address once (`madd`) and stores each field at an offset.
  Before, a node was six column stores, each with its own address.
  The host reads the rows as written.
- **One exit a function.**
  Each return was an epilogue: its saved registers restored, 3,919 `ldp` in all.
  Every return now jumps to the function's one exit, as in the lean parser.

Not done: a frame-free fast path.
The lean parser's small functions return without a frame on their fast path.
That would mean splitting each rule's function into a leaf and its body.

| | Before | After | Lean parser |
|---|---|---|---|
| Machine instructions | 157,702 | 100,134 | 10,008 |
| Stack loads | 15,133 | 365 | 161 |
| Stack stores | 5,450 | 284 | |
| Returns | 731 | 58 | 41 |
| Code bytes | 630,808 | 400,540 | |
| Tier 2, instructions | 440.2M | 351.7M | 356.9M |
| Tier 2, cycles | 108M | 71.2M | 74.2M |
| Tier 1, instructions | 616.3M | 745.3M | |

The cycles are with the machine loaded (load average 60 to 100).
Tier 1 runs more: the record pass loads each mark's word at each token.

## Built: the state as locals

The lean parser keeps a rule's values in registers between the lines that use them.
Tier 2 kept its state in `ctx`, so a value went through memory from one word to the next:
11,912 stores of the state, and as many loads.
Two changes bring part of that to the lean parser's shape.

- **A pass over each function** treats `ctx`'s fields as variables:
  - along blocks of one predecessor (a word falling into the next),
    a field's load after its store, or an earlier load, is that value;
  - a field every path into a block leaves one constant is that constant,
    and a branch on it is a jump (`TOKEN` after a `SPAN` takes a token);
  - a store of the value the field holds, or one every path writes again
    before reading, is removed.

  A call or a return reads every field, as the callee and the host read the state.
- **The innermost level's last node is two fields.**
  With wraps, each level kept the last node made at it, and its base, in a column by depth.
  Only the innermost level's is read, and each change of level writes the new one's.
  So two fields hold it: a leaf no longer stores two columns.

| | Before | After | Lean parser |
|---|---|---|---|
| Machine instructions | 100,134 | 91,391 | 10,008 |
| Loads | 27,604 | 21,097 | |
| Stores | 22,813 | 21,536 | |
| Code bytes | 400,540 | 365,564 | |
| Tier 2, instructions | 351.7M | 347.8M | 356.9M |
| Tier 2, cycles | 71.2M | 76.5M | 76.7M |

What was left of the state's traffic was the transducer's own:
`SPAN` set that the token is taken, and its end (1,684 and 1,683 stores).
The next section removes both.

## Built: a token taken for good

`TOKEN` reads the next token's record, unless the token before it was not taken yet.
So a flag said whether `SPAN` had taken it.
The lean parser has no such flag: it moves its token index where it takes a token,
and its next `match` reads the token there.

The flag was needed because the program did not fix it.
Two kinds of words took a token and then gave it back:

- **A keyword test after the take.**
  Where a keyword shares its first byte with another kind of token,
  the words took the token, then tested its word.
  A token that was no keyword here went to the state's end, taken,
  and the caller switched on it again.
- **A guard after the take** (`unless F`): where the flag was set,
  the state went on as if it had no arm on the token, with the token taken.

The interpreter allowed this, as it reads the next record only at `TOKEN`.
Now the compiler tests the keyword, and the guard, before the take.
The record names the token's word, so the test needs no end.
`SPAN` takes a token for good.

### Each word's phase

A word runs either between a `SPAN` and the next `TOKEN` (the token **taken**),
or between a `TOKEN` and the next `SPAN` (the token **not taken**).
The verifier works out each word's phase from the program (`stream::phases`):

- the program starts as after a token taken;
- a callee starts in its caller's phase;
- `RETURN m=0` goes back to the caller's `TOKEN` with the token taken,
  `RETURN m=1` past it with none;
- a word that reads the next token (`SWITCH`, `MARK`, `ROW`, `SPAN`)
  runs only with it not taken;
- every word but `TOKEN` has one phase.

The verifier refuses a program that breaks a rule.
Two more changes make the compiled grammar keep them:

- a mark's arm goes past the next state's `TOKEN`;
- a level that ends after a mark, where no token was taken, returns with `m=1`.

Over the corpus, the words that run with a token taken are `LEAF`, `MERGE`, `OPEN`,
`CLOSE m=1` and `WORD`: they read the token just taken, as the lean parser's
`push` reads `tokens[i]` before `i += 1`.

### What each executor does with it

Each word means what it did. The verifier proves that each word's phase is fixed,
so an executor need not track it:

- **The interpreter** is the reference. It still reads the next record at `TOKEN`.
- **Cranelift** reads each word's phase at compile time.
  `TOKEN` reads the next record with no test, and `SPAN` stores nothing.
  After a `SPAN`, a token's end is its record's end.
  Before it, the end is the token's start, or the last record's end at the stream's end.
  Neither phase keeps a field.
  A program where a `TOKEN` has both phases keeps the flag, but the compiled grammar has none.
- **Metal** runs tier 2 today as the Cranelift code, over tier 1's buffers
  in the device's shared memory (`tier2_on`), so it has the same code.

### Tier 2 in MSL

Tier 2 in MSL is not built yet. With the phase fixed by the program, it works as follows:

- **One thread per file**, files batched (`13-tape-on-the-gpu.md` §5.4).
  The state is the thread's registers: the token's start, end and code,
  the next record's index, and the counts.
- **No flag, so no branch on it.**
  Lanes of a SIMD group run different files.
  A `TOKEN` that tested a flag was a branch whose way differed between lanes.
  Now every lane at a `TOKEN` takes a token.
- **The next record is read at `SPAN`.**
  On the GPU a record's three columns are device-memory loads, hundreds of cycles away.
  Between a `SPAN` and the next `TOKEN` no word reads the next token (the verifier's rule),
  so MSL can issue the loads at `SPAN`, into registers of their own.
  Then `TOKEN` moves them into place, with the leaf's row writes in between to hide the wait.
  On the CPU the loads stay at `TOKEN`, as the lean parser reads its token at its `match`.
  The words are the same; where a load goes is the backend's choice.
- **Rule calls** are an explicit return stack (the `RETURNS` column),
  as MSL has no recursion and no cheap indirect calls:
  a `RETURN` is a compare tree over its return places.
  The stack is in device memory, so threadgroup memory stays well under 32 KB.

| | Before | After | Lean parser |
|---|---|---|---|
| Machine instructions | 91,391 | 87,152 | 10,008 |
| Stores | 21,536 | 18,166 | |
| `Need` and `End` stores | 3,367 | 0 | |
| Code bytes | 365,564 | 348,608 | |
| Tier 2, instructions | 347.8M | 339.0M | 356.7M |
| Tier 2, cycles | 76.5M | 74.3M | 76.6M |

## Next: words as calls

The lean parser is 10,008 machine instructions; tier 2 is 87,152.
Its words cost about what the lean parser's code does: a `LEAF` stores a row,
as the lean parser's `push` does.
The difference is how many places have that code.
The lean parser makes leaves at about 100 places,
most of them inside helpers the grammar's functions call:
`id()`, `word()`, `string()`, `p_stored()`, `bump_as()`.
Tier 2 lays out a node's code at each word: 1,097 `LEAF`s, 117 `OPEN`s, 159 `WRAP`s.

Each word's Cranelift instructions, by operation, before optimizing:

| Word | Words | Instructions each | Share |
|---|---|---|---|
| `LEAF` | 1,097 | 25 | 21% |
| `TOKEN` | 340 | 59 | 16% |
| `SWITCH` | 340 | 42 | 11% |
| `WRAP` | 159 | 88 | 11% |
| `PUSH` | 559 | 18 | 8% |
| `RETAG` | 191 | 47 | 7% |
| `NODE_FIELD` | 703 | 11 | 6% |
| `CLOSE`, `MERGE`, `OPEN` | 348 | 32 to 41 | 10% |

`TOKEN` and the words that make or change nodes are about 70% of the code.
Each one can be a generated function that every place calls,
with its kind and field as arguments:

- **The state stays in `ctx`,** so a call needs no state passed.
  After the call the caller reads the fields again,
  as the lean parser reads `self` after a call.
- **A word with a refusal** gives back 0, or the refusal,
  and its caller jumps to its own refusal with its place.
- **The functions call nothing,** so they need no frame:
  that is the frame-free path, as the lean parser's small functions have.
- **`SWITCH` stays where it is:** it is the jump table each state needs.

Two more steps then make fewer words:

- **A leaf of its token's own kind,** as the lean parser's `anon()`:
  its kind from a table by the record's code.
  Arms that differ only in their token's kind are then one arm.
  Merging words that act alike would give 5,450 places within rules, from 6,011.
- **Code shared across rules.**
  The grammar repeats alternatives in several rules (`unsigned` and `lattice_unsigned`,
  `literal` and `negated`).
  Merged across rules, the words would be 4,405 places, from 6,011.

## Built: words as calls

With a function a grammar rule, these words are generated functions of their own,
which every place of the word calls:

- `TOKEN`;
- `LEAF`, `OPEN`, `CLOSE`, `WRAP`;
- both `MERGE`s;
- `RETAG` and `NODE_FIELD` on the node made last, the moved node and the open node.

- **Each body is written once,** over what it reads and writes
  (`WordEnv`: the state's fields, `ctx`'s columns, a refusal).
  With one function for the whole program (the JSON grammar, the main loop),
  the same body is made in place.
- **The state is in `ctx`,** so a call passes `ctx` and the word's kind, nothing more.
  After it, the caller reads the fields it needs again, as the lean parser reads `self`.
- **A word that may refuse gives back 1,**
  and its caller refuses at its own place: the refusals are the interpreter's, place for place.
- **The functions call nothing, so they have no frame:**
  each is loads, the row's stores and a `ret`, as the lean parser's inlined `push` is.
  This is the frame-free path.
- **A field after its node rides in the call.**
  A `NODE_FIELD` on the node a `LEAF` or `OPEN` just made is passed with the kind:
  the row stores the kind and the field as one 32-bit word,
  so the field costs nothing, and the field word is skipped.
- **A keyword assertion is one compare and a branch.**
  It was a `bnot` of a compare, which Cranelift made a `cset`, an `eor` and a `uxtb`.

| | Before | After | Lean parser |
|---|---|---|---|
| Machine instructions | 87,152 | 43,654 | 10,008 |
| Loads | 20,595 | 5,667 | |
| Stores | 18,166 | 2,541 | |
| Code bytes | 348,608 | 174,616 | |
| Tier 2, instructions | 339.0M | 354.2M | 356.7M |
| Tier 2, cycles | 74.3M | 72.3M | 80.6M |

The machine was loaded (load average 57 to 62) for the last row, so the cycles are rough.

Tried and dropped: a callee taking its caller's token at `RETURN m=0`,
so that every return lands past the caller's `TOKEN` and the caller tests once.
The register allocator then kept `ctx` on the stack in many functions
(1,780 stack accesses, from 439): 49,440 instructions.

What is left, by Cranelift instructions in the rule functions:

| Word | Words | Share |
|---|---|---|
| `SWITCH` | 340 | 19% |
| `PUSH` | 559 | 13% |
| `LEAF` | 1,097 | 11% |
| `TOKEN` | 340 | 7% |
| `ROW` | 373 | 7% |

- **A call** is about 12 instructions, against the lean parser's 3:
  - the calls open counted and checked against a depth (5);
  - `ctx` and the place in registers, and the call (3);
  - a three-way test of its result (4).
- **Fewer words,** from the two steps above:
  a leaf of its token's own kind, and code shared across rules.

## The order

1. One symbol a token: rows and keywords in the record, one switch a state.
   Built for keywords; rows stay `ROW` tests, which are cold.
2. Marks in the record. Built: the record pass writes them into the code.
3. A cheaper `TOKEN`, and the state as the lean parser's.
   Built: no select, nothing held across calls, the tape in rows, one exit a function,
   the state as locals, a token taken for good, words as calls (and so the frame-free path).
   Next: cheaper calls, then fewer words.
4. Keyword marks from the records. Built for the marks of one word.
5. Tier 1's primitives: the picked hash, then nibble fingerprints.
