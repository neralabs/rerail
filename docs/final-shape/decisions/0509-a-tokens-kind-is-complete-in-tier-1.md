# 0509: A token's kind is complete in tier 1

Status: decided (the user, 2026-11-19: "the complete token kind in tier 1: integer, hex, binary,
real, the name split, removing the `s.bit` row tests, as lean's `T`"). Item 1 of the lean shape;
follows 0492, 0496 and 0498.

## Intro

**How the lean lexer gives a token.** Its `Tok` carries a `T` that says which token it is:
`Int`, `Hex`, `Bin`, `Real`, `Name`, `Str`, each operator. The parser matches on `T` and never
looks at the bytes again.

**What ours had.** A record's kind was the class of its first byte, and that class was shared:
`0` begins an integer, a `0x` and a `0b`; `"` begins a string and closes a template's tail; a
string's text begins with any byte, a `$` and a `"` among them. So tier 2 told the rows apart
itself, by each row's start mask (`ROW`): 62 `s.bit(…)` tests in Kotlin's generated tier 2, and
the masks stored for them.

## The decision

Tier 1 gives each token its row's kind, so the kind alone tells tier 2 which token it is.

1. **The token's own kinds.** A row that shares a first byte with a row of another kind gets a
   kind of its own, unless it is the widest (the most first bytes). That covers a `0x` and a `0b`
   beside a number, and a `.5` beside the operators. Its start mask sets it (`Records::retags`),
   as a reader's kind was set before.
   - The JIT reads that mask at the token's first bytes the mask can hold at
     (`needs_by_offset`), in the token's tail, and makes it (`readers_tags` leaves it out).
2. **A string's text and its closers.** In a token loop (`skip`), a region's content gets its own
   kind (`Reader::content`) where it shares a first byte with a part or a closer. So does a closer
   that begins with a byte a token outside begins with: a template's `"`, beside a string's opener
   (`Reader::closers`).
   - The walk writes these kinds, as it wrote a part's (`ReadPart::kind`). Their rows' start
     masks give them in `records::pass` and the generated Rust.
   - A reader's own tag is never a closer's (`retag_of` reads the token's own rows only).
3. **Rows of one kind are one.** The rows of one kind end alike, at their record's last
   (`Span::Last`, a one-byte token's start). Rows that tier 2 reads alike (`Row::alike`: one kind,
   first byte, words, end and check, as in each reader's region) are one row to a state. Their
   arms are taken together (`target`).
4. **The token marks and the gap patterns** test a moved row's own kind, not its first bytes'
   (`name_kinds`).

## Results

- Kotlin's generated tier 2 has no row test left: 62 `s.bit` tests before, 0 now.
  `tier2_aot.rs` went from 19,690 lines to 18,616, and tier 1 stores 32 mask words a block (35
  before).
- `t::` names Kotlin's kinds as the lean `T`: `INTEGER_LITERAL`, `HEX_LITERAL`, `BIN_LITERAL`,
  `REAL_LITERAL`, `STRING_CONTENT`, a closer's `"` apart from an opener's.
- The tests: kotlin_aot 4/4 (tapes the JIT's on every file, 465 taken); Kotlin 46; compiler 140;
  parser_tests 92.

## Still apart from `T`

- **One string token.** The lean lexer gives a string as one `Str` token, and its parser reads the
  pieces. Ours gives each piece a record (0323's lowering A). The pieces' kinds are now complete,
  so that is a choice of granularity, not a test tier 2 makes.
- **The `0` byte's two classes.** `INTEGER_LITERAL` and `INTEGER_LITERAL_12` are one kind in the
  grammar, but two classes of first byte: the token marks' columns split them (decision 0129).
- **The JIT's own-kind read** is a call for a token at a first byte the mask may hold at (a `.`,
  a `0`): the lean lexer's number arm knows from the bytes it read. Next: the row's end rule gives
  the kind where it reads those bytes anyway.
