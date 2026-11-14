# 0177: Tier 1 on the CPU is a token loop, as the lean lexer is

Status: decided (2026-11-13); stage 1 landed (the rules, their meaning, the CPU's ends from
them; see Stage 1); the token loop runs in the block loop; the starts, ends and the
loop's masks are the lean lexer's (see The shape); the marks and checks at the token next.
Replaces 0139's way on the CPU (its constructs stay; its "take operations out of the one
pass" does not). Metal keeps the bit-parallel form.
The design: docs/parser-generators/36; the log: docs/parser-generators/35.

## Intro

**The problem.** Tier 1 is the compiled parser's lexer: it turns a file's bytes into
records, one a token (its first byte, its last byte, its code). On the CPU it computes
every mask of the grammar at every byte, and only then reads the tokens off them.

Over the corpus (1,461 files, 13.9 MB, 2.56M tokens, 217K blocks of 64 bytes),
measured the same way (`jit_against_aot`, nine rounds):

| | Lean lexer | Tier 1 (JIT) |
|---|---|---|
| A block's masks | 4 (17M, measured on an 8.8 MB corpus) | about 350 operations, about 340M (the split probe) |
| A token's work | about 72 instructions | about 83 (record pass 62, extracts 21) |
| In all | 192.9M | 501.5M (AOT 465.9M) |

A token costs about the same in both. The gap is the block's masks. Taking operations
out of them, one mask at a time (0139's steps, and the tries of 2026-11-13: rounds,
carries, regrouping), moved tier 1 by a few millions each, some of them noise.
The form itself is the cost: **today's tier 1 on the CPU is a dead end.**

**The decision.** On the CPU, tier 1 is the lean lexer's form, generated from the grammar:

- **A few masks a block**: the bytes no token starts at (blanks), the runs a token
  is (a word's bytes), the rare starts, a two-byte punctuation's first byte.
- **A loop over a block's starts**: each token's kind by one table load of its first
  byte, its end by its run, its word by the hash table (as the record pass now).
- **A rare token read at its start**: a string, a comment, a backtick literal, by the
  walk (0139's step 1, a generated function), which jumps the loop past it.
- **The checks at the token**: what a byte, a number, an escape or a mark must be,
  tested where the token is, not at every byte.

Each is a construct of the block program with two lowerings: this token loop on the
CPU, today's masks on Metal (0139's rule: one program, not a second module).

## The shape

The lean lexer has no stages: one loop over a file's blocks, and in it, a block at a time,
its few masks, then the loop over its tokens, each written where it is found, a rare one read
by its own reader, its checks made at it. The CPU's tier 1 is generated to that one shape:

| Part | The lean lexer | The CPU's tier 1 | Built |
|---|---|---|---|
| A block's masks | space, word, rare, pair | `skip`, the runs, the pair (1, 1 and 9, 31 operations) | yes (2c) |
| The token loop | in the block loop | in the block loop, after each block, over the blocks whose outputs are final; its state the loop's values | yes |
| A token's start | the first non-space after the one before | the block's starts in a register, the first left (`records::walk`) | yes (2a, then registers) |
| A token's end | its run, its pair | its first byte's rule (`run`, `pair`, `last`) | yes (1) |
| A rare token | its reader, which moves the cursor | its region's last byte, by the walk's masks | masks for now |
| The checks | at the token | the fallback and the marks, masks at every byte | no |
| What tier 2 reads | the tokens | the records; the marks as bits at a token's start | marks as masks |

What is left is the last three rows: the marks and checks made at the token (a byte no token
takes a `BAD` kind; a mark a token's bit, from the bytes between it and the token before), and
the rare tokens by their readers. With them the CPU makes no mask but the loop's, and the body
is the lean lexer's.

## Governance

- **The meaning is the interpreter's.** Each new construct is defined first in the
  interpreter (`stream.rs`, `block.rs`), with a test that it equals today's masks over
  the corpus sample and over the block's edges, before any lowering.
- **One program, two lowerings.** A construct is lowered for the CPU and for Metal.
  No grammar gets a CPU-only or Metal-only statement.
- **Byte-identical, always.** Each step keeps, with no conversion:
  the corpus's tapes the engine's (`corpus_tapes_are_the_engines`);
  the lean tapes (`corpus_images_are_the_lean_tapes`, 1,452 files taken: a step that
  takes fewer says so and why);
  AOT equal to the JIT (`aot_tapes_are_the_jits`).
- **The contract between the tiers is the records.** Metal against the CPU
  (`tier1_on_metal_equals_cranelift`) compares the records (positions, codes), the
  totals, and the outputs tier 2 reads, no longer every mask word: the CPU will not
  make them.
- **Structural guards, as tests**, each added with its stage and kept:
  stage 1, no `lasts` extract in the CPU's tier 1 and no `last` mask of a run-ended
  token; stage 2, the CPU body's operation count under its budget; stage 3, no
  per-byte check in the CPU body that a token's test replaced. The verifier's
  no-per-byte-loop rule holds: the loops are over a block's tokens and regions.
- **No second path.** What a stage replaces on the CPU is deleted when the stage lands:
  no flag between the old form and the new.
- **Judged by shape.** A step lands when it makes the generated code more like the
  lean lexer's and keeps the tapes byte-identical. A half-built shape's cost is not a
  verdict on it: an intermediate step may cost more until the stage that completes it.
- **Measured one way, at the end.** The whole shape is measured by `jit_against_aot`
  (nine rounds), AOT and JIT both, with the load noted. The split probe moves by about
  8M under load: it diagnoses, it does not decide.
- **Stop rule.** A stage that cannot reach its shape gate stops: the shape is looked at
  again and the user told.
- **Logged.** Each step and each dropped try is a row in docs/parser-generators/35;
  docs/parser-generators/36 holds the design as built; this record its stages' status.

## Stage 1

Built: the syntax (`token K = start S run R`, `pair P`), its mask meaning
(`TokenRule::last`), the records' end rules (`Records::ends`, `rules`), the interpreter's
lasts from them, and the CPU's: the record pass finds each run- or pair-ended token's last
from its first byte, and the CPU makes no `lasts` extract (guard:
`the_cpu_makes_no_lasts_mask`). Over the corpus the rules' lasts are the masks' lasts
(1,473 files, 2,567,918 tokens). The `.rr` grammar ends its words, numbers, `#` words and
punctuation by rules, and its strings, comments and backtick literals by their masks.

In between: the ends are found in the record pass, after the blocks, which reloads the run
mask's word. The lean lexer finds them inside the block loop, from the word mask in a
register (`end = pos + max(run, n1)`); that is where the token loop goes once stages 2 and
3 have shrunk the body (the decision's "loop over a block's starts").

## Stage 2a

Built: the records' `skip` (an output: the bytes between tokens), the interpreter's loop
(`records::walk`: each token at the first byte after the one before that `skip` does not
hold), and the CPU's: the record pass finds each start in `skip`'s words and writes the
starts column, so the CPU makes no starts extract (guard: `the_cpu_makes_no_starts_mask`).
A byte no token begins with has no rule (`NO_RULE`): its token is the byte alone. The loop's
tokens are the masks' over the corpus (1,473 files, 2,567,918 tokens,
`the_loops_tokens_are_the_masks`). The CPU's form: 329 operations, from 334; the start masks
are still made for their other readers (marks, row modes), which 2b takes on.

## Stage 2c, and 2b's readers

Built: `skip` is `ws`, the word runs `word`, the number's run its digits and a float's `.`
between two (`num_run`), and `two` reads no `code`. The CPU's lean-lexer masks are now what
the lean lexer's are: `skip` one operation, the word run one, the number's run 9, the pair 31
(from 111). `second` keeps `code`, so the masks' meaning (Metal's `p_last`) is unchanged
(`token_rules_are_their_last_masks`).

The census (`cpu_outputs_by_operations`) shows what still reads the token start masks and
`code`, and so holds the body at 331 operations: the fallback (the gap check, a byte no token
covers; the escapes and errors), reading 199, 32 of them its own; the marks tier 2 reads
(`newline` through `after_comma` and `starts_g`, `json_bad`, `float_after`), each reading 70 to
250, mostly shared; and the regions' last bytes. Each is a check of stage 3, made at the token:
2b (the start masks go) is done there, not apart.

## The marks

Where the loop finds the starts, no mark is a start mask. A mark that holds between tokens
is its own mask, and the record pass lifts each of its bits to the first grammar token at it
or after (an extra's token passed over by its mark): `Records::gaps`, `records::gap_marks`,
the CPU's late marks, Metal's record kernel. A mark of one word is its other terms at the
token (none: the word alone), a token mark of one first word the word alone. Tier 2 takes
every mark from the codes: the any-mark mask goes (`the_marks_are_not_start_masks`). The CPU's
form: 285 operations, from 331. What still reads `code` and the regions among the marks:
`newline` (its `after_comma`, a fill to the next start), `float_after` and `json_bad`.

## Checks at the token

`error "WHY" by tokens when E`: an error whose bytes are the ones the token loop starts a
token at that no token begins with (`NO_RULE`). Its mask is an output of its own among the
totals (`Records::untaken`): Metal makes it; the CPU makes a zero and its loop refuses the
file at such a byte. The interpreter does both. Over the corpus and stray bytes at the blocks'
edges, the two refuse the same files with the other errors (`the_loops_errors_are_the_masks`,
1,525 files). The `.rr` grammar's `a byte no token takes` is one. CPU form: 276 operations.


## The marks' own checks

Two marks read the token starts only to ask about the token before or after them. Both are
now the gap pass's, as lean's parser asks them of its tokens:

- `mark newline = lf unless after ","`: a line break is none where the grammar token before
  it is a `,`, a list going on over lines. It was `lf & ~after_comma`, `after_comma` a fill
  from a `,` to the next grammar start (`starts_g`). The gap pass knows the token before:
  `Records::unless`, each mark's words.
- `mark float_after = frac_last, else escape "a float the file ends with"`: a bit with no
  grammar token at or after it sends the file to the fallback. It was an escape of its own,
  `last_byte & fill(frac_last, starts_g)`. The gap pass knows what is past the last token:
  `Records::unplaced`.

`starts_g` and `after_comma` go. The interpreter, the CPU's late marks and Metal's record
kernel each do both (`a_line_break_after_a_comma_is_none`, `chained_floats_go_to_the_engine`).
CPU form: 259 operations, from 276; the fallback reads 130, from 189.

## A number's checks at its token

The lean lexer checks a number where it reads it. So does the token loop now:

```text
# a second `.` (`1.2.3`): the engine reads it
escape "a float's `.` after a fraction" at number when count(dot) at least 2
# 19 digits from its first: the engine checks the i64
escape "a number of 19 digits or more" at number when run(digit) at least 19
```

`escape "WHY" at KIND when count(E) at least N` tests each token of the kind over its own
bytes: `E`'s bits in it (`count`), or `E` at each of its first `N` bytes (`run`). A token that
passes it sends the file to the fallback. The tests are the end rule's (`Records::checks`): a
rule with tests is its kind's alone, and the CPU's rule block ends at a block that tests the
token, a popcount over its words or one window compare. Metal's record kernel and the
interpreter test each token the same way.

They replace the escape over `f_dot & prev(frac)` and the 19 digits' tier 2 `CHECK` (one
local check fewer). CPU form: 249 operations, from 259; the fallback reads 85, from 130
(`a_numbers_checks_are_at_its_token`).

## Readers

The lean lexer reads a string, a comment or a backtick literal from its start: to the next
`"` past escapes, to the `\n`, to the `*/`, to the fence. A region's token is now read so:

```text
# the region whose opener holds at the token's first byte,
# then its closer's first byte after it
token string       = start quote & o_str read str
token comment      = start slash & (o_line | o_block) read line | block
token code_literal = start tick & o_tick read t1 | t2 | t3
```

`read K | …` (`TokenRule::Read`) names regions of the `regions` statement. Its meaning, which
Metal and the interpreter's masks keep, is each region's last byte from its fill. On the CPU
it is a reader (`EndRule::Read`, `Records::readers`): the first region whose opener's bit holds
at the token's first byte, then the first bit of its closer after it, a word at a time. The
last byte is the closer's last (`off`), or the byte before an `outside` closer. With no closer
after the opener, what the region says: the file is refused (open at the end), the file's last
byte (a line comment), or the `else` bytes (` `` `, the empty literal). No opener holds: the
byte's other rule (`/` as punctuation), or the file is refused (four backticks).

The token loop never starts inside a region, so the opener's bit at the start is the region's
opening, as the walk's. The errors the readers make are the grammar's: the regions open at the
end and the four backticks are `error … by tokens` and `escape … by tokens`, masks Metal makes
and the CPU does not (`the_loops_errors_are_the_masks`: 1,717 files and edge cases, 152
refused by both). The regions' last-byte masks go from the CPU: 204 operations, from 249.

## A reader says its kind

A comment and a `/` share a first byte, so the token's kind came from the `ROW` mask: a
region mask the CPU made to tell them apart. The comment's extra mark came from a mask too.
The lean lexer's reader knows what it read. Here likewise:

```text
# the comment rows move to a byte no row starts, for tier 2's view;
# the reader that read a comment gives its kind and marks
# a reader: its other rule, then the kind and marks of what it reads
recreader OTHER retag KIND MARKS
# where no reader runs: at a bit of output O, the same kind and marks
recretag O KIND MARKS
```

On the CPU the reader returns its tag with the last byte; the token loop writes that kind
and ORs those marks, and the gap pass skips them. Metal and the interpreter keep the masks
(`Records::retags`, by the moved row's start mask). The `ROW` mask and the extra mark go from
the CPU: 191 operations, from 204.

## Pairs from the next byte

A two-byte spelling (`:-`, `<=`) was a mask, `two_start`: each spelling's first byte, and in
a run of them (`:->`) every other one, by `escaped`. The lean lexer reads the byte after the
token's first instead. Here likewise:

```text
# the spellings, one list; its mask is Metal's and the interpreter's meaning
pairs two_start = ":-" "=>" "!=" "<=" "<:" ">=" "->"
token punct     = start punct & p_start pair two_start
```

`pairs P = …` makes the mask, from classes of one byte each. A token's `pair P` takes the
list too (`TokenRule::Pairs`). On the CPU each first byte gets its own rule, `next` with its
second bytes (`EndRule::Next`, `Records::nexts`): one load of the byte after the start, if
inside the file, and a compare a byte. A run needs nothing more: the loop starts each token
after the one before, so `:->` is `:-` and `>`. The pair mask goes from the CPU.

## Marks from checks at the token

A mark had its bits from a mask: `float_after` from a float's last digit, `json_bad` from
string bytes, comments and numbers, each mask reading the regions. The lean lexer marks what
it reads: a number with a `.` is a float, a string with a bad byte is not JSON. Here likewise:

```text
# a token that passes the test is a bit of the mark:
# `after`, at the byte after its last (the next grammar token has it);
# `at`, at its first (it has it, if a grammar token)
mark float_after = empty, after number when count(dot) at least 1,
                   else escape "a float the file ends with"
mark json_bad    = ff,
                   after string when count(str_bad) at least 1,
                   after string when count(uesc & ~h4) at least 1,
                   after comment when count(line_open | block_open) at least 1,
                   at punct when run(minus & ~next(digit)) at least 1,
                   at number when run(zero & next(digit)) at least 1
```

The tests are the token tests of `escape … at KIND` (`Records::checks`), each with what a
pass does (`CheckThen`: refuse, `At`, `After`). As bits, they keep the gap marks' rules
(`unless after`, `else escape`). On the CPU the token loop carries them: a grammar token takes
its own `At` hits and the bits left before it; an extra's token leaves all of its own to the
next (`pending`, a fourth word of the loop's state). Metal and the interpreter find the same
in their look-back from each grammar token (`record_hits`, `gap_marks`). A mark of tests alone
reads no mask (`Records::tested_only`).

A test `count(A & L) at least 1` with `L` local look-ahead is tier 2's instead: a `CHECK` at
the kind's tokens, at each bit of `A` in the token, as a lexer checks an escape it reads.
A hidden mark only refused checks it in its refusing levels. So the `\u` checks stay tier 2's,
none of the digits' masks tier 1's, and need no region: `uesc` is any escaped `u`, and one in a
comment is no string's.

```text
escape "a \\u escape the engine checks" at string when count(uesc & ~u_ok) at least 1
```

The meaning moves in two places, neither inside a JSON document: a `-` that is a pair's second
byte (`:-`) is no JSON minus, and a form feed in a backtick literal marks the token after it.
`the_marks_tests_are_the_masks` compares the old form's tapes and refusals with the new
(400 files and the cases' edges), and the CPU's and Metal's codes with the interpreter's.

Nothing on the CPU reads the regions now: its form calls no walk (a walk no value of which is
read goes, `cpu_form`). 57 operations, from 162.

## A rule by a jump table; one rule for the pairs

The lean lexer gives a common token's kind and length by one load of a table, and reads a
rare first byte by a `match`. The token loop chose a token's end rule by a compare a rule,
and the pairs had a rule a first byte (eleven rules). Now:

- The rule is a jump table on `Records::ends`' byte (`br_table`). The translation to Rust
  writes it as a `match` (`clif_rust`), which rustc makes a table again.
- A kind's pairs are one table (`PairTable`): a row a first byte, and in the row the set of
  second bytes. All the punctuation is one rule: the row's load by the first byte, then the
  set's word by the next. Seven rules.

Open: `u_ok`'s look-ahead is tier 2's, but `str_bad` and the punctuation and number tests are
tier 1 masks a block; the lean lexer reads them at the token's bytes alone.

## The token loop in the block loop

The lean lexer has one loop: a block's masks, then its tokens. The token loop was a function
of its own, called after each block, its state stored and loaded each time. Now it is in the
block loop's function, once: the full body and the general one both go on to it, and from it
to the next block. Its four state words are the block loop's values, passed round it as its
carries are. Only the file's end (its counts, the marks of any word) is still a call, once a
file. Tier 1's Rust is three functions, from four.

## A loop over a block's starts

The lean lexer takes a block's starts as one mask and each token as its first bit left
(`trailing_zeros`); a token's run is the word mask's ones in a register. The token loop
searched the skip mask from the token before, a load a token, and each run rule loaded its
word. Now the loop's head holds, in registers, as the lean lexer's window does:

```text
cand   the block's starts left: not skip, from the cursor
runs   each run rule's word of the block (the word's, the number's)
```

A token is `base + ctz(cand)`; after it, `cand` keeps its bits past the token's end while
the end is in the block, and is zero when it is not. With `cand` zero the loop loads the
cursor's block once (its skip word and run words), a word at a time past a block with no
start, as far as the blocks whose outputs are final. A run rule reads its word from the
register, but for a token at the block's last byte, whose run starts in the next block (a
cold block, as the lean lexer's word on past the block).

## A token's test at its bytes

The lean lexer tests a `-` or a leading `0` by the bytes at the token: `d == b'-'`, the next
byte a digit or not. The token loop loaded the test's mask word, and the block made the mask
at every byte. Now a run test of a few bytes (eight at most) whose mask is byte tests alone
(`local_ops`: classes, `next`, `prev`) is the CPU's at the token's bytes:

```text
# the records' row: the test's output, its delay, its byte tests (a LocalCheck's ops)
block 0 reclocal m30 d1 b...  p0.1 ...
```

The CPU reads each byte once at the token (`local_at`, straight-line, 0 outside the file),
and its form makes no mask of the test where nothing else of the records reads it
(`read_at_tokens`; guard `the_cpu_reads_local_tests_at_the_token`). Metal and the interpreter
read the mask, which means the same. The punctuation's and the number's tests: 51 operations,
from 57; tier 1's Rust 247 KB, from 254.

Left as masks: a test that counts over the token (a float's `.`, a string's bad bytes), and
the readers' openers (a string's `"` is `quote & ~escaped`, a carried value).

