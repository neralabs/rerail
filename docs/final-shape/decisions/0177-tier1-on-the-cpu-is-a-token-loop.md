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
