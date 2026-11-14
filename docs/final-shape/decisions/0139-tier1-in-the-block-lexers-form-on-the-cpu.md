# 0139: Tier 1 on the CPU in the block lexer's form

Status: decided (2026-11-13); step 1 built, step 2 in part and step 3's marks built
(2026-11-13). Its way on the CPU replaced by 0177 (a token loop); its constructs stay.
The design: docs/parser-generators/36.

## Intro

**The problem.** Tier 1, the compiled parser's lexer, makes every check at every byte:
about 2,500 instructions a 64-byte block, 464.6M over the corpus ahead of time.
The lean lexer, in the same block shape, makes four masks a block and then works a token
at a time: 132.4M.

**The decision.** On the CPU, tier 1 takes the lean lexer's form; Metal keeps the
bit-parallel form. CPU and GPU diverge as little as they can: each new piece is a
construct of the block program with two lowerings, not a second module.

1. **Regions** (strings, comments, backtick literals) are a construct of their own,
   `regions { … }`: an opener, a closer, what the file's end gives.
   On the CPU, a walk over a block's openers, each to the first closer after it
   (a trailing-zero count); on Metal, today's `rounds until settled`.
   `tt_later`, its `back_fill` and the backward pass go: tier 1 on the CPU is one pass.
2. **Checks a token makes** move to the token: keywords as token marks
   (a change to the `.rr` grammar), a number's length and first bytes,
   a byte no token takes as a token kind.
3. **The records from the starts alone**: one walk over a block's starts,
   each token's end from its run; no `lasts` extract.
4. **Primitives**: two-byte punctuation as one nibble product.

**Loops.** Page 12 kept the mask passes free of loops. On the CPU, tier 1 may loop over a
block's regions and its tokens, never over its bytes. Accepted where it is needed.

Target: tier 1 on the CPU within 1.5 times the lean lexer, about 200M instructions.

## Step 1, built

- **The construct.** `regions H { K = O .. C [bytes N] [outside] [else E bytes M] }`
  in Bitlex is a rounds group that is also a walk (`block::Walk`).
  The interpreter's meaning is the walk; a test checks it is the rounds' fixed point
  on every value of the `.rr` grammar, over a corpus sample and over closers that cross
  a block's end (`the_walks_are_the_rounds`).
- **On the CPU** (`native_cpu_walk`), each walk is a generated function, called once a
  block. It loops over the block's regions, each found by a trailing-zero count.
  A region open at the block's end, and closer bytes that run past it, are carried in
  one state field.
  An `else` asks whether the closer comes later in the file. Past the block, the walk
  scans the bytes for it, eight a word, by the closer's byte tests. A scan that finds
  none sets a flag, so later openers of that kind take the `else` with no scan.
  So each byte is scanned at most once.
- **On Metal**, the same program runs the rounds, as before.
- **`end_on`** was a keyword seen past blanks and comments (`back_fill`): it is now the
  token mark `tokens "." "end"` (a step 2 change, made here because it held the last
  backward pass).

Tier 1 on the CPU is one forward pass, down from five, with no masks stored between passes.
Over the corpus ahead of time it went from 700.4M instructions to 630.0M (395 to 355 a node).
The cut is smaller than the passes' share suggested:
most of pass 0's masks were needed by the later pass, and are still made there.
Steps 2 to 4 take operations out of that one pass.

## The records' marks, built (step 3, in part)

- A mark of one word (9 of the grammar's 13) is read from the word's table slot: its bits
  are in the slot's meta, set with the token's code. No test a token.
- The other marks, of any word, are set after the tokens, from their rare bits.
- The record pass over the corpus went from 310.2M instructions to 191.9M.
  The `lasts` extract is left.

## Step 2, in part: the directives' refusals are token marks

Two escapes refused a file by the token after a directive. Each was a chain of masks at
every byte: blanks and comments skipped by fills, and the directive's spelling.

- **The grammar.** They are token marks now, with two new forms of a token mark's element:
  `~X` (a token not `X`) and `end` (no token left).

  ```
  # `@`, a name, then a token that can not follow a functor named, not called
  mark fn_uncalled = tokens "@" identifier ~("(" | "," | "}")
  # `.input` or `.output`, then an `#include` or the file's end
  mark io_bare     = tokens "." "input" | "output" hash_word | end
  ```

- **Where they are tested.** A token mark the grammar takes nowhere refuses the file
  wherever it holds. As a mark, it was tested in every state, two words a state:
  tier 2 grew by 610 words.
  So `TOKEN` tests these marks instead, once a token, from the record's bit; no state
  tests them. The program marks them (`tokenmark K refuses`). `no_value` is one too.
- Tier 2's words went from 3,932 to 3,515.
- Tier 1, ahead of time, went from 626.4M instructions to 487.9M; through the JIT, from
  665.2M to 527.5M. The figures before include the records' marks. Tier 2 rose from
  394.6M to 403.8M: the one test a token.

## Step 2: a number's 19 digits, at its token

The engine checks an integer of 19 digits or more itself. Tier 1 marked one by a chain
of masks at every byte, about 16 operations a block. It is now decision 0133's local check.

```
mask int_first  = digit & int & ~prev(int)
mask int_long   = int_first & next(digit, 18)   # tier 1: a digit 18 bytes on
mask digits17   = next(digit) & … & next(digit, 17)
escape "a number of 19 digits or more" when int_long & digits17   # tier 2, at the token
```

- The mask must be rare. Written as `int_first & digits18`, the mask was set at every
  number, so every number token called the check: tier 2 through the JIT rose by 92M.
- The check sends a number to the engine with leading zeros, or with a fraction after its
  19 digits, where the old mask did not. The tape is the engine's either way.
- Tier 1 ahead of time went from 487.9M to 478.4M; the whole parse ahead of time from
  934.8M to 925.4M. Through the JIT, tier 2 rose from 477.3M to 484.2M and the whole parse
  from 1,047.7M to 1,053.8M. The JIT's `CHECK` word costs more a token than the Rust one:
  open.

## The record pass's shape (step 3, in part)

The record pass ran inside tier 1's one function, after a file's blocks. It shared that
function's registers: its token loop reloaded values from the stack, and Cranelift remade
its constants (the hash's multiplier, a table's offset) at every token. About 60 machine
instructions a token.

- **A function of its own**, which the pass calls once a file, so it keeps the cache's
  positions and gets its own registers. The tables' bases and the multiplier are its
  arguments, not constants.
- **A slot's long-word flag** is the meta's top bit: one shift and branch, not a compare,
  an AND and a test.
- **A token's length mask** is all ones shifted by `-8 * min(len, 8)`, mod 64.
- **Out of order** (a last before its start, a start inside the token before) is a
  difference below zero: the differences ORed, their sign bit tested once a file.

About 45 instructions a token. With the masked marks' masks, the record pass as the split
measures it went from 194M to 161M (JIT). Tier 1 through the JIT went from 526.7M to
496.6M; ahead of time, from 478.4M to 471.8M (LLVM had already kept the constants in
registers).

The ahead-of-time translation reads a constant inside the tables as their static and an
offset (`clif_rust::translate`'s spans).

## The block loop's registers

The pass's arguments, the row's index and the file's places were live through the block
loop, which reads none of them: 15 integer values through the hot loop. They are now in a
stack slot, read before and after a file's blocks. Values live through the loop 15 to 4,
peak integer pressure 61 to 50, spills 299 to 277; tier 1 through the JIT 496.6M to 494.7M.

Keeping the carried state words in a slot as well, read where used, made it worse (560.0M
to 572.3M, 355 spills): there are only two, each read often.

What holds the registers now is the body's own values, by statement: the floats' rounds
until settled (6,119 points over the registers), `uesc` (3,692), the region walk (3,406),
`two` (1,747).


## Floats read as simdjson reads them

simdjson marks no float in its first stage. Its second stage reads a number
straight through, at its start: digits, a `.`, digits, an exponent. It refuses
a number with a byte after it that is not whitespace or structural.

Tier 1 now does the same for a float's `.`. It is a `.` between digits, with
no fixed point over the ones before it:

```
# was: rounds until settled, a `.` after digits that are no fraction
mask f_dot = code & dot & prev(num) & next(digit)
mask frac  = fill(prev(f_dot), ~digit)
# `1.2.3`: the engine reads it (no file in the corpus has one)
escape "a float's `.` after a fraction" when f_dot & prev(frac)
```

The rounds were cheap in instructions: one round settles nearly every block.
Tier 1 through the JIT went 494.7M to 492.8M, ahead of time 471.8M to 469.9M.
Spills stayed at 277: `f_dot`'s values now hold the registers in the rounds'
place (4,189 points over them), live from the classes to the tokens that read
them.

## A float one token

simdjson's number is one scalar, whatever is in it. A float is now one token
too, its number's: the `fraction` token is gone. What makes it a float is a
mark on the token after it, from the float's last digit. Tier 2 makes the
leaf it just made a float there:

```
token number = start digit & int & ~prev(int) last numeral & ~next(numeral)
mark float_after = frac_last          # hidden: skipped where no arm reads it
choice numeric = number [relabel _ as float float_after]
# no token after it to mark: the engine reads it
escape "a float the file ends with" when last_byte & fill(frac_last, starts_g)
```

Its mark is the first one, as tier 2 reads it right after the number, before
a line's marks on the same token. It is hidden: inside JSON or a raw line the
number is merged into a leaf of another kind, and the mark is skipped.

The grammar compiler had refused this shape: a `relabel` with no field broke
its count of the nodes a choice leaves (`grammar::one_node`). Every rule with
a field around a `numeric` became a call, and tier 2 grew to 4,928 words. A
relabel to a kind alone (not `drop`) now keeps the count: 3,322 words, from
3,563.

Ahead of time the whole parse went 917.0M to 911.1M (tier 1 469.9M to
465.9M, tier 2 404.3M to 402.5M). Through the JIT it went 1,020.3M to 1,026.0M:
tier 1 492.8M to 501.5M, from the mark's mask. The 4,928-word form ran tier 2
in 372.1M ahead of time; why the calls were cheaper is open.

## Carries: tried, not kept

`uesc` and `two` held the registers by their carries: each `prev` of a word
another block made is a word held through the loop. Three ways to make fewer,
measured ahead of time and through the JIT (tier 1, from 465.9M AOT and
501.5M JIT):

- **An `&` or `|` chain lowered from its earliest delay to its latest**, so a
  delay's terms are one word before a `prev` brings it later: JIT 491.5M, AOT
  477.6M. The program grew (356 to 360 operations, 79 to 81 `prev`s): a
  grouped word replaces `prev`s other masks had shared. Chosen per chain, by
  the `prev`s and operations it adds, it was no better through the JIT.
- **`two` by its second byte**, a class of first bytes each (four carries,
  not six): 24 operations, from 30; JIT 505.2M, AOT 469.3M (with the next).
  Its two new classes cost more than the carries saved.
- **`uesc`'s and `json_bad`'s escape bytes one word** (`escaped & u`,
  `escaped & ~jesc`) before their carry to the string's delay: JIT 500.2M,
  AOT 468.0M. Noise.

So the points over the registers that the profile blames on a statement do
not price it. The diffs are in the worktree's `tmp/` (`fold-by-delay.diff`,
`fold-by-delay-chosen.diff`, `uesc-carries.diff`).
