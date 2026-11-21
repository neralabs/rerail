# 0576. The kernels' tape reads: one body per format, the format chosen once, 16z checked once

Status: decided (2026-11-21), by the user: "we need to change it completely / include it 3 times with different
includes, once for each format / format detection happens in one place and is implicit everywhere else / move all
validation to a single place / have a single read helper (the compiler can inline it if it needs to) / I want simple
and straightforward code for each one". Amends 0524 §1 (the shape of the hand-written `tape_body.h`).

## Intro

Decision 0395 §7 measured what the kernels' compile time is made of. The tape reads written by hand in
`<ukl/reads/tape_body.h>` are composed so that every read carries every format: each public read (`clif_tape_start`,
`clif_tape_size`, …) asks the view for its format and branches to format 15's, 16's or 16z's body; each 16z body opens
its block in the rolling cache, reads it as format 16, and checks after the read that the entry was not taken by
another thread meanwhile, looking it up again if it was. A view read composes a dozen such reads, and a kernel a dozen
views. Fully expanded, the join kernels' one view read (`vread`, 1.2 KB of source) is 1.5 MB, with 74 copies of the
cache's lookup and 148 of format 16's value read. On an Arc 140V that one kernel compiled in 45 s, and 2.7 s with
format 16's reads alone.

## Decision

1. **One body per format, each plain.** Format 15's reads (`tape_f15.h`) and format 16's (`<ukl/body/tape_f16.h>`) are
   each written once, for their format alone: no format test in them. Format 16's body is included twice: over the
   tape in place (`clif_f16_*`) and over a 16z block decoded into the cache (`clif_f16b_*`); 16z's reads
   (`tape_f16z.h`) are each that body's read of the node in its block. The reads made of others (a node's end, its
   subtree's end, its next sibling, its children) are written once (`<ukl/body/tape_api.h>`) and included once per format
   (`clif_f15_*`, `clif_f16_*`, `clif_f16z_*`).
2. **The format is told in one place.** `tape.h` (formats 15 and 16) and `tape_z.h` (16z too) define the public reads
   `clif_tape_*` by one macro: the view's format, then that format's read. Nothing below it asks. A kernel that knows
   its format calls that format's reads by name.
3. **One read helper each.** Every byte a format 16 body reads is read by its one helper (`ld`: `w` bytes at an
   offset), the tape's in place or the entry's in the cache (held to the entry's size there). A 16z read finds its
   block by one helper too (`clif_z_block`). The compiler inlines either where it chooses.
4. **16z is checked in one place.** A cache entry is cleared only after the cache's epoch moves on (a full head turns
   the spare into the head first). So a 16z read stands when the epoch is the same after it as before; else it is
   read again, `TRIES` times at most, then a failure is counted (the run is refused, decision 0463) and, with a
   failure counted, every block read reads the dead entry's zeros. The public reads' macro is the one place that
   checks, once per read. The cache's own protocol (lookup, take, decode, the waits of 0524 §5) is unchanged.
5. **The host's reads are unchanged** (the CLIF, decision 0310), and the same tests hold the kernels' reads to them:
   `the_device_reads_read_every_node_as_the_crate_does` reads every node of each format through both headers;
   `the_tape_headers_constants_are_the_views` reads the constants from `tape_common.h`.

6. **The view reads the same way** (`view.h`, generated from the CLIF by `view_ukl_source`). A view read that reads
   the tape (itself or through another view read) is written once, into `<ukl/body/view_reads.h>`, with its tape reads
   named `CLIF_F(end)`, `CLIF_F(parent)`, … and the view reads it calls `CLIF_V(view_x)`; `view.h` includes that body
   once per format (`clif_f15_view_*`, `clif_f16_view_*`, `clif_f16z_view_*`), and each view read keeps its name and
   signature as the one place its format is told: format 16's read, 16z's under the one check, else format 15's. So a
   view read of a 16z tape is checked once, not each tape read it makes. The reads that read no tape (the lines', the
   numbering's) are written once.
7. **The check is one macro**, `CLIF_Z_READ` (`tape_f16z.h`), which the tape's public reads and the view reads use.
