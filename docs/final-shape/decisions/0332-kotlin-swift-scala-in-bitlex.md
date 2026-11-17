# 0332: Kotlin, Swift and Scala in Bitlex: ladders against the lean parsers, and the compiler's additions

Status: decided (user, 2026-11-16: "build bitlex versions of each of these"). Kotlin's step 1 is
built. Swift and Scala are not started.

## Intro

A Bitlex version of a language is its grammar written in Bitlex and compiled by the Bitlex
compiler. Tier 1 (masks over 64-byte blocks, then the token loop) and tier 2 (the tree grammar's
automaton) are lowered through the transducer's backends, as `.rr`'s and JSON's are
(docs/parser-generators/12, 24). Its output is the engine's tape, written as it parses.

Each language is built as `.rr` was: a ladder of grammar steps (`<lang>_parser/bitlex_ladder/`).
Each step adds constructs, and the compiler grows only where a step needs it. A step refuses
what it does not read yet, and a refused file goes to the next parser down: the lean parser,
then tree-sitter. So a step never writes a wrong tape. It only covers fewer files.

```
a .kt file -> compiled Bitlex grammar -> tape        (where the ladder reaches)
              refused -> lean Kotlin parser -> tape  (0305; tree-sitter where it refuses)
```

## The reference

The reference is the lean parser's tape. 0305 and 0326 test that tape against tree-sitter's,
node for node. The grammar names the lean parser's kinds and fields (`rerail_kotlin_lean::KINDS`,
`FIELDS`), so a compiled key is the lean parser's key. The grammar's own names (marks, flags) take
ids from 0x4000 and never reach a tape, as rr's do. The crate's tests compare the two tapes as
text (`image_text` against `tape_text`). A corpus test (ignored, `RERAIL_KOTLIN_CORPUS`) counts
the files that read the same, are refused, or differ. "Differ" must stay 0.

## Kotlin, step 1 (`01-headers.bitlex`)

Step 1 reads the package header, the imports and the comments:

- `package`, `import` and `as` are soft keywords and implied.
- An import's dots are implied (a rule retagged as `identifier`).
- `.*` is a `wildcard_import` over the `*`, found by a token mark at the `.`.
- A header ends at a line break in code, or at the file's end (the `line` mark).

A file with a quote, apostrophe or backtick in code, a byte above 0x7f, or a nested block comment
is handed over. A file's first string opener is always in code, so the comment regions are exact
for every file that is not handed over.

A comment on a header's own line belongs to the header in tree-sitter's tree, because the
scanner's semicolon is the line break after it. The `line` mark `settles` (below).

## The compiler's additions

1. **Several kinds of extras in the token step.** Kotlin, Swift and Scala each have two comment
   kinds. Before this, the token step took extras only when there was one kind, and token marks
   need the token step.
   - Every extra's record still has the one extra mark bit. Tier 1, the token loop, Metal and the
     token marks read only that bit, so none of them changes.
   - Each kind after the first has a bit of its own (`Words::extras`).
   - Tier 2 gives the leaf the last such kind whose bit is set, or the first kind
     (`stream::extra_kind`, `MoreExtras` in Cranelift, the same expression in `rust_stream`).
   - This costs one mark for each kind after the first, within `records::MARKS`.
2. **Several kinds read at one first byte.** A line comment and a block comment both start at
   `/`, and the token loop has one reader for each first byte.
   - The readers of the kinds that start at one byte merge into one reader.
   - Each arm of a kind other than the reader's own carries its kind's tag (`ReadArm::retag`,
     `recarm … retag K M`). The CPU reader writes that tag for the arm it read.
   - The mask passes and Metal already retag by each row's start mask (`Records::retags`), so
     they need nothing.
   - `Reader::tags()` and `Reader::tag()` replace the reads of `Reader::retag` alone.

3. **A mark that settles (`mark … , settles`, tier 2's `SETTLE`).** A mark is placed at the
   next grammar token, so the comments before it are pending when its arm closes the
   statement, and `CLOSE` leaves pending extras out (24 §2.3).
   - The compiler gives a settling mark a mask: from each of its bits up to the next grammar
     token's start, `fill(m, starts)`.
   - The mark's leaf in its arm becomes `SETTLE mask`. It sits after the closes of the levels
     that end before the mark, and before the closes of the levels the mark ends.
   - `SETTLE` keeps in place the pending extras, from the first, that start where the mask does
     not hold: the comments before the break. The rest stay pending.
   - The interpreter, Cranelift (a loop over the pending rows, `Step::Node`) and `rust_stream`
     have it. The reference evaluator (`tools/bitlex`) parses the option and does not model it
     yet; no ladder it checks uses it.

Single-extra grammars compile as before: rr's tests and the compiler's targeted tests pass.

## Found on the way

- Q31: the ladder test `07-textes` was already red at the merged Parser-generators commit. It is
  theirs to fix.
- Q30: done in this decision (the settling mark).
- Q32: a token with no end rule ends at different bytes in the CPU tier 1 and in the interpreter.
  The grammar gives its punctuation a `pair` rule, as rr's does.

## Next

Kotlin's ladder goes on. Next: declarations, then expressions (in
precedence order, as `wrap`s), then strings with templates (regions read in parts), then
Kotlin's semicolon rules. Swift and Scala start the same way, in their own crates: their step 1 is
imports and comments, with Scala's comment kinds `comment` and `block_comment`.
