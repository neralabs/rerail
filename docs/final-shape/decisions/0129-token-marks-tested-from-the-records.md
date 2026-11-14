# 0129: Token marks, tested from the records

Status: built (2026-11-12). Tier 1: 9 passes and the records to 3 and the records.

## Intro

**The problem.** Some of a grammar's marks say what the next tokens are.
`.rr`'s `empty_parens` is a `(` whose next token is `)`.
`suppress_on` is the word `suppress`, then a name, then a string.
Tier 1 computes each one as byte masks. "The next token is X" is
`next(back_fill(X, solid))`, and each `back_fill` needs a pass backward over the file,
then a forward pass after it. Tier 1 makes 9 passes over each file, 4 of them backward,
and they are a quarter of its time.

The lean parser tests the same things one token at a time:
`self.is_id(W_SUPPRESS) && self.is_id_at(1) && self.kind_at(2) == STR`.

**The decision.** A mark may be a short sequence of tokens:

```
# A `(` whose next token is `)`.
mark empty_parens = tokens "(" ")"
# After `min` (or another aggregate's word): `(`, `)`, `:`.
mark no_value = tokens after "min" | "max" | "sum" | "mean" | "join" "(" ")" ":"
```

Each element is a token's kind (`identifier`) or a word (`"("`, `"fix"`),
or several of them joined by `|`. `after E`: the token before is `E`.
The mark holds at the first element's token. Comments between the tokens are skipped,
as the byte masks skipped them (`solid`).

Tier 1 makes no mask for such a mark.
Tier 2 tests it where it tests any mark: the token's own word,
then the records after it (and the one before, for `after`).
Most tokens fail at the first compare. The masks it replaces
(the `back_fill`s, the aggregates' words, `after_dot`) leave tier 1.

## How

- **Syntax and the reference evaluator** (`tools/bitlex`): `Stmt::TokenMark`. The
  evaluator places it from its token list.
- **The program** (`Words::token_marks`, rir `tokenmark`): a mark's order, then its
  elements by offset (-1 for `after`, then 0 up). Each element is a set of record kinds
  and a set of words.
- **The records' kinds** are split as far as an element's kind needs: a pattern's kind
  is a column of the kinds' signature, as a switch's targets are.
- **In the record**, the mark's bit is set at every grammar token whose word is its
  first element's word (`Records::marks`, with a word), so `Live` gates it as any mark.
- **`MARK`** of a token mark calls the pattern's function (outlined, one a pattern),
  which reads the records' codes after `Next`, skipping comments: a stated `Token` loop.
  The interpreter does the same.

Only marks whose tokens are all within the grammar's tokens are token marks.
`line`, `newline` and `json_bad` stay byte masks.
