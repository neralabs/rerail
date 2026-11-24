# 0564: The token loop keeps the gap masks and the semicolon's fills

Status: decided (the user, 2026-11-21: "Next: step C item 3, the escapes tested over code … Then the template
masks, which need state kept in the loop, and the semicolon masks. After those, nothing reads the regions' walk
and it can go (step D)", the instruction "do this"). Follows 0556 (tier 1's masks one pass over the blocks; its
step 2, the regions read in the token loop), 0607 (a grammar states what the token loop knows) and 0562 (the
loop keeps a template's code).

## Intro

**The problem.** Kotlin's semicolon is masks over the file. A gap mark (`semi`, `ann_lf`, `head_ctor`) reads
`lf & code`: a line break outside the regions since the grammar token before. `apart` reads `ws & code` or a
comment's opener. `semi_close` reads `close_at`. That is a line break where the statement may end
(`lf_end = lf & code & ~after_semi`), or a `}` after a `;`. `after_semi = fill(prev(code & semicolon), solid)`
is the bytes after a `;` up to the next solid byte. Two escapes read fills like it:
- "a `;` that starts a line": `line_first = solid & prev(fill(lf & code, solid))`;
- "a `;` that ends the file": `last_byte & fill(code & semicolon, solid)`.

Each was a mask made after the regions' walk. The gap marks tested it over each gap (`any_in(&m, from, s)`),
and the escapes tested their bits over the whole file.

The lean lexer keeps these as it reads: "a newline since the last token", "the last token was a `;`". The
generated tier 1 should do the same, from the grammar.

## The decision

- **A gap mask the loop keeps** (`GapAcc`). This applies to a gap mark whose mask is made of the regions,
  where it holds in no region after its start. The loop keeps whether the mask held since the grammar token
  before (`acc_J`) and since the record before (`rec_J`). It builds that from three pieces, each from the
  grammar:
  - the mask over the skipped bytes before a token (a test over the gap);
  - at the token's start (`at_start`);
  - over a token of code's bytes after its start, where the row's bytes can hold it.

  After a grammar token it starts again. After an extra (a comment) it goes on.
- **A fill the loop keeps** (`LoopFill`). This is `fill(X, E)` or `fill(prev(X), E)`, where:
  - `X` holds in no region;
  - `E` holds in none after its start, and at no skipped byte;
  - each is the same over a token of code's bytes after its start (the row's bytes against the mask's
    exact bytes).

  The loop keeps the fill and `X` at each token's last byte (`fill_I`, `xl_I`). From those it knows the fill
  at the byte before the next token (`fs_I`) and `X` there (`xs_I`). It also knows the fill over the gap, where
  `X` holds at no skipped byte there (`fgap_I`). Each row's transfer is code made from what the row's bytes
  are: a string's opener stops the fill, a comment carries it, and a `;` starts it.
  - `at_start` reads `prev(F)` as `fs_I`, and `prev(X)` as `xs_I`.
  - A gap mask that reads a fill is tested over the gap both ways (`fgap_I && …`, `!fgap_I && …`).
  - An escape that reads a fill and holds only at a start (none at a skipped byte, in a token of code after
    its start, or in a region after its start) is a test at each start.
  - An escape that is the file's last byte and a fill is a test after the loop.
- **The byte reasoning keeps the ways a mask may hold apart.** Each way is the bytes it needs at offsets from
  where it holds (`cubes`, at most 16, then widened). So a `&` whose sides need disjoint bytes at one offset is
  none. A number's bytes after its start are never a line break, and no number row tests its tail. A start is
  never a skipped byte: a mask at only skipped bytes is false at a start, and one at every other byte is true.

## Why

The criterion is the lean lexer's code (the user, 2026-11-20): which form brings the generated Rust closer to
it. The lean lexer has no mask of "a `;` before, up to a solid byte" over the file. It has a flag the token
loop sets at a `;` and clears at a token. The fill's per-row transfer is that flag, derived from the grammar
rather than written for Kotlin. The gap marks become the lean lexer's "a newline since the last token".

## What changed

- `bitlex_lex_rust.rs`: `GapAcc`, `LoopFill`, `cubes`/`bytes_at`, `exact_bytes`, `known_over`,
  `find_loop_fills`, `gap_code`, `start_escape`, the end checks; `at_start` reads the fills and folds
  constants.
- `tier1.rs` regenerated. The semicolon's masks and both escapes no longer read the walk's masks. The walk
  still makes them (step D removes it).

Kotlin: 53/53 with the corpus; bitlex 11/11; the compiler's bitlex tests 9/9.

## Not yet

These readers of the walk remain, for step D:
- the raw strings' and numbers' escapes;
- the token marks;
- tier 2's fields;
- the tails' close masks;
- the test-only asserts.
