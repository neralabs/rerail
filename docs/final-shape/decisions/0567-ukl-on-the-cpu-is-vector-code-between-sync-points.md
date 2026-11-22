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

