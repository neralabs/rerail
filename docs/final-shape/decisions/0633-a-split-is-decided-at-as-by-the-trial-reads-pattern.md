# 0633: A split is decided at `as` by the trial read's pattern

Status: decided (2026-11-23, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0630, 0632.

## Intro

**The problem.** After `x as a.B<C>` at a line's end, the lean parser splits the type only where its trial
read of the next line (`split_operand_reads`, `expr(prec::COMPARISON)`) reaches the line's end. Where the
trial stops or fails, it keeps the type whole. After 0632:

- The grammar split on the next line's first token alone.
- Masks (`as_sline`, `as_in1`, `as_k`, `as_gen`, `as_cont` and their escapes) refused the lines where an
  operator might stop the trial. Those masks count one level of brackets and stop at a line's end.
- So these were refused: `a < b`, `f(g(x)) == b`, `f(\n  a\n) == b`, `a < b > c`, `f<T>(a) == b`, and a
  line that continues on the next (`a\n  .b()`).
- Keyword lines whose trial fails were split and then refused (`class D`, `for (i in l) g()`).
- `a(\n  b, c\n)\n d()` read differently from lean: kept whole, where lean splits.
- Outside splits, a spaced generic call read differently: lean reads `val x = a < b > (c)` as a call
  (`type_args_end` asks nothing of blanks), the grammar as a comparison. `f <T>()` was refused.

## The decision

- **A run takes a bracket's contents whole.** `nest (O to C whole)`, as the lean parsers read a
  bracket's contents in full: inside that depth, any token is the run's. The run's choices and marks are
  asked only where the depth is 0. Each executor has it:
  - the program: `TokenElem::whole`, the counters' bits, written `whole{hex}`; `program::tests_at`;
  - the interpreter: `token_follows`;
  - the JIT: `pat_follow`, a branch over the depths;
  - the tier-2 Rust writer: `depth[d] > 0 || …`;
  - Bitlex's evaluator: `seq_holds`;
  - the validator: a whole depth is one the run counts.
- **A token mark takes several `before`s.** `tokens S, before S2, …`: any one of them holds it, as every
  executor already ORs a mark's entries. Bitlex's evaluator now does too. This keeps one mark for each
  split head: the program holds 64 marks at most.
- **The split, positively.** `as_split` and `is_split` hold where the next line is:
  - a call's `(` or `{`; or
  - an operand's first token, then a run of tokens of no operator the trial stops at, with brackets
    whole. The run crosses a line break that continues the statement (`without nl_end`). It may hold at
    most one generic call (`<` type arguments `>` then `(` or `{`, to its closer). Then a `}`, a `;` or the
    next statement's first token (`with nl_end`).
- **The keep patterns.** `as_open`'s `unless before`s and the split marks' are the lines where the trial
  stops or fails:
  - an operand's first token, then tokens of no `<`, `>` or word whose operand may take the operator,
    brackets whole, to an assignment's or an equality's operator, `<=`, `>`, `&&`, `||`, `:` or `,`;
  - a `<` whose type arguments `type_args_end` refuses before a `>`, or with no `(`, `{`, `.`, `?` or `::`
    after their `>`;
  - a generic call, then such an operator;
  - an infix name after a closer, its operand `()` (`for (i in l) g()`);
  - `class D` or `interface I` before a `}`.

  `as_kwn` (a keyword as a name) holds only where the line splits. Where neither the split nor a keep
  pattern holds, `as_open` holds and no rule takes it, so the file is refused. No line is read
  differently.
- **`nl_end`**: the first token on its line where the lean parser's scanner ends a statement, a mask
  mark. It is not before the tokens `semi`'s `unless before` continues a line with (`.`, `?`, `:`, `(`,
  `{`, operators, `as`, `else`, …).
- **The line masks and their three escapes are removed.**
- **Two fixes in the Rust writers:**
  - Tier 2 (`bitlex_rust.rs`) ANDed a token mark's run with its other `before`s. They are alternatives,
    as in every other executor.
  - Tier 1 (`bitlex_lex_rust.rs`): a gap mark `A & prev(fill(M, ~W))` assumed that `A` holds at every
    start (true of `own_line`'s `solid`). It now tests `A`'s conjuncts that read no region (`nl_end`'s
    tokens); one that reads the regions still holds at every start.
- **`generic`**: no `after simple_identifier`, no `without apart`, and `&` among its tokens, as
  `type_args_end` reads them.

## Why

The criterion is the lean parser's `split_type`, `split_operand_reads`, `expr_in`, `type_args_end` and
`generic_suffix`.

- Each pattern is a subset of what the trial does. A pattern that held where the trial stops would split
  where lean does not, so the patterns are conservative, and the rest is refused.
- The decision at `as` needs the bracket's contents whole, as the trial reads them: the token marks
  counted depths but tested every token at every depth.
- A mask counts no depth.

Left refused:

- a split's line that starts with a string or a character, which a token mark does not name;
- a line with a word whose operand may take the stopping operator (`if (c) d == e`);
- two generic calls on the line;
- a navigation after a generic call's type arguments (`a<B>.c`);
- an intersection type among type arguments (`f<A & B>()`): the grammar's types have none.

## What changed

- **Bitlex:**
  - `syntax.rs`: `Nest::whole`, `(O to C whole)`, a token mark's `before`s;
  - `eval.rs`: whole depths, a token mark's `before`s.
- **The compiler:**
  - `program.rs`: `TokenElem::whole`, `tests_at`;
  - `rir_text.rs` and `rir_asm.rs`: `whole{hex}`;
  - `stream.rs`: the interpreter and the validator;
  - `native_cpu_stream.rs`: the JIT;
  - `bitlex_rust.rs`: the Rust tier 2, whole depths and a token mark's `before`s;
  - `bitlex_lex_rust.rs`: the tier-1 gap mark's start test;
  - `bitlex_tier2.rs` and `bitlex_masks.rs`: the lowering.
- **`kotlin.bitlex`:**
  - the split marks, `as_open`, `as_kwn`, `nl_end` and `generic`;
  - the line masks and their escapes removed;
  - `tier1.rs`, `tier2.rs` and `kotlin.answers` regenerated.
- **Tests:**
  - `split_lines_are_the_lean_parsers`: every split line that was refused, and more;
  - `spaced_generic_calls_are_the_lean_parsers`;
  - the refused list: a string first;
  - `a_run_counts_its_depths` and `a_mark_is_before_a_token`: whole depths.
