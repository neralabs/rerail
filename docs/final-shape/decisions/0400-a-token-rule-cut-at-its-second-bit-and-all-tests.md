# 0400: A token's run cut at a mask's second bit (`once`), and `all` tests

Status: accepted (2026-11-18). Extends 0177 ("A number's checks at its token").

## Intro

Step 22 of the `.rr` ladder sent two kinds of number to the engine on purpose:
- **A chain of floats:** `1.2.3` is a syntax error in a clause, but a setting reads it as the
  engine does: `1.2`, `.`, `3`. And `1.2.3.4` is `1.2`, `.`, `3.4`.
- **A float whose integer part has 19 digits or more** (`12345678901234567890.5`). Only an
  integer of 19 digits or more is the engine's to check (an i64). A float's digits are its text.

Both escapes were 0177's token tests (`count(dot) at least 2`, `run(digit) at least 19`). They
are now read in tier 1, and the CPU's tier 1 still has no block loop (no body).

## What does not work: the masks

Which dots in a chain are a float's is a parity: the first, the third, and so on. No byte's
neighbours give it. Written as masks (`toggle` over the chain's dots, against the parity at its
first digit), tier 1 parses both cases, but the masks bring the block loop back. On the M1 Max
(quiet run, `rr_against_rust`, 2,004 files), tier 1 went from 113–127 M cycles to 203 M, with
twice the instructions (396 M to 781 M). That is too much for a spelling the corpus never has.

## The rule: the token loop cuts the run

A lexer reads this one token at a time. It reads a number's digits and one `.`, and it stops
at the next `.`. That `.` is then a punctuation token, and the digits after it start a new
number. The token rule says this:

```text
mask num_run = digit | (dot & prev(digit) & next(digit))
token number = start digit & int & ~prev(int) run num_run once dot
```

`run R once D`: a run of `R` with at most one bit of `D` in it. The second bit of `D` ends the
token before it.

- **The loops** (the interpreter's `records::walk`, the CPU's token loop) keep the run they
  have. The rule also gets one more token test, a count of `D` at least 2 that cuts instead of
  refusing (`CheckThen::Cut`). Each loop makes the cut before the rule's other tests, so those
  tests see the token the cut leaves: `float_after`'s `count(dot) at least 1`, and the
  integer check below.
- **On the CPU** (`native_cpu_block::cut_last`): the token's words of `D`, from its first
  byte, popcounted a word at a time. Where the second bit falls, `ctz` after clearing the
  lowest bit gives it, and the token's last is the byte before. The next token starts after
  that last, as after any token, so the `.` and the digits after it are found as starts.
- **The mask form** (`TokenRule::last`, which the bit-parallel form, Metal and the reference
  read) cuts each token's run at its second `D`. Two fills, stopped at every start, keep it to
  the token:

  ```text
  t = s | fill(prev(s) & R, ~R)                  # the run
  d = t & D & ~s;   one = fill(prev(d) & ~s, ~t | s)   # past the token's first D
  t' = t & ~fill(d & one, ~t | s);   last = t' & ~next(t' & ~s)
  ```

  The tokens' starts still come from the parity masks (`f_dot`, `frac`, `int`). The CPU makes
  no start mask (0177, stage 2a), so the parity is only in the bit-parallel form.
- **A guard:** with the lean lexer's starts (`lean_starts`), a run byte after a run byte is no
  start. So a cut is taken only where `D`'s bytes are no run byte, and are bytes alone. Then the
  byte the token ends before is always a start. A reader's other rule has no tests, so a kind
  that is cut and also starts at a reader's byte is refused when the grammar is lowered.

## `all(E) at least N`

The integer check is a new token test: `E` at every byte of a token at least `N` bytes long.

```text
escape "a number of 19 digits or more" at number when all(digit) at least 19
```

A run of 19 digits from the first byte also holds for a float with a long integer part. A
whole token of digits is an integer. On the CPU, the test is the count's word loop with
`popcount == len` as its hit. It is the same on Metal's record kernel and in the interpreter
(`records::check_holds`). A mark's test may not be `all` yet: the lowering refuses one.

## Found on the way

The rows of a tokened pass with a body (the token loop between the blocks) dropped the token
loop's refusals. The row's end stored the totals' flag over the one the loop's parts had ORed
into memory. It is now ORed, and zeroed at the row's start. The shape check also learned the
blocks such a body leaves to (`PassShape::exits`). These came up while the masks' version had
a body. They are kept, because any tokened pass with a body needs them.

## Measured

M1 Max, quiet run, `rr_against_rust` (2,004 files, 19.09 MB, the least of 7 rounds):

| Tier 1 | Instructions | Cycles | Lean lexer, same run |
|---|---|---|---|
| Before (the two escapes) | 393.5–396.2 M | 112.7–127.3 M | 73.4–73.6 M |
| The parity masks, with a body | 780.5 M | 202.6 M | 73.4 M |
| The cut in the loop (this) | 398.1 M | 105.5 M | 66.3 M |

The last run's host ran about 10% faster on every row (the lean lexer's 66.3 M cycles against
73.4 M), so its cycles are compared by their ratio to the lean lexer's: 1.59, within the
earlier 1.53–1.73. Instructions are +0.5–1%.

## Tests

- `chained_floats_read_as_the_engine`: chains in settings take the engine's tape, and chains
  in clauses are refused by both.
- `long_numbers_go_to_the_engine`: a float of 19 digits before its `.` is taken.
- `a_numbers_checks_are_at_its_token`: the interpreter, the CPU and Metal agree on each side of
  a block's edge.
- `the_cpu_reads_local_tests_at_the_token`: the CPU's body makes nothing.
- `rr_cases_parse_from_their_steps`: the case `.scope 1.2.3.4.5`.
- `rr_steps_parse_as_the_reference_and_the_engine`: the ladder's documents on every path.
