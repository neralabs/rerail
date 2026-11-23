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
`i32`. These are what the units have in common. A fourth, `b1` into `i32` (`C += popcount(A & B)`), came with its
kernel, the bit product (2026-11-22, "The 1-bit type, built" below). 4-bit matrices and tf32 wait for a kernel that
needs them.

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
// C[64×64] of this owner's block += A[64×K] · B[K×64], f16 into f32, on any device (`ukl/matrix.h`).
#define M_ UKL_MMA_M(f16)
#define N_ UKL_MMA_N(f16)
#define K_ UKL_MMA_K(f16)
UklMmaAt at = ukl_mma_at(lid);
ukl_mma_c_f32 c[64 / M_][64 / N_];
for (uint i = 0; i < 64 / M_; i++) for (uint j = 0; j < 64 / N_; j++) ukl_mma_fill_c_f32(&c[i][j], 0.0f, at);
for (uint k = 0; k < K; k += K_) {
    for (uint i = 0; i < 64 / M_; i++) {
        ukl_mma_a_f16 a; ukl_mma_load_a_f16(&a, A + i * M_ * lda + k, lda, UKL_ROW_MAJOR, at);
        for (uint j = 0; j < 64 / N_; j++) {
            ukl_mma_b_f16 b; ukl_mma_load_b_f16(&b, B + k * ldb + j * N_, ldb, UKL_ROW_MAJOR, at);
            ukl_mma_f16(&c[i][j], &a, &b, at __UKL_ARGS);
        }
    }
}
// The epilogue, by coordinates: a bias per column, then the store.
for (uint i = 0; i < 64 / M_; i++) for (uint j = 0; j < 64 / N_; j++) {
    UKL_MMA_EACH(c[i][j], at, row, col, v) { v += bias[j * N_ + col]; }
    ukl_mma_store_c_f32(&c[i][j], C + i * M_ * ldc + j * N_, ldc, UKL_ROW_MAJOR, at);
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
| R7 | A native type takes the unit's instruction | the compiled kernel's text holds it: `mma.sync`, `simdgroup_multiply_accumulate`, `matmul2d`, `v_wmma`, DPAS (`matrix_native_forms_take_the_units_instruction`, on the backend's text: CUDA's PTX, Metal's AIR, Intel's binary and its `has_dpas`) |
| R8 | Loads and stores read and write memory in place | fragments loaded from a relation's column in its block and stored into a relation in a block, with no staging buffer (`check_no_block_copies.py`) |
| R9 | `ukl_gemm_tile` equals level 1's loop | each type, R2's and R3's inputs |

## Order of work

1. The interface in `ukl/matrix.h`, with the emulated form and the CPU's shim lanes, and R1-R6 and R9 on the CPU.
   This fixes the shape before any device adds its form. Done 2026-11-22, on every backend ("Step 1, built" below).
2. CUDA's `mma.sync` on the Sparks (R2-R7). Then the V100: its form, or the emulation, chosen by test. Done on
   the Sparks 2026-11-23: b1's form, and C in `mma.sync`'s layout for every type ("The 1-bit type, built"); f16, bf16
   and i8 ("Step 2 on the Sparks, built"). The V100 runs the emulation, its test's choice ("The V100").
3. Metal's `simdgroup_matrix` on M1-M4 (f16; bf16 where present; i8 emulated). R4 is what checks the coordinates
   behind `thread_elements()`. Done 2026-11-23 ("Step 3, built").
4. The M5's `matmul2d`: the first owner of several groups on a device (R5).
5. Intel's DPAS on the Arc: R2, R4 and R7, the shape only. Done 2026-11-23 ("Step 5, built").
6. The Radeon 680M: the emulation, with its dot-product instructions where they help. Done 2026-11-23 ("Step 6,
   built").
7. `ukl_gemm_tile`, with one backend override (the M5's or CUDA's) to prove the override path. Done 2026-11-23,
   CUDA's ("Step 7, built").

Steps 2-6 each add one section of `backend.h` and change nothing in a kernel.

### Step 1, built (2026-11-22)

The user: "please do step 1. Find two uses for it in the existing ukl kernels".

- **`<ukl/matrix.h>`** holds the interface and its emulated form, for every backend.
- **How it differs from §3, each forced by UKL's C:**
  - **The names carry the type** (`ukl_mma_load_a_f16`, `ukl_mma_i8`, `ukl_mma_store_c_f32`). UKL is C on the CPU and
    on OpenCL, so it has no overloading.
  - **Each operation takes the thread's place, `UklMmaAt`** (its lane, and its SIMD group in the owner), from
    `ukl_mma_at(lid)`. UKL's SIMD-group operations take their lane from the caller (`ukl/simd.h`), and a backend has
    no portable lane id. So `UKL_MMA_EACH(c, at, row, col, v)` takes it too, and `ukl_mma_owner(lid)` takes the
    thread's id.
  - **A load or store from threadgroup memory is the `_local` form** (`ukl_mma_load_a_f16_local`). Metal and OpenCL
    type a pointer by its address space.
  - **Level 2 is `ukl_gemm_tile_<T>`,** one owner's 64×64 block of C, read from and written to device memory. Each
    of A, B and C has its own leading dimension and layout.
- **The emulated form:** every type 16×16×16, from `<ukl/dot.h>`'s dot products and the SIMD group's shuffles.
  - **C:** its 256 elements lay row-major across the owner's lanes. Superseded the same day: each backend's layout
    of rows by columns ("The 1-bit type, built").
  - **A and B:** each SIMD group of the owner holds all of both, packed along K (two 16-bit floats, or four bytes, a
    word).
  - **The product:** each shuffle reads the same register in every lane. At 16 lanes or more, that register is a
    constant, and a lane reads its column of B once a product.
  - **Narrower groups** (OpenCL's 8 lanes, or 1) read each element's row and column apart. Their register is picked
    by value, as an owner of several groups knows it only at run time.
  - **Owners of several groups:** a test's define (`UKL_MMA_EMULATED_GROUPS`) makes the emulation's owners span 2 or
    4 SIMD groups, so the multi-group path runs where no device has one yet.
- **The tests:** `rerail-gpu`'s `matrix_tests` (kernels in `matrix_tests.uglykl`).

  | Test | Requirements | What it checks |
  |---|---|---|
  | `matrix_shapes_divide_the_block` | R1 | each type's shape divides 64, and an owner's lanes are its groups'; the constants are printed |
  | `matrix_products_are_exact_on_small_integers` | R2, R9 | small integers, exact, in each type: K = 128 by level 1 and by level 2; K = 32 in all eight layouts from device memory, and in two from threadgroup memory |
  | `matrix_products_are_within_the_bound_on_random_inputs` | R3, R9 | random floats within `K · 2^-p · (Σ\|a\|\|b\| + \|c\|)`; random bytes exactly; level 2 equal to level 1 to the bit |
  | `matrix_each_visits_every_element_at_its_coordinates` | R4 | owners of 1, 2 and 4 groups; f32 and i32 |
  | `matrix_owners_of_several_groups_agree` | R5 | owners of 1, 2 and 4 groups, in threadgroups of 1, 2 and 4 owners |

  - R1's static assertions are in the header.
  - R6 waits for the first native form (step 2): it compares a native form with the emulation, and every type is
    emulated today.
  - R7 and R8 belong to the native forms and the first kernel.
- **Measured:** all five tests pass on all five backends, every type emulated, each at 32 lanes:
  - the CPU (its shim lanes, on a Linux host);
  - CUDA (a GB10);
  - HIP and OpenCL (the Radeon 680M);
  - Metal (an M3).

  On the CPU, each test takes 0.15 to 0.6 s. The small-integer test took 1.04 s until two cuts: its large case from
  K = 256 to K = 128, and its threadgroup-memory cases to two layouts (all row-major, all column-major). It now takes
  0.59 s. UKL's own tests compile the header for every backend, and as OpenCL C 1.2 at 32, 8 and 1 lanes.

### Uses in the existing kernels (2026-11-22)

All the existing UKL kernels were read for a matrix product: the 21 `.uglykl` files, the forest's kernels and the
library. One kernel computes a dense product: the relational bit product, `bp_tile` in
`accel/src/gpu/relational.uglykl`. It computes `C[i][j] = popcount(A_i & B_j)`, in tiles of 64×64. It serves two of
the engine's workloads from the matrix-offload spikes (`tools/matrix-offload/RESULTS.md`):

1. **W1, the prefilter** (mode 1): which rules a document may match, from its names' bits against each rule's, as a
   mask.
2. **W5, co-occurrence** (mode 0): the counts of documents that hold each pair of names (`AᵀA`).

- **Its product is of 1-bit values, not f16, bf16 or i8.** §1 left 1-bit matrices for "a kernel that needs them",
  and this is that kernel.
- **NVIDIA's units do it natively:** `mma.sync` over `.b1` with AND and POPC.
- **Elsewhere, the threads' popcount wins.** On the M-series GPU it beat the int8 units, which would also read 8
  times the bytes (RESULTS.md, finding 2).
- **The other kernels are not dense products**, and the spikes measured each one's matrix form slower than the form
  they use:
  - the count table and the joins are equality joins: counting arrays, 26 to 200 times faster than all-pairs tiles;
  - the histograms are group-bys: the one-hot product on the matrix units ran 3 times slower than threadgroup
    atomics;
  - the closure is a level-by-level DP over the condensed DAG: squaring was 100 to 600 times slower;
  - the forest's training sums f32 weights into buckets, which an f16 or bf16 product would round.

### The 1-bit type, built (2026-11-22)

The user: "yes, please add the 1-bit path and emulation/lowering to popcount on others".

- **`b1`:** `C[16×16] += popcount(A & B)` into `ukl_mma_c_i32`. In memory a b1 element is a bit of a `uint64_t`
  holding 64 along K (`UKL_MMA_PACK(b1)` is 64); K counts bits, offsets and strides count words.
  - **NVIDIA sm_80 and later** (`UKL_MMA_NATIVE(b1)`): K = 256, two `mma.sync.m16n8k256...b1.and.popc` a product,
    one for each half of C's columns. A's and B's registers are halves of the words in memory, read where they lie.
  - **Elsewhere:** K = 64, popcount on the threads. Each lane loads its own rows of A and columns of B (no shuffle:
    the lanes that share a row read the same word, a broadcast of the memory) and adds `popcount(a & b)`.
- **C's layout is each backend's:** a lane holds `UKL_MMA_RB` rows by `UKL_MMA_CB` columns (`ukl_mma_row`,
  `ukl_mma_col`).
  - On NVIDIA it is `mma.sync`'s accumulator layout, so a native product of any type reads and writes C in place.
    f16, bf16 and i8 stay emulated there, over that layout.
  - Elsewhere at 16 lanes or more, rows and columns interleave (4 rows by 2 columns at 32 lanes). The shuffled
    types now read each of a lane's rows of A and columns of B once a k-word (6 shuffles at 32 lanes, not 9).
  - Narrower groups keep rows and columns whose words lie in the same register of every lane.
- **`UKL_MMA_FORCE_EMULATED`** (a test's define) runs the emulation where a native form exists: R6's other side.
- **The tests** (`matrix_tests`) take b1 in every case above, plus `matrix_native_forms_are_the_emulations` (R6):
  where a type is native, its products equal the forced emulation's in all eight layouts, by level 1 and level 2.
  - Measured passing: CUDA (a GB10: b1 native, R6 compared), OpenCL (the GB10's, and a Mac's), Metal (an M1), and
    the CPU at 1, 8, 16, 32 and 64 lanes.
  - R7 is the native form's agreement with the forced emulation, and `UKL_MMA_NATIVE(b1)` printed as 1 on the GB10;
    there is no check on the dumped source yet.
- **The bit product (`bp_tile`) runs on it.**
  - A threadgroup's eight owners (at 32 lanes) take two fragments each, which share their rows of A.
  - A and B are staged as before, u64 k-major, at a stride of 72 words, so that NVIDIA's half-word loads fall in
    distinct banks.
  - The epilogue writes C, or sets mask bits, through `UKL_MMA_EACH`.
  - `accel`'s `bit_product_counts_and_pairs` passes on CUDA, OpenCL, the CPU and Metal.
- **Its speed, measured** (`accel`'s price bench, `RERAIL_PRICE_ONLY="bit product"`; device time, best of 7; old
  kernel and new one run back to back on the same machine):

  | Workload | GB10, old | GB10, new | M1, old | M1, new |
  |---|---|---|---|---|
  | W1: 13,088 × 444 rows, 768 bits | 0.169 ms | 0.170–0.187 ms | 1.563 ms | 1.767 ms |
  | W5: 2,048 × 2,048 rows, 13,088 bits | 1.008 ms | 0.951–1.001 ms | 7.30–7.37 ms | 7.77 ms |

  - **On the GB10 the unit gains nothing yet.** The product is not what limits the kernel: each tile reads its 64
    rows of each side from L2 into threadgroup memory, about 215 MB for W5.
  - **On the M1 the port is 6% (W5) to 13% (W1) slower.** Per word, both kernels issue the same loads and
    popcounts. What changed the time on the way:
    - b1's words had been 32 bits: 4 times slower;
    - then the emulation's K was cut from 256 to 64: 2 times;
    - then the words became 64 bits: the gap above.
    - Staging C through threadgroup memory, and storing it directly, did not change it.
- **Open:**
  - the M1's remaining gap, which needs its compiled shader read;
  - the tile's reads, which bound both machines: larger tiles, or both sides' rows read once for several tiles.
    Measured 2026-11-23: they bind neither ("The bit product's reads").

### Step 2 on the Sparks, built (2026-11-23)

- **NVIDIA sm_80 and later run f16, bf16 and i8 on the unit** (`UKL_MMA_NATIVE` 1 for each, with b1's):
  - f16 and bf16: two `mma.sync.m16n8k16` a product (`.f32.f16.f16.f32`, `.f32.bf16.bf16.f32`), one for each half of
    C's columns, K 16;
  - i8: two `mma.sync.m16n8k32` (`.s32.s8.s8.s32`), K 32, the unit's own;
  - A's and B's registers are words of two 16-bit floats or four bytes along K, the lowest k lowest, each element
    read where it lies (any layout, device or threadgroup memory); C is read and written in place, its layout already
    the unit's.
- **R6 compares as R2 and R3,** not bit for bit: a unit's float sums run in its own order. On small integers the
  native and forced-emulated products are each the exact product, to the bit; on random inputs the integer types'
  equal the emulation's, and the floats' are within R3's bound of the exact product. All eight layouts, by level 1
  and level 2.
- **R7 is checked:** `matrix_native_forms_take_the_units_instruction` reads the PTX NVRTC makes of the test program
  (`kernels::native_code`, the device's `compute_<sm>`, the library's options) and finds each native type's
  `mma.sync`; the forced emulation's holds none. A backend with a native type and no compiled text to read fails the
  test, so R7 is never skipped quietly.
- **Measured:** the seven matrix tests pass on CUDA (a GB10: every type native), and on the same host's CPU and
  OpenCL backends (every type emulated); UKL's own tests (the header for every backend) and `accel`'s GPU tests on
  CUDA pass.

### Step 3, built (2026-11-23)

- **Apple's M1-M4 run f16 and bf16 on `simdgroup_matrix`** (`UKL_MMA_APPLE`): a 16×16 fragment of A or B is four
  8×8 matrices, each loaded where it lies with `simdgroup_load` (a column-major one transposed as it loads), from
  device or threadgroup memory. A product takes each of C's four 8×8 matrices into a `simdgroup_float8x8` through
  `thread_elements()`, two `simdgroup_multiply_accumulate` along K, and back: registers only. K stays 16, the
  fragment 16×16, so every kernel's loop is unchanged.
- **C's layout on Metal is the unit's:** a lane's two elements of each 8×8 matrix, side by side in one row
  (`thread_elements()`, row `(q & 4) + (lane / 2) % 4`, columns `(q & 2) · 2 + (lane % 2) · 2` and one on, `q` the
  lane's quad). Apple does not publish it; R4 checks it, and passes on the M1 and the M3. i8 and b1, emulated, take
  the same layout.
- **bf16 is native from Metal 3.1** (`__METAL_VERSION__ >= 310`): `bfloat` and `simdgroup_bfloat8x8` came then.
  `__HAVE_BFLOAT__`, which the headers might have suggested, is not defined at 3.1 on macOS 15. On the M1 (no bf16 in
  its hardware) the compiler takes the same intrinsic, and the products agree with the emulation (R6); how its back
  end runs them is the compiler's.
- **R7 on Metal reads the AIR the device's compiler makes:** `native_code` compiles the program as a dynamic library,
  serializes it, and lists the `air.` names in its bitcode. The farm's Macs have no Metal toolchain for `xcrun metal`
  (one has no Xcode). Two things on the way:
  - metal-rs's `URL::new_with_string` owns an autoreleased `NSURL`; its release with the pool's crashed the process.
    The serialization sends `serializeToURL:error:` with an `NSURL` the pool owns.
  - Metal's compiler failed the whole test program as a dynamic library ("Compiler encountered an internal error"),
    though each construct in it compiles alone; R7 reads a program of its own, one kernel running each type's
    product from device memory.
- **Measured:** the seven matrix tests pass on the M1 Max and the M3 Max (f16 and bf16 native, i8 and b1 emulated),
  R7 finding `air.simdgroup_matrix_8x8_multiply_accumulate.v64f32.v64f16.v64f16.v64f32` and its bf16 form; CUDA's and
  the CPU's pass as before; `accel`'s GPU tests (the bit product on the new layout) pass on the M3.

### The V100 (2026-11-23)

- **Volta's unit is `mma.sync.m8n8k4`** (sm_70): f16 into f32 only, no bf16 or i8. Built as a form of its own
  (`UKL_MMA_VOLTA`): each of the warp's four quad pairs computes an 8×8 quadrant of the 16×16 fragment, four k4
  instructions a product. C takes `m8n8k4`'s layout of its f32 accumulators, for every type, as on Apple. The nine
  matrix tests pass on a V100 with it (R4 checks the layout; R7 finds `mma.sync.aligned.m8n8k4.row.col.f32.f16.f16.f32`),
  and CUDA's own tile call serves its f16.
- **The test picks the emulation** (`bench_matrix_products`; device time, median of 5; the tile calls as in step 7):

  | Type | The tile call with Volta's unit | The emulation's |
  |---|---|---|
  | f16 | 9.19 ms (the unit's, CUDA's own tile) | 12.46 ms |
  | bf16 | 16.65 ms | 12.39 ms |
  | i8 | 13.09 ms | 9.30 ms |
  | b1 | 5.23 ms | 2.19 ms |

  - Only f16's tile call is faster on the unit, 1.36×; in the products' rate, with a fragment of B loaded for each,
    it is 2.4× slower (2.90 against 1.22 ms): the unit's B is a lane's column, read element by element.
  - The layout of C is the backend's, one for every type. Volta's makes the emulated types 1.3-2.4× slower, b1 (the
    bit product's) the most.
  - So the V100 runs the emulation. Volta's form is kept behind `UKL_MMA_VOLTA_UNIT`, and the matrix tests take it
    with `RERAIL_MMA_VOLTA_UNIT=1` (both pass on the V100). A layout of C per type would let f16 take the unit
    without costing the others; no kernel needs it now.
- Turing (sm_75) has `m8n8k4` too, and is not tested: it takes the emulation.

### The bit product's reads (2026-11-23)

The 1-bit section left the tile's reads open, as the limit on the GB10 and the M1. Measured, they are neither machine's:

- **Built and measured,** in `bp_tile`: a 128×128 tile (each side's rows read half as often), whose owners hold
  blocks of 2×4 fragments (a step's 8 products from 6 fragment loads, where the 64 tile's owners take 2 from 3); and
  the 64 tile staging 16 words of K a step instead of 8 (half the barriers). Device time, `accel`'s price bench,
  back to back on one host:

  | Kernel | GB10 W1 | GB10 W5 | M1 Max W1 | M1 Max W5 |
  |---|---|---|---|---|
  | 64 tile, 8 words a step (kept) | 0.218-0.224 ms | 0.947-0.952 ms | 0.257 ms | 1.816 ms |
  | 128 tile, 2×4 blocks | 0.216 ms | 1.075 ms | 3.54 ms | 9.11 ms |
  | 64 tile, 16 words a step | 0.212-0.217 ms | 1.011-1.013 ms | | |

- **On the GB10, W5 is the unit's rate:** its 5.5·10^10 bit products at the b1 unit's own rate in registers
  (68-76 T a second, `bench_matrix_products`) are 0.72-0.80 ms, within 1.2-1.3× of the kernel. W1's products are
  0.06 ms of its 0.22; its 23 MB of counts written, and its 1,435 threadgroups of two steps each, are the rest.
- **On the M1 Max the larger tile is 3.5-14× slower:** its 19.4 KB of threadgroup memory leaves room for one
  threadgroup a core, where the 64 tile's 9.7 KB leaves three.
- So `bp_tile` stays as it was. The M1's gap of the 1-bit section (6-13%) is not its reads either; its compiled
  shader remains the way to it.

### Step 5, built (2026-11-23)

- **Intel's Xe2 (the Arc 140V) runs f16, bf16 and i8 on DPAS** (`UKL_MMA_INTEL`), through
  `cl_intel_subgroup_matrix_multiply_accumulate`:
  - the host turns it on (`UKL_OPENCL_INTEL_DPAS`) where the device has the extension and the kernels run on its
    sub-groups at 16 lanes, the width Xe2's functions take (`units`, in `compute_kernel_opencl`);
  - a product is two calls, C's rows 0-7 and 8-15: `intel_sub_group_f16_f16_matrix_mad_k16`, its bf16 form, and
    `intel_sub_group_i8_i8_matrix_mad_k32` (K 32, as on CUDA);
  - A: each lane holds its k of every row (a 16-bit float, or two bytes); B: each lane its column, eight words along
    K; each element read where it lies, any layout, device or threadgroup memory;
  - C's layout is DPAS's: a lane holds its column, every row (`UKL_MMA_RB` 16, `UKL_MMA_CB` 1). R4 checks it.
- **R7 reads Intel's binary** (`native_code` on OpenCL: the program built again and its binary's printable strings):
  each type's builtin is named in it, and the compiler's own metadata says `has_dpas: true` for the native form's
  kernel and not for the forced emulation's.
- **The shape only:** no tuning, and no workaround of Intel's compiler.
- **Measured:** the nine matrix tests pass on the Arc 140V (Windows, OpenCL 3.0 NEO), every float and i8 native, b1
  emulated.
- **Found on the way:** `rerail-facts` did not build on Windows (`Within::on_device`'s `cfg` left Windows out, where
  every caller had it), so no Windows test could run.

### Step 6, built (2026-11-23)

- **The Radeon 680M (gfx1035, OpenCL) runs the emulation,** every type, with RDNA2's dot products where the host
  turns them on (`UKL_OPENCL_AMD_DOT`): `v_dot4_i32_i8` for i8, `v_dot2_f32_f16` for f16. gfx1035 has no bf16 dot
  product; bf16 takes the reference.
- **Where they help, measured** (`bench_matrix_products`: 2,048 owners, 256 products each in registers, a fragment of
  B loaded for each; device time, median of 5). `UKL_DOT_REFERENCE` takes `<ukl/dot.h>`'s references instead:

  | Type | Dot products | References |
  |---|---|---|
  | i8 | 2.08 ms | 3.48 ms |
  | f16 | 4.15 ms | 4.20 ms |
  | bf16 | 4.24 ms (references) | 4.27 ms |

  - **i8 gains 1.67×:** one instruction for four byte products, where the reference extracts and multiplies each.
  - **f16 is the same either way:** the reference's conversions and fused multiply-adds cost no more there, and the
    product is bound by the emulation's shuffles, not its arithmetic.
  - So the dot products stay on, as they were.
- **Passing:** the nine matrix tests on the 680M (Windows, OpenCL 2.0 AMD-APP), every type emulated.
- **The measure itself:** a first version loaded B once and repeated the same product. The compiler folded the
  repeated integer products (an emulated b1 at 1.2 P multiply-adds a second on the GB10). Each repetition now loads B
  at another offset.

### Step 7, built (2026-11-23)

- **`ukl_gemm_tile_<T>` may be a backend's own.**
  - Every type has its reference, `ukl_gemm_tile_ref_<T>`: level 1's loop, fragment by fragment.
  - A backend that overrides the tile call says so, `UKL_MMA_TILE_OWN(T)` 1.
  - An override sums each element's products in the reference's order, so the two agree bit for bit.
- **CUDA's own** (f16, bf16 and i8) holds the owner's whole 64×64 block of C in registers: 16 fragments, 128 values
  a lane. Each K step then reads each of A's four row fragments and B's four column fragments once, where the
  reference reads each four times. This is the open item of the 1-bit section ("both sides' rows read once for
  several tiles"), for the tile call.
- **R9 and the override:** `matrix_tile_calls_equal_their_reference` runs the tile call and its reference on R2's and
  R3's inputs, in four layouts. They must agree bit for bit, and on R2's inputs be exact. On CUDA the three types
  must be the backend's own.
- **Measured** (`bench_matrix_products`; a GB10; 256 owners, each a 64×64 block over K 1024, device time, median of
  5):

  | Type | Reference | CUDA's own |
  |---|---|---|
  | f16 | 0.453 ms | 0.116 ms |
  | bf16 | 0.455 ms | 0.117 ms |
  | i8 | 0.310 ms | 0.107 ms |

- **Passing:** the nine matrix tests on CUDA, the CPU, the Arc's OpenCL and Metal (the M1 Max), the tile call being
  the reference everywhere but CUDA.

## Open

- **The first kernel that uses it** is the bit product (above). The learned cost model's trees on the GPU, and the
  embeddings, remain candidates for the float types. Their needs (an epilogue, a reduction by row) are what R4 and
  level 2 must serve.
- **AMD WMMA's native form** waits for an RDNA3 device. A HIP backend for AMD (decision 0440) would reach WMMA through
  the same intrinsics.
