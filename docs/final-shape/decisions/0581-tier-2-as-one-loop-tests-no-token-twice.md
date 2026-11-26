# 0581: Tier 2 as one loop tests no token twice

Status: done by the implementer. The item is the user's (2026-11-21, "do number 1": JSON's generated Rust beside the
lean parser, its differences fixed in structural order); this is the first of the fixes page 40's step 59 lists. How it
is done is the implementer's choice. Follows 0551.

## What was there

`bitlex_rust::tier2_loop` wrote JSON's tier 2 with tests the lean parser (`rerail-json-lean`'s `walk`) does not make,
each of a token a decision had already found:

- A token's take and a string's read tested the token's byte inside the arm of the `match` that had just taken it
  (`0x22 => { match peek { 0x22 => {} _ => return Err(…) } … }`, the same for `,` and `:`).
- An empty container's test read the opener again (`peek(i) == 0x7b && peek(i + 1) == 0x7d` in the `0x7b` arm).
- An atom's arm decided its word, then decided its kind's bytes and its word again, and found the token's end twice.
- An optional or repeated part that starts with a choice (`[value (',' value)*]`) decided whether it is there, then
  which alternative it is: two `match`es on the same byte.
- A `check`'s optional, repeated and alternative parts tested their first byte, then tested it again
  (`if matches!(x) { if !matches!(x) { return false } … }`).
- A place whose code only went on to another place (`9 => { st = 8; continue }`).

## The decision

1. **An arm's code is made for what its decision found** (`T`): nothing, the token's byte, or its word (its start and
   end bound). A token, a region or a level's call reached by a decision does not test it again; reached by none, it
   tests it once. A level's body reads its first token as found: each call tests it first.
2. **A part that starts with a choice is decided by the choice's alternatives** (`heads`): `[A (',' A)*]` with `A` a
   choice is one `match` on the alternatives, each going on to what follows. Not where an alternative may be empty.
3. **A check's part tested by its `if`, `while` or alternative reads its byte as found.**
4. **A place that only goes on to another is that place** (`to_place`).

## Checks

- JSON's tests (`parser_tests`, `json`): 10, `json_rusts_tapes_are_the_jits` (the generated parser's tapes are the
  JIT's) and `json_rust_is_current` among them.
- On a Linux host: `rerail-bitlex`'s and `rerail-compiler`'s tests, `rr_parser`'s `rust_tests`, the `.rerail` and
  Kotlin `rust_is_current`: 63 pass. The one failure, `rr_parser`'s `tapes_are_tape`, is a Metal run, which Linux
  refuses.
- `tier2.rs` read after: no token is tested twice.

## Not checked, not done

- The other differences of page 40's step 59 (the next fixes).
- No other grammar reaches the loop form, so `heads` and `T` were tried on JSON's alone.
