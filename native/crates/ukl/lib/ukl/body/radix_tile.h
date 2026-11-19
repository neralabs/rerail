// ukl/body/radix_tile.h: ukl/radix.h's passes over one key type (decision 0400 §3.5: a type in C is a parameter of
// an included body). Included once per key type, with, defined first:
// - `UKL_RADIX_NAME(f)`: the instance's name of each function `f` (`f##_u32`, ...);
// - `UKL_RADIX_KEY`: the key type.
// They are undefined at the end. Not `#pragma once`.

#if !defined(UKL_RADIX_NAME) || !defined(UKL_RADIX_KEY)
#error "ukl/body/radix_tile.h needs UKL_RADIX_NAME and UKL_RADIX_KEY (ukl/radix.h)"
#endif

UKL_INLINE uint32_t UKL_RADIX_NAME(ukl_radix_digit)(UKL_RADIX_KEY key, uint32_t shift) { return (uint32_t)(key >> shift) & (UKL_RADIX_DIGITS - 1u); }

// The tile's digit histogram, into `hist` digit-major. `h`: `UKL_RADIX_DIGITS` threadgroup words.
UKL_INLINE void UKL_RADIX_NAME(ukl_radix_hist_tile)(UKL_GLOBAL const UKL_RADIX_KEY* keys, UKL_GLOBAL uint32_t* hist, UklRadixArgs args, UKL_LOCAL uint32_t* h, uint32_t tile, uint32_t t)
{
    h[t] = 0u;
    ukl_tg_barrier();
    for (uint32_t k = 0u; k < UKL_RADIX_ROUNDS; k++) {
        uint32_t r = ukl_radix_row(tile, k, t);
        if (r < args.n)
            ukl_atomic_add_local(&h[UKL_RADIX_NAME(ukl_radix_digit)(keys[r], args.shift)], 1u);
    }
    ukl_tg_barrier();
    hist[t * args.tiles + tile] = h[t];
}

// The tile's rows at their places. `counts`: `UKL_RADIX_GROUPS * UKL_RADIX_DIGITS` threadgroup words (the round's
// rows of each digit, by SIMD group); `before`, `base`: `UKL_RADIX_DIGITS` each (the digit's rows in the earlier
// rounds; its offset for the tile).
UKL_INLINE void UKL_RADIX_NAME(ukl_radix_scatter_tile)(UKL_GLOBAL const UKL_RADIX_KEY* keys, UKL_GLOBAL const uint32_t* values, UKL_GLOBAL const uint32_t* offsets,
                                       UKL_GLOBAL UKL_RADIX_KEY* outKeys, UKL_GLOBAL uint32_t* outValues, UklRadixArgs args, UKL_LOCAL uint32_t* counts,
                                       UKL_LOCAL uint32_t* before, UKL_LOCAL uint32_t* base, uint32_t tile, uint32_t t)
{
    uint32_t lane = t % UKL_SIMD_WIDTH, sg = t / UKL_SIMD_WIDTH;
    base[t] = offsets[t * args.tiles + tile];
    before[t] = 0u;
    for (uint32_t k = 0u; k < UKL_RADIX_ROUNDS; k++) {
        uint32_t r = ukl_radix_row(tile, k, t);
        bool live = r < args.n;
        UKL_RADIX_KEY key = live ? keys[r] : (UKL_RADIX_KEY)0;
        // A row past the end has a digit of its own, equal to no live row's.
        uint32_t d = live ? UKL_RADIX_NAME(ukl_radix_digit)(key, args.shift) : UKL_RADIX_DIGITS;
        uint32_t m = ukl_sg_match(d __UKL_ARGS);
        uint32_t rank = ukl_rank32(m, lane);
        bool last = (m >> lane) == 1u;
        for (uint32_t s = 0u; s < UKL_RADIX_GROUPS; s++)
            counts[s * UKL_RADIX_DIGITS + t] = 0u;
        ukl_tg_barrier();
        if (live && last)
            counts[sg * UKL_RADIX_DIGITS + d] = rank + 1u;
        ukl_tg_barrier();
        // Thread `t` turns digit `t`'s counts by SIMD group into the groups' starts, and its round's total.
        uint32_t acc = 0u;
        for (uint32_t s = 0u; s < UKL_RADIX_GROUPS; s++) {
            uint32_t c = counts[s * UKL_RADIX_DIGITS + t];
            counts[s * UKL_RADIX_DIGITS + t] = acc;
            acc += c;
        }
        ukl_tg_barrier();
        if (live) {
            uint32_t at = base[d] + before[d] + counts[sg * UKL_RADIX_DIGITS + d] + rank;
            outKeys[at] = key;
            outValues[at] = values[r];
        }
        ukl_tg_barrier();
        before[t] += acc;
        ukl_tg_barrier();
    }
}

#undef UKL_RADIX_NAME
#undef UKL_RADIX_KEY
