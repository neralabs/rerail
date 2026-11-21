# 0585: The last escapes leave the walk, and the walk is the test build's

Status: decided (renumbered from 0573, which another worktree holds; the user, 2026-11-21: "go ahead", on converting the two escapes left on the walk and then
keeping the walk for the test-build asserts only). Ends 0551's step D. Follows 0572 (a region token's parts).

## Intro

**The problem.** After 0572 two escapes were still masks made after the regions' walk, tested over the file:
- "a `0x` or `0b` with no digit": `prev(hex_at, 2) & ~hexd | prev(bin_at, 2) & ~bind`;
- "a dotted type after `as` before a `<`": `code & "<" & prev(fill(as_run & dot, ~as_run))`, where
  `as_run = fill(prev(as_kw, 2), ~(code & (word | dot | " " | "?")))`.

Each reads a mask of the code two bytes back. A string or a comment may end with `0x` or `as`, so at the byte
after a region token no rule over the bytes says what `prev(hex_at, 2)` is outside the regions (0566 rightly
refuses it). The walk was kept for these and for the test build's checks of the grammar's `assume`s.

## The decision

- **An escape over each stretch of code** (`stretch_escape`). Each of its terms is `prev(X, j) & Y`, where `X`
  holds in no region's part and `Y` is stage 1's. At a byte whose `j`-th before is outside the region tokens it
  is `prev(X₀, j) & Y` (`X₀` its value outside), else none. The loop keeps where the stretch since the last
  region token starts (`code_from`). At a region token it tests the stretch before it, shifted by `j`, and
  after the loop it tests the last one. This is the `0x` escape.
- **An escape made of the regions through `code` alone** (`code_escape`, the last form tried). Its value is
  its masks with `code` as the bytes outside the region tokens. The loop records those as it reads them
  (`code_l`, one `clear` a region token), and the escape is tested after the loop. The test build asserts
  that the record is the walk's `code`. This is the `as` escape.
- **The shifts know the bytes about a byte** (`Ctx`, offsets −4 to 4). A `&`'s side is read where the other
  holds, at every offset it knows, and a place a shift reads must fit them. So `~next(w, 2)` in `as_kw`, whose
  `text("as")` puts `s` at +1, is never read over a region's opener. A `next` that reads into a region token
  where the mask is none in every part is none there, as a `prev` was.
- **The walk is the test build's.** The regions' walk and the masks made of it are one block under
  `#[cfg(feature = "test-hooks")]` (`gate_walk`). It gives out the masks the `assume` asserts and the `code_l`
  assert read. The escapes' tests stay outside it. A build without `test-hooks` makes no walk.

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. The lean lexer has no
pass over the file to find the strings. It reads tokens, and it knows a string by the token it is in. Now tier 1
is the same: stage 1's masks, then the token loop, which knows each region from the token it reads. The
grammar's `assume`s and the loop's record are checked against the walk in the test build, so each invariant it
relies on is checked.

## What changed

- `bitlex_lex_rust.rs`: `stretch_escape`, `code_escape`/`of_code`, `Ctx`, `next`'s none-in-the-token in
  `shift_before`, and `gate_walk`.
- `tier1.rs` regenerated. Without `test-hooks` it has no walk and no mask made of one.

Kotlin: 53/53 with the corpus; bitlex 11/11; the compiler's bitlex 9/9; `rerail-kotlin-aot` builds without
`test-hooks`.

## Since

- 0586 keeps the `as` escape in the token loop. `code_l` and the test after the loop go with it.
