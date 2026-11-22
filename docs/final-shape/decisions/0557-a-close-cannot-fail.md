# 0557: A close cannot fail where the room is reserved and every close is its open's

Status: done by the implementer. The item is the user's (2026-11-21, "please do these", the first): "an infallible
close". How it is done is the implementer's choice. Follows 0544, which did `open` and `leaf`. Page 40, step 51.
Numbered 0547 where it was written (Parser-generators-final-shape); 0557 here, the hub's 0547 being another.

## What was there

After 0544, `open16` and `leaf16` could not fail, but `close::<THIS, SURE>` still returned a result and was called
with `?`. The writer's `close` tested three things, and stored a failure where one did not hold:

- no node was open (`NO_OPEN`);
- the end was before the node's start or past the file (`BOUNDS`);
- a length or a size did not fit its block's area.

The lean parser's `close` is two stores.

## The decision

1. **Where a run is reserved (0544) and its calls nest as its nodes (`OneFn::stacked`), a close cannot fail**
   (`rust_stream::sure_closes`). Every close there closes the node its own open made, as `stacked` proves, so a node
   is open.
2. **There `S::close` returns nothing, and its calls have no `?`.** The writer's `close` asserts
   (`debug_assert!`):
   - that a node is open;
   - that its end is past its start and in the file;
   - that its length and size fit their block's reserved area, and that a block made final fits its room.

   These are the same invariants `open_node` and `leaf` assert since 0544. Elsewhere the fallible form stays.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`:
  - format 16's `close` returns nothing, and no call of it has `?`;
  - the writer's `close` asserts that a node is open, and stores no `NO_OPEN`;
  - the `,` and `}` arms' closes, as before, without `?`;
  - the tapes equal the JIT's, as before.
- `aot_tests::grammars_rust_tapes_are_the_jits` (debug assertions on): every grammar's Rust writes the JIT's tapes.
- The parser tests: 85 pass, all but the benches, Metal and the corpus.
- `rerail-compiler`'s tests: 128.
- `rerail-rr-aot` (generated, on a farm host): 15 pass, 6 ignored.

## Not checked

- Which other grammars run the form that cannot fail: their sources were not read.
- The JSON test compiles at `opt-level=3` without debug assertions, so there the invariants are checked only through
  the tapes equalling the JIT's.
- Nothing about speed was measured.
