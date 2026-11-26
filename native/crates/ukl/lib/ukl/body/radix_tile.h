// ukl/body/radix_tile.h: ukl/radix.h's passes over one key type (decision 0440 §3.5: a type in C is a parameter of
// an included body). Included once per key type, with, defined first:
// - `UKL_RADIX_NAME(f)`: the instance's name of each function `f` (`f##_u32`, ...);
// - `UKL_RADIX_KEY`: the key type;
// - optionally `UKL_RADIX_SPACE`: the address space of the rows, the histograms and the offsets (`UKL_GLOBAL`;
//   `UKL_COHERENT` where other threadgroups of the dispatch wrote them, as a program's stages do, ukl/stages.h).
// They are undefined at the end. Not `#pragma once`.

#if !defined(UKL_RADIX_NAME) || !defined(UKL_RADIX_KEY)
#error "ukl/body/radix_tile.h needs UKL_RADIX_NAME and UKL_RADIX_KEY (ukl/radix.h)"
#endif
#ifndef UKL_RADIX_SPACE
#define UKL_RADIX_SPACE UKL_GLOBAL
#endif

UKL_INLINE uint32_t UKL_RADIX_NAME(ukl_radix_digit)(UKL_RADIX_KEY key, uint32_t shift) { return (uint32_t)(key >> shift) & (UKL_RADIX_DIGITS - 1u); }

// The tile's digit histogram, into `hist` digit-major. `h`: `UKL_RADIX_DIGITS` threadgroup words. `t` of `lsz`
// threads (`UKL_RADIX_THREADS`, or a divisor of it down to `UKL_RADIX_THREADS / UKL_RADIX_VT_MAX`: each thread then
// the tile's threads `t`, `t + lsz`, …).
UKL_INLINE void UKL_RADIX_NAME(ukl_radix_hist_tile)(UKL_RADIX_SPACE const UKL_RADIX_KEY* keys, UKL_RADIX_SPACE uint32_t* hist, UklRadixArgs args, UKL_LOCAL uint32_t* h, uint32_t tile, uint32_t t, uint32_t lsz)
{
#if UKL_RADIX_VT_MAX == 1u
    (void)lsz;
    h[t] = 0u;
    ukl_tg_barrier();
    for (uint32_t k = 0u; k < UKL_RADIX_ROUNDS; k++) {
        uint32_t r = ukl_radix_row(tile, k, t);
        if (r < args.n)
            ukl_atomic_add_local(&h[UKL_RADIX_NAME(ukl_radix_digit)(keys[r], args.shift)], 1u);
    }
    ukl_tg_barrier();
    hist[t * args.tiles + tile] = h[t];
#else
    for (uint32_t v = t; v < UKL_RADIX_THREADS; v += lsz)
        h[v] = 0u;
    ukl_tg_barrier();
    for (uint32_t v = t; v < UKL_RADIX_THREADS; v += lsz)
        for (uint32_t k = 0u; k < UKL_RADIX_ROUNDS; k++) {
            uint32_t r = ukl_radix_row(tile, k, v);
            if (r < args.n)
                ukl_atomic_add_local(&h[UKL_RADIX_NAME(ukl_radix_digit)(keys[r], args.shift)], 1u);
        }
    ukl_tg_barrier();
    for (uint32_t v = t; v < UKL_RADIX_THREADS; v += lsz)
        hist[v * args.tiles + tile] = h[v];
#endif
}

// The tile's rows at their places (its keys alone where `outValues` is null). `counts`: `UKL_RADIX_GROUPS * UKL_RADIX_DIGITS` threadgroup words (the round's
// rows of each digit, by rank group); `before`, `base`: `UKL_RADIX_DIGITS` each (the digit's rows in the earlier
// rounds; its offset for the tile); `digits`: `UKL_RADIX_SPILL` (each thread's digit, where a rank group is not a SIMD
// group). `t` of `lsz` threads, as the histogram's: a thread's SIMD group runs the tile's SIMD groups of its threads
// one after another, each a SIMD-group operation of them all (rank groups that are SIMD groups only).
UKL_INLINE void UKL_RADIX_NAME(ukl_radix_scatter_tile)(UKL_RADIX_SPACE const UKL_RADIX_KEY* keys, UKL_RADIX_SPACE const uint32_t* values, UKL_RADIX_SPACE const uint32_t* offsets,
                                       UKL_RADIX_SPACE UKL_RADIX_KEY* outKeys, UKL_RADIX_SPACE uint32_t* outValues, UklRadixArgs args, UKL_LOCAL uint32_t* counts,
                                       UKL_LOCAL uint32_t* before, UKL_LOCAL uint32_t* base, UKL_LOCAL uint32_t* digits, uint32_t tile, uint32_t t, uint32_t lsz __UKL_PARAMS)
{
#if UKL_RADIX_VT_MAX == 1u
    (void)lsz;
    uint32_t me = t % UKL_RADIX_RG, sg = t / UKL_RADIX_RG;
    base[t] = offsets[t * args.tiles + tile];
    before[t] = 0u;
    for (uint32_t k = 0u; k < UKL_RADIX_ROUNDS; k++) {
        uint32_t r = ukl_radix_row(tile, k, t);
        bool live = r < args.n;
        UKL_RADIX_KEY key = live ? keys[r] : (UKL_RADIX_KEY)0;
        // A row past the end has a digit of its own, equal to no live row's.
        uint32_t d = live ? UKL_RADIX_NAME(ukl_radix_digit)(key, args.shift) : UKL_RADIX_DIGITS;
#if UKL_RADIX_MATCHED
        ukl_mask_t m = ukl_sg_match(d __UKL_ARGS);
        uint32_t rank = ukl_mask_rank(m, me);
        bool last = (m >> me) == 1u;
        (void)digits;
#else
        digits[t] = d;
        ukl_tg_barrier();
        uint32_t rank = 0u;
        bool last = true;
        for (uint32_t j = 0u; j < UKL_RADIX_RG; j++) {
            uint32_t dj = digits[t - me + j];
            rank += (uint32_t)(dj == d && j < me);
            last = last && !(dj == d && j > me);
        }
#endif
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
            if (outValues != 0) outValues[at] = values[r];
        }
        ukl_tg_barrier();
        before[t] += acc;
        ukl_tg_barrier();
    }
#else
    uint32_t nv = UKL_RADIX_THREADS / lsz;
#if UKL_RADIX_MATCHED
    uint32_t lane = t % UKL_SIMD_WIDTH;
    (void)digits;
#endif
    for (uint32_t v = t; v < UKL_RADIX_THREADS; v += lsz) {
        base[v] = offsets[v * args.tiles + tile];
        before[v] = 0u;
    }
    for (uint32_t k = 0u; k < UKL_RADIX_ROUNDS; k++) {
        UKL_RADIX_KEY key[UKL_RADIX_VT_MAX];
        uint32_t d[UKL_RADIX_VT_MAX], rank[UKL_RADIX_VT_MAX], row[UKL_RADIX_VT_MAX], acc[UKL_RADIX_VT_MAX];
        bool live[UKL_RADIX_VT_MAX], last[UKL_RADIX_VT_MAX];
        for (uint32_t m = 0u; m < nv; m++) {
            uint32_t v = t + m * lsz;
            row[m] = ukl_radix_row(tile, k, v);
            live[m] = row[m] < args.n;
            key[m] = live[m] ? keys[row[m]] : (UKL_RADIX_KEY)0;
            d[m] = live[m] ? UKL_RADIX_NAME(ukl_radix_digit)(key[m], args.shift) : UKL_RADIX_DIGITS;
#if UKL_RADIX_MATCHED
            // The tile's SIMD group of `v` is the group of this thread's own, its `m`-th: one match of them all.
            ukl_mask_t mm = ukl_sg_match(d[m] __UKL_ARGS);
            rank[m] = ukl_mask_rank(mm, lane);
            last[m] = (mm >> lane) == 1u;
#else
            digits[v] = d[m];
#endif
            for (uint32_t s = 0u; s < UKL_RADIX_GROUPS; s++)
                counts[s * UKL_RADIX_DIGITS + v] = 0u;
        }
        ukl_tg_barrier();
#if !UKL_RADIX_MATCHED
        // A rank group's digits read from threadgroup memory, as one thread a tile thread does (above).
        for (uint32_t m = 0u; m < nv; m++) {
            uint32_t v = t + m * lsz, me = v % UKL_RADIX_RG;
            rank[m] = 0u;
            last[m] = true;
            for (uint32_t j = 0u; j < UKL_RADIX_RG; j++) {
                uint32_t dj = digits[v - me + j];
                rank[m] += (uint32_t)(dj == d[m] && j < me);
                last[m] = last[m] && !(dj == d[m] && j > me);
            }
        }
#endif
        for (uint32_t m = 0u; m < nv; m++)
            if (live[m] && last[m])
                counts[((t + m * lsz) / UKL_RADIX_RG) * UKL_RADIX_DIGITS + d[m]] = rank[m] + 1u;
        ukl_tg_barrier();
        for (uint32_t m = 0u; m < nv; m++) {
            uint32_t v = t + m * lsz, a = 0u;
            for (uint32_t s = 0u; s < UKL_RADIX_GROUPS; s++) {
                uint32_t c = counts[s * UKL_RADIX_DIGITS + v];
                counts[s * UKL_RADIX_DIGITS + v] = a;
                a += c;
            }
            acc[m] = a;
        }
        ukl_tg_barrier();
        for (uint32_t m = 0u; m < nv; m++)
            if (live[m]) {
                uint32_t at = base[d[m]] + before[d[m]] + counts[((t + m * lsz) / UKL_RADIX_RG) * UKL_RADIX_DIGITS + d[m]] + rank[m];
                outKeys[at] = key[m];
                if (outValues != 0) outValues[at] = values[row[m]];
            }
        ukl_tg_barrier();
        for (uint32_t m = 0u; m < nv; m++)
            before[t + m * lsz] += acc[m];
        ukl_tg_barrier();
    }
#endif
}

#undef UKL_RADIX_NAME
#undef UKL_RADIX_KEY
#undef UKL_RADIX_SPACE
