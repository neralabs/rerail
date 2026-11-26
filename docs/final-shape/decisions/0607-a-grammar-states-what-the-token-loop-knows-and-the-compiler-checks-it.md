# 0607: A grammar states what the token loop knows, and the compiler checks it

Numbered 0552 where it was written (Swift-Kotlin-Scala-support); 0607 at its merge into Parser-generators-final-shape, whose 0552 is another.

Status: decided (the user, 2026-11-21: "please do 1 / to make it easier for the compiler, we can also add
information in the bitlex grammar about invariants that the grammar writer knows to be true, but they do have
to be checked in some manner", then "go ahead with 1 + 2 + 3, but be careful about the cost of the proofs and
make sure that they are cached"). Follows 0414 (a grammar's proofs are answers by content) and 0556 (tier 1's
masks one pass over the blocks); step C of the second convergence step (regions read in the token loop).

## Intro

**The problem.** A Bitlex grammar says its tokens as masks over the whole file, so every executor can make them
at once: the GPU's, the JIT's, the interpreter's. Kotlin's name starts where `word_run & ~prev(word_run)`
holds, and `word_run` is a fill over `code`, the bytes outside the strings and comments. The lean lexer reads
one token at a time instead: at a token's start, a letter starts a name. It need not ask whether the byte before
is a word's, or whether it is in a string: the token before ended there, and a string is read where it opens.

The generated tier 1 (`bitlex_lex_rust`) is meant to read as the lean lexer does. To write `alpha` where the
grammar says `word_start`, the compiler must know that the two agree wherever the token loop stands. Most such
facts it can not find alone: each holds because of how the other rows end their tokens.

**What is wanted.** The grammar's writer states the fact; the compiler uses it, and checks it.

## The decision

- **The statements.** Neither changes a mask, so the executors that make every mask at once read nothing new
  (the evaluator skips them).
  - `assume "WHY" at starts NAME = E`: where the token loop reads `NAME` at a token's start, it is `E`. The loop
    reads it at the first row whose start reads it, where no row before that one starts (a later row reading it
    is reached at fewer bytes).
  - `assume "WHY" in KIND NAME = E`: over the tokens of `KIND`, the bytes their run rule reads (each byte after
    the start the rule holds at, and the byte after the run, where it stops), `NAME` is `E`.
- **What tier 1 makes of them.** Where a token's start reads `NAME` (`at_start`), it reads `E` there. A row of
  `KIND` with a run reads its rule with the kind's assumptions: each mask the rule reads that reads `NAME`
  (through the masks it is made of) is written out with `E` for `NAME`, and the rest kept by name
  (`substitute`). `E` reads no region's mask, and the rewritten rule is used only where it reads none. A mask
  made of the regions that the token loop reads only at starts and runs then no longer needs the regions' walk.
- **How each is checked** (the user's 1 + 2 + 3):
  1. **Proved.** Each obligation is a mask question, that a mask is zero everywhere:
     - at starts: `(NAME ^ E) & start_i & ~start_0 & … & ~start_(i-1) & ~skip`, row `i` the first that reads
       `NAME`, over the rows the loop reads (not a region's parts). The loop stands nowhere else: a byte no row
       starts at refuses.
     - in a kind: for each row of `KIND`, with `S` its start, `R` its rule and `R′` the rule as tier 1 reads it,
       `(R ^ R′) & (RUN | END)`, where `RUN = fill(prev(S) & R, ~R)` and `END = prev(S | RUN) & ~RUN`.
     `Lowering::either_never` asks it (the bounded search, then induction, with the walk's equations), and the
     answer is cached by the question's content as every proof is (0414): shipped in the grammar's answers
     (`kotlin.answers`), so a compile with them proves nothing again.
  2. **Checked by the tests.** One the prover does not prove is checked in a test build
     (`#[cfg(feature = "test-hooks")]`): an at-starts one in the condition of the first row that reads it, its
     mask over the file against `E` there; an in-kind one in its row, the run's end by the rule over the file
     against the run tier 1 reads. The grammar's tests and its corpus check it; a product build trusts what they
     checked.
  3. **A refusal.** Where a test at the token decides a fact, it can be a refusal instead (as 0556's step B
     makes the regions' escapes), sound in every build.
- **The cost.** One question an at-starts assumption, one a row of an assumed kind, all of them asked with one
  prover (its settings fixed: horizon 8, 100,000 conflicts). Kotlin's first four, cold: about 14 s of the
  compile; with the shipped answers, none.
  `RERAIL_ASSUMPTIONS=1` prints each and whether it was proved.

## Kotlin

At starts, all proved: a name starts at a letter (`word_start = alpha`); a decimal, hex or binary number at its
digit not after one `.` (`int_at`, `hex_at`, `bin_at`); a real number at a `.` before a digit (`dot_at`); and
an operator at its first byte, not a pair's second (`p_start`: a comment's `/` is taken by the comment rows
before it). Tier 1's starts of those rows are now stage 1's masks at the token.

In kinds, checked by the tests (the prover does not reach them yet): a name runs over its word's bytes
(`name_more = word` in `simple_identifier`), and a number's bytes are code (`code = ~empty` in the four number
kinds), so their runs (`nb`, `hx`, `bx`) are stage 1's masks too.

## Why not

- **The compiler finding the facts itself**: each depends on how every other row ends its tokens, which is a
  proof about the token loop, not about a mask. The writer knows them; the prover can check a stated one far
  more cheaply than search for it.
- **Trusting an unchecked statement**: a wrong one would make tier 1 read a file differently from every other
  executor, silently.
- **Only proved or refusing facts (1 and 3)**: sound in every build, but the facts the prover can not reach yet
  (the semicolon's fills, `as_run`) would keep the regions' walk in tier 1. Test-checked ones let the walk go,
  and the prover can take them over as it grows.
