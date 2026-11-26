# 0428: The lone low surrogate is checked in tier 2, from the tokens before it

Status: built (audited 2026-11-23). Decided (the user, 2026-11-18: "please do this", on 0423's next step: move the
surrogate check into tier 2, as 0399 moved the hex digits). Follows 0398, 0399 and 0423.

## Intro

**The problem.** 0399 moved two of a `\u` escape's three checks to tier 2, into the escape
token's check: four hex digits, and a high surrogate followed by a low one. The third stayed in
tier 1:

```
error "a lone low surrogate" when low & ~prev(high, 6)
```

A low half (`\uDC00` to `\uDFFF`) must follow a high one, six bytes back. Whether the `\` six
bytes back starts an escape depends on the backslashes before it, so only the strings' escape
carry tells. A check over the token's bytes and the bytes after it cannot read that. In tier 1
the test was a gated region: a second nibble table, the `d`, `8 9 a b` and `c`–`f` masks, and the
`u` mask, 79 instructions of the loop's code plus 13 for the `u`.

**What tier 2 already knows.** Tier 1's carry is in tier 2's records: each token's first and
last byte. In `𐀀`, the record two before the low half's escape is the high half's
`\u`. In `\\uD800\uDC00`, a token ends at that `u` only if it is an escape. simdjson's stage 2
does the same work in order: it parses a high half, then requires a low one.

## The decision

**1. A token's check may read the tokens before it.** `LocalCheck::back` is how many bytes
before the token its frame begins. The token's first byte is then at place `back`, and before
the file every op reads 0. The new op `LocalOp::Ended` is 1 where a token before this one has its
last byte. Tier 2 reads it from the records before the token's own: at most `back` of them can
end in the frame, since each is a byte or more.

**2. The compiler lowers `A & ~L` this way** (`Lowering::token_ops_back`). This applies where
`A` lies at a row's tokens' last bytes and `L` reads that row's last bytes before the token.
- `L` is read with every row's last bytes, the records', in place of the one row's.
  `substituted` delays all of `L` where the records' mask is later.
- The prover must show that this refuses the same bytes of `A`: `A & (L ^ L')` never holds.
  For JSON, a token that ends at a `u` followed by a `d` is an escape.
- An op read before the token must be 0 there in both tiers:
  - a byte test that holds none of the prefix's bytes;
  - an `Ended`;
  - an `and`, an `or` or a `prev` of such ops;
  - or a `within`, if only an `and` with one of those reads it.
- `Ended` is never read at or after the token's last byte.

`14-json` and `15-jsonc` state the same error as before. The merged `\u` check now holds all
three tests, reading 6 bytes back: a frame of 6 + 2 + 8 = 16 bytes, still in the arm.

**3. A token's check runs only at the last bytes it can hold at** (`token_gate`). If every way
for the check to be 1 needs a byte set at the token's last byte, the arm tests that byte first.
JSON's escape check needs a `u` there. A `\n` or a `\"` skips it, as simdjson's stage 2 switches
on an escape's byte before its `\u` work. Without the gate, every escape paid for the frame:
about 280 more instructions each, +0.65M on the corpus below.

The interpreter (`stream`, `program::token_value`) and Cranelift (`token_check_short`,
`token_check_word`, `ended_word`) run both; the rir text writes `rB` and `e`. The Rust backend
has no token checks. A region that tier 2 splits (0323) is refused for a check that reads back:
its records are not the region's tokens.

## Requirements, a test each

- R1. The three `\u` tests are one token check that reads back, and tier 1 has no surrogate
  test (`a_unicode_escapes_checks_run_at_its_u`: one check of delay 8, `back` 6 with `Ended`).
- R2. The interpreter equals the rules' reference, and the CPU the interpreter. The texts:
  - 34 escape texts at offsets 50 to 69 in four documents each: pairs, lone halves, a pair
    after an escaped backslash's `uD800` (not an escape), and after runs of 1 to 4 backslashes;
  - 15 documents whose frame begins before the file.

  Mutations fail it:
  - the CPU reading only the one record before;
  - the interpreter reading only the last record;
  - the gate testing the wrong byte.
- R3. The gate is `u` for the escape check, none for the number check (the same test).
- R4. The CPU's plan has the gated regions it compiles: `json_gated_regions_equal_the_interpreter`
  now plans from the CPU's form (`cpu_form`), which the CPU compiles. With the surrogate region
  gone, the program before that form plans none for `14-json`.
- R5. Tier 1 and tier 2 equal the interpreter and the engine:
  - the compiler suite and the parser_tests guards;
  - the rr_parser suite (48);
  - the rr JSON set: 789 files, every token the same, and `the_loops_errors_are_the_masks`;
  - `json_corpus_accuracy` on the 17 corpus files within the narrow widths.

## What changed

| | After 0423 | Now |
|---|---:|---:|
| `14-json` hot loop | 470 / 1 stack | **358** / 1 |
| its masks a block (`addp` / 4) | 9 | **5**, simdjson's |
| its tables (`tbl`) | 28 | **20**, `stage1.rir`'s |
| `15-jsonc` hot loop | 632 / 7 | **523** / 6 |
| Tier 1, retired instructions, 17 corpus files | 20.0M | **19.2M** (−4%) |
| Tier 2 | 29.8M | **29.8M** |
| Both | 49.9M | **49.0M** (−1.8%) |

Instructions were counted twice each way (`json_optimizer_bench`, optimizer off) at a load of
about 20. The 17 files hold 2,298 escapes, 235 of them `\u`.

`14-json` is now 26 instructions over `stage1.rir`, with the same masks and tables. What is left
is the token kinds' word logic that tier 2 reads (string content, escapes, punctuation, words,
the skip), which simdjson's stage 2 finds by rescanning.

**Not measured:** `twitterescaped.json` (31,818 `\u`), the file dense in what is checked.
It is past tier 2's narrow widths and refused at `ACCEPT` (0423, page 37's open item).
