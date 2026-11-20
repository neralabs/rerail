# 0518. The kernels' tape writer and 16z reads are written by hand, held to the CLIF by tests

Status: decided (2026-11-20), by the user: "I don't like how we checked in writer.h and tape_z.h from an intermediate
representation / please re-work them to be actual maintainable hand-written functions". Amends 0310 §3 (the Metal
backend is the CLIF→UKL translator) and 0434 §6 (since roll-up 8, `rerail_clif::writer::ukl_source` writes the
device's ops), for these two headers.

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
4. **The generators of these two go** (`writer::ukl_source`, `reads::tape_z_ukl_source`) with their `are_these`
   checks. The other carried reads (`tape.h`, `view.h`, `ids.h`, `relation.h`) stay generated for now.

## Steps

1. **Done (2026-11-20): `<ukl/writer.h>` by hand.** 60 KB (from 249 KB). Its ops pass R15's test on the M3's Metal
   and spark1's CUDA, and on a Linux host's CPU backend; it compiles as OpenCL C 1.2 (no farm host has an OpenCL
   device). The `_Search2`/`_Search3` jumps of liblz4's `LZ4HC_compress_hashChain`, which the CLIF writes as blocks
   and jumps, are a loop over a state (`UKL_W_MAIN`, `UKL_W_SEARCH2`, `UKL_W_SEARCH3`): no backend has `goto`.
2. `<ukl/reads/tape_z.h>` by hand.
