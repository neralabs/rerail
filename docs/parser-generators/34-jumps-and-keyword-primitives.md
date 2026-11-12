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

## Built: cheaper calls, fewer words

Each word's machine code is now measured where it sits:
the dump maps every instruction to its word (Cranelift's source locations),
so a word's cost is its own bytes, not its share of the IR.

- **A rule that returns one way gives back 0.**
  412 of the 559 calls go to a rule whose `RETURN`s are all alike,
  and the caller tests the result once (`cbnz`), as a `Result`'s.
- **The calls open are an argument.**
  Within one call of a rule function the count of calls open never changes,
  so it is the function's last argument:
  a call passes one more, a `RETURN` tests it against zero,
  and `TOKEN`'s function takes it for its room check.
  No load or store of the count is left around a call.
  (Dropped since: with code shared from the middle of rules,
  the count spilled; see below.)
- **The stream's end is a token kind of its own.**
  At the end, `TOKEN` sets the token to a kind past the records' kinds,
  so a switch on kinds sends it to its end target in the same compare tree,
  as the lean parser's end token. The separate end test is gone from every switch.
- **A call goes to any rule's code that behaves the same as its callee's.**
  The word merge (`merge_words`) also finds which words behave the same across rules,
  and a call's `GOTO` goes to the first such place in any rule.
  Rules that are copies, whole or from where a call enters, become one function,
  as the lean parser's rules call one helper.
  No word reaches another rule except by a call.

| | Words as calls | Now | Lean parser |
|---|---|---|---|
| Words | 8,563 | 8,144 | |
| Machine instructions | 43,654 | 38,750 | 10,008 |
| Code bytes | 174,616 | 155,000 | |
| Tier 2, instructions | 354.2M | 335.1M | 356.9M |
| Tier 2, cycles | 72.3M | 63.9M | 69.6M |

The load average was about 30 for the last measurement.

Where the machine code is now, by word:

| Word | Words | Instructions each | Share |
|---|---|---|---|
| `SWITCH` | 322 | 27 | 28% |
| `PUSH` | 530 | 10 | 18% |
| `ROW` | 351 | 7 | 8% |
| `MARK` | 337 | 6 | 7% |

A switch already has the lean parser's shape:
rustc compiles its `match`es as compare trees too, with no jump table.
What differs is how many switches there are:
the lean parser has 1,198 compares, tier 2 has 5,102.

### Tried first: code shared from the middle of rules

Beyond whole callees, rules share tails:
code from some state onwards that behaves the same in two rules.
Sharing those as well would save about 6,200 instructions,
through 196 places where one rule goes into another's code.

- **As tail calls** (`return_call`), it saved nothing:
  each one restores every callee-saved register first,
  six `ldp`s and an `add`, at each of 587 places.
- **As a call whose result is passed on**,
  as the lean parser's `return self.helper()`, it still lost:
  - in the largest functions, the register allocator then keeps `ctx` on the stack
    and reloads it at each use: 5,700 stack loads, from 260;
  - regalloc2 splits a value at most twice before it cuts it into one piece a use;
  - the cliff stayed with the platform calling convention,
    with copies at the calls, and with the calls-open count back in memory.

Since the kept copy of shared code is in the earliest rule that has it,
a rule only goes into earlier rules' code,
so such calls nest no deeper than there are rules.
What would make it pay is `ctx` outside the allocation,
in a register of its own for the whole run (Cranelift's pinned register, x21).
`tempo_stream` would save the host's x21 and restore it,
and calls would no longer pass `ctx` at all.
Both are built now: see the next section.

## Built: `ctx` in a register, and code shared from the middle of rules

The aim is the lean parser's code shape, not a faster benchmark.
Each step below is judged by the machine code against rustc's:
how many instructions and functions there are,
what a call looks like, and how much goes through the stack.

- **`ctx` in a register of its own.**
  It sits in Cranelift's pinned register (x21) for the whole run.
  Each function reads it once, into a value the allocator keeps
  in one callee-saved register, as rustc keeps the lean parser's `self`.
  No call passes it. `tempo_stream` saves the host's x21 and restores it.
- **Rules share code from any place on.**
  The word merge now treats words of two rules as alike when they behave the same.
  A word goes to the first place of its kind, which is in the earliest rule that has it.
  So a rule only goes into earlier rules' code,
  and the verifier checks that: a jump that is not a call's
  may go into an earlier rule, never into a later one.
  Compiled, such a jump is a call of the earlier rule's function,
  whose result is passed on, as the lean parser's `return self.helper()`.
  These calls nest no deeper than there are rules.
- **One call for each place gone to.**
  However many words go to the same place in another rule,
  they jump to one block that makes the call.
- **A rule that returns one way gives back 0, across shared code.**
  A rule's ways of returning now include those of the code it goes into.
- **The calls-open count is back in `ctx`.**
  It is read and written where a call is made and returns,
  and read by `TOKEN`'s room check, as the lean parser's `self` fields are.

Three ways to share code were tried and dropped, each because of what it did to the code:

| Tried | What the machine code showed |
|---|---|
| The count as an argument, as before | One value through the whole function, given to every call: regalloc2 split it twice, then reloaded it from the stack at each use (1,148 loads) |
| The count in the high bits of the entry index | The optimizer kept the incoming argument alive to rebuild it, and that spilled instead (1,674 loads) |
| A helper function for each set of rules that share code | 84 functions instead of 56, and 1,700 more instructions |
| A tail call into shared code | An epilogue at each of 198 places (785 register restores against 170) |

| | Callees shared | `ctx` pinned | Now | Lean parser |
|---|---|---|---|---|
| Words | 8,144 | 8,144 | 6,441 | |
| Functions | 56 | 56 | 56 | |
| Machine instructions | 38,750 | 36,258 | 31,702 | 10,008 |
| Code bytes | 155,000 | | 126,808 | |
| Reloads from the stack | | 19 | 17 | 0 |
| Compares | | 5,102 | 4,689 | 1,198 |
| Tier 2, instructions | 335.1M | | 361.0M | 356.7M |
| Tier 2, cycles | 63.9M | | 76.5M | 68.8M |

The load average was about 34 for the last measurement.
More instructions run now, since the code a rule goes into is a call away.
The benchmark is reported, not used to judge the shape.

What stands between a call here and a call in the lean parser:

```
// Tier 2: a rule calls another (14 instructions)
ldr  x5, [x19, #400]    // the calls open
cmp  x5, #1024          // too deep?
b.ge refuse
add  x7, x5, #1
str  x7, [x19, #400]
ldr  x23, [x19, #304]   // the level's word and flags, kept
ldr  x24, [x19, #312]
mov  x2, x25            // the callee's entry
bl   rule
str  x23, [x19, #304]   // given back
str  x24, [x19, #312]
cbz  x2, back           // the way it returned
cmp  x2, #1
b.ne end

// The lean parser (3 instructions)
mov  x0, x19            // self
bl   comments_before
ldr  x8, [x19, #0x58]
```

So the next levers are the call's bookkeeping (the depth check, the kept word and flags,
the three-way result) and the number of compares.
`PUSH` is now a fifth of the machine code (15 instructions each),
and `SWITCH` still most of it (29%).

## Built: a call's bookkeeping as the lean parser's

A rule call cost 14 instructions here and 3 in the lean parser.
The difference was bookkeeping the lean parser does not have:
a count of the calls open, the level's word and flags kept around the call,
and a result tested two ways.
Each one now has the lean parser's shape.

- **The machine's stack counts the calls.**
  The count served four checks; two of them are proved unneeded,
  and two moved to where rustc's code would have them:
  - No `RETURN` runs with no call open, and no `ACCEPT` runs within a call.
    Both are proved over the program,
    by the words reached at the same depth from the start and from each callee.
    Neither is checked at run time.
    A test asserts the proof holds for `.rr`.
  - Too deep: one check where each rule function is entered,
    the stack pointer against a limit in `ctx`, as a stack check in a prologue.
    Past the limit the file goes to the fallback, as before (a test nests 3,000 deep).
  - `TOKEN`'s room for the nodes that returns write:
    the calls open are at most the stack's depth over the smallest rule frame,
    which Cranelift reports once the functions are compiled.
    The bound is loose, but no file of the corpus runs again on a bigger tape.
- **The word and flags are callee-saved.**
  A function whose own words change them keeps the values it was entered with
  and gives them back at its one exit, as a callee-saved register is.
  Calls keep nothing.
- **The callee takes its caller's token.**
  Every call returns to the same `TOKEN`, in its caller, so a `RETURN m=0` takes that token itself,
  as the lean parser's callee consumes the token and its caller peeks at it.
  Then every return lands on the caller's switch, and every call tests its result once.
  This was tried before and dropped: then, `ctx` spilled.
  With `ctx` pinned, it does not.

A rule call and a function's entry now:

```
// Tier 2: a call (3 instructions)
mov   x2, #12          // the callee's entry
bl    rule
cbnz  x2, end          // the run's end, passed on

// A rule function's entry: the stack check
mov   x26, x21         // ctx
ldr   x8, [x26, #232]  // the stack's limit
mov   x9, sp
cmp   x9, x8
b.lo  refuse
```

| | Before | Stack counts calls | Callee-saved context | Callee takes the token | Lean parser |
|---|---|---|---|---|---|
| Machine instructions | 31,702 | 28,962 | 27,433 | 26,575 | 10,008 |
| `PUSH`, instructions each | 15.1 | 8.1 | 4.1 | | 3 |
| Calls tested once | 35 of 366 | | | 366 of 366 | all |
| Stores | 1,826 | 1,435 | 737 | 737 | |
| Stack reloads | 17 | 15 | 10 | 10 | 0 |
| Compares | 4,689 | 4,348 | 4,348 | 3,972 | 1,198 |
| Tier 2, instructions | 361.0M | 350.4M | | 331.1M | 356.4M |
| Tier 2, cycles | 76.5M | 74.7M | | 64.0M | 68.3M |

The load average was about 21 for the last measurement.

Between the callee-saved context and the taken token, the single-way test was also made per entry
(the `RETURN`s reached from where a call enters, not from the whole function):
87 calls tested once, from 35.
It stays as the fallback where the callee cannot take the token.

What is left is mostly the switches: 36% of the machine code, 28 instructions each,
and 3,972 compares against the lean parser's 1,198.

## Built: switches as rustc lowers a `match`

A switch was a binary tree over every run of one target, gaps included.
rustc hands a `match` to LLVM, whose switch lowering does three things this did not:

- **A default target.**
  The tree splits only the cases that are not the default,
  and each side knows the range its value is in.
  So a case at a leaf is one test (an equality, a bound or a range),
  and a switch that expects one kind is one compare, as `expect(kind)`.
- **A keyword's kind is a value, not a branch.**
  The lean parser maps a keyword to its node kind (`Kind::DotDecl + (d - W_DECL)`)
  and then runs one code path.
  Here, a switch over 30 keywords had 30 arms that differed only in the leaf's kind.
  Now a leaf of a keyword's own kind names `WORD_KIND`,
  and takes its kind from the token's word through the program's `wordkind` rows.
  The keywords' strings are numbered in the order of their kinds, as the lean parser's words are,
  so the map is one add over each run of kinds.
  The arms are then the same words, and the lowering keeps one copy.
- **Bit tests.**
  Cases of at most three targets within 64 values
  are a shift and a mask per target, as LLVM makes them when they save enough compares.

```
// One keyword switch, before: a compare per keyword (30 targets)
ldrb  w6, [x25, #353]   // the token's word
cmp   x6, #28
b.lo  ...
cmp   x6, #36
b.lo  ...               // ... 30 more

// After: the words of one arm are one range; the kind is an add in the leaf
ldrb  w6, [x27, #353]
cmp   x6, #21
b.lo  ...
sub   x8, x6, #3
cmp   x8, #10
b.lo  arm

// A bit test: three targets over values 24..32
sub   x8, x2, #24
mov   x7, #1
lsl   x7, x7, x8
mov   x8, #89           // the first target's values
and   x8, x7, x8
cbnz  x8, first
tbnz  w7, #7, second    // one value: one bit
b     default
```

| | Before | A default | Keyword kinds as values | Bit tests | Lean parser |
|---|---|---|---|---|---|
| Machine instructions | 26,575 | 26,092 | 24,828 | 24,375 | 10,008 |
| `SWITCH`, instructions | 8,123 | 7,371 | 6,734 | 6,147 | |
| Compares | 3,972 | 3,755 | 3,439 | 3,004 | 1,198 |
| Tier 2, instructions | 331.1M | | | 330.9M | 356.6M |

The code is smaller, but tier 2 runs about as many instructions as before:
the paths files take were short already, and the cases cut were mostly ones they rarely reach.
(Cycles are not given: the load average was 58.)

What is left in the switches is mostly inherent:
switches whose arms call different rules, as the lean parser's `match` does too.
The largest compare tree left is not a token's switch:
it picks where a call enters a function of 21 entries,
since a rule shared from the middle of another is entered there.
The lean parser's functions have one entry each.

## Built: one entry a function, as the lean parser's

Each of the lean parser's methods is entered at its top.
Tier 2's functions were not:
a function held code other rules went into,
and a compare tree at its entry picked which of up to 21 places a call meant.
The places came from three things, each now gone or made lean-like:

- **A field on a nested choice, set before the call.**
  A rule called a nested choice with a field (`right: product`)
  by entering the choice's code at its first token, from the caller's own switch.
  The lean parser calls the choice, then sets the field where it returns:
  `self.product()?; self.field(Field::Right)`.
  Now so does tier 2: the call's return state sets the field (`NODE_FIELD m=4`)
  on the node the call made, if the node has none,
  so a field the choice sets inside still wins.
  The choice's own function switches on its first token.
- **Short tails alike in two rules.**
  The words were shared across rules where they behaved alike,
  so a leaf and a return in one rule jumped into another rule's copy.
  The lean parser writes each method's code in that method.
  Now each rule keeps its own words. Words are still shared within a rule.
  A keyword's `REMEMBER` takes its kind from the word, as its leaf does,
  so arms that remembered different keywords are one.
- **A term's rest, copied.**
  The grammar's `more` (the rest of a term after its first operand)
  was its own copy of product's and term's loops in each rule that used it,
  and those copies were shared with product's code: jumps into its middle.
  The lean parser calls `self.term()` there.
  `more` is now `[more1]`, a call of a function that takes at least one operator:

```
# Before: the loops again, wherever `more` is used
choice more  = (wrap binary_expression left operator: multiplicative right: primary)*
               (wrap binary_expression left operator: additive right: product)*
# After: a call when an operator follows, as the lean parser's self.term()
choice more  = [more1]
```

The verifier now refuses any jump from one rule into another that is not a call,
so a function is entered by calls alone.

| | Before | Field set where it returns | Own tails | A term's rest a call | Lean parser |
|---|---|---|---|---|---|
| Functions | 56 | 56 | 54 | 55 | 45 |
| Entries | 247 | 171 | 94 | 60 | 45 |
| Machine instructions | 24,375 | 22,226 | 22,258 | 23,981 | 10,008 |
| Compares | 3,004 | 2,759 | 2,737 | 2,808 | 1,198 |
| Tier 2, instructions | 330.9M | 346.1M | | 315.7M | 356.6M |
| Tier 2, cycles | | 67.4M (lean 69.6M) | | 70.7M (lean 75.1M) | |

The code grew by 1,700 instructions in the last step:
each rule's copy of what it once shared.
But tier 2 runs 9% fewer instructions than before it, and 11% fewer than the lean parser,
since no call picks its entry any more.
(Cycles are taken on a shared machine: load 17, then 30.)

Five functions still had a second entry, each a call entered past its callee's start.
Each is now a call at the start, as a lean parser's method is entered at its top:

- **A callee whose first token merges into the leaf before** (a balanced pair's level).
  A state whose arms call such a callee keeps its extras pending,
  as a state whose arms merge does, so the merge still takes them as its text.
- **A callee that may begin with a mark** (`braced_column`, which begins with `one_name`).
  The caller's state tests the mark and keeps it (`MARK`, mode bit 64) for the callee,
  which tests it again and takes it.
- **A call with a flag set for it.** The flag is on the call's `PUSH`, as a parameter:
  set after the caller's flags are saved, and the caller's given back where the call returns,
  as `let f = self.flags; ..; self.call(); self.flags = f`.

Every call now lands on its callee's first place: 55 functions, 55 entries.
The rr test checks it.

| | Before | One entry each | Lean parser |
|---|---|---|---|
| Entries | 60 | 55 | 45 |
| Machine instructions | 23,981 | 23,758 | 10,008 |
| Tier 2, instructions | 315.7M | 312.6M | 356.4M |
| Tier 2, cycles | 70.7M | 61.8M (lean 68.4M) | |

## Measured: where tier 2's code goes

Tier 2 has 23,758 machine instructions, the lean parser 10,008.
By the word that made it (19,294 of them are in words; the rest are word functions,
prologues and epilogues):

| What | Instructions | Share |
|---|---|---|
| Switches (`SWITCH`) | 5,607 | 29% |
| Each state's test for a comment (`ROW`), and its way on | 3,307 | 17% |
| Each state's comment arm (its leaf, back to the token) | 769 | 4% |
| Each state's test for the `no_value` mark, and its refusal | 1,924 | 10% |
| Taking a token (`TOKEN`) | 1,916 | 10% |

Two checks made in every state are 31% of the code,
and about 40% of the gap to the lean parser:

- **Comments.** A comment is a row the record's kind does not name,
  so every state tests for it after its switch and takes it with an arm of its own.
  The lean parser takes comments in its token step (`skip_comments`), once.
- **The `no_value` mark** (a `(` after an aggregate's word). It is not hidden,
  so every state tests it after its token, to refuse it where no arm takes it.
  The lean parser looks for it only where it reads an aggregate's `(`.

## Measured: Cranelift's code against LLVM's, of one IR

How much of the gap is Cranelift's, and how much the IR's?
To find out, the same functions are built a second way, ahead of time:
`clif_rust` writes the parser's Cranelift IR as Rust, tier 1's and tier 2's,
and rustc compiles it with LLVM.
The IR is the one Cranelift is given, after the lowering's own passes,
so the two builds differ only in the compiler.

Each function becomes one Rust function, `#[inline(never)]`, so the calls stay the JIT's.
Each value is a local; a vector is NEON's (`uint8x16_t`), each vector operation its intrinsic.
The graph is written as structured control,
as WebAssembly's backends do ("Beyond Relooper", Ramsey 2022):

```rust
// A loop header: a labelled loop; a branch back to it, `continue`.
'b12: loop {
    // A block two forward branches reach: the code after a labelled block,
    // which those branches leave with `break`.
    'm40: {
        v7 = v3 & 255;
        if v7 != 0 { v9 = v7; break 'm40; } else { continue 'b12; }
    }
    // block40's code
}
```

A first version wrote every function as a loop over a `match` on the block.
LLVM does not thread jumps through a loop's header,
so each branch went through a table: 1,182M instructions for tier 2, 3.8 times the JIT's.

Thirteen of tier 2's functions have a loop entered at two places (an irreducible graph).
Nine are made reducible by copying a block or two for one of the edges,
about 30 instructions each.
The other four, the largest rules, would need more than their own size in copies.
Their loops get one entry instead: a dispatch,
which the edges into the loop reach with a small label (`match d`).

`cargo run -p rerail-rr-aot-gen` writes the source;
the `rerail-rr-aot` crate runs it on the JIT's harness,
and refuses if the JIT's program no longer translates to it.
On the 1,362 corpus files, tier 1 writes the JIT's masks, positions and counts,
and the two tiers the JIT's tape.

Over the 1,360 files the grammar parses, on the profiling build (load 15):

| | Cycles, JIT | Cycles, ahead of time |
|---|---|---|
| Tier 1 | 138.6M | 130.0M |
| Tier 2 | 59.4M | 54.4M |
| The whole parse, a file at a time | 209.2M | 195.8M |

LLVM's code is 6 to 8% faster.
The gap to the lean parser (68.6M) is in the IR, not in Cranelift.

## Measured: the whole parse against the lean parser

The 1,360 `.rr` files the grammar parses, 8.80 MB:
one thread's CPU time, the least of 9 rounds, the profiling build (load 15).

| | Time | MB/s | Cycles | Cycles a node |
|---|---|---|---|---|
| Lean parser, whole | 15.9 ms | 553 | 68.6M | 59 |
| Lean parser, its lexer | 7.5 ms | 1,173 | 32.2M | 28 |
| Bitlex, whole (JIT) | 48.3 ms | 182 | 209.2M | 179 |
| Bitlex, whole (ahead of time) | 45.3 ms | 194 | 195.8M | 168 |
| Tier 1 (JIT) | 32.1 ms | 274 | 138.6M | 119 |
| Tier 2 (JIT) | 13.8 ms | 639 | 59.4M | 51 |
| The engine's parser (`rr::tape`) | 139.8 ms | 63 | 606.0M | 520 |

The whole parse is three times the lean parser's.
Tier 1 alone costs twice the lean parser whole.
Where the 141M cycles of the gap go:

| Where | Bitlex | Lean | Gap |
|---|---|---|---|
| Tier 1 | 139M | 32M (the lexer) | 106M |
| Tier 2 | 59M | 36M (the parse) | 23M |
| The batch's layout, the tape's room, the columns | 11M | | 11M |

Within tier 1, by pass (each run alone over the corpus, so as shares):
the five forward passes 58%, the four backward passes 24%, the record pass 18%.

- **Tier 1 does every check on every block.**
  Its block program is 806 operations, 5,500 machine instructions a 64-byte block,
  whatever the block holds. The lean lexer makes four masks a block,
  then works a token at a time; a string, a comment, an escape
  is read by a small lexer from its start.
  The costliest statements are checks the lean parser makes only where they can fail:
  a `\u` escape's digits (about 100 operations), JSON's rules (`json_bad`, 33),
  the braces' depth (`count depth`, 62), an aggregate's word (`agg_word`, 49).
- **Nine passes over each file.** Each of the 12 `back_fill`s needs a pass backward
  and a forward pass after it. The four backward passes run at under one instruction a
  cycle: a quarter of tier 1's time.
  Each block stores 21 mask words (168 bytes for 64 read) for the passes after.
- **Spills.** Pass 0 holds 45 vectors live, pass 2 73 integers,
  against about 30 and 24 registers: 1,305 stack loads and stores.
- **The record pass** runs 181 instructions a node to turn the masks into records,
  more than the lean lexer's 114 for the whole token.
- **Room and copies** (11M cycles). The batch copies the file and zeroes its mask words;
  the tape is allocated for twice the rows it holds, 55 MB over the corpus.
  The lean parser reads the file where it is.
- **Tier 2**: its states test for comments and for the `no_value` mark in every state
  (above, "where tier 2's code goes").

## Built: comments in the token step

The lean parser skips a comment where it reads the next token,
and records the comment then. Tier 2 used to test for a comment in every state:
an arm on `/`, a `ROW` test to tell a comment from a `/` token, a leaf, a jump back.
Now `TOKEN` does it once, for every state.

- **Tier 1** marks a comment's start in the token's record code.
  This is one more mark bit, beside the keyword marks (`Words::extras`, in `rir`
  `tokenextra b11 #48` for `.rr`).
- **`TOKEN`** with mode bit `TOKEN_EXTRAS`, after reading a record, tests that bit.
  Where it is set, it takes the comment's bytes and makes a deferred leaf
  (as `SPAN m=1; LEAF m=1` did), then reads the next record.
  That loop steps one token each turn: a stated `Token` loop.
- **The states** keep no comment arms; only state 0 does,
  where a comment opens the root.

Tapes are identical over the corpus (1,360 files).

| | Before | After |
|---|---|---|
| Tier 2's words | 5,271 | 3,341 |
| Comment tests (`ROW 12`) | 322 | 40 |
| Comment leaves (`LEAF m=1`) | 140 | 0 |
| Tier 2's machine instructions | 23,758 | 17,460 |
| Tier 2, benchmark instructions (cycles) | 312.6M (61.8M) | 311.3M (60.7M) |

The code is 27% smaller; the time is unchanged.
The removed arms ran only at a `/`, so they cost little at run time.
Each token now pays a shift, a mask and a branch for the comment bit.
The gain in time has to come from the next steps: marks tested only where their token can be,
and one compare on a held kind in place of a ladder.

## Built: a mark refused only where its token can be

`no_value` is a mark on a `(` after an aggregate's word. It is not hidden,
so a state that has no arm for it must refuse it. Each such state tested it
right after its token, whatever the token: 277 tests, one in almost every state.
The lean parser looks for it only where it reads a `(`.

Now each mark has the bytes its token can begin with (`Mark::first`),
found by proof when the program is compiled:

- The mark holds only at the grammar's token starts (`m & ~starts` is never).
  So moving it to the next token start adds nothing.
- A byte is left out where the mark and that byte are proved apart (`never`).
  For `.rr`: `no_value` and `empty_parens` hold only at `(`, `one_name` only at `{`.

A state's refusal of such a mark moves into its switch:
the target of each of those bytes gets the test first.
It stays after the token where a later test jumps to a mark's arm,
because the order of mark tests decides which one wins.
Refusals can move past each other, since each one only refuses.

| | Before | After |
|---|---|---|
| Tier 2's words | 3,341 | 3,193 |
| Mark tests | 320 | 193 |
| Tier 2's machine instructions | 17,460 | 17,278 |
| Tier 2, benchmark instructions (cycles) | 311.3M (60.7M) | 307.1M (58.4M) |

Tapes are identical over the corpus.
The gain is small: each moved test was a bit test on the record's code, two or three
instructions a token.

## Measured: where tier 2's run time goes, by word

Each word's runs over the corpus (the interpreter's), times its machine code
(every path of it, so an upper bound), per token (1,531,635 tokens):

| Word | Runs a token | Instructions a token, at most |
|---|---|---|
| `SWITCH` | 2.67 | 55.8 |
| `TOKEN` | 0.98 | 5.3 |
| `PUSH` | 0.94 | 1.9 |
| `RETURN` | 0.97 | 2.7 |
| `MARK` | 0.30 | 1.2 |
| All others | | under 1 each |

The switches are the cost. A token is switched on 2.67 times:
once by its state, again by each callee entered on it,
and again by its caller where a level ends at it (`RETURN m=1`, 0.73 a token).
The lean parser also tests a token again in each function it calls,
but each test is one compare on the token's kind.

A switch here has two levels. First a compare tree on the record's kind, of which there are 8;
then, for a kind whose words it names (punctuation, keywords), a tree on the word.
A switch of two ways has up to 17.9 instructions on its paths;
a `match` on two kinds needs about five.

Tier 2 is 263 instructions a node, against about 191 for the lean parser's parse
(its 305 whole, less its lexer's 114). That gap is 72 instructions a node,
about 7% of the whole parse. Tier 1's gap to the lean lexer is 532 a node (646 against 114).

## The order

1. One symbol a token: rows and keywords in the record, one switch a state.
   Built for keywords; rows stay `ROW` tests, which are cold.
2. Marks in the record. Built: the record pass writes them into the code.
3. A cheaper `TOKEN`, and the state as the lean parser's.
   Built: no select, nothing held across calls, the tape in rows, one exit a function,
   the state as locals, a token taken for good, words as calls (and so the frame-free path),
   cheaper calls, callees shared across rules,
   `ctx` in a register of its own, code shared from the middle of rules,
   a call's bookkeeping as the lean parser's, switches as rustc lowers a `match`,
   one entry a function (all 55).
   Built: comments in the token step.
   Built: marks refused only where their token can be.
   Next: a token test as one compare on a held kind, and one failure exit a function.
4. Keyword marks from the records. Built for the marks of one word.
5. Tier 1's primitives: the picked hash, then nibble fingerprints.
