# 0650: Kotlin's `$$` strings; a question within its share; closers over their own bytes

Status: decided (2026-11-23, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0636 and 0646.

## Intro

**The problem.** A Kotlin string may have a prefix of `$`s (`$$"…"`, `$$"""…"""`). Its templates
then take as many `$`s as the prefix: `$$name` and `$${…}`. A `$` alone, or a run too short for a
template, is content. The lean parser's string reader counts each run of `$`s against the prefix.
The grammar refused every such string (0636): the last refused file of the 1,266-file corpus has
one.

Reading them needs a region per prefix. That doubles the string regions, from 4 to 8, and the parts'
rows with them. Three things then failed, none of them in the grammar:

- **The budget.** One token-pair question, an induction of up to 100,000 conflicts a solve, took a
  whole compile's budget (0646). Every question after it was answered not proved.
- **Pair questions.** Many were contradictions by construction, yet each paid for a SAT window: a
  region's content row is its body less every part's bytes, so it never starts where a part does.
- **The generated walk.** It could not write a template name's mask inside a `$$"` string, so it
  wrote the mask program's value, which only a test build computes. A product build of
  `kotlin_aot` did not compile; the tests, built with `test-hooks`, did not see it.

## The decision

- **The grammar** (`kotlin.bitlex`):
  - The regions `str2`, `rstr2`, `tail2` and `rtail2`: a string whose `"` follows `$$` (`q2`), its
    template closer `t2_dollar` (`$${`, 2 bytes), and their tails.
  - In such a string (`s2`, a `$` of its body), a run of `$`s is counted against the prefix: `id2`
    is a name template's two `$`s, `extra2` each `$` before a template's two, and any other run one
    content token (`d2_first` to `d2_last`). Runs of up to four are told apart, as in 0636.
  - The prefix is a token of its own (`string_prefix`), before the `"` in `string_literal`, and
    implied. A prefix of three or more `$`s is refused.
- **A question within its share** (`bitlex_prove::Budget`): a question takes at most a 16th of its
  compile's budget. Past it, that question is not proved and the others still are. The share is of
  the budget the compile started with, so `RERAIL_PROVING_STEPS` scales it, and unlimited stays so.
- **Questions answered by their shape** (`bitlex_prove::never_by_shape`), before any window: a value
  is never set where one of its conjuncts (through `And`, `AndNot` and `Prev`) is a disjunct
  (through `Or` and `Prev`) of a negated one at the same byte, or where two of them are fills that
  `regions_apart` proved apart. A conjunct `prev(x, k)` that holds says `x` holds `k` bytes back,
  in the stream, so each is an operation at a lag.
- **Each closer over its own bytes** (`bitlex_lex_rust::fits_closer`): a value at byte `r` of a
  region's closer is checked against the closers that have more than `r` bytes, each from its own
  first byte. `str2` closes at `"` (one byte) or at `$${` (two): the byte after a `"` is no
  closer's, and may be a name's letter.
- **The walk's fallback refused**: a part's mask that reads the regions, and that the walk cannot
  write in each of them, is now an error that names it. It was the mask program's value, which a
  product build does not have.
- **A resumed string clears its own bytes of `code_l`**: where the rows' pick is not fused, the loop
  clears a region token's bytes after the row, and a tail's row (which resumes its string's record)
  leaves by its own `continue`, past that clear. With eight string kinds the pick is no longer fused,
  so after `"${b}"`'s `}` the walk read the rest of the string as code; a test build of `kotlin_aot`
  caught it (17 tests). The resume now clears its bytes itself.

## Why

The lean parser's reader is the criterion, and it counts a run of `$`s against the string's prefix.
A region per prefix keeps that count in the region, where the templates are told apart already.

The share keeps one hard question from deciding the rest. The shape check answers what the rows'
construction already says, with no window. Both leave the answers unchanged: the same token pairs go
to the fallback as before (2). Over a cold Kotlin compile with the budget lifted, the token pairs
took 9.7M steps before and 6.7M after.

The cold compile is still over the budget: its gap claims and `row_modes` take far more than 2^20
steps, as before this decision. So the bless lifts the budget, as 0646's did, and a warm compile
answers from the shipped answers. Note t55's region machine is the design that brings a cold
compile within the budget.

## What changed

- **`kotlin.bitlex`**: the regions, the masks `t2_dollar`, `q2`, `pre_at`, `s1`, `s2`, `id2`,
  `id2l`, `extra2`, `d_run2`, `d2_first`, `d2_last`, `id_any`, `id_name`, `rbody2` and `body2`; the
  parts; `string_prefix`; `string_literal`.
- **The compiler**: `Budget`'s share and `never_by_shape` (`bitlex_prove.rs`); the fills proved apart
  kept (`Lowering::apart`, `bitlex_masks.rs`); `fits_closer`, the refused fallback and the resume's
  `code_l` clear (`bitlex_lex_rust.rs`).
- **Regenerated**: `kotlin.answers` (46 KB to 120 KB), and `kotlin_aot`'s `tier1.rs` and
  `tier2.rs`.
- **Tests**: `prefixed_strings_are_the_lean_parsers`; `values_never_by_their_shape` and
  `a_question_takes_its_share` (`bitlex_prove.rs`).

## Results

- **The corpus** (1,266 files, a farm Mac, tests' features): 1,252 the same as the lean parser, 0
  refused, 0 differ, in the interpreter and in the parser as Rust; tier 1's records differ in 0. The
  lean parser refuses the other 14. Before, one file (its `$$"` string) was refused.
- **The tests** (a Linux host): `rerail-kotlin-parser`, `rerail-kotlin-aot` and the compiler's
  bitlex tests, with `kotlin_aot`'s checks on: 99 run, 99 passed.
- **The proofs**: the token pairs of a cold compile, 9.7M steps before, 6.7M after; the answers
  unchanged by the share and the shape check.
