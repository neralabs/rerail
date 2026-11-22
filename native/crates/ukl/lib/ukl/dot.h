// ukl/dot.h: dot products of small packed values into a wider accumulator, one thread's (decision 0617 §7).
//
// Each operand is a 32-bit word holding the values packed, lowest first: four 8-bit integers, or two 16-bit floats
// (f16 or bf16). Each call adds the products of the two words' values to `acc`:
//
//   ukl_dot4_i8(a, b, acc)   acc + Σ a[k]·b[k], k < 4, signed bytes, into an i32 (wrapping, never saturating)
//   ukl_dot4_u8(a, b, acc)   the same for unsigned bytes, into a u32
//   ukl_dot2_f16(a, b, acc)  acc + a[0]·b[0] + a[1]·b[1], two f16s each, into an f32
//   ukl_dot2_bf16(a, b, acc) the same for bf16
//
// The integer forms are exact on every backend. The float forms' products are exact in f32 (an f16's or bf16's
// significand times another's fits in an f32's), so only the sums round. The reference (`..._ref`) rounds twice: a
// fused multiply-add of the first product into `acc`, then of the second. A native form may round once (AMD's
// `v_dot2_f32_f16` rounds the exact sum once, measured on a Radeon 680M), so it is within one rounding of the
// reference, and equal to it wherever the sums are exact.
//
// A backend with a faster form defines it in ukl/backend.h as `ukl_..._native` with its flag: `UKL_NATIVE_DOT4`
// (both byte forms), `UKL_NATIVE_DOT2_F16`, `UKL_NATIVE_DOT2_BF16`, `UKL_NATIVE_F16` (the f16 conversion). The
// library's tests (`library_tests.rs`, `dot_products_equal_their_references`) check each against its reference.

#pragma once

#include <ukl/backend.h>

// An f16's bits (the low 16 of `h`) as an f32: exact, for every value (zeros, subnormals, infinities, NaNs).
UKL_INLINE float ukl_f16_to_f32_ref(uint32_t h)
{
    uint32_t s = (h & 0x8000u) << 16, e = (h >> 10) & 31u, m = h & 0x3ffu;
    if (e == 31u)
        return reinterpretIntAsFloat((int32_t)(s | 0x7f800000u | (m << 13)));
    if (e == 0u) {
        // A subnormal f16 is m · 2^-24: exact in f32, where it is normal.
        float f = (float)m * 5.9604644775390625e-8f;
        return s != 0u ? -f : f;
    }
    return reinterpretIntAsFloat((int32_t)(s | ((e + 112u) << 23) | (m << 13)));
}

#if defined(UKL_NATIVE_F16)
UKL_INLINE float ukl_f16_to_f32(uint32_t h) { return ukl_f16_to_f32_native(h & 0xffffu); }
#else
UKL_INLINE float ukl_f16_to_f32(uint32_t h) { return ukl_f16_to_f32_ref(h); }
#endif

// A bf16's bits (the low 16 of `h`) as an f32: its bits are an f32's top half.
UKL_INLINE float ukl_bf16_to_f32(uint32_t h) { return reinterpretIntAsFloat((int32_t)(h << 16)); }

UKL_INLINE int32_t ukl_dot4_i8_ref(uint32_t a, uint32_t b, int32_t acc)
{
    uint32_t s = (uint32_t)acc;
    for (uint32_t k = 0u; k < 32u; k += 8u)
        s += (uint32_t)((int32_t)(int8_t)(uint8_t)(a >> k) * (int32_t)(int8_t)(uint8_t)(b >> k));
    return (int32_t)s;
}

UKL_INLINE uint32_t ukl_dot4_u8_ref(uint32_t a, uint32_t b, uint32_t acc)
{
    for (uint32_t k = 0u; k < 32u; k += 8u)
        acc += ((a >> k) & 0xffu) * ((b >> k) & 0xffu);
    return acc;
}

UKL_INLINE float ukl_dot2_f16_ref(uint32_t a, uint32_t b, float acc)
{
    acc = fma(ukl_f16_to_f32_ref(a & 0xffffu), ukl_f16_to_f32_ref(b & 0xffffu), acc);
    return fma(ukl_f16_to_f32_ref(a >> 16), ukl_f16_to_f32_ref(b >> 16), acc);
}

UKL_INLINE float ukl_dot2_bf16_ref(uint32_t a, uint32_t b, float acc)
{
    acc = fma(ukl_bf16_to_f32(a & 0xffffu), ukl_bf16_to_f32(b & 0xffffu), acc);
    return fma(ukl_bf16_to_f32(a >> 16), ukl_bf16_to_f32(b >> 16), acc);
}

#if defined(UKL_NATIVE_DOT4)
UKL_INLINE int32_t ukl_dot4_i8(uint32_t a, uint32_t b, int32_t acc) { return ukl_dot4_i8_native(a, b, acc); }
UKL_INLINE uint32_t ukl_dot4_u8(uint32_t a, uint32_t b, uint32_t acc) { return ukl_dot4_u8_native(a, b, acc); }
#else
UKL_INLINE int32_t ukl_dot4_i8(uint32_t a, uint32_t b, int32_t acc) { return ukl_dot4_i8_ref(a, b, acc); }
UKL_INLINE uint32_t ukl_dot4_u8(uint32_t a, uint32_t b, uint32_t acc) { return ukl_dot4_u8_ref(a, b, acc); }
#endif

#if defined(UKL_NATIVE_DOT2_F16)
UKL_INLINE float ukl_dot2_f16(uint32_t a, uint32_t b, float acc) { return ukl_dot2_f16_native(a, b, acc); }
#else
// The reference's arithmetic, through the backend's conversion where it has one.
UKL_INLINE float ukl_dot2_f16(uint32_t a, uint32_t b, float acc)
{
    acc = fma(ukl_f16_to_f32(a & 0xffffu), ukl_f16_to_f32(b & 0xffffu), acc);
    return fma(ukl_f16_to_f32(a >> 16), ukl_f16_to_f32(b >> 16), acc);
}
#endif

#if defined(UKL_NATIVE_DOT2_BF16)
UKL_INLINE float ukl_dot2_bf16(uint32_t a, uint32_t b, float acc) { return ukl_dot2_bf16_native(a, b, acc); }
#else
UKL_INLINE float ukl_dot2_bf16(uint32_t a, uint32_t b, float acc) { return ukl_dot2_bf16_ref(a, b, acc); }
#endif
