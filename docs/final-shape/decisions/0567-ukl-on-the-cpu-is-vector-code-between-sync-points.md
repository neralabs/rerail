# 0567: UKL on the CPU is vector code between sync points, made by tape-to-tape passes

Status: agreed with the user (2026-11-21). Steps 1 and 2 are built (2026-11-21, "Step 2, as built" below). The goal is the user's: "run the same kernels on the GPU and
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
   1. **The source is the OpenCL backend's** (`rerail-ukl-vec`'s `source`, 2026-11-21): the kernel preprocessed as
      OpenCL C with 32-lane sub-groups (`UKL_OPENCL_SUBGROUPS`), into the C frontends' tokens (`Preprocess::expanded`:
      no line markers, each token's origin kept). One C dialect for the GPU drivers and this path, and no shim of its
      own: the intrinsics are OpenCL's builtins (`get_local_id`, `barrier`, `sub_group_*`, `atomic_*`), known by
      name. Where C's grammar has no place for OpenCL's words they are respelled (`__UKL_VEC`):
      - the address spaces are nothing, as the CPU's memory is one;
      - a threadgroup's own storage is marked where it is declared (`__UKL_LOCAL_STORAGE`, the `SHARED` rows);
      - OpenCL's scalar names are C's fixed-width types;
      - a kernel's rows and a structure's fields are constants (`Name__ukl_params`, `Type__ukl_fields`), not the
        reflection kernels OpenCL's host runs.
   2. **Lower:** the C tape becomes a typed kernel tape, its control flow structured (`if`, the loops and `switch` as
      nodes: the kernels have no `goto`), its variables numbered. `UKL_CALLED` functions are inlined, structs split
      into fields, overloads resolved.
   3. **Cut at sync points:** barriers, grid barriers, SIMD-group operations and `SYNC_FUNCTION` calls end a region.
      A value live across a cut is kept per lane, as a struct of arrays: the coroutine stack's replacement. A
      barrier then costs nothing; it is the boundary between two regions.
   4. **Uniformity:** each value is uniform (the same for every thread) or varying. A uniform value stays scalar.
      Each memory access is sorted by its index: uniform, consecutive in the thread id, strided, or gathered and
      scattered. As built (step 2), there are four levels: uniform, agreed, the same in each SIMD group, and
      varying. The sorting of memory accesses is step 3's, where the code generator uses it.
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

   A thread that has ended the kernel is no longer its group's: a barrier waits for the threads that have not, as
   on the GPUs and the coroutine backend (decision 0440). A thread that has not ended and is not at a sync point
   its group meets (it returned from the function the barrier is in, or took the other branch) is let off only if
   it then ends without reaching another sync point. If it reaches one, the kernel fails, naming both. The
   interpreter of step 1 checks this as it runs, and the uniformity pass of step 2 proves it, or refuses the kernel.
4. **The SIMD-group width is 32** (`UKL_SIMD_WIDTH`), as on Metal and CUDA, and other widths stay possible (the
   library passes at 32, 16, 8 and 1). Where the hardware's vector is narrower, a SIMD group is several vectors,
   looped, a value taking as many registers as its type needs (decision 6). With little register pressure, a group's wider span means fewer loop iterations and less overhead per
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
   - **Cranelift (the JIT)** with 128-bit vectors for now: a register holds 16 lanes of 8 bits, 8 of 16, 4 of 32 or
     2 of 64, so a 32-lane group's value is 2, 4, 8 or 16 registers by its type. A region is unrolled to the group's
     width as register pressure allows. Wider types in the vendored Cranelift are later work, if its numbers ask
     for them.
   - **Rust (AOT)**, written directly from the vector tape (decision 0550: not through CLIF, not by an old
     lowering), in `simd8`'s form: generic over a backend, with AVX-512 (512 bits: 16 lanes of 32 bits, 8 of 64, 64
     of 8), AVX2 (256 bits), NEON (128 bits) and scalar,
     each compiled with its CPU features and chosen at run time. On stable Rust, without `std::simd`.

## Order of work, each step gated

1. The source, the kernel tape and the cuts, run by the interpreter at one lane. Gate: equal to the coroutine
   backend on the library's tests.
2. Uniformity and vectorization, with the interpreter at 32 lanes, and the barrier rule's refusal. Gate: the same,
   with the SIMD-group operations native, and a test kernel that breaks the rule refused with its place.
3. Cranelift. Gate: the library's tests, then `join_stages` and `pp_grid` equal to the coroutine backend.
4. Rust AOT. Gate: the same, with its speed measured against the coroutine path.
5. Cranelift's wider types, if step 3's numbers ask for them.

## Step 2, as built

- **The vector tape** (`rerail-ukl-vec`'s `vector`) is one per kernel, made on its first launch. It is the kernel
  tape with every call inlined. A call is an `Inline` node over its parameters' stores and its callee's body, and
  the callee's `return`s store into the call's slot. A callee's frame follows its caller's, and sibling calls
  share it. Each node's field holds:
  - its value's level;
  - whether its subtree reaches a sync point, or a threadgroup barrier (the cut: such a node runs for the whole
    group at once);
  - whether it has an effect;
  - whether its condition is checked as it runs.
- **The levels**, as a fixed point over the kernel:
  - **Uniform:** constants, sizes, group ids, and loads at uniform addresses (the kernel's memory is race-free).
  - **Agreed:** an atomic that leaves its word as it is (a poll), at a uniform address. It is the same in a SIMD
    group, and across the group as far as the kernel's own synchronization makes it.
  - **The same in a SIMD group:** a ballot, or a shuffle from a uniform lane.
  - **Varying:** the thread's ids, the other atomics, and a SIMD group's lanes.

  A variable's level is the most varying of its stores' values and of the control they run under. Three refinements
  keep legal kernels from being refused:
  - Levels are kept by byte range, so a structure's fields are apart: `UklGrid`'s `group` is uniform and its `lid`
    is not.
  - A pointer into a lane's own variables is followed to them, offset and all. OpenCL's private memory is no
    other lane's, so `&ip` passed to a helper does not make `ip` varying.
  - Each declaration has its own number (the kernel tape's `Local`), even where scopes share a frame offset.

  Lanes that returned from an inlined call never read its variables again. So their return raises only the writes
  its caller sees: the call's result, the caller's variables, and what pointers reach. A `break` or `continue`
  raises every later write in its loop.
- **The barrier rule is proven statically** (decision 3), or the kernel is refused with both places, as
  `path:line:column`:
  - A threadgroup barrier must run under control that is agreed or uniform, and a SIMD-group operation under
    control that is the same in its SIMD group.
  - The lanes that a more varying branch, loop condition or exit leaves out must end without reaching another
    sync point. For a branch, that means its other side and what follows it; for a `return`, what follows its call;
    for a `break`, what follows its loop. A `continue`'s lanes always come back, so they are never let off.
  - Where the proof rests on an agreed condition, the condition is marked, and the interpreter checks as it runs
    that every SIMD group of the group decides it alike, or the kernel fails.
  - The tests are `a_barrier_some_threads_skip_is_refused_with_its_place`,
    `a_simd_group_operation_some_lanes_skip_is_refused` and `threads_that_end_are_let_off`. Every kernel of the
    library's tests vectorizes.
- **The interpreter runs at 32 lanes**, a SIMD group at a time, its active lanes a mask:
  - A uniform value with no effect is computed once for its SIMD group; any other node is read once and its lanes
    run in a loop.
  - A varying branch runs each side under its mask, and a loop runs until its mask is empty.
  - A cut node runs for all of the group's SIMD groups, each region between barriers one SIMD group after another.
  - SIMD-group operations are native: a ballot is the mask's lanes, and a shuffle is a permute of the lanes'
    values.
  - A lane's variables are its own, as a struct of arrays in its SIMD group's frame.
- **A scan's tile is 16 values a thread** on this path (`UKL_SCAN_VT`), as on the CPU backend (decision 0527 §3).
  A group operation costs hundreds of the tape's nodes, against a few for a thread's fold of one value. At 1 value
  a thread, the grid scan of 1,000,000 values took 3 s; at 16, the whole test takes 2.7 s.
- **Gate:** with `RERAIL_GPU=cpu RERAIL_CPU_EXEC=vec`, the library's 19 tests pass (the bench is ignored):
  - in 31–57 s at 6 threads, by the machine's load, against 223 s for step 1's one-lane interpreter;
  - against 5.7 s for the coroutine backend, on a debug build (`rerail-ukl-vec` at opt-level 3).

  The interpreter's speed is step 3's work.
