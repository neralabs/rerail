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
   named `CLIF_F(end)`, `CLIF_F(parent)`, …; `view.h` includes that body once per format (`clif_f15_view_*`,
   `clif_f16_view_*`, `clif_f16z_view_*`), and each view read keeps its name, its signature and its qualifier as the
   one place its format is told: format 16's body, 16z's under the one check, else format 15's. The format bodies are
   inlined into it, so a view read is still one function: written first with each format's body a function of its
   own (three times the called functions), the join kernels compiled 1.5x slower on an Arc 140V (`rows_copy` 18 →
   28 s, `witness_count` 46 → 82 s), as Intel's compiler makes each called function a stack call, at a cost each.
   A view read that calls another calls that read, which tells its format again. The reads that read no tape (the
   lines', the numbering's) are written once.
7. **The check is one macro**, `CLIF_Z_READ` (`tape_f16z.h`), which the tape's public reads and the view reads use.
8. **A kernel of one format is compiled for it.** The label kernels already took the batch's format as their
   specialization constant (`tape_format`: each load of the view's format reads it, and the other formats' bodies
   are dead). The resident interpreter and the sweep now do too (`label_passes::run_format`: a run places its tapes
   in format 15 or 16, a 16z tape as 16), so they hold one format's reads and no 16z read, and so does the
   transducer's interpreter over tapes: its launch takes the kernel of the batch's format (`DocTable::format`). A
   mixed batch's (0) reads every format, as before. The fixed kernels' list names them per format (`interpreter 15`,
   `resident interpreter 15`, `label sweep 16`, …).
9. **The interpreter calls its tape and its evaluators; the reads inline.** In the interpreter (one kernel function:
   `run_batch` with everything inlined, 154 KB of source) format 15's tape reads were two thirds of the compile:
   132 s, 45 s with them stubbed, on an Arc 140V, one kernel alone, as `clc build` times it. They were inlined at
   every use (66 copies each of a node's start and end, 136 of the unaligned load). So the readers the interpreter
   calls (`device_tape.h`'s `tape_*`, the runtime's `Node_*`) and its evaluators (the NFA, the Lua patterns, the
   assertions, the text tests, the output, the query admission, the look-arounds) are called functions
   (`UKL_COLD` then; `UKL_SHARED` now, a call on OpenCL only, §22): 39 s for that kernel (the reads stubbed too: 21 s). Calling the reads themselves instead (format
   16's side-list read, its line select, the parents) made the join kernels up to 7x slower (`rows_copy` 18 →
   120 s): the reads stay inlined into the readers that call them.
10. **And the interpreter's task, once.** The sweep ran a task in three places, each the whole interpreter inlined
   (`run_task_in`); it is called, and so are the interpreter's helpers inlined most (the captures' lookups, 14 and
   28 copies; the queue's insert; the arenas' reservations, 28; the lazy blocks' check, 10; the NFA's closure and
   find; the Lua class): the interpreter alone 21.8 → 12.9 s (`clc build`), the sweep 25 s alone.
11. **The join kernels that read a view are compiled per format too.** A batch's views say their format once
   (`ViewsInPlace::format`, each `ViewInPlace`'s: every document's, or 0 for several); a kernel that reads views
   (`ix_bucket`, the stages', the scans', `witness_count`, `rows_copy`) is compiled for the format of the views it
   reads, the view's format field a constant (`clif_view` at its offset), so the other formats' reads are dead. A
   kernel over views of several formats takes the source of every format, as before; the kernels compiled ahead are
   the run's format's (`Format::of_run`).
12. **The join stages and the label walks call their row helpers.** In `stage_scatter` the relation's column read
   (`rget`, through the generated `clif_rel_get`) was inlined 31 times, and `write_row` 5; `rget`, `write_row`,
   `candidates`, `eval`, `defined`, `matches` and `aggregate` are called: `stage_scatter` 43.7 → 12.6 s,
   `witness_count` 19.1 → 11.9 s (`clc build`, 3 at a time). In the label walks (`label_group`, `label_micro`)
   `scan_marks` (3 copies, each 21 KB expanded) and `micro_test` (25) are called: `label_group` 18.4 → 8.0 s.
13. **The interpreter's four largest opcode cases are called** (`step_child`: `CHILD`, `DESCEND`; `step_next`:
   `NEXT`, `FOLLOWING`; `step_fork`; `step_emit`): 6.9 KB of the switch's 10.7, and with them stubbed the interpreter
   compiled in 4.0 s against 13.5 s (the whole switch stubbed: 1.6 s). Each is written in the names the case was, and
   works on copies of the task's state it changes, written back when it goes on: as the JIT's steps (which paste the
   case, now a call) already work. The interpreter alone 13.5 → 10.3 s.
14. **A kernel that uses no SIMD group requires no sub-group size** (superseded by §16 and §22: removed). UKL's OpenCL shim required 32 lanes of every
   kernel (`intel_reqd_sub_group_size(32)`, as Metal's SIMD groups); Intel's compiler builds a kernel required to
   run 32 wide 2-5x slower than at its own 16 (the interpreter 10.4 → 5.5 s, `stage_scatter` 11.1 → 2.4 s,
   `witness_count` 12.2 → 4.1 s, `label_group` 8.0 → 3.9 s, each the same source). A source whose kernels reach no
   SIMD-group operation and read no lane says so before its includes (`UKL_SIMD_GROUP_FREE`); on OpenCL it then
   requires no size, and the driver chooses. What it says is checked where it matters: every SIMD-group operation,
   and the lane, then call a function nothing defines, so a kernel that reaches one is refused at its build ("unresolved
   external symbol"), and the library's functions no kernel reaches are not built. The sources that say it: the
   interpreters', the sweep's and the join view kernels' over format 15 or 16 (`simd_group_free`), and the tape
   tests' writer and format 15's and 16's reads. Over 16z they do not: its block decode is a SIMD group's. The label
   walks (`label_group`, `label_micro`, `label_phase`) and the sorts use SIMD groups.
15. **16z's block lookup is called, and a 16z block's load is one read.** `clif_z_block` (the cache's lookup or
   decode of a block, 13 KB expanded) was the compiler's to inline (`UKL_CALLED` then, `UKL_FUNC` now), and Intel's inlined it at every
   read: 16 copies in the tape reads' test kernel. It is called on OpenCL (`UKL_SHARED`, §22). And format 16's body over a decoded
   block read each value through one load per width (`clif_f16b_ld`: four coherent reads, each two 64-bit words
   read by atomics on OpenCL), inlined at each of 142 loads: one coherent read of the words that hold the value,
   masked to its width. The join stages call `witness_probe` (3 copies in `witness_count`: 4.1 → 2.1 s) and
   `textop_pass` (`stage_count` 7.8 → 6.0 s).
16. **An OpenCL device's SIMD groups are its native width.** `UKL_SIMD_WIDTH` was 32 on every OpenCL device, as
   decision 0440 had planned otherwise ("an OpenCL device's own subgroup width"): every kernel that uses a SIMD group
   required 32 lanes, which Xe2 runs as two halves of 16 and Intel's compiler builds 2-5x slower. The device now finds
   its width once (`lanes`, in `compute_kernel_opencl`): the narrowest of its sub-group sizes from 16 (16 on an Arc
   140V), each kernel required to run at it (`intel_reqd_sub_group_size(UKL_SIMD_WIDTH)`); `RERAIL_OPENCL_SIMD_WIDTH`
   asks for 1, 16 or 32. The engine's kernels already step by `UKL_SIMD_WIDTH` (0440): its device tests pass on the
   CPU at 16 lanes (`RERAIL_CPU_SIMD_WIDTH=16`) as at 32, and on the Arc. The 16z reads' test kernel 14.1 → 5.8 s,
   `label_micro` 10.2 → 5.2 s, `label_group` 7.8 → 3.7 s, each the same source.
17. **The block programs take the device's width.** The block programs (the transducer's batches) were written for
   SIMD groups of 32, and were first kept at 32 on the Arc (a source that stated its width, compiled at 32 where the
   device's sub-groups came in 32): at the width that compiles 2-5x slower. They now take the device's, 16 or 32 lanes
   (`block_program::group_lanes`; any other is refused, and the CPU's executors run the batch), and nothing states a
   width (that mechanism is gone). Where 32 was written in: the kernels' group (`GROUP_LANES`, now `UKL_SIMD_WIDTH`),
   their scratch slots, the borrow's carry out (the add of the lanes' masks, carried out of bit `GROUP_LANES`), the
   group's last lanes for the next's halo, the scans' last doubling; on the host the batch's layout, the launch, the
   exchanges' passes (a word a lane), and the generated text. The serial executor's lanes form holds a segment, its
   warm-up and the block after it in one group: a segment asked longer than the group holds is cut to it (29 blocks
   at 32 lanes, 13 at 16), which changes how much a wrong start runs again, never what a run gives.
   At 16 lanes a test found a bug from before: a delay-0 output's warm-up lanes wrote their own word to the output's
   last word, taken for one no stream reaches. A file of one group whose last 64-bit word is partly filled reaches
   it (200 bytes at 16 lanes of 16 bytes; at 32 lanes, about 433-496 bytes). A warm-up lane now stores nothing (the
   buffer starts zeroed). Sending its store to that word instead, as zeros, raced the last group's own store of it:
   a file's groups run at once on a GPU, and on the Arc a JSON file of three groups lost its last word.
   Tests: the block, serial and directive tests on the CPU at 16 and 32 lanes (`RERAIL_CPU_SIMD_WIDTH`), the parser
   tests' 117 at 16; on Metal (M3) and CUDA at 32; on the Arc at 16. There `prev_equals_reference_on_every_executor`
   and `binary_operators_equal_reference_on_every_executor` pass in 34 s and 49 s with their kernels built; cold they
   build about 150 kernels each (14 a program, 0.3-0.5 s of Intel's compiler each), past a minute.
18. **The tape's load is one read.** `clif_t_ld`, the one load of the tape in place, chose a load by the value's
   width at run time (four loads), and it is inlined at each of a read's loads (38 in `tape_end`, 152 loads): one
   read of the words that hold the value, masked to its width, as 16z's block load (§15). `stage_count` 5.9 → 3.7 s,
   `label_micro` 10.1 → 8.1 s, the 16z test kernel 15.5 → 12.6 s, each interpreter 0.6-0.9 s less.
19. **Format 16's side read is called.** A value too wide for its array is escaped to a side list, read by a
   binary search over the part's offsets (`side`, in `<ukl/body/tape_f16.h>`): the rare case, inlined in every read
   that reaches a value (slot, gap, length, number), three copies in `tape_end` alone. It is called (`UKL_COLD`) over a
   decoded 16z block; over the tape in place it is `UKL_RARE` (§22: called, but inlined on OpenCL), each instance
   saying how (`CLIF_16_RARE`). In a kernel that requires a sub-group size, on the Arc at 16 lanes, the
   called read over the tape misread (the 16z reads' test: node 1's child 2 read 11 for none), and read right
   inlined, at 32 lanes, and called on CUDA and the CPU: there it stays inlined (the cause, in Intel's compiler or
   ours, not yet found). The 16z test kernel 5.9 → 2.8 s, the interpreters 9.0 → 6.7 s and 9.4 → 7.4 s, the sweep
   9.6 → 8.3 s; `label_micro`, which uses SIMD groups, keeps the inlined read (6.2 s; 3.6 s with it called).
20. **A text test compares once.** `textop_one` held three comparisons (equal, starts with, ends with), each a
   `txt_same` of two eight-byte reads of eight byte reads, and `textop_pass` inlined it twice (in the set's loop and
   after it): about 100 copies of the byte read. The op picks the subject's offset and the length test, one
   `txt_same` compares (called), and the set's loop visits each member, the last too, through one `textop_one`.
   With §18: `stage_count` 5.9 → 2.9 s.
21. **Splitting a body that is there once does not help.** `apply_output`'s four large instructions, each its own
   called function, left the interpreter as it was (8.1 → 8.5 s, 9.3 → 9.3 s), though each emptied saved 0.6-1.1 s:
   Intel's compile of these kernels follows the code they reach, not the size of a function. A read inlined many
   times is the lever (§18, §19), and code that goes; the split was not kept.
22. **Intel's compile cost is worked around on OpenCL alone; every kernel uses SIMD groups.** §9-§15 made about
   seventy hot bodies calls on every backend (`UKL_COLD`: every `tape_*` read, the `Node_*` reads, `node_next`, the
   interpreter's task and steps, its evaluators, the captures' lookups, the queue's insert, the arenas' reservations,
   the join stages' row helpers, the label walks' `scan_marks` and `micro_test`, 16z's block lookup), and §14 barred
   SIMD-group operations from the interpreters, the sweep, the join view kernels and the tape tests: Intel's compile
   time bought with Metal's, CUDA's and the CPU's run time, and with SIMD groups shut out of those kernels. Both were
   wrong. A device's gap is UKL's to work around, per backend: `ukl/backend.h` defines `UKL_SHARED` (a small read
   many places reach: the `tape_*` and `Node_*` reads, a relation's column) and `UKL_LARGE` (a large body many reach:
   the steps, the evaluators, the task, the join stages' row writers, the label walks' scans, 16z's block lookup) as
   `UKL_INLINE` on Metal and the CPU, as these bodies were before §9; and as calls on CUDA and OpenCL, whose compilers
   (NVRTC, IGC) compile each inlined copy apart. On CUDA the transducer's tests' interpreter and JIT kernels
   (`all_backends_execute_every_verified_instruction_and_mode`, `wildcard_and_missing_flags_…`) compiled in 40 s and
   26 s with both called, 187 s and 131 s with the reads inlined and the large bodies called, and over 240 s with
   both inlined, forced or at NVRTC's choice: there the lever is fewer copies of the reads in the kernel's code
   (§18-§20), not inlining; and `UKL_RARE` (a rare case's body) as a call but on OpenCL,
   where a called side read misread (§19). `UKL_COLD` is left to the paths rare on every device: 16z's take and decode
   of a missed block (`clif_tc_take`), its side read over a decoded block, a lazy block's creation
   (`ensure_lazy_block`), the escaped view value (`vread`), and the reads the CLIF marks cold (decision 0310). §14's
   `UKL_SIMD_GROUP_FREE` is gone: at the Arc's native 16 lanes (§16) a required size costs nothing, and every kernel
   is compiled with its SIMD groups on every device; a device without them takes UKL's emulation
   (`UKL_OPENCL_EMULATED_SG`), never a kernel written without them. Tests: the engine's transducer, tape and view
   tests on the CPU, Metal (M3), CUDA and the Arc give the same results as before: the failures left are the pack
   data's (not pulled on the hosts), the anchors' and the evaluation budget's (on every backend, the CPU's too), and on
   the Arc `regex_block`'s and `nested_optional_types`' (before too). On the Arc
   `all_backends_execute_every_verified_instruction_and_mode` took 194-202 s, the same with the side read called and
   with no sub-group size required: its JIT kernels' compile (§23; its 24 s of 2026-11-21 were the driver's cache).
23. **The JIT's kernel reads its programs from a table, over the batch's format.** `all_backends_…` passes in 1.9 s
   on the Arc with its kernels built; cold, its JIT kernels compiled 45.8 s each. A JIT kernel held every tape
   format's reads (16z's too: `TAPE_PREFIX`), a function a program (a switch over its steps, each case a call of the
   opcode's step) and an output wrapper a program, each the same call of `apply_output`: 116 programs, about 260
   called functions. The steps were calls, so the cases' constant operands were never folded into them: the code per
   program bought nothing. Now the kernel is compiled for the batch's format (`tape_view`, as the interpreter's, §8:
   the run's format ahead, a batch of another its own at its launch), the programs' steps are one table
   (`JIT_START`, `JIT_STEPS`: opcode, mode and operand a step, as the interpreter's words), read by one switch that
   calls each opcode's step once (each inlined: 11.2 s, 10.3 s called), and `apply_output` is called as the
   interpreter calls it. That kernel 45.8 → 11.2 s (`clc build`, cold); the test cold 200 → about 88 s on the Arc,
   on CUDA 41 → 10 s, on Metal (M3) 18 s, on the CPU 6.9 s. A JIT kernel is the interpreter's runtime and 3-4 s more;
   the runtime's own compile is what is left (§9-§21).
24. **A test's block programs run as one batch; a program's body is a large function.** A block test checked each of
   its programs alone (`check_all` in a loop: `prev` at eleven distances, the binary operators eight ways, the delays
   nine) and each input alone, so each program built its own kernels at every lane width and strategy, and its own
   serial program: on the Arc, 55 builds for `prev`, 80 s cold and past the 240 s budget for the operators. The batch
   kernels and the serial passes take a batch of programs (decision 0066), and the programs differed by a few bytes
   of their sources: `Executors::every_of_all` (and `check_each`) runs every program over every input in one batch
   for each device executor, so `prev` builds 7 programs, not 55. Its serial program with the eleven in it was
   176 KB and 20.2 s: each program's bodies (`serial_P`, `lanes_P`, the cones, the groups, the records) were
   `UKL_INLINE`, and `lanes_P` inlined into two kernels; and `tempo_batch`'s `program_P` the same. They are
   `UKL_LARGE` now (§22: inlined on Metal and the CPU, a call on CUDA and OpenCL), one body a program per segment or
   group: 20.2 → 13.3 s, and `tempo_batch` 6.6 → 5.1 s. The 12 serial kernels are made in one build
   (`kernels::functions`: on OpenCL one program with each name's `__UKL_ONLY_*`, as `UKL_ONE_KERNEL`'s one kernel,
   since Intel's compiler builds every kernel of a program). Cold on the Arc (no kernel store, the driver's cache
   off): `prev` 26 s, the binary operators 20 s, the delays 38 s; with the kernels stored, `prev` 2.6 s. On the CPU
   (16 and 32 lanes), Metal (M3) and CUDA the eight batched block and compile tests pass, in 8-9 s, 15 s and 9 s.
   What is left is the compiler's slowdown within a process (above, "Measured"): the same 47 KB interpreter source
   builds in 0.7, 0.4 and 1.6 s as the test goes on, `tempo_batch` in 1.0, 2.8 and 5.1 s.
25. **Format 16's value read and start are shared reads.** `get` (a value of an array, or its escaped side list's)
   and `start_stated` (a checkpoint and the gaps after it, `get` in its loop) were `UKL_INLINE`, so every `tape_*`
   read that reaches them (start, end, end of, size, the line reads, the sibling walks: each its own function on
   OpenCL and CUDA, §22) compiled its own copy, with the side read inlined in each on OpenCL (§19). They are
   `UKL_SHARED` (§22) now: one body each on OpenCL and CUDA, inlined on Metal and the CPU as before. Each source of
   `all_backends_execute_every_verified_instruction_and_mode` built in a fresh process on the Arc 140V (`clc builds`,
   the driver's cache off), the reads inlined and called: the JIT kernel 12.2 → 9.1 s, the interpreter 5.2 → 3.8 s.
   The test passes on the Arc, and on CUDA with `wildcard_and_missing_flags_…` in 12.8 s (each kernel 1.3-2.3 s).

## Measured

On an Arc 140V (Intel's OpenCL), each fixed kernel compiled into an empty store: `rows_copy` alone first, then the
whole list 4 at a time, bounded at 460 s (`kernels compile --within`). Before is 0395 §7's (one kernel, the mixed
interpreters); "> N" a compile still running when the bound ended it.

| Kernel | Before | Tape reads per format (§1–5) | Views, kernels per format (§6–8) | Readers called (§9) |
|---|---|---|---|---|
| `join rows_copy`, alone | 45–47 s | 18.1 s | 17.5 s | 17.6 s |
| `join stage_count` | | 46.7 s | 46.4 s | 46.1 s |
| `join witness_count` | | 46.4 s | 46.8 s | 46.6 s |
| the interpreter over tapes | > 240 s | > 383 s | 145 s (15), > 332 s (16) | 47 s (15), 59 s (16) |
| the resident interpreter | > 240 s | > 383 s | 180 s (15), > 241 s (16) | 52 s (15), 65 s (16) |
| the label sweep | > 240 s | > 335 s | 145 s (15), > 206 s (16) | 103 s (15), 99 s (16) |
| the label passes, 15 / 16 | | 42.6 s / 206 s | 46.1 s / 164 s | 49.6 s / 54.9 s |
| every fixed kernel | > 460 s | > 460 s | > 460 s | 279 s |

With the task and the helpers called (§10), 4 at a time: the interpreter 27 s (15) and 29 s (16), the resident
interpreter 28 s and 29 s, the sweep 29 s and 38 s, the label passes 50 s and 69 s; the join kernels as before
(`stage_scatter` 50 s, `witness_count` 47 s); every fixed kernel in 183 s.

With the join kernels per format (§11), 4 at a time: `rows_copy` alone 17.6 → 2.5 s; `ix_bucket` 7 s,
`stage_count` 24 s, `stage_scatter` 59 s, `scan_count` 12 s, `witness_count` 28 s, `rows_copy` 8 s; every fixed
kernel in 155 s.

With the row helpers called (§12), 4 at a time, the host's CPU 98% busy with other work (so every kernel, changed or
not, slower than before: `rows_copy` alone 2.5 → 7.4 s, the interpreters 26 → 34 s): `stage_scatter` 59 → 24 s,
the label passes 49 → 28 s (15) and 65 → 34 s (16); every fixed kernel in 164 s.

With the opcode cases called and no sub-group size where no SIMD group is used (§13, §14), alone: `rows_copy` 1.5 s
(17.6 s at §11's start), the interpreter over format 15's tapes 5.8 s (132 s at §9's), the resident interpreter
12.5 s; 4 at a time (the host loaded by other work): `stage_scatter` 7.8 s, `witness_count` 14.6 s,
`tape_writer_replay` 19.6 → 11.5 s; every fixed kernel in 110 s.

Every fixed kernel one at a time, the host loaded by other work (`rows_copy` 4.2 s here, 1.5 s on a quiet host),
before and after §15: the tape reads' 16z test kernel 31.2 → 20.8 s, `witness_count` 10.1 → 6.2 s; `stage_count`
12 s either way; the interpreters 11–16 s, the sweeps 13–16 s, the label passes 16 s (15) and 21 s (16, nine
kernels); every fixed kernel 277 → 263 s one at a time, 99 s 4 at a time.

Not yet every kernel in seconds: the resident interpreter (12.5 s alone, twice the interpreter over tapes), the
sweep, the label passes' walks (which use SIMD groups), `stage_count` and `witness_count`, and the tape tests' reads
of 16z are next.

**The measurements above "with the host loaded" were not load (2026-11-21).** On an idle Arc 140V host the list
took as long (264 s one at a time), and `clc` (`gpu/device/examples/clc.rs`) built the same dumped sources two to
three times faster. Intel's compiler is slower after its first large build in a process: in one process `row_text`
builds in 2.8 s, then 8.7 s each time after (8.3 s of CPU against 2.6 s, the same binary byte for byte), whatever
was built between, in a new context too; the front end (`clCompileProgram`) takes 0.35 s each time, so it is the
back end. Small kernels are not slowed. `kernels compile` builds every fixed kernel in one process, so it pays this;
the times below are each source built first in its own process (`clc builds`), the compiler's own cost.

Each fixed kernel over 2 s, the first build in its process, before and after §16-20 (the interpreters, the resident
interpreters and the sweeps share the interpreter's code: one of each shown):

| Kernel | §15 | §16-20 |
|---|---|---|
| the 16z reads' test kernel | 13.5 s | 2.8 s |
| the label sweep (16) | 10.4 s | 8.3 s |
| the interpreter (two of its kernels) | 8.8 s, 10.3 s | 6.7 s, 7.4 s |
| `label_micro` | 10.0 s | 6.2 s |
| `label_group` | 7.8 s | 3.1 s |
| `stage_count` | 5.9 s | 1.9 s |
| `row_text` | 2.8 s | 2.1 s |
| `stage_scatter` | 2.4 s | 1.7 s |
| `tape_writer_replay` | 3.2 s | 3.2 s |
| the join kernels of `row_text`'s source (`dedup_*`, `lat_*`, ...) | 1.3-1.8 s | 0.3-1.0 s |

Not yet in seconds: `label_micro` (6.2 s: the side read inlined, §19), the interpreter's code (6.7-8.3 s in each
of its six kernels: the regex search, the assertions,
the outputs and Lua each 1.3-2.4 s of it), and `tape_writer_replay`.

