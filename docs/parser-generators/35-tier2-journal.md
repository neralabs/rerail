# Tier 2's code shape: a journal

## Intro

Tier 2 is the `.rr` parser compiled from the Bitlex grammar,
a function a grammar rule, lowered through Cranelift.
The aim is the code the lean Rust parser (`rr_syntax/src/lean.rs`) compiles to:
the same structure, within a margin.
Each step is judged by the machine code against rustc's,
not by the benchmark, which is reported alongside.

This page keeps three things:

- **The best to date**: the structure tier 2 has now, and its numbers.
- **Tried and dropped**: each approach measured and not kept,
  with what the code showed, and what could make it worth trying again.
- **The journal**: each step, in order, with its numbers and its commit.

Pages 33 and 34 explain each step at length.

How the numbers are taken:

- Machine instructions, functions, stack reloads and compares
  come from tier 2's code for `.rr`, disassembled.
- A stack reload is a load from `[sp]` that is not an epilogue's restore.
- The benchmark is tier 2 and the lean parser over the 1,360 corpus files:
  instructions and cycles, on a shared machine (its load is noted).

## The best to date

Tier 2 at `9a017c9b9`: the best in shape and in the benchmark.
(`fa23bf4e5` has fewer machine instructions, 22,226,
but functions entered at many places, as the lean parser's are not.)

| | Tier 2 | Lean parser |
|---|---|---|
| Machine instructions | 23,758 | 10,008 |
| Functions for rules | 55 | 45 |
| Entries | 55 | 45 |
| Words | 5,271 | |
| A rule call | `mov`, `bl`, `cbnz` | `mov`, `bl`, a load |
| Stores | 755 | |
| Compares | 2,810 | 1,198 |
| Benchmark, instructions | 312.6M | 356.4M |
| Benchmark, cycles | 61.8M (load 16) | 68.4M |

The next gap, measured (page 34): two checks made in every state are 31% of the code.
Comments are tested and taken in each state (4,076 instructions),
where the lean parser takes them in its token step;
the `no_value` mark is tested in each state (1,924),
where the lean parser looks for it at an aggregate's `(` alone.

Its structure:

- **A function a grammar rule**, and a call a `PUSH`, as the lean parser's methods.
- **`ctx` in a register of its own** (Cranelift's pinned register, x21) for the whole run,
  as rustc keeps `self`. No call passes it.
- **The state in `ctx`**, read and written where used, as `self`'s fields.
  Nothing is held in a register across a call.
  Within a function, a pass forwards fields along straight-line code.
- **Words as calls**: `TOKEN`, `LEAF` and the node words are functions
  every place calls, one body each. Only the main loop is inlined.
- **Words that behave the same are one word**, within a rule.
  Each rule keeps its own code, as each of the lean parser's methods does,
  so a function is entered by calls alone, at its top.
  The verifier refuses any jump across rules that is not a call.
- **A field on a called choice is set where the call returns**,
  as `self.product()?; self.field(Field::Right)`,
  on the node the call made if it has none.
- **A term's rest is a call** (`more` is `[more1]`), as the lean parser's `self.term()`.
- **Every call enters at the callee's start**: a merging callee too (the caller's extras pending),
  a marked one (the caller keeps the mark for it), and a flag the call runs with is on its `PUSH`.
- **One exit a function**, one epilogue, as rustc's.
- **A rule's result**: 0 for every return, the run's end with its top bit flipped:
  one `cbnz` a call.
  A callee takes its caller's token at `RETURN m=0` (every call returns to one `TOKEN`, alike),
  so every return lands on the caller's switch.
  Where that does not hold, a call of a place with one way tests once, else twice.
- **The calls open are the machine stack's**, as the lean parser's.
  No `RETURN` runs with no call open, and no `ACCEPT` within one (proved, not checked).
  The depth is one stack check where each rule function is entered.
  `TOKEN`'s room for the nodes returns write is bounded by the stack's depth
  over the smallest compiled frame.
  Where the proofs fail, the calls are counted in `ctx` as before.
- **The word and flags are callee-saved**: a function whose own words change them
  gives back the values it was entered with at its one exit.
- **The stream's end is a token kind of its own**, so a switch tests it in its compare tree.
- **Switches are compare trees**, as rustc compiles `match`: no jump table.
  As LLVM lowers a switch: a default target, a tree over the other cases with known ranges,
  a test at each leaf, and bit tests for cases of up to three targets within 64 values.
- **A keyword's leaf takes its kind from the token's word** (`WORD_KIND`),
  the keywords' strings numbered in kind order: one add, as `Kind::DotDecl + (d - W_DECL)`.
  Keyword arms that differed only in the kind are one. A keyword's `REMEMBER` does the same.

## Tried and dropped

Each line gives what the machine code showed, and what could change the verdict.

| Approach | What it showed | Worth trying again when |
|---|---|---|
| A jump table on the token's kind | Tier 2 from 146M to 258M cycles: the indirect branch mispredicts | Never as a table; rustc does not make one either |
| Jumps resolved in the lowering, before Cranelift | Larger code (594,132 bytes, from 590,312), 500M instructions from 484M | Cranelift stops folding jump-only blocks itself |
| The state as SSA values through each function | 691M to 908M instructions: each call stores and reloads them, joins spill them | Functions are small enough that joins do not spill |
| Ten state fields held in registers through a function | 706 KB of code, 525M instructions: stack and a return area at calls | Fewer fields live at once |
| Four fields passed to calls and given back | 21,725 stack accesses: they spill between blocks of large functions | Functions are small, or the allocator has no cliff |
| The ahead-of-time Rust's state as locals copied at calls; hot fields local; the pointers behind a read-only `&P` | 406.0M, 370.9M, 351.6M instructions against 316.2M with all of it behind `&mut S`: copies and saved registers at about a call a token | Calls are far rarer than tokens |
| A callee taking its caller's token at `RETURN m=0` | `ctx` kept on the stack in many functions: 1,780 stack accesses, 49,440 instructions | Retried with `ctx` pinned: no spill, kept (`2b46b2e8b`) |
| Shared code as tail calls, `ctx` an argument | Nothing saved: an epilogue (six `ldp`s and an `add`) at each of 587 places | Epilogues get cheaper (fewer callee-saved registers) |
| Shared code as calls, `ctx` an argument | `ctx` reloaded at each use in the largest functions: 5,700 stack loads | Solved by pinning `ctx` |
| The calls open as an argument, with shared code | One value given to every call: split twice, then reloaded at each use (1,148 loads) | The count is needed by fewer calls, or not at all |
| The calls open in the entry index's high bits | The optimizer rebuilt it from the argument, which spilled instead (1,674 loads) | As above |
| The calls open as a passed field, given back with the result | Wrong tapes (no file matched), more stack stores (929) | Never debugged: may be worth fixing |
| A helper function for each set of rules sharing code | 84 functions, not 56, and 1,700 more instructions; three loop values spilled | Helpers are small and called from few places |
| Tail calls into shared code, `ctx` pinned | An epilogue at each of 198 places: 785 restores, against 170 | Epilogues get cheaper |
| The word → kind map over strings in first-use order | A 40-leaf compare tree in the leaf (kinds scattered over the strings) | Never: the strings are numbered in kind order now |
| Words shared across rules: a rule jumps into an earlier rule's code, a call of its function | Functions of up to 21 entries, each call picking its entry by a compare tree; 346.1M instructions | Never as entries: a shared piece is a function of its own, if anything |
| Shared across rules only where the shared code is large (over 8 words) | 94 entries: product's and term's loops, copied by the grammar's `more`, still shared | Superseded: `more` is a call now, and each rule keeps its own code |
| One tail-call block a function gone to, the index its argument | The index constants folded into values live through the function, and spilled | Each place's own block is built; no reason to go back |

## The journal

Numbers are tier 2's for `.rr`. Each step's commit is listed.

| Step | Commit | Machine instructions | Benchmark instructions (cycles) |
|---|---|---|---|
| A function a grammar rule (page 33) | | 148,533 | 484.2M |
| Keywords in the switch | | (530,772 bytes) | 481.3M |
| The code laid out as the lean parser's | `d7f414abb` | 135,201 | 497.5M (105M) |
| Calls as the lean parser's: `ctx` as self, results 0 or 1 | `ca98b7188` | 159,283 | 441.0M (87.6M) |
| The tape and the state as the lean parser's | `bbbb03696` | 100,134 | 351.7M (71.2M) |
| The state as locals | `0d43c099e` | 91,391 | 347.8M (76.5M) |
| A token taken for good | `cc1e0660e` | 87,152 | 339.0M (74.3M) |
| Words as calls | `fded73721` | 43,654 | 354.2M (72.3M) |
| A rule that returns one way gives back 0 | `1a876e2a3` | 42,786 | |
| The calls open as an argument; the end a kind | `43e3f975d` | 41,084 | |
| Callees shared across rules | `8fea9d7b8` | 38,750 | 335.1M (63.9M) |
| `ctx` in a register of its own | `1fa44aff2` | 36,258 | |
| Code shared from the middle of rules; the count back in `ctx` | `de6f313fb` | 31,702 | 361.0M (76.5M) |
| The machine's stack counts the calls | `2b46b2e8b` | 28,962 | 350.4M (74.7M) |
| The word and flags callee-saved | `2b46b2e8b` | 27,433 | |
| One way by the place a call enters | `2b46b2e8b` | 27,314 | |
| The callee takes its caller's token: one `cbnz` a call | `2b46b2e8b` | 26,575 | 331.1M (64.0M) |
| Switches with a default target and known ranges | `73aed4adc` | 26,092 | |
| A keyword's leaf kind from the token's word | `cebf1c631` | 24,828 | |
| Switch cases as bit tests | `cebf1c631` | 24,375 | 330.9M (cycles not taken: load 58) |
| A field on a called choice set where it returns: 171 entries | `fa23bf4e5` | 22,226 | 346.1M (67.4M) |
| Short tails each rule's own; a keyword's `REMEMBER` by its word: 94 entries | `6be997744` | 22,258 | |
| One entry a function: each rule's own code, a term's rest a call: 60 entries | `f5357a7df` | 23,981 | 315.7M (70.7M) |
| Every call at its callee's start (merge, mark, flag on `PUSH`): 55 entries | `9a017c9b9` | 23,758 | 312.6M (61.8M) |
| The same IR through LLVM, ahead of time (`clif_rust`; a measurement, not a step) | | 22,375 | 276.6M (55.3M) |
| Tier 1 too through LLVM; the whole parse against the lean parser, by where it goes (page 34) | | | 276.6M (54.4M) |
| Comments in the token step: 5,271 words to 3,341, 322 comment tests to 40 (page 34) | | 17,460 | 311.3M (60.7M) |
| A mark refused only on its token's bytes: 320 mark tests to 193 (page 34) | | 17,278 | 307.1M (58.4M) |
| Tier 1's checks at the token: a count of tokens (`COUNT`) and local checks (`CHECK`), tier 1's 606 operations to 452 (page 34) | `5da312927` | | 328.7M (57.2M; load 15) |
| Tier 2 ahead of time as structured Rust from the native IR (`rust_stream`, decision 0135; AOT, not the JIT) | `403a5050f` | | AOT 307.0M (54.3M) |
| A local check word-parallel in the JIT: no loop over bits | `72629b5ea` | | 330.4M (58.4M) |
| The tape written directly: parents and extras with each row, wraps in place, no pass at `ACCEPT` (decision 0136) | `8a1762be0` | | 248.3M (41.0M; load 11); AOT 224.8M (36.4M) |
| The lean parser writes the tape's rows too (sizes at its closes): its whole parse 356.5M to 362.4M (69.3M), the baseline from here | | | 248.3M (41.1M) |
| **Correction (2026-11-13, decision 0140).** The two rows above did not write the tape: the engine's tape is the v9np image's bytes, and these wrote a node table (`NodeRow`, `tape::Row`). The lean parser now writes the image's bytes (`rerail_tape_format::Writer`), the engine's own encoder's byte for byte; tier 2 still writes the node table | | | |
| The lean parser's tape writer: code ids at commit points, not renumbered at edits; every column one capacity. Its whole parse 1,326.0M to 550.6M (106.9M); 362.4M was the node table's (decision 0140) | | | 248.6M (42.2M); AOT 224.9M (37.2M) |
| The writer's column stores: the widths a type (`Narrow`), the hot path inlined, one check a push and a close, the commit over local slices. The lean whole parse 550.6M to 425.6M (96.2M); the writer is 49 instructions a node over keys and starts (decision 0140, "Why the column stores were slow") | | | |
| Tier 2 writes the engine's bytes (page 37): a 12-byte working row a node, the columns at their place, codes at commit points, the image's end at `ACCEPT`; the parent and its remaps gone. Cranelift's and ahead-of-time tapes are the lean parser's over the corpus. Lean whole 454.6M (133.3M) in the same run | | | 328.2M (88.1M); AOT 274.1M (73.5M) |
| After the strata merge (format 15, rule-file tests; corpus 1,452 files, 12.25 MB, 1.77M nodes), a node: AOT whole 642 instructions against the lean parser's 366. Tier 1 is 395 against the lean lexer's 109 (five passes: 205 + 454 + 30 instructions a block, 57 masks stored a block and read back, 21 fills, two backward passes; the records' extract alone 223M, more than the whole lean lexer's 193M); tier 2 is 218 against the lean parse and tape's 257; the harness 29. The gap is tier 1's shape (decision 0139, not yet built) | | | 387.3M (74.0M); JIT 464.3M (87.9M) |
| Tier 1 on the CPU in one pass (decision 0139, step 1): the regions walked, a generated function a block called from the body, instead of rounds; `end_on` a token mark; no backward pass, no stored masks. Tier 1 AOT 700.4M to 630.0M (355 a node), JIT 810.1M to 695.8M; tier 2 AOT 394.6M; whole AOT 1,067.5M (602 a node; load 42) | | | 394.6M (79.0M); JIT 473.2M (93.3M) |
| Tier 1's walk results read from the call's memory where used, the call first: peak integer pressure 76 to 66. Tier 1 AOT 626.4M, JIT 693.2M | `9d617767a` | | |
| The records: a word's own marks (9 of 13) from its table slot, no test a token; the record pass 310.2M to 217.0M | `f61c7caf0` | | |
| The records: the other marks set after the tokens, from their rare bits; the record pass 217.0M to 191.9M. Tier 1 JIT 665.2M | `9bb27f34e` | | |
| The directives' refusals as token marks (`~X`, `end`), tested by `TOKEN` once a token, not by each state (decision 0139): tier 2's words 3,932 to 3,515. Tier 1 AOT 487.9M, JIT 527.5M; whole AOT 934.8M (527 a node; load 43) | | | 403.8M (78.9M); JIT 477.3M (93.2M) |
| A number's 19 digits a local check at its token (0133): tier 1 a rare mask, tier 2 the 17 digits between. Tier 1 AOT 478.4M, JIT 526.7M; whole AOT 925.4M, JIT 1,053.8M (the JIT's `CHECK` words cost 6.9M; load 55) | | | 404.4M (71.5M); JIT 484.2M (87.1M) |
| The record pass a function of its own, called once a file: its own registers, the tables' bases and the hash's multiplier as arguments; the long-word flag a meta bit; out of order a sign bit. About 45 instructions a token, from 60. Tier 1 JIT 496.6M, AOT 471.8M; whole AOT 919.7M (load 49) | | | 404.7M (79.4M); JIT 484.7M (94.2M) |
| Tier 1's pass holds no row value through the block loop: live through it 15 to 4, peak integer pressure 61 to 50, spills 299 to 277. Tier 1 JIT 494.7M, AOT 471.8M (the body's own values hold the rest: the floats' rounds most) | `83ec1539f` | | |
| A float's `.` read as simdjson reads one: a `.` between digits, no rounds until settled; `1.2.3` to the engine (an escape). Tier 1 JIT 492.8M, AOT 469.9M; whole AOT 917.0M; spills 277 (`f_dot`'s values in the rounds' place) | | | 404.3M (81.2M); JIT 484.2M (97.4M) |
| A float one token, its number's (simdjson's one scalar): a hidden mark on the token after it, `relabel _ as float`; a float the file ends with to the engine. A field-less relabel keeps a choice one node (`grammar::one_node`): words 3,563 to 3,322 (4,928 without, and tier 2 AOT 372.1M then: open). Tier 1 AOT 465.9M, JIT 501.5M; whole AOT 911.1M, JIT 1,026.0M | | | 402.5M (80.6M); JIT 481.9M (96.1M) |
| Tried, not kept: an `&`/`|` chain lowered by delay (tier 1 JIT 491.5M, AOT 477.6M, from 501.5M and 465.9M; chosen per chain, no better); `two` by its second byte (JIT 505.2M, AOT 469.3M); `uesc`'s escape bytes one word before their carry (JIT 500.2M, AOT 468.0M). The pressure blamed on a statement does not price it | | | |
| 0177 stage 1: end rules (`run`, `pair`), their meaning and the interpreter's lasts built; the CPU's rule in the record pass, the lasts' extract gone (kept: a step of the shape; the ends move into the block loop with the token loop) | | | |
| 0177 stage 2a: the starts by the loop (`skip`'s words after the token before), no starts extract on the CPU; 329 operations from 334 | | | |
| 0177 stage 2c: skip, runs and pair read no `code`: skip 1 op, runs 1 and 9, pair 31 (from 111); body 331, held by the fallback and the marks (stage 3) | | | |
| 0177: the token loop in the block loop: a part after each block over the final blocks, waiting at a token past them; one pass, no per-file record pass for `.rr` | | | |
| 0177: marks from the gap (records lift a mark's bits to the next grammar token), no start masks for marks, no any-mark mask; 285 operations from 331 | | | |
| 0177: `error … by tokens`: a byte no token takes refused by the loop on the CPU (a first byte with no rule), the mask Metal's; 276 operations | | | |
| 0177: the marks' own checks in the gap pass: `newline` `unless after ","`, `float_after` `else escape` at the file's end; `starts_g`, `after_comma` gone; 259 operations | | | |
| 0177: a number's checks at its token (`escape … at number when count(dot)`, `run(digit)`): the token loop's, not an escape mask and a tier 2 `CHECK`; 249 operations | | | |
| 0177: readers: a string, comment or backtick literal read from its opener to its closer's first bit after it (`read K`), not by its region's last-byte mask; open-at-end and four backticks the loop's; 204 operations | | | |
| 0177: a reader says its kind: the comment's kind and extra mark from the reader that read it, not the `ROW` mask (comment rows on a free byte for tier 2's view); 191 operations | | | |
| 0177: pairs from the next byte: `pairs two_start = ":-" …` one list; on the CPU a rule a first byte (`next`), one load of the byte after and a compare a byte, not the pair mask; 162 operations | | | |
| 0177: marks from checks at the token (`at`/`after KIND when …`): `float_after` and `json_bad`; the `\u` checks tier 2's at the string by its kind, no `f_str`; the CPU reads no region, its form calls no walk; 57 operations | | | |
| 0177: the end rule by a jump table (`br_table`, a `match` in the Rust); the pairs one table, all punctuation one rule; 11 rules to 7; tier 1's Rust 260 KB to 250 KB | | | |
| 0177: the token loop in the block loop: no call a block, its state the block loop's values; tier 1's Rust 3 functions (block loop, general body, the file's end), 250 KB to 253 KB | | | |
| 0177: a loop over a block's starts: the starts left (`cand`) and the run words in registers at the loop's head; a token `base + ctz(cand)`, the block loaded once when none is left; no skip load a token | | | |
| 0177: a token's test at its bytes: a run test of byte tests alone read at the token on the CPU (`Records::locals`, `reclocal`), no mask; the `-` and leading-zero tests; 51 operations; tier 1's Rust 247 KB | | | |
| 0177: masks from the bytes where the token loop reads them (`native_cpu_bytes`): starts and runs a window at the refill, readers' closers and counts by window functions, tests by scalar loads, `escaped` by its carry-in; the `locals` field gone; no row stored of an output the record pass makes; CPU form 51 to 8 operations | | | |
| 0177: the marks in the token loop: gap marks from the register words between grammar tokens (cold window loop across blocks), at-marks from the start bit, `unless after` from the word before; the end pass sets only the rest; CPU form 8 to 6 operations; tier 1's Rust 23 functions, 408 KB | | | |
| 0177: no word lookup for a token whose first byte starts no word (`RecordTables::wordy`, a flag a byte from the grammar's words); a branch past the hash, as the lean lexer's `words != 0` | | | |
| 0177: `escaped`'s carry loop a function of its own (`carry_fn`), called from a cold block; the byte-before test inline; tier 1's Rust 25 functions, 427 KB | | | |
| 0177: tier 2's `\u` checks read `uesc` from the bytes on the CPU (its window a call; `rust_stream` through `clif_rust`); `Records::check_masks`; the CPU's body empty (2 constants); uncalled window/carry functions not built; tier 1's Rust 17 functions, 276 KB | | | |
| 0177: no block loop where the body makes nothing (`bodiless_body`): the row to its end at once, the token loop over the file whole; the block loop cut as unreachable, the general body not built; tier 1's Rust 16 functions, 255 KB | | | |
| 0177: a token's one entry load (`RecordTables::token`: kind, rule, word flag, pair row), with its eight bytes; a pair's next byte from them; the `ends` and `wordy` tables gone; tier 1's Rust 254 KB | | | |
| 0177 measured (`jit_against_aot`, profiling build, least of 7, load average 59 on 2026-11-14; corpus 1,453 files, 12.25 MB, 1.77M nodes). Tier 1, a node: AOT 305 instructions, 73 cycles; JIT 447, 157; the lean lexer 109, 32. Tier 1 + 2: AOT 542, 123; JIT 731, 217; the lean whole 366, 89. One pass now (no block loop) | | | AOT tier 1 541.0M (129.1M); JIT 792.1M (277.5M) |
| 0177 profiled: tier 1 ahead of time 268 instructions a token against the lean lexer's 86 (every file, 2.25M tokens). Left out of a scratch compile: the marks 59 a token, the token tests 46, the word lookup 30; the core 96. Main function: 592-byte frame, 192 of 1,660 instructions on the stack, 28% of samples (lean 18.6%); the end rules' `match` a compare chain (`aot_profile_loop`) | | | AOT tier 1 608M (143M), every file |
| 0177: the line marks where tier 2 asks (`lazy_marks`: a gap mark from the bytes, not a check's target, placed, not the extras'): `line` and `newline` out of the token loop on the CPU; tier 2 tests one at its `MARK` from the record before past the extras and a window a word (`WordFn::LazyMark`, AOT `lazy_k`); Metal and the interpreter still set them. Tier 1 AOT 305 to 298 instructions a node, 73 to 65 cycles; tier 1 + 2, 542 to 540; the checks' marks are most of the marks' cost | | | |
| 0177: the token tests by what their masks need of the bytes (`native_cpu_bytes::needs`, an over-approximation: a set at offsets): a first-byte filter bit in the entry for a run test that needs the first byte alone (`-`, leading `0`; one every first byte passes dropped); a count of n skipped under n bytes; a rule's count tests fused into one function (`tests_fn`, one window a word for all, `Bytewise::words`). Tier 1 AOT 298 to 279 instructions a node, JIT 409 to 362; tier 1 + 2 AOT 540 to 521 | | | |
| 0177: a word slot is one 16-byte entry, key then meta (`RecordTables::slots`): one `ldp` after the hash from one base, as the lean lexer's `WORDS`; the main function 1,580 to 1,493 instructions, 224 to 173 stack accesses | | | |
| 0177: a refusal is a branch to a cold block that sets the file's flag, not a value the loop carries; stack accesses 173 to 159; with the slots, tier 1 AOT 279 to 274 instructions a node, tier 1 + 2 521 to 516 | | | |
| 0177: a short token's count tests by one vector compare first: the union of what they need (`Needs::union`; a string's: control bytes and `\` from the byte before), at most four runs of bytes, the token's 16 bytes compared a run; none set, the fused call skipped; `clif_rust` gains vector add and subtract. Tier 1 AOT 274 to 263 instructions a node, tier 1 + 2 516 to 505; the JIT unchanged (362) | | | |
| 0177: the two common rules if-converted (the pair rule and the register run rule most first bytes take, their tests first-byte bits, now set on their rule's bytes alone): both lasts straight-line and a `select`, a run past its word to its own block, the rest by the jump table; the census prints each rule's share of the corpus's tokens (pairs 55.5%, runs 35.3%, strings 7.5%). JIT tier 1 146 to 86 cycles a node (362 to 394 instructions), tier 1 + 2 204 to 144; AOT flat (77 cycles, 263 to 274 instructions) | | | |
| 0177: the generated loop read against the lean lexer: the refill's outputs from one window (`Reads::words_inline`), four loads a block, not sixteen; one run word in a register (the run rule most first bytes take), a number's at its token; the gap marks past a block whose mark words are zero, `unless after` only of the loop's marks; no order check with the loop's starts. Tier 1 AOT 274 to 232 instructions a node, 77 to 67 cycles; JIT 331, 80 | | | |
| 0177: the fused rules' tokens leave `rule_last` by an exit of their own, and the loop's tail is made again for it knowing no reader's tag and no extra (the lean lexer's run loop apart from its rare tokens); fusion only where the extra's mark needs a first byte the two rules have not. Tier 1 AOT 232 to 212 instructions a node, JIT 331 to 285 | | | |
| 0177: a long word's tail compare (seven words, unrolled) a function of its own (`tail_fn`), called on a hit's long flag: out of the loop, as the lean parser's; `clif_rust` writes `sp` only where a function reads it; a guard that every tier 1 function is structured. Tier 1 AOT 213 instructions a node (the path is cold); the JIT 285 to 306: Cranelift's allocator around a call in the loop (open) | | | |
| 0177: the pair rule's second byte from a block's word (`PAIR_WORD`), as the lean lexer's window makes `pair`: the pair table as nibble tables (`pair_nibbles`, a bit a first byte, checked exact against the table), the 16 bytes and the 16 from the byte after `and`ed; a token's pair is a bit test, not a table load; the run from the token's second byte by two shifts, one bound. Tier 1 AOT 213 to 222 instructions and 73 to 65 cycles a node; the JIT 306 to 292 and 79 to 67 | | | |
| 0177: the refill folds the gap marks' bits of each block it passes (no start in it) into the bits so far, and those before the cursor's block (a token across blocks) by the bytes, cold; `from` is then always in the token's block, so a token's gap test is the block's registers alone, no bounds compare and no call a block (the form feed's window, at each block's first token before). Cases of a form feed in a gap across blocks added to `the_marks_tests_are_the_masks` (a fold left out fails it). Tier 1 AOT 222 to 193 instructions and 65 to 55 cycles a node; the JIT 292 to 269 and 67 to 61 | | | |
| 0231: the records' layout a compiler choice (`records::RecordLayout::choose`): rr's token records are rows on the CPU (one base for start, last and code, the lean lexer's `Tok`), columns on Metal; the CPU loop's stores, the batch results, tier 2's Cranelift and AOT reads (`record_stride`) and Metal's reader all go through it. Tier 1 AOT 193→190 instructions a node; tier 1+2 AOT 435→437 (tier 2's stride is loaded, not known) |
| 0231: rows as `base + k*12` gave 3 instructions a node: a `madd` a token, and the record's address spilled across the dispatch. The loop carries a cursor instead (`FileCols::cursor`/`at`/`step`), bumped by 12, an index only at a part's state and the count; `clif_rust` writes `udiv_imm`. Tier 1 AOT 190→186, JIT 280→259. The profile: hot stack ops 27→19, the heaviest stay, the cursor now spilled; the loop carries the marks' and checks' state (refusal, pending, gap start, word before, hits) round every token, which the lean lexer's run loop has not. Next: that state off the common tokens' path |
| 0177: the common tokens' state: the fused rules by bits of an 8-byte entry (`TOKEN_FUSED_*`), no compare of the rule; the refill flags a block with a gap mark's bit in `pending` (`GAP_FLAG`), so a token with no marks so far and no hits skips the merge (cold) and leaves the state untouched; the word before only where an `unless` is the loop's; no extra's mark on the fused path (`never_at`). A mutation (no flag) fails `the_marks_tests_are_the_masks`. Tier 1 AOT 186→171, JIT 259→251. Spills stay (the profile: `from`, the block words, the tables' base reloaded a token) |
| 0177: `from` read from the record behind the cursor (`after_last_start`) where the refill folds, not carried (it was loaded and stored a token); `clif_rust` writes `core::hint::cold_path()` in Cranelift's cold blocks (alone, it changed f0 by 36 lines: the spills were not LLVM's misjudged paths). Hot stack ops 30→22; tier 1 AOT 171→164, JIT 251→252 |
| 0177: the record pass's tables are data of the generated Rust (`TABLES_DATA`, an `adrp` constant, as the lean lexer's statics), not an address stored at load; the token loop's functions make the tables', slots' and multiplier's constants themselves (`FileCols::of_tables`), not take them from the caller's slot. rr_aot checks the program's tables equal the data. Hot stack loads 142→129; tier 1 AOT 164→169 (the addresses rematerialized where used). The profile's top is now load latency (the word slot's pair load, the entry), as the lean lexer's |
| 0177: the side-by-side count. Lean's run loop: ~67 instructions a word, ~45 a punctuation; ours ~84 a word, and its refill (258 a block) matches lean's window (~286); the profiles split alike (token loop ~69%, window 12–19%). The gap was which path tokens take: 55.5% are punctuation, and every one looked a word up (rr's punctuation are words of the table). Now a punctuation's code by first byte and length (`RecordTables::punct`, `TOKEN_PUNCT`), filled by the word table's own lookup; `<` (`<=`, `<:`) keeps the lookup. Tier 1 AOT 169→155, JIT 253→239 |
| 0177: a string read once. The closer's search (a call a word, aligned, each word looking back for the escape's carry) and the count tests' call read the same bytes twice (10.7% of tier 1). Now `read_fn` per reader arm: windows from `s + 1`, the escape's carry passed on (`Bytewise::words_carried`, simdjson's overflow), none after the opener (`Bytewise::apart`); the tests behind a 64-byte gate (`needs_wide`), their function only at a token with a byte they need. Dropped: all test masks in the loop (155→159: short strings had skipped them). Mutations of the gate (always quiet) and of the carry out (always none) each fail a named test. Tier 1 AOT 155→147, JIT 239→222; the reader 460→153 lines |
| 0177: the side-by-side count by kind (`tier1_by_kind`): punctuation 72 vs lean's 54, words 109/98, strings 395/241, numbers 350/107. The punctuation path's stack traffic came from every rule's blocks inline in the loop. Now each rule but the fused two is a function of its own (`rule_fn`, `RuleMode::Calls`/`Only`), called from the loop's dispatch. Tried: all inline 147 a node; one function for all rare rules 150 (JIT 200); a function a rule 149 (kept; JIT 217; punctuation 69, words 107, strings 430, numbers 379); the readers kept inline 150 (punctuation 75). Packing the four returns in two registers: costed, dropped. Also: `READ_CALL` moved off `PAIR_WORD`'s value; `add_decisions` (dead since its caller went) removed |
| 0177: the gate's needs by offset (`needs_by_offset`; a newline before a string's quote no longer opens it); a run not in the registers reads its first 16 bytes by one vector (`short_word`, one-part `bitmask`); punctuation codes in the entry's free bits by width (`punct_in_entry`; first 14+14 tried: rr's codes carry marks, 23 bits). Strings 430→349, numbers 379→256, punctuation 69→66; tier 1 AOT 149→146 a node. Mutations of the run's bound and the gate's offsets fail named tests |
| 0177: the lean lexer's starts. The refill cuts the starts by the file's end (`valid`, once a file), word interiors (`word & word << 1`) and pairs' second bytes (chains read left to right by a carry); `lean_starts` proves it from the bytes' sets, `#` leaves the fused path (`run_fused`). A fused token clears its own bit (`run &= run - 1`); another rule's puts back the byte after it, or refills where a chain crosses its end. Entries are 512, by first byte and pair bit (lean's `TOKEN[c | pair << 8]`): a punctuation's code is one field. Punctuation 66→57 (lean 54), mixed 69→61 (lean 63); strings 349→368, numbers 256→272; tier 1 AOT 146→140 a node, JIT 217→198. Mutations (no put-back; no chain order) fail the records test |
| 0177: the word hash's multiplier kept. rustc made the literal again at each lookup (`mov` + three `movk`). Tried: multipliers of two or three 16-bit parts; none parts rr's words (a short word hashes by its first two bytes). Now `clif_rust` makes a wide constant (`wide`: over two parts, no `orr` immediate) used apart from its block once (`kept`, an empty `asm`): words 103.4→101.6, punctuation 57.0→56.6. `the_word_multiplier_is_kept` checks the source |
| 0177: no guard the zeros make. A number's window tested the file's ends three times (a `within`, the window's cut, the run's bound), ~30 instructions; a batch's files have zeros both sides. `zero_kills` (false where a byte some way back is zero); a `within(d ≤ FRONT)` left out of a conjunction another conjunct is false on (`unguarded`, windows with a position only); `short_word` leaves out its cut where the output is false on zero. Numbers 272→246, comments 591→498. `a_within_goes_where_a_zero_byte_refuses`; an always-true `zero_kills` passes the named tests (rr's guards are all the zeros') |
| 0177: a short gate's span by lane: `needs_some` cuts its hits by one compare of the lanes' indexes with the span, not each half's shifts. Numbers 246→233. A one-lane span fails `a_numbers_checks_are_at_its_token` |
| 0177: a comment's count its opener makes. `json_bad … after comment when count(line_open | block_open) at least 1` always holds; each comment paid a failing 64-byte `/` gate and a block-by-block count call. `Lowering::implies` proves the reader's openers imply the mask; the test gets `least` 0 (`TokenCheck::always`): a constant hit on the CPU, the reader's openers on Metal and in the interpreter (`test_holds`; a `/` division gets none). Comments 498→371 (lean 193). Open: storing the unread mask in the CPU body flips a refusal flag |
| 0177: a comment's tag is its code. The loop looked every `/` token up as a word (`/` is one), comments too; a tagged token now jumps to `coded` with its tag (kind and marks) as its code, no selects, no lookup; the interpreter and Metal give a tagged token no word. Tried first: clearing the word bit when tagged (a select): comments −29 but other kinds +2 from rustc's register allocation. Words 101.6→99.2, punctuation 56.6→55.4, mixed 61.1→59.9, strings 368→360, numbers 233→226, comments 371→338 |
