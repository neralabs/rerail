# Roll-up 48: CUDA-support (7eff18d065) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 95aa1bcd25 (roll-up 47, Q468). Theirs: CUDA-support at 7eff18d065, pinned
2026-11-22 in `tmp/rollup/pins4.txt`. The merge base is 949511d462, roll-up 44's CUDA-support. Theirs brings four
commits:

- 0576 §13-21: Intel's OpenCL at its own 16 lanes, with the interpreter's large opcode cases, 16z's block lookup and
  the join's probes as calls (ad3aa60747, 89ef9b0d12, b9c30c38dc). A source whose kernels use no SIMD group says so
  (`UKL_SIMD_GROUP_FREE`). A source written for one width says so (`UKL_SIMD_WIDTH_NEEDED`). The device says which
  widths it offers (`offers_simd_width`). The pool's take fences before it reads a head's link.
- 7eff18d065: the M3 Max's and M1 Max's read fixes (decision 0540), ported into the restructured reads. These are
  G15's slot mask by `SLOTS_MAX` and LZ4's 32-bit places, and G13's per-branch node find in `clif_view_field`.

## Items

1. **The read fixes are theirs' form, the only form.** Ours does not have G15's slot mask
   (`git grep 'SLOTS_MAX - 1'` finds nothing at ours' tip), nor the 32-bit places in `clif_tz_lz4`. The commits that
   first made them (18367a7deb, e55c8a4d09) are on neither side, so ours had the M3 Max's 16z failures too. The
   merge takes theirs' port: `cache::SLOTS_MAX` and `Spec`'s refusal past it (view.rs), the mask in CLIF's
   `cache_lookup` and in `clif_tc_slot`, the decode's places, and the find in each branch (view_reads.rs).
   0517's Rust reads need no mask, since `Spec` refuses more slots on every path.
2. **`relation.h` is the merged CLIF's output.** It is generated (`rerail_clif::reads::relation_ukl_source`,
   decision 0325), so it was not merged by hunk. The merged CLIF holds ours' 0494 step 7 reads (dictionary codes and
   key runs) and theirs' slot mask. `RERAIL_WRITE_READS=1` wrote it again on spark2. `ids.h`, `view.h` and
   `body/view_reads.h` were already the merged CLIF's. The generated file has the mask at both of its sites (131071).
   The audit's line findings in it are the generated values' numbering.
3. **`UKL_SIMD_GROUP_FREE` holds on ours' code.** Theirs marks the interpreters', the sweep's and the join kernels'
   sources over format 15 or 16 as free of SIMD groups. On OpenCL a kernel there that reaches a SIMD-group
   operation fails its build.
   - Since the base, ours changed those sources only in `join_stages.uglykl`: a stage's padding, and `vread` cases
     9-11 (`clif_view_tape`, `clif_view_depth`, `clif_view_order`, 0353 row 5).
   - Those reads, and the view and format 15/16 tape reads they call, use no SIMD-group operation (`view.h`,
     `body/view_reads.h`, `tape_f15.h`, `tape_f16.h`, `tape_common.h`: none).
   - `simd_roots` in `device_tape.h` is a SIMD group's. Only the label passes call it, and those are not free.
4. **The OpenCL shim moved, then changed.** Ours moved `ukl_opencl_shim.h` to `native/crates/ukl/mldb/` unedited
   (f0f3aa6f5f). Theirs edited it at its old path. The merge has theirs' file at ours' path. The five base lines the
   audit lists as ours' are theirs' change: 0576 §16 takes the device's own sub-group width in place of requiring 32.
5. **`device_tests.rs`: one port.** Both sides moved the store test off the removed `store::last_here` onto
   `store::recording`, the same way. Theirs' names and comment are kept.
6. **Found: `UKL_CALLED` means two things, and the first wins.** This is not this merge's change. Roll-up 41 merged
   `backend.h` with no conflict, so each backend defines `UKL_CALLED` twice:
   - first as plain `static` (the compiler inlines or calls as it chooses: 0440 step D, 6ba6565614, for
     `clif_ukl`'s generated programs and `clif_t_ld`);
   - then inside `#ifndef UKL_CALLED` as `noinline` (compiled once and called: 0523, 1d0fed55ea, for the grid's
     barriers and scans and the preprocessor's tape reads and phases).

   The second never applies. Fixed in the commit after this merge, one name per job.

## Checks

- spark2, CPU: `cargo check --tests --examples` of rerail-device, rerail-gpu, rerail-clif, rerail-ukl, rerail-metal,
  rerail-tape-format, rerail-blocks and rerail-engine is clean. rerail-clif, rerail-tape-format, rerail-blocks and
  rerail-ukl pass, 142 of 142, among them the four generated-reads checks.
- Not run: the device reads on Metal. The merged Metal reads are the same CLIF as the CPU's, which ran. Theirs ran the
  device reads on the M3 Max and the M1 Max at its tip.
- No bench: the merge was judged on the code, by the user's order of 2026-11-22.
