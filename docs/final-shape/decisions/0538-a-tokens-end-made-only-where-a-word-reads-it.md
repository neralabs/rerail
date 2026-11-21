# 0538: In a direct run, a token's end is made only where a word reads it

Status: done by the implementer, as the next gap between the generated JSON Rust and the lean parser's `walk` (the
user, 2026-11-20: "go back to your actual useful work", after 0537). Follows 0537. Page 40, step 43.

## Intro

**The gap.** Every arm of a switch on a token's first byte made the token's end: `s.rec_end = s.start + 1;` for each
`,`, `:`, `{`, `[`, `}`, `]` and `"`. The lean parser keeps no such field. It computes an end where it writes one
(`w.close(sink, ord, at(v) + 1)`), and a `,` or a `:` writes nothing.

## The decision

`rust_stream::end_read(x, t)`: whether the end an arm makes is read from its place `t` on. It applies in a direct
format 16 run, without `flagged`. It walks the words twice over: once for the end itself (`rec_end`), and once for the
end a `TOKEN` passes on as the last token's (`last_end`). A walk stops where something makes that end again.

- **Reads the end itself:** a string, a `CLOSE` of its own token's end, a `LEAF`, a `CHECK` and a length's `SPAN`.
- **Reads the last token's end:** a `CLOSE` of the last token's end, and a deferred `LEAF`.
- **Makes the end again:** every arm of a fetched switch on a fresh token. A second `TOKEN` makes the last one again.
- **Read by nothing:** `OPEN`, the node words, `GOTO`, a last-byte `SPAN`, `COUNT`, `EMPTY`, and calls and returns,
  which go to their callees and places.
- **Anything else is taken as a read.** Outside a direct run, every end is read.

The walk follows the words, not the rule graph's `succ`, which writes each switch's arms and so asked this again.

A one-byte arm makes its end only where `end_read` holds. In JSON, `,`, `:`, `{` and `[` make none. `}` and `]` make
theirs, which the close of their container reads.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`: no arm of `,`, `:`, `{` or `[` makes its end, every arm of
  `}` and `]` does, and the tapes equal the JIT's.
- `aot_tests::grammars_rust_tapes_are_the_jits`: every grammar's Rust writes the JIT's tapes.

## Strings (done after)

- A string copies the end it is given as the last token's (`s.last_end = s.rec_end;`), then makes both again at its
  parts. The walk takes the copy on, as the last token's end, at the closer's place, and nothing else.
- An opener's arm (`"`) makes its end only where `end_read` holds. In JSON, none does.
- The string's copies of the last token's end, at its head and after each content part, are written only where a
  word after the closer reads them (`end_read_from(closer, last)`). In JSON, none is.

Checked by the JSON test (the tapes equal the JIT's) and the AOT test.

## Left

- `s.part_at` stays a field the opener's arm sets and the string reads, where the lean parser passes the place as an
  argument (`self.string(v)`). Words can come between the arm and the call (an `OPEN`, a field), so the call does not
  know the opener's place without the field.
