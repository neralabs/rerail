# 0310. The tape's reads written once, in CLIF, for three backends

Status: built (audited 2026-11-23). Decided (2026-11-16), by the user: the CPU JIT's tape reads split, the common case
in the generated code and the rare one a call, but "this needs to be some kind of more generic inlining capability,
not done by hand"; then, of where the reads' one definition lives, "Compiler IR, 3 backends". Building. Step 5 of
0293.

## Intro

Three readers of a tape exist, each written by hand:

- **Rust:** the tape crate's readers (`rerail_tape_format::f16::View`, format 15's columns). rustc inlines them into
  every Rust caller: the matcher's CPU half, the axes, the views (0293 §5, §6, §8).
- **Metal:** `metal_tape.metal`, a port of the same reads (0293 §9).
- **The CPU JIT** (`native_cpu`, Cranelift): it reads the packed batch's 44-byte node records, not the tape. It is
  the last reader of the packed batch that a product run reaches (0293, "Interim").

Cranelift cannot inline what rustc compiled: a call from the JIT's code into the tape crate stays a call, once a
read. Emitting the reads into the JIT by hand would make a third copy of every read, and a copy at every site.

## Decision

1. **The reads are CLIF functions, written once.** Each primitive read of format 15 and 16 (a node's code, its
   subtree size, a value from its side list, a start from its checkpoint and gaps, an end, a parent search, a next
   sibling) is a function built in Cranelift's IR, in a crate below the tape crate and the compiler
   (`rerail-clif`). The format's numbers (widths, block and part sizes) are its parameters or constants, as the
   format crate states them.
2. **One generic inlining pass in the JIT.** The JIT's code calls the reads by name, as ordinary calls. A pass over
   each generated function (Cranelift's `Context::inline`, with our policy) inlines a call whose callee the policy
   marks hot, from the same CLIF, and leaves the rest calls to the compiled helper. The policy is the callee's mark
   (a function written as a common case and a rare one marks the rare one cold), never a per-site choice. Any other
   CLIF helper the JIT calls gets the same pass.
3. **Three backends of the one definition:**
   - **Cranelift:** the JIT, through the pass.
   - **Rust:** `clif_rust::translate` (which already turns the parser's CLIF into Rust for rustc, `rr_aot`), moved
     into `rerail-clif`, writes the reads as Rust for the tape crate, at build time. The tape crate's API stays
     hand-written over them; rustc inlines them as it does now.
   - **Metal:** a CLIF to MSL translator beside it writes the reads for the kernels, in place of `metal_tape.metal`'s
     hand port.
4. **The hand-written readers stay the reference until each is replaced:** a test runs every generated read against
   the tape crate's own over every format's test tapes; a backend replaces a hand-written reader only when its
   reads are equal on all of them.

## Order

1. `rerail-clif`: `clif_rust::translate` moved in; the inlining pass and its policy; the reads of format 16 and 15.
2. The JIT reads the tape through them, for every packed-batch read it makes (kind, field, flags, span, parent,
   children, next sibling, subtree end); the packed batch's node records go from the CPU path.
3. The Rust backend in the tape crate; the hand-written primitive reads go.
4. The MSL backend; `metal_tape.metal`'s hand port goes.

## Progress

- **Step 1** (3b6f4355b): `rerail-clif` holds the reads of format 15 and 16 and the inlining pass. The hot reads are
  inlined, and the cold ones (`tape_f16_side`, `tape_parent`) are defined once per module and called. A test equals
  them with the tape crate's own on every node of format 16's test tapes and on format 15. `clif_rust::translate`
  has not moved in yet: it moves with step 3, its first user.
- **Step 2** (7d975e96b, 1ed1adff9, bb9787c65): every node read the JIT makes is a read call (`native_cpu_tape.rs`),
  on the document's tape where it is held. Format 15 is held in one buffer, and a 16z tape is laid out as 16 once
  (`DocTape::in_place`). The CPU path's batch (`PackedBatch::new_in_place`) packs no node records, children or
  class pairs; Metal's packed path still does, until step 5's kernels read the slabs alone. A test equals the
  reads with the document's on every format (`the_clif_reads_read_every_format_as_the_document_does`). The JIT
  finds the same keys as the interpreter and both Metal backends on real files.
- **Step 3** (90e0f7d1d): the translator is `rerail_clif::rust`. The tape crate's `F16`, `F16Z` and `F15` read
  through the reads' Rust (`tape_format/src/generated_reads.rs`; since 0534 written directly, `reads.rs`): `View`, `Sv` and the hand-written primitive
  reads are gone. Three reads were added for the crate's API: `tape_length`, `tape_start_next` and `tape_slot`
  (16z's parent steps cross its blocks). Choices made here:
  - The generated file is checked in and kept current by a test (`the_tape_crates_reads_are_these`), not written
    at build time, which would make cranelift-codegen a build dependency of the crate every crate builds on.
  - The Rust backend checks each load of the tape against its bytes, so a corrupt tape panics as the slices did.
    The JIT's and the kernels' loads are unchecked, as before.
  - The reads' offsets and spans are u32s: a reader refuses a tape or a source past 4 GiB.
- **Step 4** (ad06f0849): `rerail_clif::msl` writes the reads as MSL (`metal_tape_reads.metal`, kept current by
  `the_kernels_reads_are_these`). `metal_tape.metal`'s hand port of them is gone; its readers wrap the generated
  ones.
- **Found on the way:** the step 1 CLIF `tape_start` and the Metal hand port added a checkpoint's gaps one at a
  time, a per-byte loop at the usual one-byte gap width. The read now folds them in one word, as the tape crate's
  hand-written reader did, in all three backends.
- **A third family** (0353 step 2): a projection's numbering (`rerail_clif::ids`, `id_reads`): node ids, order keys,
  rank and select, over a descriptor in the projection's block. Its Rust is the facts crate's `id_reads.rs`, its MSL
  `id_reads.metal`; the MSL translator gained `ctz` and `popcnt`.
