# 0524. The kernels' tape writer and 16z reads are written by hand, held to the CLIF by tests

Status: built (audited 2026-11-23). Decided (2026-11-20), by the user: "I don't like how we checked in writer.h and
tape_z.h from an intermediate representation / please re-work them to be actual maintainable hand-written functions".
Amends 0310 §3 (the Metal backend is the CLIF→UKL translator) and 0434 §6 (since roll-up 8,
`rerail_clif::writer::ukl_source` writes the device's ops), for these two headers.

## Intro

Two of the UKL library's headers were the CLIF→UKL translator's output, checked in:

- `<ukl/writer.h>`, format 16's writer's ops (`w_open`, `w_close`, … `w_lz4`), 249 KB, from `writer::ukl_source`;
- `<ukl/reads/tape_z.h>`, the tape's reads for kernels that read 16z batches too, 198 KB, from
  `reads::tape_z_ukl_source`.

The translator writes each CLIF value as a numbered variable (`v117=...`), each branch as nested `if`s, and a block
reached by two paths twice. Nobody can read, review or change that text: a change is made in the CLIF and the file
written again, and a third of each file repeated earlier runs of 30 lines (the 2026-11-20 survey of the carried
headers).

## Decision

1. **Each is a header of functions written by hand**, one function an op or read, under the CLIF function's name
   (`clif_w_open` is `w_open`; `clif_tape_size_z` is the 16z body of `tape_size`), with named fields and constants,
   small shared helpers, and loops and early returns where the CLIF has them.
2. **The CLIF stays the one source for the host** (0310): its Rust and its JIT. The hand-written header is the
   kernels' copy of the same functions, and a change is made in both.
3. **Tests hold the two to each other**, on every device backend a host has:
   - the writer: `the_device_writes_the_references_tape` (R15) runs the header's ops on the device over R13's cases
     against the hand reference, which the CLIF ops are held to by `the_clif_ops_write_the_references_tape` and the
     JIT's test; `the_writer_headers_constants_are_the_ops` holds the header's layout constants (`CLIF_W_*`: the
     state's field offsets, the parts' numbers, format 16's constants, the failures) to `writer.rs`'s.
   - the 16z reads: the device's tape tests, which read 16z batches through the header against the host's reads.
4. **The generators of these go** (`writer::ukl_source`, `reads::tape_z_ukl_source`, and `reads::tape_ukl_source`
   too: `tape.h` is the same reads without the 16z bodies, so both are one hand-written `tape_body.h`) with their
   `are_these` checks. The other carried reads (`view.h`, `ids.h`, `relation.h`) stay generated for now.
5. **A wait in hand-written kernel code is a turn of its function's one loop**, and the turn's only way back is at
   the loop's foot: no `continue`, and no inner loop that polls. On Apple's GPUs the lanes of a SIMD group do not each
   go their own way. Lanes that take a way back while others of the group are still in the turn run round it again
   and again, and the others stand still. A lane waiting for a 16z block that another lane of its group had claimed
   so ran its wait to its end, its SPIN polls, while the claimant stood before its decode. The translator's output
   kept this by accident: one loop over a `switch`, every block a turn. The hand-written lookup (`clif_tc_get`) keeps
   it by its shape.

## Steps

1. **Done (2026-11-20): `<ukl/writer.h>` by hand.** 60 KB (from 249 KB). Its ops pass R15's test on the M3's Metal
   and a DGX Spark's CUDA, and on a Linux host's CPU backend; it compiles as OpenCL C 1.2 (no farm host has an OpenCL
   device). The `_Search2`/`_Search3` jumps of liblz4's `LZ4HC_compress_hashChain`, which the CLIF writes as blocks
   and jumps, are a loop over a state (`UKL_W_MAIN`, `UKL_W_SEARCH2`, `UKL_W_SEARCH3`): no backend has `goto`.
2. **Done (2026-11-20): the tape's reads by hand.** `<ukl/reads/tape_body.h>`, 63 KB, is both `tape.h` and
   `tape_z.h` (198 KB and 59 KB generated), each a wrapper that names its set. Its constants
   (`CLIF_T_*`) are held to the views' by `the_tape_headers_constants_are_the_views`. It passes the device's tape
   tests (formats 16, 16z and 15) on the Mac mini's Metal, a DGX Spark's CUDA and a Linux host's CPU, and the engine's
   resident-tape, slab and label-kernel tests on the Mac mini. The first hand-written lookup failed on Metal only:
   its waits were loops of their own, and a lane waiting for its own SIMD group's claim spun out every time (about
   1.5M failed reads, 35 to 78 s a command buffer, on the M3). Hence point 5. Once a lookup of a batch's cache has
   failed, no lookup waits any more: the batch is refused (0463), and a lookup gone wrong costs a few turns, not
   minutes of the GPU.
