# 0400. GPUs beyond Metal: kernels written once in UKL, a device layer under the lanes, memory by class

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
| CPU | the CPU shim (coroutines for barriers) | `ukl_cpu_shim.h` (exists) | a reference for tests, not a product path |
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

Steps 1, 2 and E1 are small and settle the bets the rest rests on: the preprocessor, the shim, and in-place
bandwidth. Step 3 can run beside them.

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
