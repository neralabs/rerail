# Roll-up 60: CUDA-support (78f75cdb07) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at e97af45130. Theirs: CUDA-support at 78f75cdb07, five commits of its own and no
merges (decision 0576 §17, §22-§25):
- §22: every kernel uses SIMD groups (`UKL_SIMD_GROUP_FREE` is gone). Inlining is UKL's policy per backend:
  `ukl/backend.h` defines `UKL_SHARED` (a small read many places reach), `UKL_LARGE` (a large body many reach) and
  `UKL_RARE`. They are inlined on Metal and the CPU, and are calls on CUDA and OpenCL. Block programs run at the
  device's SIMD width, 16 or 32 lanes (`GROUP_LANES`, `block_program::group_lanes`);
- §23: the JIT's kernel reads its programs' steps from one table, compiled for the batch's tape format;
- §24: a test's block programs run as one batch (`Executors::every_of_all`). The forward carry's kernels are made
  together, which on OpenCL is one program;
- §17: a generated block program's warm-up lanes store nothing.

## Items

1. **The look-back is gone in ours (0540 step 7, f0f7e4cca7).** Most of theirs' block-program hunks are in its code:
   the look-back generator, `block_program.uglykl`, `CONTROL` and `lane_code`, and the `run_split` executors.
   - Each of theirs' commits was applied to ours' file in order (`/tmp/obs/rebuild.sh`).
   - Of each rejected hunk, the lines it removed were looked for in ours. None survive except the files' head
     comments, which carry theirs' 16-or-32 text now.
   - `block_program.uglykl` stays removed.
   - `source_lanes`, which only the look-back's generator called, is not taken.
2. **The device's width.** `block_common.uglykl` takes theirs' `GROUP_LANES` (the device's SIMD width, 16 or 32), and
   its `UKL_SIMD_WIDTH_NEEDED` pin goes. The host side's `group_of` and `ready` merged clean.
   - The OpenCL shim moved in ours (`native/crates/ukl/mldb/…`), and its width override goes, as in theirs.
   - Neither side uses the pin anywhere else.
3. **The forward carry's kernels** are made by theirs' `kernels` (all at once: one program on OpenCL) from ours' one
   list of them, `serial_kernels()`. The same list says when they are ready.
   - Ours' single `kernel` stays for the back kernels.
   - The program bodies are `UKL_LARGE` (theirs). Ours' lowering check looks for `UKL_LARGE void lanes_0(`.
4. **§17's warm-up store** is in the look-back's generator only. Ours' forward carry already stores only from its
   own lanes (`if (mine) out[…]`, and `b >= first && b < last` in the serial pass), so it has no such race.
5. **The test executors**: theirs' batch of programs and inputs (`every_of`, `every_of_all`), over ours' one device
   executor, the forward carry, at segments of 3 and 29 blocks.
   - `run` with the device takes the same path, so a device run goes one way.
   - Past the launch budget is a hard failure (0601): ours' panic, not theirs' skip list.
6. **backend.h keeps both vocabularies; each has its own job.**
   - Ours' `UKL_ONCE` is a body compiled once and called on every backend: grid barriers, and the pp tape reads that
     made Metal's compile minutes long.
   - Theirs' `UKL_SHARED`, `UKL_LARGE` and `UKL_RARE` are inlined or called per backend.
   - `UKL_SIMD_GROUP_FREE` goes, as §22 removed it (nothing in ours defines it). Ours' `__UKL_VEC` block stays.
7. **`rget` is `UKL_SHARED`** (§22: a relation's column), with ours' memo (roll-up 58). Its `UKL_COLD` was §12's
   (949511d462), which §22 supersedes.
8. **The JIT's format.** Theirs' `dispatch_batch` compiles the programs' kernel for a batch of another format. Ours had
   split `execute_packed_once` (`execute_on_device`, `dispatch_batch`), so `programs` is now passed down to it.
9. **`gpu/src/kernels.rs`**: ours' branch to the vector path's library, timed by theirs' `RERAIL_GPU_TIMES`.
10. **0576's text** keeps ours' `UKL_FUNC` naming beside theirs' §22 wording.

## Audit

`scripts/check/merge_audit.py` reports 129 findings:
- 28 on ours' side: items 2, 3, 5, 8 and 9's rewrites, and the shim's width override (item 2).
- 101 on theirs' side: the look-back's code (item 1), theirs' `run_split` executors (item 5), and the kernel list
  that `serial_kernels()` holds (item 3).

The landed-lines check (`/tmp/obs/r58/landed.pl`) finds each of theirs' added lines as merged, or accounted for
above. The lost-lines check finds no line of ours dropped but items 2, 3, 5 and 9's.

## Checks

On spark2, CPU, one platform: `cargo check --workspace --tests` is clean. Locally under Relax, CPU: rerail-ukl-vec's 26
tests (backend.h's vector path) pass.
