# 0567: UKL on the CPU is vector code between sync points, made by tape-to-tape passes

Status: agreed with the user (2026-11-21). Steps 1 and 2 are built (2026-11-21, "Step 2, as built" below; its interpreter rebuilt as a code tape the same day). The goal is the user's: "run the same kernels on the GPU and
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
      nodes: the kernels have no `goto`), its variables numbered, structs split into fields, overloads resolved,
      `offsetof` and `_Static_assert` folded. `UKL_CALLED` functions were to be inlined. As built (step 2) they are
      called: one body per context a function is called in (below, "A body per context").
   3. **Cut at sync points:** barriers, grid barriers, SIMD-group operations and `SYNC_FUNCTION` calls end a region.
      A value live across a cut is kept per lane, as a struct of arrays: the coroutine stack's replacement. A
      barrier then costs nothing; it is the boundary between two regions.
   4. **Uniformity:** each value is uniform (the same for every thread) or varying. A uniform value stays scalar.
      Each memory access is sorted by its index: uniform, consecutive in the thread id, strided, or gathered and
      scattered. As built (step 2), there are six levels: uniform, agreed, the same in each SIMD group, a lane's,
      the thread's id in its group, and varying. The sorting of memory accesses is step 3's, where the code generator uses it.
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
3. **Compiled vector code** (the bar set 2026-11-21, replacing "Cranelift, gate: equal to the coroutine
   backend"):
   - 1000 times faster than the interpreter;
   - SSA form;
   - Rust generated from it with SIMD intrinsics or other implicit SIMD parallelism (the precompiled form);
   - Cranelift code with SIMD, every register used and spills minimal;
   - function calls with caller and callee saving chosen per call;
   - a register allocator over the vector, float, integer and mask registers, which tells the register pressure;
   - each block's width chosen (1, as the coroutines, 2, 4, 8, 16 or 32 lanes) at the register pressure's sweet
     spot.

   Built in this order, each part on the one before (§ "Step 3, the plan").
4. Cranelift's wider types, if the vector code's shape asks for them.

## Step 2, as built

- **The vector tape** (`rerail-ukl-vec`'s `vector`) is one per kernel, made on its first launch. Its root is the
  kernel's body, then one `Body` per function in each context it is called in, and a call is a `Call` node naming
  its body. Calls are not inlined: step 3's gate kernels inlined are hundreds of millions of nodes. A lane's frame
  holds the deepest chain of calls' frames, and a call that recurses is refused. Each node's field holds:
  - its value's level;
  - whether its subtree reaches a sync point, or a threadgroup barrier (the cut: such a node runs for the whole
    group at once);
  - whether it has an effect;
  - whether its condition is checked as it runs.
- **A body per context.** A function's levels depend on its arguments', so it gets a body for each context it is
  called in: the levels of its arguments (an aggregate's, its most varying byte's). The control it is called under
  is not part of the context. A body's levels are relative to its entry, the lanes that call it, and what its
  caller's control makes of its result and writes is the caller's. Each function also has a fallback body, with no
  context, that is right for any call. The bodies are found in rounds:
  - a round makes the bodies wanted so far, and the analysis puts each call in the body of its context if there is
    one, else in the fallback;
  - a call wants the context it has at its last visit (the levels' fixed point), not the ones it passes on the way;
  - each call site keeps the contexts it has wanted, and a round makes only those at the sites it reaches. The sets
    only grow, so the rounds end (at most 16; then the fallbacks stand);
  - then the bodies the calls were put in, alone, are the kernel's, analysed once more and checked.

  The gate's largest kernel, `pp_setup`, ends at 158 K nodes in its bodies; the join kernels at 30–80 K.
- **The levels**, as a fixed point over the kernel:
  - **Uniform:** constants, sizes, group ids, and loads at uniform addresses (the kernel's memory is race-free).
  - **Agreed:** an atomic that leaves its word as it is (a poll), at a uniform address. It is the same in a SIMD
    group, and across the group as far as the kernel's own synchronization makes it.
  - **The same in a SIMD group:** a ballot, a reduction (`sub_group_reduce_add`, `_min`, `_max`), or a shuffle from
    a uniform lane.
  - **A lane's** (`Lane`): the lane's number in its SIMD group, and what is made of it alone. A reduction of a
    lane's value is the SIMD group's, and agreed if what it reduces is the same in every SIMD group.
  - **The thread's id in its group** (`Lid`): its remainder by a power of two up to 32 (or its low bits) is a
    lane's; its quotient by a multiple of 32 (or its high bits) is the SIMD group's. That holds only if the group's
    lanes are laid out so: a kernel with a `Lid` level is refused a launch whose group is not 1-D and whose first
    dimension is not a multiple of 32.
  - **Varying:** everything else a thread computes from its ids, and the other atomics.

  The library's SIMD-group reductions are native on this path (`UKL_NATIVE_SG_REDUCE`, `backend.h`): their
  butterfly of shuffles is every lane's sum too, but that the result is the same in every lane is not something the
  shuffles show. `ukl_tg_sum`, whose SIMD groups each reduce the same partials, is so found the same across the
  group.

  A variable's level is the join of its stores' values and of the control they run under. These refinements keep
  legal kernels from being refused:
  - Levels are kept by byte range, so a structure's fields are apart: `UklGrid`'s `group` is uniform and its `lid`
    is not.
  - A pointer into a lane's own variables is followed to them, offset and all. OpenCL's private memory is no
    other lane's, so `&ip` passed to a helper does not make `ip` varying.
  - Each declaration has its own number (the kernel tape's `Local`), even where scopes share a frame offset.
  - **A body's own variables are flow-sensitive:** a store at a uniform address replaces what the variable held.
    Where control rejoins (a branch's end, a loop's top, a `switch`'s end), what was written since it split is
    raised by the level that split it. That includes the lanes that come back from a `continue` (at the iteration's
    end) and from a `break` out of a case. Without the `continue`'s rejoin, `pp`'s `if_number` was wrong: a
    variable stored only by the lanes that did not continue kept its uniform level, so a lane read another's
    (`a_continue_that_varies_leaves_the_rest_of_the_body`).
  - **A call's effects are its own:** the caller's state goes into the callee for what its arguments reach, and
    what the callee writes there comes back after the call, as it leaves it.

  Lanes that returned from a call never read its variables again. So their return raises only the writes its
  caller sees: the call's result, the caller's variables, and what pointers reach.
- **The analysis's cost** is what the rounds are made of. It is a worklist of bodies: a body is walked again only
  when something its walk read changed (a variable of another body, its entry state, a callee's effects). A whole
  pass then checks that nothing changes; in the gate's kernels it never does. A body's state is a list of its
  variables, each one's cells shared until written, so a branch's copy of it is the list, not the cells. Together
  with the per-site contexts, `stage_count` went from 5.8 s to 1.4 s, and `pp_setup` from 15 s to 11 s (a test
  build, `rerail-ukl-vec` at opt-level 3). That time is in the analysis's maps and the allocator, which an optimized
  build does not change.
- **The device keeps the code tape in its store** of compiled kernels (`rerail_device::store`, as every backend
  keeps its compiled form there): `vec/<key>.tape`, keyed by the vectorizer's build (a hash of its sources and the
  C parser's, `build.rs`), the preprocessed source and the kernel's name. So a process loads a kernel another
  vectorized.
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
- **The interpreter runs a code tape** (`code`, `run`). The first version walked the vector tape recursively:
  each node decoded at each visit, each lane dispatched on its operation and type, every value 64 bits a lane, and a
  uniform value broadcast to all 32. It was ten times slower than the coroutine backend. It was rebuilt in step 2,
  not left to step 3 (2026-11-21):
  - **The code tape:** the vector tape lowered once, on the kernel's first launch, to one `Code` node in the tape
    format. Its payload is a header, the parameters, and instructions of 5 words, read in place.
  - **Fused opcodes:** an operation and its type are one opcode (`Add32`, `LtI32`, `AddF64`, `Truth64`, ...). An
    instruction is one indirect jump through the table of a `match`. Its handler is a loop over the 32 lanes with
    nothing left to decide, which the compiler makes straight NEON (`AddF32` is eight `fadd.4s`). The rare cases
    are generic (`VBinGen`, ...), decided once an instruction, then lane by lane: a narrow type's arithmetic, a
    division (its zero is an error), a float's conversion. 32-bit kernels use no generic opcode (the test
    `the_interpreter_runs_fused_opcodes_and_each_thread_gets_its_value`).
  - **Typed registers:** a SIMD group has 32-bit lanes (every type of 4 bytes or fewer), 64-bit lanes, and scalars.
    A scalar is a value that is the same in the SIMD group and has no effect, computed once. An expression's
    temporaries are allocated by its depth, and a variable lives in the frame.
  - **Masks:** the lanes running, and those that took a `break`, a `continue` or a `return`, or ended. A uniform
    branch or loop condition is a jump; a varying one sets the running lanes for each side, packed from the
    condition by NEON (`vtstq`, `vaddvq`). No code runs with no lanes: a jump skips it.
  - **A lane's own variables** stay a struct of arrays in its SIMD group's frame. A direct read or write of a whole
    scalar variable is 32 consecutive values: one copy, or one blend under the mask. A load or store whose 32
    addresses are consecutive is one copy too.
  - **A threadgroup barrier ends a SIMD group's turn.** Each runs to it or to its end, then the next. Once all
    have, they go on, and SIMD groups that wait at different barriers fail the kernel. This replaces the cut's
    group-at-once nodes.
  - **x86:** the same loop compiled again with AVX2, chosen at run time (`is_x86_feature_detected`). The gate was
    run on two x86 Linux hosts with AVX2, the library's 19 tests on each, against the coroutine backend back to
    back: 4.3 s against 6.6 s on one, and 6.2 s against 7.7 s on the other (the test harness's times).
  - **`switch`** was first each case under the lanes whose value is its label and those that fell through the case
    before: every label compared over 32 lanes at each run, twice with a `default`. Step 3 replaced it (below).
  - **A poll of another group's progress** (`ukl_poll_yield`, a grid barrier's: `Yield`) ends a SIMD group's turn,
    and once every SIMD group of its group waits, the group's. Up to the coroutine backend's resident count of groups
    run together, each in a slot of its own, taking turns in order. So a persistent grid of up to that many groups
    meets its barriers, and a larger one fails their bound (1000 rounds, as the CPU backend's). Run one after
    another, `pb_pool`'s 8 groups failed their first barrier.
- **A scan's tile is 16 values a thread** on this path (`UKL_SCAN_VT`), as on the CPU backend (decision 0527 §3).
  A group operation costs hundreds of the tape's nodes, against a few for a thread's fold of one value. At 1 value
  a thread, the grid scan of 1,000,000 values took 3 s; at 16, the whole test takes 2.7 s.
- **Gate:** with `RERAIL_GPU=cpu RERAIL_CPU_EXEC=vec`, the library's 19 tests pass (the bench is ignored). The
  runs were at 6 threads on a debug build (`rerail-ukl-vec` at opt-level 3), back to back at a load of 9:

  | Executor | Wall | CPU |
  |---|---|---|
  | The code tape's interpreter | 3.6 s | 4.4 s |
  | The coroutine backend | 5.0 s | 5.9 s |
  | The tree-walking interpreter it replaced | 31–57 s, by load | |
  | Step 1's one-lane interpreter | 223 s | |

  In a sample of the suite, the interpreter's loop is nearly all of the running thread's time.
- **Step 3's gate kernels, on the interpreter** (2026-11-21). Every kernel of `join_stages`, `pp_grid` and the pp
  probes (35) vectorizes (`the_written_sources_lower_and_vectorize`, reading the sources the device writes with
  `RERAIL_UKL_VEC_SOURCES`). With `RERAIL_GPU=cpu RERAIL_CPU_EXEC=vec`:
  - `rerail-gpu`'s 25 tests pass, as on the coroutine backend.
  - pp's arena tests (`pp_arena::`, `pp_grid`'s kernels): the 9 pass, as on the coroutine backend.
  - The join and setfix device tests (`metal_join::`, `setfix::device`, in `rerail-device-metal`): the 44 pass, as
    on the coroutine backend. On a Mac (M3 Max, idle), 82 s against the coroutine backend's 16 s, warm store. Most
    of it is the text functions' kernel (`row_text`) run by the interpreter: `parity_random_texts` 66 s against
    11 s, `text_functions_on_the_device_are_the_cpus` 31 s against 2.8 s. Compiling that is step 3's.

## Step 3, the plan

The first build (`54995fd34c`, `jit.rs`) is not step 3. It compiles the interpreter's code tape instruction by
instruction: each register is a slot in the SIMD group's memory, each instruction loads its operands and stores its
result, a mask is an integer expanded at each use, and what it does not compile is a call into the interpreter. That
is the interpreter's handlers placed in a row, and it is slower than the scalar coroutine code. It stays as a
reference until the SSA's Cranelift back end replaces it, then it is removed.

1. **SSA** (`ssa`), from the vector tape, per body: typed values (integers by width, floats, addresses), each
   uniform (a scalar register) or varying (a vector of lanes), and masks as values. A variable whose address is
   not taken is SSA values, not frame memory; the frame keeps only what is addressed. A uniform branch is a branch;
   a varying one is predication under a mask value, and a varying loop runs while its mask has lanes. Block
   parameters join values, masks too. Written once into a block, as every derived fact is.
2. **Register model and allocator** (`regs`), per target: integer, float, vector and mask registers (NEON: 31, 32
   of 128 bits, masks in vectors; AVX2: 16, 16 of 256 bits; AVX-512: 32 of 512 and 8 mask registers). Liveness
   over the SSA, the pressure at each point at a width, an allocation with its spills. It is what the width is
   chosen by, and what each back end's code is checked against.
3. **Width per block** (1, 2, 4, 8, 16, 32 lanes): the widest a region runs without spilling, its values split into
   that many lanes' registers; a region of a narrower width runs as several passes over the SIMD group's lanes.
4. **Calls** as functions: arguments and results in registers, and who saves a live value (the caller or the
   callee) chosen at each call by its liveness on each side.
5. **Cranelift back end** from the SSA and the allocation: each value its registers, every register used, spills
   only where the allocator has them.
6. **Rust back end** from the SSA, in `simd8`'s form (intrinsics per target, chosen at run time), the precompiled
   form.

Gate: the library's tests, `join_stages` and `pp_grid` equal on each back end; a kernel's run time (its launches,
not the test's host side, which is the same on every back end) 1000 times the interpreter's; the generated code
read beside hand-written SIMD Rust of the same kernel, with each difference fixed.

## Step 3, an idea noted, not built: a scalar version below x lanes

The user's (2026-11-22): a pragma that, where it is on, runs a fully scalar version (recursion included) once fewer
than x lanes of the group's are still running.

- **Why.** A vector round costs about the same with one lane on as with all (a gather is a load a lane either way,
  and the mask's work and the test of any lane left are the same). A loop's last rounds, run for its slowest lanes,
  and a recursion, whose lanes part further at each depth, are where few lanes run. `nearest_jump` on a deep forest
  is one: 3.5 ms vectorized (hand or written), against 2.4 ms scalar, its chunks' last rounds for one lane or two.
- **How it fits `rs.rs`.** A function's scalar version is one more instance: every parameter one value (`Sh::S`),
  nothing masked, recursion Rust's (a lanes' instance is `#[inline(always)]`, so a recursive one is a call anyway).
  At a loop's round or a call, with fewer than x lanes on: each lane's live values out of the registers, the scalar
  rest of the loop (or the call) for each lane in turn, the results back in their lanes, and the vector code on from
  the loop's end. The pragma names x; its sense is per width (of 8 lanes on NEON at W=8, of 32 on a GPU).
- **When it may.** The lanes one after another is one interleaving of their threads, so only where the region holds
  no barrier, no SIMD-group operation, and no wait on another thread's progress (a lane run alone first would wait
  forever). The vectorizer marks the first two; the third is a check to write. Otherwise the pragma is refused.
- **Not for the GPU.** A warp's idle lanes cost the same, and there is no cheaper scalar unit; the GPU's form of it
  is to gather the few running lanes of many SIMD groups into full ones, through the work queue (continuations).
- **The cost**: a second, scalar copy of each marked loop or function. x is a vector round's fixed cost against a
  lane's scalar one, high (3–4 of 8) for a loop of gathers. The first measure: the hand-vectorized `nearest_jump`
  going scalar at 2 lanes or fewer, on the deep forest.

## Step 3, format 16's parent: written, by hand, measured

`kernels/f16_parent.uglykl` is `clif_f16_parent` for every node; `parent_all_w8.rs` its Rust at 8 lanes, its values
of 64 bits two registers of 2 lanes a register of 4 (`the_f16_parent_kernels_rust_is_the_checked_in_one`). The tests
(`crates/ukl_aot`) read real tapes: JSON documents parsed by `rerail-json-aot` into the engine's format 16 writer,
the host's `reads::f16::parent` the reference, every node of each, the written kernel at 5 group sizes. By hand
(`hand.rs`): `parent_all`, the kernel's walks 8 lanes in lock-step, and `parent_all_scan`, not the kernel's
algorithm: a walk's only state is its `j`, so a node whose walk steps to a sibling `j'` has `j'`'s parent, an answer
made before (or a lane to its left, by 3 rounds of pointer jumping in registers); one slot read a node.

What the documents' walks are: short (1.7 to 4.5 steps on average, never more than 8: a checkpoint every 8
siblings steps to the parent), and at width 1 a checkpoint's step is escaped (its side list's, a binary search):
a long array's walks visit one each. Times (`examples/parent_bench`, profiling build, M-series Mac at load 10–15):

| Document | Nodes | Host | Walks by hand | Scan by hand | Written, W=8 |
|---|---|---|---|---|---|
| 40,000 records | 1.4 M | 6.1 ms | 10.7 ms | 3.0 ms | 18.9 ms |
| 200,000 numbers | 200 k | 5.2 ms | 4.1 ms | 0.66 ms | 34.5 ms |
| random values | 53 k | 0.41 ms | 0.67 ms | 0.18 ms | 1.25 ms |

- **The walks don't pay in lock-step.** A walk is a few dependent steps; 8 lanes wait for the longest (1.8–2.3
  times the steps), and a vector round's gathers are a load a lane anyway.
- **The escape is the cost, and its search is where lanes run alone.** A chunk's lanes reach the same checkpoint in
  different rounds, so each side read is one lane's. By hand: only live lanes' escapes (a done lane reads node 0's,
  the root's, escaped), one lane's search the host's read, several lanes' searches interleaved (`side8`), and the
  checkpoint read last kept in a register. Written: the call is under `if any` of live and escaped lanes, right, but
  its search runs 8 lanes' loads for one lane, 8 times a chunk, the 7 times the host's time on the numbers. That is
  the idea above (a scalar version below x lanes), its first case outside `nearest_jump`.

## Step 3, format 16's parent: the parallel shape (the user's, 2026-11-22)

The user: "there should be a more efficient shape for this computation / do all 8 steps in lockstep / do all of the
binary searching in parallel ... I would prefer to take advantage of parallelism", then "go ahead with them / they
should also improve the speed on Metal, but please verify that it is true".

1. **The side search is a count, in every copy of the read** (CLIF, the host's Rust, the kernels' UKL body). A run of
   offsets is ascending, so the first at or past a node's low byte is the count below it: 8 a word, each byte compared
   unsigned within the word, summed by popcount, no load waiting on another.
2. **A walk's first step is a read of its own** (`parent_step`, host Rust and UKL; `parent` is a loop of it). A walk's
   only state is its node, so a node whose step goes on from a previous sibling's node `j` has `j`'s parent: no
   walk, a chain of at most K - 1 = 7 links (`every_chain_of_links_is_at_most_seven`).
3. **The kernel is one step a node and a chase of the links**, as one launch (`parent_group`, its Rust checked in: below).

Metal, an M3, the GPU's clock, the median of 9 (`examples/parent_gpu`); every node checked against the host's:

| Kernel | 40,000 records (1.4 M nodes) | 200,000 numbers | random values (53 k) |
|---|---|---|---|
| The walk, the halving search (before) | 0.474 ms | 0.382 ms | 0.069 ms |
| The walk, the count search | 0.509 ms | 0.231 ms | 0.070 ms |
| Step, 3 rounds of pointer jumping (4 launches) | 1.027 ms | 0.197 ms | 0.061 ms |
| Step, then the links' chase (2 launches, removed) | 0.581 ms | 0.120 ms | 0.037 ms |
| One launch: steps in threadgroup memory, a barrier, the chase there | 0.400 ms | 0.127 ms | 0.049 ms |

- **Not faster everywhere as first built.** Pointer jumping reads and writes two arrays of every node each round:
  on records, whose walks are short (1.7 steps), twice the walk's time. The traffic, not the threads.
- **The chase** reads one word a node and hops through the steps' words only where linked: 3.2 and 1.9 times the
  walk's speed where escapes and long sibling runs are, but its second launch's 8 bytes a node keep it 23% behind the
  walk on records.
- **One launch is faster than the walk on all three** (1.2, 3.0 and 1.4 times): a group's links mostly stay in it,
  and a link that leaves it is the walk from that node. It is the one kernel now; the two-launch form is removed.

The two-launch kernels' Rust on the CPU (instructions and cycles a node, this Mac at load 10–50, `PARENT_ONLY` with
`/usr/bin/time -l`): records 167 → 71 and 52 → 17 cycles; numbers 1,525 → 345 and 558 → 89; random 258 → 92 and
101 → 23. The host's scalar read then 18, 95 and 33 cycles.

## Step 3, a barrier's Rust: phases (the user's, 2026-11-22)

The user, of `parent_group`'s barrier: "implement it as phase splits". `rs.rs` writes a kernel whose function has
threadgroup barriers at its top as phases:

- **The group's threadgroup memory is an array of the kernel's function** (`let mut tg = [0u64; bytes / 8]`), named
  as one more buffer argument (`arg{k}`, after the launch's), so its reads and writes are a buffer's: consecutive
  lanes' a register's, others a gather.
- **The function's top-level statements between barriers are its phases.** Its Rust takes the phase and its chunk's
  state: `if phase == j { 'phase: { ... } }` for each, a return that ends every lane `live = 0; break 'phase`. A
  phase's end saves the chunk's lanes that go on (`live`) and each variable (a slot of the widest lanes each, 32 P
  bytes); the next phase's start reads them, and a chunk with no lane left returns. That is what the GPU's threads
  keep in their registers across the barrier. A later phase is written for a mask of lanes, whatever its instance's.
- **The kernel runs each phase's loop of the chunks in turn**, the phase a constant there (the function inlined in
  it is that phase's), every chunk's phase before any chunk's next: the barrier's meaning, with no thread waiting.
  The state is the group's chunks' (`Vec::with_capacity`, written before it is read).
- **A barrier elsewhere is refused**: in a branch or a loop, or in a function the kernel's function calls
  (`a_barrier_not_at_the_top_of_the_kernels_function_is_refused`); so is a function with barriers that returns a
  value.

`parent_group_w8.rs` is checked in (`the_f16_parent_kernels_rust_is_the_checked_in_one`), and matches the host's
parents on every node of the test documents, at groups of 8, 20, 32, 36 and 256 threads
(`the_kernels_rust_parent_is_the_hosts`). `parent_step` and `parent_link` are removed, with their Rust.

On the CPU, the same counts (this Mac at load 30–45, its cycles ±3 a node; the host's read 105, 479 and 133
instructions, 17–21, 93 and 20–29 cycles):

| Document | Two launches (removed) | One launch, in phases |
|---|---|---|
| 40,000 records | 71 instr, 17 cycles | 114 instr, 32–34 cycles |
| 40,000 numbers | 345, 89 | 377, 94 |
| random values | 92, 23 | 130, 36 |

- **The phases' cost is not the difference**: the saved state is 256 bytes a chunk of 8, a load and a store of 16
  registers. A loop of both phases with the phase a value (not a constant) was 5 instructions a node more.
- **The links that leave a group are the difference**: each is the walk, which the two launches never took (their
  steps were every node's, in device memory). A record's elements of the top-level array are 35 nodes apart, so a chain of 7
  links spans about 245, and nearly every element's leaves its group of 256; with the walk skipped (wrong parents,
  for the count only) records took 36 instructions a node fewer. On the CPU a chunk with one such lane runs the walk
  masked for all 8. On the GPU the same kernel is 1.45 times the two launches' speed on records.

## Step 3, a spike: chains pushed forward, no barrier (the user's, 2026-11-22)

The user: "why do we need a threadgroup barrier here?", then "go ahead with a spike on option 1". The barrier is
only there to share steps: a thread a node puts a parent's children far apart (each child's subtree between it and
the next), so a chain of 7 links spans warps and groups. The spike turned the chase around: a link is always to the
previous sibling, `i = j + size(j)` (held on every link of the test documents), so a chain's head (a node whose step
names its parent) writes its parent, then each sibling after it whose step links back, by sizes. Every node is
written once by one thread; none reads another's result. Its Rust matched the host's on every node at every group.

| | records (1.4 M nodes) | numbers (200 k) | random (10 k) |
|---|---|---|---|
| Metal, an M3: group | 0.399 ms | 0.058 ms | 0.040 ms |
| Metal, an M3: push | 0.878 ms | 0.100 ms | 0.066 ms |
| CPU, Rust at 8 lanes: group (instr, cycles a node) | 114, 28 | 381, 94 | 129, 32 |
| CPU, Rust at 8 lanes: push | 293, 74 | 921, 216 | 390, 100 |

Slower everywhere, 1.7 to 2.6 times. Every node still reads its own step to learn whether it heads a chain, and a
head then reads its chain's slots again (an inner sibling's twice: its size, and the slot before the next): two to
three slot reads a node against about one. And only the heads work in the loop, one lane in 8 at most, for as many
rounds as the chunk's longest chain: the threads a node keep the lanes busy, the threads a chain leave them idle.
The kernel and its Rust are removed; `parent_group` stays the one parent kernel.

## Step 3, a spike: a look-back, no barrier (the user's, 2026-11-22)

The user: "what exactly is shared? and what happens if no other thread produces it? How do we ensure that each step
is calculated exactly once? Why would we stop and wait for coordination rather than just take what is already
calculated?", then "yes, please do / on the GPU, we can ensure rising order by making waves each take a ticket (once
per wave, not 32 times) via an atomic add".

`parent_group` shares a group's steps, and only those: a chain that leaves the group recomputes its tail, each
thread of it apart, and the barrier is there only because threadgroup memory starts as garbage. A step is the same
whoever computes it, so it needs computing at least once, not exactly once, and nothing needs to wait.
`parent_back` (`kernels/f16_parent.uglykl`) is the look-back:

- **`out` is the shared word, every node's**: `NOT_READY` before the launch (`parent_back_clear`), then the node's
  step (its parent, or `LINKED | j`), then its parent. A thread stores its step first, then follows its links
  through the words: a parent ends the chase, a link hops on, and only `NOT_READY` is a step computed again. Every
  word a reader sees is true, so a stale one costs a step, never a wrong parent. (With final parents alone shared,
  numbers' links, each to the node before, all in the same 8 lanes, were recomputed back to the chunk's start: 1,785
  instructions a node.)
- **A thread's place is its SIMD group's ticket** (`ukl_sg_ticket`, `<ukl/simd.h>`): lane 0 takes one atomic add of
  the group's width and shuffles it to the lanes, so groups started later hold later nodes and look back at nodes
  mostly written. On the CPU's backends, whose groups and chunks run in their ids' order, it is the id (a macro,
  so the lanes stay consecutive). The node's work is a function of its place (`parent_back_node`): a local
  variable assigned the id would be each lane's in `rs.rs`, a parameter keeps the shape.
- `rs.rs`: a store of lanes' values at one address, seen in a pass before a call's result is found to be each
  lane's, is an error only if the passes settle with it.
- `rerail-ukl-aot` builds at opt-level 3 in dev: unoptimized, the generated Rust's inlined calls overflowed a test
  thread's stack.

Its Rust matches the host's on every node of the test documents at every group size. This Mac at load 7–11:

| Instructions, cycles a node | records (1.4 M nodes) | numbers (40 k) | random (107 k) |
|---|---|---|---|
| The host's read | 105, 18 | 479, 88 | 132, 33 |
| `parent_group` | 114, 29 | 377, 90 | 134, 40 |
| `parent_back` (its `NOT_READY` fill counted) | 60, 16 | 334, 76 | 84, 25 |

Metal, an M3, the GPU's clock, every node checked against the host's:

| | records | numbers (200 k) | random (107 k) |
|---|---|---|---|
| `parent_group` | 0.400 ms | 0.127 ms | 0.064 ms |
| `parent_back`: the clear, then the launch | 0.540 ms | 0.127 ms | 0.061 ms |
| the clear alone | 0.121 ms | 0.022 ms | 0.014 ms |

On the CPU the look-back is the fastest form yet on all three, the two launches' too (71, 345, 92 instructions).
On Metal its launch alone is at or under `parent_group`'s, and the clear (a write of every node's word) is what
puts it behind on records.

### The fill in the same launch, then a bitmap of published groups (the user's, 2026-11-22)

The user: "couldn't we do it in the same launch by having the threads cooperate by taking and filling large ranges,
incrementing a started and finished count, and then having them spin on the finished count once there is no more
work to do?", then "I am fucking telling you to do that", then "what about a bitmap with one bit per 32-bit value
saying whether it's occupied or not?".

- **The fill** (`ukl_sg_fill_lanes`, `<ukl/simd.h>`): a SIMD group takes a range from `started` (lane 0, read
  first, then one atomic add), its lanes fill it together, a fence, then `finished`; a group with no range left waits
  for `finished`. It waits only for ranges groups that are running took, which wait for nothing (0540 point 1), and
  boundedly (`UKL_FILL_SPINS`): a thread whose wait runs out reads no shared word, and its parent is the walk. On the
  CPU's backends each thread takes ranges alone. With `started` and `finished` in one cache line, records took 97 ms
  (the waits' polls slowed the takes); a line apart, 0.787 ms, no wait run out.
- **What is filled**: not `out`, but `seen`, a bit a SIMD group (a bit a 32-bit word would be `out`'s 1/32, 175 KB
  at 1.4 M nodes; a bit a group of 32 words is 5.5 KB). A group stores its steps, then publishes (a fence, the
  group's barrier, lane 0's or of its bit); a reader takes `out[j]` only where `j`'s group's bit is set (then a
  fence), else computes the step. `out` is never cleared. On the CPU a node of the reader's own launch group is
  written already (its chunks run in turn), and a group's last lane sets its bit.
- Correct on both from poison: `out` zeros (wrong parents), `seen` all ones (every group published), on the CPU at
  every group size and on the M3.
- `rs.rs` along the way: an atomic add of one value at one address was one atomic for the chunk, every lane given
  its old value (a miscompile: C's is each thread's); it is now the value times the running lanes, each lane's old
  value its own (`a_uniform_atomic_add_is_each_lanes`), another operation each running lane's in turn. An atomic at
  each lane's address, a shift right by each lane's count, and a function's one `bool` result are written too.

| M3, the GPU's clock | records | numbers (200 k) | random (107 k) |
|---|---|---|---|
| `parent_group` | 0.398 ms | 0.127 ms | 0.063 ms |
| `out` filled in the launch | 0.787 ms | 0.415 ms | 0.298 ms |
| `seen` filled in the launch | 0.554 ms | 0.175 ms | 0.083 ms |
| the same, `finished` read plainly first | 0.563 ms | 0.201 ms | 0.091 ms |

On the CPU (load 12–18) the bitmap's form is 106, 402 and 132 instructions a node (25, 116, 40 cycles), against
`parent_group`'s 115, 380, 135. Still behind on Metal: against the separate clear's launch (0.42 ms on records), it
adds a group's publish (a fence, a barrier, an or) and, before each hop's word, the bit's atomic load and a fence.

Tried next, at the user's word ("please test it"): a thread keeps the group it last saw published (its own first),
and a hop into it reads `out[j]` without the bit's load and fence. Correct on both; on the M3 0.584, 0.173 and
0.085 ms, the same as without it. The per-hop bit and fence are not the cost, and the change is not kept.

### Where the look-back's time goes (the user's: "find what is expensive in the shape", 2026-11-22)

Measured by switches in the kernel (each part taken out in turn; kept as `tmp/parent-back-ablation-switches.patch`,
not in the tree) and by counters of its hops, on the M3 and in the Rust at 8 lanes.

- **The hops mostly hit.** Records: 595 k nodes chase, 940 k hops, 52 k into a group not yet published (5.6%), 660 k
  into the reader's own group, 199 k into the one before. Numbers: 688 k hops, 14 k unpublished (2%), 625 k own
  group. Random: 96 k hops, 8 k unpublished (8%), 68 k own group. No chain is walked past 7.
- **On the GPU the shared-word protocol is free.** The publish's fence and barrier, the fence after the bit and an
  atomic read of `out[j]`, all three taken out: 0.553, 0.200, 0.087 ms against 0.560, 0.205, 0.090.
- **On the GPU the fill is the cost, as a barrier, not as bytes.** An empty launch is 0.109, 0.020, 0.013 ms; the
  fill alone 0.236, 0.056, 0.044. Neither its fence nor every wave's read of `finished` costs anything (a hot word
  read by every lane, read-write or read-only: the empty launch's time). But 2,613, 1,912 and 1,938 waves (a
  GPU's resident waves) find the fill unfinished and wait: the launch's whole first round idles until 5.5 KB is
  cleared. That is the separate clear's cost again (0.121 ms on records): a grid-wide sync point, whatever it
  writes. Without it the launch would be about 0.43, 0.15, 0.05 ms against `parent_group`'s 0.400, 0.127, 0.064.
- **On the CPU the fill is 13 instructions a node** (each thread's own counter loads and mask work, a chunk of 8
  at a time), **and its result known only at run time 17 more** (the walk for a fill that ran out stays in the
  node's code): 106, 402, 132 instructions a node; the fill run but `filled` then true, 89, 399, 117; no fill, 76,
  384, 104 (`parent_group` 114, 380, 134). Without the fill the look-back is the CPU's best on records and random;
  on numbers every form is the step's own cost (slot width 1: each escaped step a search of the side list).

### The two launches, taken apart (the user's: "can they be stacked sequentially?", 2026-11-22)

`parent_step` then `parent_link` (as removed above), each part on the M3 by the GPU's clock (the measuring example
and the old kernels kept as `tmp/parent-two/`, not in the tree):

| M3 | records | numbers | random |
|---|---|---|---|
| step alone | 0.306 ms | 0.078 ms | 0.032 ms |
| links alone | 0.281 ms | 0.042 ms | 0.030 ms |
| both, one command buffer | 0.586 ms | 0.116 ms | 0.059 ms |
| both, two buffers, no host wait / a host wait between | 0.583 / 0.550 ms | 0.121 / 0.121 ms | 0.063 / 0.062 ms |
| links without the walk past 7 | 0.151 ms | 0.025 ms | 0.018 ms |
| `out[i] = steps[i]` alone | 0.124 ms | 0.022 ms | 0.015 ms |
| `parent_group` | 0.399 ms | 0.127 ms | 0.065 ms |

- **Stacked, they cost their sum and nothing more.** The two launches share the command buffer's serial compute
  encoder; a host wait between adds only the host's wall (about 0.2 ms), not GPU time. Nothing needs the host.
- **The second launch's cost is the walk it never takes.** A chain is at most 7 links
  (`every_chain_of_links_is_at_most_seven`), so `clif_f16_parent` after the chase does not run on any tape; but its
  code in the kernel halves the launch's speed (0.281 against 0.151 ms on records), within 22% of a plain copy's.
  Without it the two launches would be 0.457, 0.103 and 0.050 ms: behind `parent_group` on records only.

### The two launches are the parent kernel again, a chain past 7 a corrupt tape (the user's, 2026-11-22)

The user: "isn't 'at most 7 hops' a guarantee of the format?", then "go ahead / if we hit more than 7, this should
be reported as tape corruption somehow; it should not be a silent failure".

- **It is the format's.** A leaf's slot names the next node's parent on every K-th sibling (K = 8,
  `rerail_tape_format::f16`, decision 0156 §1), and the writer sets it in one place (`writer16.rs`, a sibling index
  a multiple of K); a first child's step is its parent (the node before). A chain from sibling m runs back to the
  first multiple of 8 or the first child: at most 7 links on any tape the writer made.
- **`parent_step` and `parent_link` are back, the links with no walk.** A node still linked after 7 is a corrupt
  tape: its word NONE, and `out`'s word past the nodes' (0 before the launch) 1 more than one such node. The host's
  runner (`parent::run_two`, `parent::run_link`) returns it as an error, `parent::Corrupt` ("corrupt format 16
  tape: node N's parent is more than 7 links of siblings back"); the GPU harness reads the word after every run and
  runs a chain of 8 on the device (`a_chain_past_seven_links_is_a_corrupt_tape` on the CPU, at groups of 8, 20 and
  256: a chain of 7 a parent, of 8 the error).
- **The report is a word of `out`, by a plain store.** A store to a second buffer, under the chase's condition,
  made the launch as slow as the walk had (records 0.278 against 0.150 ms; numbers 0.040 against 0.026): an atomic
  max, a plain store, a SIMD group's vote and one lane's store, a store after `out[i]`'s, all the same. Binding the
  buffer and not writing it, or choosing NONE by a select, cost nothing; the store to `out[nodes]` costs nothing
  (0.150 ms). Why the second buffer costs this on Metal is not known; measured only.
- **`rs.rs`**: a store of each lane's value at one address (C's threads racing, any one's value standing) is each
  running lane's in lane order, the last one's left (`a_store_of_lanes_values_at_one_address_is_a_running_lanes`);
  it was refused.

| | records (1.4 M nodes) | numbers (200 k) | random (107 k) |
|---|---|---|---|
| Metal, an M3: two launches | 0.454 ms | 0.101 ms | 0.047 ms |
| Metal, an M3: `parent_group` | 0.399 ms | 0.127 ms | 0.064 ms |
| CPU, Rust at 8 lanes: two launches (instr, cycles a node) | 72, 20 | 344, 84 | 93, 32 |
| CPU, Rust at 8 lanes: `parent_group` | 114, 29 | 381, 97 | 134, 40 |
| CPU: the host's scalar read | 105, 17 | 482, 95 | 132, 33 |

(This Mac at load 7–13.) The two launches are faster than `parent_group` on both backends, but for records on
Metal (14% behind); `parent_group` stays beside them, and its walk past a group's links is a real case (its walk
past 7 is not reported: the host's read, `clif_f16_parent`, walks any chain).

### The step's Rust as hand-written SIMD would be (the user's: "please address all of these", 2026-11-22)

The step's Rust was slower than the host's scalar step because it did per lane what a hand-written NEON loop does
once a chunk. Each gap, and what `rs.rs` writes now (commits 6c931fa5d2, 94388e3d25):

1. **The window load, a gather.** Each lane's aligned word, shift and second word at its own 64-bit pointer.
   Now `ukl_ld_window` of 8 bytes is one unaligned load under `UKL_VEC` (memory.h: it touches exactly the words the
   aligned form reads), and an index of lanes `first + j * stride` (`V::Lin`, the stride a value read from the tape,
   carried through `+ - * <<`, a zero extension and a 64-bit base in `V::At`) is loaded as `vld2q_u32` pairs where
   the program finds `stride == 8` and no lane's index wraps, else gathered.
2. **64-bit lanes as `uint64x2_t`.** Now `[uint32x4_t; 2P]`, low halves then high, one mask register for both;
   `V::Z` where the high half is known zero. Carries, borrows, the product's cross terms, shifts across 32, compares.
3. **An `any()` after each early return.** A short side that ends (no loop, call or effect) merges by its mask.
4. **Values uniform across the chunk, per lane.** `i - 1` stays a `u32` of its first lane (a variable stored where
   all lanes run, or where the lanes off have returned, `Cx::left`), passed as `Sh::Iota`. A function that shifts
   it down by K or masks it to K bits asks its callers for `Sh::Run(K)`; the caller that made the lanes tests the
   chunk once (`(f & (2^K - 1)) <= 2^K - W`) and calls that instance, where `i >> 10` is one value (the block's
   lookup scalar code) and `i & 1023` consecutive lanes. A `?:`, a short circuit or a loop whose condition is one
   value in an instance of uniform arguments is Rust's, whatever the tape's level says.
5. **The side search for the whole chunk.** A `?:` arm that is a call of a function that reads no lane id, SIMD
   group or barrier runs, where at most 2 of the chunk's lanes take it, its instance of one value once a lane
   (a loop over the mask's bits); its offset loop is then a plain scalar loop. 4 lanes measured the same as 2.
6. **The links as a second launch.** Measured against the walk inline (`parent_group`, `parent_back`): the two
   launches cost fewer instructions and cycles on every document, so they stay.

A test kernel, `ukl_aot/kernels/wide.uglykl`, checks every 64-bit operation, the stride-tested window (8 and 4),
a `Run` instance and a rare call against a scalar model, at groups that end in part of a chunk.

Instructions (cycles) a node, `parent_bench`, profiling build, this Mac at load 11–21:

| | records (`parent_bench records 40000`) | numbers (`numbers 200000`) | random (`random 400000`) |
|---|---|---|---|
| the step, before | 50.0 | 330 | 73.5 |
| the step, now | 27.5 (5.0) | 58.3 (9.6) | 31.2 (7.4) |
| the host's scalar step | 53.9 (10.6) | 86.3 (10.8) | 57.3 (16.1) |
| two launches, now | 54.9 (11.0) | 112.0 (29.0) | 59.7 (17.5) |
| `parent_group` (walk inline) | 79.7 (19.0) | 135.7 (38.9) | 85.9 (25.3) |
| `parent_back` | 83.7 (21.2) | 172.8 (63.3) | 95.2 (33.9) |
| the host's scalar read | 104.7 (17.6) | 482.0 (94.9) | 132.1 (34.1) |
| hand-written scan (`hand::parent_all_scan`) | 27.9 (4.5) | 73.5 (9.8) | 34.5 (9.6) |

Then three more, each in the compiler or the library, none in a kernel:

7. **A variable is its number in its function.** The C lowering gives variables of scopes apart one frame offset (a
   loop's counter, then a later declaration), and rs.rs named its Rust variables by it: the two were one, each
   lane's if either was. A `Local`'s word 3 is the variable's number (`vector::VAR_IN_FUNCTION`), and rs.rs keys
   every variable by it. `wide.uglykl` declares a uniform width in the loop counter's slot again: one value.
8. **Narrow slots.** Under `UKL_VEC` the window of a whole value's bytes (1, 2, 4 or 8) is one unaligned load of
   those bytes alone (`memory.h`): only the words the aligned form touches, never past its last byte's. A lane's
   consecutive slots are then a vector load of their elements under the stride's runtime test, as 8-byte ones
   were. No table lookup across the chunk, which would read past the C's words. Widths 3, 5, 6, 7 (`ukl_ld_bits`)
   take the word path. `wide.uglykl` row 27 reads every width, 1 to 8.
9. **The link launch.** A round of a lanes loop costs as much for 1 running lane as for 8 (each address a
   gather). Where at most 2 lanes go on, each one's rounds to the loop's end are Rust's rounds of one value
   (`finish_scalar`): the lane's copies of the variables the loop reads, written back after. Only for a loop that
   leaves by its condition, calls nothing, writes only variables, and reads no lane id, SIMD group, barrier or
   atomic (a lane's rounds alone are then one interleaving of the threads'). A short circuit of one value and a
   cheap right side of lanes (variables only, no call) is the two masks and-ed, not a branch. A store of lanes'
   values at one address (C's race) is one store of the last running lane's value, not 8 tested ones.

| now | records | numbers | random |
|---|---|---|---|
| the step | 26.9 | 53.0 | 30.1 |
| the link | 22.8 | 55.2 | 26.1 |
| two launches | 49.7 | 108.7 | 56.7 |
| the host's scalar step | 54.2 | 86.4 | 57.9 |
| `parent_group` | 77.8 | 134.8 | 84.0 |
| `parent_back` | 82.5 | 166.7 | 93.0 |

Instructions a node, this Mac at load 31–35 (cycles not compared at that load). Two launches now cost fewer
instructions than the host's scalar step on records and random.

Left: numbers' link (55 a node, half of its two). Its chains are long and many lanes run them together, so each
round is a gather. Those rounds' indices are consecutive only among the running lanes, and a vector over them
reads past every address the C proves valid.

## Step 3, the engine's bulk parents: `label_parents` (the user's, 2026-11-23)

The user: "if we don't call it directly, then find which UKL kernel we _do_ run inlines this and use it from there"
and "improve current host performance on a kernel which actually runs (the "engine bulk pass")". The engine has no
host caller of the step. Its bulk parent pass is `label_passes.uglykl`'s `label_parents` (decision 0475), which the
CPU device runs as c++ of its UKL. It has a thread per word of 32 nodes, its slot read inlined (`tape_parent`,
`clif_tape_slot`), heads on a local atomic list, and 3 rounds of pointer jumping in threadgroup memory, between 6
barriers.

rs.rs now writes it (`ukl_aot/src/label_parents_w8.rs`, from the source as the engine assembles it, the format 16
define for its specialization constant). That took these, each in the compiler:

1. **Phases inside a callee.** A call of a function with phases spans them (`Part::Sub`): its first phase in the
   caller's, each next one a phase of its own, its state after the caller's. A loop of a constant count (at most
   16) whose body a barrier parts is its rounds one after another (`unrolled`, `Part::Set`).
2. **A struct copied from bytes no lane writes** (`DocView d = docs[file]`): read in place where its copy is only
   read, with no copy kept (decision 0353). An aggregate argument is each lane's address.
3. **Constant memory** (`static DATA`); a scalar loaded from it is its literal.
4. **A load at one index for all lanes:** where the running lanes' indexes are one (a view's fields, when a group's
   words are one file's), one load and a broadcast.
5. **Lanes a stride apart** (`up[lane * 32 + b]`): each lane's load and store at its step from the first's
   (`vld1q_lane_u32` / `vst1q_lane_u32`, a lane that does not run storing to the stack), where no index wraps;
   else each lane's index.
6. **A loop whose `continue`s are some lanes'** is rounds of one condition, with a mask of the round's lanes that
   continued; every lane comes to the step.
7. **Unsigned `/` and `%` of lanes by a power of two** are a shift and a mask, not each lane's division.

Instructions (cycles) a node, one run of `label_bench` (20 runs less 1, the setup and the c++ compile out), this Mac
under load. `cpu` is the CPU device's c++ of the same UKL (`RERAIL_GPU=cpu`):

| | records (700,002 nodes) | numbers (700,002) | random (190,963) |
|---|---|---|---|
| Rust, first run | 129.9 (29.8) | 155.7 (40.7) | 136.0 (41.6) |
| Rust, 4 | 116.9 (25.7) | 136.2 (30.7) | 122.3 (29.3) |
| Rust, 5 to 7 | 99.7 (21.1) | 119.4 (25.3) | 105.3 (23.3) |
| c++ (`cpu`) | 111.7 (40.5) | 157.0 (49.9) | 118.7 (58.0) |

The guard: `the_label_passes_parents_take_no_more_instructions` (ukl-aot) counts the test thread's instructions
(`thread_selfcounts`) and fails above a tenth over 98.1, 117.3 and 103.3 a node, or at the host's scalar read
(105.3, 491.9, 132.1). The checked-in test asserts the shapes of 4 to 7.

### The CPU device runs it (2026-11-23)

The checked-in Rust the engine runs lives in `rerail-ukl-rust` (no dependency): each kernel with the digest of the
source and defines it was written from (`sources.rs`, FNV-1a over the defines as `#define` lines, then the text).
rerail-ukl-vec's test writes both; ukl-aot re-exports the crate's `Launch`, so it is one definition.

`rerail_gpu::kernels::program_with` compiles the library as before, then `with_rust(text, defines)`. On the CPU,
a kernel whose program has a digest here runs as its Rust (`Run::Host`), its arguments the compiled library's
rows bound as before, each group in turn on the queue's thread. A launch that gives it a threadgroup row is refused:
its threadgroup memory is its own. Any other kernel, source, format or processor is the compiler's build, as
before. `RERAIL_CPU_RUST=0` keeps the compiler's build, for measuring.

The engine's test `the_label_source_is_the_one_its_rust_was_written_from` (device_metal) checks that the label
source a run's batch compiles has that digest, so a change to the passes or their pieces fails it until the Rust
is written again.

The device's run (`label_bench`, `cpu`), instructions (cycles) a node:

| | records | numbers | random |
|---|---|---|---|
| the device, its Rust | 99.1 (20.9) | 119.4 (25.9) | 105.0 (23.6) |
| the device, `RERAIL_CPU_RUST=0` (c++) | 111.6 (33.0) | 156.1 (38.9) | 117.6 (53.5) |

## Step 3, the leaf tests: `label_leaves` (2026-11-23)

The next kernel of the label passes: each node's leaf tests, a thread a word of 32 nodes, its tests' bits a block
of 8 in `bits[LEAF_BLOCK]`. rs.rs writes it now (`rerail_ukl_rust::label_leaves_w8`, checked in by rerail-ukl-vec's
`the_label_leaves_kernels_rust_is_the_checked_in_one`). rerail-ukl-aot's `the_label_passes_leaves_are_the_cpu_devices`
holds it to the CPU device's c++ build of the same UKL, over each document alone and all of them in one batch. What
rs.rs gained:

1. **A thread's array** (a variable over 8 bytes, its every access a 4-byte integer's at it plus an offset) is an
   array of registers of the chunk's lanes, each element one (`arrays`, `elem`). Its offset is one value for all
   lanes, or an index the vector tape proves one value in the lanes that run, kept as each lane's: the largest of
   the running lanes'. Not under barriers yet (a phase's state would have to hold it).
2. **A variable whose every read is of one value in the lanes that run** (the tape's `Uniform`: where control
   rejoins, a variable written apart is raised to the level that split it) is one value, wherever its writes run
   (`read_uniform`). The test loop's `j` was each lane's, because it is set inside the varying node loop. That made
   every test's dispatch a mask, where it is one branch.
3. **A `break` all the lanes in a round take together** (each statement between the loop and it an `if` of a
   condition of one value) is Rust's `break` of the loop's label (`Cx::exit`). It is not a mask of lanes that left.
4. **Each lane's addresses of a pointer** (64-bit lanes, as `[usize; W]`) are made once where the variable is set,
   and reused by its every read in that block (`addrs`, in `memo`). The same holds for the pointer plus one offset
   (a field). A loop forgets those of the variables it sets. The code's kind, field and flags were one rebuild each.
5. A function returning a pointer starts its result at null, not `0`.

The test documents had no code table. Their views pointed past their bytes, so both builds read whatever lay there,
and differed from run to run. `parent::json_doc` now places the language's codes after the tape, as a slab does.

`label_bench`, `LABEL_KERNEL=leaves`, 23 tests (each kind at several arguments), instructions (cycles) a node:

| | records | numbers | random |
|---|---|---|---|
| the Rust | 1577.1 (460.2) | 2120.8 (544.5) | 1712.2 (522.0) |
| the CPU device's c++ | 1734.2 (277.5) | 3313.8 (618.2) | 1856.0 (394.9) |
| the Rust, the code's tests alone (19) | 475.1 (92.5) | | |
| the c++, the code's tests alone | 822.1 (123.4) | | |
| the Rust, tests 18 / 19 / 21 / 22 | 342.1 (101.5) / 118.2 (24.8) / 394.0 (125.1) / 547.0 (179.1) | | |
| the c++, tests 18 / 19 / 21 / 22 | 340.2 (62.1) / 171.5 (31.5) / 287.9 (56.9) / 407.3 (79.1) | | |

The tests of a node's code are faster than the c++, and so is test 19 (a node's size). Tests 18, 21 and 22 are
slower. Each walks the tape (`tape_next`, a node's children, its siblings) in loops whose rounds differ from lane to
lane, and a round is a gather of each lane's. On records and random the whole is more cycles than the c++, so the
device does not run it yet (`rerail_ukl_rust::kernels` names `label_parents` alone). Next: the walks' loops.

## Step 3, the join program: `join_run` (the user's, 2026-11-23)

The user: "replace the hand-coded CPU join path by an uglykl path, so that both CPU and GPU are running the same
kernel (but one via AOT Rust and the other via Metal/whatever)". The kernel is `join_stages.uglykl`'s `join_run`, the
work-queue program Metal runs (`metal_join::program_pipe`, `JOIN_PROGRAM`), with the view reads of `view.h` (all
three formats: the format is the view's, chosen at run time). rs.rs now writes all of it: 1,039 instances of 370
functions, at 32 lanes, one SIMD group a group (`rs::kernel_group`: its barriers lower to nothing, its SIMD-group
operations act across the chunk's registers). What it took, in the lowering (`lower.rs`) and in rs.rs:

1. **C the program uses**: an enumeration at file scope (its enumerators `int` constants), `offsetof`, and a call
   whose callee is a parenthesized typedef (a cast).
2. **A thread variable's address passed** (`&mut`, `Sh::Ref`), **a thread array passed** (`Sh::Arr`), and **a
   structure of fields** (each field a variable, `fields`): copied into whole, passed whole, returned. A structure
   read whole has every byte a field's: the bytes no access names are fields too (`gaps`). Without them, the stage
   record `join_run` copies and passes to its stage dispatch had its op as `0`, and LLVM, rightly, removed every
   stage (the kernel compiled to 52 KB). A structure copied in from memory and only read is read where it is
   (`aliasable`), not copied. A structure of other uses is bytes of each lane's.
3. **`switch`** (an entry index; a case runs where `e <= j`; a `break` all lanes take, the label, else a mask),
   **`do`**, atomics of every operation (one at each lane's address, a loop over the lanes), a fence, the SIMD-group
   operations (ballot, reductions, shuffles), lanes of 1 and 2 bytes, aggregate results, updates of memory, a
   store at each lane's address.
4. **A loop whose breaks are its label's** no longer sees an outer loop's mask of lanes that broke: with it, a
   `break` of the inner loop wrote that mask and the loop went on (`label_leaves` lost a test's bit to it; its test
   against the c++ build caught it).
5. **At 32 lanes the code is written once over registers and lanes** (`rolled`): a register list that is one
   expression of its index is `core::array::from_fn` (`roll`), a 64-bit value's two halves too, and a gather is a
   loop over the lanes into a `[u32; 4P]` (`load_lanes`). `join_run`'s Rust went from 29 MB to 10.7 MB.
6. **A function the source declares a call** (`UKL_SHARED`, `UKL_LARGE`, `UKL_COLD`) is the Rust compiler's to inline
   or not when its instance is over 200 lines (`CALLED_LINES`), inlined otherwise. The label passes' reads are
   54 lines at most; left to LLVM they stayed calls, and in instructions (`label_bench`, 700 K nodes, 20 runs)
   `label_parents` took 1,511 M against 1,403 M inlined, `label_leaves` 24,837 M against 22,947 M.

`join_run`'s 10.7 MB of Rust compiles (rustc, opt-level 1) in 587 s at 6 GB, to 9.5 MB of code. That is not a
kernel to check in: the instances of the view reads are the bulk (`clif_verdict_entry` and `clif_cache_open` 3,800
lines each, twice; each format's `view_loc_*` read 1,900). Next: the code's size, then the kernel against Metal's
and the c++ build's results, then the CPU joins on it.

### The reads called, not written again (the user's: "under 200kb ... substantially smaller than 1MB", 2026-11-23)

The reads written once in CLIF (decision 0310) are compiled code on the CPU already (`rerail_clif::jit::Reads`). A
kernel's Rust now calls them through the launch's table (`Launch::reads`, `READS`' order: `Reads::table`), not as 32
lanes' vector code of their UKL (`rs::Read`, `call_compiled`): once where the arguments are each one value, else once
per running lane, its result gathered into the lanes. Their UKL passes a base `t` and the pointer's offset `a0` from
it, and computes addresses as offsets from `t`; the compiled read takes `t + a0`. So a read whose result is an
address from that pointer gives its result less `t`, and one parameter that is an address (`cache_done`'s) is passed
plus `t`: `reads::addresses`, a flow of the pointer through each read's CLIF, finds them
(`the_reads_addresses_are_these`: eight results, one parameter).

Two changes it needed in `rs.rs`. A structure copied whole that `fields` had left to be read in place, where its copy
then finds it cannot be (its source's bytes a write reaches), is marked (`unaliased`) and gets its fields from the
next pass. And a pass that learned something and then refused a statement ends only that pass: the refusal may be of
what it learned too late to apply.

`join_run`'s Rust: 10.8 MB to 2.53 MB (26,245 lines, 309 instances of 161 functions; the view, relation and lines
reads now 79 calls). The largest: `row_text_one` 210 KB, `txt_of` 174 KB (2 instances), `lat_merge_one` 145 KB,
`ukl_radix_scatter_tile_u32_prog` 134 KB (2). Not compiled yet. Next: the instances (one per shape where shapes
differ little), then the text of each (declarations, splats, masks).

### The instances and their text (the user's: "please address these", 2026-11-23)

`join_run`'s Rust, 2.53 MB to 669 KB (17,026 lines), in these steps (each its size after):

1. **Instances of one text, once** (2.33 MB). A function's instances are keyed by what a call knows of its arguments
   (`Em::inst`), and some of that changes nothing written. The module's last pass (`merge_instances`) writes once each
   set whose text is one but for its temporaries' numbers and its callees (each merged too, to a fixed point).
2. **A lane's loads and stores as loops** (1.70 MB). A gather, a scatter, a strided store: a loop over the lanes of
   their words, not 32 lines of `vgetq_lane_u32` (`lane_words`).
3. **Operations over the registers, once** (1.23 MB, then 1.11 MB). `roll` takes a list whose elements are one
   expression but for a constant offset from the register's index (`x[r]` and `x[r + 8]`, a 64-bit value's
   halves); the prelude has each NEON operation of whole registers over a kernel's arrays (`and_r(a, b)` for
   `from_fn(|r| vandq_u32(a[r], b[r]))`), its reductions (`maxv`, `minv`, `maxv_on`), and its words of registers
   (`words8`, `regs8`); indentation is a tab.
4. **Large functions written once** (959 KB). A function the source declares a call, or of more than 400 nodes
   (`vector::JOIN_NODES`), has one body for its calls, not one per context (`vector::joins`), at a call where
   neither it nor its caller reaches a sync point: a context is what shows a group's lanes reach one together, and
   a callee's result of one value what keeps its caller's control so. Its instances' shapes are joined in its Rust
   (`Em::joined`): a value's parameter each lane's where calls differ, the instance of a mask where any call's lanes
   part.
5. **Names and types** (854 KB). Temporaries are numbered in each function; a `let`'s type is left out where its
   value says it (`untyped`); a function that does not read the thread's id does not compute it.
6. **The lanes' accesses as the prelude's functions** (699 KB): an element of each lane's index (`ld_u32(b, ix, s)`,
   one load where the indexes are one), a running lane's index for one that is not (`ix_on`), a load or a store at
   each lane's address (`ldj_u32(|j| ...)`, `stj_u32(k, v, |j| ...)`), each lane's 64-bit address (`addrs`), and the
   operations of one register NEON has as several (`vmulhiq_u32`, `vlt64q_u32`, `veq64q_u32`, `vadcq_u32`) or a
   64-bit splat (`splat64`), which roll as NEON's do.
7. **Splats** (669 KB): the zero register a constant (`Z`), a value in every lane `splat(x)`.

**Compiled** (the user's "compile it and see"). At opt-level 1 `join_run` did not compile in 15 minutes: one thread
in LLVM's full unroll of loops (`LoopFullUnrollPass`), its time in `ScalarEvolution::forgetLoop`. The prelude's
functions of a loop over the lanes (`ld_*`, `ldj_*`, `stj_*`, `stjf_*`) were `#[inline(always)]`, inlined before
LLVM simplifies anything, so each of their 32-iteration loops was unrolled in the kernel's function, every unroll
walking that function's values. As `#[inline]` each is simplified (unrolled) on its own and then inlined: 158 s,
1.2 GB, 2.1 MB of code. Keeping long instances out of line instead (`#[inline(never)]`) did not end the unrolls'
walk and cost `label_parents` 2-4% of its instructions; it was not kept.

Every kernel checked in at 8 lanes is now also checked in at 32 (`rerail-ukl-aot`'s `*_w32`,
`the_kernels_rust_at_32_lanes_is_the_checked_in_one`), and its tests run both against the same answers (the scalar
kernels', the host's parents, the CPU device's c++ build's leaves): the Rust of more than two registers, which only
`join_run` had, is run. Writing them found an operation reading a 64-bit value's low half as an array of the
result's registers; such an operand is `lo(x)` (`Em::wide`).

### The oracle: `join_run` by hand (the user's: "hand-create an idiomatic Rust SIMD version of the kernel ... then identify the correct structure of the code ... I am SURE that we need much smaller functions", 2026-11-23)

`rerail_ukl_rust::join_run` is the join program written by hand from `join_stages.uglykl`: 2,609 lines, 90 KB,
117 functions (median 9 lines; the largest `row_text` 186, the turn loop `run` 94, the item's dispatch `item` 88).
It calls the compiled reads through `Launch::reads` as rs.rs's Rust does (36 of them, their places checked against
`READS`: `the_reads_are_at_their_places`). The CPU device runs it in place of its c++ build for the join program's
three texts (formats 0, 16 and 16z, `JOIN_PROGRAM`; the format is the view's, read by the compiled reads, so one Rust
serves all three; `the_join_programs_rust_is_of_this_source` fails when the source changes). For it the CPU device
now gives a kernel's Rust the compiled reads: `rerail_ukl_rust::Reads`, a table and the JIT's code that holds it,
made by rerail-gpu (`rerail_clif::jit::Reads`) for the library whose kernel calls them (`calls_reads`), none for
the others, and kept by that library.

**Checked.** `setfix::device`'s 39 tests (`RERAIL_GPU=cpu`: the strata, the parity families, recursion, lattices,
texts, verdicts, views) pass on it, as on the c++ build (`RERAIL_CPU_RUST=0`). Two things they found. `substr`'s
range wraps on the device (`substr_range` of extreme numbers); the Rust's did not, and a debug build panicked. And
a kernel's Rust that panicked took the CPU queue's thread with it, and the batch's waiter waited for ever: the
launch now catches it, and the panic is the launch's error.

**Measured** (this Mac, rustc 1.99.1, `rustc --crate-type=lib -C codegen-units=1` of the file alone):

| | rs.rs's `join_run` (32 lanes) | the hand-written `join_run` |
|---|---|---|
| Rust | 669 KB, 309 functions (`row_text_one` 58 KB, 1,329 lines) | 90 KB, 117 functions (`row_text` 186 lines) |
| opt-level 1 | 158 s, 1.2 GB, 2.1 MB of code | 0.6 s, 42 KB of code |
| opt-level 3 | 337 s, 1.9 MB of code | 0.8 s, 51 KB of code (17 functions after LLVM's inlining) |
| a group's threads | 32 of its 256 | 256 |

**The structure.** The program's work is a thread's: an item's job is a function of `t` (`stage_count_one`,
`dedup_claim_one`, `lat_merge_one`, ...) whose control is the thread's own (a candidate loop runs to its binding's
end, a probe takes its hash's chain, a CAS loop its own retries, a text its own length), and whose every step calls a
read or follows an address. The oracle is that, read as a CPU runs it:

1. **A group is a loop over its threads**, each a call of the item's function (`Prog::item`): scalar, its branches
   one arm, its loops its own, its calls calls. LLVM inlines what it chooses.
2. **What a SIMD group does together is one thread's walk.** `row_text`'s automaton over a text's bytes (the GPU's
   chunked prefix of states), a `.`'s or a `/`'s place (a ballot, a minimum), a text's characters (a sum), a compare
   eight bytes a word: each a loop over the bytes.
3. **The group's own steps are plain code.** Lane 0 takes an item, a barrier, every thread reads it: one thread takes
   it and runs it. A scan's tile is a running sum; a sort's tile a histogram and a stable scatter.

rs.rs writes the other shape: every value of every function is the 32 lanes' (`[uint32x4_t; 8]`, 16 registers for
64 bits), both arms of a branch run under masks, a read is a loop over the running lanes, and a function has an
instance for each shape of its arguments. For code whose lanes part at the first branch this multiplies the text
and buys nothing: the vector work is masks. And `kernel_group` takes one SIMD group for the group, so of a group's
256 threads only 32 run.

What rs.rs must do so that its `join_run` is the oracle's size:

- **A width for each function, not one for the kernel** (step 3's bar: "width 1–32 per block"). A function whose
  control depends on its thread's values (a loop bound, a branch on a read, an address followed) is width 1: written
  once as the source's function, scalar, a call. Width 32 is for a block whose lanes keep together and whose
  operations are each lane's (consecutive loads, arithmetic, selects), as `label_parents`' are.
- **A kernel cut at its barriers, each part a loop over the group's threads** (all of them, by SIMD groups where a
  part is width 32), what a thread holds across a cut in arrays of the group's threads.
- **A SIMD-group operation a cut too**: the lanes up to it a loop, the operation over their values, the lanes after
  it a loop.
- **No instances at width 1** (one value has one shape), and no inlining of the generator's own: functions the size
  of the source's.

### The program's own waste, on Metal and the CPU alike (the user's: "Fix the join program that metal runs as well", 2026-11-23)

A profile of the closure stratum (400 nodes, 157,228 rows; the CPU device running `join_run`) put most of the time
in the program's own work, not the joins': the round's dedup table cleared at its room, and the head's index rebuilt
every round. Both are the program's, so both are fixed in `join_stages.uglykl` (Metal's) and `join_run` alike.

1. **The dedup's places are its round's rows'** (done). A dedup's table was twice its input's room, cleared every
   round: 2^20 places for a union of room 311,855 that a round fills with a few thousand rows. Its clear, its claims
   and its marks now take their mask from the round's rows (`dedup_mask`: twice `min(totals[in_slot], cap)`, a power
   of two, 16 at least, within its room), and the clear's items are those rows and 4, four places a thread. Closure:
   245 ms to 227 ms. Small: the room's clear was not the cost the profile's `fill_u32` was.
2. **The head's rows a set the program keeps, not an index each round builds** (done). A round's new rows were its
   union made distinct, less the head's rows (an ANTI stage probing an index of all the head's columns). That index
   is live (`IX_LIVE`): every round sorted every row the head had and placed its offsets over all its buckets
   (262,144, 24 rounds), so a component's time was its rounds times its rows. Indexes of a range of rows
   (`Index::from`) do not help: the rounds are the device's (a template the program repeats), so the host cannot
   choose a round's ranges. A fold (the lattice heads' `lattice_fold`, a key map kept across rounds) does not either:
   it writes the relation as it claims, so its gate must come first and count the union's rows. Against an exact
   bound (the closure's reach, 160 K rows) a round of mostly held rows would stop the program for nothing.

   Now a set head keeps a map of its rows (`SetHead::map`), laid out as a lattice head's key map, in its block, twice
   the program's bound. Each program first empties it and places the relation's rows in it (`Batch::set_map`, code 4
   of `lat_rehash_one`), so a round that a gate stopped in the last program leaves nothing behind. The round's dedup
   (`Batch::dedup_less`) marks a row new where it is the first of its rows and its map holds no equal row
   (`dedup_held`, reading the relation in place). Its scatter places each new row in the map as the row it becomes:
   the relation's rows as the device holds them, and its place among the new ones, which `append_live` writes there.
   The gate reads the new rows' count, as before. `round_index` and the ANTI stage are gone.

   **Measured** (this Mac, debug build, `joins_timed_on_the_cpu_and_the_device`, the CPU device running
   `join_run`): closure, 400 nodes, from 227 ms to 56.8 ms wall (70 ms CPU); the CPU's scalar joins take 42.8 ms
   (59 ms CPU). The keyed join (100 K × 100 K) is unchanged: 80 ms against the CPU joins' 54 ms. **Checked:**
   `setfix::device`'s tests pass on the CPU (40). On Metal they fail no test that passed before: m3 fails 11, 12
   before (`a_leveled_recursion_on_the_device_has_the_cpus_levels` now passes; the rest are "a stratum past one
   launch and its budget's relaunches"), and the mini fails 2, 3 before (text functions).

### The jobs as lanes, driven phase by phase (the user's: "complete this", 2026-11-23)

**Driven phase by phase, no queue** (`Prog::drive`). The CPU device runs a launch's groups one after another on one
thread, so the first group runs every item. It walks a launch's items in order: the items a last launch left, the
seeds, then each phase's items (its segments' ranges, `Work`) as the phase before ends. It counts them as the host
counts pending items (`ctl[2]`). Only when the budget ends with items left are they written to the slots, for the
next launch, which takes them back first. `push`, `take` and the turn bound are gone.

**At 8 lanes** (`rerail-simd`, the relation reads of `crate::lanes`):

- **Relational stages** (`stage_lanes`): a join, semi-join or anti-join of a relation in its block, probed by keys
  or read whole. Each step does 8 bindings' keys, screen, probe, candidate rows, key and column tests, binds,
  arithmetic and row program as vectors, each lane's walk masked past its own length. A product's, quotient's and
  remainder's overflow test is each lane's: NEON has no vector test for them.
- **Dedups and index keys** (`dedup_lanes`):
  - A mark walks 8 rows' places at once (gathers of the places, of the rows they name, of a set head's map and the
    relation's rows).
  - The claims and map placements take their places from 8 rows' hashes; each compare-and-swap is its lane's.
  - An index's keys are 8 rows' hashes a step.
- **Still per thread:**
  - stages over views (`has`, a parent's children, a view read in place);
  - text stages and aggregates;
  - the lattice fold's claim, best and merge.

  Neither stratum below uses them.

**Measured** (m3, `--profile profiling`: every crate at opt-level 3; `RERAIL_THREADS=1`, so the CPU's joins run on
one thread, as the CPU device's program does; `joins_timed_on_the_cpu_and_the_device`, the best of three, the
process's CPU time):

| stratum | the CPU's scalar joins | the join program at 8 lanes |
|---|---|---|
| keyed join, 100 K × 100 K (249,484 rows) | 113.9 ms | 103.4 ms |
| closure, 400 nodes (157,228 rows, 24 rounds) | 56.6 ms | 72.6 ms |

The keyed join is 9% faster than the scalar joins. The closure is 28% slower; where its time goes at opt-level 3 is
not yet measured (a profile of this build is the next step), and that gap is the next work.

### The large joins, profiled (the user's: "we should be able to do a LOT better on the larger joins, especially if we vectorize things like hash computation and the hash searches and scans", 2026-11-23)

Each change was found by a profile of the build above (xctrace and `sample`, the program's thread), then measured.
The program's own time is what the timed test prints as "its program" (waited on). The rest of a run is the host's
work, the same for both columns: the rule's SQL and the rows' way out.

**A stage's candidates as a stream** (`stage_lanes::Lanes::walk`), as a GPU's threads expand them:
- A tile of 256 bindings has its groups found 8 bindings a step: keys hashed, screened and looked up together.
- The tile's groups are then expanded into chunks of 512 candidates, each one a binding and a place. Every step
  takes 8 candidates, so every lane has work, however long or short each binding's group is. Before, a lane walked
  its own binding's group, and the step waited for the longest.
- A count's verdicts are a chunk's exclusive prefix sums. A binding's count is the sums at its candidates' end less
  their start.

**Reads by index.** A column's rows in one chunk, an index's rows and a dedup's map are gathers from one base by
32-bit indices, scaled by shifts (`lanes::gather64_ix`, `gather32_ix`, `scatter64_ix`). Before, each lane read
through its own 64-bit pointer. `rerail-simd`'s pointer offsets also scale by shifts where the element's size is a
power of two.

**A scatter as a stream compaction.** A range's rows out are contiguous, from its first offset.
- Where the range has room for 8 more rows, each column's passing lanes move to the front with one table lookup
  (`swizzle_dyn`, NEON's `tbl`, a 256-entry table) and are stored as one 64-byte write. Later rows overwrite the
  places past them.
- The masked per-lane scatter is left for a range's last places.
- The per-lane stores had been half the program's samples. Page faults were not the cause: writing every page of
  the output first changed nothing.

**Lanes built by an inlined loop.** `core::array::from_fn` stayed a call inside the walk, its array passed through
memory. `rerail-simd` and the lane reads now build each vector with an unrolled loop.

**Measured** (m3, opt-level 3, one thread, the program's time, the best of three):

| | keyed join, 100 K × 100 K | closure, 400 nodes |
|---|---|---|
| the jobs at 8 lanes, the section above | 33.0 ms | 33.3 ms |
| the candidates as a stream | 23.9 ms | 27.6 ms |
| reads by index | 21.9 ms | 26.1 ms |
| the scatter packed, lanes inlined | 19.2 ms | 24.7 ms |

Whole runs, against the CPU's scalar joins: the keyed join takes 83.4 ms against 105.8 ms. The closure takes 62.0 ms
against 51.3 ms.

**Where the program's time is now** (the keyed join, `sample`, about 1,270 samples):

| Phase | Samples |
|---|---|
| Scatter | 354 |
| Count | 283 |
| Dedup mark | 228 |
| Dedup claim | 116 |
| Kept rows' scan | 73 |

The line profile is flat, no line above 6%. The time is in the reads every candidate needs:
- its row's block and chunk (`chunk`, `rel_at`);
- its key's and its binds' columns, by gathers;
- its binding's slots.

Two costs remain that are structure, not vector code:

- **The count and the scatter each walk every candidate.** Both hash the keys, find the groups, read the rows and
  test them: half the program's time is that walk, twice. The scatter cannot be cheaper while it repeats the count's
  probe. Removing the second walk means the count keeps what it found: each tile's passing candidates, compacted, as
  a derived fact of the stage, which the scatter then writes out. That changes the program's phases on Metal too.
- **A dedup's rows are hashed twice.** The claim hashes every row, then the mark hashes it again and walks the same
  places.

## Step 3, first build (superseded: `jit.rs`, a reference only)

- **Cranelift compiles the code tape** (`rerail-ukl-vec`'s `jit`, `RERAIL_CPU_EXEC=jit`), not the vector tape: one
  function per section (the kernel's, each body's), each instruction its handler's work inline, in 128-bit vectors
  (4 lanes of 32 bits, 2 of 64). The SIMD group's state stays the interpreter's (`Sg`'s files, in memory):
  - A section stops at a barrier, a poll, the end, and a call of a section that can stop and its return. The
    driver in `run` does there what the interpreter's loop does (the call stack, the windows, the turns), and enters
    the next section at its resume point through a jump table at its entry.
  - A call of a section that cannot stop (none of the gate's text functions can) is direct: compiled code calling
    compiled code, through the dispatch's table of entries, the windows advanced in registers. Each call through the
    driver had cost two entries and exits of the caller.
  - Within a block, a value stored to a file is taken from the register it was computed in, not loaded again; the
    stores still go to memory, which a stop and an interpreter's step read. The lanes of a mask are made once per
    block. A store to a lane's variable under all 32 lanes is plain stores, not a blend.
  - What it does not compile itself (an atomic, a shuffle, a reduction, a generic opcode, a division, a copy past 64
    bytes) is one step of the interpreter, called with the SIMD group: the same code, the same results. The crate's
    kernel tests run every kernel both ways and compare.
- **The compiled code is kept** in the device's store with the code tape (`vec/<key>.jit`, keyed by the code tape's
  key and the host's target and features, `jit::host`). It holds no address of its process (the interpreter's step
  and the entries are the dispatch's; its constants and jump tables are PC-relative), so its bytes are the code: a
  process loads them (`jit::load`, `define_function_bytes`) rather than compiling. A compile costs about 20 µs an
  instruction (`row_text`'s 39 K: 0.8 s; `pp_setup`'s 189 K: 2.6 s), and each of nextest's processes compiled every
  kernel it launched: before the store, the join tests took 62 s compiled against 38 s interpreted. The test
  `compiled_code_kept_as_bytes_runs_the_same` fails on a host whose code would need relocating.
- **`switch` is a jump by its value through a table** (`Switch` and its sorted `Label`s: the interpreter searches
  them, Cranelift's `Switch` makes a jump table or a tree), to its case; the cases follow in order, falling through.
  On a value that varies, a round for each value the running lanes have: the first remaining lane's, and the lanes
  that share it, through the same jump. A round costs one comparison, not one for each label. The text functions'
  switches have dozens of labels: in a profile of the compiled `row_text`, the label comparisons were half its
  time, and the join tests on the interpreter went from 82 s to 34 s (`a_switch_runs_each_lanes_case`).
- **A launch's setup** was most of a short launch: the program's tape checked whole to find its data (twice), and
  each SIMD group's frame allocated and cleared at each group's start. The data's place is found once (`Program`),
  and a thread keeps the slots of its last four launch shapes. A lane's variables start as C's locals do, unset, as
  on the coroutine backend.
- **Gate** (2026-11-21), with `RERAIL_GPU=cpu`:
  - `rerail-gpu`'s 25 tests pass on the JIT: on this Mac 2.1 s against the interpreter's 3.2 s (warm store, at
    6 threads); on an x86-64 Linux host (AVX2) 2.3 s against 4.0 s. The crate's 16 tests pass on both.
  - pp's arena tests (`pp_grid`'s kernels): the 9 pass.
  - The join and setfix device tests (`join_stages`' kernels): the 44 pass. On a Mac (M3 Max, nextest at 8
    threads, warm store, a load of 4–8):

    | Executor | The 44 tests | `parity_random_texts` |
    |---|---|---|
    | Compiled (this step) | 15.7 s | 14.4 s |
    | The interpreter | 32.0 s | 24.7 s |
    | The coroutine backend | 12.1 s | 11.2 s |

    The times are whole tests, their host side included, which is the same for each executor.

