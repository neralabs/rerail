// ukl/matrix.h: matrix units through fragments a kernel writes once (decision 0617). This is order-of-work step 1:
// the interface and its emulated form, which every backend runs until its unit's own form lands in ukl/backend.h.
//
// A fragment is a tile of a matrix held in registers by its owner: one SIMD group, or `UKL_MMA_GROUPS(T)` of them
// together. Each type has its shape, `C[M×N] += A[M×K] · B[K×N]`, as compile-time constants:
//
//   UKL_MMA_M(T), UKL_MMA_N(T), UKL_MMA_K(T)   the shape; each divides UKL_MMA_BLOCK (64)
//   UKL_MMA_GROUPS(T)                          the SIMD groups that own a fragment
//   UKL_MMA_NATIVE(T)                          1 where the device's unit runs it, 0 where it is emulated
//
// `T` is `f16` or `bf16` (into f32) or `i8` (into i32). In memory, a 16-bit float is its bits (`uint16_t`), an i8 an
// `int8_t`, an accumulator a `float` or an `int32_t`.
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
// The emulated form, every type 16×16×16, is the library's reference:
// - C's 256 elements, row-major, lie across the owner's lanes: element e of owner lane `part · W + lane` is
//   element `e · L + part · W + lane` (`W` the SIMD width, `L` the owner's lanes).
// - A and B are packed words along K (two 16-bit floats or four bytes a word). Each SIMD group of the owner holds the
//   whole of A and B, k-major: word j of a lane is word `j · W + lane`, at k-word `idx / 16` of row (A) or column
//   (B) `idx % 16`.
// - So a product's lane reads its row of A and its column of B from the other lanes by shuffle, each from the same
//   register in every lane (`16 · kw / W`), and adds them with `<ukl/dot.h>`'s dot products: the device's
//   instructions where it has them.
// - Its sums are in k's order, the dot products' own rounding: f32, or exact for i8.
//
// At 16 lanes or more a lane's column of B is the same for each of its elements, so it is read once a product. A
// narrower group (an OpenCL device's 8 lanes, or 1) reads each element's row and column apart; every lane still reads
// the same register, which may then be known only at run time (an owner of several groups), so it is picked by value.

#pragma once
#include <ukl/simd.h>
#include <ukl/dot.h>

#define UKL_MMA_BLOCK 64u
#define UKL_ROW_MAJOR 0u
#define UKL_COL_MAJOR 1u

// The SIMD groups an emulated owner spans: 1, or (a test's define) 2 or 4, so that the multi-group path is tested
// where no device has it yet (requirement R5).
#ifndef UKL_MMA_EMULATED_GROUPS
#define UKL_MMA_EMULATED_GROUPS 1u
#endif

#define UKL_MMA_M(T) UKL_MMA_M_##T
#define UKL_MMA_N(T) UKL_MMA_N_##T
#define UKL_MMA_K(T) UKL_MMA_K_##T
#define UKL_MMA_GROUPS(T) UKL_MMA_GROUPS_##T
#define UKL_MMA_NATIVE(T) UKL_MMA_NATIVE_##T

#define UKL_MMA_M_f16 16u
#define UKL_MMA_N_f16 16u
#define UKL_MMA_K_f16 16u
#define UKL_MMA_GROUPS_f16 UKL_MMA_EMULATED_GROUPS
#define UKL_MMA_NATIVE_f16 0
#define UKL_MMA_M_bf16 16u
#define UKL_MMA_N_bf16 16u
#define UKL_MMA_K_bf16 16u
#define UKL_MMA_GROUPS_bf16 UKL_MMA_EMULATED_GROUPS
#define UKL_MMA_NATIVE_bf16 0
#define UKL_MMA_M_i8 16u
#define UKL_MMA_N_i8 16u
#define UKL_MMA_K_i8 16u
#define UKL_MMA_GROUPS_i8 UKL_MMA_EMULATED_GROUPS
#define UKL_MMA_NATIVE_i8 0

// The emulation's layout: the owner's lanes, C's elements a lane, A's and B's words a lane (16 rows of 8 words for a
// 16-bit type, of 4 for bytes).
#define UKL_MMA_LANES (UKL_MMA_EMULATED_GROUPS * UKL_SIMD_WIDTH)
#define UKL_MMA_C_PER (256u / UKL_MMA_LANES)
#define UKL_MMA_WORDS_16 (128u / UKL_SIMD_WIDTH)
#define UKL_MMA_WORDS_8 (64u / UKL_SIMD_WIDTH)

// Requirement R1: every shape divides the block; an owner's lanes divide C's elements.
UKL_STATIC_ASSERT(UKL_MMA_BLOCK % UKL_MMA_M_f16 == 0u && UKL_MMA_BLOCK % UKL_MMA_N_f16 == 0u && UKL_MMA_BLOCK % UKL_MMA_K_f16 == 0u, "f16's shape divides 64");
UKL_STATIC_ASSERT(UKL_MMA_BLOCK % UKL_MMA_M_bf16 == 0u && UKL_MMA_BLOCK % UKL_MMA_N_bf16 == 0u && UKL_MMA_BLOCK % UKL_MMA_K_bf16 == 0u, "bf16's shape divides 64");
UKL_STATIC_ASSERT(UKL_MMA_BLOCK % UKL_MMA_M_i8 == 0u && UKL_MMA_BLOCK % UKL_MMA_N_i8 == 0u && UKL_MMA_BLOCK % UKL_MMA_K_i8 == 0u, "i8's shape divides 64");
UKL_STATIC_ASSERT(UKL_MMA_LANES <= 256u && 256u % UKL_MMA_LANES == 0u, "an emulated owner's lanes divide a fragment's 256 elements");

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

// C's element `e` of this thread: its index, row-major in the 16×16 fragment.
UKL_INLINE uint32_t ukl_mma_c_index(uint32_t e, UklMmaAt at) { return e * UKL_MMA_LANES + at.part * UKL_SIMD_WIDTH + at.lane; }

typedef struct ukl_mma_a_f16 { uint32_t w[UKL_MMA_WORDS_16]; } ukl_mma_a_f16;
typedef struct ukl_mma_b_f16 { uint32_t w[UKL_MMA_WORDS_16]; } ukl_mma_b_f16;
typedef struct ukl_mma_a_bf16 { uint32_t w[UKL_MMA_WORDS_16]; } ukl_mma_a_bf16;
typedef struct ukl_mma_b_bf16 { uint32_t w[UKL_MMA_WORDS_16]; } ukl_mma_b_bf16;
typedef struct ukl_mma_a_i8 { uint32_t w[UKL_MMA_WORDS_8]; } ukl_mma_a_i8;
typedef struct ukl_mma_b_i8 { uint32_t w[UKL_MMA_WORDS_8]; } ukl_mma_b_i8;
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
        for (uint32_t row = ukl_mma_c_index(__ukl_mma_e, (at)) >> 4, col = ukl_mma_c_index(__ukl_mma_e, (at)) & 15u,    \
                      __ukl_mma_once = 1u;                                                                              \
             __ukl_mma_once != 0u; __ukl_mma_once = 0u)                                                                 \
            for (UKL_MMA_ELEM_TYPE_((c).x[0]) v = (c).x[__ukl_mma_e]; __ukl_mma_once != 0u;                             \
                 (c).x[__ukl_mma_e] = v, __ukl_mma_once = 0u)

// ---- C: fill, load, store (`C` f32 or i32, `CT` its element type). ----

#define UKL_MMA_C_OPS(C, CT, SPACE, SUF)                                                                                \
    UKL_INLINE void ukl_mma_load_c_##C##SUF(UKL_PRIVATE ukl_mma_c_##C* c, SPACE const CT* p, uint32_t stride,           \
                                            uint32_t layout, UklMmaAt at)                                               \
    {                                                                                                                   \
        for (uint32_t e = 0u; e < UKL_MMA_C_PER; e++) {                                                                 \
            uint32_t i = ukl_mma_c_index(e, at), r = i >> 4, k = i & 15u;                                               \
            c->x[e] = layout == UKL_ROW_MAJOR ? p[r * stride + k] : p[k * stride + r];                                  \
        }                                                                                                               \
    }                                                                                                                   \
    UKL_INLINE void ukl_mma_store_c_##C##SUF(UKL_PRIVATE const ukl_mma_c_##C* c, SPACE CT* p, uint32_t stride,          \
                                             uint32_t layout, UklMmaAt at)                                              \
    {                                                                                                                   \
        for (uint32_t e = 0u; e < UKL_MMA_C_PER; e++) {                                                                 \
            uint32_t i = ukl_mma_c_index(e, at), r = i >> 4, k = i & 15u;                                               \
            if (layout == UKL_ROW_MAJOR)                                                                                \
                p[r * stride + k] = c->x[e];                                                                            \
            else                                                                                                        \
                p[k * stride + r] = c->x[e];                                                                            \
        }                                                                                                               \
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

// ---- A and B: loads of `T`'s elements (`ET` in memory, `P` a word, `BITS` each), packed along K. ----

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

// ---- The product: `c += a · b`, by shuffles and `DOT` (16 / P k-words, each `P` products). ----

#if UKL_SIMD_WIDTH >= 16
// A lane's column of B is the same for each of its elements (`lane % 16`), read once; each word read from register
// `16 · kw / W` of lane `16 · kw % W` plus the row or the column: a constant register.
#define UKL_MMA_PRODUCT(T, C, CT, P, WORDS, DOT)                                                                        \
    UKL_INLINE void ukl_mma_##T(UKL_PRIVATE ukl_mma_c_##C* c, UKL_PRIVATE const ukl_mma_a_##T* a,                       \
                                UKL_PRIVATE const ukl_mma_b_##T* b, UklMmaAt at __UKL_PARAMS)                           \
    {                                                                                                                   \
        uint32_t bw[16u / P];                                                                                           \
        for (uint32_t kw = 0u; kw < 16u / P; kw++)                                                                      \
            bw[kw] = ukl_sg_shuffle(b->w[(16u * kw) / UKL_SIMD_WIDTH], (16u * kw) % UKL_SIMD_WIDTH + (at.lane & 15u) __UKL_ARGS); \
        for (uint32_t e = 0u; e < UKL_MMA_C_PER; e++) {                                                                 \
            uint32_t r = ukl_mma_c_index(e, at) >> 4;                                                                   \
            CT acc = c->x[e];                                                                                           \
            for (uint32_t kw = 0u; kw < 16u / P; kw++)                                                                  \
                acc = DOT(ukl_sg_shuffle(a->w[(16u * kw) / UKL_SIMD_WIDTH], (16u * kw) % UKL_SIMD_WIDTH + r __UKL_ARGS), bw[kw], acc); \
            c->x[e] = acc;                                                                                              \
        }                                                                                                               \
    }
#else
// A narrow group (fewer than 16 lanes): each element's row of A and column of B read apart, the register picked.
#define UKL_MMA_PRODUCT(T, C, CT, P, WORDS, DOT)                                                                        \
    UKL_INLINE void ukl_mma_##T(UKL_PRIVATE ukl_mma_c_##C* c, UKL_PRIVATE const ukl_mma_a_##T* a,                       \
                                UKL_PRIVATE const ukl_mma_b_##T* b, UklMmaAt at __UKL_PARAMS)                           \
    {                                                                                                                   \
        for (uint32_t e = 0u; e < UKL_MMA_C_PER; e++) {                                                                 \
            uint32_t i = ukl_mma_c_index(e, at), r = i >> 4, col = i & 15u;                                             \
            CT acc = c->x[e];                                                                                           \
            for (uint32_t kw = 0u; kw < 16u / P; kw++) {                                                                \
                uint32_t ia = 16u * kw + r, ib = 16u * kw + col;                                                        \
                uint32_t av = ukl_sg_shuffle(ukl_mma_word_(a->w, WORDS, ia / UKL_SIMD_WIDTH), ia % UKL_SIMD_WIDTH __UKL_ARGS); \
                uint32_t bv = ukl_sg_shuffle(ukl_mma_word_(b->w, WORDS, ib / UKL_SIMD_WIDTH), ib % UKL_SIMD_WIDTH __UKL_ARGS); \
                acc = DOT(av, bv, acc);                                                                                 \
            }                                                                                                           \
            c->x[e] = acc;                                                                                              \
        }                                                                                                               \
    }
#endif

// ---- Level 2: one owner's 64×64 block of C (at `C`, `lc` its layout) += A[64×K] · B[K×64], from device memory. ----
// The reference is level 1's loop, fragment by fragment; a backend may override it (decision 0617 §5).

#define UKL_MMA_TILE(T, C, CT, ET)                                                                                      \
    UKL_INLINE void ukl_gemm_tile_##T(UKL_GLOBAL CT* Cp, uint32_t ldc, uint32_t lc, UKL_GLOBAL const ET* A,             \
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
                    ukl_mma_a_##T a;                                                                                    \
                    ukl_mma_b_##T b;                                                                                    \
                    ukl_mma_load_a_##T(&a, A + (la == UKL_ROW_MAJOR ? ci * lda + k : k * lda + ci), lda, la, at);       \
                    ukl_mma_load_b_##T(&b, B + (lb == UKL_ROW_MAJOR ? k * ldb + cj : cj * ldb + k), ldb, lb, at);       \
                    ukl_mma_##T(&c, &a, &b, at __UKL_ARGS);                                                             \
                }                                                                                                       \
                ukl_mma_store_c_##C(&c, Cp + co, ldc, lc, at);                                                          \
            }                                                                                                           \
    }

#define UKL_MMA_TYPE(T, C, CT, ET, UT, P, BITS, WORDS, DOT)                                                             \
    UKL_MMA_AB_LOADS(T, ET, UT, P, BITS, WORDS, UKL_GLOBAL, )                                                           \
    UKL_MMA_AB_LOADS(T, ET, UT, P, BITS, WORDS, UKL_LOCAL, _local)                                                      \
    UKL_MMA_PRODUCT(T, C, CT, P, WORDS, DOT)                                                                                   \
    UKL_MMA_TILE(T, C, CT, ET)

UKL_MMA_TYPE(f16, f32, float, uint16_t, uint16_t, 2u, 16u, UKL_MMA_WORDS_16, ukl_dot2_f16)
UKL_MMA_TYPE(bf16, f32, float, uint16_t, uint16_t, 2u, 16u, UKL_MMA_WORDS_16, ukl_dot2_bf16)
UKL_MMA_TYPE(i8, i32, int32_t, int8_t, uint8_t, 4u, 8u, UKL_MMA_WORDS_8, ukl_dot4_i8)
