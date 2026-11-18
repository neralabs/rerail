# 0399: A `\u` escape's checks run at its token, in tier 2

Status: decided (the user, 2026-11-18: "Check numbers and \u escapes once per token in tier 2",
page 39's item 1, its second half). Follows 0398.

## Intro

**The problem.** JSON's `\u` checks were tier 1's: a region gated on the escapes, 76
instructions of the hot loop at page 39's count. There are two:
- the hex-digit error, `(prev(unicode, 1) | … | prev(unicode, 4)) & ~hex`;
- the lone-surrogate error, `(high & ~next(low, 6)) | (low & ~prev(high, 6))`.

Both sides of each depend on `unicode`, which needs the strings' escape carry. Neither side was
local, so neither could be a check.

## The decision

1. **The grammars state each test at the escape's `u`, of the bytes after it.** This is
   `14-json`, `15-jsonc` and `json_parser/json.bitlex`:
   - `unicode & ~(next(hex, 1) & … & next(hex, 4))`;
   - `high & ~(next(backslash, 5) & next(u, 6) & next(dee, 7) & next(low_2, 8))`. After four hex
     digits, a backslash can only start an escape, so the low half's escape is a byte test;
   - `low & ~prev(high, 6)` stays tier 1's: the escape six bytes back may follow an escaped
     backslash, which only the carry tells.

   Each refuses the same files as before: a fallback asks only whether any bit is set.
2. **`A & ~L` with a local `L` is a check.** `L`'s ops are lowered, then its `not` is added
   (`pending_checks`' flag). A `~` lowered as a mask is cut to the stream (`within`, less the
   prefix), and a check made from that mask could not read the cut. `local_ops_in_stream`
   takes the prefix as 0, and is used only where every `prev` and `within` reaches back no
   further than the check's delay. Every byte read is then at or after `A`'s place, inside the
   stream.
3. **Where `A` lies at a row's tokens' last bytes, the check is a token's check past its
   token** (`last_row`, `Lowering::token_ops_past`). Here the row is the escape, `\u`, which
   ends at its `u`.
   - Inside the token, `A` is the row's last-byte mask (here `esc_last`) at the last byte, and
     the rest of `A`'s ops there. So `A`'s ops are lowered with that mask taken as all ones.
     This is checked to be read only at `A`'s own place.
   - The check's delay is the larger of `A`'s and `L`'s: 8. The frame runs that many bytes past
     the token. The test at place `i` is the last op at `i + delay`, and it refuses at places
     `delay` to `n + delay`.
   - A `within k` in the frame tests the file's end: place `i - k` is in the frame and in the
     file.
   - Tier 2 reads no mask, and tier 1 makes none of it.
4. **A row's token's checks are one** (`merge_token_checks`): each test at the larger delay,
   ORed, with the ops both make kept once. This gives one load and one set of classes an
   escape.
5. **The CPU:** a frame of 16 bytes or fewer (the token and its delay) is checked in its arm,
   as 0398's numbers are. A `\u` is 2 + 8.

**Rejected: a check of `A`'s mask** (the general local check, `A` a mask tier 2 reads). It
worked, but the CPU's tier 1 stores no `unicode`, so the outlined check rebuilt it from the
bytes at each escape, escape carry included. That cost about 990 instructions a `\u`: 2.1 times
HEAD's instructions on `twitterescaped.json`.

## Measured

`14-json`'s hot loop: 802 → **772 instructions** (20 stack loads or stores). Tier 1's block
program: 138 → 122 operations. JSON's values an edge block differs in: 47 → 41.

Retired instructions a pass, against HEAD before 0398 (`json_bench`, `RERAIL_BENCH_RUN`, 100
passes, M5 Max):

| Files | Tier 1 | Tier 2 | Both |
|---|---|---|---|
| `twitter.json` | 10.45M → 8.79M | 10.98M → 11.85M | **−3.7%** |
| `twitterescaped.json` (31,818 `\u`) | 12.52M → 11.13M | 14.58M → 20.50M | **+17%** |

Before the checks were merged, `twitterescaped.json` was +27%. As with 0398's numbers, a file
dense in what is checked costs more per token than tier 1's gated region costs per block.

## Tests

- `compiler`:
  - `tier2_tests::a_unicode_escapes_checks_run_at_its_u` (new). 21 escape texts at offsets 50
    to 69, across a block's last bytes, in four documents each. They include escaped
    backslashes before a `u`, surrogate pairs, lone halves, and an unclosed string that ends
    in the escape (the frame's `within`). The interpreter must equal a reference of the rules,
    and the CPU the interpreter.
  - every test but `random_needs_are_answered`: 104 passed.
- `parser_tests`:
  - `json_edge_values_are_its_ends_checks`: its bound was a third of the body's values. The
    body shrank (187 → 122), so it is now a ratchet on the count: 14-json 41, 15-jsonc 56.
  - the `json`, `loop_keeps_values`, `lowers_to`, `equal_the`, `gated`, `lanes`, `registers`,
    `stage1`, `vectors`, `on_every_executor` and `compile_tests::json` guards pass.
- `rr_parser`: the JSON equality set with `RERAIL_JSON_SUITE` and `RERAIL_JSON_CORPUS`, and
  `json_loop_against_step_14`: 129 files parse the same, 216 are refused by both, none differ.
