// ukl/body/histogram_part.h: ukl/histogram.h's counting over one code type (decision 0400 §3.5: a type in C is a
// parameter of an included body). Included once per code type, with, defined first:
// - `UKL_HIST_NAME(f)`: the instance's name of each function `f` (`f##_u16`, ...);
// - `UKL_HIST_CODE`: the code type.
// They are undefined at the end. Not `#pragma once`.

#if !defined(UKL_HIST_NAME) || !defined(UKL_HIST_CODE)
#error "ukl/body/histogram_part.h needs UKL_HIST_NAME and UKL_HIST_CODE (ukl/histogram.h)"
#endif

// Adds to `table[c - base]` each code `c` of `codes[a..b]` in `base..base + len`; whether any code was at or above
// `limit` (seen by this thread).
UKL_INLINE bool UKL_HIST_NAME(ukl_tg_count_window)(UKL_LOCAL uint32_t* table, UKL_GLOBAL const UKL_HIST_CODE* codes, uint32_t a, uint32_t b, uint32_t base, uint32_t len,
                                    uint32_t limit, uint32_t lid, uint32_t lsz)
{
    bool over = false;
    for (uint32_t e = a + lid; e < b; e += lsz) {
        uint32_t code = (uint32_t)codes[e];
        uint32_t c = code - base;
        if (c < len)
            ukl_atomic_add_local(&table[c], 1u);
        over = over || code >= limit;
    }
    return over;
}

// Partition `p`'s codes (`codes[offsets[p]..offsets[p + 1]]`) counted into `H[p * width + c]`.
UKL_INLINE void UKL_HIST_NAME(ukl_histogram_part)(UKL_GLOBAL const UKL_HIST_CODE* codes, UKL_GLOBAL const uint32_t* offsets, UKL_GLOBAL uint32_t* H, UKL_GLOBAL uint32_t* bad,
                                   uint32_t width, UKL_LOCAL uint32_t* table, uint32_t p, uint32_t lid, uint32_t lsz)
{
    uint32_t a = offsets[p], b = offsets[p + 1u];
    UKL_GLOBAL uint32_t* out = H + (uint64_t)p * width;
    for (uint32_t base = 0u; base < width; base += UKL_HIST_TABLE) {
        uint32_t len = width - base < UKL_HIST_TABLE ? width - base : UKL_HIST_TABLE;
        ukl_tg_fill(table, len, 0u, lid, lsz);
        ukl_tg_barrier();
        if (UKL_HIST_NAME(ukl_tg_count_window)(table, codes, a, b, base, len, width, lid, lsz) && base == 0u)
            ukl_atomic_store(bad, 1u);
        ukl_tg_barrier();
        for (uint32_t i = lid; i < len; i += lsz)
            out[base + i] = table[i];
        ukl_tg_barrier();
    }
}

#undef UKL_HIST_NAME
#undef UKL_HIST_CODE
