# The compiled `.rr` grammar against the lean parser: where the shape differs

## Intro

Page 30 wrote a lean Rust parser for `.rr`, as a fast hand-written parser would be.
Page 31 gave it a lexer in Bitlex's own block shape.
Both make the same named-only tape as the engine (page 32).

The compiled grammar should come out in that shape:
Bitlex is to generate a known form, not find one.
This page measures the two side by side and lists where the generated code differs,
each with what it costs and the shape to build instead.

All runs: the 1,360 `.rr` files the grammar parses, 8.8 MB, 1.17 million nodes,
1.54 million tokens. The least of 7 rounds, load 25 to 70.

## The numbers

| | instructions | cycles | IPC | a token |
|---|---|---|---|---|
| tier 1 | 560M | 109M | 5.1 | 363 / 71 |
| tier 2 | 555M | 139M | 4.0 | 360 / 90 |
| **compiled, both** | **1,115M** | **248M** | | 723 / 161 |
| lean lexer | 133M | 33M | 4.1 | 86 / 21 |
| lean parse (whole less lexer) | 224M | 36M | 6.2 | 145 / 23 |
| **lean, both** | **357M** | **69M** | 5.2 | 232 / 45 |

"A token" is instructions / cycles per token.

- **Tier 1 against the lean lexer:** 4.2 times the instructions, 3.3 times the cycles.
- **Tier 2 against the lean parse:** 2.5 times the instructions, 3.9 times the cycles.
- **Both:** 3.1 times the instructions, 3.6 times the cycles.

Where the slots go (`xctrace`, CPU Counters, page 30):

| | useful | processing | delivery | discarded |
|---|---|---|---|---|
| tier 1 | 48% | 41% | 5% | 6% |
| tier 2 | 39% | 39% | 8% | 14% |
| lean lexer | 38% | 31% | 5% | 25% |
| lean, whole | 51% | 17% | 11% | 21% |

Tier 2 waits on the back end for 39% of its slots.
The lean parse, by the difference of the two lean rows, for few.
That is memory: tier 2 keeps its state there (below).

Code size:

| | machine instructions |
|---|---|
| tier 2 | 160,000 (17,089 stack loads and stores, 20,607 jumps) |
| tier 1 | 13,747 (1,363 stack loads and stores), 9 passes |
| lean parser and lexer | about 12,000 |

## Tier 2

Tier 2 runs 452 Cranelift instructions a token, by word:

| word | a token | instructions each | a token |
|---|---|---|---|
| `RETURN` | 0.76 | 219 | 167 |
| `SWITCH` | 1.62 | 54 | 87 |
| `TOKEN` | 1.00 | 61 | 61 |
| `MARK` | 1.68 | 25 | 42 |
| `SPAN` | 0.99 | 20 | 19 |
| `LEAF` | 0.42 | 36 | 15 |
| `PUSH` | 0.76 | 18 | 14 |
| the rest | | | 47 |

### 1. A rule's return goes through a table

`RETURN` pops a place from a stack in memory,
and finds it by a compare tree over the 68 places a return can go back to, 7 deep.
That is 37% of tier 2's instructions.
The lean parser returns with `ret`, which the CPU predicts from its return stack.

**Shape:** each grammar rule a generated function; a rule's use a native call.
Only the token loop is inlined.
This is the standing rule for generated code (calls, not inlining).

### 2. A switch reads the token's bytes again

`SWITCH` loads a byte of the token's text and runs a compare tree
over its runs of bytes: 13.5 runs, about 4 compares deep, 1.6 times a token.
The lean parser switches once on a kind code the lexer gave,
a jump table on one byte.

**Shape:** tier 1 gives each token a kind code
(the lean lexer's one table load: first byte, pair, `<:`)
and a word's code (one 8-byte load, a hashed table).
A switch is a jump table on the code.

### 3. Tier 2 finds its own tokens

`TOKEN`, `SPAN` and `MARK` take 122 instructions a token.
They rebuild each token from tier 1's masks:
the next start bit, the end, the marks set at it.
The lean parser reads one 16-byte record: kind, start, end, word code.

**Shape:** tier 1 writes the token records, as simdjson's stage 1 writes its index.
Tier 2 reads them in order.
A mark becomes a bit of the record, or a look at the next record's kind
(`fix` before a name is the next token's kind, not a mask).

### 4. The state lives in memory

Tier 2's state is 23 slots in memory (`Mem`):
position, end, limit, the nodes made, the open nodes, the returns, the marks live.
Each word loads what it needs and stores what it changes.
The code has 24,728 stores.
Its edge blocks are mostly spills: nine loads and stores to the stack, then a jump.

The lean parser holds its position and its output's end in registers
inside a function, and passes the rest as arguments.

**Shape:** inside a rule's function the state is SSA values.
The tape is written through a pointer that only moves forward.
Only what a call must keep crosses it.

### 5. One function, 160,000 instructions

Tier 2 is one function of 32,904 blocks (and 5 outlined).
That is why the allocator spills at every edge.
The lean parser is 45 functions, about 12,000 instructions in all.

**Shape:** follows from 1: a function a rule, a few hundred instructions each.

### 6. The tape is allocated for the worst case

A pass allocates 3.08 million rows for 1.17 million written (2.6 times)
and zeroes 86 MB of them: 6.8% of a pass.
The lean parser's tape is sized from the token count and grows if it must.

**Shape:** a bound from the token count, not zeroed.

## Tier 1

Tier 1 is 865 operations a 64-byte block, in 9 passes.
The costliest statements (`rr_parser_profile`):

| statement | operations a block |
|---|---|
| the tree section's checks and the outputs | 97 |
| `count depth` | 62 |
| `mask agg_word` | 49 |
| `mark json_bad` | 33 |
| the keyword words `suppress_w`, `input_w`, `output_w`, `policy_w`, `options_w`, `fix_w`, `over_w` | 130 |

### 7. Keywords are found a byte at a time

About 180 of the 865 operations find keywords: a mask a byte for each letter.
They feed the marks (`fix_on`, `policy_on`, `no_value` …).
`policy_on` (page 32) alone added 28M instructions, 5% of tier 1.
The lean lexer finds every word's code once a token,
by one load and a hashed table.

**Shape:** word codes per token (2), and the marks that need a keyword
become tier 2's look at the next token (3).
Tier 1 keeps what only a byte mask can see:
strings, comments, escapes, the bytes' classes.

### 8. Tier 1 spills

The hot loop of its largest pass needs 32 integer and 43 vector registers at its peak.
The core has about 24 and 30. That pass spills 393 times.
Fewer statements a block (7) lowers the pressure.

## What to build, in order

1. Token records from tier 1 (3), with kind and word codes (2).
   It moves work out of tier 1 (7) and out of tier 2 (`TOKEN`, `SPAN`, `MARK`, `SWITCH`).
2. A function a rule, native calls and returns (1, 5),
   with the state as values inside a function (4).
3. The tape's bound (6).

The lean parser is the reference for each:
its lexer is the token records' writer,
its functions the shape of a rule's.

## After the three changes

All three are built:

- **Token records.** Tier 1 writes each token's first and last bytes (two zipped streams)
  and a code: the kind by one table load, the word by a hashed table.
  Tier 2's `TOKEN` reads the record, `SPAN` takes its end, `WORD` is one compare,
  and a switch on the first byte is a switch on the kind.
- **A function a rule.** A call of the grammar is a native call, its return a native return.
  Code that several rules reach is a function of its own, entered by a tail call:
  435 functions, no word lowered twice. The optimizer is on.
- **The tape's bound.** Room for half again as many nodes as tokens, not zeroed.

The same corpus, least of 7 rounds:

| | instructions | cycles | IPC | a token |
|---|---|---|---|---|
| tier 1 | 659M (560M) | 124M (109M) | 5.3 | 428 / 80 |
| tier 2 | 447M (555M) | 88M (139M) | 5.1 | 290 / 57 |
| **compiled, both** | **1,106M** (1,115M) | **212M** (248M) | | 718 / 137 |
| **lean, both** | **357M** | **69M** | 5.2 | 232 / 45 |

In brackets: before.

- **Tier 2 against the lean parse:** 2.0 times the instructions (was 2.5),
  2.4 times the cycles (was 3.9).
- **Tier 1 against the lean lexer:** 5.0 times the instructions (was 4.2),
  3.8 times the cycles (was 3.3). The record pass and the second stream cost 99M.
- **Both:** 3.1 times the instructions, 3.1 times the cycles (was 3.6).
- The tape's allocation: 7.8 ms to 1.6 ms a pass.

Tier 2's words, in Cranelift instructions a token (269 in all, was 452):

| word | a token | instructions each | a token |
|---|---|---|---|
| `SWITCH` | 1.62 | 48 | 78 |
| `TOKEN` | 1.00 | 61 | 61 |
| `MARK` | 1.68 | 25 | 42 |
| `LEAF` | 0.42 | 36 | 15 |
| `PUSH` (the call) | 0.76 | 19 | 15 |
| `CLOSE` | 0.34 | 35 | 12 |
| `RETURN` | 0.76 | 10 | 8 |

Two tries were measured and dropped:

- **A jump table on the kind.** Tier 2 went from 146M to 258M cycles:
  its indirect branch mispredicts. The compare tree stays.
- **The state as SSA values in each function.** At every set of fields, more instructions
  (691M to 908M) and more cycles:
  each call stores the values and loads them back, and the joins spill them.
  The state stays in memory, one register away.

## What to build next

1. **Marks from the records** (3, 7).
   `MARK` still tests tier 1's bit masks: 42 instructions a token.
   Tier 1 still finds keywords a byte at a time, for those marks.
   A mark becomes a bit of the record, or a look at the next record's code.
   Then the keyword masks leave tier 1: about 180 of its 865 operations a block.
2. **A cheaper `TOKEN`.** 61 instructions:
   three column loads, a choice at the stream's end, three room checks,
   and eight fields stored.
   The lean parser reads a record and moves a pointer.
   - An end record, a kind of its own, in place of the choice.
   - The room checked once for each rule's call, not at each token.
   - Fewer fields: a token's start, end and limit are its record's, read where used.
3. **The state across calls in registers.** The token's place and the tape's end
   passed to a rule and given back by it (a call's arguments and results),
   not stored and loaded around each call.
4. **The record pass into the extract pass.** The kind and the word read
   where tier 1 writes the starts, as the lean lexer does, not in a pass of their own.
   For JSON, the second stream costs 124 instructions a block;
   a record with no end for a one-byte token would save it.
5. **Small functions.** 244 of the 435 are one or two words: a tail call on.
   Code: 175,768 machine instructions (15,740 stack loads and stores);
   the lean parser is about 12,000.
   A piece that small is copied into its callers, not called.
6. **Switches on the kind.** 48 instructions each.
   Kinds numbered so a switch's targets are few runs, and the hot arm first.
