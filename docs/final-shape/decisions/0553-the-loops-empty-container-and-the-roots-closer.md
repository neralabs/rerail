# 0553: The loop's empty container is one leaf, and the root's closer is checked first

Status: done by the implementer. These are 0545's and 0579's items (the user's, 2026-11-20) redone in the form 0551
writes; the user asked for them in that form (2026-11-21). How they are done is the implementer's choice. Follows
0552. Page 40, step 56.

## The decision

1. **A level's brackets come from its rule.** A nested rule whose body is a token of one byte, then parts that may
   all be empty, then a token of one byte (`brackets`) has an opener and a closer. In JSON these are `object`'s
   `{` `}` and `array`'s `[` `]`.
2. **The empty container is one leaf at its call**, as the lean parser's `empty!`. After the depth's test (the JIT
   counts an empty container's depth too), where the record is the opener and the next is the closer, the call
   writes one leaf of the level's number from the opener to past the closer, takes both, and goes to the place the
   call returns to. A leaf, and an open and a close with nothing between them, are the same cells (0545).
3. **The root's closer is checked before the walk.** Where a level's call returns straight to the root's end (its
   return place is the one made of the root's end), the call first tests that the file's last record is the
   level's closer, as the lean parser's `last != '}'`. Else it refuses, as the walk would.
4. **A decision reads a token's word only where the word picks an arm.** Where a row's words and its own test all
   go to one arm, that arm's bytes are the row's (`decide`). Before, an arm whose tests were all of `value`'s was
   written once for each word of the number row (`true`, `false`, `null`, and the rest): four copies at each
   `[value …]`. `tier2.rs` went from 2296 lines to 1435.

## Checks

- `json_tests::json_rusts_tapes_are_the_jits`: 21 more files: the empty containers in each place, with white space
  inside and around, and files whose last record is not the root's closer (`[1] 2`, `{} {}`, `[]]`, `{}}`, `[1]]`,
  `{"a":1}]`, `[1`, `{`, `[`, `[][`). 196 tapes the JIT's, 106 refused by both.
- `json_tests::json_rust_is_current`: every call of `object` and `array` peeks for its closer (5 value sites,
  the root's among them), and each closer is checked before the walk once.
- JSON's tests: 24 (`json_`).

## Not checked

- Nothing about speed was measured.
