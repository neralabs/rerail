# 0583: Tier 2 as one loop: a list is one place, and its cycle another

Status: done by the implementer. The item is the user's (2026-11-21, "do number 1"). This is the third of the fixes in
page 40's step 59. How it is done is the implementer's choice. It follows 0582.

## What was there

`bitlex_rust::tier2_loop` walked `[A (',' A)*]` as two copies of `A`: the first element, then the star's element.

- JSON's pair code was written after `{` and again after `,`.
- An array's element code was written after `[` and again after `,`.
- Each copy had its own place for what follows the element (a pair's close).
- The optional list was decided after the opener, although the level's call had already taken `O C` as the empty level.

The lean parser (`rerail-json-lean`'s `walk`) has one field place and one continue place for each container.
`object_begin` goes to `object_field`. `object_continue` decides `,` (back to the field) against `}` (the end). A close
returns by the container's kind at the depth below.

## The decision

1. **A list `A (sep A)*` is two places.** Its head is `A`, which tests its first token there. Its cycle is one decision
   on the separator (going back to the head) and on what follows the list (going on). The list is not used first in a
   sequence whose decision found the token, or that is written with a key: its head reads the token as unfound.
2. **A bracketed level's optional middle is there.** Each call takes `O C` as the empty level (0553), so the body after
   the opener reads `M` without deciding it.
3. A close returns to the place its level's call left at its depth. With 1, that place is the continue place of the
   container below it. That is the lean parser's return by kind.
   - Later, the same day: the return places are a fixed array by depth (`ret[depth]`, `depth`), as the lean parser's
     `object[d]`, not a `Vec` pushed and popped. A level ends only inside its call, so the pop has no empty case. The
     tape test's deep nests past the depth limit pass.

## Checks

- JSON's tests (`parser_tests`, `json`) pass, including `json_rusts_tapes_are_the_jits` and `json_rust_is_current`.
- `json_rust_is_current` used to expect five value-decision sites. Its sites are now the grammar's three: the root, a
  pair's value, and an array's element. The test now expects three.
- On a Linux host, 63 tests pass:
  - the `rerail-bitlex` and `rerail-compiler` tests;
  - `rr_parser`'s `rust_tests`;
  - the `.rerail` and Kotlin `rust_is_current` tests.
- I read `tier2.rs` after the change. Each container now has one begin place, one element place and one continue place.

## Not checked, not done

- Fixes 4–7 of page 40's step 59.
- A list whose separator may be empty, and a list with a key, keep the old walk. JSON has neither.
