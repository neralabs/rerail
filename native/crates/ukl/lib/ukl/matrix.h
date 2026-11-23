// ukl/matrix.h: matrix units through fragments a kernel writes once (decision 0617): the interface, its emulated
// form on every backend, and the native forms that have landed (NVIDIA's `mma.sync`: f16, bf16, i8 and b1).
//
// A fragment is a tile of a matrix held in registers by its owner: one SIMD group, or `UKL_MMA_GROUPS(T)` of them
// together. Each type has its shape, `C[M×N] += A[M×K] · B[K×N]`, as compile-time constants:
//
//   UKL_MMA_M(T), UKL_MMA_N(T), UKL_MMA_K(T)   the shape; M and N divide UKL_MMA_BLOCK (64)
//   UKL_MMA_GROUPS(T)                          the SIMD groups that own a fragment
//   UKL_MMA_NATIVE(T)                          1 where the device's unit runs it, 0 where it is emulated
//   UKL_MMA_PACK(T)                            T's elements in one element in memory (1; 64 for b1)
//
// `T` is `f16` or `bf16` (into f32), `i8` (into i32), or `b1` (into i32: `C += popcount(A & B)`, the bit product). In
// memory, a 16-bit float is its bits (`uint16_t`), an i8 an `int8_t`, a b1 element one bit of a `uint64_t` holding 64
// of them along K (lowest first), an accumulator a `float` or an `int32_t`. K counts T's elements (b1: bits); a
// pointer's offset and a stride count elements in memory (b1: 64-bit words).
//
// Every operation is convergent over the owner: every lane of each of its SIMD groups calls it, with its place
// (`UklMmaAt`, from `ukl_mma_at(lid)` in a 1-D threadgroup). A threadgroup holds whole owners; `ukl_mma_owner(lid)`
// says which. No operation names a lane's element: a kernel visits its elements with their coordinates
// (`UKL_MMA_EACH`), as Apple's and the M5's units allow.
//
//   ukl_mma_fill_c_<C>(&c, v, at)                        every element of C set to v (C: f32 or i32)
//   ukl_mma_load_a_<T>(&a, p, stride, layout, at)        A from memory at `p`, row-major or column-major
//   ukl_mma_load_b_<T>(&b, p, stride, layout, at)        B likewise; ..._c_<C> for C
//   ukl_mma_<T>(&c, &a, &b, at)                          c += a · b
//   ukl_mma_store_c_<C>(&c, p, stride, layout, at)       C into memory
//   UKL_MMA_EACH(c, at, row, col, v) { ... }             each element this thread holds: its row, column and value
//   ukl_gemm_tile_<T>(C, ldc, A, lda, B, ldb, K, ...)    level 2: a 64×64 block of C += A[64×K] · B[K×64]
//
// The loads and stores read device memory (`UKL_GLOBAL`); their `_local` forms read threadgroup memory. Each reads
// the memory where it lies: a fragment is registers (decision 0353's memory hierarchy, not a copy).
//
// C's layout is the backend's: each thread holds `UKL_MMA_RB` rows by `UKL_MMA_CB` columns of the 16×16 fragment,
// element `i · CB + j` at row `ukl_mma_row(i, at)` and column `ukl_mma_col(j, at)`.
// - NVIDIA sm_80 and later: `mma.sync`'s accumulator layout (rows g and g + 8, `g` the lane's quad; two columns in
//   each half of eight), so that a native product reads and writes C in place, whatever the type.
// - Elsewhere, a group of 16 lanes or more: rows and columns each, so that the emulation reads each of its rows of A
//   and columns of B once a k-word, not once an element. They interleave (a lane's columns `16 / CB` apart, its
//   rows `16 / RB`), so that lanes side by side hold columns side by side: a store of C writes whole runs of a row.
//   A narrower group's lanes hold rows and columns whose words lie in the same register of every lane (below).
//
// The emulated form, every type 16×16, is the library's reference:
// - f16, bf16 and i8 (K 16): A and B are packed words along K (two 16-bit floats or four bytes a word). Each SIMD
//   group of the owner holds the whole of A and B, k-major: word j of a lane is word `j · W + lane`, at k-word
//   `idx / 16` of row (A) or column (B) `idx % 16`. A product's lane reads its rows of A and columns of B from the
//   other lanes by shuffle, each from the same register in every lane, and adds them with `<ukl/dot.h>`'s dot
//   products: the device's instructions where it has them. Its sums are in k's order, the dot products' own
//   rounding: f32, or exact for i8.
// - b1 (K 64, one word; NVIDIA's unit's is 256): popcount on the threads. Each lane loads its own rows of A and
//   columns of B, where they lie (the lanes that share a row read the same words: a broadcast of the memory, not a
//   shuffle), and adds `popcount(a & b)` a 64-bit word at a time, which beat the int8 units where there is no 1-bit
//   unit (`tools/matrix-offload/RESULTS.md`, finding 2). Its K is one word so that a lane's words stay a few
//   registers, and its words are 64 bits: 32-bit ones took the M1 twice the loads, and twice the time.
//
// `UKL_MMA_FORCE_EMULATED` (a test's define) runs the emulation, in its own layout, where a native form exists
// (requirement R6).

#pragma once
#include <ukl/simd.h>
#include <ukl/dot.h>

#define UKL_MMA_BLOCK 64u
#define UKL_ROW_MAJOR 0u
#define UKL_COL_MAJOR 1u

// The SIMD groups an emulated owner spans: 1, or (a test's define) 2 or 4, so that the multi-group path is tested
// where no device has it yet (requirement R5). An owner of several groups is emulated on every device.
#ifndef UKL_MMA_EMULATED_GROUPS
#define UKL_MMA_EMULATED_GROUPS 1u
#endif

// NVIDIA's units, where they serve (sm_80 and later: `mma.sync`, f16 and bf16 into f32, i8 into i32, and 1-bit AND
// and POPC): one warp an owner.
#if defined(UKL_CUDA) && !defined(UKL_HIP) && !defined(UKL_MMA_FORCE_EMULATED) && UKL_SIMD_WIDTH == 32 &&           \
    UKL_MMA_EMULATED_GROUPS == 1 && __CUDA_ARCH__ >= 800
#define UKL_MMA_CUDA 1
#else
#define UKL_MMA_CUDA 0
#endif

// Apple's units (M1-M4: `simdgroup_matrix`, 8×8×8; f16, and bf16 from Metal 3.1, where `bfloat` came): one SIMD group
// an owner, a 16×16 fragment four of its matrices. (`__HAVE_BFLOAT__` is not defined at 3.1: the version is the test.)
#if defined(UKL_METAL) && !defined(UKL_MMA_FORCE_EMULATED) && UKL_SIMD_WIDTH == 32 && UKL_MMA_EMULATED_GROUPS == 1
#define UKL_MMA_APPLE 1
#else
#define UKL_MMA_APPLE 0
#endif
#if UKL_MMA_APPLE && __METAL_VERSION__ >= 310
#define UKL_MMA_APPLE_BF16 1
#else
#define UKL_MMA_APPLE_BF16 0
#endif

// Intel's units (Xe2, the Arc 140V: DPAS through `cl_intel_subgroup_matrix_multiply_accumulate`, which the host
// finds, `UKL_OPENCL_INTEL_DPAS`): f16, bf16 and i8, 8 rows by 16 columns a call at sub-groups of 16; one sub-group an
// owner.
#if defined(UKL_OPENCL) && defined(UKL_OPENCL_INTEL_DPAS) && defined(UKL_OPENCL_SUBGROUPS) && !defined(UKL_MMA_FORCE_EMULATED) && \
    UKL_SIMD_WIDTH == 16 && UKL_MMA_EMULATED_GROUPS == 1
#define UKL_MMA_INTEL 1
#else
#define UKL_MMA_INTEL 0
#endif

#define UKL_MMA_M(T) UKL_MMA_M_##T
#define UKL_MMA_N(T) UKL_MMA_N_##T
#define UKL_MMA_K(T) UKL_MMA_K_##T
#define UKL_MMA_GROUPS(T) UKL_MMA_GROUPS_##T
#define UKL_MMA_NATIVE(T) UKL_MMA_NATIVE_##T
#define UKL_MMA_PACK(T) UKL_MMA_PACK_##T

#define UKL_MMA_M_f16 16u
#define UKL_MMA_N_f16 16u
#define UKL_MMA_K_f16 16u
#define UKL_MMA_GROUPS_f16 UKL_MMA_EMULATED_GROUPS
#define UKL_MMA_NATIVE_f16 (UKL_MMA_CUDA || UKL_MMA_APPLE || UKL_MMA_INTEL)
#define UKL_MMA_PACK_f16 1u
#define UKL_MMA_M_bf16 16u
#define UKL_MMA_N_bf16 16u
#define UKL_MMA_K_bf16 16u
#define UKL_MMA_GROUPS_bf16 UKL_MMA_EMULATED_GROUPS
#define UKL_MMA_NATIVE_bf16 (UKL_MMA_CUDA || UKL_MMA_APPLE_BF16 || UKL_MMA_INTEL)
#define UKL_MMA_PACK_bf16 1u
#define UKL_MMA_M_i8 16u
#define UKL_MMA_N_i8 16u
// NVIDIA's and Intel's units' K (m16n8k32; DPAS's k32); the emulation's 16, four bytes a word.
#if UKL_MMA_CUDA || UKL_MMA_INTEL
#define UKL_MMA_K_i8 32u
#else
#define UKL_MMA_K_i8 16u
#endif
#define UKL_MMA_GROUPS_i8 UKL_MMA_EMULATED_GROUPS
#define UKL_MMA_NATIVE_i8 (UKL_MMA_CUDA || UKL_MMA_INTEL)
#define UKL_MMA_PACK_i8 1u
#define UKL_MMA_M_b1 16u
#define UKL_MMA_N_b1 16u
// NVIDIA's unit's K; the emulation's two words, so that a lane's rows of A and columns of B stay a few registers.
#if UKL_MMA_CUDA
#define UKL_MMA_K_b1 256u
#else
#define UKL_MMA_K_b1 64u
#endif
#define UKL_MMA_GROUPS_b1 UKL_MMA_EMULATED_GROUPS
#define UKL_MMA_NATIVE_b1 UKL_MMA_CUDA
#define UKL_MMA_PACK_b1 64u

// The owner's lanes, C's elements a lane, A's and B's words a lane for the shuffled types (16 rows of 8 words for a
// 16-bit type, of 4 for bytes), b1's 64-bit k-words.
#define UKL_MMA_LANES (UKL_MMA_EMULATED_GROUPS * UKL_SIMD_WIDTH)
#define UKL_MMA_C_PER (256u / UKL_MMA_LANES)
#define UKL_MMA_WORDS_16 (128u / UKL_SIMD_WIDTH)
#define UKL_MMA_WORDS_8 (64u / UKL_SIMD_WIDTH)
#define UKL_MMA_KW_b1 (UKL_MMA_K_b1 / 64u)

// C's rows (`RB`) and columns (`CB`) a lane.
#if UKL_MMA_CUDA || UKL_MMA_APPLE
#define UKL_MMA_RB 2u
#define UKL_MMA_CB 4u
#elif UKL_MMA_INTEL
// DPAS's: a lane holds its column, every row.
#define UKL_MMA_RB 16u
#define UKL_MMA_CB 1u
#elif UKL_SIMD_WIDTH >= 16
#define UKL_MMA_RB (UKL_MMA_C_PER >= 128u ? 16u : UKL_MMA_C_PER >= 32u ? 8u : UKL_MMA_C_PER >= 8u ? 4u : UKL_MMA_C_PER >= 2u ? 2u : 1u)
#define UKL_MMA_CB (UKL_MMA_C_PER / UKL_MMA_RB)
#elif UKL_MMA_LANES < 16
#define UKL_MMA_RB 16u
#define UKL_MMA_CB (16u / UKL_MMA_LANES)
#else
#define UKL_MMA_RB (256u / UKL_MMA_LANES)
#define UKL_MMA_CB 1u
#endif

// Requirement R1: every shape's M and N divide the block, and K whole words; an owner's lanes divide C's elements,
// each a lane's rows by its columns.
UKL_STATIC_ASSERT(UKL_MMA_BLOCK % UKL_MMA_M_f16 == 0u && UKL_MMA_BLOCK % UKL_MMA_N_f16 == 0u && UKL_MMA_BLOCK % UKL_MMA_K_f16 == 0u, "f16's shape divides 64");
UKL_STATIC_ASSERT(UKL_MMA_BLOCK % UKL_MMA_M_bf16 == 0u && UKL_MMA_BLOCK % UKL_MMA_N_bf16 == 0u && UKL_MMA_BLOCK % UKL_MMA_K_bf16 == 0u, "bf16's shape divides 64");
UKL_STATIC_ASSERT(UKL_MMA_BLOCK % UKL_MMA_M_i8 == 0u && UKL_MMA_BLOCK % UKL_MMA_N_i8 == 0u && UKL_MMA_BLOCK % UKL_MMA_K_i8 == 0u, "i8's shape divides 64");
UKL_STATIC_ASSERT(UKL_MMA_BLOCK % UKL_MMA_M_b1 == 0u && UKL_MMA_BLOCK % UKL_MMA_N_b1 == 0u && UKL_MMA_K_b1 % 64u == 0u, "b1's shape divides 64, its K whole words");
UKL_STATIC_ASSERT(UKL_MMA_LANES <= 256u && 256u % UKL_MMA_LANES == 0u, "an emulated owner's lanes divide a fragment's 256 elements");
UKL_STATIC_ASSERT(UKL_MMA_RB * UKL_MMA_CB * UKL_MMA_LANES == 256u, "a lane's rows by its columns are its elements");

// A thread's place in its owner: its lane, and which of the owner's SIMD groups it is in.
typedef struct UklMmaAt {
    uint32_t lane;
    uint32_t part;
} UklMmaAt;

// The place of thread `lid` of a 1-D threadgroup, and its owner.
UKL_INLINE UklMmaAt ukl_mma_at(uint32_t lid)
{
    UklMmaAt at;
    at.lane = lid % UKL_SIMD_WIDTH;
    at.part = (lid / UKL_SIMD_WIDTH) % UKL_MMA_EMULATED_GROUPS;
    return at;
}
UKL_INLINE uint32_t ukl_mma_owner(uint32_t lid) { return lid / UKL_MMA_LANES; }

// Row `i` (of `RB`) and column `j` (of `CB`) of this thread's elements of C. A narrow group's (under 16 lanes) are
// those whose words, in the shuffled types' packing, lie in the same register of every lane of the group: all 16
// rows and interleaved columns (an owner under 16 lanes), or a column a lane (16 or more).
UKL_INLINE uint32_t ukl_mma_row(uint32_t i, UklMmaAt at)
{
#if UKL_MMA_CUDA
    return (at.lane >> 2) + 8u * i;
#elif UKL_MMA_INTEL
    return i;
#elif UKL_MMA_APPLE
    // `thread_elements()`'s: a lane's two elements of an 8×8 matrix lie in one row, side by side.
    return ((at.lane >> 2) & 4u) + ((at.lane >> 1) & 3u) + 8u * i;
#elif UKL_SIMD_WIDTH >= 16
    return (at.part * UKL_SIMD_WIDTH + at.lane) / (16u / UKL_MMA_CB) + (16u / UKL_MMA_RB) * i;
#else
    return ((at.part * UKL_SIMD_WIDTH + at.lane) / (16u / UKL_MMA_CB)) * UKL_MMA_RB + i;
#endif
}
UKL_INLINE uint32_t ukl_mma_col(uint32_t j, UklMmaAt at)
{
#if UKL_MMA_CUDA
    return 8u * (j >> 1) + 2u * (at.lane & 3u) + (j & 1u);
#elif UKL_MMA_INTEL
    (void)j;
    return at.lane;
#elif UKL_MMA_APPLE
    return 8u * (j >> 1) + ((at.lane >> 2) & 2u) * 2u + (at.lane & 1u) * 2u + (j & 1u);
#elif UKL_SIMD_WIDTH < 16 && UKL_MMA_LANES < 16
    return UKL_MMA_LANES * j + at.part * UKL_SIMD_WIDTH + at.lane;
#elif UKL_SIMD_WIDTH >= 16
    return (at.part * UKL_SIMD_WIDTH + at.lane) % (16u / UKL_MMA_CB) + (16u / UKL_MMA_CB) * j;
#else
    return ((at.part * UKL_SIMD_WIDTH + at.lane) % (16u / UKL_MMA_CB)) * UKL_MMA_CB + j;
#endif
}

#if UKL_MMA_CUDA
// `mma.sync`'s registers (g the lane's quad, t its place in it), each word two 16-bit floats or four bytes along K,
// the lowest k lowest: A's rows g (words 0 and 2) and g + 8 (1 and 3) at k from `P · t` (0, 1) and half K on (2, 3);
// B's columns g of each half of eight (words 0, 1 and 2, 3), at k from `P · t` and half K on.
typedef struct ukl_mma_a_f16 { uint32_t w[4]; } ukl_mma_a_f16;
typedef struct ukl_mma_b_f16 { uint32_t w[4]; } ukl_mma_b_f16;
typedef struct ukl_mma_a_bf16 { uint32_t w[4]; } ukl_mma_a_bf16;
typedef struct ukl_mma_b_bf16 { uint32_t w[4]; } ukl_mma_b_bf16;
typedef struct ukl_mma_a_i8 { uint32_t w[4]; } ukl_mma_a_i8;
typedef struct ukl_mma_b_i8 { uint32_t w[4]; } ukl_mma_b_i8;
#elif UKL_MMA_INTEL
// DPAS's operands: A's 16 rows, each this lane's k (a 16-bit float, or two bytes: K 32 over 16 lanes); B this lane's
// column, 8 words along K (two 16-bit floats or four bytes each, the lowest k lowest).
typedef struct ukl_mma_a_f16 { uint16_t w[16]; } ukl_mma_a_f16;
typedef struct ukl_mma_b_f16 { uint32_t w[8]; } ukl_mma_b_f16;
typedef struct ukl_mma_a_bf16 { uint16_t w[16]; } ukl_mma_a_bf16;
typedef struct ukl_mma_b_bf16 { uint32_t w[8]; } ukl_mma_b_bf16;
typedef struct ukl_mma_a_i8 { uint16_t w[16]; } ukl_mma_a_i8;
typedef struct ukl_mma_b_i8 { uint32_t w[8]; } ukl_mma_b_i8;
#else
// Apple's: A's 8×8 matrices row tile by k tile (`2 · ti + tk`), B's k tile by column tile (`2 · tk + tj`).
#if UKL_MMA_APPLE
typedef struct ukl_mma_a_f16 { simdgroup_half8x8 m[4]; } ukl_mma_a_f16;
typedef struct ukl_mma_b_f16 { simdgroup_half8x8 m[4]; } ukl_mma_b_f16;
#else
typedef struct ukl_mma_a_f16 { uint32_t w[UKL_MMA_WORDS_16]; } ukl_mma_a_f16;
typedef struct ukl_mma_b_f16 { uint32_t w[UKL_MMA_WORDS_16]; } ukl_mma_b_f16;
#endif
#if UKL_MMA_APPLE_BF16
typedef struct ukl_mma_a_bf16 { simdgroup_bfloat8x8 m[4]; } ukl_mma_a_bf16;
typedef struct ukl_mma_b_bf16 { simdgroup_bfloat8x8 m[4]; } ukl_mma_b_bf16;
#else
typedef struct ukl_mma_a_bf16 { uint32_t w[UKL_MMA_WORDS_16]; } ukl_mma_a_bf16;
typedef struct ukl_mma_b_bf16 { uint32_t w[UKL_MMA_WORDS_16]; } ukl_mma_b_bf16;
#endif
typedef struct ukl_mma_a_i8 { uint32_t w[UKL_MMA_WORDS_8]; } ukl_mma_a_i8;
typedef struct ukl_mma_b_i8 { uint32_t w[UKL_MMA_WORDS_8]; } ukl_mma_b_i8;
#endif
#if UKL_MMA_CUDA
// `mma.sync`'s registers: A's rows g and g + 8 at 32-bit k-words q and q + 4 (q the lane's place in its quad); each
// half of B's columns, its column g at 32-bit k-words q and q + 4. Each is a half of a word in memory, read there.
typedef struct ukl_mma_a_b1 { uint32_t w[4]; } ukl_mma_a_b1;
typedef struct ukl_mma_b_b1 { uint32_t w[4]; } ukl_mma_b_b1;
#else
// The lane's own rows of A and columns of B, each its k-words.
typedef struct ukl_mma_a_b1 { uint64_t w[UKL_MMA_RB * UKL_MMA_KW_b1]; } ukl_mma_a_b1;
typedef struct ukl_mma_b_b1 { uint64_t w[UKL_MMA_CB * UKL_MMA_KW_b1]; } ukl_mma_b_b1;
#endif
// The accumulators' elements are `x`: `UKL_MMA_EACH` names them.
typedef struct ukl_mma_c_f32 { float x[UKL_MMA_C_PER]; } ukl_mma_c_f32;
typedef struct ukl_mma_c_i32 { int32_t x[UKL_MMA_C_PER]; } ukl_mma_c_i32;

// Each element of `c` this thread holds, as `v` (read, and written back after the body), at `row` and `col` of the
// fragment. `break` and `continue` leave the element's body, its value kept.
#if defined(__cplusplus)
#define UKL_MMA_ELEM_TYPE_(x) decltype(x)
#else
#define UKL_MMA_ELEM_TYPE_(x) __typeof__(x)
#endif
#define UKL_MMA_EACH(c, at, row, col, v)                                                                                \
    for (uint32_t __ukl_mma_e = 0u; __ukl_mma_e < UKL_MMA_C_PER; __ukl_mma_e++)                                         \
        for (uint32_t row = ukl_mma_row(__ukl_mma_e / UKL_MMA_CB, (at)), col = ukl_mma_col(__ukl_mma_e % UKL_MMA_CB, (at)), \
                      __ukl_mma_once = 1u;                                                                              \
             __ukl_mma_once != 0u; __ukl_mma_once = 0u)                                                                 \
            for (UKL_MMA_ELEM_TYPE_((c).x[0]) v = (c).x[__ukl_mma_e]; __ukl_mma_once != 0u;                             \
                 (c).x[__ukl_mma_e] = v, __ukl_mma_once = 0u)

// ---- C: fill, load, store (`C` f32 or i32, `CT` its element type). ----

#define UKL_MMA_C_OPS(C, CT, SPACE, SUF)                                                                                \
    UKL_INLINE void ukl_mma_load_c_##C##SUF(UKL_PRIVATE ukl_mma_c_##C* c, SPACE const CT* p, uint32_t stride,           \
                                            uint32_t layout, UklMmaAt at)                                               \
    {                                                                                                                   \
        for (uint32_t i = 0u; i < UKL_MMA_RB; i++)                                                                      \
            for (uint32_t j = 0u; j < UKL_MMA_CB; j++) {                                                                \
                uint32_t r = ukl_mma_row(i, at), k = ukl_mma_col(j, at);                                                \
                c->x[i * UKL_MMA_CB + j] = layout == UKL_ROW_MAJOR ? p[r * stride + k] : p[k * stride + r];            \
            }                                                                                                           \
    }                                                                                                                   \
    UKL_INLINE void ukl_mma_store_c_##C##SUF(UKL_PRIVATE const ukl_mma_c_##C* c, SPACE CT* p, uint32_t stride,          \
                                             uint32_t layout, UklMmaAt at)                                              \
    {                                                                                                                   \
        for (uint32_t i = 0u; i < UKL_MMA_RB; i++)                                                                      \
            for (uint32_t j = 0u; j < UKL_MMA_CB; j++) {                                                                \
                uint32_t r = ukl_mma_row(i, at), k = ukl_mma_col(j, at);                                                \
                if (layout == UKL_ROW_MAJOR)                                                                            \
                    p[r * stride + k] = c->x[i * UKL_MMA_CB + j];                                                       \
                else                                                                                                    \
                    p[k * stride + r] = c->x[i * UKL_MMA_CB + j];                                                       \
            }                                                                                                           \
    }

UKL_INLINE void ukl_mma_fill_c_f32(UKL_PRIVATE ukl_mma_c_f32* c, float v, UklMmaAt at)
{
    (void)at;
    for (uint32_t e = 0u; e < UKL_MMA_C_PER; e++)
        c->x[e] = v;
}
UKL_INLINE void ukl_mma_fill_c_i32(UKL_PRIVATE ukl_mma_c_i32* c, int32_t v, UklMmaAt at)
{
    (void)at;
    for (uint32_t e = 0u; e < UKL_MMA_C_PER; e++)
        c->x[e] = v;
}
UKL_MMA_C_OPS(f32, float, UKL_GLOBAL, )
UKL_MMA_C_OPS(f32, float, UKL_LOCAL, _local)
UKL_MMA_C_OPS(i32, int32_t, UKL_LOCAL, _local)
UKL_MMA_C_OPS(i32, int32_t, UKL_GLOBAL, )

// ---- A and B of the shuffled types: loads of `T`'s elements (`ET` in memory, `P` a word, `BITS` each), packed
// along K. ----

#define UKL_MMA_AB_LOADS(T, ET, UT, P, BITS, WORDS, SPACE, SUF)                                                         \
    UKL_INLINE void ukl_mma_load_a_##T##SUF(UKL_PRIVATE ukl_mma_a_##T* a, SPACE const ET* p, uint32_t stride,           \
                                            uint32_t layout, UklMmaAt at)                                               \
    {                                                                                                                   \
        for (uint32_t j = 0u; j < WORDS; j++) {                                                                         \
            uint32_t i = j * UKL_SIMD_WIDTH + at.lane, kw = i >> 4, r = i & 15u, w = 0u;                                \
            for (uint32_t t = 0u; t < P; t++) {                                                                         \
                uint32_t k = kw * P + t;                                                                                \
                w |= (uint32_t)(UT)(layout == UKL_ROW_MAJOR ? p[r * stride + k] : p[k * stride + r]) << (t * BITS);    \
            }                                                                                                           \
            a->w[j] = w;                                                                                                \
        }                                                                                                               \
    }                                                                                                                   \
    UKL_INLINE void ukl_mma_load_b_##T##SUF(UKL_PRIVATE ukl_mma_b_##T* b, SPACE const ET* p, uint32_t stride,           \
                                            uint32_t layout, UklMmaAt at)                                               \
    {                                                                                                                   \
        for (uint32_t j = 0u; j < WORDS; j++) {                                                                         \
            uint32_t i = j * UKL_SIMD_WIDTH + at.lane, kw = i >> 4, c = i & 15u, w = 0u;                                \
            for (uint32_t t = 0u; t < P; t++) {                                                                         \
                uint32_t k = kw * P + t;                                                                                \
                w |= (uint32_t)(UT)(layout == UKL_ROW_MAJOR ? p[k * stride + c] : p[c * stride + k]) << (t * BITS);    \
            }                                                                                                           \
            b->w[j] = w;                                                                                                \
        }                                                                                                               \
    }

// Word `i` of a fragment's `n` words, `i` the same in every lane of the group but known only at run time (a narrow
// group's): picked by value, so the words stay registers.
UKL_INLINE uint32_t ukl_mma_word_(UKL_PRIVATE const uint32_t* w, uint32_t n, uint32_t i)
{
    uint32_t v = w[0];
    for (uint32_t j = 1u; j < n; j++)
        v = j == i ? w[j] : v;
    return v;
}

// ---- The shuffled product: `c += a · b`, by shuffles and `DOT` (`KW` k-words). Each k-word: the lane's rows of A
// and columns of B read by shuffle, once each, then every element's dot product. ----

#if UKL_SIMD_WIDTH >= 16
// A row's or a column's word of k-word `kw` lies in register `16 · kw / W` of lane `16 · kw % W` plus the row or
// column: a constant register.
#define UKL_MMA_PRODUCT(T, C, CT, KW, WORDS, DOT)                                                                       \
    UKL_INLINE void ukl_mma_##T(UKL_PRIVATE ukl_mma_c_##C* c, UKL_PRIVATE const ukl_mma_a_##T* a,                       \
                                UKL_PRIVATE const ukl_mma_b_##T* b, UklMmaAt at __UKL_PARAMS)                           \
    {                                                                                                                   \
        for (uint32_t kw = 0u; kw < KW; kw++) {                                                                         \
            uint32_t ar[UKL_MMA_RB], bc[UKL_MMA_CB];                                                                    \
            for (uint32_t i = 0u; i < UKL_MMA_RB; i++)                                                                  \
                ar[i] = ukl_sg_shuffle(a->w[(16u * kw) / UKL_SIMD_WIDTH], (16u * kw) % UKL_SIMD_WIDTH + ukl_mma_row(i, at) __UKL_ARGS); \
            for (uint32_t j = 0u; j < UKL_MMA_CB; j++)                                                                  \
                bc[j] = ukl_sg_shuffle(b->w[(16u * kw) / UKL_SIMD_WIDTH], (16u * kw) % UKL_SIMD_WIDTH + ukl_mma_col(j, at) __UKL_ARGS); \
            for (uint32_t i = 0u; i < UKL_MMA_RB; i++)                                                                  \
                for (uint32_t j = 0u; j < UKL_MMA_CB; j++)                                                              \
                    c->x[i * UKL_MMA_CB + j] = DOT(ar[i], bc[j], c->x[i * UKL_MMA_CB + j]);                             \
        }                                                                                                               \
    }
#else
// A narrow group (fewer than 16 lanes): the register of a row's or a column's word is the same in every lane of the
// group (the layout's choice), known at run time where the owner spans groups: picked by value.
#define UKL_MMA_PRODUCT(T, C, CT, KW, WORDS, DOT)                                                                       \
    UKL_INLINE void ukl_mma_##T(UKL_PRIVATE ukl_mma_c_##C* c, UKL_PRIVATE const ukl_mma_a_##T* a,                       \
                                UKL_PRIVATE const ukl_mma_b_##T* b, UklMmaAt at __UKL_PARAMS)                           \
    {                                                                                                                   \
        for (uint32_t kw = 0u; kw < KW; kw++) {                                                                         \
            uint32_t ar[UKL_MMA_RB], bc[UKL_MMA_CB];                                                                    \
            for (uint32_t i = 0u; i < UKL_MMA_RB; i++) {                                                                \
                uint32_t ia = 16u * kw + ukl_mma_row(i, at);                                                            \
                ar[i] = ukl_sg_shuffle(ukl_mma_word_(a->w, WORDS, ia / UKL_SIMD_WIDTH), ia % UKL_SIMD_WIDTH __UKL_ARGS); \
            }                                                                                                           \
            for (uint32_t j = 0u; j < UKL_MMA_CB; j++) {                                                                \
                uint32_t ib = 16u * kw + ukl_mma_col(j, at);                                                            \
                bc[j] = ukl_sg_shuffle(ukl_mma_word_(b->w, WORDS, ib / UKL_SIMD_WIDTH), ib % UKL_SIMD_WIDTH __UKL_ARGS); \
            }                                                                                                           \
            for (uint32_t i = 0u; i < UKL_MMA_RB; i++)                                                                  \
                for (uint32_t j = 0u; j < UKL_MMA_CB; j++)                                                              \
                    c->x[i * UKL_MMA_CB + j] = DOT(ar[i], bc[j], c->x[i * UKL_MMA_CB + j]);                             \
        }                                                                                                               \
    }
#endif

// ---- b1: the bit product. ----

#if UKL_MMA_CUDA
#define UKL_MMA_B1_LOADS(SPACE, SUF)                                                                                    \
    UKL_INLINE void ukl_mma_load_a_b1##SUF(UKL_PRIVATE ukl_mma_a_b1* a, SPACE const uint64_t* p, uint32_t stride,       \
                                           uint32_t layout, UklMmaAt at)                                                \
    {                                                                                                                   \
        SPACE const uint32_t* h = (SPACE const uint32_t*)p;                                                             \
        for (uint32_t q = 0u; q < 4u; q++) {                                                                            \
            uint32_t r = (at.lane >> 2) + 8u * (q & 1u), kw = (at.lane & 3u) + 4u * (q >> 1);                           \
            a->w[q] = h[2u * (layout == UKL_ROW_MAJOR ? r * stride + kw / 2u : (kw / 2u) * stride + r) + (kw & 1u)];    \
        }                                                                                                               \
    }                                                                                                                   \
    UKL_INLINE void ukl_mma_load_b_b1##SUF(UKL_PRIVATE ukl_mma_b_b1* b, SPACE const uint64_t* p, uint32_t stride,       \
                                           uint32_t layout, UklMmaAt at)                                                \
    {                                                                                                                   \
        SPACE const uint32_t* h = (SPACE const uint32_t*)p;                                                             \
        for (uint32_t q = 0u; q < 4u; q++) {                                                                            \
            uint32_t c = 8u * (q >> 1) + (at.lane >> 2), kw = (at.lane & 3u) + 4u * (q & 1u);                           \
            b->w[q] = h[2u * (layout == UKL_ROW_MAJOR ? (kw / 2u) * stride + c : c * stride + kw / 2u) + (kw & 1u)];    \
        }                                                                                                               \
    }

// Two `mma.sync` m16n8k256, one for each half of C's columns, accumulating in place.
UKL_INLINE void ukl_mma_b1(UKL_PRIVATE ukl_mma_c_i32* c, UKL_PRIVATE const ukl_mma_a_b1* a, UKL_PRIVATE const ukl_mma_b_b1* b,
                           UklMmaAt at __UKL_PARAMS)
{
    (void)at;
    for (uint32_t h = 0u; h < 2u; h++)
        asm volatile("mma.sync.aligned.m16n8k256.row.col.s32.b1.b1.s32.and.popc {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};"
                     : "+r"(c->x[2u * h]), "+r"(c->x[2u * h + 1u]), "+r"(c->x[4u + 2u * h]), "+r"(c->x[5u + 2u * h])
                     : "r"(a->w[0]), "r"(a->w[1]), "r"(a->w[2]), "r"(a->w[3]), "r"(b->w[2u * h]), "r"(b->w[2u * h + 1u]));
}
#else
#define UKL_MMA_B1_LOADS(SPACE, SUF)                                                                                    \
    UKL_INLINE void ukl_mma_load_a_b1##SUF(UKL_PRIVATE ukl_mma_a_b1* a, SPACE const uint64_t* p, uint32_t stride,       \
                                           uint32_t layout, UklMmaAt at)                                                \
    {                                                                                                                   \
        for (uint32_t i = 0u; i < UKL_MMA_RB; i++) {                                                                    \
            uint32_t r = ukl_mma_row(i, at);                                                                            \
            for (uint32_t kw = 0u; kw < UKL_MMA_KW_b1; kw++)                                                            \
                a->w[i * UKL_MMA_KW_b1 + kw] = layout == UKL_ROW_MAJOR ? p[r * stride + kw] : p[kw * stride + r];       \
        }                                                                                                               \
    }                                                                                                                   \
    UKL_INLINE void ukl_mma_load_b_b1##SUF(UKL_PRIVATE ukl_mma_b_b1* b, SPACE const uint64_t* p, uint32_t stride,       \
                                           uint32_t layout, UklMmaAt at)                                                \
    {                                                                                                                   \
        for (uint32_t j = 0u; j < UKL_MMA_CB; j++) {                                                                    \
            uint32_t c = ukl_mma_col(j, at);                                                                            \
            for (uint32_t kw = 0u; kw < UKL_MMA_KW_b1; kw++)                                                            \
                b->w[j * UKL_MMA_KW_b1 + kw] = layout == UKL_ROW_MAJOR ? p[kw * stride + c] : p[c * stride + kw];       \
        }                                                                                                               \
    }

UKL_INLINE void ukl_mma_b1(UKL_PRIVATE ukl_mma_c_i32* c, UKL_PRIVATE const ukl_mma_a_b1* a, UKL_PRIVATE const ukl_mma_b_b1* b,
                           UklMmaAt at __UKL_PARAMS)
{
    (void)at;
    for (uint32_t kw = 0u; kw < UKL_MMA_KW_b1; kw++)
        for (uint32_t i = 0u; i < UKL_MMA_RB; i++)
            for (uint32_t j = 0u; j < UKL_MMA_CB; j++)
                c->x[i * UKL_MMA_CB + j] += (int32_t)ukl_popcount64(a->w[i * UKL_MMA_KW_b1 + kw] & b->w[j * UKL_MMA_KW_b1 + kw]);
}
#endif

// ---- Level 2: one owner's 64×64 block of C (at `C`, `lc` its layout) += A[64×K] · B[K×64], from device memory. ----
// The reference (`ukl_gemm_tile_ref_<T>`) is level 1's loop, fragment by fragment; a backend may override
// `ukl_gemm_tile_<T>` (decision 0617 §5), and says so (`UKL_MMA_TILE_OWN(T)`, 1). An override sums each element's
// products in the reference's order, so the two agree bit for bit.

#define UKL_MMA_TILE_REF(T, C, CT, ET)                                                                                  \
    UKL_INLINE void ukl_gemm_tile_ref_##T(UKL_GLOBAL CT* Cp, uint32_t ldc, uint32_t lc, UKL_GLOBAL const ET* A,         \
                                      uint32_t lda, uint32_t la, UKL_GLOBAL const ET* B, uint32_t ldb, uint32_t lb,     \
                                      uint32_t K, UklMmaAt at __UKL_PARAMS)                                             \
    {                                                                                                                   \
        for (uint32_t i = 0u; i < UKL_MMA_BLOCK / UKL_MMA_M_##T; i++)                                                   \
            for (uint32_t j = 0u; j < UKL_MMA_BLOCK / UKL_MMA_N_##T; j++) {                                             \
                uint32_t ci = i * UKL_MMA_M_##T, cj = j * UKL_MMA_N_##T;                                                \
                uint32_t co = lc == UKL_ROW_MAJOR ? ci * ldc + cj : cj * ldc + ci;                                      \
                ukl_mma_c_##C c;                                                                                        \
                ukl_mma_load_c_##C(&c, Cp + co, ldc, lc, at);                                                           \
                for (uint32_t k = 0u; k < K; k += UKL_MMA_K_##T) {                                                      \
                    uint32_t km = k / UKL_MMA_PACK_##T;                                                                 \
                    ukl_mma_a_##T a;                                                                                    \
                    ukl_mma_b_##T b;                                                                                    \
                    ukl_mma_load_a_##T(&a, A + (la == UKL_ROW_MAJOR ? ci * lda + km : km * lda + ci), lda, la, at);     \
                    ukl_mma_load_b_##T(&b, B + (lb == UKL_ROW_MAJOR ? km * ldb + cj : cj * ldb + km), ldb, lb, at);     \
                    ukl_mma_##T(&c, &a, &b, at __UKL_ARGS);                                                             \
                }                                                                                                       \
                ukl_mma_store_c_##C(&c, Cp + co, ldc, lc, at);                                                          \
            }                                                                                                           \
    }

#define UKL_MMA_TILE_PARAMS(CT, ET)                                                                                     \
    UKL_GLOBAL CT *Cp, uint32_t ldc, uint32_t lc, UKL_GLOBAL const ET *A, uint32_t lda, uint32_t la,                    \
        UKL_GLOBAL const ET *B, uint32_t ldb, uint32_t lb, uint32_t K, UklMmaAt at __UKL_PARAMS

// The reference and the tile call that is it.
#define UKL_MMA_TILE(T, C, CT, ET)                                                                                      \
    UKL_MMA_TILE_REF(T, C, CT, ET)                                                                                      \
    UKL_INLINE void ukl_gemm_tile_##T(UKL_MMA_TILE_PARAMS(CT, ET))                                                      \
    {                                                                                                                   \
        ukl_gemm_tile_ref_##T(Cp, ldc, lc, A, lda, la, B, ldb, lb, K, at __UKL_ARGS);                                   \
    }

// Which types' tile calls a backend overrides.
#if UKL_MMA_CUDA
#define UKL_MMA_TILE_OWN_f16 1u
#define UKL_MMA_TILE_OWN_bf16 1u
#define UKL_MMA_TILE_OWN_i8 1u
#else
#define UKL_MMA_TILE_OWN_f16 0u
#define UKL_MMA_TILE_OWN_bf16 0u
#define UKL_MMA_TILE_OWN_i8 0u
#endif
#define UKL_MMA_TILE_OWN_b1 0u
#define UKL_MMA_TILE_OWN(T) UKL_MMA_TILE_OWN_##T

#if UKL_MMA_CUDA
// ---- NVIDIA's f16, bf16 and i8: `mma.sync` registers, each a word of `P` elements read where they lie (`KH` half of
// the type's K). ----

#define UKL_MMA_CUDA_LOADS(T, ET, UT, P, BITS, KH, SPACE, SUF)                                                          \
    UKL_INLINE void ukl_mma_load_a_##T##SUF(UKL_PRIVATE ukl_mma_a_##T* a, SPACE const ET* p, uint32_t stride,           \
                                            uint32_t layout, UklMmaAt at)                                               \
    {                                                                                                                   \
        for (uint32_t q = 0u; q < 4u; q++) {                                                                            \
            uint32_t r = (at.lane >> 2) + 8u * (q & 1u), k0 = P * (at.lane & 3u) + KH * (q >> 1), w = 0u;               \
            for (uint32_t t = 0u; t < P; t++)                                                                           \
                w |= (uint32_t)(UT)(layout == UKL_ROW_MAJOR ? p[r * stride + k0 + t] : p[(k0 + t) * stride + r]) << (t * BITS); \
            a->w[q] = w;                                                                                                \
        }                                                                                                               \
    }                                                                                                                   \
    UKL_INLINE void ukl_mma_load_b_##T##SUF(UKL_PRIVATE ukl_mma_b_##T* b, SPACE const ET* p, uint32_t stride,           \
                                            uint32_t layout, UklMmaAt at)                                               \
    {                                                                                                                   \
        for (uint32_t q = 0u; q < 4u; q++) {                                                                            \
            uint32_t c = 8u * (q >> 1) + (at.lane >> 2), k0 = P * (at.lane & 3u) + KH * (q & 1u), w = 0u;               \
            for (uint32_t t = 0u; t < P; t++)                                                                           \
                w |= (uint32_t)(UT)(layout == UKL_ROW_MAJOR ? p[(k0 + t) * stride + c] : p[c * stride + k0 + t]) << (t * BITS); \
            b->w[q] = w;                                                                                                \
        }                                                                                                               \
    }

// Two `mma.sync` (`SHAPE`, `TYPES`), one for each half of C's columns, accumulating in place (`R` C's constraint).
#define UKL_MMA_CUDA_PRODUCT(T, C, SHAPE, TYPES, R)                                                                     \
    UKL_INLINE void ukl_mma_##T(UKL_PRIVATE ukl_mma_c_##C* c, UKL_PRIVATE const ukl_mma_a_##T* a,                       \
                                UKL_PRIVATE const ukl_mma_b_##T* b, UklMmaAt at __UKL_PARAMS)                           \
    {                                                                                                                   \
        (void)at;                                                                                                       \
        for (uint32_t h = 0u; h < 2u; h++)                                                                              \
            asm volatile("mma.sync.aligned." SHAPE ".row.col." TYPES " {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};" \
                         : R(c->x[2u * h]), R(c->x[2u * h + 1u]), R(c->x[4u + 2u * h]), R(c->x[5u + 2u * h])            \
                         : "r"(a->w[0]), "r"(a->w[1]), "r"(a->w[2]), "r"(a->w[3]), "r"(b->w[2u * h]), "r"(b->w[2u * h + 1u])); \
    }

// CUDA's tile call: the owner's whole 64×64 block of C in registers (16 fragments, 128 a lane), so that each K step
// reads each of A's four row fragments and B's four column fragments once, where the reference reads each four times
// (one a fragment of C). Each element's products are summed in the reference's order.
#define UKL_MMA_CUDA_TILE(T, C, CT, ET)                                                                                 \
    UKL_MMA_TILE_REF(T, C, CT, ET)                                                                                      \
    UKL_INLINE void ukl_gemm_tile_##T(UKL_MMA_TILE_PARAMS(CT, ET))                                                      \
    {                                                                                                                   \
        ukl_mma_c_##C c[16];                                                                                            \
        _Pragma("unroll") for (uint32_t f = 0u; f < 16u; f++) {                                                         \
            uint32_t ci = 16u * (f >> 2), cj = 16u * (f & 3u);                                                          \
            ukl_mma_load_c_##C(&c[f], Cp + (lc == UKL_ROW_MAJOR ? ci * ldc + cj : cj * ldc + ci), ldc, lc, at);         \
        }                                                                                                               \
        for (uint32_t k = 0u; k < K; k += UKL_MMA_K_##T) {                                                              \
            uint32_t km = k / UKL_MMA_PACK_##T;                                                                         \
            ukl_mma_b_##T b[4];                                                                                         \
            _Pragma("unroll") for (uint32_t j = 0u; j < 4u; j++)                                                        \
                ukl_mma_load_b_##T(&b[j], B + (lb == UKL_ROW_MAJOR ? km * ldb + 16u * j : 16u * j * ldb + km), ldb, lb, at); \
            _Pragma("unroll") for (uint32_t i = 0u; i < 4u; i++) {                                                      \
                ukl_mma_a_##T a;                                                                                        \
                ukl_mma_load_a_##T(&a, A + (la == UKL_ROW_MAJOR ? 16u * i * lda + km : km * lda + 16u * i), lda, la, at); \
                _Pragma("unroll") for (uint32_t j = 0u; j < 4u; j++)                                                    \
                    ukl_mma_##T(&c[4u * i + j], &a, &b[j], at __UKL_ARGS);                                              \
            }                                                                                                           \
        }                                                                                                               \
        _Pragma("unroll") for (uint32_t f = 0u; f < 16u; f++) {                                                         \
            uint32_t ci = 16u * (f >> 2), cj = 16u * (f & 3u);                                                          \
            ukl_mma_store_c_##C(&c[f], Cp + (lc == UKL_ROW_MAJOR ? ci * ldc + cj : cj * ldc + ci), ldc, lc, at);        \
        }                                                                                                               \
    }

#define UKL_MMA_CUDA_TYPE(T, C, CT, ET, UT, P, BITS, SHAPE, TYPES, R)                                                \
    UKL_MMA_CUDA_LOADS(T, ET, UT, P, BITS, UKL_MMA_K_##T / 2u, UKL_GLOBAL, )                                            \
    UKL_MMA_CUDA_LOADS(T, ET, UT, P, BITS, UKL_MMA_K_##T / 2u, UKL_LOCAL, _local)                                       \
    UKL_MMA_CUDA_PRODUCT(T, C, SHAPE, TYPES, R)                                                                         \
    UKL_MMA_CUDA_TILE(T, C, CT, ET)
#endif

#if UKL_MMA_APPLE
// ---- Apple's f16 and bf16: four 8×8 `simdgroup_matrix` a fragment of A or B (`MT` its type, `ST` an element's),
// each loaded where it lies, a column-major one transposed as it loads. ----

#define UKL_MMA_APPLE_LOADS(T, ST, SPACE, SUF)                                                                          \
    UKL_INLINE void ukl_mma_load_a_##T##SUF(UKL_PRIVATE ukl_mma_a_##T* a, SPACE const uint16_t* p, uint32_t stride,     \
                                            uint32_t layout, UklMmaAt at)                                               \
    {                                                                                                                   \
        (void)at;                                                                                                       \
        SPACE const ST* e = (SPACE const ST*)p;                                                                         \
        for (uint32_t ti = 0u; ti < 2u; ti++)                                                                           \
            for (uint32_t tk = 0u; tk < 2u; tk++) {                                                                     \
                if (layout == UKL_ROW_MAJOR)                                                                            \
                    simdgroup_load(a->m[2u * ti + tk], e + 8u * ti * stride + 8u * tk, stride, ulong2(0, 0), false);    \
                else                                                                                                    \
                    simdgroup_load(a->m[2u * ti + tk], e + 8u * tk * stride + 8u * ti, stride, ulong2(0, 0), true);     \
            }                                                                                                           \
    }                                                                                                                   \
    UKL_INLINE void ukl_mma_load_b_##T##SUF(UKL_PRIVATE ukl_mma_b_##T* b, SPACE const uint16_t* p, uint32_t stride,     \
                                            uint32_t layout, UklMmaAt at)                                               \
    {                                                                                                                   \
        (void)at;                                                                                                       \
        SPACE const ST* e = (SPACE const ST*)p;                                                                         \
        for (uint32_t tk = 0u; tk < 2u; tk++)                                                                           \
            for (uint32_t tj = 0u; tj < 2u; tj++) {                                                                     \
                if (layout == UKL_ROW_MAJOR)                                                                            \
                    simdgroup_load(b->m[2u * tk + tj], e + 8u * tk * stride + 8u * tj, stride, ulong2(0, 0), false);    \
                else                                                                                                    \
                    simdgroup_load(b->m[2u * tk + tj], e + 8u * tj * stride + 8u * tk, stride, ulong2(0, 0), true);     \
            }                                                                                                           \
    }

// Each of C's four 8×8 matrices: its two elements this lane holds (`thread_elements()`, at `ukl_mma_row` and
// `ukl_mma_col`) into a `simdgroup_float8x8`, two products along K, and back: registers, no memory.
#define UKL_MMA_APPLE_PRODUCT(T)                                                                                        \
    UKL_INLINE void ukl_mma_##T(UKL_PRIVATE ukl_mma_c_f32* c, UKL_PRIVATE const ukl_mma_a_##T* a,                       \
                                UKL_PRIVATE const ukl_mma_b_##T* b, UklMmaAt at __UKL_PARAMS)                           \
    {                                                                                                                   \
        (void)at;                                                                                                       \
        for (uint32_t i = 0u; i < 2u; i++)                                                                              \
            for (uint32_t j = 0u; j < 2u; j++) {                                                                        \
                simdgroup_float8x8 acc;                                                                                 \
                acc.thread_elements()[0] = c->x[4u * i + 2u * j];                                                       \
                acc.thread_elements()[1] = c->x[4u * i + 2u * j + 1u];                                                  \
                for (uint32_t kk = 0u; kk < 2u; kk++)                                                                   \
                    simdgroup_multiply_accumulate(acc, a->m[2u * i + kk], b->m[2u * kk + j], acc);                      \
                c->x[4u * i + 2u * j] = acc.thread_elements()[0];                                                       \
                c->x[4u * i + 2u * j + 1u] = acc.thread_elements()[1];                                                  \
            }                                                                                                           \
    }

#define UKL_MMA_APPLE_TYPE(T, ST)                                                                                       \
    UKL_MMA_APPLE_LOADS(T, ST, UKL_GLOBAL, )                                                                            \
    UKL_MMA_APPLE_LOADS(T, ST, UKL_LOCAL, _local)                                                                       \
    UKL_MMA_APPLE_PRODUCT(T)                                                                                            \
    UKL_MMA_TILE(T, f32, float, uint16_t)
#endif

// The emulation: the reference form, and every type a device's unit does not run.
#define UKL_MMA_EMULATED_TYPE(T, C, CT, ET, UT, P, BITS, WORDS, DOT)                                                    \
    UKL_MMA_AB_LOADS(T, ET, UT, P, BITS, WORDS, UKL_GLOBAL, )                                                           \
    UKL_MMA_AB_LOADS(T, ET, UT, P, BITS, WORDS, UKL_LOCAL, _local)                                                      \
    UKL_MMA_PRODUCT(T, C, CT, 16u / P, WORDS, DOT)                                                                      \
    UKL_MMA_TILE(T, C, CT, ET)

#if UKL_MMA_INTEL
// ---- Intel's DPAS: A's rows of this lane's k (`PA` elements in a short), B's column (`PB` elements in a word), each
// element read where it lies; two calls a product, C's rows 0-7 and 8-15 (`FN` the extension's function, `V` C's
// vector type, `E` its element's). ----

#define UKL_MMA_INTEL_LOADS(T, ET, UT, PA, PB, BITS, SPACE, SUF)                                                         \
    UKL_INLINE void ukl_mma_load_a_##T##SUF(UKL_PRIVATE ukl_mma_a_##T* a, SPACE const ET* p, uint32_t stride,           \
                                            uint32_t layout, UklMmaAt at)                                               \
    {                                                                                                                   \
        for (uint32_t m = 0u; m < 16u; m++) {                                                                           \
            uint32_t w = 0u;                                                                                            \
            for (uint32_t t = 0u; t < PA; t++) {                                                                        \
                uint32_t k = PA * at.lane + t;                                                                          \
                w |= (uint32_t)(UT)(layout == UKL_ROW_MAJOR ? p[m * stride + k] : p[k * stride + m]) << (t * BITS);    \
            }                                                                                                           \
            a->w[m] = (uint16_t)w;                                                                                      \
        }                                                                                                               \
    }                                                                                                                   \
    UKL_INLINE void ukl_mma_load_b_##T##SUF(UKL_PRIVATE ukl_mma_b_##T* b, SPACE const ET* p, uint32_t stride,           \
                                            uint32_t layout, UklMmaAt at)                                               \
    {                                                                                                                   \
        for (uint32_t i = 0u; i < 8u; i++) {                                                                            \
            uint32_t w = 0u;                                                                                            \
            for (uint32_t t = 0u; t < PB; t++) {                                                                        \
                uint32_t k = PB * i + t;                                                                                \
                w |= (uint32_t)(UT)(layout == UKL_ROW_MAJOR ? p[k * stride + at.lane] : p[at.lane * stride + k]) << (t * BITS); \
            }                                                                                                           \
            b->w[i] = w;                                                                                                \
        }                                                                                                               \
    }

#define UKL_MMA_INTEL_A8(a, h)                                                                                          \
    (short8)((short)(a)->w[8u * (h)], (short)(a)->w[8u * (h) + 1u], (short)(a)->w[8u * (h) + 2u], (short)(a)->w[8u * (h) + 3u], \
             (short)(a)->w[8u * (h) + 4u], (short)(a)->w[8u * (h) + 5u], (short)(a)->w[8u * (h) + 6u], (short)(a)->w[8u * (h) + 7u])
#define UKL_MMA_INTEL_PRODUCT(T, C, V, E, FN)                                                                           \
    UKL_INLINE void ukl_mma_##T(UKL_PRIVATE ukl_mma_c_##C* c, UKL_PRIVATE const ukl_mma_a_##T* a,                       \
                                UKL_PRIVATE const ukl_mma_b_##T* b, UklMmaAt at __UKL_PARAMS)                           \
    {                                                                                                                   \
        (void)at;                                                                                                       \
        int8 bw = (int8)((int)b->w[0], (int)b->w[1], (int)b->w[2], (int)b->w[3], (int)b->w[4], (int)b->w[5], (int)b->w[6], (int)b->w[7]); \
        for (uint32_t h = 0u; h < 2u; h++) {                                                                            \
            UKL_PRIVATE E* x = c->x + 8u * h;                                                                           \
            V acc = (V)(x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]);                                                \
            acc = FN(UKL_MMA_INTEL_A8(a, h), bw, acc);                                                                  \
            x[0] = acc.s0; x[1] = acc.s1; x[2] = acc.s2; x[3] = acc.s3;                                                 \
            x[4] = acc.s4; x[5] = acc.s5; x[6] = acc.s6; x[7] = acc.s7;                                                 \
        }                                                                                                               \
    }

#define UKL_MMA_INTEL_TYPE(T, C, CT, ET, UT, PA, PB, BITS, V, FN)                                                       \
    UKL_MMA_INTEL_LOADS(T, ET, UT, PA, PB, BITS, UKL_GLOBAL, )                                                          \
    UKL_MMA_INTEL_LOADS(T, ET, UT, PA, PB, BITS, UKL_LOCAL, _local)                                                     \
    UKL_MMA_INTEL_PRODUCT(T, C, V, CT, FN)                                                                              \
    UKL_MMA_TILE(T, C, CT, ET)

UKL_MMA_INTEL_TYPE(f16, f32, float, uint16_t, uint16_t, 1u, 2u, 16u, float8, intel_sub_group_f16_f16_matrix_mad_k16)
UKL_MMA_INTEL_TYPE(bf16, f32, float, uint16_t, uint16_t, 1u, 2u, 16u, float8, intel_sub_group_bf16_bf16_matrix_mad_k16)
UKL_MMA_INTEL_TYPE(i8, i32, int32_t, int8_t, uint8_t, 2u, 4u, 8u, int8, intel_sub_group_i8_i8_matrix_mad_k32)
#elif UKL_MMA_CUDA
UKL_MMA_CUDA_TYPE(f16, f32, float, uint16_t, uint16_t, 2u, 16u, "m16n8k16", "f32.f16.f16.f32", "+f")
UKL_MMA_CUDA_TYPE(bf16, f32, float, uint16_t, uint16_t, 2u, 16u, "m16n8k16", "f32.bf16.bf16.f32", "+f")
UKL_MMA_CUDA_TYPE(i8, i32, int32_t, int8_t, uint8_t, 4u, 8u, "m16n8k32", "s32.s8.s8.s32", "+r")
#else
#if UKL_MMA_APPLE
UKL_MMA_APPLE_TYPE(f16, half)
#else
UKL_MMA_EMULATED_TYPE(f16, f32, float, uint16_t, uint16_t, 2u, 16u, UKL_MMA_WORDS_16, ukl_dot2_f16)
#endif
#if UKL_MMA_APPLE_BF16
UKL_MMA_APPLE_TYPE(bf16, bfloat)
#else
UKL_MMA_EMULATED_TYPE(bf16, f32, float, uint16_t, uint16_t, 2u, 16u, UKL_MMA_WORDS_16, ukl_dot2_bf16)
#endif
UKL_MMA_EMULATED_TYPE(i8, i32, int32_t, int8_t, uint8_t, 4u, 8u, UKL_MMA_WORDS_8, ukl_dot4_i8)
#endif
UKL_MMA_B1_LOADS(UKL_GLOBAL, )
UKL_MMA_B1_LOADS(UKL_LOCAL, _local)
UKL_MMA_TILE(b1, i32, int32_t, uint64_t)
