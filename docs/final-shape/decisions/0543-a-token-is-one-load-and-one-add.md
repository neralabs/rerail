# 0543: A token is one load and one add

Status: done by the implementer. The item is the user's (2026-11-20, "address each of these in turn", the fourth):
"token_all does extra work on every token. It clamps the index, tests for the end and copies last_end. The lean
take!() is one load and one add, because its index ends in sentinel entries." How it is done is the implementer's
choice. Follows 0542. Page 40, step 47.

## The decision

1. **A stream ends in two entries of the file's length**, past its count, as simdjson's stage 1 ends its index.
   - The CPU's tier 1 writes them after its passes (`Batch::end_streams`), in the extract's slack, which already
     held two slots past the count.
   - The copying harness (`run_raw`) pads its copy with them in place of `[0, 0]`.
   - Metal's tier 1 does not write them. Its tier 2 is the JIT's, which still clamps.
2. **The Rust's `TOKEN` is one load and one add** (`t.start = *starts.add(t.next * rs); t.next += 1;`), with no
   clamp and no select. This holds where no way takes more than two tokens past the count (`ends_suffice`), and
   the walk checks it:
   - past the count a token's start is the file's length, whose byte (padding) every switch refuses but at its end
     arm;
   - from each take, the ways on through the end arms alone are counted;
   - a string's place scans its bytes and takes nothing from the index.
   Elsewhere the clamped form stays.
3. **No copy of the last token's end at each token** (`last_ends`), in a direct format 16 run. A `TOKEN` leaves the
   last end in `rec_end`, and an arm makes its own token's end there only where a word reads it (`end_read`, 0526).
   - A close of the last end reads `t.rec_end` where no arm made it again on any way in. That covers the `,` and
     `:` arms in JSON.
   - Otherwise the close reads `t.last_end`, which the writing arm keeps just before it makes its own. In JSON,
     that is `}` and `]` after a value: `t.last_end = t.rec_end; t.rec_end = t.start + 1;`.
   - An arm keeps it only where every state into its switch has the last end in `rec_end`. A walk over the words
     checks this, with each arm's places taken from `fetched_arms` itself (`ArmGroup`).
   - Where some reader would have neither field on some way in, the copy stays in `TOKEN`. The same holds where the
     run keeps the end elsewhere: a deferred leaf, a part's `keep`, `TOKEN`'s unmade end, or outside a direct
     format 16 run, where every arm makes its end and an arm's copy would be one per token.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`:
  - the format 16 `token_all` is exactly the load and the add;
  - a `,` arm closes with `t.rec_end`;
  - the `}` arm keeps, then makes its end, then closes with `t.last_end`;
  - no other arm keeps;
  - the tapes equal the JIT's, as before;
  - new: fifteen files cut off in a container (`[1,`, `{"a":`, `[[1,`, …) and the empty file. These take past
    the count, and the Rust refuses each, as the JIT does.
- With ends of 0 written in place of the length (once, then put back), this test fails on the valid files: the end
  is never seen.
- The parser tests: 85 pass, all but the benches, Metal and the corpus, among them every grammar's Rust tapes
  against the JIT's.
- `rerail-compiler`'s tests: 127.
- `rerail-rr-aot` (generated, on a farm host): 15 pass, 6 ignored.

## Not checked

- Metal with the Rust tier 2: Metal's buffers have no ends, and no Metal path runs the Rust.
- Whether the compiler keeps `t` in registers: no machine code was read.
- Nothing about speed was measured.
