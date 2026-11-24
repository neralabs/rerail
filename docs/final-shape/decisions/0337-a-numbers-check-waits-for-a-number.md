# 0337: A number's check waits for a number

Status: built (audited 2026-11-23). Decided (2026-11-17); landed on the CPU. Follows 0335.

## Intro

**The problem.** After 0335, JSON's tier 1 took 15.8 instructions a byte
over long ASCII strings, where simdjson's stage 1 takes 2.15. Almost no
token starts there, so this is the cost of the block masks alone: about
1,000 instructions per 64 bytes, against simdjson's 140.

The full body's ASCII path made 14 movemasks a block. simdjson's makes
four or five: quotes, backslashes, white space and structurals (one table
lookup), and bytes below 0x20. Reading the generated code against the block
program, two causes stood out.

1. **Five start masks for tier 2.** Tier 2 tells apart two token rows that
   begin with one byte by their start masks (`ROW`). JSON's opener quote, the
   escape, the string's content, the punctuation and the number were all
   "tested", so tier 1 made and stored five more mask words every block.
   A lean parser needs none: it knows whether it is in a string.
2. **The number's check on every block.** `json.bitlex` checks a number's
   grammar:

   ```text
   check number = ['-'] ('0' | nonzero digit*) ['.' digit+] [expo ['+' | '-'] digit+]
   ```

   The compiler makes a byte machine of it, as masks over the whole file
   (three `fill`s, about ten byte classes: `'0'`, `'-'`, `'.'`, `1`–`9`, …).
   Every block made all of them, a string's block included. simdjson checks
   a number when it meets one, in stage 2.

## The decisions

**A region is a mode.** Tier 2's states know the modes their tokens are read
in (`row_modes`). A mode was only a `toggle(x)`: its value `t`, and `x`, its
flips. JSON's string is a `regions` construct, a fill, so it was no mode,
and every state could "meet" every row.

A region's fill `f` is a mode the same way. `f` holds from the opener up to
the closer, so it changes only where `f ^ prev(f)` holds: at the opener's
first byte and at the closer's. Each region kind is now pushed as
`(f, f ^ prev(f))` with the toggles. The rule that a mode qualifies only
when every flip is a token's start is unchanged.

A row's mode is fixed where its start proves it. For the rows outside a
string, the start is `code & …`, where `code = ~hidden`, and `hidden` is the
regions' carry. The plain search takes the carry as free. So `row_modes`
proves with the fixed points' equations (`bitlex_prove::Induction`) where
the plain search does not, as the marks' proofs already do.

For JSON every row's mode is now fixed. No state meets a row of the other
mode, no row is tested, and the five start masks are gone: 16 outputs to 11.

**A run's first byte is a sparse gate.** The lane planner skips work behind
gated regions: a value zero wherever its gate is zero, behind one branch a
block (`native_cpu_lanes::regions`). Without measured densities a gate
needs two conditions, one of them carried state (an escape, a fill). This is
the rule that skips the error checks in blocks with no escapes.

The number check's gate is `start & (digit | '-')`, where `start` is
`word & ~prev(word)`. Its only carried state is `~hidden`, on the
`and not` side, which the rule did not count. A run's first byte,
`a & ~prev(a)`, is as rare as the runs are, so it now counts as carried.
The number check is then a region of its own (weight 233, 52 values). A
block with no number starts skips all of it.

## Measured

`json_tier1_by_pass`, least of 5 rounds, instructions a byte:

| Input | 0335 | No start masks | And the number check gated | simdjson |
|---|---|---|---|---|
| Corpus (12 files) | 29.4 | 29.4 | 27.7 | 2.9 |
| Long ASCII strings | 15.8 | 15.9 | 10.2 | 2.15 |
| Long UTF-8 strings | 20.1 | 20.4 | 14.7 | 3.26 |
| White space | 16.0 | 15.3 | 9.6 | 2.15 |
| Numbers | 50.0 | 49.3 | 49.3 | 4.51 |
| Short strings | 71.8 | 71.9 | 66.2 | 4.51 |

The start masks cost little: they were mask words already made, then stored.
The number check was most of what was left: a third of the ASCII body.

## What is left

- **The block masks**, about 10 instructions a byte on ASCII. Each class
  group is still a table pair of its own, where simdjson does one lookup.
  The string region is a call a block (`f2`), through the stack.
- **The token loop**: numbers and short strings, at 49 and 66.

## Tests

- `rr_parser`'s `jsons_cpu_records_are_the_interpreters`: 789 files, the CPU's
  records are the interpreter's.
- `jsons_tier2_split_is_tier1s`: 386 files, as before.
- `json_loop_against_step_14`: 129 parsed the same, 216 refused by both,
  none differ.
- rr: `tmp/verify.sh`.
