# 0440. GPUs beyond Metal: kernels written once in UKL, a device layer under the lanes, memory by class

Numbered 0400 where it was written (renumbered at the merge of Native-and-GPU-strata, whose 0400 is the small root's).

Status: draft (2026-11-18); the four questions of §9 answered by the user the same day (recorded there, and folded
into §3.3, §5, §7 and §8). The user asked for Rerail to
run on any GPU, starting with CUDA, "without rewriting all of the kernels over and over again", by way of MLDB's
UKL (the Ugly Kernel Language). It covers the platform layer, the kernel language, subroutines, the C
preprocessor, a library of primitives (prefix sums and the like), and memory: shared against partitioned, GPUs
with little RAM, and several GPUs and hosts. Amends 0353 between memory domains (§7.2).

## Intro

**The problem.** Every GPU path in the engine is Metal. That covers:

- the kernels, written in Metal's shading language (MSL), by hand or generated as MSL text;
- the host side, which calls Metal's API through the `metal` crate;
- the memory model, which assumes the CPU and the GPU share one memory.

Each path compiles only under `cfg(target_os = "macos")`. A Linux build has no GPU at all. The DGX Sparks on the
farm have an NVIDIA GPU that the engine cannot use.

**What changes.** There are three layers, each written once:

1. **Kernels in UKL.** A kernel is written in C with UKL's macros for the parts that differ between GPU languages:
   address spaces, kernel signatures, barriers, atomics and SIMD-group operations. A short per-backend header (a
   "shim") turns the macros into MSL, CUDA C++, HIP or OpenCL C. Our own C preprocessor (`rerail-c-pp`, decision
   0327) applies the shim at run time. The code generators write UKL in place of MSL.
2. **A device layer** under the executor's GPU lane covers devices, buffers, modules, submissions and timing.
   Metal is its first implementation and CUDA its second.
3. **Memory by class.** Unified-memory GPUs (Apple, and the Spark's GB10) keep today's model, blocks read in place.
   Partitioned GPUs (discrete cards with their own RAM) need a residency policy and a per-device budget.

**One kernel, before and after.** Today a kernel is MSL, so only Metal can run it:

```c
// MSL: Metal only. `device` and [[...]] attributes are Metal syntax.
kernel void count_kinds(device const uint* kinds [[buffer(0)]],  // input, GPU memory
                        device atomic_uint* hist [[buffer(1)]],   // output histogram
                        uint id [[thread_position_in_grid]]) {    // this thread's index
    atomic_fetch_add_explicit(&hist[kinds[id]], 1, memory_order_relaxed);
}
```

In UKL each parameter is a row of four: access, type, name and bounds. The shim generates each backend's
signature from those rows:

```c
// UKL: the same source for Metal, CUDA, HIP and OpenCL.
DEFINE_KERNEL2(engine, count_kinds,
    ROBUFFER, uint32_t,   kinds, "[n]",   // read-only buffer of n kinds
    RWBUFFER, atomic_uint, hist, "[k]",   // read-write histogram of k bins
    GID0,     uint32_t,   id,,)           // global thread index, axis 0
{
    atom_inc(&hist[kinds[id]]);           // a relaxed atomic increment on every backend
    KERNEL_RETURN();
}
```

The host binds arguments by the same rows, which the shim also turns into reflection data (`GEN_ARG_INFO`). So a
launch names its arguments in the same way on every backend.

**Status.** This record holds a survey of what exists (§1), measurements taken on a Spark (§2), the proposed
approach (§3 to §7), an order of work with gates (§8), and the decisions it needs from the user (§9).

## 1. What exists (survey of `final-shape-serverplan-quality`, e5d076eb0)

### Kernels

| Kind | Where | Size |
|---|---|---|
| Hand-written MSL | 21 `.metal` files: the transducer's packed passes, tape reads, label passes, views and ids, the block program, the accel joins, tables, bits and closure | 13.0 K lines |
| MSL generated from Rust | `clif/src/msl.rs` (CLIF to MSL, decision 0310's third backend), `metal/src/block_serial.rs` and `block_program.rs` (block programs, 0066), `transducer/packed/metal_join.rs` (join stages, 0095), `accel/src/gpu/join.rs` and `tile.rs`, `compile/matcher_metal.rs`, `setfix/device.rs` | about 20 emitters |
| UKL | `forest/src/mldb/`: UKL's macros and its CPU and Metal shims, and the random forest kernels (`randomforest_kernels_ukl.h`, `_common.h`, 2.6 K lines). `forest/build.rs` preprocesses them with the host's `cc -E` into MSL at build time, on macOS only | 3.0 K lines |

What the hand-written MSL uses, which sets what UKL's shims must cover:

- **Atomics:** 213 relaxed, 8 sequentially consistent. They are load, store, add, or, and, max, sub, exchange and
  compare-exchange.
- **`coherent(device)`:** 58 uses (the cache helpers and the work queue).
- **Barriers:** 57 threadgroup barriers.
- **SIMD-group operations:** about 60. They are `simd_shuffle`, `simd_shuffle_up`, `simd_broadcast`,
  `simd_prefix_exclusive_sum`, `simd_sum`, `simd_any` and `simd_is_first`.
- **Kernel attributes:** 380 `[[buffer(n)]]` bindings, plus the grid, threadgroup and SIMD-lane positions.
- **Other:** 29 templates, 53 `popcount`/`ctz`/`clz`, and 4 `as_type`.
- **Not used:** function constants (the kernel store specializes by a `constant` index instead) and quad
  operations.

### Host

- **`rerail-metal`** (`native/crates/metal`) holds the device and queue, the buffer budget and its leases, mapped
  no-copy buffers, the launch budget and its dispatch records, the pool and the slabs.
- **`rerail_accel::gpu`** holds the process's one device and queue (`shared`), the leases and evictor, and the
  kernel store (`kernels`, decision 0395).
- **`rerail-device-metal`** mounts the transducer's Metal sources.
- The API surface is concentrated: 48 non-test files touch the `metal` crate's types, in 376 lines. Most go through
  `runtime`, `launch`, `shared` and `kernels`.

### Memory

Unified memory is assumed throughout:

- Tapes sit in residency slabs that the device reads in place (0032).
- Blocks are linear, with relative offsets, and are the same bytes on the CPU, the GPU and disk (0260, 0266).
- Relations are blocks that both sides read in place (0325).
- No block is ever copied (0353, immutable).
- The Metal budget is half the run's budget, on top of it (0126).

### UKL's state upstream

MLDB's `opencl-random-forests` branch (2022-02) has:

- `builtin/cuda/ukl/ukl_cuda_shim.h`, a CUDA shim of 341 lines;
- `builtin/cuda/compute_kernel_cuda.cc`, which is still a copy of the Metal host (`mtlpp` calls), so the CUDA host
  side was never written;
- an OpenCL host (`compute_kernel_opencl.cc`). Its random forest kernels (`.cl`) are a separate hand port, not UKL
  output.

Only the CPU and Metal shims were brought into `forest/`.

### Preprocessor

`rerail-c-pp` (0327) is a clang-shaped C and C++ preprocessor. It supports `__VA_OPT__`, `#include_next`,
`__has_include`, configured predefines, and an oracle against `cc -E` (`examples/pp_oracle.rs`). It is not used
for kernels yet.

## 2. Measured on a Spark (2026-11-18)

**The hardware.** An NVIDIA GB10 with compute capability 12.1:

- 48 SMs, a 32-lane warp and a 24 MB L2;
- 100 KB of shared memory per SM, 48 KB per block statically, and more with opt-in;
- `integrated = 1`, with 122 GB of LPDDR5X shared with the CPU;
- CUDA 13.0 and driver 580, with NVRTC and nvJitLink installed.

**Its device attributes:**

- `pageableMemoryAccess = 1`, through the host's page tables;
- `hostNativeAtomics = 1`;
- `concurrentManagedAccess = 1`;
- cooperative launch and memory pools.

**Reading memory in place.** One kernel summed 256 MB of `u64` (best of 4):

| Memory the kernel reads | Result | Time | Bandwidth |
|---|---|---|---|
| `cudaMalloc` (copied in) | correct | 1.17 ms | 229 GB/s |
| `malloc`, read in place | correct | 1.88 ms | 143 GB/s |
| file `mmap` (shared), read in place | correct | 1.72 ms | 156 GB/s |
| anonymous `mmap`, read in place | correct | 1.81 ms | 148 GB/s |
| the same, after `cudaHostRegister` | correct | 1.81 ms | 148 GB/s |
| the same, with transparent huge pages (`madvise`) | correct | 1.79 ms | 150 GB/s |
| `cudaHostAlloc` (pinned, mapped), read in place by the GPU, written by the CPU | correct | 1.21 ms | 223 GB/s |
| copy of pinned memory to `cudaMalloc` memory | — | 5.10 ms | 53 GB/s |

A GPU atomic into `malloc`ed memory was seen by the CPU.

- **So the Spark is a unified-memory GPU, like Apple's.** A kernel can read a block where it lives, including a
  file-backed `mmap`, with no registration. The engine's in-place model carries over unchanged.
- Pageable memory (`malloc`, `mmap`) read in place runs at about 0.65× of `cudaMalloc` memory. Registration and
  huge pages do not close the gap. **Pinned memory from `cudaHostAlloc` does**: the GPU reads it in place at 223
  GB/s, 0.97× of `cudaMalloc`, and the CPU reads and writes it as ordinary memory. So on a GB10 the block pool
  (0342's allocator) takes its slabs from `cudaHostAlloc` and loses nothing; file-backed blocks (0295's mapped
  slabs) are read at the pageable rate, or are filled into pool slabs where the planner finds that pays (E1).
- A copy into device memory is no help on a unified device: 53 GB/s, slower than reading in place.
- `cudaHostRegisterReadOnly` on a file mapping is refused ("operation not supported"). Nothing needs it.

**A discrete card: a V100 (16 GB, sm_70) on a cloud VM, driver 580, CUDA 12.9** (2026-11-18). This is the
partitioned case (§7.2). The same probe, 256 MB of `u64`, best of 4:

| Memory the kernel reads | Result | Time | Bandwidth |
|---|---|---|---|
| `cudaMalloc` (device memory) | correct | 0.54 ms | 500 GB/s |
| `cudaHostAlloc` (pinned, mapped), read in place over PCIe | correct | 21.8 ms | 12.3 GB/s |
| copy of pinned memory to `cudaMalloc` memory | — | 21.7 ms | 12.3 GB/s |
| anonymous `mmap` after `cudaHostRegister`, read in place | correct | 21.9 ms | 12.3 GB/s |
| `malloc`, anonymous `mmap`, huge pages, file `mmap`, read in place | illegal access, or a crash | — | — |

`hostNativeAtomics = 0`. A GPU atomic into `malloc`ed memory faults.

- **Host memory is about 40× slower to read than device memory here**, and pageable memory cannot be read at all.
  A block a kernel reads more than once is copied into device memory first, as Q1's answer says (§7.2).
- Reading pinned memory in place costs the same as copying it, so a block read once can stay in place.
- The card has 16 GB, so §7.3's limits apply. Volta is the oldest card CUDA 12 builds for: CUDA 13 dropped sm_70.

**UKL on CUDA, as it stands.** We compiled the vendored random forest kernels against upstream's CUDA shim
(`nvcc -ptx -arch=sm_121`). It failed with 42 errors, all from the shim falling behind the kernels:

- a missing `uint64_t` typedef;
- `registerCudaKernel` undefined;
- `DEFINE_KERNEL2`'s parameter rows out of step, so arguments are passed in the wrong order;
- `std::shared_ptr` in device code.

The macro language holds up. The shim is a draft.

## 3. The kernel language: UKL, extended

**One source dialect.** Every hand-written kernel, and every generated one, is UKL: C (a C++ subset where
templates are needed) plus UKL's macros. The shims are:

| Backend | Compiler | Shim | Notes |
|---|---|---|---|
| Metal | Metal front end, then the binary archive (0395) | `ukl_metal_shim.h` (exists) | output must equal today's MSL in behaviour, checked per kernel |
| CUDA | NVRTC to PTX, then nvJitLink to cubin per `sm_XX` | `ukl_cuda_shim.h` (upstream's draft, repaired) | first new target |
| ROCm (HIP) | hipRTC | near-copy of CUDA's | wave64 on CDNA (see below) |
| OpenCL | `clBuildProgram` (C 1.2 or 2.0), or SPIR-V | upstream had one | in-place reads need OpenCL 2.0 shared virtual memory (SVM) |
| CPU | the host's C++ compiler, at run time (`RERAIL_GPU=cpu`) | Rerail's `builtin/cpu/ukl/ukl_cpu_shim.h` (stackful coroutines; upstream's, over MLDB's runtime, is not carried) | for debugging kernels, not a product path (progress, "The CPU backend") |
| WebGPU | not by macros | — | §3.4 |

### 3.1 What the macros must grow to cover

From the survey:

- **Atomics with an order:** `atom_*_explicit(x, v, order, scope)`.
  - Metal has relaxed device atomics, plus `coherent(device)` for cross-threadgroup visibility.
  - CUDA has `cuda::atomic_ref` with thread scopes, and `__threadfence`.
  - Two scopes are enough: threadgroup and device.
  - `coherent(device)` becomes `UKL_COHERENT_GLOBAL`. On CUDA it is plain `volatile` loads or `ld.relaxed.gpu`.
- **SIMD-group operations with masks:** shuffle, shuffle-up, broadcast, prefix-exclusive-sum, sum, any and ballot.
  - On CUDA (Volta and later) every one takes a member mask (`__shfl_sync`, `__ballot_sync`), because threads of a
    warp are scheduled independently.
  - The UKL operations take the active-lane set implicitly (a full warp) and are documented as convergent: they
    must be called by every lane of the group.
  - Code that relied on Metal's lockstep without a barrier must add `ukl_simdgroup_barrier()`, which is
    `__syncwarp()` on CUDA.
- **The SIMD width is a value, not 32.**
  - Apple and NVIDIA use 32 lanes. AMD's CDNA uses 64 and its RDNA 32 or 64. Intel uses 8 to 32.
  - Decision 0359 (a short walk's 64 nodes are one SIMD group's) and every `simd_*`-based scan must read
    `UKL_SIMD_WIDTH`.
  - On a backend whose width differs from what a kernel was written for, the kernel is specialized or refused,
    never silently wrong.
- **Threadgroup memory budgets per device.** Apple has 32 KB. The GB10 has 48 KB statically and up to about
  99 KB with opt-in. The occupancy rule (keep it well under the maximum) is a fraction of the device's value, not a
  fixed number.
- **Kernel attributes:** grid, threadgroup and SIMD positions and sizes (`GID*`, `LID*`, plus SIMD lane and group
  ids, to add). Today's `[[buffer(n)]]` parameter blocks (`constant uint* p`) become `LITERAL` rows or one
  `PARAMS` struct passed by value. CUDA passes it in its parameter space, with no buffer.
- **Bit operations and casts:** `ukl_popcount`, `ukl_clz` (exist), `ukl_ctz`, `ukl_as_type<T>`.

### 3.2 Subroutines

UKL already has two kinds:

- **`UKL_FUNCTION(v, Ret, Name)`** is a plain device function. It becomes an `inline` function in MSL and a
  `__device__` one in CUDA.
- **`SYNC_FUNCTION` / `SYNC_CALL` / `SYNC_RETURN`** mark a function that contains a barrier. On a GPU it is an
  ordinary call. On the CPU shim it is a coroutine that yields at each barrier, which is how the CPU backend runs a
  threadgroup's threads together.

The rules every backend can hold to:

- no recursion, which Metal forbids;
- no function pointers, so dispatch is a `switch`, as `clif/msl.rs` already does for irreducible control flow;
- no `goto`;
- address spaces stated on every pointer parameter (`UKL_GLOBAL_PTR(T)`, `UKL_LOCAL_PTR(T)`).

Functions shared by several kernels live in UKL headers. The tape reads are generated, as below.

### 3.3 Generated code

The emitters write UKL text in place of MSL text. The largest is `clif/src/msl.rs`, which already writes C-shaped
code: `uint`/`ulong`, inline functions, and `while`/`switch` in place of `goto`. It becomes `clif/src/ukl.rs`, and
its MSL-specific parts become macros:

- `device const uchar*` becomes `ROBUFFER(uint8_t)`;
- `coherent(device)` becomes `UKL_COHERENT_GLOBAL`;
- `thread` becomes the default address space.

So decision 0310's three backends (Cranelift, Rust, Metal) become Cranelift, Rust and UKL, and UKL reaches every
GPU. The other emitters (block programs, join stages, matcher, tiles) change in the same way, one at a time, each
checked against the MSL it wrote before. Decided (the user, Q2: "write UKL"): there is one emitter per kind of
code, and it writes UKL; no emitter writes a backend's language.

### 3.4 WebGPU

WGSL is not a C dialect. It has no preprocessor, no pointers into arbitrary buffers, no 64-bit integers, and
subgroups only as an extension. Macros cannot reach it. WebGPU is reached through the IR path instead: a
CLIF-to-WGSL backend for the generated code, and the library's primitives written for it, with `u64` emulated as
`vec2<u32>`. It comes after CUDA, ROCm and OpenCL, and is planned separately.

### 3.5 OpenCL: how UKL can reach it (a spike, 2026-11-18)

**The problem.** The library (§5) and the generated reads are C++: 26 templates in five headers (16 in
`memory.h`, over a load's address type), six reference parameters (`UKL_PRIVATE uint32_t&`), one method (`lz4.h`'s
`at`), and the reads' functional casts (`uint32_t(x)`: 3,419 in `tape_z.h`, 5,616 over the five files). OpenCL C,
the language every OpenCL driver compiles from source, has none of these.

**How MLDB did it.** Not through UKL. MLDB's OpenCL kernels (`randomforest_kernels.cl`, upstream's
`opencl-random-forests` branch) are OpenCL C 1.2:

- a prelude that defines UKL's macros for OpenCL (`ROBUFFER(T)` as `__global const T*`, the barriers, the atomics,
  `SYNC_CALL` as a plain call);
- the shared code in a C subset (`randomforest_kernels_common.h`), with no templates;
- `__kernel` wrappers written by hand;
- the SIMD-group operations emulated through `__local` memory and atomics;
- arguments bound by name through `-cl-kernel-arg-info`.

So UKL's macro layer was designed to admit C. Our library has since moved away from it.

**The routes, tried on this Mac** (Apple M5 Max: OpenCL 1.2, OpenCL C 1.2, 32-bit atomics only, no subgroups, no
SPIR-V):

- **A. The library in a subset that is both OpenCL C and C++.**
  - The C++ constructs are replaced as follows:
    - a template becomes overloads, made by a macro per instance, marked overloadable (`UKL_INLINE` adds clang's
      `__attribute__((overloadable))` on OpenCL; every OpenCL compiler we target is clang-based: Apple's, Intel's
      NEO, AMD's, Mesa's, pocl);
    - a reference becomes a pointer;
    - a method becomes a function;
    - a functional cast becomes a C cast (one line in `clif/src/ukl.rs`).
  - Tried: the real generated `tape_z.h` (all of 16z's reads, 200 KB), with its casts rewritten and an OpenCL
    section of about 40 lines for the library calls it makes. It built in 2.0 s on Apple's OpenCL and ran. A
    probe of overloads, `noinline`, structs and pointer out-parameters built and ran too. Templates are refused
    (`unknown type name 'template'`).
  - The cost: a run-time compile from source, as on Metal and CUDA. No toolchain is needed where it runs.
  - NVIDIA's OpenCL front end may lack `overloadable`; NVIDIA is CUDA's anyway.
- **B. The library as is, as C++ for OpenCL, compiled to SPIR-V.** Homebrew's clang 23 compiles C++ for OpenCL
  to SPIR-V with no other tool (`-cl-std=clc++2021 --target=spirv64`). The same `tape_z.h`, kept as C++, gave a
  1.1 MB module in 0.6 s. The driver loads it with `clCreateProgramWithIL`, which needs OpenCL 2.1 or
  `cl_khr_il_program` (Intel NEO, Mesa rusticl, pocl). Apple's OpenCL cannot load it, so it cannot be tested on a
  Mac. Kernels are generated at run time, so clang would be needed at run time wherever it runs.
- **C. A lowering of our C++ subset to OpenCL C with the C parser in the tree** (instantiating templates where
  they are used, references to pointers). It is possible, but it is a compiler to keep for constructs that A
  removes once.

**What OpenCL lacks either way:**

- **Subgroups.** On a device without them (Apple's 1.2), the SIMD-group operations are emulated. MLDB's way is
  through `__local` memory, which needs the whole workgroup to reach each barrier. The simpler way is a group of
  one lane: `UKL_SIMD_WIDTH` 1, every reduction its own value. With `cl_khr_subgroups` (or Intel's), the native
  ones are used.
- **64-bit atomics.** The library uses only 32-bit atomics (§5).
- **Reading in place (0353).**
  - Without OpenCL 2.0's fine-grained SVM, a block is a buffer made over its own pages (`CL_MEM_USE_HOST_PTR`).
  - On a device that shares the host's memory (an integrated GPU, Apple's), that is the same bytes.
  - On a discrete device the driver may stage it, and that would be a copy. So a discrete OpenCL device needs SVM
    or does not take blocks.

**The choice (the user, 2026-11-18): A.** UKL source, the library and the generated reads included, is a C subset
that OpenCL C 1.2 accepts. It also keeps compile times down: no templates to instantiate. The rules:

- **No templates.** A function over several types is written once in a macro that defines it per type, or, where
  call sites must not name the type (the loads and stores take a `uint32_t` or a `uint64_t` address), each variant
  is marked `UKL_OVERLOAD`: nothing in C++ (Metal, CUDA, the CPU), `__attribute__((overloadable))` in OpenCL C.
- **No references, methods, namespaces, `auto` or `constexpr`.** Pointers, functions, `#define`d constants.
- **Casts are C casts:** `(uint32_t)(x)`, not `uint32_t(x)`.
- **`UKL_STATIC_ASSERT(cond, msg)`**, not `static_assert`.
- Only `backend.h`'s sections use their own languages: they are where each system's primitives are written.

**Done (2026-11-18): the library is in the subset.**
- **Templates.** `memory.h`'s loads, stores and atomics are defined per address type by macros (`uint32_t`,
  `uint64_t`), and marked `UKL_OVERLOAD`; `ukl_min` and `ukl_max` are defined the same way per scalar type.
  `radix.h`, `histogram.h` and `lz4.h` include a body once per type (`ukl/body/radix_tile.h`, `histogram_part.h`,
  `lz4_decode.h`), whose parameters are its names (`UKL_RADIX_NAME(f)`, `f##_u32`) and its type. The decoder's
  output is a `UKL_GLOBAL uint8_t*`, or any output a body instance gives an address macro (`UKL_LZ4_AT`).
- **References** are pointers (`ukl_wq_take`, `ukl_wq_idle`, `ukl_lz4_length`); **structs** are `typedef struct`.
- **The generated reads** (`clif/src/ukl.rs`, and the structs of `view`, `view_reads` and `ids`) write C casts and
  `UKL_STATIC_ASSERT`. A struct's text includes `<ukl/backend.h>` first, as it is also pasted into hand-written
  kernels ahead of the library. The five headers have no functional cast left (there were 5,616).
- **Unchanged behaviour:** the library's 16 tests on Metal, CUDA (V100) and the CPU; the reads' gate on all three;
  the engine's reads tests, the device join tests and the label kernels on Metal.

**The SIMD group on any width, and its state as parameters (the user, 2026-11-18).** Two choices settle the
subgroup problem above:

- **`UKL_SIMD_WIDTH` is the device's native width**, a constant each compile is given (32 on Metal, CUDA and the
  CPU; an OpenCL device's own subgroup width; 32 when it has none and they are emulated; 1 as a last fallback).
  The library takes any power of two from 1 to 32. The ballot and match stay 32-bit masks; a 64-wide device
  (AMD) needs a wider mask type first.
- **`__UKL_PARAMS` and `__UKL_ARGS`**: every function that reaches a SIMD-group operation takes `__UKL_PARAMS`
  after its parameters and passes `__UKL_ARGS` after its arguments. They are empty where the hardware holds the
  group (Metal, CUDA, the CPU). Under OpenCL's emulation they carry the threadgroup's scratch (`__local` memory,
  which OpenCL C 1.2 can declare only in a kernel), declared by the kernel's entry.
- **The rule under emulation**: OpenCL 1.2's only wait is the workgroup's barrier, so every SIMD-group operation
  must be reached by every live thread of the threadgroup, not only by its SIMD group.

**Done (2026-11-18):**
- `__UKL_PARAMS`/`__UKL_ARGS` are threaded through `simd.h`, `group.h`, `scan.h`, `workqueue.h`, `backend.h`'s
  primitives, the radix body and the library's test kernels (a script found every function that reaches a
  primitive, to a fixpoint).
- `group.h` takes any width: with more SIMD groups than lanes (a 1,024-thread group of width 8), the partials are
  summed by a loop and scanned by one thread. `UKL_TG_SCRATCH` is a word per SIMD group of the largest group
  (`UKL_TG_MAX`, 1,024) and the total.
- `radix.h` needs 8 lanes or more (its counts by SIMD group fill a threadgroup's memory below that); narrower,
  its kernels are left out, and `radix_sort_pairs` says why.
- The device gives the preprocessing (`Device::preprocess()`, with its defines) and its width
  (`Device::simd_width()`); the tests' references use the width.
- The CPU backend runs a kernel as a narrower device would (`RERAIL_CPU_SIMD_WIDTH`), and checks the rule
  (`RERAIL_CPU_OPENCL_RULE=1`: a SIMD-group operation waits for every live thread of the threadgroup, so a kernel
  that breaks the rule fails with its waits named, where it would hang on the device).
- Measured: the library's 16 tests pass on the CPU at widths 32, 16, 8 and 1 (the radix sort skipped at 1) and
  under the rule, the work queue's test included (its SIMD groups stay together); unchanged on Metal and on CUDA
  (V100: 16 + 7, the reads' gate).

**Done (2026-11-18): OpenCL runs UKL** (`RERAIL_GPU=opencl`).
- **The gate.** Every library header, the generated reads included, preprocessed for OpenCL
  (`rerail_ukl::Backend::OpenCl`, as C: no `__cplusplus`), is OpenCL C 1.2 that clang's front end accepts, at
  widths 32, 8 and 1 (`every_library_header_is_opencl_c_1_2`); its preprocessing equals clang's.
- **The shim** (`forest/src/mldb/builtin/opencl/ukl/ukl_opencl_shim.h`): a `__kernel` entry from the rows, `RWLOCAL`
  rows as `__local` arguments the host sizes, `SHARED` rows and the SIMD group's scratch declared in the entry. For
  the host, one-thread reflection kernels beside each kernel (`Name__ukl_info`: the rows' layout and text) and
  each reflected structure (`Type__ukl_layout`), as OpenCL cannot read a program's constants back; type kinds by
  `_Generic`. MLDB's older calls (`SYNC_*`, `atom_*`, `ukl_simdgroup_*` through the caller's scratch word) for the
  forest's kernels.
- **`backend.h`'s OpenCL section**: the SIMD group emulated through `__ukl_sg` (a word a thread, two barriers an
  operation), or one lane at width 1; native subgroups (`cl_khr_subgroup_shuffle`, `_ballot`) are refused as not
  written, as no device we can test has them. **Coherent memory is atomics**: OpenCL 1.2 has no coherent
  qualifier, and on Apple's GPU a `volatile` load from another threadgroup's write read a stale line (the 16z
  decode cache's bytes read 0). So an atomic load is `atomic_or(p, 0)`, and `memory.h` takes two hooks
  (`UKL_COHERENT_LOAD64`, `UKL_COHERENT_STORE_BITS64`): a coherent load reads its words' halves atomically, a
  coherent store sets its bits by `atomic_and` then `atomic_or` per half, never touching a neighbour's bytes. On
  Metal, CUDA and the CPU the hooks are absent and `memory.h` is the same code as before.
- **The runtime** (`gpu/src/mldb/builtin/opencl/compute_kernel_opencl.rs`): the system's OpenCL loaded by `dlopen`
  (no link dependency). The device must share the host's memory: a buffer is host pages the device reads in place
  (`CL_MEM_USE_HOST_PTR`, 0353); a discrete device (NVIDIA's OpenCL on the V100) is refused with its reason, as it
  would stage copies (SVM is not written). A queue holds its work until a flush, which enqueues it on its root's
  in-order queue. A wait for a host-signalled event holds the rest on the host, and the root's later flushes
  behind it, until the signal: Apple's `clFlush` blocks on a barrier on a user event, so the device cannot hold
  it (`a_wait_holds_the_roots_later_work_until_the_signal`, which fails if the wait is dropped once reached, as a
  first version did). The device seconds are the kernels' profiled times. The device's largest threadgroup is
  `Device::max_threadgroup()` (256 on Apple's), and the scratch is sized to it (`UKL_TG_MAX`).
- **The forest's kernels are in the subset** too (`typedef struct` arguments, no `constexpr`, a prototype by
  `UKL_FUNCTION`), and on every device but Metal they are built from UKL source (Metal keeps the build step's).
  Their SIMD-group calls keep OpenCL's rule (the CPU's check passes them).
- **Measured on Apple's OpenCL (M5 Max):** the library's 16 tests and the runtime's own, at widths 32 and 1; the
  reads' gate (every read of every node of the plain, 16z and format-15 tapes); the forest's 40 tests (training,
  prediction, armed prediction). The library's kernels build in 0.24 s, the forest's in 0.41 s. Unchanged on Metal
  (gpu, the reads' gate, the forest, rerail-device-metal's 56), CUDA (V100: gpu, ukl, the reads' gate, the
  forest) and the CPU.
- **Open:** native subgroups where a device has them (Intel's, Mesa's); a 64-lane mask type (AMD); SVM for discrete
  devices; the engine's own kernels (the GPU lane, 0440 step 3) on OpenCL; a device other than Apple's.

## 4. The C preprocessor

The kernels are preprocessed in-process by `rerail-c-pp`, never by the host's `cc`:

- **The input:** the UKL source (a file, or an emitter's text) and the backend's shim directory.
- **The predefines:** the backend (`__CUDA_ARCH__=1210`, `__METAL_VERSION__`, and so on), `UKL_SIMD_WIDTH`, and
  the device's limits.
- **The output:** the backend's source, handed to NVRTC, hipRTC, the Metal front end or OpenCL.
- **Cached:** in the kernel store (0395), keyed by the source's hash, the backend and the predefines. So a warm
  process neither preprocesses nor compiles.
- **What this removes:** `forest/build.rs`'s dependency on a host C++ compiler. It also lets generated kernels use
  the same headers as hand-written ones.

The check before anything depends on it: `rerail-c-pp`'s output equals `cc -E`'s, token for token, on the random
forest kernels for Metal and for CUDA. UKL leans hard on the preprocessor:

- a 200-argument `COUNT_ARGS`;
- GNU's `, ## __VA_ARGS__` comma elision;
- `__VA_OPT__`;
- token-pasted macro names through two levels of `CALL2`.

Any gap is fixed in `rerail-c-pp`.

## 5. The library

**Primitives written once in UKL, tested on every backend.** These exist today as Metal code spread over several
files, and the library gathers them into UKL headers, with each primitive's interface stated once:

| Primitive | Today | Levels |
|---|---|---|
| Prefix sum (exclusive and inclusive), segmented | `simd_prefix_exclusive_sum` uses, `metal_parallel.metal`, the join stages' count, scan and scatter | SIMD group, threadgroup, device-wide |
| Stream compaction and scatter by offsets | `metal_join.rs` `stage_count`/`stage_scatter` | device-wide |
| Radix sort (16, 24 and 32-bit keys), key and value | `accel/src/gpu/join.rs` | device-wide |
| Reduce, histogram | `common.metal`, `bits.metal` | threadgroup, device-wide |
| Bitsets and bitmap containers | `bits.metal`, 0045 | thread, SIMD group |
| Hash table build and probe | `table.metal` | device-wide |
| Work queue in a persistent dispatch | `metal_work_queue.metal`, 0027 | device-wide |
| Tape and relation reads | generated from CLIF (0310, 0325) | thread |

**Three rules for the library:**

- **The call is the interface; the implementation is per backend** (the user, Q3: "same code, high-level calls
  will optimize per backend behind the call"). A kernel or a host plan calls `ukl_scan_exclusive`,
  `ukl_radix_sort_pairs`, `ukl_compact`, and so on. Behind each call:
  - a portable UKL implementation, which every backend has and which is the reference in tests;
  - per backend, the fastest known form: CUB (CUDA, compiled by NVRTC from its headers), rocPRIM (ROCm), Metal's
    own SIMD-group operations, a single-pass scan where the backend guarantees progress.
  The choice is the backend's and the measurement's, never the caller's. A backend form replaces the reference
  only when it gives the reference's results on the library's tests.
- **Device-wide scans are written for the weakest progress guarantee by default.** Metal guarantees no forward
  progress between threadgroups, so a single-pass scan's decoupled look-back can hang. The reference is the
  two-level form (reduce, scan, then downsweep). As with 0027, every wait is bounded.
- **Calls at two levels.** Device-wide calls are made from the host plan (a sort, a scan of a whole column).
  Group-level calls (`ukl_simdgroup_*`, `ukl_threadgroup_*`) are made inside kernels. Both are behind the same
  rule.

## 6. The device layer (host)

**A trait for what every backend provides**, in `rerail_accel::gpu`, with Metal as its first implementation:

| Part | Metal today | CUDA |
|---|---|---|
| Device and properties | `MTLDevice`; simd width 32, 32 KB threadgroup memory, unified | `cuDeviceGet` and attributes: warp size, shared memory, `integrated`, `pageableMemoryAccess`, VRAM |
| Buffer over a block, in place | `newBufferWithBytesNoCopy` over a page-aligned slab | the block's pointer itself (unified; §2); or pinned and mapped (`cuMemHostRegister`) on a partitioned GPU |
| Device-resident buffer | `StorageModePrivate` | `cuMemAlloc` / a memory pool |
| Module and function | MSL to AIR to a pipeline; binary archives (0395) | NVRTC to PTX, nvJitLink to cubin per `sm_XX`; `cuModuleLoadData` |
| Kernel store | `~/Library/Caches/rerail/metal-kernels/<device>/` | `~/.cache/rerail/gpu-kernels/cuda/<sm_XX>-<driver>/` |
| Submission | command buffer and compute encoder; serial dispatches see the last one's writes | a stream; launches in order on it; CUDA graphs for repeated batches |
| Completion and timing | `waitUntilCompleted`, GPU start and end times | events: `cuEventRecord`/`cuEventElapsedTime` |
| Launch budget and dispatch records | `launch.rs` | the same records, from the trait |

**What does not change:**

- the executor's lanes (0100);
- leases in place of a lock (0344);
- device work belonging to its job and committed before a wait (0386);
- a kernel compile failure being fatal (0382);
- the kernel store's maintenance compile (0395).

These are written against the trait. "Metal" in their names becomes "the GPU lane".

**Bindings: our own, opened at run time** (changed from `cudarc` while building step 4, 2026-11-18). The forest's
CUDA runtime (`forest/src/mldb/builtin/cuda/compute_kernel_cuda.rs`) opens `libcuda` and `libnvrtc` with `dlopen`
and takes the 36 entry points it uses by name. The engine builds with no CUDA toolkit and runs without a GPU where
none exists, as `cudarc` would allow. We chose our own because:
- it needs no feature that pins a CUDA version: the entry points used have the same ABI on CUDA 12 and 13;
- it adds no dependency.
ROCm is the same shape through `libamdhip64` and hipRTC. OpenCL goes through `opencl3`.

**Builds.** The CUDA backend compiles on Linux and Windows under a `cuda` feature (on by default on Linux). The
GPU code stops being gated on macOS and is gated per backend. The Sparks build it as farm hosts (Linux, aarch64).

**Migration.** The 48 files that name the `metal` crate move to the trait in place. The Metal implementation is
the code that is there today, so this step changes no behaviour. The parity tests (`metal_semantic_conformance.rs`
and the rest) gain a CUDA run.

## 7. Memory: shared, partitioned, and small

### 7.1 Two classes of device

- **Unified (U):** Apple; NVIDIA's GB10, GH200 and GB200 (through ATS and NVLink-C2C); AMD's MI300A; integrated
  GPUs.
  - The GPU reads host memory in place, coherently.
  - Today's model holds unchanged: blocks in place, no copies, the GPU's budget on top of the run's (0126).
  - The only new choice is where a block's pages come from, which is a bandwidth question (E1).
- **Partitioned (P):** discrete cards over PCIe. A PCIe 4 or 5 link moves 25 to 60 GB/s, against 1 to 3 TB/s for
  the card's own memory.
  - A kernel can still read pinned host memory over the bus, in place ("zero-copy"). That is right for data read
    once in order (a tape streamed through a pass) and wrong for data read many times (a relation probed by a
    join, an index).

The device layer reports the class from the device's attributes (`integrated`, `pageableMemoryAccess`,
`hostNativeAtomics` on CUDA; `hasUnifiedMemory` on Metal). It never guesses from the name.

### 7.2 Blocks across memory domains: copies, and the coherency the engine manages

Decided (the user, Q1): "blocks will need to be copied to and from devices, and for us to manage coherency
between blocks read and written by different kernels on different hosts and GPUs (a host may have more than one GPU
too)". This amends 0353: **within one memory domain** no block is copied, and every reader reads it where it lives,
as 0353 says. **Between domains** a block is copied, and the engine, not the driver, keeps the copies coherent.

**Memory domains.** A domain is memory that every processor attached to it reads in place:
- a host's RAM, shared by its CPUs and by its unified GPUs (Apple, GB10);
- each partitioned GPU's own memory: a host with two discrete cards has three domains;
- another host's RAM, reached over the network;
- disk: a block's mapped file (0260, 0295) is where a frozen block can always be read back from.

Every device in the device layer (§6) names its domain. Two devices in one domain share their blocks with no copy.

**Replicas and versions.** A block is named by its id and its version. Frozen versions never change (0260's
mutable-then-frozen, 0266's deltas), so a frozen version may have any number of replicas, one per domain, and they
never need to be invalidated: a write makes a new version, it never changes an old one. That turns coherency from a
cache protocol into bookkeeping:

| A block's state | Replicas | A kernel may |
|---|---|---|
| frozen version v | any number, one per domain, all identical | read v in any domain that holds it |
| mutable (being written) | exactly one, in the writer's domain | write it there; readers elsewhere read the last frozen version |
| frozen v+1 from a delta | the delta is shipped to replicas of v and applied there (O(delta), 0260) | read v+1 where it has been made |

**The directory.** One table per process (`rerail_blocks`, beside 0342's allocator) records, for each block: its
current version, the domain that owns its mutable version if any, and which domains hold which frozen versions,
with an event per replica for when its copy completes. Across hosts the directory is per host, and a host asks the
block's owner (named in the block's id) for a version it lacks.

**Kernels state their reads and writes.** UKL's parameter rows already say it: `ROBUFFER` is a read, `RWBUFFER`
and `WOBUFFER` are writes (MLDB used the same rows for its hazard tracking, which `forest/` did not bring over).
Before a launch on a device, the scheduler, for each argument:
1. **read:** finds or makes a replica of the needed version in the device's domain, and waits on its event;
2. **write:** takes ownership of the block's mutable version in the device's domain (moving it if another domain
   owns it), so no two domains write one block;
3. **after:** a written block is frozen when its writer's work ends (0386: device work belongs to its job and is
   committed before a wait), which publishes the new version to the directory.

Copies run on the devices' copy engines and the network, overlapped with kernels, and the scoreboard (0113) waits
on their events as it waits on a kernel's. Within one domain each step is a no-op, so a unified device pays nothing
and today's plans run unchanged.

**Eviction.** A frozen replica can be dropped whenever another domain, or the disk, holds the same version. A
mutable block is written back (or frozen and published) before its pages go. The evictor (0126, 0342) chooses
among replicas by least recent use and by what re-fetching them would cost.

**The planner prices copies.** Placement (0304, 0312, 0321) adds a transfer term: the bytes of each read block's
version not already in the device's domain, divided by the measured bandwidth of the link between domains
(on-host copy engine, PCIe, NVLink, network). Data read once in order can instead be read over the link in place
(pinned host memory read by a partitioned GPU, "zero-copy"), and the planner prices that choice the same way.

### 7.3 GPUs with little memory

The budget becomes per device. A device's budget is the smallest of three:

- the configured `--gpu-mb` (today `--metal-mb`);
- its free memory on a P device;
- 0.5× the run's limit on a U device (today's rule).

The machinery to stay inside it exists and is generalized, not replaced:

- **Leases and one evictor** (0126, `shared::set_evictor`, 0342's one block allocator): idle buffers go first, then
  the least recently used resident blocks.
- **Work that outgrows the device is split along an axis it is independent on** (0016): batches of files (0066,
  0067), morsels of a scan (0278), regions read in parts (0323).
- **Joins larger than the device** are radix-partitioned by key hash. Each partition's build side must fit, and
  partitions are streamed through.
- **A refused lease runs the stratum elsewhere**, as today (0126: "a lease past the Metal budget is refused, and
  the stratum runs on the CPU").
- **Managed-memory oversubscription** (letting the driver page) is not a plan. It thrashes under the engine's
  random access. It is a tested last resort at most.
- **Work budgets never change results** (0334). A smaller device changes where and in how many pieces work runs,
  never its findings.

### 7.4 More than one GPU, more than one host

Several GPUs on a host, and several hosts, are the same model as §7.2: each is a lane with its own domain, budget
and directory entries; the scoreboard places work on lanes, and the directory moves versions between domains. Peer
copies between GPUs use NVLink or PCIe peer-to-peer where the devices allow it, and go through host memory
otherwise. The first CUDA slice runs one GPU; the directory is built with domains from the start so that a second
GPU or host adds a lane, not a redesign.

## 8. Order of work, with gates

Each step is committed and checked on its own. "Checked" means named tests on the step's subject, on the Mac and
on a Spark.

| Step | Work | Done when |
|---|---|---|
| E1 | Spark: block pool slabs from `cudaHostAlloc` (measured: 0.97× of `cudaMalloc`, §2); still to measure: `cuMemCreate` with CPU access, and the CPU's speed on pinned slabs | the pool's allocator for U-class CUDA is chosen; the price of reading a file-backed block at the pageable rate is known |
| 1 | `rerail-c-pp` preprocesses UKL | token-equal to `cc -E` on the random forest kernels for Metal and CUDA; `forest/build.rs` uses it; no host `cc` |
| 2 | Repair the CUDA shim; the forest's kernels on CUDA through NVRTC | the random forest tests (`rf_kernel_test.rs`) give Metal's results on a Spark: the cost model runs on CUDA. This is the smallest real GPU user that is end to end |
| 3 | The device trait, with Metal behind it | no behaviour change on Metal (named Metal tests); the 48 files moved |
| 4 | The CUDA implementation of the trait (run-time bindings, §6) and the kernel store for CUDA | the forest runs through the trait on a Spark |
| 5 | The library (§5) in UKL, tested on the CPU shim, Metal and CUDA | each primitive equal across backends on random inputs and edge sizes; Metal's speed not worse than today's per primitive |
| 6 | `clif/src/msl.rs` becomes the UKL backend | every generated read equals the tape crate's on every test tape, on Metal and CUDA (0310 §4's test, per backend) |
| 7 | The hand-written MSL ported to UKL, file by file (copy into place first), in the product path's order: tape reads, label passes, views and ids, the batch runtime, block programs, joins | each file's Metal output passes its semantic conformance tests, unchanged, before the MSL file goes |
| 8 | The remaining emitters write UKL | as 7 |
| 9 | CUDA end to end on a Spark | the data layer's 142 checks give native's findings with the GPU lane on CUDA |
| 10 | The block directory (§7.2): domains, versions, replicas, events; kernels' read and write rows drive it | on a unified device every step is a no-op (no copies counted); with a P-class policy forced on a Spark, findings unchanged and copies counted |
| 11 | Partitioned memory on a discrete GPU (§7.2, §7.3), on a cloud VM with a discrete NVIDIA card (§9, Q4) | the data layer's checks give native's findings on the discrete card; copies and their bytes reported per run |
| 12 | Several GPUs and hosts (§7.4) | two lanes on two domains give one lane's findings |
| later | ROCm (HIP), OpenCL, WebGPU (§3.4) | planned as each is taken up |

Progress (2026-11-18):
- **Step 1: done.** The preprocessor's output is token-equal to clang's for both backends; the forest builds
  without `cc`.
- **Step 2, first half: done.** All 17 random-forest kernels are written once in UKL
  (`randomforest_kernels_ukl.h`), and the hand-written Metal wrappers (`randomforest_kernels.metal`) are gone.
  - On Metal, the forest's library is built from the UKL source, and the forest's tests pass.
  - On CUDA, NVRTC compiles all 17 into one cubin: for sm_121 (GB10, CUDA 13) and for sm_70 (V100, CUDA 12.9). Registers range from 10 to 76; static shared
    memory is 0 to 32,640 bytes per kernel.
  - UKL gained the rows the kernels needed (§3.1): `ARGS` (a structure of arguments), `CONSTBUFFER`, `SHARED`
    (threadgroup storage declared in the kernel's own scope, which is the only place Metal allows it),
    `TGID0..2`, `LANE` and `NLANES`.
- **Step 2: done, through steps 3 and 4 for the forest.** On a GB10, the forest's 42 tests pass on CUDA, three runs
  out of three. They include:
  - every training kernel of a level against the host's;
  - trees and forests trained on the device against the recursive and partitioned host trainers;
  - classification and regression;
  - bagging, row decoding, column bucketizing, prediction, and armed predictions.

  The Mac's 41 pass on Metal from the same source.
  - **The device layer, for the forest** (`forest/src/device.rs`): one interface, picked by target (Metal on
    macOS, CUDA on Linux), with buffers, queues, events, libraries, functions, bind contexts and host-signalled
    events. The forest's code names only it. The argument types moved to `compute_kernel.rs`, which is neutral.
    The engine's own 48 Metal files (step 3 proper) are not moved yet.
  - **Arguments without reflection.** Beside each kernel, the CUDA shim emits:
    - the text of its rows;
    - each row's type's size, alignment and kind;
    - for each `args` structure, its fields' names (`UKL_REFLECT_STRUCT`) and a one-thread kernel that measures
      its layout. NVRTC has no constant `offsetof`.

    The host turns these into the argument list Metal's reflection gives, so the grid binds both backends the
    same way. A structure whose named fields do not tile it is refused.
  - **Memory: pinned blocks in a pool** of power-of-two classes (`cuMemHostAlloc`, E1's choice). A block goes back
    to the pool, never to the driver: `cuMemFreeHost` waits for the whole device. Under an armed prediction,
    whose stream waits for the host's signal, freeing a buffer deadlocked the host.
  - **Buffer lifetimes as Metal's.** A queue keeps each buffer its launches bind, and each one its copies and
    fills touch, until that work completes. Work dropped unawaited keeps its buffers on a pending list, swept as
    it completes.
  - **Two kernel-side fixes the port found:**
    - `extract16BitRange32` loaded two words as a `uint2` from a 4-byte-aligned address. Apple's GPUs tolerate
      the misaligned 8-byte load; CUDA faults on it. It now reads two words.
    - An empty grid launches nothing, as on Metal; CUDA calls an empty launch an invalid argument.
  - **On the V100 too (sm_70, CUDA 12.9, a discrete card):** the 42 tests pass, three runs out of three, including
    the warp sum for sm < 80 (a shuffle loop). The first run hung, and the hang is a rule for every CUDA queue:
    - The driver runs streams on a few hardware queues (8 by default), given out in turn as streams are made.
      Streams that share one run as one stream.
    - A stream waiting for the host (an armed prediction) then held back another test's stream. A pinned-to-pinned
      copy on it spun in the driver holding the driver's lock, and the thread that would signal could not take it.
    - Metal has the same rule within a command queue, which is why the forest keeps waiting work on a queue of its
      own (`shared.waiting`). On CUDA a queue root is now a set of streams made once at the start: `waiting` (1
      stream, made first) and `queue` (16, taken in turn). The driver is given 32 hardware queues, so each stream
      has one of its own. Queues of a root share its streams: their work is ordered more than it must be, never
      less.
    - The device layer moved to a crate of its own, `rerail-gpu` (`gpu/`), MLDB's files at their upstream paths
      under `gpu/src/mldb/`. The forest and the UKL libraries' tests (step 5) build on it.
  - **Not yet run:**
    - A discrete card reads the pool's pinned blocks over PCIe (12.3 GB/s): correct, but slow until step 10.
    - Separately, `rerail-accel` does not build on Linux. This predates this work: it declares `rerail-blocks`
      for macOS only but uses it everywhere.

- **Step 5: the library, done** (2026-11-18). The library is Rerail's UKL headers in
  `native/crates/ukl/lib/ukl/`. `rerail-ukl` carries them, and every kernel, hand-written or generated, finds them
  as `<ukl/NAME.h>`.
  - **Layers.** `backend.h` is the one per-backend file: qualifiers, bit counts, shuffles, ballot, match, barriers
    and atomics, a section per backend. A new backend is a new section there. Every other header is written once
    over it:

    | Header | What it holds | Level |
    |---|---|---|
    | `bits.h` | masks, fields, rank, select, bitsets (test, atomic set, next set bit) | thread |
    | `memory.h` | values at any byte address of a block, read in place: plain and coherent loads, coherent stores, bit fields, packed arrays, atomics at byte addresses (the helpers every generated tape-read file carries today, once) | thread |
    | `simd.h` | sum, min, max, inclusive and exclusive scans, votes, match, broadcast | SIMD group |
    | `group.h` | sum, exclusive scan with total | threadgroup |
    | `lz4.h` | an LZ4 block decoded by a SIMD group, output placed by the caller (the 16z piece's two parts); length, literal copy and overlapping match copy are pieces of their own | SIMD group |
    | `histogram.h` | windowed counting in threadgroup memory; the per-partition histogram kernels (u16 and u32 codes) | threadgroup, device |
    | `scan.h`, `compact.h` | exclusive scan (output `n + 1`, the total last) and compaction, written for no forward progress between threadgroups | device |
    | `radix.h` | stable LSD radix sort of u32 and u64 keys with a u32 value, an 8-bit digit a pass | device |
    | `hash.h` | hash table of u32 keys: open addressing, claimed by compare-and-swap, every probe bounded by the capacity (a full table reported); build kernel (each key's first row, by atomic minimum, and its count), probe kernel | thread, device |
    | `workqueue.h` | the persistent dispatch's work queue (0027's protocol, from `metal_work_queue.metal`): take a thread at a time, or a SIMD group at a time | thread, SIMD group |

  - **Native and reference forms.** Each operation a backend has natively keeps its reference form beside it:
    - Metal's `simd_sum`, `simd_prefix_*_sum` and so on;
    - CUDA's `__reduce_*_sync` from sm 8.0, and `__match_any_sync` from sm 7.0.

    The tests check each native form against its reference on each device.
  - **Linked by inclusion.** A kernel includes what it uses, and the backend's compiler (the Metal front end,
    NVRTC) sees one translation unit and inlines across it. A device-code link (NVRTC's relocatable code with
    nvJitLink, Metal's dynamic libraries) would compile the library once, but it blocks inlining across the call
    and costs a call per use. That cost lands in the per-element primitives (a load, a rank), which must be inline
    to match hand-written code. The kernel store (0395) caches the compiled program, so a warm process compiles
    nothing either way.
  - **The host's calls** (`rerail_gpu::library`): `scan_exclusive_u32`, `compact_u32`, `histogram`,
    `radix_sort_pairs`. Each enqueues on the caller's queue. `rerail_gpu::kernels` compiles a UKL program at run time
    for the process's device and binds a launch's arguments by name.
  - **Tests** (`gpu/src/library_tests.rs`): each primitive against its Rust reference on random inputs and edge
    sizes. LZ4 is checked against lz4-sys's blocks (the 16z compressor's) and against malformed blocks. 12 tests,
    passing on Metal (Mac), on CUDA on a GB10 (sm_121) and on a V100 (sm_70). Every header preprocesses alone as
    clang does, for Metal, sm_121 and sm_70 (`rerail-ukl`).
  - **Speed against today's Metal** (`accel`'s `library_speed`, `--ignored`): the same passes on the same input,
    the command buffers' GPU time, warmed, the median of nine. Measured on the Mac with a load average of 14, which
    touches GPU time little:

    | Operation | `accel`'s kernels | UKL library | Ratio |
    |---|---|---|---|
    | Exclusive scan, 16M u32 | 0.742 ms | 0.701 ms | 0.95× |
    | Radix sort, 4M u32 keys with rows (4 passes) | 0.928 ms | 0.854 ms | 0.92× |
    | Histogram, 4M u16 codes, 1,024 partitions, width 1,000 | 0.077 ms | 0.075 ms | 0.97× |
    | Histogram, the same, width 5,000 (3 windows) | 0.255 ms | 0.221 ms | 0.87× |

    The histogram's pair ran in `accel`'s `gpu::library_speed`, at a load average of 16, where the scan and the sort
    ran again at 0.95× and 0.93×.
  - **The work queue on CUDA: a SIMD group at a time.** 0027's protocol, a thread at a time, is correct on CUDA but
    slow: every take is a compare-and-swap on the head that every worker contends for. With 4,096 workers on a GB10,
    whose blocks are pinned host memory, an item cost 1.5 ms, linear in the items (10,000 items: 16 s; Metal's whole
    test takes under half a second). `ukl_wq_take_sg` has one lane claim up to 32 items with one swap and hand them out
    by shuffle, so there are 32 times fewer contenders and swaps; the idle test is the group's, so the worker loop
    stays uniform. 100,000 items then take well under a second on the GB10 and on the V100. The thread-at-a-time
    form stays, for workers that are few. The engine's work queue moves to the group form when it is ported (step 7).
  - **The scan of popcounts** (`accel`'s `scan_block` with `pop`): `scan_exclusive_popcount_u32`, the scan's
    `popcount` field: each word counts as its set bits, so the scan of a bitset's words gives each word's first rank.

  - **14 tests** (`gpu/src/library_tests.rs`), passing on Metal (Mac), on a GB10 (sm_121) and on a V100 (sm_70,
    two runs out of two, with the forest's 40).
  - **Not in step 5:** 64-bit hash keys. Metal has no 64-bit compare-and-swap, so they need a two-word claim, and no
    kernel asks for them yet.

    The kernels already written move onto the library in steps 6 and 7: the generated tape reads call `memory.h`,
    the 16z decoder calls `lz4.h`, and `accel`'s scan, sort and histogram are the library's calls.

- **Step 6: the reads' backend writes UKL, done** (2026-11-18).
  - **The emitter.** `clif/src/msl.rs` is `clif/src/ukl.rs`. Its structure is unchanged: structured bodies, the
    32-bit values, the switch form. What changed is what it writes:
    - UKL's types and qualifiers (`uint32_t`, `UKL_GLOBAL`, `UKL_INLINE`) in place of MSL's;
    - every load, store and atomic as the library's call (`ukl_ld_*`, `ukl_ldc_*`, `ukl_stc_*`, `ukl_atomic_*_at`,
      `ukl_fence_device`), and bit counts and minimums as `ukl_clz32`, `ukl_min` and so on. The reads' own helper
      block is gone;
    - a read marked cold (0310's policy: `tape_f16_side`, `tape_parent`, `tape_z_block`, two of the numbering's) as
      `UKL_COLD`. That is a call on CUDA (`__noinline__`) and on Metal (`noinline`; below). Every read forced
      inline (CUDA's `UKL_INLINE`) put 16z's block decode into each of a kernel's 23 calls: NVRTC ran past 500 s and
      3 GB on the conformance kernel. With the cold reads called it takes 12 s on the V100.
    - The structs the reads are over (`TapeView`, `ViewRow`, `ViewDir`, `ViewDoc`, `Run`, `Numbering`) are written
      in UKL's types too (`view::ukl`, `view_reads::ukl`, `ids::ukl`).
  - **Where the reads are held: the library.** The five generated files are `<ukl/reads/tape.h>`, `tape_z.h`,
    `relation.h`, `ids.h` and `view.h`, in `native/crates/ukl/lib/ukl/reads/`, carried by `rerail-ukl` and written
    again by `rerail-clif`'s tests (`RERAIL_WRITE_READS=1`) as the `.metal` files were. `view.h` includes `tape_z.h`
    and `ids.h`, whose reads it calls; `tape.h` and `tape_z.h` are one set of functions, and a kernel that includes
    both is refused. The reads' hooks (`CLIF_VIEW_SPACE`, `clif_view`) are in `ukl/clif.h`, which a kernel defines
    first where it needs another: the join kernels read the view in device memory, the label passes make the format
    a constant. Every library header, the reads included, preprocesses as clang does on Metal, sm_121 and sm_70.
  - **The hand-written MSL takes them by `#include`** until step 7 ports it: the join kernels, the label passes, the
    program backend, `rerail-blocks`' and `accel`'s join kernels, and the Metal tests. A source that includes
    `<ukl/...>` is preprocessed for Metal in `accel`'s one path to the front end (`gpu::kernels::msl`, which
    `library` and `ship` call). The Metal front end's own headers (`<metal_stdlib>`, `<metal_atomic>`) are empty to
    the preprocessor: the front end includes them itself. The kernel store's key for such a source adds the
    library's fingerprint (`rerail_ukl::fingerprint`), because the source names its headers, not their text, so
    changed reads would otherwise come back from stale archives. `rerail-blocks` sits below the preprocessor (through
    the C lexer and the tape crate), so its `Gpu::on` takes the preprocessed text from its caller.
  - **The gate, 0310 §4's test per backend** (`rerail-tape-format`'s
    `the_device_reads_read_every_node_as_the_crate_does`): every tape read (size, end, code, start, end, parent,
    next, next in parent, first child, child count, children 0 to 12) of every node equals the crate's own, compiled
    for the process's GPU:
    - format 16 with every side list in use, through `tape.h` and through `tape_z.h`;
    - 16z through a cache that the device's threads share, sized for the tape, through `tape_z.h`;
    - format 15, through both headers.

    It passes on Metal (Mac) and on CUDA (a V100, sm_70). The GB10s were off the network for this step. The engine's
    Metal tests of each family pass on the new headers too: the tape's (`the_device_reads_resident_tapes_...`,
    `the_device_reads_every_format_...`), the numbering's (`the_kernels_id_reads_are_the_hosts`), the relation's
    (`a_sources_rows_in_their_block_are_read_in_place`, `column_statistics_...`), the join kernels with all four
    (`a_strata_seeds_on_the_device_...`) and the label kernels (`the_label_kernels_compile_for_every_format`).
  - **Found on the way:**
    - `the_view_reads_are_the_hosts_rows` timed out cold, with the old MSL as with the new: its kernel's Metal back
      end took 412 s with the old reads and 387 s with the new, measured with no shader cache. The front end took
      under a second, so the text the back end never reaches (dead code) is not the cost; the code it reaches after
      inlining is. A kernel calling `view_node` alone took 138 s; `child` 42 s, `field` 54 s, `line` 14 s. Fixed:
      `UKL_COLD` is a call on Metal too (`__attribute__((noinline))`). The whole kernel's back end takes 2.9 s, the
      test 3.2 s, and `the_label_kernels_compile_for_every_format` 55 s (86 s before). The price, measured: a kernel
      that only walks parents (`clif_tape_parent` is cold), 2^20 threads over format 16, runs in 11.60 ms called and
      11.45 ms inlined (best of 7, twice each), 1.3%. A kernel may define `UKL_COLD` as `UKL_INLINE` first.
    - The engine's and `rerail-device-metal`'s test targets did not compile on this branch. Five calls predated
      `in_format`'s `source` argument and `DocTape::new`'s, and a glob re-export missed a `pub(super)` function.
      They are fixed.
    - `rerail-blocks` did not build on Linux (`mincore`'s dirty bits are macOS's), and so neither did the tape
      crate's tests, which CUDA's gate needs. On Linux one `msync` writes the pages back.

- **The CPU backend: done** (2026-11-18). Every UKL kernel also runs on the host, for debugging: in a debugger,
  under sanitizers, the same every time.
  - **The choice is the process's, at run time:** `RERAIL_GPU=cpu` makes `gpu::shared`'s device the CPU, otherwise
    it is the target's GPU. `gpu::device`'s types (`Device`, `Buffer`, `ComputeQueue`, `ComputeFunction`, ...) are
    each one of the two, so the forest and the grid runtime run unchanged on either. The engine's own Metal kernels
    take Metal's device through `Device::metal()`; under `RERAIL_GPU=cpu` there is none, and its lanes stay on the
    host.
  - **The shim** (`forest/src/mldb/builtin/cpu/ukl/ukl_cpu_shim.h`) makes a kernel a C++ function. A threadgroup's
    threads are stackful coroutines on one host thread, each on a stack of its own with a guard page below it. The
    context switch is about 20 instructions of assembly (aarch64, x86-64). A barrier, and every SIMD-group
    operation, suspends the thread. The scheduler releases a SIMD group when all its live lanes wait at an
    operation, and the threadgroup when all its live threads wait at a barrier. A thread that has returned no longer
    counts, as on a GPU. When no barrier can be passed (for example a SIMD group split between a shuffle and a
    barrier), the launch fails and says which thread waits where, where a GPU would hang. SIMD groups are 32 lanes,
    as the library requires. No function needs marking: a barrier is reached in any call, unlike upstream's
    stackless shim, which needed `SYNC_*` on every function holding one. The shim takes no system header.
  - **The runtime** (`gpu/src/mldb/builtin/cpu/compute_kernel_cpu.rs`) has the Metal and CUDA runtimes' interface:
    - `c++ -std=c++20 -O1 -g` (or `RERAIL_CPU_CXX`) compiles the preprocessed text into a shared object, which is
      loaded. It is kept by the text's hash in the temporary directory, beside its debug information;
    - buffers are host memory;
    - a queue's root is a thread that runs what its queues flush, in order;
    - arguments are read from the rows, as on CUDA. Their parsing is now one module for both (`gpu/src/rows.rs`).
  - **`rerail-ukl`** has `Backend::Cpu`. Its predefines are the host's architecture and system, and the include
    search tries the backend's shim directory before UKL's own: upstream's CPU shim has the same file name.
    `backend.h` has a CPU section, and every library header preprocesses for it as clang does.
  - **Tests, on the Mac (M5 Max):**
    - the library's 14 tests (`rerail-gpu`, `RERAIL_GPU=cpu`) pass in 9.5 s (0.37 s on Metal);
    - two tests of the shim run whatever `RERAIL_GPU` is: a threadgroup with threadgroup memory, a barrier and
      shuffles gives each thread what a GPU gives it, and a split SIMD group is an error that names its waits;
    - the generated reads' gate (`the_device_reads_read_every_node_as_the_crate_does`) passes. `tape.h` compiles
      in 0.3 s and `tape_z.h` in 1.5 s;
    - all 40 of the forest's tests pass with the trainer on the CPU (254 s; 3 s on Metal).
    - On Linux x86-64 (the V100 VM), the same: `rerail-gpu`'s 16 tests and `rerail-ukl`'s 7 pass on CUDA, the 16
      pass on the CPU (15 s; the x86-64 switch and Linux's symbols), and the forest's kernels compile and reflect on
      both. The device types' CUDA side builds and runs unchanged.

    Two changes were needed in the forest's kernels' environment, none in the kernels themselves. The shim gives
    the GPU languages' built-in `min`, `max`, `fma` and `sqrt`. It also undefines `UKL_CPU_BACKEND` once the
    backend is chosen, because the kernels' `#if UKL_CPU_BACKEND` blocks are upstream's runtime's debugging
    (iostreams, MLDB's printers).
  - **Found on the way:** two compiles of one text in one process raced on a temporary name. Each compile now has
    its own.

- **Before step 7: the strata branches to port (a survey, 2026-11-18; nothing in them changed).** Two active
  branches change the MSL that step 7 ports, and the emitter that step 6 rewrote:
  - **Native and GPU strata** (142 commits past this branch's base, e5d076eb0). It changes `clif/src/msl.rs`,
    `reads.rs`, `relation.rs`, `view.rs`, `view_reads.rs` and `rust.rs`. Its generated reads are:
    - block-major format 16 (0399);
    - 16z read through a cache (0353 steps 4a and 4b), which removes `decode16z.metal`;
    - reads proven to stay within the tape's bytes (0403).

    It also changes the label passes (`metal_label_passes.metal`, +211 lines), the relation reads, and the blocks'
    `atoms.rs` and `kept.rs`.
  - **oscar** (9 commits past Native and GPU strata). It adds the CLIF tape writer:
    - `clif/src/writer.rs` (3.2 K lines) and its generated Metal (`writer.metal`, 6.0 K lines);
    - `f16/ops.rs`, `reference.rs` and `generated_writer.rs`;
    - more changes to `msl.rs` (+117 lines) and `reads.rs` (+1.4 K lines).
  - **What porting needs**, at the merge:
    - their `msl.rs` changes carried into `ukl.rs`, as UKL's types and the library's loads;
    - their reads and the writer generated again, as `<ukl/reads/*.h>` and a `<ukl/writer.h>`;
    - their label-pass changes ported with step 7's file;
    - their tests run on Metal, CUDA and the CPU: 0310 §4's gate, and the writer's own.

    The writer is the first generated code that stores into a tape. The library has its stores (`ukl_st*`), so
    nothing new is needed in UKL for it.

Steps 1, 2 and E1 are small and settle the bets the rest rests on: the preprocessor, the shim, and in-place
bandwidth. Step 3 can run beside them.

### The next set of work (agreed 2026-11-19)

Steps 1, 2, 5 and 6, the CPU backend and OpenCL are done. What is left moves the engine's own GPU code onto UKL
and the device layer, then runs it on CUDA. In the tree at the start: about 2,700 lines of hand-written Metal in the
product path (the label passes, 1,077 of them, with the sweep, set steps, nearest and lattice; the batch runtime,
work queue and parallel files, 500; the block programs' carry, program and sinks, 480; the tape reads and the 16z
decoder, 220; `accel`'s join kernels, 315), and about 50 Rust files that name Metal, several of which write Metal
source (the label patterns, the program backend, `metal_backend`, `block_program.rs`, `accel`'s join and tile).

| # | Work | Done when |
|---|---|---|
| A | Merge `Native-and-GPU-strata` at a pinned commit; carry its emitter changes into `ukl.rs`; its reads (block-major format 16, 16z through a cache, reads proven within the tape) generated again as `<ukl/reads/*.h>`, its writer as `<ukl/writer.h>` | the reads' gate on Metal, the CPU, OpenCL and CUDA (a GB10 and the V100); the branch's own tests on Metal |
| B | Step 3 proper: the engine's GPU lane on `rerail-gpu`'s device types, not Metal's | no behaviour change on Metal: `rerail-device-metal`'s tests and each family's |
| C | Step 7: the hand-written Metal to UKL, file by file, copied into place first, in the product path's order: tape reads, label passes, views and ids, the batch runtime with the work queue (to its SIMD-group form), block programs, joins | each file's Metal tests pass; then the file runs on the CPU (with OpenCL's SIMD-group rule checked) and on OpenCL |
| D | Step 8: the generators write UKL (label patterns, program backend, block programs, `accel`'s joins); `accel`'s scan, sort and histogram are the library's calls | as C |
| E | Step 9: `rerail-accel` builds on Linux; the data layer's 142 checks with the GPU lane on CUDA | native's findings, on a GB10 (speed judged there) and on the V100 (correctness only: it reads pinned host memory over PCIe until step 10) |

**A: done (2026-11-19, merge 3f701ec4e).** `Native-and-GPU-strata` merged at ef9c3dcf2. Its emitter changes (each
read's 16z body called apart, 64-bit offsets for every family but the tape's) are in `ukl.rs`, and its reads
(block-major format 16, 0434; 16z through a cache, 0353 step 4b; reads within the tape's bytes, 0463) are the
library's `<ukl/reads/*.h>`, written again from the merged emitter. A read is cold where 0310's policy says so and
for every 16z body called apart. This record was 0400 there; the branch's 0400 (a small root checked by its
listings) kept the number. The writer (`<ukl/writer.h>`) waits for oscar, which is not on the branch at the pin.
- The reads' gate passes on Metal, the CPU and Apple's OpenCL (Mac), and on CUDA on a GB10 (sm_121), with the
  forest's 40 and `rerail-gpu`'s 17 there; the branch's own tests pass on Metal (the tape crate's 26, the blocks'
  33, accel's 85, `rerail-device-metal`'s 57, the engine's slab, view, id, resident-tape and label-kernel tests).
- Found on the way: NVIDIA's OpenCL on the GB10 says it shares the host's memory (`CL_DEVICE_HOST_UNIFIED_MEMORY`)
  but keeps a copy of a `CL_MEM_USE_HOST_PTR` buffer, made visible only by a map: the reflection kernels' words
  read 0. The runtime now runs a one-thread write into host pages when it opens a device, and refuses one whose
  write is not there (0353). OpenCL on NVIDIA needs SVM, which is not written. Three of the branch's tests failed
  under parallel runs and were fixed in the merge (the merge commit says how): the writer's tail outgrew its room
  (0434 R4), and two tests read process-wide counts that the tests beside them move.

**B: the device layer's part (2026-11-19).** The engine's Metal uses five things the device layer lacked. Each is a
primitive of it now, on every backend, with a test (`gpu/src/device_tests.rs`) that passes on Metal, the CPU and
Apple's OpenCL, and on a GB10's CUDA and its CPU (aarch64 Linux):
- **Host memory in place** (`wrap_host_memory`; the engine's seven no-copy wraps): Metal's no-copy buffer and
  OpenCL's `CL_MEM_USE_HOST_PTR`, each over page-aligned memory only; on CUDA the host pointer itself when the
  device reads pageable memory through the host's page tables (the GB10), else the pages registered and their
  device address. The CPU's buffer is the memory. Memory a backend cannot take is an error, never a copy (0353).
- **Device addresses** (`Buffer::device_address`; the view directories' tape pointers) and **residency**
  (`uses`: Metal's `useResource`; elsewhere the buffer is kept to the launch's end). OpenCL 1.2 has no pointer
  in a buffer: `None` there, until SVM. Metal's reflection now takes a structure's pointer field as its address.
- **Indirect launch** (`run_indirect`; seven sites): Metal's indirect dispatch; on the CPU the counts are read
  when the launch runs. CUDA and OpenCL have none: the shims give each kernel a hidden last parameter, the
  counts' address (null in a direct launch). The host launches the caller's bound of groups, the groups past the
  counts return first, and `GSZ` reads the counts' grid, so the kernel sees what Metal gives it. No host read of
  the counts, no stall in the queue.
- **Specialization** (`program_with`; the label passes' function constant): a define a value, each value a
  program compiled once. A define is portable where a function constant is Metal's alone.
- **A bind at an offset** (`buffer_at`; most of the engine's binds are a block's allocation): Metal's offset,
  the address plus the offset on CUDA and the CPU, and a sub-buffer on OpenCL (at its base alignment, else an
  error).

**B: the blocks and the pool on the device layer (2026-11-19).** The device layer is a crate of its own now,
`rerail-device` (`gpu/device`: the backends' runtimes at their MLDB paths, `device`, the shared device), below the
preprocessor; `rerail-gpu` re-exports it and keeps the preprocessing (`Preprocessing`), the kernels and the
library. The blocks could not take the device layer's types while it sat above them (`rerail-gpu` reaches the
preprocessor, whose lexer reaches the tape crate, then CLIF, then the blocks).
- A block's device buffer is the device layer's, on every backend (`Block::device_buffer`, `Backing::buffer`, a
  deferrable's wrap): its pages wrapped in place, made once and kept. `gpu_buffer` gives it as Metal's to the
  kernels step C has not moved yet. A test (`a_blocks_device_buffer_is_its_pages`) has a kernel write a block's
  allocation through it, and the host read the block where it lies, on Metal, the CPU, OpenCL and CUDA.
- The pool (`accel::gpu::shared`) leases the device layer's buffers, made by it; `Shared::layer` is the process's
  device in it. The Metal accessors stay for the kernels not yet moved.
- Found on the way: on Linux, a blocks test read the process-wide count of zeroed bytes, which its neighbours
  move; it reads its thread's count now (`zeroed_bytes_here`). The merge left `Tape16Z::pieces` unused: removed.
- **The kernel store** (`rerail_device::store`, 0395's on the device layer): each backend's compiled form on
  disk, under a key of everything its compile reads, so the next process loads it. CUDA keeps NVRTC's cubin
  (keyed by the architecture, the options, NVRTC's and the driver's versions), OpenCL the program's binary (the
  device's name, version and driver), the CPU the compiler's library (its `--version`: the old key named the
  compiler but not its build, so an upgraded one loaded stale libraries; the libraries were in the temporary
  directory). An entry that does not load is compiled again and replaced. The store keeps within
  `RERAIL_GPU_KERNELS_MB` (2 GB), least recently used first, pruned by upkeep beside the Metal archives and once a
  process. Metal's pipelines stay in the archive store until step C moves the engine's kernels. Tested
  (`a_kernel_compiled_once_is_loaded_from_the_store`) on the CPU, OpenCL and a GB10's CUDA.

B is done: what the engine's GPU lane needs is in the device layer, on every backend; the blocks and the pool hold
its buffers. The engine's call sites stay on Metal's types until each moves with its kernel (C).

**C: the tape's views in UKL (2026-11-19).** `metal_tape.metal`, the views every tape kernel reads documents
through (a node's code, kind, parent, siblings, lines; a pass's roots; a SIMD group's roots), is UKL now:
`device_tape.h`. Its types are UKL's, `tape_code` gives a pointer (OpenCL C has no references), the SIMD group's
helpers take the lane and call `ukl_sg_*`, and its constants are defines. Its kernels (the label passes, the sweep,
the program backend) include it as before, preprocessed for Metal.
- Its test (`the_device_reads_resident_tapes_as_the_packed_batch_records_them`) is a UKL kernel launched through
  the device layer by its rows' names, binding the batch's slab by its device-layer buffer (`Slab::device_buffer`;
  a slab holds the device layer's buffer now, Metal's for the kernels not yet moved). It runs on the process's
  device and checks every node of a batch against the tape's own answers; its source is compiled for the CPU and
  OpenCL too. OpenCL found a parameter named `local`, an address space there: renamed.
- The residency runs on Metal alone still: its slabs lease from the pool, whose `get` is Metal's while the engine's
  kernels are, and `rerail-accel`'s GPU lane builds on macOS alone (step E). The kernels move next, in the plan's
  order: the label passes, the sweep, then the batch runtime.
- Found on the way: `rerail-store`'s tests did not compile after a merge, and the journal's replay ignored the
  caller's bound (Q184) for the settle window's wait (Q179). Both fixed. Four observer tests that fail with or
  without the fix are Q250.

**C: the engine's lane on the device layer, and the first family on it (2026-11-19).** What every ported family
stands on:
- **The pool on any device** (`accel::gpu::shared::layer`): the process's device in the device layer, whatever its
  backend, and its queue (on Metal the same as `get`'s). The pool leases from it and the blocks wrap for it, so the
  engine's blocks are leased on the CPU and OpenCL too. `get` stays Metal's, for the kernels not yet moved.
- **Kernels from the engine's store as the device layer's** (`accel::gpu::kernels::function`, `function_fixed`,
  `ready_function`): on Metal the store's pipelines, from its shipped and stored binary archives as before, now
  made with their arguments' reflection (`MetalComputeFunction::from_reflection`), so a launch binds them by name;
  elsewhere the device layer's compile and store, the constant given as a define (`UKL_CONSTANT_0`). The device
  layer's programs are now kept by their text's digest too: two sources of one name were one program.
- **The lane** (`rerail_metal::lane`): a command of launches on the device layer, each charged to the evaluation's
  launch budget and recorded as Metal's dispatches are (`launch::attempt`, `mark`), bound by argument names; a
  command past the budget runs none of its launches, as Metal's rule says, on every backend (its launches are held
  until the commit, since an OpenCL or CPU queue starts a launch when it is enqueued). Its times are the device's
  and, on Metal, the command buffer's host-clock times. The launch guard's source check counts it with
  `launch.rs` as a checked layer.
- **Nearest ancestors** (`nearest.uglykl`, was `metal_nearest.metal`) is the first family on it: the kernel in UKL, its
  host on the lane, in fresh device-layer buffers or the pool's blocks. A new conformance test
  (`the_device_finds_each_cells_nearest_ancestor_in_its_set`: 5,000 nodes, two sets, both kinds of buffer, against
  one pass in preorder) passes on Metal, the CPU and OpenCL (`RERAIL_GPU=cpu`, `=opencl`); its only test before was
  an ignored measurement.
- Found on the way: a pool lease's length and host address went through its Metal buffer (fixed: the device
  layer's); an accel test compared a process-wide live count exactly, which its neighbours move (it counts its own
  thread's deferrable leases now); the device layer gained a device's largest buffer (OpenCL's
  `CL_DEVICE_MAX_MEM_ALLOC_SIZE`).

**C: the lattice, the set steps, and the scans retired (2026-11-19).**
- **The lattice fixpoint** (`lattice.uglykl`, was `metal_lattice.metal`) runs on the device work queue of the
  library (`<ukl/workqueue.h>`), its coherent 64-bit words through `<ukl/memory.h>`; its host and the queue's
  (`metal_work_queue.rs`) are on the lane. Metal's compare-and-swap may fail without a rival (a weak exchange): the
  claims and the publish retry until the word is seen to differ. `ukl_atomic_and` is a primitive on every backend
  now. A conformance test (`the_device_reaches_the_lattices_least_fixpoint`, against the host's fixpoint by rounds)
  passes on Metal, the CPU and OpenCL.
- **The set steps** (`set_steps.uglykl`, was `metal_set_steps.metal`): a threadgroup a file, its lanes over the
  steps, the threadgroup's barriers `ukl_tg_barrier` and `ukl_tg_barrier_device`. Its test against the CPU's sets
  passes on Metal, the CPU and OpenCL.
- **The saturating scans** (`metal_parallel.metal`) had no caller since the grid machine went (0324): removed, with
  their tests. Their one live helper, a checked dispatch, moved to `metal_rows`, its only user (it goes with the
  row programs' generator in D).
- Kernel files are `.uglykl` (the user's choice): easy to find and tell from Metal's.

**C: the label passes in UKL (2026-11-19).**
- **The eight label kernels** (`label_passes.uglykl`, was `metal_label_passes.metal`; the device work queue's Metal
  copy, `metal_work_queue.metal`, went with it) are UKL, each a `DEFINE_KERNEL2` whose rows keep the old slots'
  order. They compile on Metal, the CPU and OpenCL (`gpu/examples/ukl_check.rs`, a compile check of a source on every
  backend this machine has). The tape format is a function constant on Metal and a define elsewhere
  (`UKL_CONSTANT_U32`).
- **Still on the Metal runner.** The runner binds the arguments its positional buffers do not reach (the block, the
  walk statistics, the spill queue) where the kernel's reflection puts them (`LabelKernel`), and a kernel without
  such a row gets none (`label_round` reads no document: binding the block at a missing slot crashed Metal's
  encoder). The pattern kernels stay Metal until D and call the shared `simd_roots` with their lane.
- The engine's Metal tests of the label passes, the sweep, the resident interpreter and the tape reads pass on the
  M3 (66; two others need the packs' test data, which the host's copy lacked).
- **Next in C:** the batch runtime and the sweep (with the JIT's generated text), then the runner and the program
  backend on the lane, so the label passes run on the CPU and OpenCL too.

**C: the batch runtime, the sweep and the JIT's text in UKL (2026-11-19).**
- **The runtime** (`batch_runtime.uglykl`, was `metal_batch_runtime.metal`) is C: a node's reads are functions of the
  node (`Node_kind`, …) where they were methods, the tables' lookups functions (`node_at`, `line_of`, `file_at`), and
  every reference a pointer, its uses through it. `run_batch` is a kernel of 31 rows in the old slots' order. The
  compare-and-swap loops retry on a weak failure, as the lattice's. C has no braced value outside a declaration:
  each is a declared local.
- **The sweep** (`label_sweep.uglykl`) is a kernel of its own on the same rows. It took `run_batch`'s parameter
  text spliced in, which rows have no use for.
- **The JIT's text** (`jit_support`) is C: a step function for each opcode the programs use (a template's
  instantiation chose them before), and the context passed by pointer. A step works on copies of the task's
  state under the names the opcode bodies use, and writes them back when it finishes. A step that stops the task
  writes nothing back, as the interpreter's `return` ends the task there.
- **64 rows a kernel.** The macros (`ukl_auto_macro_gen.py`, six levels of commas, was five) took 32 arguments,
  and `run_batch` on OpenCL and CUDA has 33 (its buffers, its thread and the indirect launch's row).
- Both forms (packed and resident) and the sweep compile on Metal, the CPU and OpenCL. On the Metal runner, the
  engine's Metal tests of the program backend, the semantic conformance (the interpreter and the JIT), the label
  passes, the sweep and the resident interpreter pass on the M3 (149), and the device layer's, UKL's and the
  forest's (125).
- **Next in C:** the runner and the program backend on the lane, so these run on the CPU and OpenCL.

**C: the runner and the program backend on the lane (2026-11-19).**
- **A command is owned** (`lane::Command`): its steps (launches, copies, fills) in order, each holding its kernel
  and buffers, so the runner builds one across a level's calls and runs it at the flush. The blits are its copies
  and fills.
- **Rows bound by number** (`Launch::slot`, `slot_value`) where a caller binds a kernel's rows in order, as the
  runner's positional buffers do; by name elsewhere. A label kernel's block, walk statistics and spill queue are
  found by their rows' names on every backend (`LabelKernel::of`), the pattern kernels' too.
- **Exact grids** (`Launch::threads_exact`) for `run_batch` and the sweep, which read their lane by their thread
  and do not bound it: Metal's non-uniform grid ran exactly the lanes, and a level's lanes need not be a power of
  two. The other label kernels bound their own threads, or stride by the grid.
- **Host-mapped buffers on every backend** (`runtime::HostMapped`): an anonymous mapping, wrapped by the device
  layer, in the workspace, the programs' buffers and every runner buffer. The mapping is the holder's, not the
  buffer's: a buffer bound to a launch is held (`Runner::retained`, the prefix as an `Arc`) until its command has
  run. Two holders let theirs go early: the labels read after the runner had gone (a crash on the M3), and a
  formula's statistics when profiling is off. Both are held now.
- **The 16z cache on the device layer** (`slabs::zcache_for`): made on the process's device and placed by device
  addresses. OpenCL 1.2's kernels follow no address a buffer holds, so a 16z batch has no cache there and is refused;
  the other formats need no address.
- The pattern kernels are still Metal's text (their generator is step D's); the device layer takes them as written
  on Metal.
- The engine's Metal tests of the program backend, the semantic conformance, the label passes, the sweep, the
  resident interpreter, the slabs and the JIT pass on the M3 (181).

**C: the pattern kernels, the slabs and their test on every backend (2026-11-19).**
- **The pattern kernels are UKL** (`metal_label_patterns.rs`): the generator writes C, its functions taking a
  pattern's bindings by pointer, its goal tables and text pool as `UKL_TABLE` arrays. It was step D's, and is moved
  into C: every clause with a pattern or a fact runs it, so without it the label passes ran on the CPU for 23 of
  the resident interpreter test's 122 clauses.
- **`UKL_TABLE`, `UKL_TABLE_SPACE`** (UKL's `backend.h`): a table at file scope and a pointer into one: Metal's and
  OpenCL's constant space, CUDA's `__constant__` (a pointer into it is generic), the CPU's `static const`.
- **A slab is leased on the process's device** (`slabs::Slab::lease`), not Metal's: under `RERAIL_GPU=cpu` every
  admission was refused as over capacity. The slabs' test of every format runs its kernel in UKL through the lane.
- **A settle skips for an earlier kernel's decision, not its own launch's** (`label_settle`): a recursive group's
  last settle has thread 0 decide that the round before reached the fixpoint, and every settle returned at once
  when the flag was set. The flag landed while other threadgroups of the same launch had not started: never on
  Metal's or OpenCL's GPU, every time on the CPU, which runs the groups in turn, so their scratch stayed unzeroed
  and 10 of the 122 clauses lost roots there (a SIMD width of 1 changed nothing: not the lanes). The flag now holds
  the deciding round plus one, and a settle of that round goes on.
- **OpenCL 1.2's limits, kept in the kernels' interface:** a buffer region is bound only at a multiple of 4,096
  bytes, so the prefix's parts are aligned to it and the labels are bound at 0, behind the two head words
  (`label_settle`'s `head`, the pattern kernel's `head + 2`); `local` is a keyword there, so no parameter has
  the name; and a buffer has no kernels' address (`Device::has_addresses`), so a 16z batch, whose cache each view
  points at, is refused there, as the slabs' tests now check.
- **A SIMD group's rounds step by `UKL_SIMD_WIDTH`**, not 32 (`micro_run`, `label_group_run`, the pattern kernel).
- On Metal, the CPU and OpenCL, the 225 targeted tests pass except: the two reading pack data this checkout lacks
  (`pyflakes/F601.py`); and, on the CPU and OpenCL, the block programs' test and the anchor scan's, both still
  Metal (C's remaining files). On OpenCL two retained-grid tests take 115 and 151 s alone (Apple's OpenCL
  compiler) and time out at 240 s in the full set.

**C: the anchor scan, in UKL and in place; OpenCL's fills (2026-11-19).**
- **The anchor scan reads the tapes where they are** (`metal_backend::prepare_anchor_scan`): it had copied each
  file's kinds, spans and source into buffers of its own, a copy of the tapes from before the check (decision 0353,
  which reads only added lines). Now the files are resident in one block (`DocTable`), and the kernel, in UKL
  through the lane, finds each node's document among the views (`doc_index`, `device_tape.h`, with `doc_of`,
  moved there from the label passes) and reads its kind, span and text through the tape view. Hits are pairs in a
  buffer; a hit's file and ordinal come from the views, so no per-node table is made.
- **The tape view is written once** for the label passes and the anchor scan (`metal_program_backend::tape_view`,
  in both crates that build the passes).
- **OpenCL's fills are a kernel** (`ClComputeQueue::enqueue_fill_array`): Apple's `clEnqueueFillBuffer`, on a
  buffer over host pages, left garbage that the next kernel read (the scan's hit count, 1,103,694,145 for 25
  nodes; `a_fill_then_a_kernel_are_in_the_host_pages`, which failed about half its runs). A plain OpenCL C kernel
  (the device layer is below the UKL preprocessor), compiled at a queue root's first fill, writes the pattern's
  words; a pattern of 1 or 2 bytes is widened to a word, and one of no whole words is written from the host.
- On Metal, the CPU and OpenCL, the 409 targeted tests (the anchor tests with them) pass but for the pack-data two
  and, on the CPU and OpenCL, the block programs' test. Three slow tests time out at 240 s at a load average of 55
  to 74 and pass alone.

**C: accel's gpu lane on every backend (2026-11-19).**
- **What C still holds**, counted: the block programs (`block_program.rs` and its three `.metal` files,
  `block_serial.rs`), the joins (the engine's `metal_join.rs` and `metal_rows.rs`, accel's `join.rs` and `tile.rs`),
  and accel's lane, below.
- **accel's lane** (`accel/src/gpu/mod.rs`): the bit product, the count table and the closure are UKL
  (`relational.uglykl`); its scans and histogram are the UKL library's (`rerail_gpu::library`), whose histogram
  step 5 measured no slower than the lane's (so its speed gate goes with the lane's own `hist`); the zero fill is
  the queue's. A submission is enqueued on the device layer's queue, stage by stage, as it was encoded with a
  barrier between stages; a page-aligned column is wrapped in place (`wrap_host_memory`). So the lane runs on the
  process's device whatever its backend; programs and joins on it stay Metal's (`can`) until they are UKL.
- Its 23 tests pass on Metal; on the CPU and OpenCL all but accel's two column-statistics tests, which run
  `join.rs`'s kernels.

**C: the joins and the row programs on every backend (2026-11-19).**
- **The joins' stages are UKL** (`packed/join_stages.uglykl`): the nested loops' count and scatter, the witnesses,
  the texts made per row and their tests, the replacements, the text functions, the copies, the head's distinct
  rows, the device index and the lattices. The views' expansion and the kind-set axes are `chunk_views.uglykl`, a
  batch's root layout written as C by `blocks`' `structure!` (`typedef struct`, checked by static asserts of its
  size and offsets), as its MSL was. The row programs' kernels (`metal_rows.rs`) and accel's join kernels
  (`join.uglykl`, `join_sort.uglykl`) are UKL too; the MSL is gone.
- **A stage's constants are a buffer**, not arguments: a `Stage` is 2,320 bytes, with arrays and an address, and
  OpenCL takes arguments by value up to 1 KB, of scalars alone. So every stage, copy, dedup and index kernel reads
  its parameter block as its last buffer (`Batch::block`); the small scalar ones (`LatP`, the text kernels') stay
  arguments.
- **One pass on the device layer** (`join::Pass`): the batch's launches go on one queue of the process's device and
  are committed once, as Metal's command buffer was. A SIMD-group kernel (a row's text, a term's verdict) runs
  threadgroups of one SIMD group (`Pass::run_bytes_simd`), OpenCL 1.2's rule for kernels whose lanes loop by the
  data, and its scratch is sized by the device's width.
- **The views read in place are Metal's alone**: their directory holds GPU addresses, which OpenCL 1.2's kernels
  cannot follow. Elsewhere a view is made by `view_expand` (the listed copy, 0353 step 4), as a refused batch's is;
  `views_made_on_the_device_are_the_hosts_rows` checks both ways.
- **Widths below 32 found two overruns**, both from before the port: a threadgroup's prefix kept a word per SIMD
  group in 32 words (`join_tg_prefix`, and `relational.uglykl`'s maximum), 256 at a width of 1; and the radix sort
  kept its counts per SIMD group, 8 × 256 words, which needs 32 × 256 at a width of 8 and 256 × 256 at 1. The
  prefix's words are now `JOIN_SGS`. The sort ranks a row among a group of at least 32 threads: a SIMD group of 32
  lanes or more, through shuffles, as before; else 32 threads, through their digits in threadgroup memory. So its
  counts stay 8 × 256 words at every width. At a width of 1 the head's distinct rows had kept one text of each name
  (`parity_row_texts_past_the_arena`), and the set fixpoint's run died of a bus error.
- `ukl_mulhi64` (UKL's `backend.h`): a 64-bit product's high half on each backend, for the stages' arithmetic.
- On Metal, the CPU and OpenCL: the joins', row programs' and set fixpoint's device tests (37: the parity suites,
  text functions and tests, witnesses, waves, lattices, views) and accel's 23 lane tests pass, its two column
  statistics tests among them. At SIMD widths of 1 (OpenCL) and 8 (the CPU) the text tests and accel's pass too.
- Left in C: the block programs (`block_program.rs` and its three `.metal` files, `block_serial.rs`), `launch.rs`'s
  kernels and the tests with Metal kernels of their own. accel's `can` still keeps programs on Metal.

**C: the block programs in UKL (2026-11-19).**
- **The interpreter and its helpers are UKL** (`block_common.uglykl`, `block_program.uglykl`; were
  `block_carry.metal`, `block_sinks.metal`, `block_program.metal`): one compile per lane width (`BW` 16, 32, 64,
  `BM` its word), the carries' look-back and halo exchange, the sinks. The generated forms (`generate_forms`, one
  function a program and one kernel a batch) and the passes' kernels (`block_serial.rs`: the serial and lanes
  runs, the cones, the checks, counts, prefixes, writes, the records and the back fill) write UKL too. Every
  launch goes through the lane (`lane::Command`), binding its buffers and parameters by row name; a kernel takes the
  rows it declares (`buffer_if`, `value_if`), as Metal's took the indices they declared. The device zeroes a run's
  large blocks by the queue's fill (`pool::zeroed_on_device`), in a launch scope of its own.
- **A block program needs SIMD groups of 32** (`block_program::ready`): a lane's word and the group's 32 words are
  the format. A device with other widths is refused (`UnsupportedBackend`), and the executors skip it.
- **One SIMD group a threadgroup for the lanes kernels**, whose groups are segments: OpenCL 1.2's rule, as the
  joins' SIMD-group kernels (above). The serial kernels choose their ASCII body per thread, where a group's vote
  (`simd_all`) had chosen it for all: a vote is reached by every thread or none.
- **No SIMD-group operation in an operand of `SEL`**, a C conditional (Metal's `select` evaluated both): a lane that
  skips a shuffle leaves the lanes reading it with an inactive lane's value. The 64-byte lanes' `prev` read 0 at
  each group's first lane on Metal (`halo_before`'s `SEL(sg_up64(..), P, l == 0)`). Each shuffle is a statement of
  its own now, in the helpers and in what the generators write; the scans' two prefixes under a reset too.
- **A worker's lanes need not run in lockstep** (the CPU's lanes are threads; CUDA schedules each thread on its
  own): lane 0 takes a group's ticket and reads whether the group is done, and the group takes both from it in
  one shuffle. Each lane had read `done` itself, so on the CPU lane 0 finished a group, marked it done, and the
  other lanes, reading it after, skipped it: only lane 0's words were written. Metal's lockstep had hidden it.
- **No thread-local in a CPU kernel library** (`ukl_cpu_shim.h`): the running scheduler and thread are in one
  thread-local of the host's, which each launch names, and the scheduler's storage is the queue thread's. macOS
  gives each loaded image's thread-locals a pthread key of their own; the block suites load a library per generated
  program, and after a few hundred the process ran out of keys and aborted at a thread-local's first use
  (`_tlv_bootstrap_error`).
- **Lane code is straight-line**, as before: the helpers that loop are named in the test's list (`LOOPING`) where
  the MSL had marked them by leaving out `inline`, and the helpers the port had given a loop or a `?` (`rotr64`,
  `clamp_i32`, `keep_word`, the prefix XOR, `texclusive1`) are unrolled or selects.
- **A fixed point's vote is the lanes form's** (`emit`'s `Word::lanes`): in the serial form each thread runs its own
  segment, so its rounds end when its own carries settle; a vote there (`simd_any`, kept from the MSL) was a
  SIMD-group operation in a function without the group's parameters, which OpenCL's emulation refused.
- The block suites' 45 tests pass on Metal, on the CPU under OpenCL's rule (`RERAIL_CPU_OPENCL_RULE=1`) and on
  OpenCL, and `rr_parser`'s 48 on Metal. Found on the way: `rr_parser`'s refusal test still took a setting's `1.2.3` as refused,
  which 0400 had made a reading; its case is an integer of 19 digits now, which the record pass refuses.

**C: the tape tests' kernels in UKL (2026-11-19).** The tape tests' diagnostics (O39: the footprint of a submission,
of a kernel's private stack, of one label kernel's dispatch) launch UKL kernels through the lane on the process's
device; only the measure of new command queues is Metal's, since a device layer's queue is its device's. A label
kernel's rows are bound from its reflection, each buffer the zero buffer and each value zero bytes of its type's
width (`lane::Launch::bytes`). The tests gate on the process's device, not Metal's. The probe of the label
kernels' compile (Q181) stays a probe of Metal's compiler: its function constants and pipelines are what it times.

**C: the numbering's and the views' probes in UKL (2026-11-19).** The two tests that check the reads written once
in CLIF against the host's (`the_kernels_id_reads_are_the_hosts`, `the_view_reads_are_the_hosts_rows`,
`facts/vtab/syntax/tests.rs`) launch UKL kernels on the process's device, each reading its projection's block where
it lies (`Block::device_buffer`). The views' probe reaches the tape and the lines by their device addresses, words
of its parameters, as the engine's views do; it runs where buffers have them (not OpenCL 1.2). Both pass on Metal,
the CPU and OpenCL. Seen on the way: a 16z tape's in-place view reports format 16 there, so the probe's 16z cache
branch is not reached; it was not before either.

**C: the views' directory on the process's device (2026-11-19).** A tape's slab room, the views' directory and its
16z cache are the device layer's (`tape::placed`, `device_chunk::build_views`): the slabs are the process's device's,
whatever its backend, and a tape is read in place where that device's buffers have addresses. So the CPU reads the
`node`, `child`, `field`, `core_node` and `line` views in place as Metal does (`views_made_on_the_device_are_the_hosts_rows`:
10 of 10 on both), and OpenCL 1.2 makes them. The vtab's gate on Metal is gone.

**C: the blocks crate's kernels in UKL (2026-11-19).** The join's kernels (`rerail_blocks::gpu`) are UKL, their
structures declared by `ukl_of` (the field trait is `KernelField` now, its MSL half gone: nothing read it). The crate
is below the device layer, so it holds the source and its callers launch it: the `paces` lab (its join and overflow
rerun, 126 checks on Metal, the CPU and OpenCL) and the tests (`the_device_reads_a_relation_as_the_host_does`,
`a_kernels_writes_are_written_back`), through `rerail-gpu` as a dev-dependency.

- **The pool's hooks are installed by the device layer's entry too** (`POOL_HOOKS`): the ledger's reservations and
  the residency's evictor were installed only by `METAL_DEVICE`'s first touch, which a process on the device layer
  alone need never make.
- Gone with their last users: `Slab::buffer` (Metal's), `MappedBuffer::slab`. The pool's, the slabs' and accel's
  block tests read the device layer's buffers and addresses, and gate on its device: accel's two block tests had
  gated on Metal's `Shared`, and failed under `RERAIL_GPU=cpu`, whose pool leases slabs.
- **A race fixed:** `a_wrap_of_host_memory_leaves_the_metal_budget_unchanged` compared the pool's whole idle count
  across a trim while other tests lease and return small buffers from the same pool; it checks its own 64 MB.

**C: programs on the GPU lane of every device (2026-11-19).** accel's GPU lane takes a program wherever the program
offers that lane (`can`): the gate on Metal's `Shared` is gone, since the programs that offer it (the span prefilter,
the matcher wave) run their kernels through the lane on the process's device. The span prefilter's and the wave's
tests pass on Metal, the CPU and OpenCL, and accel's 85 on all three (the CPU's in 399 s cold: its compiles).

**C: the blocks crate's labs in UKL (2026-11-19).** `pinning`, `deferrable` and `encodings` launch UKL kernels on the
process's device. `encodings`' two function constants are defines (`BITS`, `PB`: a program each), its kernels'
`simd_is_first` lane 0, its 1,024-thread scan a threadgroup of 256; every check against the CPU's encoder holds word
for word on Metal and OpenCL at full size and on the CPU at 1/64 (`ENCODINGS_GPU_SHIFT`, for a slow device).
`deferrable`'s `DEALLOC` and `POOL` switches stay Metal's: they ask about Metal's own wraps (a deallocator, an
autorelease pool), so the crate's `metal` is a macOS dev-dependency now, not a dependency. `Block::gpu_buffer` went
with its last caller; `release_gpu_buffer` is `release_device_buffer`.

**C: a machine's samples keyed by the process's device (2026-11-19).** The cost store's machine key named Metal's
device, so a run on OpenCL or the CPU's backend keyed its GPU-lane samples as a machine with no GPU. It names the
device layer's (`Device::name`: the GPU's model, `OpenCL <device>`, or `CPU`; CUDA's the backend's until step E
reads the driver's). The engine's unused re-export of `METAL_DEVICE` is gone.

**C: done (2026-11-19).** Every hand-written Metal kernel on the product path is UKL, launched through the device
layer on the process's device: the tape reads, the label passes, the views and ids, the batch runtime with its work
queue, the block programs, the joins and row programs, accel's GPU lane, and the blocks crate's kernels and labs.
Each family's tests pass on Metal, on the CPU (OpenCL's SIMD-group rule checked) and on OpenCL. What names Metal
still:

- **Metal's backend itself:** its compile of a UKL source (`rerail_ukl::metal_source`), its pipelines' archive store
  and that store's test, the probe of the label kernels' compile (Q181), the driver's allocated size, the Apple9
  family check of the block programs, and the measure of new command queues in the tape tests' diagnostics.
- **Step D's:** the generators that write MSL (`clif_msl.rs`, and `c_directives`' test of its words), `pp_on.rs`
  and `launch.rs`' guards of Metal's dispatch calls.
- **Left as they are, off the product path:** the tape format 16 spike (`native/lab/tape16`, its own workspace:
  decision 0156's measures), the Swift GPU probe (`tools/matrix-offload`) and the enqueue experiment's kernels in
  `docs/rule-engine-direction`: records of measures already taken, not code the engine runs.

**Names: `.uglykl` everywhere (2026-11-19).** A UKL source is `.uglykl` on disk and in the names programs are compiled
and cached under (`format!("{name}.uglykl")` in accel's other-device path, the tests' programs, the `/<tests>/`
paths), so a search for UKL finds every one and none is taken for another language's. The last file that kept the
old suffix, `gpu/src/library_tests.ukl`, is renamed.

**D: the preprocessor's words in UKL, and the last Metal launch path gone (2026-11-19).**

- **`clif_ukl`** (was `clif_msl`) prints the C preprocessor's compiled words (`native_cpu_pp::ukl_source`) as UKL's C:
  C types and casts, the library's bit counts, `ukl_min`/`ukl_max`, and one kernel, `pp_batch`, by `DEFINE_KERNEL2`.
  Its functions are `UKL_CALLED` (the compiler inlines as it chooses; `UKL_FUNCTION` is MLDB's shims' own, a
  versioned macro). The words read and write their arena at several widths, so every access is a byte or an
  aligned 64-bit word: the library's new `ukl_ldw_*` and `ukl_stw_*` (`ukl/memory.h`), where MSL had its aliasing
  typedefs and direct accesses. The library has `ukl_umulhi64` and `ukl_reverse32`/`64` too, each tested
  (`word_stores_at_unaligned_addresses`, `bits_within_words`) on Metal, OpenCL, the CPU and CUDA on a GB10.
  `Shape`'s labelless form is `Shape::c`.
- **`pp_on`** launches it through the lane on the process's device. A device whose buffers have no address
  (OpenCL 1.2) gets a base of its own: the words read every pointer as an offset from the arena's base.
  `the_device_words_are_the_interpreters` and `the_device_rungs_run_a_tu_again_past_its_room` pass on Metal, OpenCL
  and the CPU's backend.
- **Metal's checked encoder is gone** (`CheckedComputeEncoder`, `commit_checked`, `wait_checked`, the runtime's
  `pipeline_named*`, accel's `pipeline`, `pipeline_fixed`, `ready`, `ready_behind`): every launch is a lane
  command's. The launch guards (`gpu_launch_guard_counts_all_dispatch_apis_and_refuses_submission`,
  `an_over_budget_command_does_not_run_its_allowed_prefix`) launch UKL kernels through it on all three. The
  kernel store counts a kernel made on any backend in `made_here`, which `pipelines_compiled_here` reads; it had
  counted only the Metal pipelines nothing asked for any more. The pattern passes ask `ready_function`, not
  Metal's `ready`.
- **Found on the way:** two `parser_tests` were red. `json_msl_is_one_kernel` (now `json_ukl_is_one_kernel`)
  counted `kernel void` in UKL; it counts `DEFINE_KERNEL2(`, and its line budget is over what is generated after
  the head every source shares (`block_program::head`, since step C the whole of `block_common.uglykl`).
  `every_step_on_metal_equals_the_interpreter` compared a token's last on files tier 1 refuses, where 0411's
  gap rule and Metal's extract need not agree (its R5): it excuses the gap lasts there, as `ladder::tier1_agrees`
  does on the CPU. The crate's 86 pass on Metal.

**D: accel's scans and sorts are the library's calls (2026-11-19).**

- **The library grew to what accel's own kernels did**, so no second scan or sort is left:
  - `ScanSpace`: a scan's temporaries, made once (from a caller's scratch, `new_in`) and reused, so a scan
    allocates nothing. Each level's tile sums are scanned in place into their offsets. A scan may be in place, its
    buffer `n + 1` values with the total last.
  - A scan sized on the device: `ScanSpace::size` writes each level's count and grid from a count the work before
    made (`min(counts[slot], cap)`, accel's `scan_size`), and `scan_sized` launches on those grids. It also writes a
    grid over the outputs for the launches around it.
  - `RadixSpace` and `radix_sort_pairs_in`: the sort in kept temporaries, its histograms scanned in place.
  - The radix sort at every SIMD width: below 32 lanes a rank group is 32 threads, their digits read from
    threadgroup memory (accel's method); from 32 up, a SIMD group matched as before. The sort had refused widths
    below 8. The library's tests cover in-place and sized scans, and sorts at the CPU's width 1 and 8 and OpenCL's 1.
  - `ready()` compiles the library ahead of a first call.
- **accel calls them.** `Scan` holds a `ScanSpace`, and `encode`, `encode_into`, `size` and `encode_sized` are
  library calls on the pass's queue (`Pass::library`, whose launches count to the submission's budget). The
  sort (`encode_sort`) and the pairs' sort (`encode_sorted_pairs`) call `radix_sort_pairs_in` between accel's own
  `sort_init` and its runs. `rows_scan`'s users (the tile join, the engine's row programs) scan through `Scan` too.
  Gone: `scan_block`, `scan_add`, `scan_size`, `scan_block_sized`, `scan_add_sized`, `rows_scan`, `sort_hist`,
  `sort_scatter`, and `library_speed`, whose other side they were. Every scanned buffer has room for its total, and
  a probe's pair count is that word, read after completion. The stages that scanned `n + 1` values to get a total
  (the axes' starts, the texts' lengths) scan `n`.
- **Speed, last measured before the kernels went** (`library_speed`, Metal, load average 11 to 14): the GPU ran in
  two states that moved both sides together. In the fast one the library's 16M scan took 0.44 ms against accel's
  0.47 (0.94×), and its 4M sort 0.83 ms against 0.91 (0.91×), as at step 5. In the slow one the scan took 0.81 ms
  against 0.65 to 0.74.
- accel's 85 pass on Metal, the CPU and OpenCL, and its `gpu::` tests at OpenCL's width 1. `rerail-device-metal`'s
  59 (the engine's device stages, row programs and tile joins) pass on Metal, OpenCL and the CPU, and again with every
  stage sized on the device (`RERAIL_DEVICE_SIZED_MIN=0`). The library's 19 pass on CUDA on a GB10.
- **Found on the way:** `production_encoders_cannot_bypass_the_launch_guard` had been red since step C. It refused
  any `.commit()` in the engine's device files, and since then they commit accel's `Pass`, the checked submission.
  It now refuses a commit in a file that makes a Metal command buffer.

**Intel GPUs: OpenCL on an Arc 140V, and what Intel's own stack would add (2026-11-19).**

- **The machine:** a Lunar Lake laptop's Arc 140V (Xe2, device 0x64a0, 64 vector engines, 16 GB shared with the
  host), under WSL2. Its OpenCL is Intel's compute-runtime (NEO) 26.40: OpenCL 3.0, sub-groups of 16 or 32, 128 KB of
  local memory, work-groups of up to 1,024.
- **Setting it up:** WSL2 passes the GPU through (`/dev/dxg`), but Ubuntu 24.04's `intel-opencl-icd` (23.43) predates
  Xe2. Intel's release packages (compute-runtime 26.40, IGC 2.42.6, gmmlib 22.10.2, checked against their published
  sums) and Ubuntu's ICD loader, unpacked without root into a directory of the farm's, run it: the loader finds the
  driver through `OCL_ICD_VENDORS`, the libraries through `LD_LIBRARY_PATH` (with `/usr/lib/wsl/lib`'s). Intel
  tested WSL against Windows driver 101.9030; the machine's 101.8826 works.
- **Validated under OpenCL:** `rerail-gpu`'s 19 (the library: loads, stores, bits, SIMD-group operations, scans,
  compaction, histograms, radix sorts, LZ4, the work queue, hash tables) and the forest's 40, its device training,
  bagging and prediction tests among them. `rerail-accel` does not build on Linux yet (its `rerail-blocks`
  dependency is macOS's alone): that is step E, and the Arc is its second Linux target, beside CUDA.
- **Intel has no kernel language of its own.** Its native runtime is Level Zero (oneAPI's driver API, as CUDA's is
  NVIDIA's), which loads SPIR-V or native binaries. Those come from OpenCL C, through IGC (`ocloc`, also a library),
  or from SYCL's C++ (DPC++, with ESIMD for explicit SIMD). For UKL, Intel's path is the OpenCL shim's C either way:
  through OpenCL, or compiled by `ocloc` for Level Zero.
- **What Level Zero would be wanted for, Intel's OpenCL has already.** The backend uses none of it yet:
  - **Sub-groups:** `cl_khr_subgroups` with ballot, shuffle, relative shuffles and rotate, and
    `cl_intel_required_subgroup_size` to fix 32 lanes a kernel. The shim emulates every SIMD-group operation in
    local memory with two work-group barriers (`UKL_OPENCL_EMULATED_SG`). Native ones remove those barriers, and the
    work-group-wide waits a SIMD group's operation now costs, on every device that has them.
  - **Device addresses:** `cl_intel_unified_shared_memory` (pointers, `clSetKernelArgMemPointerINTEL`). The backend
    reports no buffer addresses (`has_addresses`, OpenCL 1.2's limit), so code that follows pointers takes the
    offset-from-a-base form (`pp_on`'s `NO_ADDRESS`). With USM, an Intel device has them, and block pool slabs can
    be host USM allocations, as CUDA's are `cudaHostAlloc`'s (E1).
  - 64-bit atomics (`cl_khr_int64_base_atomics`, `_extended_atomics`) and SPIR-V input (`cl_khr_il_program`).
- **Order, cheapest first:**
  1. Native sub-groups in the OpenCL shim and backend, where the device has them at 32 lanes (Intel's, and any
     OpenCL 3.0 device with `cl_khr_subgroups`): the shim's SIMD-group operations as `sub_group_*`, kernels
     declared at 32 lanes, the backend choosing by the device's extensions. The shim's emulation stays for
     OpenCL 1.2. Measured against the emulation on the Arc.
  2. Intel's USM in the OpenCL backend: buffers with addresses, and slabs from host USM.
  3. A Level Zero backend, only if a measurement finds OpenCL's launch or submission costs matter after 1 and 2: a
     device of the trait's, as large as CUDA's (about 900 lines), with `ocloc` to compile and Level Zero's loader
     (which Intel ships apart from the compute-runtime). It would add immediate command lists, indirect launches,
     explicit residency and device timestamps. The engine's fixed handful of launches per evaluation leaves
     launch cost little room.
  4. Windows itself (the OpenCL loader opens `OpenCL.dll`; today it opens only `libOpenCL.so`) comes with the
     workspace's Windows build, last.
- **Xe2 is SIMD16 in hardware.** A 32-lane sub-group runs as two halves. Code written for any width (the library's,
  the radix sort's rank groups since step D) also runs at 16, which may be faster there: to be measured after 1.

**E: `rerail-accel` builds on Linux, its GPU lane on any device (2026-11-19).**

- **The manifest:** the device layer's crates (`rerail-blocks`, `rerail-device`, `rerail-gpu`, `rerail-ukl`) are
  plain dependencies. Only Metal's own (`metal`, `objc`, `block`, `core-foundation-sys`) are macOS's.
- **The `gpu` lane compiles everywhere `gpu` is on.** Metal's own tier stays macOS's: its pipelines with their
  binary archives and wanted lists, `ship`, the `Shared` Metal handles, `Pooled::after`, the command buffers'
  timeline. Elsewhere a kernel is the device layer's compile (`kernels::function`), counted as Metal's are.
  `released` is an autorelease pool on macOS and the closure elsewhere. `host_clock` is the monotonic clock
  elsewhere. A submission's timeline elsewhere is the event's device seconds and the rest of its host span.
- **The lane is present when the process has a GPU on any backend** (`lanes::gpu`: the device layer's device, not
  the CPU's backend), not only a Metal one. A column's store and the block policy's device blocks lease from the pool
  on any device.
- **Found on the way:**
  - The blocks' reservations are address space (`mem::reserve`), and Linux's overcommit check refused a block's 64
    GiB on a 20 GiB machine. On Linux they take `MAP_NORESERVE`.
  - `upkeep::footprint` was 0 off macOS, so idle work never saw its memory budget. On Linux it is the resident set.
  - A sharding test asked for more used shards than an 8-thread machine has.
  - CUDA's event was `Send` and not `Sync`. Its methods take `&self` and call the thread-safe driver API.
- **Results:** accel's 60 on Linux (the Mac's 25 more are SME's, the Neural Engine's and Metal's tier) pass on CUDA on
  a GB10, and on the Arc 140V under OpenCL. The Mac's 85 pass on Metal and OpenCL. The blocks' 33 pass on Linux.
- **One limit, on the Arc under WSL2:** Intel's OpenCL wraps anonymous memory, private or shared, and private file
  mappings, but refuses a shared file mapping (`clCreateBuffer` -6), probed directly. CUDA wraps it. A deferrable
  lease for the device (0295) is such a mapping. No product path takes one yet (the store's segments are the
  host's, `lease_deferrable_host`), so the test stops there on that refusal alone. Open: whether native Linux's
  Intel driver has the same limit, and what a device lease of the deferrable tier is on a device that has it.

**E, part 2: the engine's GPU lane builds on Linux (2026-11-19).**

- **Corrected from part 1:** the device layer is macOS's and Linux's alone (`rerail-device` and `rerail-gpu` are
  gated on those targets), so plain dependencies broke accel's WASM build. They are dependencies on those two
  targets. The lane is `cfg(gpu_lane)`, which `accel/build.rs` sets where the `gpu` feature is on and the device
  layer is. accel checks for `wasm32-wasip1` again.
- **One gate, "where the device layer is":** every gate on the GPU lane in the engine's crates
  (`rerail-metal`, `rerail-device-metal`, the engine, `rerail-facts`, `rerail-c-directives`, `rerail-rr-parser`,
  `rerail-parser-tests`) is `any(target_os = "macos", target_os = "linux")`, where it was macOS's (about 150).
  These stay macOS's:
  - the OS's own facts (resource usage in bytes, FSEvents, Mach, thread names);
  - Metal's own tier: the kernel store's archives and maintenance, Metal-only kernels and tests, the cost models'
    Metal forests, the Neural Engine and SME;
  - the Apple Silicon benches, and the machine-code budgets of the AArch64 tests (`target_arch = "aarch64"`).
- **`rerail-metal` is the lane's host side on any device:** `HostMapped` is its host-mapped buffer. A pipeline's
  limits are `lane::limits` (Metal's pipeline; `None` elsewhere), and a command's times are `launch::command_times`.
  `MappedBuffer`, `PrivateInputUpload`, the private-bytes statics and the queue accessors were dead: deleted.
  Their ledger tests run on `HostMapped`. `residency::metal` only re-exported `MappedBuffer`: deleted.
- **Found on the way:**
  - **CUDA ran submissions out of order.** A queue root had 16 streams, each new queue taking the next, and a
    queue issued each launch, copy and fill as it was encoded. Metal runs nothing until a command buffer is committed,
    and a queue's command buffers in the order they are committed, and the engine relies on both: a batch reads
    what one committed before it wrote (`Batch::commit_open`), and a `Pass` dropped uncommitted runs nothing. Two
    threads' submissions ran at once, one thread's launch before another's earlier commit: the device tests faulted
    (an illegal address in `dedup_scatter`, then error 715 for every later launch of the process), and, rarely, a
    stratum read 0 rows. A root is one stream (the waiting root's stays its own, so an armed wait holds back nothing
    else), and a queue records its operations (`Op`) and issues them at `flush`, a submission's contiguously
    (`ISSUE`), between its own two events (its GPU time is its own). A queue dropped unflushed runs nothing.
  - **NVRTC has no `offsetof`.** Its front end is EDG's, with no `__builtin_offsetof` and no standard headers, so the
    layout checks of every generated read header (`reads/ids.h`, `reads/view.h`, the blocks', `join_stages`) did
    not compile on CUDA: 29 of the lane's device tests failed. They are `UKL_OFFSETOF` (`backend.h`): EDG's
    `__INTADDR__` on CUDA, `__builtin_offsetof` elsewhere. Probed on the GB10: a wrong offset fails the assert.
  - **`rerail-clif` linked Cranelift's JIT on WASM.** It takes the tree's stand-in there, as the compiler does, so
    the engine checks for WASM. (The break predates this step: 0325 step 1.)
  - **`Source::in_block` was gated on macOS**, though host code reads it and `InBlock` is on every target: ungated.
  - The AOT preprocessor's generated Rust is NEON: generated on AArch64 alone.
  - A view read in place is wanted wherever buffers have addresses (`Device::has_addresses`), not on Metal alone.
  - Linux-only warnings (unused AArch64-only helpers, a profiler's `mut`) fixed by moving the code under its gate.
- **Results:**
  - The engine, `rerail-parser-tests` and `rerail-device-metal` (libraries and tests) check on Linux and macOS; the
    engine checks for WASM.
  - The Mac's lane suite (`rerail-device-metal`) passes on Metal and on OpenCL (59 each); `rerail-metal`'s 15 and
    the header writers' tests (`rerail-clif`, `rerail-blocks`) pass.
  - On CUDA (a GB10): the lane suite's 58 (the Mac's one more is Metal's) pass, four runs of four with the tests in
    parallel, five of five one at a time; `rerail-device`'s, `rerail-gpu`'s and `rerail-metal`'s pass.
  - **The data layer's replay on the device.** The replay of the 142 checks' recorded tests ran every program bound to
    the CPU (`finding_keys`), so it said nothing of the GPU lane: profiled on the GB10, its sample launched no
    kernel. It takes the CLI's binding (`RERAIL_DEVICE=metal`, by price, or `always`). With `always`, the sample
    (two events a check) gives the recorded findings exactly on CUDA: 413,000 launches, 3.4 s of GPU time
    (`nsys`; joins' counts and scatters, copies, dedups, indexes, the library's scans and sorts, text verdicts,
    the label passes).
- **Found after the commit:** accel's tests did not compile (`blocks.rs`' test module named `Budget` and `Size`, whose
  import had moved into the lane's function): imported in the tests. accel's 85 pass on the Mac, its 60 on the Arc.

**The Intel plan, step 1: OpenCL's native sub-groups (2026-11-20).**

- **Where the device has them** (`native_subgroups`): OpenCL 3.0's sub-groups with shuffles, relative shuffles and
  ballot (`cl_khr_subgroups`, `_subgroup_shuffle`, `_subgroup_shuffle_relative`, `_subgroup_ballot`), and a kernel
  may require 32 lanes (`cl_intel_required_subgroup_size`, 32 among `CL_DEVICE_SUB_GROUP_SIZES_INTEL`). The host
  defines `UKL_OPENCL_SUBGROUPS` and builds as OpenCL C 3.0. `RERAIL_OPENCL_SUBGROUPS=0` keeps the emulation, to
  measure one against the other. Apple's OpenCL 1.2 has none: emulated, as before.
- **The shim:** each kernel requires 32 lanes (`intel_reqd_sub_group_size(32)`), a lane is
  `get_sub_group_local_id()`, and `ukl_sg_*` are the device's `sub_group_*` functions (`backend.h`). A lane outside
  the group, or the group's own at a relative shuffle's edge, reads its own value, as CUDA's and the emulation's do
  (the device's functions leave it undefined). No threadgroup word per thread, and no barriers.
- **OpenCL C 3.0 has types named `atomic_uint` and `atomic_int`** (C11's `_Atomic`, which 1.2's atomic functions do
  not take). UKL's sources use the names for a word the atomics take, so under 3.0 they are the integers' by macro;
  1.2's atomic functions remain in 3.0. The source is preprocessed on the host, before the driver defines
  `__OPENCL_C_VERSION__`, so the shim keys this on `UKL_OPENCL_SUBGROUPS`.
- **Results on the Arc 140V (WSL2):**
  - the library's 19, the device layer's 6, accel's 60 and the lane suite's 58 pass;
  - the lane suite: 2,242 s the first time (each kernel compiled cold, OpenCL C 3.0, by Intel's compiler), 21 s from
    the store (the emulation's: 773 s cold; the GB10's CUDA: 29 s);
  - accel's `host_costs`, device time, warm (each run twice; load about 1.2):

    | rows | radix sort, native | emulated | probe, native | emulated |
    |---|---|---|---|---|
    | 1,000 | 41 µs | 67 µs | 23 µs | 25 µs |
    | 20,000 | 120 µs | 156 µs | 63 µs | 64 µs |
    | 200,000 | 1,084 µs | 1,735 µs | 945 µs | 970 µs |

    The sort's ranks are SIMD-group work (ballots, shuffles): 38% less at 200,000 rows. The probe has little.
- **Open:** a cold compile of the lane's kernels takes Intel's compiler 37 minutes (an engine's first run on a new
  driver); the store keeps them after. Whether SPIR-V (`cl_khr_il_program`), compiled ahead, or Xe2's native 16
  lanes compile faster is to be measured.

**The Intel plan, step 2: what USM gives, measured (2026-11-20).**

- **The Arc 140V under WSL2** (`cl_intel_unified_shared_memory`): host, device, and single- and cross-device shared
  allocations have access and atomic access (not concurrent access); **shared system memory has none** (capabilities
  0). SVM is coarse-grained buffers only (`CL_DEVICE_SVM_CAPABILITIES` 1), and no extension gives a
  `CL_MEM_USE_HOST_PTR` buffer an address (`cl_ext_buffer_device_address` is absent).
- **So USM gives addresses to allocations, not to wrapped memory:** a host USM allocation (`clHostMemAllocINTEL`) is a
  pointer the kernels follow, as CUDA's `cuMemHostAlloc` is, and the pool's leases (columns, scratch, the blocks the
  device must read) can be ones. Host memory the device reads in place by a wrap (a mapped tape file, a block it may
  read) has none. CUDA registers such memory (`cuMemHostRegister`); OpenCL here cannot.
- **Built** (`RERAIL_OPENCL_USM=1`): a buffer the device allocates is a host allocation (`clHostMemAllocINTEL`) whose
  address is its own (`ClBuffer::address`, `Buffer::device_address`); a wrap has none. `Device::has_addresses`
  (every buffer, wraps too) stays false on OpenCL, and views read in place and 16z's slabs keep the offset form
  there. `Device::allocations_have_addresses` is what the fused sorts and probes ask (`join::Fused::can`): their
  tables hold only leases' addresses. A host allocation is bound and copied as itself
  (`clSetKernelArgMemPointerINTEL`, `clEnqueueMemcpyINTEL`; a copy to or from a wrap is the wrap's own write or
  read), and a launch names the allocations it reaches by address (`use_buffer`: `CL_KERNEL_EXEC_INFO_USM_PTRS_INTEL`,
  Metal's `useResource`).
- **Found:** an OpenCL buffer made over a host allocation (`CL_MEM_USE_HOST_PTR`) is a copy on Intel's driver (the
  in-place probe saw no write in its pages), so the allocation has no `cl_mem`. The opening checks both: heap
  buffers read in place (`reads_host_pages_in_place`), then a kernel's write through an address a table holds
  (`follows_addresses`); without the second, buffers stay the heap's. A view test asked an allocation for an
  address to mean "the views' directory is read in place"; it asks `has_addresses` now.
- **Results on the Arc 140V (WSL2), both ways:** the device layer's 6, accel's 60 and the lane suite's 58 pass.
  - allocating a buffer: 1.4 µs, against 78 µs for heap memory wrapped (the driver pins it);
  - the lane suite (warm, the fused sorts and probes on): 17.4 and 17.3 s, against 19.8 and 17.9 s;
  - the radix sort (`host_costs`): the kernels' device time is unchanged (1,065 against 1,070 µs at 200,000 rows),
    but its wall is 3-5 times as long from 20,000 rows (4.7 against 1.5 ms; 10-13 against 2.3 ms at 200,000);
    1,000 rows are equal. The CPU's reads and writes of a host allocation are the heap's speed (26 and 24 GB/s,
    against 23 and 24) and page faults are equal; the time is the driver's between kernels (more user time, a third
    of the context switches: its wait spins). Shared allocations (`clSharedMemAllocINTEL`) are the same.
- **So it is not the default:** the fused path gains about nothing on the suite's small inputs, and a large sort
  loses. Open: the gap between kernels on host allocations (a native Linux or Windows driver, or Level Zero's
  immediate command lists, may not have it; the plan's step 3 is that measurement).

**The Intel plan, steps 3 and 4: not called for yet (2026-11-20).**

- **Level Zero (step 3) waits:** its condition is a measurement in which OpenCL's launch or submission costs matter
  after steps 1 and 2. On the Arc a launch costs 6.1-6.6 µs and an empty submission 74-96 µs (`host_costs`), against
  the engine's fixed handful of launches per evaluation, and the lane suite takes 17-19 s warm (the GB10's CUDA:
  29 s). The one cost found, the gap between kernels on host allocations, is the opt-in USM path's; whether Level
  Zero's immediate command lists lack it is that backend's first measurement, if the path is wanted. The machine has
  Intel's Level Zero driver and `ocloc`, not the loader.
- **Windows (step 4)** comes with the workspace's Windows build, which is not written; the laptop runs the farm's
  Linux jobs under WSL2.

**A 64-lane mask: `ukl_mask_t` (2026-11-20).**

- **The type** (`ukl/backend.h`): a SIMD group's lanes as bits, lane `l` at bit `l`; 32 bits, or 64 where
  `UKL_SIMD_WIDTH` is wider (AMD's 64 lanes). Every backend's ballot and match return it, and `ukl_mask_count`,
  `_lowest`, `_lane`, `_below` and `_rank` read it at either width. `UKL_SIMD_WIDTH` is a power of two from 1 to 64.
- **Its users:** the votes (`ukl_sg_count`, `_count_below`, `_first`), the match, and the radix sort's rank
  (`radix_tile.h`: a 64-lane rank group); the transducer's parities (`block_common`). That file's borrow is a
  carry-lookahead over a 32-lane add, so it refuses a wider group at compile time. Upstream's CPU shim operations
  (`ukl_simdgroup_*`) stay 32-lane, as upstream's kernels are.
- **Tested on the CPU at 64 lanes** (`RERAIL_CPU_SIMD_WIDTH=64`, now allowed): the library's 19 pass, as at 32, 8
  and 1. Two of the tests assumed 32-thread groups (a threadgroup of 32 threads, the LZ4 decoder's group): they take
  the device's width now. The test of the votes writes each mask as two words.

- **Open:** `rerail-device-metal`'s tests register Python's and `.rr`'s packs (`test_languages`) only where two tests
  call it, while its corpus runs read `.rr` documents (`lines::of`) in tests that do not: a CPU worker panics
  ("no language registered for Rr"), now and then by which test runs first, caught and run again. Every test passes
  through it, on the Mac as on Linux. The crate's tests need the engine's load-time registration.

A comes first because C and D port the files the branch changes; B before C so that each ported file runs on every
backend as it lands. Between them, small: a 64-lane mask type (`ukl_mask_t`), tested on the CPU at width 64 (AMD's
width, with no AMD device). Native OpenCL subgroups and SVM wait for a device to test them on.

## 9. Questions for the user, and the answers (2026-11-18)

- **Q1. Blocks on a partitioned GPU.** Answer: "blocks will need to be copied to and from devices, and for us to
  manage coherency between blocks read and written by different kernels on different hosts and GPUs (a host may
  have more than one GPU too)". Folded into §7.2 (copies between memory domains, coherency by versions and a
  directory; 0353 holds within a domain) and §7.4.
- **Q2. Generated code as UKL text.** Answer: "write UKL". Folded into §3.3.
- **Q3. Vendor libraries.** Answer: "yes, that is the goal: same code, high-level calls will optimize per backend
  behind the call". Folded into §5.
- **Q4. A discrete GPU.** Answer: a cloud VM with a discrete NVIDIA card, used before for embedding benchmarks.
  Its access is kept outside the repository. Folded into §8, step 11.
