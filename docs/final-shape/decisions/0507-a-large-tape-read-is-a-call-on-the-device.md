# 0507. A large tape read is a call on the device

Status: accepted (2026-11-20). Refines 0310's inlining policy for the UKL backend (decision 0440); the JIT's policy
is unchanged.

## Intro

The tape's reads are written once in CLIF (0310) and emitted for each backend. Each read carries a mark, `hot` or
cold. The JIT inlines a hot read at its call sites, and calls a cold one. The UKL backend wrote the same mark as
`UKL_INLINE` (`static inline`) or `UKL_COLD` (`noinline`), and the GPU's compiler then inlines every hot read at
every call site, together with the hot reads it calls in turn.

After the merge of the integration hub's tape reads (UKL in place of the hand-written `metal_tape_reads_z.metal`),
the transducer's interpreter kernel (`run_batch`, about 289 KB of source) took 44.7 s to build on an M5 Max, all of
it in the AGX back end's machine passes. The engine builds several variants of it. So a run with a cold kernel store
spent minutes in the Metal compiler before its matcher wave, and every bench run hit its cap.

## Decision

1. **On the device, a hot read whose body is large is a call.** `rerail_clif::ukl::translate_reads` writes a read's
   body first and counts its statements. A read the policy marks hot is written `UKL_INLINE` only when its body has
   at most `DEVICE_INLINE` (70) statements. Otherwise it is written `UKL_COLD`.
2. **Small reads stay inlined.** These are the reads a kernel calls most per node: `child`, `next`, `start`, `end`,
   `slot`, `number`, `has_kind`, `length` (59 statements or fewer).
3. **The large ones are calls.** These decode a format's lines or a node's stated span: `start_stated` (255
   statements), `line` (156), `start_next` (100), `end_stated` (92), `size` (87), `line_start` (86). The view's and
   the numbering's reads of that size are calls too.
4. **The JIT is unchanged.** Cranelift's inlining pass keeps 0310's policy as it was. The cost of inlining there is
   the JIT's own compile, which is measured apart (t37).

## Measured

`run_batch`'s source, dumped (`RERAIL_METAL_DUMP`) and built alone (`MTLDevice.makeComputePipelineState`) on an M5
Max:

| Reads inlined | Front end | Pipeline |
|---|---|---|
| every hot read (before) | 1.03 s | 44.7 s |
| all but `start_stated`, `size`, `end_stated`, `line` | 2.00 s | 10.3 s |
| none | 0.76 s | 5.5 s |
| this decision | 1.26 s | 7.1 s |

Thirty files of the bench's corpus, strict, `--metal`, a debug build, cold kernel store: killed at 100–110 s on every
run before (the store stuck at 34 kernels), finished at 148 s after (124 violations, 87 kernels).

On the M3 (ab98, 2026-11-20, 300 files, strict, `--metal`, load 5–9), t82 (every hot read inlined) against t83 (this
decision), each with a kernel store of its own:

| | t82 | t83 |
|---|---|---|
| First run, cold kernel store | 55.2 s wall | 42.1 s wall |
| Three runs at 1 GB, warm kernels: wall | 24.8, 24.3, 23.7 s | 23.7, 22.3, 23.3 s |
| Instructions (the same runs) | 536, 555, 570 G | 527, 545, 567 G |
| Violations | 463 each | 463 each |

The calls cost the kernels nothing the runs show.

On the resident interpreter (`lowered_clauses_find_the_same_roots_on_the_resident_interpreter`, M3, 2026-11-20) the
results are the same with and without this decision: 103 of 123 clauses equal, 20 refused at the launch bound, and
the same five runs that needed more chain levels. Four clauses had failed with `BackendFailure` there, with or
without it. That was not the reads: an interpreted run whose chain outgrew `label_phase`'s first level (0463) failed,
where a micro walk's run doubled the levels. It now doubles them too (`metal_label_passes`, the label sweep's retry
loop). A command buffer the GPU fails is now said where it is waited on (`rerail_metal::launch::wait_checked`), once a
run, as the joins' are.

## Tests

- `rerail-tape-format`'s `the_device_reads_read_every_node_as_the_crate_does`: every node's reads on the device,
  through `tape.h` and `tape_z.h`, equal the crate's.
- `lowered_clauses_find_the_same_roots_on_the_resident_interpreter` and the rest of `transducer::packed` and
  `device_tape` on the device (nextest, `--features metal-tests`: 297 pass).
- The headers are written again by `RERAIL_WRITE_READS=1 cargo test -p rerail-clif`; the clif crate's tests check
  that the checked-in headers are the ones the reads make.
