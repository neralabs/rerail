# Roll-up 44: CUDA-support (949511d462) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 92a47c14f1 (roll-up 43, main, Q464).

Theirs: CUDA-support at 949511d462. Its merge base with ours is 9a204ab036 (roll-up 40). Theirs brings decision
0576's kernels of one tape format:

- the tape and view reads have one body per format, and the format is told once. `reads/tape_body.h` is split into
  `tape_common.h`, `tape_f15.h`, `tape_f16.h`, `tape_f16z.h`, and `body/` (`tape_api.h`, `tape_f16.h`,
  `tape_read.h`, the generated `view_reads.h`) (fe216a66a4, e6be3c71d3);
- the interpreters and the sweep are compiled per format, and a batch's launch takes its format's interpreter
  (`interpreter_kernel(format)`) (§9);
- the interpreters call their tape readers and helpers rather than inlining them (§9–§10);
- the join kernels that read views are compiled per format, and stage scatter and label walks call their row helpers
  (§11–§12).

## The invariants every item is checked against

From ours:

- **Device waits read by polls** (0540, Q123): on Apple's GPUs a relaxed load may keep reading the core's copy. A
  word read again by a wait, or where a stale copy breaks the 16z cache's protocol, is read by
  `clif_tc_poll`/`clif_tc_read`.
- **A lookup whose wait ran out takes an entry of its own** (0540, `clif_tc_take`'s `own`).
- **A large read is cold, not inlined** (0548). The AGX back end took 44.7 s on the transducer's interpreter with
  every tape read inlined, and 5.5 s with none (`clif/src/ukl.rs`, `DEVICE_INLINE`).
- **The crate split's manifests** (`rerail-classic`, not `rerail-w`), and ours' clippy-clean closures.
- **The generated reads are the generator's output** (`the_view_kernels_reads_are_these`).

From theirs:

- **The format is told once.** A read asks the view for its format in one place (`CLIF_TAPE_READ`), and a kernel
  is compiled per format.
- **Calls over copies on the device** (§9–§12): an interpreter calls its readers and helpers.

## Items

1. **`tape_body.h` is gone, and ours' changes are in theirs' files.** Ours changed 23 hunks of `tape_body.h` since
   the base. Each hunk was tried against every one of theirs' new files:
   - The sixteen 16z cache hunks (the polls, `clif_tc_read`, the pending waits, the take's `own`) apply to
     `reads/tape_f16z.h`. The `own` argument and its comments go onto theirs' `UKL_COLD clif_tc_take`.
   - A comparison of the cache code with ours' differs only where theirs changed it: the block decode's reads
     through `clif_t_ld` (0576), and the probe constants moved to `tape_common.h`.
   - Ours' forward declarations have no counterpart: theirs' format files are included in order.
2. **The six large public reads are cold, through theirs' one macro.** Ours made `size`, `start_stated`,
   `end_stated`, `start_next`, `line` and `line_start` `UKL_COLD` (0548). In theirs each public read is generated
   by `CLIF_TAPE_READ` as `UKL_INLINE`: three formats' bodies and 16z's checked retry, copied into every caller.
   The macro now takes its linkage (`CLIF_TAPE_READ(link, …)`). Those six are `UKL_COLD`, and the rest stay
   `UKL_INLINE`, as ours had them. This is theirs' direction too: §9–§10 call the readers. `clif/src/ukl.rs`'s
   comment now names `body/tape_read.h`. Test: `every_library_header_stands_alone_on_every_backend` and the other
   ukl header tests, with clang on a farm Mac.
3. **The batch's interpreter is chosen inside ours' `dispatch_batch`.** Ours had moved the launch into
   `dispatch_batch(engine, buffers, table, lanes, …)`. Theirs chose the kernel by the batch's format at the
   launch. The merge puts theirs' choice (`Engine::Interpreter` → `interpreter_kernel(table.format()…)`, a JIT
   engine its own kernel) in ours' function, under ours' "Rerail" launch names.
4. **The prewarm list**: ours' three closures (written as blocks for clippy). Theirs' per-format loop replaces the
   format-blind interpreter and label-pass entries.
5. **`device_metal/Cargo.toml` is ours'.** Ours already had theirs' `rerail-clif` dependency and features, from
   roll-up 40, under the crate split's `rerail-classic` names.
6. **The generated view reads are written again.** Ours' `reads.rs` added `view_order`, `view_tape` and
   `view_depth`, which theirs' generated `view.h` and `body/view_reads.h` predate. Both are written by
   `RERAIL_WRITE_READS=1` from the merged generator. Test: `the_view_kernels_reads_are_these`, with and without
   test-hooks.

## Checks

- spark2: `cargo check --tests` of device-metal, clif and ukl is clean. The clif and ukl libraries pass, 27/27,
  and clif alone without test-hooks passes, 17/17.
- m1max, compile and clang only, with no GPU launch: the ukl library passes, 10/10, with clang present, and
  `cargo clippy --tests` of device-metal, clif and ukl is clean.
