# 0617. Matrix units through UKL: native-shaped fragments, owned by one or more SIMD groups, emulated where none

Status: decided (2026-11-22), by the user's answers:

- types: "f16, bf16, i8";
- shape: "Native shape per device";
- the M5: "Yes: multi-group fragments";
- devices with no machine to test them: "You can un-drop the ark you dummy to test that, but either way our goal is
  to get the shape right, not to cover everything perfectly".

It extends decision 0440 §5 (the library: the call is the interface, the implementation per backend).

## Intro

Several of the GPUs we run on have matrix units: hardware that multiplies two small matrices in one instruction, many
times faster than the same work done lane by lane.

- NVIDIA's tensor cores, reached through `mma.sync`.
- Apple's `simdgroup_matrix`, and on the M5, the neural accelerators reached through Metal 4's tensor operations.
- AMD's WMMA (RDNA3 and later) and MFMA (CDNA).
- Intel's XMX, reached through DPAS.

The rest have none: the Radeon 680M (RDNA2), and the CPU.

UKL needs one call that a kernel writes once and that every backend runs:

- the unit's own instruction where there is one;
- an emulation where there is none, giving the same results.

Two things make this harder than UKL's other SIMD-group operations:

1. **The units differ in shape.** Each multiplies its own sizes of tile:
   - 16×8×16 on NVIDIA, 8×8×8 on Apple, 16×16×16 on AMD, 8×16×16 on Intel;
   - on the M5, sizes the kernel configures.
2. **They differ in who holds a tile.** On NVIDIA, Apple, AMD and Intel, one SIMD group holds a tile in its
   registers. On the M5, several SIMD groups hold one tile together. In most cases the layout of a tile's elements
   across lanes is not published, or is published only for one vendor.

A **fragment** here means a tile of a matrix held in registers by its owner, one SIMD group or several. The
**owner** is the set of SIMD groups that hold a fragment and call its operations together.

## Decision

### 1. Types

The first cut has three input types, each with its accumulator: `f16` into `f32`, `bf16` into `f32`, and `i8` into
`i32`. These are what the units have in common. 1-bit and 4-bit matrices, and tf32, wait for a kernel that needs
them.

Whether a type is native is a fact of the device, not of the backend. For example, Metal's `simdgroup_matrix` has
`half` and `bfloat` but no `i8`. So each (device, type) pair is either native or emulated, and a kernel's source is
the same either way.

### 2. Shapes: each device's own, as compile-time constants

The backend defines each type's fragment shape and its owner for the device it compiles for:

| Constant | Meaning |
|---|---|
| `UKL_MMA_M(T)`, `UKL_MMA_N(T)`, `UKL_MMA_K(T)` | the shape of one multiply-accumulate of type `T`: `C[M×N] += A[M×K] · B[K×N]` |
| `UKL_MMA_GROUPS(T)` | the SIMD groups that own one fragment: 1, or more (the M5's tensor operations) |
| `UKL_MMA_NATIVE(T)` | 1 where the unit runs it; 0 where it is emulated |

**One rule binds every backend: each of M, N and K divides 64** (`UKL_MMA_BLOCK`). A kernel tiles its work in blocks
of 64 (or multiples of 64) and loops over the native fragments inside each block, so the same loop is right on every
device. Every shape listed above divides 64. The M5's configurable shapes are chosen to divide it too.

### 3. Fragments and their operations

A fragment's type is opaque, one per type and role: `ukl_mma_a_f16`, `ukl_mma_b_f16`, `ukl_mma_c_f32`, and so on.
Each backend defines what is inside: registers, a `simdgroup_matrix`, or a `cooperative_tensor`.

Every operation is convergent over the owner: every lane of every owning SIMD group calls it.

| Operation | What it does |
|---|---|
| `ukl_mma_fill_c(&c, v)` | every element of C set to `v` |
| `ukl_mma_load_a(&a, p, stride, layout)`, `_b`, `_c` | a fragment read from device or threadgroup memory, row-major or column-major (compile-time `layout`) |
| `ukl_mma(&c, &a, &b)` | `c += a · b` |
| `ukl_mma_store_c(&c, p, stride, layout)` | C written to device or threadgroup memory |
| `UKL_MMA_EACH(c, row, col, v) { … }` | each element this thread holds, with its row and column, read and written by name (epilogues: bias, activation, masking, a row's reduction) |
| `ukl_mma_owner()` | which fragment owner this SIMD group belongs to: its SIMD group index divided by `UKL_MMA_GROUPS` |

- **No per-lane indexing.** Neither Apple nor the M5 publishes which lane holds which element. So a kernel never asks
  for "lane i's element j": it visits its elements with their coordinates (`UKL_MMA_EACH`).
  - CUDA's layout is published. Apple's `thread_elements()` and the M5's `cooperative_tensor` each give a thread's
    elements with a way to find their coordinates.
  - The test of requirement R4 below checks each backend's coordinates.
- **Loads and stores read memory where it lies.** A fragment is registers: decision 0353's memory hierarchy, not a
  copy. A fragment is loaded from the block or threadgroup memory a kernel already reads, and stored to the relation
  or threadgroup memory it writes.
- **The owner is part of the shape.** A kernel's threadgroup holds a whole number of owners. It finds its owner with
  `ukl_mma_owner()`, and gives each owner its own fragments. A kernel written this way runs unchanged with owners of
  one SIMD group or of four.

### 4. Each backend's form

| Device | Native form | Types native | Shape (M×N×K), owner | Machine to test it |
|---|---|---|---|---|
| NVIDIA sm_80 and later (the Sparks' GB10) | PTX `mma.sync` | f16, bf16, i8 | 16×8×16 (i8: 16×8×32), 1 group | the Sparks |
| NVIDIA sm_70 (V100) | `mma.sync` m8n8k4, or `wmma` 16×16×16 | f16 | to choose by test; bf16 and i8 emulated | the V100 VM |
| Apple M1-M4 | `simdgroup_matrix` | f16; bf16 where the device has it (to verify per Mac) | 8×8×8, 1 group | the Macs |
| Apple M5 | Metal 4 `matmul2d` over `cooperative_tensor` | f16, bf16; i8 to verify | configured (dividing 64), several groups | the M5 |
| Intel Xe2 (the Arc 140V) | `cl_intel_subgroup_matrix_multiply_accumulate` (DPAS) | f16, bf16, i8 | 8×16×16 (i8: K 32) at sub-group width 16 (to verify) | the Arc: shape only, no performance work (decision 0576's compile times) |
| AMD RDNA3 and later | `v_wmma_*` | f16, bf16, i8 | 16×16×16, 1 group (wave32) | none yet: emulated until one joins the farm |
| AMD RDNA2 (the Radeon 680M) | none | none | emulated: 16×16×16, 1 group | the Radeon 680M host |
| The CPU | none | none | emulated: 16×16×16, 1 group, shim lanes | every host |

- **Emulated means the reference form.** It is the library's portable UKL, written over the SIMD group's shuffles:
  1. each lane holds rows of A and columns of B;
  2. it reads the other lanes' values by shuffle;
  3. it accumulates its own elements of C.

  Where the device has dot-product instructions (RDNA2's `v_dot2_f32_f16` and `v_dot4_i32_i8`, confirmed on
  gfx1035; CUDA's `dp4a`), the emulation uses them through `<ukl/dot.h>` (§7).
- **Hopper's and Blackwell's warpgroup instructions** (`wgmma`, `tcgen05`), where four warps own one tile, have
  the M5's shape: an owner of several groups. They fit the same interface, and are not in this first cut.

### 5. Level 2: tile calls

`ukl_gemm_tile(A, B, C, …)` multiplies a whole tile from memory (a block of 64 or a multiple). Its reference is
written once over level 1, and a backend may override it (cuBLASDx or CUTLASS-style code on CUDA; Metal's
Performance Primitives on the M5), as decision 0440 §5 says. A kernel that only needs whole tiles calls level 2. A
kernel with its own epilogue, or with a loop that interleaves other work, uses level 1.

### 6. What a kernel looks like

```c
// C[64×64] of this owner's block += A[64×K] · B[K×64], f16 into f32, on any device.
#define M_ UKL_MMA_M(f16)
#define N_ UKL_MMA_N(f16)
#define K_ UKL_MMA_K(f16)
ukl_mma_c_f32 c[64 / M_][64 / N_];
for (uint i = 0; i < 64 / M_; i++) for (uint j = 0; j < 64 / N_; j++) ukl_mma_fill_c(&c[i][j], 0.0f);
for (uint k = 0; k < K; k += K_) {
    for (uint i = 0; i < 64 / M_; i++) {
        ukl_mma_a_f16 a; ukl_mma_load_a(&a, A + i * M_ * lda + k, lda, UKL_ROW_MAJOR);
        for (uint j = 0; j < 64 / N_; j++) {
            ukl_mma_b_f16 b; ukl_mma_load_b(&b, B + k * ldb + j * N_, ldb, UKL_ROW_MAJOR);
            ukl_mma(&c[i][j], &a, &b);
        }
    }
}
// The epilogue, by coordinates: a bias per column, then the store.
for (uint i = 0; i < 64 / M_; i++) for (uint j = 0; j < 64 / N_; j++) {
    UKL_MMA_EACH(c[i][j], row, col, v) { v += bias[j * N_ + col]; }
    ukl_mma_store_c(&c[i][j], C + i * M_ * ldc + j * N_, ldc, UKL_ROW_MAJOR);
}
```

How many fragments a kernel holds depends on the device's shape, and so does how many registers it uses. The block
size (64) is the kernel's choice, and a backend's measurements may change it, never its results.

### 7. Dot products: one thread's, every backend (2026-11-22)

The user: "please also make dot product operations available, generically, through UKL and hook them up".

`<ukl/dot.h>` gives every kernel one thread's dot products of packed small values. Each operand is a 32-bit word
holding four bytes or two 16-bit floats, lowest first:

| Call | Result |
|---|---|
| `ukl_dot4_i8(a, b, acc)` | `acc + Σ a[k]·b[k]` over four signed bytes, into an i32: exact, wrapping |
| `ukl_dot4_u8(a, b, acc)` | the same over unsigned bytes, into a u32 |
| `ukl_dot2_f16(a, b, acc)` | `acc + a[0]·b[0] + a[1]·b[1]` over two f16s, into an f32 |
| `ukl_dot2_bf16(a, b, acc)` | the same over two bf16s |
| `ukl_f16_to_f32(h)`, `ukl_bf16_to_f32(h)` | a 16-bit float's bits as an f32, exactly |

- **The float forms are within one rounding.** Their products are exact in f32, so only the sums round:
  - The reference rounds twice: a fused multiply-add of each product in turn.
  - AMD's `v_dot2_f32_f16` rounds the exact sum once. On the Radeon 680M it matched the exact sum rounded once in
    every case of a probe, and the two-`fma` chain in about 1 case in 7.
  - So a native float form is checked within one rounding of the reference, and to the bit where the sums are
    exact.
- **The native forms, each behind a flag in `ukl/backend.h`:**

| Backend | Bytes (`UKL_NATIVE_DOT4`) | f16 (`UKL_NATIVE_DOT2_F16`) | f16 conversion (`UKL_NATIVE_F16`) |
|---|---|---|---|
| CUDA | `__dp4a` (sm_61 and later) | the reference's arithmetic | PTX `cvt.f32.f16` |
| Metal | the reference (Metal has no integer dot product) | the reference's arithmetic | `half` |
| OpenCL on AMD RDNA2 (`gfx103x`) | `v_dot4_i32_i8`, `v_dot4_u32_u8` | `v_dot2_f32_f16` | `vload_half` |
| OpenCL with `cl_khr_integer_dot_product` (built as OpenCL C 3.0) | `dot(char4, char4)` | the reference's arithmetic | `vload_half` |
| Other OpenCL | the reference | the reference's arithmetic | `vload_half` |
| The CPU | the reference | the reference | the reference |

  - The OpenCL host chooses the define once, when it opens the device (`dots`: `UKL_OPENCL_AMD_DOT` or
    `UKL_OPENCL_INT_DOT`). `RERAIL_OPENCL_DOT=0` keeps the reference.
  - Other AMD chips' instructions differ (RDNA3's `v_dot4_i32_iu8`, for example), so they take the reference until
    one is tested.
- **The test, `library_tests::dot_products_equal_their_references`:** 65,536 threads.
  - The byte forms are compared with the host's exact sums. Every thread's words are checked, and a third of the
    threads get random bits.
  - The float references are compared with the host's to the bit. The native float forms are compared with the
    reference within one rounding, and to the bit on small integers.
  - The f16 conversion is checked on all 65,536 bit patterns, NaNs included.
  - It passes on the CPU and CUDA (a GB10), on Metal (an Apple-silicon Mac), and on OpenCL on the Radeon 680M. On the Radeon,
    AMD's native forms ran: `fdot2` equalled the reference to the bit in 40,699 cases and was within one rounding in
    2,992, none of them on small integers.
- **The matrix units' emulation (§4) is written over these calls,** so a device with dot-product instructions
  emulates its fragments with them.

## Requirements, and the test of each

Each test runs on every backend, in UKL's library tests (`ukl/tests`). Every result is checked against the CPU's
emulated form, which is the reference.

| # | Requirement | Test |
|---|---|---|
| R1 | Every backend's shapes divide 64, and its owner divides the threadgroup's SIMD groups | a compile-time assert in `backend.h` for each type, and a test that prints each device's constants |
| R2 | Exact where the arithmetic is exact: i8 always, and f16 and bf16 on small integers | a 64×64×256 product on each type, inputs small integers (every product and sum exact in f32), compared bit for bit |
| R3 | Floats within a bound elsewhere | random f16 and bf16 inputs; each element within `K · 2^-p · Σ|a||b|` of the reference (`p` the input type's precision) |
| R4 | `UKL_MMA_EACH` visits each element of C once, at its true coordinates | C filled with `row × 256 + col` through `EACH`, then stored, and compared with the matrix of coordinates |
| R5 | Owners of several groups run unchanged | the R2 kernel run with threadgroups of 1, 2 and 4 owners; on the CPU the emulation run with `UKL_MMA_GROUPS` forced to 2 and 4, so the multi-group path is tested where no device has it |
| R6 | Native and emulated forms agree on each device | where `UKL_MMA_NATIVE(T)` is 1, the same kernel run again with the emulation forced, results compared as R2 and R3 |
| R7 | A native type takes the unit's instruction | the dumped kernel holds it: `mma.sync`, `simdgroup_multiply_accumulate`, `matmul2d`, `v_wmma`, DPAS (a check on the dumped source, as the lowering tests do) |
| R8 | Loads and stores read and write memory in place | fragments loaded from a relation's column in its block and stored into a relation in a block, with no staging buffer (`check_no_block_copies.py`) |
| R9 | `ukl_gemm_tile` equals level 1's loop | each type, R2's and R3's inputs |

## Order of work

1. The interface in `ukl/matrix.h`, with the emulated form and the CPU's shim lanes, and R1-R6 and R9 on the CPU.
   This fixes the shape before any device adds its form.
2. CUDA's `mma.sync` on the Sparks (R2-R7). Then the V100: its form, or the emulation, chosen by test.
3. Metal's `simdgroup_matrix` on M1-M4 (f16; bf16 where present; i8 emulated). R4 is what checks the coordinates
   behind `thread_elements()`.
4. The M5's `matmul2d`: the first owner of several groups on a device (R5).
5. Intel's DPAS on the Arc: R2, R4 and R7, the shape only.
6. The Radeon 680M: the emulation, with its dot-product instructions where they help.
7. `ukl_gemm_tile`, with one backend override (the M5's or CUDA's) to prove the override path.

Steps 2-6 each add one section of `backend.h` and change nothing in a kernel.

## Open

- **The first kernel that uses it.** The interface is shaped to fit a real kernel, and this decision names none.
  Candidates are the learned cost model's trees on the GPU, and the embeddings. The first kernel's needs (an
  epilogue, a reduction by row) are what R4 and level 2 must serve.
- **AMD WMMA's native form** waits for an RDNA3 device. A HIP backend for AMD (decision 0440) would reach WMMA through
  the same intrinsics.
