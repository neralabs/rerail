# 0582: Tier 2 as one loop writes a shared read once, as a method

Status: done by the implementer. The item is the user's (2026-11-21, "do number 1"). This is the second of the fixes
listed in page 40's step 59. How it is done is the implementer's choice. Follows 0581.

## What was there

`bitlex_rust::tier2_loop` wrote a rule's code inline at each place that reached it.

- JSON's `string` read was written at each of its places: a value at the root, an object's key, a member's value, and
  an array's element.
- `value`'s atoms were written at each place a value is read. An atom is a string, `true`, `false`, `null` or a number,
  with its word and its check.

The lean parser (`rerail-json-lean`'s `walk`) writes each of these once. It has a `string` method and a `primitive`
method, and each value's place calls them.

## The decision

1. **A region's read is a method** (`fn string`). It is made once, and every place that reads the region calls it. A
   method takes the next record (`ip`) and returns the token's end.
2. **A choice's alternatives that call no level are the choice's method** (`fn value`). This is the lean parser's
   `primitive`. The caller decides between a level (`{`, `[`) and the rest, and the method decides among the rest, as
   the lean parser's `walk` switches and then `primitive` switches.
3. **A node's number is the caller's** (`n[j]`). Each number the method writes with its caller's key is an argument
   (the key's field and supertypes). Each call passes the place's own numbers. The lean parser's `string(v, field)`
   takes its field the same way. Numbers that do not depend on the caller, such as a string's pieces, stay literals.

## Checks

- JSON's tests (`parser_tests`, `json`) pass, including:
  - `json_rusts_tapes_are_the_jits`, which checks that the generated parser's tapes are the JIT's;
  - `json_rust_is_current`.
- On a Linux host, these 63 tests pass:
  - `rerail-bitlex`'s and `rerail-compiler`'s tests;
  - `rr_parser`'s `rust_tests`;
  - the `.rerail` and Kotlin `rust_is_current`.
- I read `tier2.rs` after the change. It writes the string read once and the atom read once.

## Not checked, not done

- The remaining fixes of page 40's step 59 (3–7).
- The loop form is reached only by JSON's grammar, so methods were tried on JSON's grammar alone.
