// ukl/body/lz4_decode.h: the LZ4 decoder of ukl/lz4.h over one output (decision 0400 §3.5: a type in C is a
// parameter of an included body, not a template). Included once per output, with, defined first:
// - `UKL_LZ4_NAME(f)`: the instance's name of each function `f` (`f` itself for the contiguous output);
// - `UKL_LZ4_OUT`: the output's type;
// - `UKL_LZ4_AT(out, p)`: the address of output byte `p`.
// They are undefined at the end. Not `#pragma once`.

#if !defined(UKL_LZ4_NAME) || !defined(UKL_LZ4_OUT) || !defined(UKL_LZ4_AT)
#error "ukl/body/lz4_decode.h needs UKL_LZ4_NAME, UKL_LZ4_OUT and UKL_LZ4_AT (ukl/lz4.h)"
#endif

// `n` bytes from `src` to output bytes `op..op + n`, the lanes in turn.
UKL_INLINE void UKL_LZ4_NAME(ukl_sg_copy_out)(UKL_LZ4_OUT out, uint32_t op, UKL_GLOBAL const uint8_t* src, uint32_t n, uint32_t lane)
{
    for (uint32_t i = lane; i < n; i += UKL_SIMD_WIDTH)
        *UKL_LZ4_AT(out, op + i) = src[i];
}

// `n` output bytes from `dist` back to `op..op + n` (they may overlap), in rounds a round's reads were written before.
UKL_INLINE void UKL_LZ4_NAME(ukl_sg_copy_back)(UKL_LZ4_OUT out, uint32_t op, uint32_t dist, uint32_t n, uint32_t lane)
{
    uint32_t step = dist < UKL_SIMD_WIDTH ? dist : UKL_SIMD_WIDTH;
    for (uint32_t at = 0u; at < n; at += step) {
        uint32_t i = at + lane;
        if (lane < step && i < n)
            *UKL_LZ4_AT(out, op + i) = *UKL_LZ4_AT(out, op + i - dist);
        ukl_sg_barrier();
    }
}

// The block `s[0..len]` decoded to `out`: the bytes written, or `UKL_NONE` on a malformed block or one that would
// write past `cap` or read before its output's start.
UKL_INLINE uint32_t UKL_LZ4_NAME(ukl_lz4_decode_sg)(UKL_GLOBAL const uint8_t* s, uint32_t len, UKL_LZ4_OUT out, uint32_t cap, uint32_t lane)
{
    uint32_t ip = 0u, op = 0u;
    while (true) {
        if (ip >= len)
            return UKL_NONE;
        uint32_t tok = s[ip++];
        uint32_t lit = tok >> 4;
        if (lit == 15u && !ukl_lz4_length(s, &ip, len, &lit))
            return UKL_NONE;
        if (ip + lit > len || op + lit > cap)
            return UKL_NONE;
        UKL_LZ4_NAME(ukl_sg_copy_out)(out, op, s + ip, lit, lane);
        ip += lit;
        op += lit;
        // The last sequence is literals alone, and ends the block.
        if (ip == len)
            return op;
        if (ip + 2u > len)
            return UKL_NONE;
        uint32_t dist = (uint32_t)s[ip] | ((uint32_t)s[ip + 1u] << 8);
        ip += 2u;
        uint32_t ml = tok & 15u;
        if (ml == 15u && !ukl_lz4_length(s, &ip, len, &ml))
            return UKL_NONE;
        ml += 4u;
        if (dist == 0u || dist > op || op + ml > cap)
            return UKL_NONE;
        ukl_sg_barrier();
        UKL_LZ4_NAME(ukl_sg_copy_back)(out, op, dist, ml, lane);
        op += ml;
    }
}

#undef UKL_LZ4_NAME
#undef UKL_LZ4_OUT
#undef UKL_LZ4_AT
