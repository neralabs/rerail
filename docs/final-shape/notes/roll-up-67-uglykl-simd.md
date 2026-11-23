# Roll-up 67: uglykl-CPU-SIMD-parallelization (7ff70f5dd7) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at e6a661e43b. Theirs: uglykl-CPU-SIMD-parallelization at 7ff70f5dd7. The one merge
base is 091c4e099d, theirs' pin in roll-up 58. Theirs brings twelve commits of its own and no merges (decision 0567
step 3, UKL on the CPU as vector code):
- the element sum as vector code: inlined calls, a full group's form, contiguous lanes, paired loads, one `addv`;
- the matrix product kernel. Its loop is as hand-written: `fmla` by element, invariant addresses hoisted, counting
  to 0. It also comes as a product of floats stored apart (sign bits, exponent bytes, 3-byte fractions), unpacked
  and packed as vector code, its loop at 32 lanes with no spill;
- a SIMD group's chunks are a loop of its code, not fibers. Narrow code is as hand-written, with a width model by
  loop and by unit;
- a uniform word's lane shifts are scalar, windows under 16 bytes load to vector registers, and uniform 32-bit
  values read for their low bits are not zero-extended.

## Items

1. **The merge is git's**: one base, and no file changed on both sides since it. Ours' files since roll-up 58 are
   others, theirs' are `rerail-ukl-vec` and the vendored Cranelift.
2. **Theirs' Cranelift patches are recorded in the vendor's `PATCHES.md`.** They are what the lowering cannot shape
   (0567's bar: no spills):
   - the elaboration's placement and loop stack;
   - the constant-sum rewrites;
   - AArch64's shared address sums;
   - load and store pairs;
   - the `addv` and `i64x2` pairwise reductions.
3. **Roll-up 65's leftover: the vector path read the coherent atomics as defined twice.** Roll-up 65's `memory.h`
   (0603) defines the atomics at a coherent base as overloads. The vector path reads OpenCL's branch, where a coherent
   base is `volatile __global`, and its C reader tells no overload apart by `volatile`. Every rerail-ukl-vec kernel
   that took `memory.h` was refused (31 of its 35 tests). On the vector path there is one memory and a coherent base
   is a global one, so those overloads are not defined there (`#if !defined(__UKL_VEC)`). Every other backend keeps
   them.
4. **`fiber.rs`' `PREPARED`**, a count of the fibers this thread prepared that its tests read, is `Thread` in
   run_state's STATE, beside the same file's `POOL`.

## Audit

`scripts/check/merge_audit.py` reports no findings.

## Checks

- On spark2, CPU, one platform: `cargo check --workspace --tests` is clean. The crate DAG, global-state and
  thread-state checks pass.
- Locally under Relax: rerail-ukl-vec's 35 tests pass. Theirs' 10 new ones are among them: the matrix products, the
  split floats, the chunked barrier, the per-lane atomic, the uniform values and the byte lanes.
