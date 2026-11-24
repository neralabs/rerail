// ukl/memory.h: a block's bytes read and written in place at any byte address (decisions 0440 §5, 0353, 0310).
//
// A tape, a relation's block or an index is a byte buffer whose values lie at any byte offset: the generated reads
// (CLIF's, 0310) read them here, where they lie, and nothing is copied out first. The buffer's base is 8-byte
// aligned, and it is readable through the 8-byte word that holds its last byte (a block's pages are).
//
// A value at an unaligned address is read from the aligned word that holds its first byte, and the next one only
// when the value runs into it, so a value is never read past the word that holds its last byte. A direct load where
// the address is aligned: CUDA faults on an unaligned one (Apple's GPUs allow it).
//
// Two families, the same code: `ukl_ld_*` reads `UKL_GLOBAL` memory; `ukl_ldc_*`, `ukl_stc_*` read and write
// `UKL_COHERENT` memory, which another threadgroup of the same dispatch writes. A write of a value at an unaligned
// address is a read-modify-write of its words: not atomic with another thread's write to the same word.
//
// The address type is the caller's: 32-bit where a buffer is under 4 GB (cheaper on a GPU), else 64-bit. Each
// function is defined for both (`UKL_OVERLOAD`), by a macro over the address type `A`.
//
// A backend whose coherent memory is no qualifier (OpenCL 1.2: a load may be answered by a cache another
// threadgroup's write has not reached) defines `UKL_COHERENT_LOAD64(p)`, its coherent read of the 64-bit word at
// `p`, and `UKL_COHERENT_STORE_BITS64(p, m, x)`, its coherent write of the bits `m` of that word to `x`'s (the
// others left as they are, whoever writes them meanwhile). Then every coherent load and store goes through them: a
// value is read and written in its words, never directly.

#pragma once
#include <ukl/bits.h>

// A buffer handed on by its own address where that is not 8-byte aligned (a document's lines in its tape, at a 4-byte
// offset, decision 0612) is read from the word below it: the base `p - UKL_PTR_LOW3(p)`, its offsets `UKL_PTR_LOW3(p)`
// further. A backend whose loads take any alignment defines it 0 (Metal, `backend.h`).
#ifndef UKL_PTR_LOW3
#define UKL_PTR_LOW3(p) ((uint32_t)(uint64_t)(p) & 7u)
#endif

#if defined(UKL_COHERENT_LOAD64)
#define __UKL_STC_DIRECT 0
#define __UKL_STC_BITS(w, m, x) UKL_COHERENT_STORE_BITS64(w, m, x)
#define __UKL_WORD_ukl_ld(p) (*(p))
#define __UKL_WORD_ukl_ldc(p) UKL_COHERENT_LOAD64(p)
#define __UKL_DIRECT_ukl_ld 1
#define __UKL_DIRECT_ukl_ldc 0
#else
#define __UKL_STC_DIRECT 1
#define __UKL_STC_BITS(w, m, x) (*(w) = (*(w) & ~(m)) | (x))
#define __UKL_WORD_ukl_ld(p) (*(p))
#define __UKL_WORD_ukl_ldc(p) (*(p))
#define __UKL_DIRECT_ukl_ld 1
#define __UKL_DIRECT_ukl_ldc 1
#endif

// The CPU's vector code (`UKL_VEC`) reads a window of a whole value's bytes (1, 2, 4 or 8) as one unaligned load of
// those bytes alone: the words it touches are the aligned form's (the word that holds its first byte, and the next
// only if they run into it), never past its last byte's, and a lane's consecutive windows are then one vector load
// of their elements (rs.rs), not each lane's word, shift and second word. The bytes above `n` are 0.
#if defined(UKL_VEC)
#define __UKL_WINDOW_WHOLE(Ld, Space, t, a, n)                                                                          \
    if ((n) == 8u)                                                                                                      \
        return __UKL_WORD_##Ld((Space const uint64_t*)((t) + (a)));                                                     \
    if ((n) == 4u)                                                                                                      \
        return *(Space const uint32_t*)((t) + (a));                                                                     \
    if ((n) == 2u)                                                                                                      \
        return *(Space const uint16_t*)((t) + (a));                                                                     \
    if ((n) == 1u)                                                                                                      \
        return (t)[a];
#else
#define __UKL_WINDOW_WHOLE(Ld, Space, t, a, n)
#endif

#define UKL_DEFINE_BYTE_LOADS(Ld, Space, A)                                                                             \
    /* The 64 bits from byte `a` up, of which the low `n` bytes are wanted. */                                          \
    UKL_INLINE UKL_OVERLOAD uint64_t Ld##_window(Space const uint8_t* t, A a, uint32_t n)                               \
    {                                                                                                                   \
        __UKL_WINDOW_WHOLE(Ld, Space, t, a, n)                                                                          \
        Space const uint64_t* w = (Space const uint64_t*)(t + (a & ~(A)7));                                             \
        uint32_t s = (uint32_t)(a & (A)7) * 8u;                                                                         \
        uint64_t lo = __UKL_WORD_##Ld(w) >> s;                                                                          \
        return s != 0u && s + n * 8u > 64u ? lo | (__UKL_WORD_##Ld(w + 1) << (64u - s)) : lo;                           \
    }                                                                                                                   \
    UKL_INLINE UKL_OVERLOAD uint8_t Ld##_u8(Space const uint8_t* t, A a)                                                \
    {                                                                                                                   \
        return __UKL_DIRECT_##Ld ? t[a] : (uint8_t)Ld##_window(t, a, 1u);                                               \
    }                                                                                                                   \
    UKL_INLINE UKL_OVERLOAD uint16_t Ld##_u16(Space const uint8_t* t, A a)                                              \
    {                                                                                                                   \
        return __UKL_DIRECT_##Ld && (a & (A)1) == 0 ? *(Space const uint16_t*)(t + a) : (uint16_t)Ld##_window(t, a, 2u);\
    }                                                                                                                   \
    UKL_INLINE UKL_OVERLOAD uint32_t Ld##_u32(Space const uint8_t* t, A a)                                              \
    {                                                                                                                   \
        return __UKL_DIRECT_##Ld && (a & (A)3) == 0 ? *(Space const uint32_t*)(t + a) : (uint32_t)Ld##_window(t, a, 4u);\
    }                                                                                                                   \
    UKL_INLINE UKL_OVERLOAD uint64_t Ld##_u64(Space const uint8_t* t, A a)                                              \
    {                                                                                                                   \
        return __UKL_DIRECT_##Ld && (a & (A)7) == 0 ? *(Space const uint64_t*)(t + a) : Ld##_window(t, a, 8u);          \
    }                                                                                                                   \
    /* `n` bits (at most 57) from bit `b` up: a bit-packed field. */                                                    \
    UKL_INLINE UKL_OVERLOAD uint64_t Ld##_bits(Space const uint8_t* t, A b, uint32_t n)                                 \
    {                                                                                                                   \
        uint32_t r = (uint32_t)(b & (A)7);                                                                              \
        return (Ld##_window(t, b >> 3, (r + n + 7u) / 8u) >> r) & ukl_mask64(n);                                        \
    }

// Entry `i` of an array of `n`-bit entries packed from bit 0 (`n` at most 57).
#define UKL_DEFINE_PACKED_LOAD(Ld, Space)                                                                               \
    UKL_INLINE uint64_t Ld##_packed(Space const uint8_t* t, uint64_t i, uint32_t n) { return Ld##_bits(t, i * n, n); }

UKL_DEFINE_BYTE_LOADS(ukl_ld, UKL_GLOBAL, uint32_t)
UKL_DEFINE_BYTE_LOADS(ukl_ld, UKL_GLOBAL, uint64_t)
UKL_DEFINE_BYTE_LOADS(ukl_ldc, UKL_COHERENT, uint32_t)
UKL_DEFINE_BYTE_LOADS(ukl_ldc, UKL_COHERENT, uint64_t)
UKL_DEFINE_PACKED_LOAD(ukl_ld, UKL_GLOBAL)
UKL_DEFINE_PACKED_LOAD(ukl_ldc, UKL_COHERENT)

// The low `n` bits of `x` written from byte `a` up (`n` a multiple of 8, at most 64).
#define UKL_DEFINE_STORES(A)                                                                                            \
    UKL_INLINE UKL_OVERLOAD void ukl_stc_window(UKL_COHERENT uint8_t* t, A a, uint64_t x, uint32_t n)                   \
    {                                                                                                                   \
        UKL_COHERENT uint64_t* w = (UKL_COHERENT uint64_t*)(t + (a & ~(A)7));                                           \
        uint32_t s = (uint32_t)(a & (A)7) * 8u;                                                                         \
        uint64_t m = ukl_mask64(n);                                                                                     \
        x &= m;                                                                                                         \
        __UKL_STC_BITS(w, m << s, x << s);                                                                              \
        if (s != 0u && s + n > 64u) {                                                                                   \
            uint32_t r = 64u - s;                                                                                       \
            __UKL_STC_BITS(w + 1, m >> r, x >> r);                                                                      \
        }                                                                                                               \
    }                                                                                                                   \
    UKL_INLINE UKL_OVERLOAD void ukl_stc_u8(UKL_COHERENT uint8_t* t, A a, uint64_t x)                                   \
    {                                                                                                                   \
        if (__UKL_STC_DIRECT)                                                                                           \
            t[a] = (uint8_t)x;                                                                                          \
        else                                                                                                            \
            ukl_stc_window(t, a, x, 8u);                                                                                \
    }                                                                                                                   \
    UKL_INLINE UKL_OVERLOAD void ukl_stc_u16(UKL_COHERENT uint8_t* t, A a, uint64_t x)                                  \
    {                                                                                                                   \
        if (__UKL_STC_DIRECT && (a & (A)1) == 0)                                                                        \
            *(UKL_COHERENT uint16_t*)(t + a) = (uint16_t)x;                                                             \
        else                                                                                                            \
            ukl_stc_window(t, a, x, 16u);                                                                               \
    }                                                                                                                   \
    UKL_INLINE UKL_OVERLOAD void ukl_stc_u32(UKL_COHERENT uint8_t* t, A a, uint64_t x)                                  \
    {                                                                                                                   \
        if (__UKL_STC_DIRECT && (a & (A)3) == 0)                                                                        \
            *(UKL_COHERENT uint32_t*)(t + a) = (uint32_t)x;                                                             \
        else                                                                                                            \
            ukl_stc_window(t, a, x, 32u);                                                                               \
    }                                                                                                                   \
    UKL_INLINE UKL_OVERLOAD void ukl_stc_u64(UKL_COHERENT uint8_t* t, A a, uint64_t x)                                  \
    {                                                                                                                   \
        if (__UKL_STC_DIRECT && (a & (A)7) == 0)                                                                        \
            *(UKL_COHERENT uint64_t*)(t + a) = x;                                                                       \
        else                                                                                                            \
            ukl_stc_window(t, a, x, 64u);                                                                               \
    }

UKL_DEFINE_STORES(uint32_t)
UKL_DEFINE_STORES(uint64_t)

// Memory read and written at several widths (a field written as 32 bits, read in a 64-bit word): every access a byte
// or an aligned 64-bit word (`ukl_ldw_*`, `ukl_stw_*`), so no direct access of another width lets the compiler's
// type-based alias rules reorder them. A write of a value that is not a byte, or not an aligned 64-bit word, is a
// read-modify-write of its words: not atomic with another thread's write to the same word.
#define __UKL_WORD_ukl_ldw(p) (*(p))
#define __UKL_DIRECT_ukl_ldw 0
UKL_DEFINE_BYTE_LOADS(ukl_ldw, UKL_GLOBAL, uint32_t)
UKL_DEFINE_BYTE_LOADS(ukl_ldw, UKL_GLOBAL, uint64_t)

#define UKL_DEFINE_WORD_STORES(A)                                                                                    \
    UKL_INLINE UKL_OVERLOAD void ukl_stw_window(UKL_GLOBAL uint8_t* t, A a, uint64_t x, uint32_t n)                  \
    {                                                                                                                \
        UKL_GLOBAL uint64_t* w = (UKL_GLOBAL uint64_t*)(t + (a & ~(A)7));                                            \
        uint32_t s = (uint32_t)(a & (A)7) * 8u;                                                                      \
        uint64_t m = ukl_mask64(n);                                                                                  \
        x &= m;                                                                                                      \
        w[0] = (w[0] & ~(m << s)) | (x << s);                                                                        \
        if (s != 0u && s + n > 64u) {                                                                                \
            uint32_t r = 64u - s;                                                                                    \
            w[1] = (w[1] & ~(m >> r)) | (x >> r);                                                                    \
        }                                                                                                            \
    }                                                                                                                \
    UKL_INLINE UKL_OVERLOAD void ukl_stw_u8(UKL_GLOBAL uint8_t* t, A a, uint64_t x) { t[a] = (uint8_t)x; }           \
    UKL_INLINE UKL_OVERLOAD void ukl_stw_u16(UKL_GLOBAL uint8_t* t, A a, uint64_t x) { ukl_stw_window(t, a, x, 16u); } \
    UKL_INLINE UKL_OVERLOAD void ukl_stw_u32(UKL_GLOBAL uint8_t* t, A a, uint64_t x) { ukl_stw_window(t, a, x, 32u); } \
    UKL_INLINE UKL_OVERLOAD void ukl_stw_u64(UKL_GLOBAL uint8_t* t, A a, uint64_t x)                                 \
    {                                                                                                                \
        if ((a & (A)7) == 0)                                                                                         \
            *(UKL_GLOBAL uint64_t*)(t + a) = x;                                                                      \
        else                                                                                                         \
            ukl_stw_window(t, a, x, 64u);                                                                            \
    }

UKL_DEFINE_WORD_STORES(uint32_t)
UKL_DEFINE_WORD_STORES(uint64_t)

// Relaxed atomics on the aligned 32-bit word at byte `a` of a block.
#define UKL_DEFINE_ATOMICS_AT(A)                                                                                        \
    UKL_INLINE UKL_OVERLOAD UKL_GLOBAL uint32_t* ukl_word_at(UKL_GLOBAL const uint8_t* t, A a) { return (UKL_GLOBAL uint32_t*)(t + a); } \
    UKL_INLINE UKL_OVERLOAD uint32_t ukl_atomic_load_at(UKL_GLOBAL const uint8_t* t, A a) { return ukl_atomic_load(ukl_word_at(t, a)); } \
    UKL_INLINE UKL_OVERLOAD uint32_t ukl_atomic_poll_at(UKL_GLOBAL const uint8_t* t, A a) { return ukl_atomic_poll(ukl_word_at(t, a)); } \
    UKL_INLINE UKL_OVERLOAD void ukl_atomic_store_at(UKL_GLOBAL const uint8_t* t, A a, uint32_t x) { ukl_atomic_store(ukl_word_at(t, a), x); } \
    UKL_INLINE UKL_OVERLOAD uint32_t ukl_atomic_add_at(UKL_GLOBAL const uint8_t* t, A a, uint32_t x) { return ukl_atomic_add(ukl_word_at(t, a), x); } \
    UKL_INLINE UKL_OVERLOAD uint32_t ukl_atomic_or_at(UKL_GLOBAL const uint8_t* t, A a, uint32_t x) { return ukl_atomic_or(ukl_word_at(t, a), x); } \
    UKL_INLINE UKL_OVERLOAD uint32_t ukl_atomic_cas_at(UKL_GLOBAL const uint8_t* t, A a, uint32_t e, uint32_t n) { return ukl_atomic_cas(ukl_word_at(t, a), e, n); }

UKL_DEFINE_ATOMICS_AT(uint32_t)
UKL_DEFINE_ATOMICS_AT(uint64_t)

// The same at a coherent base (a block other groups of the dispatch write, `<ukl/clif.h>`'s `CLIF_BLOCK_SPACE`,
// decision 0603): an atomic is served where the word's writes meet already, so it is the device's.
#define UKL_DEFINE_COHERENT_ATOMICS_AT(A)                                                                               \
    UKL_INLINE UKL_OVERLOAD uint32_t ukl_atomic_load_at(UKL_COHERENT const uint8_t* t, A a) { return ukl_atomic_load_at((UKL_GLOBAL const uint8_t*)t, a); } \
    UKL_INLINE UKL_OVERLOAD uint32_t ukl_atomic_poll_at(UKL_COHERENT const uint8_t* t, A a) { return ukl_atomic_poll_at((UKL_GLOBAL const uint8_t*)t, a); } \
    UKL_INLINE UKL_OVERLOAD void ukl_atomic_store_at(UKL_COHERENT const uint8_t* t, A a, uint32_t x) { ukl_atomic_store_at((UKL_GLOBAL const uint8_t*)t, a, x); } \
    UKL_INLINE UKL_OVERLOAD uint32_t ukl_atomic_add_at(UKL_COHERENT const uint8_t* t, A a, uint32_t x) { return ukl_atomic_add_at((UKL_GLOBAL const uint8_t*)t, a, x); } \
    UKL_INLINE UKL_OVERLOAD uint32_t ukl_atomic_or_at(UKL_COHERENT const uint8_t* t, A a, uint32_t x) { return ukl_atomic_or_at((UKL_GLOBAL const uint8_t*)t, a, x); } \
    UKL_INLINE UKL_OVERLOAD uint32_t ukl_atomic_cas_at(UKL_COHERENT const uint8_t* t, A a, uint32_t e, uint32_t n) { return ukl_atomic_cas_at((UKL_GLOBAL const uint8_t*)t, a, e, n); }

// The vector path (`UKL_VEC`, decision 0567) reads one memory: a coherent base is a global one there, and its C
// reader tells no overload apart by `volatile` (rerail-ukl-vec's `overload`), so its atomics are the global ones.
#if !defined(UKL_VEC)
UKL_DEFINE_COHERENT_ATOMICS_AT(uint32_t)
UKL_DEFINE_COHERENT_ATOMICS_AT(uint64_t)
#endif
