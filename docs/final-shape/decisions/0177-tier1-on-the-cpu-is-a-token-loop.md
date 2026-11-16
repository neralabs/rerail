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
| A rare token | its reader, which moves the cursor | its reader, its closer found in the bytes from its opener | yes |
| The checks | at the token | at the token, from its bytes | yes |
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

## Masks from the bytes, where the loop reads them

The lean lexer makes four masks a window, in registers, and stores none; it reads a string
from its opener to its closer, and a test from the token's bytes. The CPU's block body made
every mask the token loop reads (31 outputs, 51 operations) and stored them all, a row a
block, for the loop to load again.

Now the compiler finds, from the program alone, which outputs are made from the bytes near
each byte (`native_cpu_bytes`): a byte-local value (`native_cpu_lanes`: tests, classes,
`prev`, `within`), their logic and `prev`s, and `escaped(a)` of one. The token loop makes each
where it reads it:

```text
the block's starts and runs   a window at the block, on the byte vectors, in the loop
a reader's closer, a count    the window's function (`bytes_fn`), a call a word read
a test at the token           its bytes, scalar loads
escaped(a)                    simdjson's formula; its carry in, the byte before tested,
                              and only in a run of `a` a cold loop back to its start
```

The windows are the block body's own lowering (`Lanes::window`), at any position. The CPU's
form makes none of these outputs (`read_at_tokens`) and stores no row of what it does not make
(`Loop::stored`). A read from the bytes waits on no block, so the loop's bounds are only those
of the rows it still reads. Metal and the interpreter make the masks, which mean the same.
No grammar hint: the `.rr` grammar is unchanged.

The CPU's form: 8 operations, from 51; what is left is the marks' (the newlines' and the
directives'), next. The guard `the_cpu_reads_local_tests_at_the_token`; the edges (runs of
backslashes across windows, strings and comments over blocks) in
`the_cpus_records_are_the_interpreters`.

## The marks in the token loop

The lean lexer sets a newline mark where it passes the newline, between two tokens. The CPU's
end pass found each mark bit after the loop, a binary search over the tokens a bit.

Now the token loop sets the marks whose masks are made from the bytes (`loop_marks`). Its
state carries two more words: the byte from which the gap is not yet counted, and the word of
the grammar token before (for `unless after ","`):

```text
a gap mark (newline, line)   its bits between the grammar token before and this one,
                             from the block's words in registers; a cold loop of
                             windows when the gap crosses blocks
a mark at the token          the bit at its first byte, from the register word
a mark never placed          refused at the file's end, as the interpreter
```

The end pass sets only the others (a reader's, a word's, of tests alone). The CPU's form makes
none of the loop's marks: 6 operations, from 8. Same tests as above, and the AOT tapes.

A token whose first byte starts no word looks up none: the record tables carry a flag a byte
(`RecordTables::wordy`), made from the grammar's words, and the loop branches past the hash,
as the lean lexer's `words != 0`. Punctuation, numbers and strings take no hash.

`escaped(a)`'s carry past a window's first byte (a run of backslashes before it) is a
function of its own a run (`carry_fn`); the loops that read it call it from a cold block, and
keep only the test of the byte before inline.

## Tier 2's checks read the bytes

The last mask the CPU's body made was `uesc`, an escaped `u`, for the two `\u` checks tier 2
runs at a string (decision 0133: the digits' tests at the token, none of them tier 1's). It
was 4 operations a block, and a row stored for tier 2 to load.

Tier 2 on the CPU now reads a check's mask from the bytes where they make it: the check's
function calls the mask's window (`Bytewise::word`), at each token of its kind. The
ahead-of-time tier 2 (`rust_stream`) does the same: the window's Cranelift function goes
through `clif_rust`, a module of the source (`window_N::mask`). 0133 holds: the digits are
tested at the token, never in a block. Metal and the interpreter make `uesc` a mask, as
before; the meaning is one.

```text
records' check_masks   the masks tier 2's checks alone read (text row `recchecked`)
the CPU's form         makes none of them: nothing left but constants
```

A window or carry function no code calls is not built (it was: 10 of tier 1's 26 functions
in the Rust). Tier 1's Rust: 17 functions, 276 KB, from 423 KB. The guard
`the_cpu_reads_local_tests_at_the_token` holds the body empty; `rr_checks_its_unicode_escapes_at_the_string`
the checks' cases, on the JIT and (`aot_tapes_are_the_jits`) ahead of time.

## No block loop

With the body empty, the token loop never waits on a block: each of its reads is from the
bytes. The pass then has no block loop (`bodiless_body`): a file goes from its row to its
end at once, which runs the token loop over it whole, then the end's function. The block
loop's blocks are unreachable and cut (Cranelift's `eliminate_unreachable_code`), and the
general body, which nothing calls, is not built. Tier 1 on the CPU is one loop over a file's
windows and tokens, as the lean lexer is.

Tier 1's Rust: 16 functions, 255 KB. Its main function is the token loop: the refill's
window inline (its 125 vector operations), each token's rule, reader, word and marks; the
rest are calls (windows, readers, the carry, the end).

## A token's one load

The lean lexer reads a token's kind and length with one load (`TOKEN`, by its first byte).
The loop read three tables by the first byte: the end rule, the kind, the word flag; a pair
also loaded its next byte and its row. Now each first byte has one entry, four bytes
(`RecordTables::token`):

```text
kind | rule << 8 | a word starts with it << 16 | its pair row << 24
```

A token loads its first eight bytes and its entry. The rule's jump, the kind, whether to
look a word up and a pair's row are fields of the entry; a pair's next byte is the second
of the eight. A pair loads its set's word, one load more.

## Where the loop's instructions go (profiled 2026-11-14)

Tier 1 ahead of time against the lean lexer, over every corpus file (2.25M tokens): 608M
instructions, about 268 a token, against the lean lexer's 86. Measured by leaving one part of
the loop out of a scratch compile at a time (its output wrong, its cost the difference), and
by xctrace's CPU Profiler on the main function's machine code (`aot_profile_loop`).

```text
part left out          instructions   a token
none                   608M           268
the marks              475M           -59
the token tests        504M           -46
the word lookup        539M           -30
all three, and stores  218M            96   (the core: refills, starts, end rules)
```

The core is about the lean lexer's whole. The gap is three things the lean lexer does not do
a token, and the registers:

- **The marks.** The loop sets the line marks at every grammar token: the mark words in the
  refill's registers, the gap's bits, `unless after`. The lean lexer sets none: its parser
  asks whether a token is on a line only in the statements that ask it, by the bytes since
  it last asked (`on_line`).
- **The token tests.** A count test, or a run test past eight bytes, calls its mask's window
  function for each word of the token: a full 64-byte classification a test a word. A rule's
  tests each classify again. The lean lexer tests a number's digits as it reads them.
- **The word lookup.** A slot's key and its meta are two tables, two loads and a branch for
  a long word. The lean lexer's slot is one 16-byte entry (key, length, code).
- **The registers.** The main function's frame is 592 bytes; 192 of its 1,660 instructions
  touch the stack, and 28% of its samples land on them (the lean lexer: 18.6%). The loop's
  state, its column pointers and table bases are live across the calls in its body, more
  than the callee-saved registers hold. The end rules' `match` is a chain of compares.


## The line marks, where tier 2 asks

The lean parser does not mark lines as it lexes. A statement that ends at a line break asks
whether the next token is on a new line, and only then (`on_line`). The `.rr` grammar's
`line` and `newline` marks are asked the same way: only in a directive's or a declaration's
tail.

So the compiler finds the gap marks tier 2 can test itself (`lazy_marks`). A mark qualifies
when it is:

- a gap mark, its mask made from the bytes;
- no check's `at` or `after` target, and not of tests alone;
- placed, and not the extras' own.

For `.rr` these are `line` and `newline`. On the CPU, tier 1 sets neither: they are out of
the token loop, its registers and the end pass. Tier 2 tests one where a state asks:

```text
MARK line       at the stream's end: the end's, as before
                else lazy_mark: the record before, past the extras (comments);
                its word an `unless after` one: none;
                else any bit of the mask from the byte after its first to this
                token's first, a window function a 64-byte word
```

That is `records::gap_marks`'s meaning, read at one token, not computed at all of them.
The Cranelift tier 2 calls an outlined function (`WordFn::LazyMark`). The ahead-of-time Rust
has a method per mark (`lazy_k`), its window through `clif_rust`.

Metal and the interpreter still set the bits in the codes; the CPU's codes are theirs
without them. Tests: the CPU's records, the marks' tests and Metal against Cranelift compare
the codes with those bits masked out. The corpus tapes, the lean tapes and the AOT tapes are
unchanged.

Measured (`jit_against_aot`, load average 120): tier 1 ahead of time 298 instructions and
65 cycles a node, from 305 and 73; tier 1 + 2, 540 and 106, from 542 and 123. Most of the
marks' 59 a token stays: it is not the line marks' gap. It is the checks' marks (`json_bad`'s
`after string when …`, `float_after`): their hits at a token, the bits a token leaves the
next, the extras' select. Those go with the token tests, next.

## The token tests, by what they need of the bytes

The lean lexer tests a `-` only at a `-`, and counts a number's dots as it reads the digits.
The loop tested every punctuation token for `-` (56% of `.rr`'s tokens). Each count test
called its mask's window function, a full 64-byte classification, once per test per word.

The compiler now asks what each test's mask needs of the bytes (`native_cpu_bytes::needs`).
That is an over-approximation from the instructions: a bit at `p` only where a byte from
`p + lo` to `p + hi` is in a set.

```text
byte test, class, pair      its bytes, at 0 (a pair: the cheaper of its two)
a & b                       the cheaper of the two (either is sound)
a & ~b                      a's
a | b                       the union, the offsets' span
prev(a, k), escaped(a)      a's, k (or 1) bytes back
not, carried, position      none: anything
```

Three tricks follow from it:

- **A first-byte filter.** A run test whose mask needs only the token's first byte gets a
  bit in that byte's entry (`TOKEN_TESTS`, bits 17 to 23), set for the bytes it can hold at.
  The loop skips the test on one bit of the load it already has. For `.rr`: the `-` not
  before a digit, the leading `0`. A filter that every first byte of its rule passes (a
  number's digits) decides nothing, and is dropped.
- **A length bound.** A count of `n` needs `n` bytes: a token shorter skips it (a second `.`
  in a one-digit number).
- **Fusion.** A rule's count tests are one function (`tests_fn`, called from the loop): one
  loop over the token's words, each word's masks from one window (`Bytewise::words`, its
  vectors and its `escaped` shared), a popcount per test. A string's two tests classify its
  bytes once.

Guard: `a_tests_first_byte_filters_it` names the filtered tests (`-`, `0`).

Measured (`jit_against_aot`, load average 119): tier 1 ahead of time 279 instructions a
node, from 298; the JIT 362, from 409; tier 1 + 2 ahead of time 521, from 540.

## A word's slot is one entry

The lean lexer's word table is one array of 16-byte entries: the key, the length, the code.
The loop had two tables, keys and metas, so two bases and two dependent loads after the
hash, each from its own register.

Now a slot is the key and its meta side by side (`RecordTables::slots`). The lookup is the
lean lexer's:

```text
mul   x12, x12, x14          the hash
lsr   x12, x12, #55          its slot
add   x12, x13, x12, lsl #4  16 bytes a slot
ldp   x13, x12, [x12]        key and meta, one load
cmp / ccmp                   length, then key
```

The main function: 1,493 instructions, from 1,580; 173 stack accesses, from 224 (one base
less live across the loop). Metal reads the word table itself and is unchanged.

## A refusal is a branch

The loop carried its refusal as a value: each token's order differences and its tests'
refusal ORed in, the sign bit read at the file's end. That is a register live across the
whole loop, for an event most files never have. The lean lexer returns its error at once.

Now a refusal is a branch to a cold block that sets the file's flag in memory (the count
after the extracts', which the file's end ORs into); the loop carries nothing for it. The
main function: 159 stack accesses, from 173. Measured with the slots (`jit_against_aot`,
load average 88): tier 1 ahead of time 274 instructions a node, from 279; tier 1 + 2, 516.

## A short token's tests by one vector compare

A string's two tests (a bad byte, a bad `\u`) need a control byte or a backslash: the union
of what their masks need (`Needs::union`) is `0x00..=0x1f` and `\`, from the byte before the
token (an `escaped`'s run) to its last. Most strings have neither.

So where a rule's count tests need a set of at most four runs of bytes, within eight bytes
of the token, the loop first tests them on the token's bytes by one 16-byte vector: a
compare a run (`v - a < b - a + 1`), the lanes of the span kept, both halves ORed. None set:
no test can hold, and the fused call is skipped. A token too long for one vector calls as
before. The bytes read stay in the buffer: 16 zero bytes before each file (`FRONT`), its
last block padded, 64 after the last file; a zero past the file is a control byte, so at
worst the call is made.

Guard: `a_strings_tests_need_a_control_byte_or_a_backslash`. Measured (`jit_against_aot`,
load average 53): tier 1 ahead of time 263 instructions a node, from 274; tier 1 + 2, 505
from 516. The JIT's is unchanged (362): Cranelift's vector code for the compare costs about
the call it saves.

## The two common rules, if-converted

Most tokens are of two rules (`.rr`, by the census over the corpus: a pair's, punctuation,
55.5%; a run's, words, 35.3%; strings 7.5%, the rest under 2%). Tokens of the two alternate,
so a branch between them is mispredicted often.

The compiler if-converts them. It takes the pair rule and the run rule (one whose word is in
a register) that the most first bytes take, where their tests are all first-byte bits. Those
bits are now set on their rule's bytes alone, so a test can run on a path two rules share.
Then:

```text
either = rule == pair || rule == run      one branch, mostly taken
l_pair = s + the pair's next-byte bit      straight-line (`next_last`)
l_run  = s + ctz(~(word >> (s + 1)))       the run in the block's word
slow   = rule == run && the run leaves the word     to the run's own block
l      = select(rule == pair, l_pair, l_run)
the two rules' tests, by their bits
```

Every other rule goes through the jump table as before.

Measured (`jit_against_aot`, load average 59): the JIT's tier 1, 86 cycles a node from 146
(394 instructions from 362, the mispredicted dispatch gone); its tier 1 + 2, 144 cycles from
204. Ahead of time, flat: 77 cycles, 274 instructions from 263 (rustc had made the dispatch
a chain of compares, as cheap as the straight-line work it replaces).

## The generated loop against the lean lexer, side by side

Read side by side, the generated Rust and the lean lexer differed most in four places:

- **The refill classified the block four times.** Each output the loop keeps in a register
  (the starts, each run, a mark) made its own window: 16 vector loads, the class lookups
  again, four bitmask reductions. The lean lexer's `Window::at` loads once and makes its
  masks from that. Now the refill makes them all from one window (`Reads::words_inline`,
  `Bytewise::words`): four loads a block.
- **A rare token's run was in every refill.** A number's run (`num_run`, which reads ahead
  a byte, so a window of its own) was made for every block, for 0.5% of the tokens. The lean
  lexer reads a number at its start. Now one run word is in a register, the run rule's that
  the most first bytes take (a word's); a rarer run reads its word at its token, by a call.
- **The gap marks ran at every token.** A form feed's mark (`json_bad`) is a gap mark with
  token tests, so not lazy; its range and `unless after` ran at every token. Now a block
  whose mark words are zero (nearly all) takes one branch past them, and `unless after` is
  only of the marks the loop sets.
- **Every token checked its order.** With the loop's starts no rule's last is before its
  start, and no start is inside the token before (the starts below it are cleared): only a
  rule's own refusal is left.

Measured (`jit_against_aot`, load average 141): tier 1 ahead of time 232 instructions and 67
cycles a node, from 274 and 77; the JIT 331 and 80; tier 1 + 2 ahead of time 474 and 116.

## The common tokens' tail apart

The lean lexer's run loop writes a word or a punctuation token and nothing else; strings and
comments are read apart. The generated loop sent every token through one tail: a reader's
retag selects, an extra's selects (the gap state kept, the marks left to the next token),
for tokens that can be neither.

Now the fused rules (`rule_last`'s pair and run) leave by an exit of their own, and the loop's
tail is made again for it, knowing a token there is no reader's and no extra: none of those
selects. Every other rule keeps the general tail. Sound only where the extra's mark needs a
first byte neither fused rule has (a comment's `/` is its reader's).

Measured (`jit_against_aot`): tier 1 ahead of time 212 instructions and 66 cycles a node,
from 232; the JIT 285 and 75, from 331; tier 1 + 2 ahead of time 454.

## A long word's compare is a call

A word past eight bytes needs its other words compared with its table row. The loop did it
inline: seven loads and compares, unrolled, in the loop's function, though almost no token
takes it. The lean lexer keeps such work out of its loop.

Now the compare is a function of its own (`tail_fn`), called where a hit's long flag is set.
The loop's function holds only the call. Tier 1's Rust also checks, at compile time, that
each of its functions is structured control, not the fallback loop over a `match`.

Measured (`jit_against_aot`): tier 1 ahead of time 213 instructions a node, unchanged (the
path is cold). The JIT went from 285 to 306: Cranelift's register allocator weighs a call in
a cold block as one in the loop. That is open.

## A pair's second byte from the block's word

The lean lexer finds the two-byte punctuation once a block. Its window looks up each byte's
first-byte bits and the next byte's second-byte bits, `and`s them, and keeps one `pair` word.
A token then tests one bit. The generated loop loaded the pair table's row for the next byte
at every token: a shift, an add, a load and a test, on the token's own chain.

Now the compiler makes the lean lexer's tables from the pair table (`pair_nibbles`). Each
first byte with a pair gets its own bit, eight at most, so the first byte's lookup is exact.
The second byte's bits are the first bytes it ends a pair after. The tables are checked
against the table for every first byte and every next byte; a table they do not match, or one
with a pair ending in a zero byte (the padding), keeps the per-token load. The refill makes
the word with the block's other words (`PAIR_WORD`, in the registers), and the fused pair
rule's last is the token's first byte plus its bit.

The fused run rule's last is read the same way: the run after the first byte by two shifts of
the block's word, one bound on where it ends. No check is left for a token at the block's
last byte.

For `.rr` the six first bytes (`:` `=` `!` `<` `>` `-`) are six bits
(`the_pairs_are_a_blocks_word`).

Measured (`jit_against_aot`): tier 1 ahead of time 222 instructions and 65 cycles a node,
from 213 and 73. The word costs its block more than the loads cost its tokens, but no token
waits on a load for its end. The JIT 292 and 67, from 306 and 79.

## The gap marks fold at the refill

A gap mark (`.rr`'s form feed between tokens) marks the next grammar token where its bit is
anywhere from the byte after the grammar token before's first to the token's own. The loop
tested that range at each token, from the block's registers when the range began in the
token's block. At a block's first token it began in an earlier block, so the loop called the
mark's window function once a block, from there to the token, the token's own block
included: bytes the refill had just classified.

The lean lexer keeps its masks in its window and asks nothing of a block it has left.

Now the refill folds. Each block it passes with no start gives its bits from `from` to the
bits so far (`pending`). Bits before the cursor's block (a token across blocks, a comment the
next grammar token is after) are read once by the bytes, cold. Then `from` moves to the new
block. Every token's range starts in its own block, so its test is the registers alone: no
bounds compare, no call. `unless after` is applied to the bits so far as before, so folding
changes no code. The fold needs words that wait on no block (made from the bytes); a loop
that waits keeps the per-token test.

`the_marks_tests_are_the_masks` has form feeds in gaps across blocks: after spaces past a
block, in a backtick literal, in a comment. A fold left out fails it.

Measured (`jit_against_aot`): tier 1 ahead of time 193 instructions and 55 cycles a node,
from 222 and 65; the JIT 269 and 61, from 292 and 67.

## The records' layout (2026-11-14)

The token loop held three column bases for its records, and they were among
the values it spilled each token. The records' layout is now the compiler's
choice (decision 0231): rows on the CPU for rr, so the loop's stores share
one base, as the lean lexer's `Tok`; columns on Metal.

## The common tokens' state (2026-11-14)

The lean lexer's run loop takes a word or a punctuation by its table and
its masks: no rule is compared, no mark is merged. Tier 1's loop did both
at every token. Now:

- **The fused rules by the entry's bits.** A first byte's entry is 8 bytes
  (it was 4), with a bit where the byte's rule is the fused pair rule and
  one where it is the fused run rule (`TOKEN_FUSED_*`,
  `RecordTables::fused`). The loop tests a bit of the entry it loaded.
  The rule's number is compared only off that path, and only where the
  loop's choice differs from the table's.
- **No marks' merge where there is nothing to merge.** The refill sets a
  flag in the marks so far where the block has a gap mark's bit
  (`GAP_FLAG`). A token whose marks so far and hits are zero keeps its
  code and leaves the state untouched; the merge is a cold block.
- **The word before only where an `unless` is the loop's.** rr's
  `unless after ","` is a lazy mark, so its loop keeps no word.
- **No extra's mark on the fused path.** The fused rules' first bytes
  never start an extra (`fused_rules`), so their tokens read no mark
  word at their first byte (`never_at`).

Tier 1 AOT 186 to 171 instructions a node, JIT 259 to 251. The loop
still spills around its dispatch: see decision 0231, "Why the spills
stay".

Two more (the same day):

- **`from` is read, not carried.** Where the refill folds the gap marks
  (decision 0177), the bits not yet taken start past the token before's
  first byte, in the token's block. That byte is in the record behind the
  cursor (`FileCols::after_last_start`). The merge, the refill and the end
  read it there; the loop carries no value a token for it.
- **Cold blocks are cold in the Rust.** `clif_rust` writes
  `core::hint::cold_path()` at each block Cranelift has cold, so rustc
  lays them out and weighs them as Cranelift does.

Tier 1 AOT 171 to 164 instructions a node.

## Punctuation by the table (2026-11-14)

Counted over the corpus (2.56M tokens), 55.5% are the fused pair rule's
(punctuation) and 35.3% the fused run rule's (words); 86% had a first
byte some word starts with, so they looked a word up: every punctuation
too, as rr's punctuation are words of the table. The lean lexer looks
none up: `TOKEN[c | pair << 8 | …]` gives a punctuation's kind.

Now a punctuation's code bits are by its first byte and its length
(`RecordTables::punct`, `TOKEN_PUNCT`), a load beside the entry's. The
compiler fills the table with the word table's own lookup, so it holds
what the lookup gives; a first byte whose pairs are two words (`<`:
`<=`, `<:`) keeps the lookup. `rrs_records_are_rows_on_the_cpu` checks
which bytes take the table.

Tier 1 AOT 169 to 155 instructions a node, JIT 253 to 239.

## A string read once (2026-11-14)

A string's end and its tests read the same bytes. Before, the loop read
them twice:

- the closer's search, a call a word (`bytes12`), from aligned words, so
  each word looked back a byte for the escape's carry;
- then the count tests' function (`tests1`), skipped only where a string's
  span fit 16 bytes with none of the bytes the tests need.

The two took 10.7% of tier 1's time. The lean lexer's `string_end` takes
one loop. Its window starts after the opener, where no escape is open,
and it carries the escape's parity from one window to the next.

Now a reader's arm with count tests is one function (`read_fn`), as the
lean reader:

- **The closer from `s + 1`, 64 bytes apart.** `Bytewise::words_carried`
  takes `escaped`'s carry in and gives its carry out (simdjson's add
  overflow), so the loop passes it on. `Bytewise::apart` proves the
  opener is no escape byte, so the first carry is zero, with no load.
- **The tests behind a gate.** One check of four vectors (`needs_wide`)
  says whether the token's bytes, up to 64, hold any byte the tests need.
  Where none does, there are no hits and no test mask is made. Where one
  does, or the token is longer, the tests' function runs, as rare as
  such bytes.

```rust
// A string with no `\` and no control byte (the tests' needs):
// two compares a window, the escape's carry, the gate; no test mask.
let (ws, couts) = bw.words_carried(f, &file, &[a.closer], at, &cins, heads);
```

A first form made every test mask in the loop, for every string. It
was dropped on a count (155 to 159 instructions a node), which is not
how a step is judged. It was lean's shape, and it is back: see "A
string's tests in its read loop" below.

Tier 1 AOT 155 to 147 instructions a node, JIT 239 to 222. The string
reader is 153 lines of machine code, down from 460, and saves two
register pairs, not six.

## The rare tokens' rules are calls (2026-11-14)

A corpus average hides which tokens cost what. `tier1_by_kind` (in
rr_aot, run by hand) counts instructions a token on files of one kind
each, tier 1 against the lean lexer.

The punctuation path in the loop showed stack traffic around the rule
dispatch: six loads and stores of the pending marks, the entry, and the
run and pair words. The loop held every rule's blocks inline: the
readers, the numbers' word loop, their tests. Their registers were the
common tokens' registers too.

The lean lexer has the same split in its loops: the common tokens in a
tight run loop, the rare ones outside it.

Now each rule but the two fused ones is a function of its own
(`rule_fn`, `RuleMode`), where its reads are the bytes'. The loop's
dispatch calls it, so a rare token is one call and the loop's code is
the common tokens'. The function is `rule_last`'s own block for that
rule, built alone (`RuleMode::Only`).

Measured, instructions a token (tier 1 AOT; the lean lexer's after the
arrow):

| Shape | Corpus a node | Punct. | Words | Strings | Numbers |
|---|---|---|---|---|---|
| All rules inline | 147 | 72 | 109 | 395 | 350 |
| One function for all rare rules | 150 | 68 | 112 | 430 | 374 |
| A function a rare rule (kept) | 149 | 69 | 107 | 430 | 379 |
| Same, the readers inline | 150 | 75 | 111 | 391 | 374 |
| The lean lexer | 109 | 54 | 98 | 241 | 107 |

The JIT went 222 to 217 a node. The common tokens (90% of the corpus)
are nearer the lean lexer's. A rare token pays one call: about 30
instructions of prologue, argument moves and a four-value return
through memory. Packing the return into two registers was costed and
dropped then. That was a count's decision. A lexer's function returns
its token in registers, so the packed return was taken later ("A
spent block is read once; a rule returns two words").

Open: numbers are 379 a token against 107. Their run's word is a
window function's call at each token, and their tests' gate is the
short one.

## Three rare paths nearer the lean lexer's (2026-11-15)

- **A string's gate by offset.** The tests' needs were one byte set
  over a range of offsets: a control byte or a `\` from the byte before
  the token to its last. A newline before a string's quote then opened
  the gate. `native_cpu_bytes::needs_by_offset` keeps a set for each
  offset (a `\` before, a control byte at), and the gate checks each
  set over the token's bytes at its own offset (`counts_need_by_offset`).
- **A number's run by one vector.** A run whose word is not in the
  loop's registers read a 64-byte window at each token. Now its first
  16 bytes come from one vector (`Bytewise::short_word`, a one-part
  `bitmask`), as a lexer reads a number's few digits. A run on past
  them takes the word's path (cold).
- **A punctuation's code from its entry.** The code was a second load
  beside the entry's. The compiler places the codes in the entry's free
  bits, by their widths, past the kind's bits
  (`RecordTables::punct_in_entry`), where they fit; else the table
  stays. rr's fit: the one-byte code's 23 bits, the two-byte code's 4.

Mutations of the run's bound and of the gate's offsets each fail a
named test.

| | Before | After | Lean |
|---|---|---|---|
| Strings | 430 | 349 | 241 |
| Numbers | 379 | 256 | 107 |
| Punctuation | 69 | 66 | 54 |
| Corpus, a node | 149 | 146 | 109 |

## The lean lexer's starts (2026-11-15)

**The problem.** After each token the loop cleared the starts inside
it: a shift, a mask, and a check that the token ended in its block
(5 instructions). The lean lexer clears one bit (`run &= run - 1`).
Its starts already leave out the bytes no token starts at. Every
token also compared its start with the file's length.

**The change.** The refill builds the lean lexer's starts:

- no byte past the file (`valid`, made once a file);
- no run byte after a run byte (`word & word << 1`, from the cursor);
- no pair's second byte (`pairs << 1`), pairs read left to right
  where they chain (`=>=`), by a carry: a run of first bytes keeps
  every other bit from its first.

A fused token (`fused_rules`) then covers no start after its own, and
its tail clears its bit alone. No token compares its start with the
length.

`lean_starts` proves when this holds, from the bytes' sets:

- no pair byte is a run byte;
- a pair's second byte and a run byte are no space and no zero;
- the run rule's first bytes that are no run byte (`#` of
  `#include`) leave the fused path (`run_fused`).

Another rule's token (a string, a number) may end where a start was
left out (`12abc`, `"="=>=`). Its tail puts the byte after it back,
or refills where a chain of pairs runs across its end.

**A punctuation's entry by its pair.** There are now 512 entries, by
first byte and pair bit, as the lean lexer's `TOKEN[c | pair << 8]`.
A punctuation's code is one field of its entry, with no choice by its
length. A fused token's next start needs no max with its first.

Taking out the put-back, or the chain's order, each fails
`the_cpus_records_are_the_interpreters`. Its new inputs cover these
cases at the blocks' edges.

| A token | Before | After | Lean |
|---|---|---|---|
| Punctuation | 66 | 57 | 54 |
| Words | 107 | 103 | 98 |
| Mixed code | 69 | 61 | 63 |
| Strings | 349 | 368 | 241 |
| Numbers | 256 | 272 | 107 |
| Corpus, a node | 146 | 140 | 109 |

Strings and numbers pay for the put-back on the rules' general tail.
`tier1_by_kind` now counts comments as tokens, for both lexers.

## The word hash's multiplier, kept (2026-11-15)

**The problem.** The lean lexer keeps its word hash's multiplier in a
register. The generated Rust used the 64-bit literal at each word's
lookup, so it was made there each time: a `mov` and three `movk`.
The source had a literal where lean's has a value kept.

**Tried and dropped.** Multipliers of two or three 16-bit parts (two
or three instructions to make). No such multiplier parts rr's words.
A short word's hash then depends on its first two bytes alone, so
`abc` and `abd` meet.

**The change.** `clif_rust` makes a wide constant once (`kept`, an
empty `asm` the value passes through) where the constant is made in
one block and used in others: a loop's invariant, as the multiplier
where a file's token loop starts. `wide` is a constant of more than
two parts that is not an `orr` immediate. Others stay literals.
rustc then keeps the multiplier in a register, or reloads it from its
slot with one load.

`the_word_multiplier_is_kept` checks the generated source, and which
constants `wide` takes.

| A token | Before | After | Lean |
|---|---|---|---|
| Words | 103.4 | 101.6 | 97.9 |
| Punctuation | 57.0 | 56.6 | 53.5 |

## No guard the zeros make (2026-11-15)

**The problem.** A number's 16-byte window tested the file's ends
three times:

- a `within` of the program (the byte after a `.` is in the file);
- the window's cut at the file's length;
- the run's bound at it.

That is about 30 instructions for each number. But a batch's files
have zeros on both sides (`FRONT` before, at least the delay and 16
after: simdjson's padded input).

**The change.** `native_cpu_lanes::zero_kills` asks of a value
whether it is false wherever the byte some way back is zero: an
`eq_byte` of a byte not zero, a class without zero, a `prev` further
back, either side of an `and`, both of an `or`.

- A `within(d)` (`d` at most `FRONT`) is left out of a conjunction
  where another conjunct is false on the byte it guards
  (`unguarded`), in the windows that have a position, not a full
  block's body.
- `short_word` leaves out its cut at the length where the output is
  false on a zero byte.

Comments had the same guard.

`a_within_goes_where_a_zero_byte_refuses` checks the analysis on a
small program. Making `zero_kills` always true still passes the named
tests: rr's guards are all ones the zeros make.

| A token | Before | After | Lean |
|---|---|---|---|
| Numbers | 272 | 246 | 107 |
| Comments | 591 | 498 | 193 |

## A short gate's span by lane (2026-11-15)

`needs_some` cut a short token's vector of hits to its span by each
64-bit half's shifts (about 12 instructions). It now compares the
lanes' indexes with the span once, as `needs_wide` does. Numbers
246→233 a token. A one-lane span fails
`a_numbers_checks_are_at_its_token` and the engine's tapes.

## A comment's count its opener makes (2026-11-15)

**Intro.** A lexer that reads a comment knows it starts with `//`
or `/*`. rr's grammar marks the token after a comment for JSON:

```
# Holds at every comment: a comment starts with an opener.
after comment when count(line_open | block_open) at least 1
```

The generated loop did not know it. Each comment ran a 64-byte
gate for `/` (its own opener has one, so the gate always failed),
then a call that counted the openers block by block.

**The change.** The lowering proves the count holds
(`Lowering::implies`): every arm opener of the comment's reader
implies `line_open | block_open`.

- The proof walks the operations: a conjunct or each disjunct of
  the left side, a disjunct or each conjunct of the right, a `prev`
  as its operand at a smaller delay, `within` at matching delays.
- A proved test has `least` 0 (`TokenCheck::always`). The CPU makes
  its hit a constant and fuses no count call for it.
- It holds where the rule's reader reads a token. A `/` that the
  reader passes to its other rule (division) gets no hit
  (`records::test_holds`). Metal tests the reader's openers the
  same way.

`a_comments_opener_is_its_count` checks the fold and where it holds.
When the interpreter let the hit through at every `/`, the records
test failed on a division in the corpus.

| A token | Before | After | Lean |
|---|---|---|---|
| Comments | 498 | 371 | 193 |

Open: if the always-true test's mask moves from "made at the
tokens" into the CPU body's stored outputs, one refusal flag flips
(a string open at the file's end). Nothing reads that mask now. It
stays at the tokens, but the flip is not yet explained.

## A comment's tag is its code (2026-11-15)

**Intro.** The lean lexer's comment branch records the comment and
goes on. It never looks a comment up as a word. The generated loop
did, because `/` starts a word (the `/` punctuation):

```
// Before: every token past the rule took the word step.
kind = has_tag ? tag_kind : entry_kind;   // four selects
if wordy(entry) { look the word up }      // a comment too
// After: a reader's tag is the code.
if has_tag { code = tag; goto coded; }
```

**The rule.** A token a reader reads and tags (`Reader::retag`, a
comment) is no word. The interpreter (`records::pass`) and Metal set
its word to none; the CPU's token loop takes its tag as its code and
skips the lookup. rr's only word starting with `/` is one byte long
and a comment is two bytes or more, so rr's codes do not change.

The first try kept the selects and cleared the word bit when tagged.
Comments went down 29, but the other kinds rose about 2 each, because
rustc assigned the loop's registers differently. The branch removes
the selects too:

| A token | Before | After | Lean |
|---|---|---|---|
| Words | 101.6 | 99.2 | 97.9 |
| Punctuation | 56.6 | 55.4 | 53.5 |
| Mixed | 61.1 | 59.9 | 63.1 |
| Strings | 368 | 360 | 241 |
| Numbers | 233 | 226 | 107 |
| Comments | 371 | 338 | 193 |

## A byte test is a load and a compare (2026-11-15)

**Intro.** The lean lexer tests a comment's opener as
`c == b'/' && d == b'*'`. The CPU's scalar tests of a mask at a
token (`native_cpu_bytes::scalar`) checked each byte's place in the
file instead, about 38 instructions an opener:

```
// Before, one byte:
b = (i < len) ? bytes[i] : 0;            // bounds select
x = ((b >> 6) == 0 ? SET : 0) >> b & 1;  // 256-bit set
x &= (i >= 1) & (i < len + 1);           // prev's and within's
// After:
x = bytes[i] == '/';
```

**The change.** The batch's layout (`Batch`) puts `FRONT` (16) zero
bytes before each file. After its end there are zeros as far as the
program looks ahead. So near a token:

- A byte off the file reads as zero: no bounds select (`byte_at`).
- A `prev`'s bound goes where its operand is false on a zero byte,
  read at most `FRONT` before the token (`zeros_bound`).
- A `within` goes where another conjunct is false on its zero byte
  (`unguarded`, at or after the token). So does the bit's bound at
  the file's end.
- A one-byte set is one compare.

`a_prevs_bound_goes_where_the_zeros_say_it` checks the bound. rr's
masks give no case where it must stay, so dropping every bound still
passes the named tests.

| A token | Before | After | Lean |
|---|---|---|---|
| Words | 99.2 | 98.5 | 97.9 |
| Punctuation | 55.4 | 54.4 | 53.5 |
| Mixed | 59.9 | 59.0 | 63.1 |
| Strings | 360 | 344 | 241 |
| Numbers | 226 | 222 | 107 |
| Comments | 338 | 312 | 193 |

## A bitmask's pairwise adds kept (2026-11-15)

**Intro.** A 16-lane compare becomes a 16-bit mask by weighting
each lane and adding pairs three times (simdjson's arm64
`to_bitmask`, five instructions). The CLIF says
`iadd_pairwise(a, a)`. `clif_rust` wrote it `vpaddq_u8(a, a)`, and
LLVM turned each one into three instructions:

```
// Before (rustc from vpaddq_u8(a, a)), each step:
uzp1.16b v1, v0, v0
uzp2.16b v0, v0, v0
orr.16b  v0, v1, v0
// After (vpadd_u8 of the halves, recombined):
addp.16b v0, v0, v0
addp.8b  v0, v0, v0
addp.8b  v0, v0, v0
```

**The change.** A pairwise add of a vector with itself is written
as the 64-bit `addp` of its two halves, recombined
(`vcombine_u8(d, d)`). The lanes are the same, and LLVM keeps one
`addp` a step. `a_bitmask_is_pairwise_adds` checks the generated
source.

| A token | Before | After | Lean |
|---|---|---|---|
| Words | 98.5 | 97.4 | 97.9 |
| Punctuation | 54.4 | 54.3 | 53.5 |
| Mixed | 59.0 | 58.6 | 63.1 |
| Strings | 344 | 337 | 241 |
| Numbers | 222 | 215 | 107 |
| Comments | 312 | 306 | 193 |

## A digit is a range (2026-11-15)

**Intro.** A lexer tests a digit as `b - b'0' <= 9`. The CPU's
vectors tested every class by its nibble tables: two lookups, two
`and`s, a compare (about seven instructions a vector). A number's
run reads three vectors (a `.` between two digits needs the bytes
either side), so about twenty of a number's instructions were
lookups.

**The change.** A class whose bytes are one run `lo..=hi`
(`native_cpu_lanes::set_range`) is `b - lo <= hi - lo` on the
vector: two instructions, and one compare for a single byte. A class
whose table this window has looked up already keeps the lookup: it
then costs an `and` and a compare.

The scalar tests at a token keep a run as the set's word. As a
subtract and compare in the main loop's cold tests (a `-` before a
digit), rustc allocated the loop's registers differently, and every
kind rose about one instruction. Without it, no kind rose.
`a_digit_is_a_range` checks the runs.

| A token | Before | After | Lean |
|---|---|---|---|
| Numbers | 215 | 197 | 107 |

Other kinds did not change.

## A float's `.` is counted where it is read

**Intro.** A lexer knows a number is a float because it read the
`.`. rr's grammar says so with two counts at a number: a `.` makes
it a float (`count(dot) at least 1`), and a second `.` is an error
(`at least 2`). The CPU checked a short token's bytes for a `.` in
one vector. Where none was there, it skipped the counts. A float has
one, so every float called the counts' function, which counts a word
at a time: about 14% of a number's time.

**The change.** Where a count's mask is its token's bytes exactly
(`native_cpu_bytes::exact`: byte tests and their unions at one
offset), a token of 16 bytes at most is counted in the same vector.
The lanes in the set are ones. The two halves are added, and a
multiply sums their bytes. A `.` mask is used by both counts and is
computed once. A longer token still makes the call. A mask that
reads its neighbours (a string's bad escape) is not exact and keeps
the old gate.

```
lanes = (bytes[s .. s+16] == '.') & (index < len)   // 0 or 1 each
n     = ((lo64(lanes) + hi64(lanes)) * 0x0101..01) >> 56
```

`a_floats_dots_are_its_bytes` checks that the number's counts are
exact and a string's are not.

The count by a multiply is gone where a test asks only one or two
(see "A count test asks what it asks" below).

| A token | Before | After | Lean |
|---|---|---|---|
| Numbers | 197 | 183 | 107 |

Other kinds did not change.

## A short number calls nothing

**Intro.** A lexer reads a number's few digits and returns. The
number rule's function did the same work, but it first saved ten
registers and restored them at the end: about 26 instructions a
number. They were saved because its rare paths call other functions:
a run past its first 16 bytes reads on a word at a time, and a long
token's counts are a call.

**The change.** A run rule whose bytes the CPU reads gets two
functions. The whole one (`RuleMode::Only`) is as before. The short
one (`RuleMode::Short`) is the one the loop calls:

```
short(bytes, len, blocks, s, raw, entry):
    run = the rule's mask over bytes s+1 .. s+16   // one vector
    if the run ends inside it:
        the token's tests, knowing it is 16 bytes at most
        return
    return whole(bytes, len, blocks, s, raw, entry)
```

The token's tests know its length bound (`tested`'s `most`). A count
whose mask is exact needs no "does it fit" branch and no call. A
test that needs more bytes than the token can have is not made (a
number of 19 digits or more). The short function is kept only where
it calls nothing but its whole function (`calls_of`). Otherwise the
loop calls the whole function as before.
`a_short_numbers_function_calls_only_its_whole` checks the shape.

The short function still saves two registers. The rules return four
values through memory, so its long case can't hand its caller's
result slot straight on.

**Tried and dropped then.** Returning two registers (the last, and
the hits with the refusal as their sign bit) made the long case a
plain tail call. It was dropped on the other kinds' counts, which is
not how a step is judged. It is lean's shape, and was taken later ("A
spent block is read once; a rule returns two words").

| A token | Before | After | Lean |
|---|---|---|---|
| Numbers | 183 | 176 | 107 |

Other kinds did not change.

## A short comment calls nothing

**Intro.** A lexer reads a comment by searching for its closer from
where the comment starts. The comment reader's function searched the
closer's mask in the comment's own 64-byte block. It called that
block's window function, so it saved six registers for every comment.
A comment that crossed into the next block was found a block later.

**The change.** A reader gets a short function too
(`RuleMode::Short`), as a run does. Its closer's first word is made
inline from the 64 bytes after the opener's byte (`Bytewise::word`
at `s + 1`), not from the aligned block. A comment shorter than that
is found without a call. A longer one returns the whole function's
call. The short function is kept where every call it makes is to its
whole function: one a reader's arm.

The window may start anywhere in the file. The batch pads each file
with zeros to its last block's end, and `word` cuts the bits past
the file's end, so its reads stay inside the batch.

A first try took the closer's word from the aligned block, as the
whole function does. Half of these comments cross a block's edge, and
each of them paid for the short try and then the whole read:
comments 306 → 317.

| A token | Before | After | Lean |
|---|---|---|---|
| Comments | 306 | 271 | 193 |

Other kinds did not change.
`a_short_tokens_function_calls_only_its_whole` checks that a number
and a comment each have one.

## A refused read is still its reader's token

**Intro.** A block comment open at the file's end refuses the file.
The interpreter still records the comment's first byte as a token of
the reader: tagged as a comment, with the reader rule's tests run on
it. The CPU recorded it as a `/` and ran no tests. The file is
refused either way, but the CPU's records must be the interpreter's.

**The change.** A reader with no closer and no other way out
(`ReadNone::Refuse`) goes on to the rule's tests with its token one
byte long, its tag, and the refusal. The tests' entry block takes a
refusal in for this, zero everywhere else.
`the_cpus_records_are_the_interpreters` now has comments whose
closer lies 59 to 65 bytes after the opener, and comments the file
ends in, at five offsets in their block. These cases found the bug.

## A plain string calls nothing

**Intro.** A lexer reads a string without a `\` straight through to
its closing quote. The CPU's string took three calls: the loop called
the string rule's function, that called the fused read (the end and
the counts' gate), and the read called the counts where the gate
failed. Each of the first two saved registers.

**The change.** The string rule gets a short function too. Its arm
makes the fused read inline (`read_fn`'s first window): the closer in
the 64 bytes after the quote, then the counts' gate over the token.
A string whose closer is in that window and whose bytes the counts
don't need goes straight on to the rule's other tests. Every other
string returns the whole function's call: a longer one, one whose
bytes the counts need, one the file ends in.

Two things kept other calls in it:

- The quote's own test, `quote & ~escaped`, asks whether a run of `\`
  ends just before it. Where one does (rare), the scalar test calls
  the run's carry function. In a short function that path returns
  the whole function's call instead (`Bytewise::bail`).
- The tests' entry block calls the counts. The inline read jumps past
  it, so nothing reaches that call. `calls_of` now counts the calls
  in blocks the entry reaches, as the compiled code has them.

| A token | Before | After | Lean |
|---|---|---|---|
| Strings | 337 | 306 | 241 |

Other kinds did not change. `the_cpus_records_are_the_interpreters`
has `\` runs right before a string's quote, at four offsets.

## A returned call is the return

**Intro.** A short function's long case returns its whole function's
result as it is. The Rust the backend wrote took the call's four
results apart and built them again for the return:

```
let t = unsafe { f36(pin, v0, v1, v2, v3, v4, v5) }; v112 = t.0; …
return (v112, v113, v114, v115);
```

rustc returns four values through memory, the caller's slot. It
gave the call a slot of its own and copied it over, so the short
function kept a frame and a saved register for that slot's address.

**The change.** Where a call's results are returned at once, in
order (`clif_rust::returned_call`), the backend writes the call in
the return: `return unsafe { f36(…) };`. rustc then hands the
caller's slot on, and the call is a branch. The short functions have
no prologue now. `a_returned_call_is_the_return` checks the source.

| A token | Before | After | Lean |
|---|---|---|---|
| Strings | 306 | 298 | 241 |
| Numbers | 176 | 168 | 107 |
| Comments | 271 | 263 | 193 |

Words, punctuation and mixed code did not change.

## A string's gate reads its own bytes

**Intro.** A lexer reads a short string's bytes and no more. Before
calling a string's counts, the CPU checks the token's bytes for any
the counts need (a `\`, a control byte). That check read four
16-byte vectors from the opener for every string, cutting the lanes
past the token by their index. Most strings are under 32 bytes.

**The change.** The gate (`needs_spans`) reads one vector for a token
of 16 bytes at most, two for 32, four for 64. Each step is one branch
on the length, and these branches go the same way for strings of
like lengths. The vectors are ORed as before and reduced once at the
end.

```
any = in_set(v[0]) & lanes < span
if span > 16: any |= in_set(v[1]) & lanes < span
if span > 32: any |= in_set(v[2]) & …; any |= in_set(v[3]) & …
```

`the_cpus_records_are_the_interpreters` has a tab and a `\u` escape
14 to 48 bytes into a string, at each vector's edge.

| A token | Before | After | Lean |
|---|---|---|---|
| Strings | 298 | 285 | 241 |

Other kinds did not change.

## A long string's gate reads it a chunk at a time

**Intro.** A lexer that finds a string's closing quote also knows
whether it passed a `\` on the way. Our string read finds the quote
first, then checks the token's bytes for any the counts need (a `\`,
a control byte), and calls the counts only if it finds one. That
check covered 64 bytes at most. A longer string always called the
counts, which read every byte of it again.

**The change.** Past 64 bytes the gate (`needs_chunks`) loops over
64-byte chunks: four vectors a chunk, then the rest as before. It
stops at the first chunk with such a byte, which then calls the
counts as before. It is a loop of whole windows, like the closer's
search, not a loop over bytes.

```
while rest > 64: if any(chunk at p): call counts; p += 64; rest -= 64
if any(rest at p, cut at rest): call counts
```

`the_cpus_records_are_the_interpreters` now has a tab and a `\u` in
strings past 64 bytes, at each chunk's edges, in a whole chunk and
in the rest, and long strings with neither. Making the long gate
always quiet fails that test.

| One string a line | Before | After | Lean |
|---|---|---|---|
| 66 bytes | 930 | 691 | 514 |
| 80 bytes | 970 | 716 | 517 |
| 120 bytes | 1083 | 776 | 526 |
| 250 bytes | 1587 | 1090 | 730 |

The by-kind lines did not change: their strings are 37 bytes at
most. `tier1_by_kind` takes `RR_LINE` to count a line of one's own.

Tried and dropped along the way:

- The two-register return again (`(last+1) | (tag+1) << 32`, and
  the hits with the refusal as their sign bit), dropped on the
  common kinds' counts. Taken in the next section, as lean's
  function returns its token in registers.
- Not marking the rule calls cold. rustc's code did not change.

## A spent block is read once; a rule returns two words

**Intro.** A lexer that reaches the end of a window's tokens reads
the next window, and a lexer's function returns its token in
registers. Ours did neither. When a block's starts ran out, the
refill classified the cursor's block again from the cursor. After a
token that ends inside its block that is the same block, and the
pass found nothing. A rule's function returned four words, which
Rust passes through memory: four stores in the callee, a vector
load and lane moves in the loop.

**The changes.**

- The refill reads the next block whole where the token before
  started in the cursor's block, since that block's starts are
  spent (`skips`). One case reads the block again from the cursor:
  a run of pairs going on across another rule's token's end. The
  token sets a flag for it in the marks so far (`REREAD_FLAG`, their
  sign bit, next to `GAP_FLAG`), and the refill clears it. No test
  reaches that case: no `.rr` token ends in a pair's first byte with
  pairs chained, and a mutation that never sets the flag passes.
- A rule's function returns `(last + 1) | (tag + 1) << 32` and
  `hits | refusal << 63` (`pack_rule`), where every reader's tag
  fits in 32 bits (`packs`). The loop unpacks them; only the
  refusal's sign is read.

Stepped by instruction, a block holding one token: 471 → 349, one
refill where there were two. The loop's calls now have no result
slot and no vector reloads (none of either in `f0`), and the short
number function ends `add x0 …; orr x1 …; ret`.

`the_cpus_records_are_the_interpreters` has a token alone in its
block followed by spaces to and past the block's end, after words,
strings, comments, pairs and floats. `a_rules_function_returns_two_words`
checks the generated source.

Counts, reported, not the criterion. rustc placed the token path's
spills differently, and that path is now longer:

| A token | Before | After | Lean |
|---|---|---|---|
| Words | 97.4 | 104.1 | 97.9 |
| Punctuation | 54.3 | 61.6 | 53.5 |
| Mixed | 58.6 | 64.6 | 63.1 |
| Strings | 285.3 | 288.5 | 240.6 |
| Numbers | 167.8 | 171.9 | 107.3 |
| Comments | 263.4 | 265.2 | 193.3 |

Still apart from lean: the spent block's gap marks are re-read by
the bytes (`marks_in`, a call a block), and the token path has more
stack accesses than lean's two loads.

## Two loops, as the lean lexer's

**Intro.** The lean lexer has two loops: one over windows, which
makes a window's masks, and one inside it over the window's starts,
which only reads them. Ours had one loop. Its head took the block's
run words as parameters, so every token passed them back, and rustc
stored and reloaded them each token. The pair mask was stored at a
token's start and loaded at its end, and the marks so far went
through the stack and back.

**The change.** The block's starts are a loop of their own
(`from_word`, its header): the token's state words and the starts
left are its parameters. The run words are the refill's values,
which dominate it, and no token passes them. A token that leaves
starts goes on in the inner loop (`while run != 0`); one that leaves
none goes back to the head, whose refill reads the next block. The
head no longer branches on the starts: it is reached with none.

The token path, stepped by instruction on a line of words: 47
instructions for a short word (lean: 67). It has two loads from the
stack, the pair mask and the bytes' address, against lean's two (its
word and pair masks). It has no stores and one back branch, where
there were seven stack accesses and two jumps.

Counts, reported (before this change, after; lean):

| A token | Before | After | Lean |
|---|---|---|---|
| Words | 104.1 | 100.1 | 97.9 |
| Punctuation | 61.6 | 55.9 | 53.5 |
| Mixed | 64.6 | 59.2 | 63.1 |
| Strings | 288.5 | 293.2 | 240.6 |
| Numbers | 171.9 | 172.0 | 107.3 |
| Comments | 265.2 | 270.6 | 193.2 |

### The token path, read against lean's

A trace of a line of words, stepped by instruction, has two token
paths. Both have lean's shape.

A word that is not a keyword takes 47 instructions. It loads the
byte word at the start, then the pair mask's bit. It finds the
token's row in the class table, finds the token's end from the run
word, then writes the record. The next start is the low bit of the
starts left, and the loop goes back. Lean takes 67 instructions for
the same token, with the same steps.

A word that may be a keyword takes 72. The 25 more are the keyword
probe, the same probe as lean's:

```
cmp   len, #8 ; csel        // at most 8 bytes
lsl / lsr     bytes, shift  // the word's bytes alone
eor   len                   // and its length
mul   mult ; lsr #55        // the hash
ldp   [table + hash]        // the keyword's bytes and kind
ccmp ; csel                 // a match keeps the kind
```

Three differences from lean remained. The generated code and the
lean lexer are both Rust, compiled by the same rustc, so each comes
from a difference in the source, not from rustc:

- The hash multiplier was loaded from the stack at each probe. Lean
  keeps it in a register (`x19`). Our source hid it behind an empty
  `asm!` (`kept`), so rustc could only keep it in a slot.
- The bytes' address was reloaded at the back branch. Our token body
  called a function of its own for a word past eight bytes, so more
  values had to live across a call than registers keep.
- The probe's shift, `(8 - len) * 8`, is three instructions where
  lean's is two. Lean's length is known to be 1 to 8 (`clamp(1, 8)`
  after `words != 0`); ours, `min(len, 8)`, may be 0.

## A long word is compared in the loop, as lean's

**Intro.** Lean's `word_code` checks a word past eight bytes inline:
its one such word, `violation`, has its ninth byte compared. Ours
called a function (`tail_fn`) on a hit of a long word, which
compares up to seven more words. A call in the token body makes
rustc keep its values across it.

**The change.** Where every word is 16 bytes at most (rr's are), a
slot is 32 bytes and holds the word's bytes 8 to 16. The token
compares its own, read at `from + 8` where it is longer than eight
bytes, into `hit`: straight-line, no call, as lean's ninth byte.
Longer words keep the call. The multiplier is a literal where the
hash is, as lean's (`kept` removed).

The token path, stepped by instruction (lean's in brackets):

- A plain word: 46 instructions [67]. The bytes' address and the
  table's base stay in registers: no reload, no `adrp`.
- A word that may be a keyword: 86. The multiplier is now made at
  each probe (`mov` and three `movk`), as rustc makes a literal
  where no register is free across the loop. Lean's loop has one.
  Ours keeps four state words across it (refusal, pending marks,
  their start, the word before) that only rare tokens change, where
  lean keeps one (`cursor`). That is the next step.

Counts, reported (before, after; lean):

| A token | Before | After | Lean |
|---|---|---|---|
| Words | 100.1 | 107.7 | 97.9 |
| Punctuation | 55.9 | 57.5 | 53.5 |
| Mixed | 59.2 | 59.9 | 63.1 |
| Strings | 293.2 | 282.9 | 240.6 |
| Numbers | 172.0 | 162.8 | 107.3 |
| Comments | 270.6 | 261.2 | 193.2 |

## A block's starts are runs, split at its rare ones

**Intro.** The lean lexer's loop inside a window is two loops. The
outer one splits the window's starts at the first rare one (a
string's, a comment's, a number's first byte: `w.rare`); the inner
one, `while run != 0`, takes the starts before it. Only words and
punctuation are in the inner loop, and it calls nothing but the
rare `word_of`. The rare token's calls are in the outer loop.

Ours had one loop over a block's starts. Every token went through
the same body, which branched to every rule, so the body held the
rules' calls (five rule functions and the word across a block).
What a call must keep was kept on every token's path: the hash
multiplier, the pair mask and the bytes' address had no register.

**The change.** The rare first bytes are those of no fused rule (no
`TOKEN_FUSED_*` bit), as nibble tables (`set_nibbles`); the refill
makes the block's rare word from its bytes (`set_word`), as the
lean lexer's window its `rare`. A loop over the block's runs
(`mid`) takes `rare = cand & rare_w` and
`run = cand & (rare - 1) & !rare`:

- a run: its first start, in the inner loop, with the run alone;
- else a rare start: a token of the rules' general tail;
- else the refill.

A fused token goes on in the run; the run done, back to `mid` with
the starts after it. Every other token goes back to `mid`, with the
starts left of the run and after it (a word across its block, too).

The inner loop calls nothing (`the_run_loop_calls_nothing`).
The token path, stepped by instruction (lean's in brackets):

- A plain word: 42 [67]. No stack access (lean: two loads). The
  pair and word masks, the tables and the bytes are registers.
- A word that may be a keyword: 78. The multiplier is a register
  (`x19`), as lean's. One load is left: the pair mask's register is
  taken by the constant 8 in the second word's cut.
- A string: 185, of which 104 are its rule's function. After the
  call, `mid` reloads the block's words (ten stack accesses), as
  the lean lexer's outer loop reads its window's. The token after a
  rare one goes through `mid`: 65 for a comma.

Counts, reported (before, after; lean):

| A token | Before | After | Lean |
|---|---|---|---|
| Words | 107.7 | 107.8 | 97.9 |
| Punctuation | 57.5 | 54.5 | 53.5 |
| Mixed | 59.9 | 57.6 | 63.1 |
| Strings | 282.9 | 321.5 | 240.6 |
| Numbers | 162.8 | 187.8 | 107.3 |
| Comments | 261.2 | 301.3 | 193.2 |

The rare kinds rose by about 40 a token: `mid`'s split and the
reloads after each call. Open: their paths against lean's, and the
second word's compare (12 instructions; lean's ninth byte, 3).

## A rare start has its own head

**Intro.** The lean lexer's rare token never runs its run loop's
code. It takes `pos = base + rare.trailing_zeros()`, reads the byte
and `match`es on it. Ours entered the same token block as the run
loop's tokens: the entry, the pair bit, the fused rules' test, then
the rules' jump table. The run loop's head held the `match`, and
the rare path's work could be moved into it.

**The change.** The token's body is made once per head (`Side`):

- the run loop's (`Side::Fused`): the fused rules alone. A token
  not of them is the run rule's past its block, whose block it
  takes alone (`RuleMode::Only`), as lean's `word_end`;
- the rare starts' (`Side::Rare`): no fused rule, the rules' jump
  table, as lean's `match`.

`mid`'s rare start goes to the rare head (`rare_word`), with the
state and the block's words as the run loop's.

The run loop has neither a call nor a `match`
(`the_run_loop_calls_nothing`). Stepped by instruction, a string
and the comma after it: 231 (lean 223). Its rule function is 104
(lean's `string_end`, 124); the loop's part is 127 (lean's, 99).
The rare head is 17 instructions to its call (lean's, 14).

Counts, reported (before, after; lean):

| A token | Before | After | Lean |
|---|---|---|---|
| Words | 107.8 | 106.0 | 97.9 |
| Punctuation | 54.5 | 53.4 | 53.5 |
| Mixed | 57.6 | 56.4 | 63.1 |
| Strings | 321.5 | 306.9 | 240.7 |
| Numbers | 187.8 | 173.7 | 107.3 |
| Comments | 301.3 | 285.6 | 193.2 |

Not taken: sending the run's end straight to the rare start `mid`
had found, with one shared token head. Lean's run loop and its rare
`match` are apart, each with its own head, and a shared head puts
the rare path's checks into the common tokens' code. The heads stay
apart (below).

## The run's end takes the rare start found

**Intro.** The lean lexer finds `rare` before its run. When the run
ends, it takes `rare`'s lowest bit at once (`ldr`, `cbz`, `rbit`,
`clz`). Ours went back to `mid`, which found `rare` and `run` again
from the starts after the run (four loads), found no run, then the
rare start.

**The change.** The run's end (`run_done`) is a block of `mid`'s,
which `mid` comes to with no run: the rare start `mid` found, else
the refill. A fused token whose run is done goes there with the
starts after the run.

Stepped: the run's end is `ldr`, `cbnz`, `rbit`, `clz` into the
rare head, as lean's. A string and its comma: 224 (lean 223).
Counts, reported: words 105.8, punctuation 53.3, mixed 56.2,
strings 305.5, numbers 173.0, comments 285.1.

## A rare token that ends plainly puts no start back

**Intro.** The lean lexer leaves some bytes out of its starts: a
word's inner bytes, and a pair's second byte. After a rare token it
only does `starts &= bits(cursor - base)`. It never puts a start
back, because each `match` arm knows what it read: a string ends at
a quote, and the number arm reads the word after it itself.

Ours put a start back after every rare token. That means the bit
after the token where it is a word's inner byte or a pair's second,
plus the reread flag for chained pairs. About 15 instructions, and
the values it needs stayed live across the reader's call.

**What the compiler knows.** Whether a rule's tokens need a start
put back follows from the grammar. A token needs none when:

- its last byte is no word byte and no pair's first byte, or
- the byte after it is no word byte and no pair's second.

`plain_end` decides this per rule, from the outputs' exact bytes at
an offset (`native_cpu_bytes::output_byte_at`):

- a reader: each arm at its closer, and its `other` rule;
- a last: its output at the token's end;
- a pair rule: its first byte where no second follows, and each
  second.

For `.rr`, the string, the comment and the backtick literal end
plainly. A digit, `#`, a word byte and `(` do not.

```
// The comment's reader, from the grammar:
//   line  = line_open .. lf outside   -> the byte after is `\n`
//   block = block_open .. block_end   -> its last byte is `/`
//   else  `/` alone (a pair rule)     -> `/` is no pair's first
```

**The change.** The rule's fact is a bit of its first byte's entry
(`TOKEN_PLAIN_END`). A rare start's token body branches on it once,
after the entry's load, as the lean lexer's `match` picks an arm.
Each class (`Side::Rare(Some(plain))`) gets its own rule table,
holding only its rules, and its own tail. The plain class's tail
has no put-back.

Not taken: one tail, with a branch on the bit after the join. In
that source both tails' work meets in one block and is picked by a
select, so every rare token does both. Lean's arms each end their
own token. The decision comes before the join, as lean's does.

Also dropped: the rare tail ORed the run's `rest` into its starts.
A rare start's starts are `rest` already (`run_done`'s). The OR
only kept `rest` live across the call.

Stepped, a string and its comma: 216 (lean 223). The starts left
after the string are `lsr; lsl; ldr; cmp; csel` (lean: `cmp; b`,
then the mask). Counts, reported: words 105.8, punctuation 53.2,
mixed 56.2, strings 297.3, numbers 173.0, comments 271.9.

Around the call, the reloads after it (three loads, two constants,
the table's address) are about lean's: its `starts`, its window's
words and the reader's result are in memory too.

## A number puts back a run byte alone

**Intro.** Lean's number arm asks one question after the digits:
is the next byte a word byte (`CLASS[b[end]] & IDENT`)? If so, it
reads that word. Ours still put back both kinds of left-out start
after a number, a run byte and a pair's second, and checked for a
run of pairs across the token's end. That is 14 instructions, with
two words loaded from the stack.

A pair's second is left out only after a pair's first byte. A
number never ends with one: its last byte is a digit.

**What the compiler knows.** A rule's tokens' last bytes
(`last_bytes`), from the grammar:

- a reader's closers at their offset, its opener's bytes where none
  closes, and the rule it goes on to;
- a run's first byte, and the bytes the run can end with;
- a pair's first byte and its seconds.

The bytes a run can end with (`run_last_bytes`) leave out an `Or`
term whose next byte is surely in the run. In `.rr`'s number:

```
mask num_run = digit | (dot & prev(digit) & next(digit))
// a dot has a digit after it: it never ends the run.
// The number's last bytes: digits.
```

Where no last byte is a pair's first (`word_end`), the rule's tokens
take a third form, `TokenEnd::Word`. Its tail puts back the byte
after the token where it is a run byte (`w & bit`), with no reread.
After a last byte that is no run byte, that bit is a start already,
so it changes nothing. The form is the entry's `TOKEN_WORD_END`.
That bit is free on a rare byte, which has no punctuation code. The
rare body branches on it after the plain bit.

For `.rr`, the number and the `#` word take it.

Stepped, a number and its comma: 193 → 187 (lean about 128; the
number's reader is 69 of ours, lean's digit loop is inline). The
tail after the number is 9 instructions: the mask (2), the run byte
put back (4), the block (3). Counts, reported: words 105.8,
punctuation 53.2, mixed 56.2, strings 297.5, numbers 166.6,
comments 272.2.

## A rare start's rule by its first byte

**Intro.** Lean's rare tokens are a `match` on the first byte, and
each arm has its own tail. Ours had three steps:

- load the entry, its index with the pair bit (`TOKEN[c | pair << 8]`);
- test the class bits (plain, word);
- switch on the rule, a second time, inside the class.

**The change.** The rare side's rules are one `rule_last`, with an
exit per class (`TokenEnd`). Each rule's arms go to its class's
exit, and each exit gets its own tail. The rule's block comes from
a table on the first byte, and only rare bytes are in it: no other
byte comes to the rare head (`mid`'s rare word). `clif_rust` writes
a table as a `match` whose entries for one target are one arm, with
ranges:

```
match v968 { 34 => … 35 => … 47 => … 48..=57 => … 96 => … _ => … }
```

A rule's class holds for every first byte that reaches it, so a
reader's `other` rule takes its readers' first bytes too. The class
bits in the entry are gone. A rare byte's entry is the same with or
without the pair bit, so the rare side loads it by its byte alone
(`RecordTables::rare_unpaired`).

Not taken: a table over all 256 bytes, each to its own rule. Lean's
`match` on a rare start has arms for the rare bytes only, and its
other bytes go to one default. A table of every byte would also
send the common bytes, which the run loop has already taken, to
their rules.

Stepped, the rare head: `add …, uxtb #3; ldr` (the entry), then
`cmp #0x2e; b.gt; cmp #0x2f; b.eq; cmp #0x60; …; sub #0x30;
cmp #0xa` to the number's call (lean: `cmp #0x2e; b.gt; cmp #0x22;
b.eq`). Counts, reported: words 105.8, punctuation 53.2, mixed 56.2,
strings 289.6, numbers 166.6, comments 268.2.

## A tagging reader's exit apart

**Intro.** A comment's reader tags its token (`Reader::retag`): the
code's kind and marks, which no first byte gives. The rare class it
shared with the string merged the comment's tag with the string's
"none" (`-1`), so the tail tested for a tag after the join.

**The change.** A rare class is keyed by how its tokens end and by
whether its reader tags (`Side::Rare(Some((end, tagged)))`). In a
tagged class whose readers give one tag, and whose tests all hold at
each token an arm reads (`TokenCheck::always`), the tag and the hits
are the compiler's constants where the tag says an arm read it
(`tag_known`). `.rr`'s comment is such a reader:

```
after comment when count(line_open | block_open) at least 1
// always: the comment's first byte is its opener
```

**What is left.** The marks' merge after a comment is the grammar's:
the mark goes to the token after it (`json_bad`), into the loop's
`pending`. Lean has no such mark. The comment's function also reads
`/` alone (`Reader::other`), so the two met at the tail's `coded`, and
the constants did not reach the merge. That is undone in "Each arm
writes its own token" below.

Tried and dropped: the last and the tag as signed halves of the
returned word, to take each out by one sign extension. A comment's
tag has its marks in its high bits and does not fit an `i32`, so
every rule call went back to four words through memory
(`packs`).

Counts, reported: words 105.8, punctuation 53.2, mixed 56.2,
strings 287.6, numbers 164.6, comments 267.2.

## A string's tests in its read loop

**Intro.** The lean lexer's `string_end` makes its escape test in
the scan that finds the closing quote. Each 64-byte window gives the
quote, the escapes and the test's bits together:

```rust
// lean (micro.rs): one loop, the test ORed in a window at a time
let (esc, out) = escaped(blk.eq(b'\\'), carry);
let quote = blk.eq(b'"') & !esc;
u |= esc & blk.eq(b'u') & below(quote);
```

Ours found the quote first. Then a gate read the token's bytes again
for any byte the tests need, and a second function (`tests_fn`) read
them a third time to count. That form was kept over the loop form
on an instruction count. It is undone here because it is not lean's
shape: three passes over a string's bytes where lean has one.

**The change.** `read_fn` passes the tests' outputs to
`words_carried` with the closer's, so each window makes them all.
A window with no closer goes into the accumulators whole. The
closer's window goes in up to the closer, `x ^ (x - 1)`. A test of
one byte ORs its words, like lean's `u`. A test of two or more
counts them (`popcnt`). The short reader's inline first window
(`RuleMode::Short`) does the same, so there is no gate there either.

```rust
// generated (tier1_aot.rs, the string reader), a window:
v73 = v72 & v21;              // the closer, unescaped
v147 = (v93 | v145) & v21;    // the test: a control byte, or a bad escape
if v73 != 0 {                 // the closer: its window's test bits to it
    v157 = v11 | (v147 & (v73 ^ (v73 - 1)));
    return (close, ((v157 != 0) as u64) << 28);
}
v150 = v11 | v147;            // no closer: all of it, the next window
```

**Its condition (`tests_in_read`).** The loop starts after the
opener, so no test may hold at the opener's byte. The proof is
`native_cpu_bytes::never_both`. Each `Or` term of the test needs a
byte the opener never has there, or needs a value the opener
excludes. `.rr`'s test is `control | (escaped(\) & !valid)`. Its
opener is `" & !escaped(\)`. The byte sets alone can't show it,
since an escaped byte can be any byte. The second term needs
`escaped(\)`, and the opener excludes it. The token's last must also
be the closer's (`off` 0). Otherwise the gate and `tests_fn` stay.

**What is left.** The grammar's test is wider than lean's. Lean
looks for `\u` only, one compare a block. `.rr` marks a string with
any escape that needs unescaping, or a control byte (`After(12)`).
That is a compare, a two-table nibble class, and one more movemask a
window. The tape must be the engine's, so the predicate stays.

Tests: `the_cpus_records_are_the_interpreters` gains a test byte
right after the closer in its window, and the next string's `\u` in
the same window.

Counts, reported: words 105.8, punctuation 53.2, mixed 56.2,
strings 323.3, numbers 164.6, comments 267.2.

## A count test asks what it asks

**Intro.** Lean's number arm asks one question about a `.`: is there
one, with a digit after it? It never counts. rr's grammar asks two
questions at a number: at least one `.` (a float) and at least two
(an error). Ours answered both with one count. Each lane in the set
was a 1, the halves were added, and a multiply summed their bytes.
Then came a compare with each threshold. That form was kept over a
bitmask form because the two cost the same, which is a count's
reason. The tests are predicates, so they are lowered as predicates.

**The change.** Where a token fits one vector (`exact`), each set's
bytes are made by what its tests ask:

- at least one: any lane in the set (`needs_some`), `m != 0`;
- at least two: the lanes' bitmask (`bitmask`), more than one bit
  set, `m & (m - 1) != 0`;
- more than two: the count, as before.

```rust
// generated (tier1_aot.rs, the number's rule), the dots in the token:
v54 = movemask(dot & in_token) & 0xffff;
v56 = v54 & v54.wrapping_sub(1);         // at least two: refused
v60 = v27 | (((v56 != 0) as u64).wrapping_neg());
v64 = v34 | (((v54 != 0) as u64) << 16); // at least one: a float
```

Also, a hit is a 0 or a 1 put in place. `hit_into` shifts it to its
mark's bit, or negates it into the refusal. Before, every hit was
made all ones (`sextend`, `ineg`) and then masked to its bit.
`tests_fn` and the read loop already put a hit in place this way.

Tests: `a_numbers_checks_are_at_its_token` and
`chained_floats_go_to_the_engine` (`1.2.3`, `10.25.5`).

Counts, reported: words 105.8, punctuation 53.2, mixed 56.2,
strings 323.2, numbers 160.6, comments 267.2.

## Each arm writes its own token

**Intro.** Lean's `/` arm has three outcomes: a line comment, a
block comment, or the `/` punctuation. Each one writes its own token,
with its kind known in that branch. Ours had one function for the
class (the comment's reader, and `/` alone as its `other` rule), and
one tail after it. The comment's tag and hits were constants
(`tag_known`), but they were passed into the tail's `coded` block
with the `/`-alone token's run-time values. The marks' merge then
worked on the joined values, and the `/` alone looked itself up in
the word table.

**The change.**

- **A known tag's own tail (`known_tail`).** Where a tagged class's
  tag and hits are the compiler's, its tagged tokens go to a block of
  their own. The tail is built again for that block, with the code
  and the hits as constants made in it. The merge's test
  (`pending | hits`) and the extra's selects then fold.
- **A punctuation's code by its byte, in a rare class.** The lone `/`
  is a pair rule's token (`EndRule::Next`): its code bits are given
  by its first byte and its length, which is how the fused pair
  rule's tokens get theirs (`TOKEN_PUNCT`). `punct_codes` now covers a
  byte whose rule is a pair rule, or whose reader's `other` is one.
  In a tagged rare class whose bytes all have such codes, the token
  that is not tagged takes them: one load and a select, no word
  probe. The codes in the entries (`punct_in_entry`) stay the fused
  rule's alone.
- **No word to look up.** A rare class none of whose first bytes
  starts a word (`TOKEN_WORDY`, by the bytes the rare table sends to
  its rules) gives its code as the kind. That is the string class
  (`"`, `` ` ``).

```rust
// generated (tier1_aot.rs), the `/` arm after its call:
if tag >= 0 {                  // a comment: its own tail
    v1365 = 2147483648;        // its code, the compiler's
    v1366 = 268435456;         // its hits (`json_bad` after it)
    if (pending | v1366) != 0 { /* the merge, its constants folded */ }
} else {                       // `/` alone: its code by byte and length
    v1307 = load(entry + punct);
    v1310 = if len > 1 { v1307 >> 32 } else { v1307 & 0xffff_ffff };
}
```

Tests: `the_cpus_records_are_the_interpreters` gains a `/` alone at
a block's edges, at the file's end, and beside both comments.

Counts, reported: words 105.8, punctuation 53.2, mixed 56.2,
strings 325.2, numbers 161.0, comments 253.2.

## A block's gap marks stay in its words

The lean lexer keeps a window's masks for the whole window. Ours
made a block's gap-mark word (`json_bad`'s form feed, an `eq(12)`
mask) at the refill and then dropped it. Only a flag
(`GAP_FLAG`) stayed.

The next refill needed that word again. A form feed after the last
token's first byte belongs to the next grammar token, so the refill
folds the bytes from there to the block's end into the marks so far.
It did that from the bytes: a loop that called the mask's function
(`marks_in`'s slow path) once per block it covered. That ran on every
refill, in files with no form feed too.

**The change.**

- **The words go to the refill.** The head already has a slot for
  each of the block's words. A token that leaves the block now passes
  its gap marks' words there, where it passed zeros before.
- **The refill folds them.** The last token started in the block the
  loop left. Its bits past that start are `word & (!0 << start)`,
  taken where the start is inside that block and before the block the
  refill reads.
- **Only crossed blocks by the bytes.** The bytes after that block,
  up to the one read now, are blocks a token crossed. Those alone go
  through the slow path, and its range test skips it when there are
  none.
- **A wait keeps them.** A part's head carries no words, so a token
  that waits stores the marks so far with the carried bits already
  in them.

```rust
// generated (tier1_aot.rs), the refill:
v250 = start & 63;
v253 = (v250 != 0) & (start < block_q * 64);   // the start's block, passed
v255 = gap_word & (!0 << v250);                // carried from the head
v260 = pending | (((v255 != 0) & v253) << 12); // json_bad's bit
// then the slow path from the next block, empty unless a token crossed
```

Tests: `the_marks_tests_are_the_masks` gains pads 61 to 63, which put
the last start at its block's end. With the carried bits dropped, the
test fails. No rr test makes a token wait, because each of rr's reads
is from the bytes, so the wait's fold is not tested.

Counts, reported: words 101.1, punctuation 52.9, mixed 55.1,
strings 306.2, numbers 158.0, comments 231.2.

## A word past its block reads a few bytes on

When a word reaches the end of the lean lexer's window, `word_end`
reads on a byte at a time and writes the token at once. A word rarely
goes on past more than a few bytes of the next block.

Ours read the next block's whole word mask for this. That was a call
to the mask's function over 64 bytes, and then the refill made the
same mask again for that block.

**The change.** Where the run's output comes from the bytes, the run
past its register word reads the next block's first 16 bytes with one
vector (`Bytewise::short_word`), as the number rule's run already
does. A run that is still going after those 16 bytes goes on to the
whole words, as before. Blocks stay aligned, and the token is written
where it ends, as lean writes it.

Not done: carrying the open token to the next block's refill, which
would finish it from that refill's own word mask. It would remove the
16-byte read too. But it would move the token's tail out of its block,
past the waits and the gap marks' attribution, and the lean lexer
does not do it.

```rust
// generated (tier1_aot.rs), a word past its block:
v726 = vld1q_u8(bytes + next);           // the next block's first 16
v744 = movemask(word class of v726) & 0xffff;
v747 = (!!v744).trailing_zeros();        // the run's bytes there
if v747 < 16 { last = next + v747 - 1 }  // else the whole words
```

Tests: `the_cpus_records_are_the_interpreters` gains words that go 1,
15, 16, 17 and more bytes past their block, and words that end the
file. With the last byte off by one, the test fails.

Counts, reported: words 96.5, punctuation 53.9, mixed 55.2,
strings 306.1, numbers 158.0, comments 231.2.

## A number's tests from its run's window

The lean lexer's number arm finds its `.` while it reads its digits,
in one pass over the bytes.

Ours read a number's bytes twice. The run (`short_word`) read the
window from the token's second byte, with the `.` lanes inside its
own expression (`digit | (dot & prev digit & next digit)`). Then the
count tests (at least one `.`, at least two) loaded the token's bytes
again from its first byte, compared each for `.` again, and cut the
vector to the token's length.

**The change.**

- **One window for several outputs.** `Bytewise::short_words` makes
  several outputs' 16 bits from one window. An output that is one
  lane's instruction is read in the first output's window, with the
  delays' difference as its lag (`Lanes::get_lagged`), so its loads
  and compares are the first output's.
- **The run's window gives the tests their bits (`run_tests`).** This
  holds for a run rule read from the bytes, and its count tests whose
  output is a byte set at its own byte (`exact`, no delay) that none
  of the rule's first bytes is in. The token's first byte then has
  no bit, and the window from its second byte covers the rest.
- **The cut is the token's length.** In `tested`, such a test's bits
  are the window's, below `len - 1`, a scalar mask. A test that asks
  for more than two takes the popcount.

```rust
// generated (tier1_aot.rs), the number (f38):
v30 = vld1q_u8(bytes + s + 1);
v39 = vceqq_u8(v30, splat(b'.'));        // the run's `dot`
v55 = movemask(digit | (v39 & …));       // the run
v62 = movemask(v39);                     // the tests', the same lanes
…
v87 = v62 & ((1 << (len - 1)) - 1);      // the token's bytes after its first
```

Tests: `the_marks_tests_are_the_masks` and
`a_numbers_checks_are_at_its_token` pass. With the cut one byte too
long, `the_marks_tests_are_the_masks` fails. A lag one byte off
cannot be seen in this grammar: a number's `.` is never its first or
last byte.

Counts, reported: numbers 153.0, the others unchanged.

## A reader knows its first byte

The lean lexer's arms are a `match` on the token's first byte, so an
arm asks only about the bytes after it: `b'/' if d == b'*'`.

Ours chose the rule by the first byte as well (`Records::ends`, the
entry's rule and the rare table both), and the rule's function then
tested that byte again. The string reader checked `s == '"'`. Each
comment arm checked `s == '/'` before its second byte.

**The change.**

- **A rule's first bytes (`first_bytes`).** These are the bytes whose
  end rule is `k`, plus those of a reader whose `other` leads to `k`,
  since a reader goes on to its `other` with the same first byte.
- **Tests of that byte fold (`Bytewise::fold`).** The scalar reader of
  an opener (`bit_first`) is given that set. A byte test of the first
  byte is true where the set is inside the test's, and false where
  the two are apart. A union of byte tests at that byte is decided by
  its whole set. A `prev` folds where the zeros before the file give
  its bound.
- **No operation with a known side.** An `and` with one side known
  true is the other side, and an `or` with one side known false is
  the other side. A `prev` whose bound the zeros give is its operand.
  The opener's `and` starts from its first undecided conjunct, not
  from a constant 1.

```rust
// generated (tier1_aot.rs), the comment reader (f36), its `//` arm:
v55 = *(bytes + s + 1);                  // the byte after the `/`
if v55 == b'/' { /* the line's end */ }
// the string reader (f31): no `"` test, the escape before it alone
v79 = *(bytes + s - 1);
if v79 == b'\\' { return f30(…) }
```

Tests: `a_first_byte_decides_its_tests` checks the fold: decided,
undecided, another byte, and a `prev`'s bound. In rr, every reader's
arms start with the same byte, so the rr tests cannot tell a fold
that is too wide.

Counts, reported: strings 301.1, comments 228.3, the others
unchanged.

## A rule's function takes what it reads

A rare token's rule is a call, by the rule that only the main loop is
inlined. Every rule's function took the same six arguments: the
bytes, the file's length, its blocks, the token's first byte, its
first eight bytes and its entry. The main loop moved all six into
registers at each call.

**The change.** The arguments are in the order the rules read them
(`RULE_ARGS`): the bytes, the token's first byte, the length, the
blocks, the first eight bytes, the entry. A function takes the first
of them up to the last it reads (`rule_reads`), counted in its blocks
reached from its entry. It is built with all six, then built again
with those. A call passes as many as the callee's signature has
(`rule_args`), and a short function's bail passes as many as its long
function takes.

What each takes in rr:

| Function | Reads | Takes, was 6 |
|---|---|---|
| the string (`f31`) | bytes, first byte, length | 3 |
| a pair rule's, a run's (`f33`, `f40`) | and the blocks | 4 |
| the number, the comment (`f38`, `f36`) | all six | 6 |

The number's function reads its first eight bytes and its entry too
(its tests' bits by its first byte), so it keeps all six. Order is a
prefix, not a set: rule 3 reads no bytes and no blocks, and the
comment's long function no first eight bytes, but they take them.

Not changed: the values the main loop reloads after a call. They are
live across it, more than the callee-saved registers hold.

Counts, reported: unchanged but strings 302.1 and comments 228.2.
