# 0558: A return is told by the depth, then by one test

Numbered 0548 where it was written (Parser-generators-final-shape); 0558 here, the hub's 0548 being another.

Status: done by the implementer. The item is the user's (2026-11-21, "please do these", the second): "a two-way
return". How it is done is the implementer's choice. Follows 0541. Page 40, step 52.

## What was there

A call kept its number in its container's frame (`keep_ret`). JSON's calls had three numbers: 0 for the document's
`{` and `[`, 1 for a value in an array, 2 for a value in an object. A return matched on it three ways, with a fourth
arm that refused (`why::RETURN`). The lean parser's `scope_end` asks `d == 0` to end the walk, then `object[d]`: two
ways, with no refusal.

## The decision

1. **The document's own calls are found from the IR** (`rust_stream::root_number`): the calls in the document's
   words (reached from word 0, no call followed), none of them in any rule's words. They must all store one number
   that no other call stores.
2. **Their return is the one where the depth is 0 after the return's decrement.** The depth counts the calls open,
   so it is 0 there and only there. The return then reads:
   - `if s.depth == 0`: the document's place;
   - else, where two numbers are left, `if ret == a { … } else { … }`;
   - else, where one is left, its place.

   More than two left keeps the `match`.
3. **The document's own calls keep no number** (no `keep_ret(0)`). The depth tells their return.
4. **No refusal of an unknown number.** The calls that store a number are the only ones that return there (`OneFn`).

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`, in format 15 and format 16:
  - the return is `s.depth -= 1; if s.depth == 0 { … } else { if ret == 1 { … } else { … } }`;
  - no `match ret`, no `why::RETURN`, no `keep_ret(0)`;
  - the tapes equal the JIT's, and the 1025-deep refusals are unchanged.
- `aot_tests::grammars_rust_tapes_are_the_jits`: every grammar's Rust writes the JIT's tapes.
- The parser tests: 85 pass, all but the benches, Metal and the corpus.
- `rerail-compiler`'s tests: 128.
- `rerail-rr-aot` (generated, on a farm host): 15 pass, 6 ignored.

## Not checked

- Which other grammars take the two-way form: their sources were not read.
- Nothing about speed was measured.
