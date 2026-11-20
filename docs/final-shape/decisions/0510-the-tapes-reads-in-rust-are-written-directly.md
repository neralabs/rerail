# 0510: The tape's reads in Rust are written directly

Status: decided (the user, 2026-11-20: "please address these", of 0508's report, where the tape's reads were still
Rust translated from CLIF; and earlier, "we should not be translating Rust from CLIF. We should be generating it
directly"). Follows 0508. Supersedes 0310 step 3's generated file (`tape_format/src/generated_reads.rs`,
`rerail_clif::reads::rust_source`).

## Intro

**What was left after 0508.** The writer was written directly, but this crate's readers (`F16`, `F16Z`, `F15`,
`StoredPresence`) still read a node through the tape's reads in CLIF translated to Rust. That file held one numbered
function per CLIF block graph, its locals `v0`, `v1`, … and a checked pointer load (`ld`) for each load of the tape.

## The decision

1. **The reads are written directly** (`tape_format/src/reads.rs`), as a hand-written reader of the format reads
   it. Modules `f16` and `f15` hold the reads the crate's readers call, and `has_kind` is shared:
   - each load is a slice's (`u8_at`, `u16_at`, `u32_at`, `word_at`), and a load past the bytes panics, as before;
   - a block's column, a value with its escape to the side list, a checkpoint, and the gaps summed by folding a
     word are each one small function;
   - the side list's search and the newline index's select are `#[cold]`, as the JIT keeps them calls.
   The reads hold each value where the next step needs it held, as the CLIF does (decision 0403): a size is at
   least 1, spans lie in the source, a parent's step moves back.
2. **The JIT holds them equal.** `f16::tests::the_rust_reads_are_the_jits` compares every read the crate has with
   the CLIF read the JIT compiled: every node of a format 16 tape whose side lists are all in use (size, number,
   stated and held spans, length, parent, slot, a scan's next start), every byte's line, every line's start,
   every kind in every block, and every read of a format 15 tape. The test of the crate's readers against the JIT
   (`the_clif_reads_read_every_node_as_the_crate_does`) stays.
3. **The reads stay written once in CLIF for the JIT and the kernels** (0310): the CLIF is the JIT's and MSL's
   source, and the Rust is a reader of the same bytes, held to it by the test.
4. **The translation of the tape's reads is removed:**
   - `rust_source` and its test (`the_tape_crates_reads_are_these`);
   - `generated_reads.rs`;
   - `public`, and the translator's tape parts: its tape entry functions, its `ld` prelude and its checked load.
   The translator keeps the relations' reads (`rerail_blocks::generated_reads`) and the numbering's
   (`facts/vtab/syntax/id_reads.rs`). Both were written again without the prelude they never used.

## Left

- The relations' and the numbering's reads in Rust are still translated from their CLIF (`rust::translate_reads`).
  They are next, in the same way: written directly, held to the JIT by a test.
