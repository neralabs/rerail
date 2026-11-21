# 0552: A grammar states what the token loop knows, and the compiler checks it

Status: decided (the user, 2026-11-21: "please do 1 / to make it easier for the compiler, we can also add
information in the bitlex grammar about invariants that the grammar writer knows to be true, but they do have
to be checked in some manner", then "go ahead with 1 + 2 + 3, but be careful about the cost of the proofs and
make sure that they are cached"). Follows 0414 (a grammar's proofs are answers by content) and 0551 (tier 1's
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

- **The statement.** `assume "WHY" at starts NAME = E`: at every byte a token starts at, mask `NAME` is `E`. It
  changes no mask, so the executors that make every mask at once read nothing new (the evaluator skips it).
- **What tier 1 makes of it.** Where a token's start reads `NAME` (`at_start`), it reads `E` there. `E` reads no
  region's mask (the compiler refuses one that does). A mask made of the regions that the token loop reads only
  at starts then no longer needs the regions' walk.
- **How each is checked** (the user's 1 + 2 + 3):
  1. **Proved.** The obligation is a mask question: `(NAME ^ E) & starts & ~skip` is zero everywhere, where
     `starts` is the union of the rows the loop reads (not a region's parts). The loop stands nowhere else: a byte
     no row starts at refuses. `Lowering::either_never` asks it (the bounded search, then induction, with the
     walk's equations), and the answer is cached by the question's content as every proof is (0414): shipped in
     the grammar's answers (`kotlin.answers`), so a compile with them proves nothing again.
  2. **Checked by the tests.** One the prover does not prove is checked at every token's start in a test build
     (`#[cfg(feature = "test-hooks")]`): its mask over the file against `E` there. The grammar's tests and its
     corpus check it; a product build trusts what they checked.
  3. **A refusal.** Where a test at the token decides a fact, it can be a refusal instead (as 0551's step B
     makes the regions' escapes), sound in every build.
- **The cost.** One question an assumption, all of them asked with one prover (its settings fixed: horizon 8,
  100,000 conflicts). Kotlin's first four, cold: about 14 s of the compile; with the shipped answers, none.
  `RERAIL_ASSUMPTIONS=1` prints each and whether it was proved.

## Kotlin

The first four, all proved: a name starts at a letter (`word_start = alpha`), and a decimal, hex or binary
number at its digit not after one `.` (`int_at`, `hex_at`, `bin_at`). Tier 1's starts of those rows are now
stage 1's masks at the token.

## Why not

- **The compiler finding the facts itself**: each depends on how every other row ends its tokens, which is a
  proof about the token loop, not about a mask. The writer knows them; the prover can check a stated one far
  more cheaply than search for it.
- **Trusting an unchecked statement**: a wrong one would make tier 1 read a file differently from every other
  executor, silently.
- **Only proved or refusing facts (1 and 3)**: sound in every build, but the facts the prover can not reach yet
  (the semicolon's fills, `as_run`) would keep the regions' walk in tier 1. Test-checked ones let the walk go,
  and the prover can take them over as it grows.
