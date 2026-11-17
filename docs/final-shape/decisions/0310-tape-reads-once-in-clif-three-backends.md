# 0310. The tape's reads written once, in CLIF, for three backends

Status: decided (2026-11-16), by the user: the CPU JIT's tape reads split, the common case in the generated code
and the rare one a call, but "this needs to be some kind of more generic inlining capability, not done by hand";
then, of where the reads' one definition lives, "Compiler IR, 3 backends". Building. Step 5 of 0293.

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
4. The MSL backend; `metal_tape.metal`'s hand port goes. The device's document entry is then one definition with `TapeView`: until then it is two
   declarations of one layout, `rerail_metal::slabs::ChunkTape` (0289 §4's generated MSL; `rerail-blocks` is not
   built for WASM, where the tape crate and `TapeView` are) and `TapeView`, asserted equal field for field. Their
   first field is the document's place in the chunk block (`at`), and a read's base is the document's start.
