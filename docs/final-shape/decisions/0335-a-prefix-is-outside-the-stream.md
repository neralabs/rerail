# 0335: A prefix is outside the stream

Status: decided (2026-11-17); landed on the CPU and in `bitlex`'s evaluator.
Metal refuses a program with a prefix for now. Follows 0330.

## Intro

**The problem.** 0330 measured JSON's tier 1 against simdjson's stage 1
and moved the byte order mark (BOM) to delay 0. One cost was left: the
token loop's form. `json.bitlex` wrote the BOM as masks:

```text
mask bom        = ~prev(~empty) & ef & next(bb) & next(bf, 2)
mask bom_bytes  = bom | prev(bom, 1) | prev(bom, 2)
mask word       = code & wordbyte & ~bom_bytes
skip ws | bom_bytes
```

The token loop takes the lean lexer's form only where the skip is a byte
class alone (`lean_starts`). `bom_bytes` put `ef`, `bb` and `bf` at offsets
0 to 2 of the skip, so the loop kept its general form for one test of a
file's first three bytes. With `skip ws` alone (not correct on a file with
a BOM) the corpus took 28.6 instructions a byte, against 36.5.

A lean lexer, and simdjson, strip the BOM once, before their loop:

```rust
// simdjson's stage 1, and a lean lexer's prologue
if bytes.starts_with(&[0xef, 0xbb, 0xbf]) { start = 3 }
```

**The decision.** The language says so: a new statement.

```text
# A UTF-8 byte order mark at the start of the file, which simdjson skips: the stream starts
# after it, so no class, token or error has its bytes.
prefix 0xef 0xbb 0xbf
mask word       = code & wordbyte
skip ws
```

`prefix B …` (one to eight bytes): where the file starts with these bytes,
the stream starts after them. No class, `~` or token has them, the
stream's first byte is the one after them, and positions stay the file's.
Nothing else in the grammar names the BOM.

## Its meaning

The stream was the file's bytes, `within`. With a prefix it is
`within & ~P`, where `P` is the prefix's bytes, set only where the file
starts with them. Exactly what is cut to the stream:
- each class that holds a prefix byte (`class & ~P`);
- `~a` (`stream & ~a`), `next` and `back_fill`, and the regions' inputs,
  which were already cut to `within`;
- the outputs, as before.

With clean inputs, `prev`, `toggle`, `fill`, `escaped` and the counts start
at the stream's first byte as they start at a file's: a `prev` there reads
the zeros of a cut class.

`bitlex`'s evaluator (`eval.rs`) has the same definition: a `stream` mask,
with classes, `~`, `next`, `back_fill` and the gap check cut to it.

`syntax::prefix_of` refuses a prefix together with constructs whose meaning
reads raw bytes or positions: a machine, `ucode`, hypotheses, a number.
Both evaluators call it.

## Each part

- **`bitlex_masks`.** `prefix_value` makes `P` before any other statement,
  from the file's own `within`: the test at delay `n - 1` (at least 1),
  brought to delay 0 by one `early` over the whole of it (0330). Every value
  that reads the prefix reads that `early`, so:
  - a full block knows it is zero (`full_words`);
  - a byte window from position 16 on makes none of it (`mask_at`'s cold
    branch).

  A first form made `P` from `early_to` term by term. Some terms were plain
  `prev`s and compares, and every window of the token loop computed them:
  white space took 23.3 instructions a byte, not 16.
- **`within(d)`** returns the stream: `Within(d) & ~prev(P, d)`.
- **The prover** (`proof_ops`) reads `P` as zero. Its positions are the
  stream's, and its model before the stream ("no byte class has it, every
  carried value is 0") is what the cut values are at the prefix's bytes.
  With `P` free, as an `early` is to the prover, `~x` was no longer known to
  be one at a stream byte. Every byte became some row's first, and the
  reader's kind had no free byte left: JSON did not compile.
- **A rule's mask** (`run R`, `pair P`) is read at a token's bytes, from its
  first on, never before it, so never at a prefix byte. Where it is byte
  classes alone, it is lowered with no cut (`Lowering::ahead`). A `prev` in
  a rule's mask could reach back from the stream's first bytes into the
  prefix, so such a mask keeps its cut. With the cut, JSON's run word (a
  call a number token) tested the window's position every token: numbers
  took 51.5 instructions a byte, not 49.8.
- **The records** carry the prefix (`Records::prefix`, `recprefix K B…` in
  the program's text, so the digest covers it).
  - The reference loop (`records::walk`) starts at `Records::start`.
  - The CPU's loop starts its state's "end of the token before" there
    (`stream_start`): once a file, an 8-byte load and a compare, as the lean
    lexer's prologue.
- **Metal** refuses records with a prefix, as it refuses an `early`
  (`block_program::starts_late`).

## Measured

`json_tier1_by_pass`, least of 5 rounds, instructions a byte. The
"`skip ws` alone" column is the grammar of 0330 with only the skip
changed: no BOM handling in the loop, so not correct on a file with a BOM.
It is the bound for this change.

| Input | 0330 | Now | `skip ws` alone | simdjson |
|---|---|---|---|---|
| Corpus (12 files) | 36.5 | 29.2 | 28.7 | 2.9 |
| Long ASCII strings | 19.0 | 15.7 | 15.4 | 2.15 |
| Long UTF-8 strings | 23.3 | 19.9 | 19.6 | 3.26 |
| White space | 22.5 | 15.8 | 15.4 | 2.15 |
| Numbers | 56.4 | 49.8 | 49.5 | 4.51 |
| Short strings | 77.9 | 71.6 | 70.8 | 4.51 |

**What is left of the prefix:** 0.3 to 0.8 instructions a byte, in the
block masks. A class cut by `P` is a mask operation, so it leaves the
vectors. A class that was combined with others there (`lead2 | lead3 |
lead4`) then takes a movemask of its own. A full block knows `P` is zero,
but the lane plan is one for both bodies, and is made before. Folding it
needs a plan for the full body alone, or a cut on the vectors in the
general body.

**What is left against simdjson** is unchanged from 0330: the block masks
(about 13 instructions a byte on ASCII) and the token loop (about 80
instructions a number token).

## Tests

- `compiler`'s `a_prefix_is_outside_the_stream`: a grammar with a prefix,
  over files with the prefix whole, doubled, cut, not at the start, and
  none, lengths 0 to 200 over block edges.
  - `bitlex`'s evaluator gives each mask as a reference made by hand: the
    stream, the bytes after the prefix, read as a lexer that strips it.
  - The interpreter and the CPU give the same outputs, and those are the
    reference.
  - A batch too.
  - The prefix adds no delay.
- `rr_parser`'s `jsons_cpu_records_are_the_interpreters` (BOM inputs whole,
  cut, doubled and not at the start): 789 files, all the same.
- `jsons_tier2_split_is_tier1s`: 386 files, as before.
- `json_loop_against_step_14`: 129 parsed the same, 216 refused by both,
  none differ. Step 14 still writes the BOM as masks.
- rr unchanged (`tmp/verify.sh`).
