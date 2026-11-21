# 0526: A string's place is its method's argument; a dispatch's value is named

Status: done by the implementer. The two items are the user's (2026-11-20: "complete this": `s.part_at` "is still a
field the `"` arm sets and the string method reads. The lean parser passes the place as an argument instead", and
"the dispatch states are bare numbers (`d = 3`). The lean parser uses a named `Label` enum"). How each is done is the
implementer's choice. Follows 0519, 0521. Page 40, step 44.

## The decision

1. **The opener's place is passed to the string's method** (`rust_stream::opener_place_at_call`, `Facts::part_arg`):
   `s.string(s.start + 1)?`, with `fn string(&mut self, mut at: u32)`, as the lean parser's `string(v)`. The method
   keeps the next part's place in `at`. No arm sets a field, and `S` has no `part_at`.
   - This holds only where every part is read by a method, and every way into each method's `TOKEN` comes from an
     arm of a switch on a fresh token's first byte. Words can come between: `OPEN`, a supertype, a field, a `GOTO`, a
     `SPAN`, a `LEAF` of a fixed kind. None of them changes the token's start, so the opener's place is still
     `s.start + 1` at the call.
   - The ways are found over the words reached from the rules' entries, by their successors (`succ`).
   - Anywhere else (a part written inline, a way from a rule's entry, a call or a return), the field stays as before.
2. **A dispatch's value is named for the place it goes on to** (`Shape::tree_labelled`, in `rerail-clif`).
   - The name is its loop's label in the type's case: `'close_153: loop` is `L::Close153`.
   - One enum, `L`, holds every value the run's rules use, and each `match d` matches on its variants.
   - `Shape::tree_named` keeps the numbers, for `clif_rust`.

The names are of places, not of the grammar's meaning: `L::Token32`, `L::Span40`, `L::Switch155`, where the lean
parser has `ObjectBegin`, `ArrayValue`, `ScopeEnd`. This is a gap; see below.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`:
  - one `fn string(&mut self, mut at: u32)`;
  - at least three `s.string(s.start + 1)?;`;
  - no `part_at` in the format 15 source or the format 16 one;
  - no dispatch value a number, an `enum L`;
  - the tapes equal the JIT's, as before.
- The parser tests (89, all but the benches, Metal and the corpus writer), which include
  `aot_tests::grammars_rust_tapes_are_the_jits`: every grammar's Rust writes the JIT's tapes.
- `rerail-clif`'s tests (14).
- `rerail-rr-aot` (generated, on a farm host): 15 pass, 6 ignored.

## Not checked

- Which grammars other than JSON pass the place (`part_arg`), and which keep the field. The AOT test checks their
  tapes, not their shape.
- Nothing about speed was measured.

## Left: the gaps to the lean parser's `walk`

- **One dispatch, not three.** The rule has three nested dispatch loops (`'entries_103`, `104`, `105`). Some arms
  only pass a value on to another dispatch (`L::Span40 => { d = L::Close41; break 'to_entries_105; }`). The lean
  parser has one `match label`, of six labels.
- **The labels' names.** They name places, not what the grammar's rules do there.
- **The token's state in fields.** `start`, `rec_end`, `next`, `last_end`, `code` and `pending` are fields of `S`,
  written through `s.`, where the lean parser keeps `v`, `end` and `next` as locals.
- **Taking a token.** `token_all` clamps the index to its count and tests for the end at every token
  (`if i == n { len }`), and copies `last_end`. The lean parser's `take!()` is one load and one add: the index ends in
  sentinel entries.
- **Each node's writer call can fail** (`open16(…)?`, `leaf16(…)?`, refusing `WRITER`), and sets `pending = 0`. The
  lean parser's `open` and `leaf` cannot fail: its room is reserved once.
- **Empty containers.** The lean parser peeks the next token for `}` or `]` and writes a leaf (`empty!`). Ours opens
  and closes the node through the general path.
- **The document's ends.** The lean parser checks that the outer container's closer is the last token, once, before
  the walk. Ours tests the end at each token (`s.start == s.len`).
