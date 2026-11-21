# 0508: The writer is one Rust source written directly; a direct run's one stack is the writer's

Status: decided (the user, 2026-11-20: "complete the work", of page 40 step 38's order after 0505). Follows 0505,
0434 §6. Supersedes the translation of the writer's CLIF to Rust (`rerail_clif::writer::rust_source`,
`tape_format/src/generated_writer.rs`).

## Intro

**What was left after 0505.** A direct run's per-node writer (`W16`) was written directly. Its cold paths, the
document's end, 16z's compression and the host's writer (`f16::Writer`) were still the writer's CLIF translated to
Rust: the `w` module of every generated tier 2, and the host's own ops. Tier 2's walk also kept its own stack of
open nodes beside the writer's frames (page 40's D2).

## The decision

1. **One writer in Rust, written directly** (`tape_format/src/writer16.rs`), in simdjson's `tape_builder` and
   `tape_writer` shape:
   - the state's named fields (`State`, `#[repr(C)]`), each part a buffer at its offset from the state;
   - each cell a store at its column's offset from its block; the side lists, the block switch, `settle`,
     `finalize`, `lay`, `finish` and 16z (its pieces, its tail, LZ4 HC at level 4) as plain functions.
   The host's writer calls it (`f16::ops`). A generated tier 2 holds the same text as its module `w`
   (`WRITER_RUST`), and `W16`'s cold paths call it. The writer's CLIF stays the JIT's and the device's (R15).
2. **The state's layout is held to the CLIF writer's** (`f16::ops::layout::the_writers_state_is_the_ops`): each
   field's offset, each part's number, each constant.
3. **The translation is removed:** `rust_source`, its test, `generated_writer.rs`, and the translator's
   `translate_ops` with its unchecked-load switch. The five ops 0505 made public are private again.
4. **A direct run's one stack is the writer's (D2).** The walk keeps no stack and no node count of its own:
   - its slot for a call's number (`keep_ret`, `taken_ret`) is the writer's frame's bytes 12 to 16, which the
     frame did not use;
   - `close` and `accept` read the writer's open count, and `accept` its node count.
5. **A direct run stops at the writer's failure** (its stack is the walk's), and the entry returns 0, so the host
   reads the failure as it reads the JIT's, whose run goes on to its end. A full open part is run again with
   more depth (`write16`).
6. **The kinds' numbers are not made constants (D7), not yet.** A node's number is the language's code table's
   (`CompiledStream::numbers(&lang)`), and the language is given at run time; the tests make it per file from
   format 15's first-use table. Constants need one table per language, known when the Rust is generated. Done in
   0516: the generator takes the language, and the numbers are literals.

## Checks

- `tape_format`'s writer tests: the host's writer, now this Rust, writes the reference writer's tapes and
  presence (R13); its LZ4 is liblz4's bytes at HC 4. The JIT's ops and the device's MSL write the same tapes.
- Mutations, each failing those tests: a cell written again keeping its old side-list entry; LZ4 kept where it
  saves less than a tenth.
- `json_tests::simdjson_forms_rust_trees_are_the_jits` (a direct run, D2): its format 16 and 16z tapes are the
  JIT's and the reference's, the 400-deep document through the run again.
- `aot_tests` (json, rr, probe: window runs on the general ops), the compiler's `tier2_tests` and
  `stream_tests`, `rerail-clif`'s writer tests (the MSL unchanged).
