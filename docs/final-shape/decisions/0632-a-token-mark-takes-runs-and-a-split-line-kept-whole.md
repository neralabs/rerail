# 0632: A token mark takes runs, and a split's line is kept whole

Status: decided (2026-11-23, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0416 (a token mark's run), 0630.

## Intro

**The problem.** 0630 left refused the lines after a split type where the lean parser's trial read stops
on the line and keeps the type whole: `a = b`, `a.b = c`, `a == b`, `f(a) == b`. The grammar decides
the split at `as`, by the token mark `as_split`. Its one run of tokens was the type's, and a token mark
held one run at most. So it could not look on to the next line's operator. Every executor took one
run a mark (`stream.rs`'s validator, the JIT's `token_pattern`), and a run's tokens were kinds and
words alone, with no `without semi` to stay on a line.

## The decision

- **A token mark takes runs.** Any number, never first or last, and none right after another (the
  second's place would be no token). Everywhere they are tried as before, as nested forward scans:
  the elements after a run from each of its records on, its depths 0, its `ending` held.
  - `eval.rs`'s `seq_holds` and `stream::token_follows` already recursed.
  - The JIT's `token_pattern` now builds them by `pat_follow`, which recurses. A run's scan holds the
    next run's, and the inner scan's failure steps the outer run.
  - The Rust tier 2's chained functions call the next element's from a run's loop. A run there needs
    nothing new.
  - `stream::validate` takes them.
- **A run's tokens take `with M` and `without M`**, after their choices: `(~C without semi)*`, a run on
  one line, as the lean parsers' scans stop at `peek_at(j).nl`. The interpreter's, the JIT's and the
  Rust writer's element tests already test a `~` and its marks together. `eval.rs`'s and tier 2's
  lowering take them.
- **The keep pattern.** An operand's first token, then a run of tokens on its line, with brackets at
  depth 0 (`nest`). The run takes no `<`, `>`, `->`, `@`, `;` and none of `if`, `when`, `try`, `return`,
  `throw`, `as`, `is`, `fun` or `object`. Then an assignment's operator, an equality's, `<=`, `&&` or
  `||`. The lean parser's trial read of the operand at comparison precedence stops there, before the
  line's end, every time. It is:
  - each split mark's `unless before`, after its type's run: the type stays whole;
  - `as_open`'s `unless before`, so the line is a statement and nothing refuses it. A mark's `with
    as_open` reads the code's bit, the mask's, which no `before` changes: both are needed.
  - `as_k`, the masks' copy, which keeps 0630's escapes off such a line. It is a subset of the
    pattern: one level of brackets, as the masks do not count depth. Where the pattern holds and
    `as_k` does not, the escapes refuse, as before.

## Why

The criterion is the lean parser's `split_type` and `split_operand_reads`. A pattern that holds where
the trial would reach the line's end would read the line differently. The words and tokens the run
takes none of are the ones whose operand may take the operator (an `if`'s branch, a `return`'s value)
or that a type or a generic call may hold (`<`, `>`, `as`, `is`). Where one is on the line, the line
stays refused, by 0630's escapes.

Left refused:

- a `<` or `>` on the line (`a < b`, `f(a) < b`): the lean parser's `generic_suffix` may take them.
- nested brackets before the operator (`f(g(x)) == b`), which `as_k` does not count.
- a word whose operand may take the operator before it, and keyword lines whose trial fails
  (`class D`, `for (i in l) g()`).

## What changed

- `syntax.rs`: runs, a run's `with` and `without`.
- `eval.rs`, `bitlex_tier2.rs`: a run's `~` with its marks.
- `stream.rs`: the validator.
- `native_cpu_stream.rs`: `Pat`, `pat_code`, `pat_after` and `pat_follow`, `token_pattern` over them.
- `program.rs`: the run's doc.
- `kotlin.bitlex`:
  - the split marks' and `as_open`'s `unless before`;
  - `as_dng`, `as_unsafe` and `as_k`, and the escapes that read `as_k`.
  - `tier1.rs`, `tier2.rs` and `kotlin.answers` regenerated.
- Tests:
  - `split_lines_are_the_lean_parsers`: assignments, equalities, `&&`, `||`, after a call, an index, a
    lambda;
  - `as_splits_are_the_lean_parsers`: `a = b` read;
  - the refused list: a `<` after brackets, nested brackets.
