# t42: the tape as every reader reads it: what is still a translation, and the final shape's choices

Surveyed 2026-11-17 at the hub (968fa0dc6) and the pinned tips of the three branches that change it: oscar
(sync-integration-hub-tape, f15aac19d), final-rerail-shape (8e2b232ad) and Native-and-GPU-strata (a3f9c49f2).
Nothing here is measured anew; the sizes are 0156's (E24/E25).

## Intro

The user's invariants (2026-11-17):

- the tape is written and read **in its stored format**, the same bytes on the CPU and the GPU;
- **no decoding or translation** into another format on the way to a reader;
- the readers (MSL, Rust, the Cranelift JIT) share **accessors defined once**, inlined where the backend can.

The third already holds on oscar's line: the reads (`tape_size`, `tape_start`, `tape_next`, …) are CLIF
functions written once (`rerail_clif::reads`), printed as Rust for the tape crate, as MSL for the kernels, and
inlined by the JIT's pass when hot (decision 0310 steps 1–4).

The roll-up preview offered to choose between the branches' spellings of `TapeView`. That was the wrong question:
the four lines agree on what `TapeView` is (below), and differ only in how its MSL declaration is printed. The
question is what still stands between a stored tape and its readers, and which of it the final shape keeps.

## What `TapeView` is on every line

A 112–116 byte record per document: u32 offsets to the tape's regions (format 15's five columns, or format 16's
five arrays, four side lists and their part starts), the widths, the language's cutoff and prefix numbers, and the
document's place in a batch (its block offset `at` or `slab`, its source, line starts and code table, and its
first byte and line in the batch's numbering). The reads take `(base, view, node)`.

| line | how the record is declared | its MSL |
|---|---|---|
| hub (my oscar merge, 119357fc1) | `ChunkTape`, a `structure!` of `at` and the view's fields, offsets asserted equal to `TapeView`'s | `msl_of` |
| oscar | `TapeView`, plain `repr(C)` with `slab` | the kernels' `DocView`, hand-declared |
| final-rerail-shape | `TapeView` with `at` (0289 §4) | `view::msl()`, printed from the fields with offset asserts |
| Native-and-GPU-strata | `tape_view_fields!` makes `TapeView` and `ChunkTape` from one list | `msl_of` |

None copies or re-encodes node data. They differ in where the declaration is printed from.

## What is still a translation

Each of these sits between the stored bytes and the reads today (oscar's line; the hub and the other two match):

1. **The view is derived, not stored.** A stored format 16 header holds each region's *length* (`head::LENGTHS`),
   not its start. `f16::placement` computes the starts by a prefix sum and checks the side lists' part starts
   (once per tape loaded: `Tape16::from_bytes`, and once per placement on the device). `bind` then adds
   **the language's current** cutoff, prefix numbers and slot/gap/length widths (`Lang::device`): they are not in
   the tape. A tape is read with whatever numbers its language has now, not the ones it was written with.
2. **A 16z tape is decoded at placement.** `residency/slabs.rs` lays a 16z tape out as format 16 in its slab
   (`Tape16Z::unblock_into`): the device never reads 16z. On the CPU, a 16z block is decoded into the thread's
   cache and read there (`Tape16Z`).
3. **Line starts are made at placement.** `residency/slabs.rs:100,147` counts newlines byte by byte on the host and
   writes a u32 array per document (a per-byte loop, and a derived array).
4. **The code table is translated per document.** `resident::device_codes_of` builds a 24-byte `DeviceCode` per
   code from the image's table and the grammar, written into each document's slab space, though a format 16
   tape's codes are numbered in its *language's* table.
5. **Format 15 and 16 behind one read.** The MSL reads are built for either format and test `view.format` on every
   read (`reads::by_format`); the Rust reads are built per format (`build_for`). Format 15 stays for a language with
   no format 16 table (`.rr`), or a document with a code its table lacks (`tape/formats.rs`, `keeps_15`).

Sizes (0156, bytes a node): format 16 in memory 4.5–4.8; format 15 8.5–9.9; 16z (LZ4 HC + dictionary, blocks of
1,024, with the index) 1.2–2.4; zstd 3 + dictionary over format 16's whole arrays 0.9–2.1, smaller than 16z on every
language measured. Loading from 16z costs 4.6–5.6 ns a node, from zstd 9–14 (upper bounds).

## The choices

Each axis is independent. Within an axis, the scenarios are alternatives.

### A. Where the view comes from

- **A1: derived at load (today).** One record per document, built from the header and the language. Costs O(1)
  per document plus the part-start check. It is a translation of the header, and it reads the language's numbers
  at read time, so a language table change can misread stored tapes unless the store's keys catch it (not checked
  here).
- **A2: the stored header is the view.** The writer writes the region starts (tape-relative u32s), the widths and
  the cutoff and prefix numbers into the header, in the layout the reads take. A reader is given the tape's first
  byte, and the header is the view's tape half. The batch keeps a per-document entry for what is the batch's,
  not the tape's: its block offset, its source, and its first byte and line in the batch's numbering. Nothing is
  built. A tape carries the numbers it was written with, so (1)'s hazard goes. Cost: a format revision of 16 and 16z
  (stored tapes are written again; a store built before reads cold), about 40 bytes more a header (noticeable
  only for documents of a few dozen nodes). The reads change only in where `at::` points.
- **A3: no view; the reads take the header's lengths.** Each read (or each kernel's entry) sums the region lengths.
  It keeps the format unchanged but puts a 9-term sum before the first load of every read that is not hoisted.
  Not recommended: A2 gets the same "read the stored bytes" with no work.

### B. The parts a reader needs beside the tape

- **B1: made at placement (today).** Line starts by a host byte loop per placement; a `DeviceCode` table per
  document.
- **B2: written with the tape.** The parser writes the line starts as a tape region. Lean lexers already find line
  ends in their masks, so it is a popcount and compress per word, not a byte loop. The code table is the language's,
  placed once per batch per language and named by the header's language tag. Nothing is derived at placement; a
  tape and its source are all a document is.
- **B3: line starts on demand.** No array; a line is found from the source's newline masks when a rule asks for
  one. Saves the region (4 bytes a line), costs a word scan per question. Cheaper to store than to re-answer
  unless line questions are rare.

### C. 16z

- **C1: decoded at placement (today).** The device holds format 16 (4.5 B a node). LZ4's random access is unused
  there; 16z buys only faster loading than zstd (5 vs 9–14 ns a node).
- **C2: resident compressed, decoded per block in the kernel.** A threadgroup decodes a 1,024-node block (about
  4.6 KB as format 16, inside 0289's threadgroup budget) and runs the same reads over it. Half the device memory
  and bandwidth (2.3 B a node). LZ4 is serial within a block, and reads that leave a block (a parent, a subtree's
  end) need another block decoded. The CPU already reads 16z this way. Needs a measurement before it is chosen.
- **C3: no 16z in memory.** On disk, zstd over format 16 (smaller than 16z everywhere measured); in memory, format
  16 read in place on both sides. One in-memory format, one decode only at the disk boundary (the user's "zstd only
  on disk"). The cost: loading is about twice 16z's (still under 15 ns a node), and a document larger than memory
  has no compressed in-memory form.

### D. Format 15

- **D1: one read for both, a branch per read (today, MSL).** Uniform within a document; a SIMD group mixing
  formats diverges.
- **D2: kernels built per format.** `build_for` already does it for Rust; a batch is split by format, and each
  pipeline reads one. Two pipelines per program.
- **D3: format 15 retired for documents.** Every language gets a format 16 table (`.rr` is the one without), and a
  code missing from its table is added at write time, not a reason to fall back. One format and one read; the
  view loses format 15's fields.

## Recommendation

**A2 + B2 + C3 + D2 now, D3 when `.rr` has its table.** That puts the stored tape and its source in front of every
reader with nothing built from them. It keeps one in-memory format, read through the one set of CLIF reads on
every backend, with the batch's own facts (where a document lies, its place in the numbering) the only per-batch
record. C2 is the alternative if device memory or bandwidth binds. On a 1 GB budget, 10 M nodes are 45 MB as
format 16, so it does not bind yet; measure it before choosing it.

What is unmeasured and would change the choice: (a) how often line questions are asked against the region's cost
(B2 vs B3), (b) the device's residency pressure at the data-layer scale (C3 vs C2), and (c) how many small
documents there are, against A2's larger header.

## For the roll-up

None of the four lines builds A2–D3; each is A1 + B1 + C1 + D1. The merge keeps oscar's reads (the one
definition), takes `at` (0289 §4) as the block offset, and declares the record once (the spelling that prints its
MSL from the Rust fields). The final shape's steps go on the work queue as their own items, not into the merge.
