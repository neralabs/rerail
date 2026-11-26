# 0562: The `.rerail` parser's tier 2 is written from the grammar

Status: done by the implementer. The direction is the user's (2026-11-21): the `.rerail` parser's Rust needs the
new generator to cover the grammar (0550). How it is done is the implementer's choice. Follows 0560, which wrote tier
1 and left tier 2 out.

## What was there

Kotlin's recursive generator (`bitlex_rust::tier2`) refused `rr.bitlex` at its first `drop` (`test_tail`). It was
given the tree as parsed, while the build desugars `any` and `balanced` first. It read only the nodes, choices,
flags and readings, so it ignored `remember`, `provisional` and `refuse … in …`. It had no `relabel`.

## The decision

1. **The generator reads the tree as the build does** (`grammar::desugared`, `Lowering::tree_desugared`): each
   rule's `any` and `balanced` desugared, and each `balanced` level a nested choice. The level's body is one function
   (`balanced_level`), called by the build and by `desugared`.
2. **`relabel F as M inner G`**, as the machine's `NODE_FIELD` and `RETAG` on the node made last (modes 2 and 3):
   - `inner G`: the field of the node its wrap holds first;
   - `F`: its field;
   - `M`: its kind.

   **`drop`** (kind `GONE`) takes the node out, and the nodes inside it take its place (`reread`'s rewrite, the node
   gone). Then no node is made last at the level, as in the reference.
3. **A keyword named by its word** (`any`'s tokens, as the build names them: `#include`) is that word's test.
4. **`A as K` over a choice of tokens** (`aggregate_word as variable`) is one decision, an arm a token, each a leaf of
   kind `K`.
5. **`remember C`**: a token of choice `C` sets its level's word (`P::word`). A call keeps the caller's word, and
   the caller gets its own back at the return, as the machine's `PUSH`. `as C` in a wrap, a relabel or a reading is
   that word.
6. **`provisional F`**: a node still holding `F` when the root closes refuses the file.
7. **`refuse M in L`**: a token taken by a level in `L` that carries mark `M`'s bit refuses the file. A token another
   level takes does not. `P::refusing` holds the level's set, the callee's own for a call and the caller's again at
   its return.
8. **A field id past a key's 6 bits** (`rr_parser`'s own ids, from `0x4000`: `bare`) is given a slot from the top
   of the 6 bits down, the same in the rules, the readings and the provisional test. A grammar whose fields reach the
   slots is refused.
9. **A `whole` root starts at byte 0**, as the syntax defines it and both JITs open it (`OPEN` before any token).
   The generator started it at its first token or comment, as Kotlin's lean parser does. Kotlin's generated
   `tier2.rs` changes in that one place. Its 48 tests pass, `rust_is_current` among them.
10. **Each runtime part a grammar needs is written only into a parser that uses it**: the helpers that only some
    grammars' rules call (`MORE`), and the fields of `remember` and `refuse`. They go in by anchored replacements
    in the runtime, and a missing anchor is an error. Kotlin's and JSON's output is otherwise unchanged.
11. **`rerail-rr-aot` holds tier 2** (`tier2.rs`) and `parse`.

## Checks

- `rust_tests::tapes_are_the_jits`: on the samples and the made cases, 28 tapes are the JIT's node for node (key,
  start, end, size), and 20 files are refused by both. The cases include bad escapes in a clause
  and in a policy's JSON document (`json_bad`).
- `corpus_tapes_are_the_jits`: 1784 files, 1779 parsed, every tape the JIT's; the other 5 refused by both.
- `corpus_tier1s_records_are_the_interpreters` and `tier1s_records_are_the_interpreters` still pass.
- `rerail-kotlin-parser`'s suite (on a Linux host): 48. JSON's tests: 9, `json_rust_is_current` among them.
  `rerail-compiler`'s tests: 151.

## Found on the way

- The tape comparison found the root's start in a file that begins with whitespace (decision 9).
- The comparison of a policy's document with a bad escape found that `refuse … in …` was ignored. The generated
  parser took the file the JIT refuses.

## Not checked, not done

- Speed: nothing was measured.
- The generated parser's writer is given no supertype classes. `rr.bitlex` has no supertypes, so a grammar with
  them was not tried.
- Kotlin's corpus test (`RERAIL_KOTLIN_CORPUS`) was not run, so a Kotlin file that starts with whitespace was not
  seen through the change to the root's start.
- `fold` is in the generator's first tokens but not written; `rr.bitlex` has none.
- The loop form (`tier2_loop`) still refuses `rr.bitlex`, at its marks, extras, flags and readings.
- `relabel … inner G` checks only that the node made last has a child. The JIT also refuses where that node is no
  wrap, or holds a wrap first.
