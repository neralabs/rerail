# Roll-up 56: Go-C-and-C-support (cb23f84a49) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 9fa02104d7 (roll-up 55, Q476). Theirs: Go-C-and-C-support at cb23f84a49. The merge
bases are CUDA-support's 7dcb64b839, the hub's d01d8cdef4 and b18a332a59. Theirs brings 19 commits, all the C
preprocessor's on the device:
- `UKL_ONCE` (082e8995c0);
- 0540 §3's join in `pp_setup`;
- a grid sized by its kernel's residency (0568);
- barriered functions as dynamic libraries of their own (0571);
- CUDA and OpenCL (0573);
- where the GPU time goes (0580);
- the expansion as one loop a lane group runs (0588).

## Items

1. **Files theirs changed only by its merges take ours** (32 files, among them `corpus.rs`, `combining.rs`, `rr.rs`,
   the UKL shims, `windows.rs` and decisions 0165, 0353 and 0547).
   - Theirs merged hub roll-up 38 and CUDA-support, and ours holds both and resolved them later.
   - Theirs' resolutions were read for lines of their own: the corpus's path spelling, the corpus listing's test, the
     json_lean note and the device's `moved` count. Ours has each in its later form (`corpus/base.rs`'s `slashed_by`,
     simdjson by clang-cl).
2. **The preprocessor's grid: theirs'.** 0540 leaves the preprocessor's waits to its owner ("the preprocessor is
   another agent's"), and marks `<ukl/grid.h>`'s barrier as not for the engine's kernels. So 0568 is the owner's
   rule for its own grid, and the pp files take theirs: `pp_arena`, `pp_eval`, `pp_expand`, `pp_grid`, `pp_probes`
   and the evaluation tier's test.
   - Ours' t51 item 4 (7e654c99d8) had made the same grids join (0540 §3). Theirs did that too, and adds 0568.
   - Ours' `grid.h` stays as merged: the CPU's join closes at its resident groups (`UKL_GRID_RESIDENT`).
3. **`<ukl/pool.h>` goes.** 0588 removed it with its users, and the evaluation tier's test checks that none is
   left. Ours' fence fix in it (b9c30c38dc) goes with it.
4. **One vocabulary for a function's call: `UKL_ONCE` and `UKL_FUNC`.** Ours (77e2fdb1cd) named the forced call
   `UKL_CALLED` and the compiler's choice `UKL_FUNC`. Theirs named the forced call `UKL_ONCE` and kept `UKL_CALLED`
   for the choice. The merge holds `UKL_ONCE` (the forced call, which 0571's `split` reads) and `UKL_FUNC` (the
   choice), and no `UKL_CALLED` is left.
5. **The setup's call depth is 4.** `split.depth` counts forced calls, and Metal's `UKL_COLD` is one. Theirs' tape
   reads under 16z were three forced calls deep: `clif_tape_size`, then `clif_tape_size_z`, then
   `clif_tape_z_block`. Ours' 0576 checks 16z once in the public read, so a read is two: `clif_tape_size`, then
   `clif_z_block`. `the_setup_splits_one_unit_a_barriered_function` states 4.
6. **Decisions.** Theirs' 0557 and 0558 are ours' 0577 and 0578 (roll-up 55). Theirs' edit names a decision's
   number in its title, and ours' titles already do. The index takes theirs' 0568, 0571, 0573, 0580 and 0588.

## Audit

`scripts/check/merge_audit.py`'s lines are items 1-6:
- theirs' side is its older hub text, ours' later (194 of them `relation.h`, ours' regeneration);
- ours' side is the pp files (item 2).

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean, and `check_crate_dag.py` passes;
- rerail-c-directives `eval::` 35 pass, and rerail-metal `pp_` 9 pass on the CPU backend.
