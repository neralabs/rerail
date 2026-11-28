# 0630: A split type's line, as the lean parser's trial reads it

Status: decided (2026-11-22, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0629.

## Intro

**The problem.** After `x as a.B<C>` at a line's end, the lean parser splits the type only where a trial
read of the next line reaches the line's end (`split_operand_reads`). The trial reads it as a comparison's
operand, `expr(prec::COMPARISON)`. In it, a keyword no expression starts with is a name (`kw_name`:
`for`, `while`, `do`, `class`, `interface`, `typealias`, `package`). Elsewhere it keeps the type whole,
and the line is a statement. The grammar split wherever the line's first token could start an operand.
So:

- `a < b`, `a == b`, `a && b`, `-a < b` and `f(a) == b` read differently from the lean parser (a
  comparison over the split, where the lean parser has two statements);
- `for (i in l) {}`, `while (c) {}` and `do {} while (c)` were refused, where the lean parser reads a
  call (`for(i in l) {}`) or an infix operation (`do {} while (c)`).

## The decision

- **Keywords as names.**
  - `as_split`'s next tokens take the seven keywords.
  - The line's first token, one of them, is a name (`as_kwn`, a token mark `with as_open`; `kw_name`,
    one `simple_identifier` leaf).
  - In a comparison's operand, such a keyword on the operand's line is an infix operation's name
    (`as_kwi`, `without semi`; `kw_infix_op`). A comparison's operand meets those keywords nowhere
    else in a file the grammar reads.
- **The operators the trial stops at, refused.** The grammar decides the split at `as`, before the
  line. Its token marks hold one run of tokens, and `as_split`'s is the type's, so the line's operators
  are not known there. Where the trial may stop, the file is refused:
  - The split's line (`as_sline`): from the next line's first token where `as_open` holds, to its end.
  - An operator there (`<` after a blank, `>` but a `->`'s, `=`, `&`, `|`) outside brackets: refused.
    `as_in1`, from an opener to the next closer, is surely inside brackets; elsewhere it may not be.
  - A `<` touching a name: a generic call's where a `>` that a `(` or `{` follows closes it on the line
    (`as_gen`), as the grammar's `generic` reads it. A line that ends inside one: refused.
  - A line left open (a bracket open at its end): the trial reads on. The first line that starts with
    a closer (`as_cline`) is checked the same way.

- **The interpreter, a fix.** Where tier 2 reads a string in parts, `stream::run` writes the parts'
  codes by `records::pass` over the parts alone. Its `gap_marks` started the first part's gap at the
  file's start, so a string's opener had every gap mark that held anywhere before it. After a split,
  `f("a")` was refused at the `"` by `as_open`, where the JIT read it. The opener's gap marks are now
  its record's, as the JIT's and the Rust tiers' are.

## Why

The criterion is the lean parser's `split_type`, `as_split`, `split_operand_reads` and `kw_name`.
Each line the lean parser splits that the grammar reads is the same. The ones where its trial stops
(the type whole) are refused, so no line reads differently.

Left refused:

- the lines the lean parser keeps whole (`a < b`, `a = b`, `class D`, `for (i in l) g()`). Reading them
  needs the decision at `as` to know the next line, a second run in `as_split`. The program format, its
  validator and every executor take one run a token mark (`stream.rs`, `native_cpu_stream.rs`).
- a `<` after a blank that the lean parser reads as a generic call (`a < b > (c)`), and a generic call
  with a blank before its `(`.
- a line left open whose operand closes past a line that starts with another closer.

The tier-1 Rust writer keeps no fill whose start reads another loop fill (`fill(as_nl, …)`), so the
keyword marks are token marks, not marks of `as_sline`.

## What changed

- `stream.rs`: the opener part's gap marks.
- `kotlin.bitlex`:
  - the masks `as_nl`, `as_lead`, `as_first`, `as_skip`, `as_sline`, `as_opener`, `as_in1`, `as_glt`,
    `as_ggt`, `as_gen`, `as_cmp`, `closer`, `line_lead`, `as_cont`, `as_cline`, `as_c_open`, `as_c_in1`,
    and their escapes;
  - the marks `as_kwn` and `as_kwi`, the rules `kw_name` and `kw_infix_op`, and `as_split`'s keywords.
  - `tier1.rs`, `tier2.rs` and `kotlin.answers` regenerated.
- Tests:
  - `split_lines_are_the_lean_parsers`;
  - the refused list's split lines.
