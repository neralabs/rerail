# Roll-up 58: uglykl-CPU-SIMD-parallelization (091c4e099d) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at cbee7a03c1. Theirs: uglykl-CPU-SIMD-parallelization at 091c4e099d. Git's merge base
is virtual: theirs has several merge bases with ours, df1ecee5aa among them. Theirs' merge inputs are already in ours
(43034c9cce, 87fe4b4a28, df1ecee5aa, ae31a25982, 064453276e), except Native-and-GPU-strata's 0353 step 3 (c472dfef5e,
b170f02cb5, 2a8f28507e), whose commits are theirs' own here. Theirs brings 28 commits of its own:
- decision 0567, steps 1 to 3: UKL on the CPU as vector code. The new crate `rerail-ukl-vec` reads a kernel's C tape
  and lowers it to a kernel tape, SSA and Cranelift code at a width. `rerail_gpu::vec` runs a program through it
  (`RERAIL_CPU_EXEC=vec`). The CPU device runs interpreted libraries (`CpuInterpreter`);
- 0353 step 3 on the CPU: the views' directory is self-relative (homes, offsets) and made without a device. A 16z
  batch's cache lives in a block of its own (`ZCache`). The directory's searches are written once, with a memo
  (`clif_dir_doc_of_row`, `clif_dir_doc_of_key`);
- Q396: a field's part is priced by its children's kinds (`Fielded::part_size`), and a demand scan's rows are carried
  across the base atoms its variables share (`over::base_fan`).

## Items

1. **Files theirs changed only through merges take ours**: 77 files, among them about 50 where theirs' merges had
   dropped `windows` from the device cfgs. The recursive base let theirs' older text win clean hunks there. The same
   happened in theirs' own files, so each was also checked for lines of ours that none of theirs' commits removed:
   - `windows` back in the cfgs of `vtab.rs`, `syntax.rs`, `device_chunk.rs`, `values.rs`, `placed.rs`,
     `syntax/tests.rs` and `device_metal/Cargo.toml`;
   - `#[cfg(not(target_os = "macos"))]` for CUDA in `gpu/src/lib.rs` and `gpu/device/src/lib.rs`;
   - `view_reads.rs`' `#[cfg(feature = "ir")]` on `up` and `end`;
   - the work queue's "(0475)".
2. **The CPU device's library is ours' plus theirs' commit.** Ours' `os::Lib` and `os::Stacks` (the Windows port)
   carry theirs' `Code` (compiled or interpreted) and `CpuInterpreter`, as 93f698edcb wrote them. The file is ours
   with that commit applied; its three hunks over ours' `os::Lib` were written by hand.
3. **The program cache**: ours' slot per key, compiled ahead (`compile_ahead`), with theirs' branch to the vector
   path's library (`vec::library`) where the device runs vector code.
4. **0353 step 3 on ours' 0576 layout**:
   - `ViewsInPlace` keeps ours' `format` (the stages' tape format, 0576) beside theirs' `at`, `Option` `dir` and
     `ZCache`.
   - Theirs' tape-format binding in `views_in_place` is `tape_format`, so that it no longer shadows the batch's
     `format`.
   - `vread` takes theirs' memo, over ours' rows from `p->from` (0353 step 6).
   - `rget` stays `UKL_COLD`, as 0548 made it.
   - ours' `Within::has_walk` reads `v.dir?`, as theirs' merge fixed it.
   - The generated reads were written again from the merged generator, unchanged.
5. **backend.h**:
   - Ours' `UKL_SIMD_GROUP_FREE` block keeps its `#endif`, and theirs' `__UKL_VEC` reductions follow it.
   - The device-scope fence (ours' b9c30c38dc) is OpenCL C 3.0's, which the vector path's C reader has no name for.
     There the fence is `mem_fence`, the one fence that path reads (`Builtin::MemFence`).
6. **The OpenCL shim moved in ours** (`native/crates/ukl/mldb/…`). Theirs' 43655243a4 is applied there. Its kernel
   info is a macro of its own (`__UKL_KERNEL_INFO`), and the vector path's version is a constant. That macro sits in
   ours' one-kernel selection (`UKL_ONE_KERNEL`, `__UKL_DEFINE_KERNEL2`).
7. **Q396 on ours' split**:
   - `part_size` and the field's part pricing go to `plan/search.rs`, where ours keeps `Fielded` and the pricing.
   - `ndv_or_default` is `pub(crate)`, for `demand/over.rs`.
   - `over::candidates` is `pub`, for its test in rerail-logic's mount.
   - Theirs' sketch fix is ours' `sharing` token (9ef8597d64), the same bug fixed once, with theirs' test already in
     ours.
8. **`block_serial.rs` and `pp_expand.rs` take ours.** Theirs' only commit there (2b4dbc0afc) put back the text of
   older inputs, which ours holds in newer form.
9. **The state lists**: rerail-ukl-vec's three thread-locals are `Thread` in run_state's STATE, and its trace variable
   `V` goes in the global-state list. The decisions index gets 0567's row. Theirs' 0557 and 0558 rows are ours' 0577
   and 0578.

## Audit

`scripts/check/merge_audit.py` reports 976 findings:
- 26 on ours' side: items 2 to 7's ports, and the generated `relation.h`.
- 950 on theirs' side: theirs' older merge text in the files item 1 takes as ours (the `Canonical` imports, the
  TypeScript catalog's Git-registry code, the global-state rows ours renamed, and the `windows` cfgs).

The landed-lines check (`/tmp/obs/r58/landed.pl`) finds each of theirs' added lines as merged, or accounted for
above.

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean, with no warnings. `check_crate_dag.py`, `check_thread_state.py` and the
  global-state list's entries for this merge pass.
- rerail-clif's `reads_are_these` (3) pass, and writing the reads again changes nothing.
- rerail-ukl-vec: 26 tests pass.
- rerail-tests-logic `within_read_from_a_` (2, with theirs' new one) and rerail-logic's
  `a_scan_bound_through_a_small_kinds_children_is_not_demanded` pass.
- rerail-tests-lang `directory` (4) pass.
