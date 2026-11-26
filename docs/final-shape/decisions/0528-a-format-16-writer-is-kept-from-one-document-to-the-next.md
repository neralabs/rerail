# 0528: A format 16 writer is kept from one document to the next

Status: built (audited 2026-11-23). Decided (the user, 2026-11-20: "go ahead in that order", the first of page 40 step
34's three changes). Amends 0434's writer (`rerail_tape_format::f16::Writer`).

## Intro

**The problem.** Page 40's step 34 measured the simdjson form end to end against simdjson. About 30% of our time on
`twitter.json` was the format 16 writer zeroing memory. Each document had a new writer, and its parts were sized for the
worst case and zeroed by `Vec::resize`: about 3 MB for a 631 KB file. simdjson's caller keeps one parser, and its tape,
from one document to the next.

## The decision

1. **`Writer::begin` starts the next document in a writer.** It keeps every part's buffer and grows only a part that is
   too short, so a writer used again zeroes no room. `Writer::new` is `begin` on `Writer::empty()`.
2. **The ops write each byte before they read it,** except in two parts, which `begin` sets for each document:
   - the presence's words, which each node's bits are or-ed into (zeroed: one word a kind and a field, a block's set
     each);
   - in 16z, the seen pieces' buckets (set empty) and the LZ4 tables (zeroed).

   In tests, `begin` fills every other part it keeps with `FRESH` (`0xa5`). A byte the ops read before they write then
   shows. Filling the presence or the LZ4 tables that way fails the tests, which is how those two were found.
3. **`finish` takes `&mut self`.** The tape still moves out with the result, as before. **`Writer::recycle(tape)`** takes
   it back as the next document's room. It does so without zeroing, when it is the same allocation `finish` gave out.
4. **Tier 2's format 16 run takes the caller's writer** (`Parser::image16_in`). `image16` is `image16_in` with a new
   writer.

## Measured

`json_tests::simdjson_form_against_simdjson`, `--profile profiling`, arm64, load about 17, instructions a byte:

| File | A writer a document | One writer, the tape given back | simdjson |
|---|---:|---:|---:|
| `twitter.json` | 59.9 | 48.7 | 5.8 |
| `numbers.json` | 56.2 | 44.6 | 14.2 |

Ours against simdjson's time: 13.5× → 9.4× on `twitter.json`, 4.1× → 3.4× on `numbers.json`. In a profile, the zeroing
is gone: `__bzero` went from 1,295 samples to 83.

## Checks

- `f16::tests::a_writer_used_again_writes_the_references_tape` runs one writer through every case of
  `writes_the_references_tape` in turn: 16 and 16z, larger and smaller, with block presence and without. Each tape and
  presence is the reference writer's. Without the presence's zeroing in `begin`, it fails.
- `f16::tests::a_tape_given_back_is_the_next_tapes_room`: the next tape is written in place in the room given back, and
  equals a new writer's tape.
- All 34 of `rerail-tape-format`'s tests pass with every kept part filled with `FRESH`. The JSON tests and the AOT
  ladders (JSON, the probes) pass.

## Re-decided by shape (2026-11-20)

The user: no decision by cycles, instructions or time; the generated Rust like simdjson's and the lean parser's. Kept
by shape: simdjson keeps its `parser`, its buffers allocated once, and the lean parser reserves its columns once
(`reserve_columns`). The counts above are information, not the reason (page 40, step 38).
