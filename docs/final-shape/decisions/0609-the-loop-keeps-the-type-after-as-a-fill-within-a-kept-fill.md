# 0609: The loop keeps the type after `as`, a fill within a kept fill

Numbered 0586 where it was written (Swift-Kotlin-Scala-support); 0609 at its merge into Parser-generators-final-shape, whose 0586 is another.

Status: decided (the user, 2026-11-21: "go ahead", on keeping the `as` escape's state token by token in the
loop, as the semicolon's fills, in place of a test after the loop). Follows 0608 (the last escapes leave the
walk) and 0564 (the loop keeps the semicolon's fills).

## Intro

**The problem.** 0608 tested "a dotted type after `as` before a `<`" after the loop. It took its masks over the
bytes outside the region tokens (`code_l`), recorded by the loop:

```
mask as_kw  = kw_start & text("as") & ~next(w, 2)
mask as_run = fill(prev(as_kw, 2), ~(code & (word | dot | text(" ") | text("?"))))
escape "a dotted type after `as` before a `<`" when code & text("<") & prev(fill(as_run & dot, ~as_run))
```

That is a pass over the file after the tokens, which the lean parser does not make. It reads the type after `as`
as it reads the tokens. The loop's kept fills (0564) are that shape: a boolean a fill has at each token's last
byte, carried to the next token. But they took `as_run` for none of four reasons:
- its start is two bytes after `as_kw`;
- its stop holds at skipped bytes (a line break ends the type);
- its stop holds at every byte of a region token (a comment or a string ends it), not at none;
- its stop holds at some bytes of a token after its first (`?:`), and the fills knew only none or all.

And `fill(as_run & dot, ~as_run)` is a fill made of another fill.

## The decision

- **`as_run` is a kept fill.** It needs four extensions to the kept fills, each exact:
  - `prev(X, j)` is `prev(prev(X, j - 1))`: the fill's `X` is `prev(X, j - 1)`, which the shifts outside the
    regions know (0608's `Ctx`).
  - A stop at skipped bytes (`eg`): over the gap the fill is its value at the token before's last byte, or
    its start at the gap's first byte, with no stop after (`any_in` over the gap).
  - A region token at all of whose bytes the stop holds: the fill is none there after its first byte.
  - A stop over some of a token's bytes after its first: the fill at its last byte is a start after the
    last stop (`last_in`), or, with no stop, its value at the first byte or a start after it.
- **A fill within a kept fill** (`within_fill`). `fill(F & Y, ~F)`, for `F` kept and `Y` of stage 1, is each of
  `F`'s runs from its first `Y`. It is none where `F` is none, so:
  - at a token's first byte it is `F & (Y | its value before)`;
  - over a gap it is its value before while `F` holds (`fill_G && fs_F`);
  - over a token's bytes after its first:
    - where no start of `F` is, it is `F` at the last byte and either its value at the first byte or a `Y`
      over those bytes;
    - where a start may be and no `Y` is, it is its first byte's value while no byte stops `F` without
      starting it (`E & ~S`, one stage-1 mask);
  - over a region token at all of whose bytes `F` is none, it is none after the first byte.
- **The escape** is tested at each token's first byte (`code & "<"` there and the fill within at the byte before,
  `fs_G`), and over a token's bytes after its first where a `<` may be (`..<`: `x as a.B..<c`). There, `F` holds
  from the first byte to its first stop over them, and `G` from the first byte, or from the first `Y` over them.
  So the test is an `A` after one of them, up to that stop (`tail_escape`, `first_in`).
- **A kept fill is kept only if something reads it.** The extensions let two more fills in (`word_run` and the
  number's), which no test reads. A fill is kept where code other than its own updates reads it, or a kept fill
  does; the others' state and updates are left out.
- The test build's `code_l` record goes with the escape after the loop: no escape is made of `code` alone now.
  `code_escape` stays the last form tried before a test over the file.

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. The lean parser reads
`x as a.b.C<D>` token by token. Now tier 1 does too: a boolean for "in the type after `as`" and one for "after a
dot in it", each updated at each token from the token's own bytes, as the semicolon's fills are. No escape is
tested after the loop.

## What changed

- `bitlex_lex_rust.rs`:
  - `LoopFill` gains `eg`, `within`, `info` (`RowFill`), `start0` and `stop0`;
  - `find_loop_fills` gains the four extensions, and `within_fill` is new;
  - `start_escape` gains the tail tests (`tail_escape`, `tail_checks`);
  - the kept fills' liveness;
  - the `first_in` and `last_in` helpers.
- `tier1.rs` regenerated: `fill_4` (`as_run`) and `fill_6` (its dotted part).
- `kotlin_parser` tests:
  - `refused` now also checks that tier 1 as Rust refuses as the interpreter's tier 1 does;
  - `the_as_escape_is_the_interpreters` covers the escape after a token, over a gap, over `..<`, and an `as` in a
    comment, a string or a backtick name, and a comment or line break in the type. Disabling the tail test in
    the generated file fails it.

Kotlin: 54/54 with the corpus; bitlex 11/11; the compiler's bitlex 9/9; `rerail-kotlin-aot` builds without
`test-hooks`.
