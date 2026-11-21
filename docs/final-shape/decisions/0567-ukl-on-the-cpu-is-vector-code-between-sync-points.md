# 0567: UKL on the CPU is vector code between sync points, made by tape-to-tape passes

Status: agreed with the user (2026-11-21), not yet built. The goal is the user's: "run the same kernels on the GPU and
the CPU", where the CPU backend's coroutine C++ (decision 0440 §8, "The CPU backend") "is correct but horribly
slow". The answers below are the user's, given to the questions of the design pass. Follows 0440 and 0310.

## What is there

- **The kernels:** 21 `.uglykl` files, about 13 K lines (`pp_expand` 4 K, `join_stages`, `pp_grid`, `label_passes`).
  - Sync points: 48 threadgroup barriers, 29 grid barriers and 12 `UKL_CALLED` phase functions. SIMD-group operations
    are used only through the library (`scan.h`, `radix.h`, `group.h`, `grid.h`).
  - 386 atomics.
  - About 380 `for` loops, 70 `while` loops and 13 `switch` statements; no `goto`.
- **The language is C already.** The kernels and the library use no template, lambda, namespace, reference, cast or
  `auto`. The C++ is in the shims: the CPU shim's coroutines and its layout templates (`ukl_type_code`), and Metal's
  MSL sections of `backend.h`. The one feature beyond C is `UKL_OVERLOAD` (30 library functions):
  `__attribute__((overloadable))` in OpenCL C, nothing in C++.
- **The CPU backend** (`ukl_cpu_shim.h`, `compute_kernel_cpu.rs`): a threadgroup's threads are stackful coroutines.
  A barrier is a switch to the next thread, and a SIMD group is emulated by its threads in turn. A persistent grid
  must be resident: at most 8,192 threads at once.
- **What the path is built on:**
  - `rerail-c-pp` preprocesses UKL for a backend, at run time.
  - `c_lean` writes tree-sitter-c's tape as it parses.
  - The tape's reads are written once in CLIF (0310), so a JIT inlines them.
  - `simd8` is the pattern for portable Rust SIMD (0343): code generic over a backend (NEON, AVX2, SSE, scalar),
    compiled per CPU feature and chosen at run time.
- **Cranelift** (vendored 0.125.4) has 128-bit vector types only. AVX-512 appears in 16 x64 lowering rules, as VL
  encodings of 128-bit operations. There are no 256-bit or 512-bit types and no mask registers.

## The decision

1. **UKL kernels are C.** The CPU path parses C (tree-sitter-c's tape, by `c_lean`), not C++. Not every OpenCL
   compiler takes C++, and C is far easier to lower. A kernel or library header that needs C++ is refused.
   `UKL_OVERLOAD` is C with clang's `overloadable`, which the lowering resolves by its argument types.
2. **The path is passes from tape to tape**, each one reading its input tape in place and writing its output once,
   in a block. Each output is a derived fact, not a copy (0353). The tapes are in **the existing tape format**, with
   node kinds of their own, so the reads, blocks and rules work on them unchanged.
   1. A "cpu-simd" shim for `rerail-c-pp`: the macros become marked calls (barriers, SIMD-group operations, atomics,
      address spaces), not coroutines.
   2. **Lower:** the C tape becomes a typed SSA kernel tape. `UKL_CALLED` functions are inlined, structs split into
      fields, address spaces explicit, overloads resolved.
   3. **Cut at sync points:** barriers, grid barriers, SIMD-group operations and `SYNC_FUNCTION` calls end a region.
      A value live across a cut is kept per lane, as a struct of arrays: the coroutine stack's replacement. A
      barrier then costs nothing; it is the boundary between two regions.
   4. **Uniformity:** each value is uniform (the same for every thread) or varying. A uniform value stays scalar.
      Each memory access is sorted by its index: uniform, consecutive in the thread id, strided, or gathered and
      scattered.
   5. **Vectorize** (whole-function vectorization, as POCL's and Intel's OpenCL CPU runtimes do it): a varying
      operation becomes a W-wide one. A varying `if` is masked execution, and a varying loop runs until its mask is
      empty. Atomics:
      - at a uniform address, the lanes reduce in registers and issue one atomic;
      - otherwise, each active lane runs its atomic in turn, or AVX-512's conflict detection groups them where the
        CPU has it.
   6. **Schedule:** a threadgroup runs region by region, each region looping over the threadgroup in vectors. A grid
      barrier is the same cut one level up: phase k runs over every group before phase k+1 starts. A persistent grid
      then needs no residency on the CPU.
3. **A kernel that breaks the barrier rule fails.** A barrier or SIMD-group operation must be reached by every
   thread of its group together, and the uniformity pass proves it or refuses the kernel with its place. There is
   no fallback to the coroutine backend: some GPUs deadlock on such a kernel, so it is wrong everywhere. Loosening
   the rule on hardware that supports it (independent thread scheduling) is later work.
4. **The SIMD-group width is 32** (`UKL_SIMD_WIDTH`), as on Metal and CUDA, and other widths stay possible (the
   library passes at 32, 16, 8 and 1). Where the hardware's vector is narrower, a SIMD group is several vectors,
   looped. With little register pressure, a group's wider span means fewer loop iterations and less overhead per
   lane; a kernel whose live state does not fit runs a narrower native width per step within its 32.
5. **SIMD-group operations are written once**, as operations of the vector tape, for every back end:
   - ballot is a move-mask;
   - prefix sums and reductions are log-step operations in registers;
   - shuffle is a permute;
   - match and compaction are ballot and popcount.

   Each one also spans the several vectors of a 32-lane group. The library's tests are their gate on every back end.
6. **Three back ends from the vector tape:**
   - **The interpreter** runs the vector tape at once, each operation over a group's lanes, so a kernel runs while
     its compile is in the background.
   - **Cranelift (the JIT)** with 128-bit vectors for now: 4 lanes of 32 bits a register, unrolled to the group's
     width as register pressure allows. Wider types in the vendored Cranelift are later work, if its numbers ask
     for them.
   - **Rust (AOT)**, written directly from the vector tape (decision 0550: not through CLIF, not by an old
     lowering), in `simd8`'s form: generic over a backend, with AVX-512 (16 lanes), AVX2 (8), NEON (4) and scalar,
     each compiled with its CPU features and chosen at run time. On stable Rust, without `std::simd`.

## Order of work, each step gated

1. The cpu-simd shim, the kernel tape and the cuts, run by the interpreter at one lane. Gate: equal to the coroutine
   backend on the library's tests.
2. Uniformity and vectorization, with the interpreter at 32 lanes, and the barrier rule's refusal. Gate: the same,
   with the SIMD-group operations native, and a test kernel that breaks the rule refused with its place.
3. Cranelift. Gate: the library's tests, then `join_stages` and `pp_grid` equal to the coroutine backend.
4. Rust AOT. Gate: the same, with its speed measured against the coroutine path.
5. Cranelift's wider types, if step 3's numbers ask for them.
