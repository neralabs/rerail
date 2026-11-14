# 0133: Tier 1's checks made at the token, in tier 2

Status: built (2026-11-12). Tier 1: 606 operations to 452, 634.5M instructions to 525.8M (JIT);
the whole parse 45.32 ms to 37.04 ms (JIT), 40.83 ms to 32.62 ms (ahead of time).

## Intro

**The problem.** Tier 1 computes every check at every byte, a 64-byte word at a time.
Some checks can only fail at a few tokens.
`.rr`'s depth of braces counts `{`, `[`, `}` and `]`.
A string's `\u` escapes need their digits checked, and only strings with a `\u` have any.
Tier 1 still paid for them over every word of every file:
the depth's bit-planes (60 operations), the escape's digit tests (about 80).

The lean parser makes these checks where it meets the token:
it counts its depth as it opens a brace,
and checks a string's escapes when its lexer noted a `\u` in it.

**The decision.** The compiler moves a check to tier 2's arms when it can prove the check
belongs to a token. Two forms:

```
# A count whose ups and downs are tokens' starts.
scan depth = count(code & (lbrace | lbracket), code & (rbrace | rbracket)) max 127
# An escape `A & L`: `A` lies in one row's tokens, `L` is a few byte tests.
escape "a \\u escape the engine checks" when uesc & ~u_ok
```

- **A count of tokens** (`COUNT`). The count's ups and downs are proved to be the
  starts of tokens of a few words (`{` `[`, `}` `]`).
  Tier 2 adds one or takes one away at each arm that takes such a token,
  and refuses the file past the count's most. Tier 1 makes no planes.
- **A local check** (`CHECK`). A check `A & L` whose `L` is local
  (byte tests at fixed offsets, constants, `prev`s; no carried value),
  and whose `A` is proved to lie in one row's tokens:
  at the row's starts, or inside a `fill` whose runs are the row's tokens.
  Tier 1 makes `A` alone, a mask.
  At each token of the row, tier 2 loads the mask's word at the token's start.
  Where the mask has a bit in the token's bytes,
  an outlined function walks those bits and runs `L`'s byte tests at each.
- **A hidden mark only refused** (`refuse M in L`) is split the same way.
  Each of its alternatives that has this form becomes a `CHECK`,
  in the arms of the refusing levels' states alone.
  `json_bad`'s `uesc & ~h4` moves this way. Its leading zero and its `-` read carried values,
  so they stay in the mark.

The proofs use the existing prover (`bitlex_prove::never`, and the bounded SAT check).
A check that cannot be proved stays in tier 1, as before.

## How

- The program: `COUNT` (0x65) and `Words::counts` (rir `tokencount N max M`);
  `CHECK` (0x66) and `Words::local_checks` (rir `localcheck`), with `program::local_value`,
  the check's meaning, which the interpreter runs.
- The compiler: `bitlex_masks` (`pending_counts`, `pending_checks`, `token_row`,
  `local_ops`); `bitlex_tier2::Emit::arm` places the words after the arm's `SPAN`.
  Call arms are skipped: the callee's arm takes the token.
- Cranelift: `Mem::Depth`; `check_word` and `local_at`.
  The check is always a function of its own, as rare code.
  Its two loops are a word's (`Step::Block`) and a bit's (`Step::Bits`).
  The ladder's shape check allows one loop over bits per local check, and no other.

Tests: depth 127 taken, 128 refused; good and bad `\u` escapes, in `.rr` strings and in JSON,
in strings past 64 bytes. The tapes are identical over the corpus.
