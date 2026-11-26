# 0566: A shift outside the regions is of the masks' outside values

Status: decided (the user, 2026-11-21: "… After those, nothing reads the regions' walk and it can go (step D)",
the instruction "do this"). The first part of 0556's step D. Follows 0564 (the loop keeps the gap masks and the
semicolon's fills).

## Intro

**The problem.** After 0564, most of what still read the regions' walk read a mask made of the regions,
shifted a byte or more. Examples:
- a number's masks (`frac_dot = code & dot & prev(di) & next(digit)`, the `real` mark's count);
- an escape after a number ("a digit after a binary number": `w & digit & prev(int_last)`);
- the escapes in a string's body.

Outside the regions, `code` is every byte, so `w` there is `word`. But `prev(w)` at a byte reads the byte
before, which may be in a region token, where `w` is none. So the compiler could not say what `prev(w)` is
outside the regions, and these masks were made after the walk.

## The decision

- **How a region token ends** (`End`, from each row's closers, `row_geos`). For each way it ends:
  - its kind;
  - where its closer comes after the token, the closer's first bytes (the byte after the token);
  - the bytes its last four bytes may be (the closer's, at their offsets from the token's last).

  The bytes the openers' first four bytes may be are kept too.
- **`prev(a, j)` at a byte outside the regions is `prev(a₀, j)`**, `a₀` being `a`'s value outside. This holds
  where, for each way a region token ends and each of the `j` bytes before in it, no way `a` or `a₀` may hold
  (its `cubes`) fits the bytes there. The byte itself is one of the context's (the rest of a `&`). After a line
  comment, the byte is the line break: a mask that needs a digit there is never there. `next(a, j)` is the same
  with the openers' first bytes.
- **`fill(s, e)` at a byte outside is `fill(s₀, e₀)`**. This holds where, at each way a region token ends,
  `e` (exactly) stops the fill and `s` does not start it. That is at the token's last byte (`e`'s value in
  its region there), or at the closer after it (`e₀`). So no run of the fill comes out of a region.
- **A region's start is its opener's first byte**, a context `in_region` applies there. A region's opener
  mark `o_K` holds where its opener does (`cubes`).
- **Over a token of code, a mask is its value outside the regions** (`row_val`). This covers a token test's
  mask (`count`, `run`, `all`) and a token escape's. A token mark of the code is its mask at the start
  (`at_start`).

## Why

The criterion is the lean lexer's code: which form brings the generated Rust closer to it. The lean lexer
reads a number as code; it never asks whether the byte before a digit is in a string. A mask made after the
walk asks that at every byte of the file. The outside value asks it once, in the compiler, and the answer is
checked against the grammar's own closers.

## What changed

- `bitlex_lex_rust.rs`: `End`, `find_ends`, `outside_shift` (by `cubes` against the ends' bytes), the opener's
  byte as a region start's context, `o_K`'s cubes, `row_val`, token marks by `at_start`.
- `tier1.rs` regenerated. These no longer read the walk:
  - the `real` mark's count;
  - the token marks of the code;
  - the escapes "a digit after a binary number", "a `$` before a backtick" and "a backslash before `$` in a
    raw string".

  Six escapes are still tested over the file.

Kotlin: 53/53 with the corpus; bitlex 11/11; the compiler's bitlex tests 9/9.
