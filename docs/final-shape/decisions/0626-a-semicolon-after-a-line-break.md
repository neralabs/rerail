# 0626: A semicolon after a line break

Status: decided (2026-11-22, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0451.

## Intro

**The problem.** The grammar refused a `;` that starts a line (`val a = 1\n;val b = 2`) and one that ends
the file (`val a = 1;`), by two escapes. The lean parser reads both. Its lexer's `asi`, looking past a line
break for what follows, returns the first `;` it finds (`Asi::Semi`), on the token's line or the next. The
`;` read is then the statement's semicolon, and the line break is none.

## The decision

- **`semi` and `ann_lf`**: `unless before ";"`. A line break before a `;` is no semicolon; the grammar
  takes the `;` (`statement (semi | ';')`, and the same in every list of statements and members).
- **The escapes go.** At the file's end the `;` needs no more: the grammar ends after it.
- **A `;` after a line break and a comment** stays refused, as the lean parser refuses it. There `asi` reads
  the comment, gives a semicolon of no width at the line break, and the `;` that follows takes no
  statement's place. The escape is made of `code` alone, so the token loop tests it: a comment's first byte
  is a region's first byte (`r_first = ~code & prev(code)`) that is a `/`, as no string starts with one.
- **Tier 1 as Rust, a fix**: `absorb` (`a && b`, each conjunct once) split each side on `&&` with
  `parts_of`, which takes a side's outer parentheses off. So `x && (y || z)` came back as `x && y || z`.
  `semi_close`'s value at a token's start, `}` after a `;`, then held at any token right after a `;`. The
  file that showed it, `\n;val`, was refused before. A disjunction joined goes back in its parentheses.

## Why

The criterion is the lean lexer's `asi` and the lean parser's `semi`: each case is a path of their code.

## What changed

- `kotlin.bitlex`: `semi`'s and `ann_lf`'s `unless before`; the escapes; `r_first`, `cmt_on_lf`.
  `tier1.rs`, `tier2.rs` and `kotlin.answers` regenerated.
- `bitlex_lex_rust.rs`: `absorb`.
- Tests: `a_semicolon_after_a_line_break_is_the_lean_parsers`: first on a line, at the file's end, alone on
  its line, in a block, a class body, after imports and the package, after a comment on the token's line,
  a `/` in a string. The refused list has a `;` after a line comment and after a block comment.
