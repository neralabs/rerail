# 0545: An empty container is one leaf

Status: done by the implementer. The item is the user's (2026-11-20, "address each of these in turn", the sixth):
"Empty {} and [] go through the general open/close path. The lean parser looks at the next token and writes a
single leaf." How it is done is the implementer's choice. Follows 0544. Page 40, step 49.

## The decision

1. **Found from the IR, not from JSON's names** (`rust_stream::empties`). A container qualifies where all of these
   hold:
   - its `OPEN`, past its key's words (`NodeSuper`, the field), is a call;
   - the call's first words are a plain `TOKEN` that takes and a fresh byte switch on it;
   - that switch's arm (its own, or the other switch's it takes on) of exactly one byte is a one-byte token whose
     code no word reads;
   - that arm goes straight to the container's `CLOSE` of its own end, then the `RETURN`.

   It applies only in a direct format 16 sentinel run of one function with its stack. In JSON these are `{`'s `}`
   and `[`'s `]`.
2. **The opener's arm looks at the next token first**, as the lean parser's `empty!`. Where its byte is the closer,
   the arm:
   - writes one leaf of the container's number, from the opener's start to the closer's end;
   - takes the closer as the call's switch would take it;
   - goes on at the call's return place.

   Else it opens the container as before. The return place is a second successor of the `OPEN`, so the shaper and
   the analyses (`last_ends`) see the edge.
3. **The empty path does what the general path does to the run's state.** It tests the depth as the call does: our
   JIT counts an empty container's depth, and the Rust's refusals stay the JIT's. simdjson and the lean parser do
   not count it. It makes `rec_end` as the closer's arm does, and keeps the last end where that arm keeps it. The
   peek reads `starts[t.next]`. Past the count that is one of the stream's two ends (0543), the file's length, whose
   byte (padding) is no closer.
4. **The tape is the same.** A leaf, and an open and close with no child between, write the same cells (the code,
   the gap, the checkpoint, the length; no slot for a size of 1). The new comparisons below check this.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`:
  - every `[` and `{` arm of the direct run peeks for its closer before anything but its enclosing nodes' opens;
  - each such arm adds one node call to format 15's count;
  - new: the Rust's format 16 and 16z tapes equal the JIT's for `[]`, `{}`, `[[]]`, `[{}]`, `{"a":[]}`,
    `{"a":{}}`, `[[],{},[[]],{"b":[]}]`, `{"a":[],"b":{},"c":1}`, `[ ]` and `{ }`;
  - the depth refusals (1025 nested) are unchanged;
  - the tapes equal the JIT's on the test's files, as before.
- The parser tests: 85, all but the benches, Metal and the corpus, with every grammar's Rust tapes against the JIT's
  (debug assertions on, 0544).
- `rerail-compiler`'s tests: 127.
- `rerail-rr-aot` (generated, on a farm host): 15 pass, 6 ignored.

## Not checked

- Which other grammars' containers take the empty path: their sources were not read.
- Nothing about speed was measured.
