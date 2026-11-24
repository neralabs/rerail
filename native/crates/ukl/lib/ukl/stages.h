// ukl/stages.h: scans, segmented scans and a stable radix sort as stages over tiles (decision 0653).
//
// A call is a few stages, each a phase of a work queue's program (`<ukl/workqueue.h>`; the C preprocessor's
// `pp_step`): every tile of a stage runs before any of the next, the tile that finishes a stage's last pushing the next
// stage's tiles. No tile waits for another. A stage's tiles are `UklTiles.tiles`, this one `UklTiles.tile`; a stage of
// a call runs over the same tiles as the call's other stages but one (`*_sums`, `*_carry`, and a small call: tile 0
// alone). A tile is a threadgroup, `lid` of `lsz` threads.
//
// Reads of what another tile wrote in a stage before go through `UKL_COHERENT` memory: a plain device load may be
// served from a cache the other tile's write did not reach (Q367).
#pragma once
#include <ukl/group.h>

// A stage's tiles, and where this thread is.
typedef struct UklTiles {
    uint32_t tiles, tile, lid, lsz;
} UklTiles;

// A tile's share of `n` items: `[first, end)`, contiguous, the shares in tile order.
UKL_INLINE uint32_t ukl_tiles_first(UklTiles t, uint32_t n, uint32_t tile)
{
    uint32_t per = (n + t.tiles - 1u) / t.tiles;
    uint64_t f = (uint64_t)per * tile;
    return f < n ? (uint32_t)f : n;
}

// At most this many values, a scan or a segmented scan is tile 0's alone: its tiles one after another, one stage.
#define UKL_STAGE_SMALL 4096u
// The scans one call runs at once at most.
#define UKL_STAGE_SCANS 4u

// One exclusive scan of a scans call (`ukl_scans_*`): `in`'s first `n` values into `out` (`in` may be `out`), the total into
// `*total`.
typedef struct UklScanJob {
    uint32_t n;
    UKL_COHERENT const uint32_t* in;
    UKL_COHERENT uint32_t* out;
    UKL_COHERENT uint32_t* total;
} UklScanJob;

// The values a thread takes of a scan's tile (`UKL_SCAN_VT`, consecutive): a tile is `lsz * UKL_SCAN_VT` values, each
// thread folding its own in order before the group's one operation over the threads. A GPU's is 1 (each thread a
// value, the group's reads coalesced); the CPU's backend more, as each of the group's operations switches all its
// threads (decision 0527 §3).
#ifndef UKL_SCAN_VT
#define UKL_SCAN_VT 1u
#endif

// Thread `lid`'s values of the tile from `base`, within `end`: `[*lo, *hi)`.
UKL_INLINE void ukl_scan_run(uint32_t base, uint32_t end, uint32_t lid, UKL_PRIVATE uint32_t* lo, UKL_PRIVATE uint32_t* hi)
{
    uint32_t a = base + lid * UKL_SCAN_VT, b = a + UKL_SCAN_VT;
    *lo = a < end ? a : end;
    *hi = b < end ? b : end;
}

// The sum of a group's tiles of `in[first..end)`, in every thread.
UKL_INLINE uint32_t ukl_stage_sum_tiles(UklTiles t, uint32_t first, uint32_t end, UKL_COHERENT const uint32_t* in, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    uint32_t acc = 0u;
    for (uint32_t base = first; base < end; base += t.lsz * UKL_SCAN_VT) {
        uint32_t lo, hi, v = 0u;
        ukl_scan_run(base, end, t.lid, &lo, &hi);
        for (uint32_t i = lo; i < hi; i++)
            v += in[i];
        acc += ukl_tg_sum(v, t.lid, t.lsz, scratch __UKL_ARGS);
    }
    return acc;
}

// A group's tiles of `in[first..end)` scanned from `run` into `out` (each value read before it is written, by the same
// thread); the run after them.
UKL_INLINE uint32_t ukl_stage_scan_tiles(UklTiles t, uint32_t first, uint32_t end, uint32_t run, UKL_COHERENT const uint32_t* in,
                                        UKL_COHERENT uint32_t* out, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    for (uint32_t base = first; base < end; base += t.lsz * UKL_SCAN_VT) {
        uint32_t lo, hi, v = 0u;
        ukl_scan_run(base, end, t.lid, &lo, &hi);
        for (uint32_t i = lo; i < hi; i++)
            v += in[i];
        UklScan s = ukl_tg_exclusive_sum(v, t.lid, t.lsz, scratch __UKL_ARGS);
        uint32_t acc = run + s.exclusive;
        for (uint32_t i = lo; i < hi; i++) {
            uint32_t x = in[i];
            out[i] = acc;
            acc += x;
        }
        run += s.total;
    }
    return run;
}

// --- The scans: `k` (at most `UKL_STAGE_SCANS`) exclusive scans at once ------------------------------------------
//
// Where every job holds at most `UKL_STAGE_SMALL` values (`ukl_scans_small`), one stage of tile 0 (`ukl_scans_one`).
// Else three stages over `tiles` (`sums`: `k * (tiles + 1)` words of device memory): each tile sums its share of each
// job (`ukl_scans_sum`); tile 0 scans each job's sums (`ukl_scans_sums`, one tile); each tile scans its shares from
// their offsets (`ukl_scans_out`). `scratch` is `UKL_TG_SCRATCH` words of threadgroup memory.

UKL_INLINE bool ukl_scans_small(UKL_PRIVATE const UklScanJob* jobs, uint32_t k)
{
    bool small = true;
    for (uint32_t j = 0u; j < k; j++)
        small = small && jobs[j].n <= UKL_STAGE_SMALL;
    return small;
}

// The small call's stage: tile 0 scans every job.
UKL_ONCE void ukl_scans_one(UklTiles t, UKL_PRIVATE const UklScanJob* jobs, uint32_t k, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    if (t.tile != 0u)
        return;
    for (uint32_t j = 0u; j < k; j++) {
        uint32_t run = ukl_stage_scan_tiles(t, 0u, jobs[j].n, 0u, jobs[j].in, jobs[j].out, scratch __UKL_ARGS);
        if (t.lid == 0u)
            *jobs[j].total = run;
    }
}

// Stage 1: each share's sum.
UKL_ONCE void ukl_scans_sum(UklTiles t, UKL_PRIVATE const UklScanJob* jobs, uint32_t k, UKL_COHERENT uint32_t* sums, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    uint32_t w = t.tiles + 1u;
    for (uint32_t j = 0u; j < k; j++) {
        uint32_t first = ukl_tiles_first(t, jobs[j].n, t.tile), end = ukl_tiles_first(t, jobs[j].n, t.tile + 1u);
        uint32_t acc = ukl_stage_sum_tiles(t, first, end, jobs[j].in, scratch __UKL_ARGS);
        if (t.lid == 0u)
            sums[j * w + t.tile] = acc;
    }
}

// Stage 2, one tile: each job's sums scanned (`sums[j * w + tiles]` its total), over the `tiles` of stages 1 and 3.
UKL_ONCE void ukl_scans_sums(UklTiles t, uint32_t tiles, uint32_t k, UKL_COHERENT uint32_t* sums, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    uint32_t w = tiles + 1u;
    for (uint32_t j = 0u; j < k; j++) {
        uint32_t run = ukl_stage_scan_tiles(t, 0u, tiles, 0u, sums + j * w, sums + j * w, scratch __UKL_ARGS);
        if (t.lid == 0u)
            sums[j * w + tiles] = run;
    }
}

// Stage 3: each share scanned from its offset (each value read before it is written, by the same thread).
UKL_ONCE void ukl_scans_out(UklTiles t, UKL_PRIVATE const UklScanJob* jobs, uint32_t k, UKL_COHERENT const uint32_t* sums, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    uint32_t w = t.tiles + 1u;
    for (uint32_t j = 0u; j < k; j++) {
        uint32_t first = ukl_tiles_first(t, jobs[j].n, t.tile), end = ukl_tiles_first(t, jobs[j].n, t.tile + 1u);
        ukl_stage_scan_tiles(t, first, end, sums[j * w + t.tile], jobs[j].in, jobs[j].out, scratch __UKL_ARGS);
        if (t.tile == 0u && t.lid == 0u)
            *jobs[j].total = sums[j * w + t.tiles];
    }
}

// --- The segmented scan's parts -------------------------------------------------------------------------------

// The segmented scans' operators.
#define UKL_SEG_ADD 0u
#define UKL_SEG_MAX 1u

// A segment's identity is 0 for both.
UKL_INLINE uint32_t ukl_seg_op(uint32_t op, uint32_t a, uint32_t b) { return op == UKL_SEG_ADD ? a + b : (a > b ? a : b); }

// `(fa, va)` then `(fb, vb)`: `vb` alone where `b` starts a segment.
UKL_INLINE uint32_t ukl_seg_join(uint32_t op, uint32_t va, uint32_t fb, uint32_t vb) { return fb != 0u ? vb : ukl_seg_op(op, va, vb); }

// The inclusive segmented scan of `(f, v)` over the SIMD group's lanes, in order; in `*fin` whether a head is at or
// before this lane. The backend's in one operation (`UKL_NATIVE_SG_SEG`), else Hillis–Steele over shuffles.
UKL_INLINE uint32_t ukl_sg_seg_inclusive(uint32_t op, uint32_t f, uint32_t v, uint32_t lane, UKL_PRIVATE uint32_t* fin __UKL_PARAMS)
{
#if defined(UKL_NATIVE_SG_SEG)
    (void)lane;
    return ukl_sg_seg_inclusive_native(op == UKL_SEG_ADD, f, v, fin __UKL_ARGS);
#else
    for (uint32_t d = 1u; d < UKL_SIMD_WIDTH; d <<= 1u) {
        uint32_t pv = ukl_sg_shuffle_up(v, d __UKL_ARGS), pf = ukl_sg_shuffle_up(f, d __UKL_ARGS);
        if (lane >= d) {
            v = ukl_seg_join(op, pv, f, v);
            f = f | pf;
        }
    }
    *fin = f;
    return v;
#endif
}

// `ukl_sg_seg_inclusive`, and in `*tv`, `*tf` the SIMD group's whole aggregate (its last lane's): the backend's in
// one operation (`UKL_NATIVE_SG_TOTALS`), else the scan and two shuffles.
UKL_INLINE uint32_t ukl_sg_seg_inclusive_total(uint32_t op, uint32_t f, uint32_t v, uint32_t lane, UKL_PRIVATE uint32_t* fin,
                                               UKL_PRIVATE uint32_t* tv, UKL_PRIVATE uint32_t* tf __UKL_PARAMS)
{
#if defined(UKL_NATIVE_SG_TOTALS)
    (void)lane;
    return ukl_sg_seg_inclusive_total_native(op == UKL_SEG_ADD, f, v, fin, tv, tf __UKL_ARGS);
#else
    uint32_t x = ukl_sg_seg_inclusive(op, f, v, lane, fin __UKL_ARGS);
    *tv = ukl_sg_shuffle(x, UKL_SIMD_WIDTH - 1u __UKL_ARGS);
    *tf = ukl_sg_shuffle(*fin, UKL_SIMD_WIDTH - 1u __UKL_ARGS);
    return x;
#endif
}

// The inclusive segmented scan of `(f, v)` over the group's threads, in order: each SIMD group's scan, then each
// thread's carry from the SIMD groups before it (their aggregates in `sv` and `sf`, a word a SIMD group of
// threadgroup memory). In `*fin` whether a head is at or before this thread, and in every thread the group's whole
// aggregate in `*tv`, `*tf`. Three waits of the group, whatever its size (one SIMD group: one).
UKL_INLINE uint32_t ukl_tg_seg_inclusive(uint32_t op, uint32_t f, uint32_t v, uint32_t lid, uint32_t lsz, UKL_LOCAL uint32_t* sv,
                                         UKL_LOCAL uint32_t* sf, UKL_PRIVATE uint32_t* fin, UKL_PRIVATE uint32_t* tv, UKL_PRIVATE uint32_t* tf __UKL_PARAMS)
{
    uint32_t lane = lid % UKL_SIMD_WIDTH, sg = lid / UKL_SIMD_WIDTH, groups = (lsz + UKL_SIMD_WIDTH - 1u) / UKL_SIMD_WIDTH;
    uint32_t fi = 0u;
    // A group of one SIMD group: its scan is the group's, and no threadgroup memory is used.
    if (groups == 1u) {
        uint32_t x = ukl_sg_seg_inclusive_total(op, f, v, lane, &fi, tv, tf __UKL_ARGS);
        *fin = fi;
        return x;
    }
    uint32_t x = ukl_sg_seg_inclusive(op, f, v, lane, &fi __UKL_ARGS);
    if (lane == UKL_SIMD_WIDTH - 1u || lid == lsz - 1u) {
        sv[sg] = x;
        sf[sg] = fi;
    }
    ukl_tg_barrier();
    uint32_t cv = 0u, cf = 0u, av = 0u, af = 0u;
    for (uint32_t k = 0u; k < groups; k++) {
        if (k == sg) {
            cv = av;
            cf = af;
        }
        av = ukl_seg_join(op, av, sf[k], sv[k]);
        af |= sf[k];
    }
    *fin = fi | cf;
    *tv = av;
    *tf = af;
    ukl_tg_barrier();
    return fi != 0u ? x : ukl_seg_op(op, cv, x);
}

// `ukl_tg_seg_inclusive`, and in `*ev`, `*ef` the thread before's (its exclusive; 0 for the first): the backend's in
// one operation where the group is one SIMD group (`UKL_NATIVE_SG_TOTALS`), else through `sv`, `sf` (`lsz` words each).
UKL_INLINE uint32_t ukl_tg_seg_scan2(uint32_t op, uint32_t f, uint32_t v, uint32_t lid, uint32_t lsz, UKL_LOCAL uint32_t* sv,
                                     UKL_LOCAL uint32_t* sf, UKL_PRIVATE uint32_t* fin, UKL_PRIVATE uint32_t* ev, UKL_PRIVATE uint32_t* ef,
                                     UKL_PRIVATE uint32_t* tv, UKL_PRIVATE uint32_t* tf __UKL_PARAMS)
{
#if defined(UKL_NATIVE_SG_TOTALS)
    if (lsz <= UKL_SIMD_WIDTH)
        return ukl_sg_seg_inclusive_ex_native(op == UKL_SEG_ADD, f, v, fin, ev, ef, tv, tf __UKL_ARGS);
#endif
    uint32_t x = ukl_tg_seg_inclusive(op, f, v, lid, lsz, sv, sf, fin, tv, tf __UKL_ARGS);
    ukl_tg_barrier();
    sv[lid] = x;
    sf[lid] = *fin;
    ukl_tg_barrier();
    *ev = lid > 0u ? sv[lid - 1u] : 0u;
    *ef = lid > 0u ? sf[lid - 1u] : 0u;
    ukl_tg_barrier();
    return x;
}

// A tile of a segmented scan from `base` (within `end`), from the carry `(*cv, *cf)`: written into `out` where `write`
// (each value read before it is written, by the same thread), and the carry after it. A thread's values are
// `UKL_SCAN_VT`, consecutive: it folds them, the group scans the threads' folds, and it writes them from the one
// before's.
UKL_INLINE void ukl_seg_tile(UklTiles t, uint32_t op, uint32_t base, uint32_t end, bool write, UKL_COHERENT const uint32_t* in,
                             UKL_COHERENT const uint32_t* heads, UKL_COHERENT uint32_t* out, UKL_LOCAL uint32_t* sv, UKL_LOCAL uint32_t* sf,
                             UKL_PRIVATE uint32_t* cv, UKL_PRIVATE uint32_t* cf __UKL_PARAMS)
{
    uint32_t lo, hi, fin, tv, tf;
    ukl_scan_run(base, end, t.lid, &lo, &hi);
#if UKL_SCAN_VT == 1
    bool live = lo < hi;
    uint32_t x = ukl_tg_seg_inclusive(op, live && heads[lo] != 0u ? 1u : 0u, live ? in[lo] : 0u, t.lid, t.lsz, sv, sf, &fin, &tv, &tf __UKL_ARGS);
    if (write && live)
        out[lo] = fin != 0u ? x : ukl_seg_op(op, *cv, x);
#else
    uint32_t lv = 0u, lf = 0u, ev = 0u, ef = 0u;
    for (uint32_t i = lo; i < hi; i++) {
        uint32_t h = heads[i] != 0u ? 1u : 0u;
        lv = ukl_seg_join(op, lv, h, in[i]);
        lf |= h;
    }
    if (write) {
        ukl_tg_seg_scan2(op, lf, lv, t.lid, t.lsz, sv, sf, &fin, &ev, &ef, &tv, &tf __UKL_ARGS);
        uint32_t acc = ef != 0u ? ev : ukl_seg_op(op, *cv, ev);
        for (uint32_t i = lo; i < hi; i++) {
            uint32_t v = in[i];
            acc = heads[i] != 0u ? v : ukl_seg_op(op, acc, v);
            out[i] = acc;
        }
    } else {
        ukl_tg_seg_inclusive(op, lf, lv, t.lid, t.lsz, sv, sf, &fin, &tv, &tf __UKL_ARGS);
    }
#endif
    *cv = ukl_seg_join(op, *cv, tf, tv);
    *cf |= tf;
}

// --- The segmented scan: `in`'s first `n` values into `out` (`in` may be `out`), inclusive, a segment starting at
// each nonzero `heads` word (`op`: `UKL_SEG_ADD` or `UKL_SEG_MAX`). At most `UKL_STAGE_SMALL` values, one stage of
// tile 0 (`ukl_seg_one`); else three: each tile's share's aggregate (`ukl_seg_agg`; `sums`: `2 * tiles` words, value
// then whether it holds a head), each tile's carry by one thread (`ukl_seg_carry`), each share from its carry
// (`ukl_seg_out`). `sv`, `sf` are `lsz` words each of threadgroup memory.

UKL_ONCE void ukl_seg_one(UklTiles t, uint32_t op, uint32_t n, UKL_COHERENT const uint32_t* in, UKL_COHERENT const uint32_t* heads,
                            UKL_COHERENT uint32_t* out, UKL_LOCAL uint32_t* sv, UKL_LOCAL uint32_t* sf __UKL_PARAMS)
{
    if (t.tile != 0u)
        return;
    uint32_t cv = 0u, cf = 0u;
    for (uint32_t base = 0u; base < n; base += t.lsz * UKL_SCAN_VT)
        ukl_seg_tile(t, op, base, n, true, in, heads, out, sv, sf, &cv, &cf __UKL_ARGS);
}

UKL_ONCE void ukl_seg_agg(UklTiles t, uint32_t op, uint32_t n, UKL_COHERENT const uint32_t* in, UKL_COHERENT const uint32_t* heads,
                            UKL_COHERENT uint32_t* out, UKL_COHERENT uint32_t* sums, UKL_LOCAL uint32_t* sv, UKL_LOCAL uint32_t* sf __UKL_PARAMS)
{
    uint32_t first = ukl_tiles_first(t, n, t.tile), end = ukl_tiles_first(t, n, t.tile + 1u);
    uint32_t av = 0u, af = 0u;
    for (uint32_t base = first; base < end; base += t.lsz * UKL_SCAN_VT)
        ukl_seg_tile(t, op, base, end, false, in, heads, out, sv, sf, &av, &af __UKL_ARGS);
    if (t.lid == 0u) {
        sums[2u * t.tile] = av;
        sums[2u * t.tile + 1u] = af;
    }
}

// One thread: each tile's carry, the aggregate of the tiles before it (in place), over the `tiles` of the other two.
UKL_INLINE void ukl_seg_carry(uint32_t op, uint32_t tiles, UKL_COHERENT uint32_t* sums)
{
    uint32_t cv = 0u, cf = 0u;
    for (uint32_t k = 0u; k < tiles; k++) {
        uint32_t v = sums[2u * k], f = sums[2u * k + 1u];
        sums[2u * k] = cv;
        sums[2u * k + 1u] = cf;
        cv = ukl_seg_join(op, cv, f, v);
        cf |= f;
    }
}

UKL_ONCE void ukl_seg_out(UklTiles t, uint32_t op, uint32_t n, UKL_COHERENT const uint32_t* in, UKL_COHERENT const uint32_t* heads,
                            UKL_COHERENT uint32_t* out, UKL_COHERENT const uint32_t* sums, UKL_LOCAL uint32_t* sv, UKL_LOCAL uint32_t* sf __UKL_PARAMS)
{
    uint32_t first = ukl_tiles_first(t, n, t.tile), end = ukl_tiles_first(t, n, t.tile + 1u);
    uint32_t cv = sums[2u * t.tile], cf = 0u;
    for (uint32_t base = first; base < end; base += t.lsz * UKL_SCAN_VT)
        ukl_seg_tile(t, op, base, end, true, in, heads, out, sv, sf, &cv, &cf __UKL_ARGS);
}

// --- The radix sort ---------------------------------------------------------------------------------------------

#include <ukl/radix.h>
#if UKL_SIMD_WIDTH >= 8
// The tile bodies over coherent rows: a pass reads what other tiles scattered.
#define UKL_RADIX_NAME(f) f##_u64_stage
#define UKL_RADIX_KEY uint64_t
#define UKL_RADIX_SPACE UKL_COHERENT
#include <ukl/body/radix_tile.h>

// Where a sort's rows end: in its first buffers when its digits (`bits` in bytes) are even.
UKL_INLINE bool ukl_sort_in_input(uint32_t bits) { return ((bits + 7u) / 8u) % 2u == 0u; }

// The threadgroup memory a sort's stages take: `h`, `counts`, `before`, `base`, `digits` (`ukl_radix_*_tile`'s;
// `digits` is `UKL_RADIX_SPILL` words), and `tile`, `UKL_SORT_TILE_WORDS` words (a tile's keys and values, for
// the sort of one tile).
#define UKL_SORT_TILE_WORDS (3u * UKL_RADIX_TILE)
// The words of `counts`, `h`, `before`, `base` and `digits` laid end to end (where one block holds them, and `tile`
// aliases it: no phase uses both).
#define UKL_SORT_RADIX_WORDS (UKL_RADIX_GROUPS * UKL_RADIX_DIGITS + 3u * UKL_RADIX_DIGITS + UKL_RADIX_SPILL)
// The larger of the two.
#define UKL_SORT_WORDS (UKL_SORT_TILE_WORDS > UKL_SORT_RADIX_WORDS ? UKL_SORT_TILE_WORDS : UKL_SORT_RADIX_WORDS)
typedef struct UklSortLocal {
    UKL_LOCAL uint32_t* h;
    UKL_LOCAL uint32_t* counts;
    UKL_LOCAL uint32_t* before;
    UKL_LOCAL uint32_t* base;
    UKL_LOCAL uint32_t* digits;
    UKL_LOCAL uint32_t* tile;
} UklSortLocal;

// `a` before `b` by their low `bits` bits, ties by place.
UKL_INLINE bool ukl_sort_before(uint32_t ah, uint32_t al, uint32_t ai, uint32_t bh, uint32_t bl, uint32_t bi)
{
    return ah < bh || (ah == bh && (al < bl || (al == bl && ai < bi)));
}

// --- The sort: a stable LSD sort of `keys`' first `n` by their low `bits` bits (a multiple of 8), with `values`
// (`iota`: each row's index before the sort, written by its first stage). Tiles of `UKL_RADIX_THREADS` threads.
// `keys2`, `values2` are the other buffers of each pass; `hist` is `256 * rtiles + 1` words (`rtiles`: `n` in
// `UKL_RADIX_TILE`s). The rows end in `keys`, `values` where `ukl_sort_in_input` says, else in `keys2`, `values2`.
// `low` (a multiple of 8, at most `bits`): the rows are already in the order of their low `low` bits, so only the
// digits above are passed (0: none is known).
// - Few rows (`ukl_sort_few`: `n <= UKL_RADIX_TILE`), over `n` in tiles of `lsz`: each tile holds every row in
//   threadgroup memory, and each thread ranks its row among them all and writes it at its rank in `keys2`, `values2`
//   (`ukl_sort_rank`); where they end in `keys`, a stage moves them back (`ukl_sort_back`).
// - More: the bits that vary among the rows (`ukl_sort_vary`, `sums` `4 * tiles` words; `ukl_sort_varying` reads them,
//   one thread), then each digit that varies passed: its tiles' histograms (`ukl_sort_hist`), their scan (a scan call
//   of `256 * rtiles` values, in place, its total at `hist[256 * rtiles]`), and the scatter (`ukl_sort_scatter`); the
//   passes' buffers alternate from `keys`. Where the last pass's rows are not where they end, `ukl_sort_back` moves
//   them.

UKL_INLINE bool ukl_sort_few(uint32_t n) { return n <= UKL_RADIX_TILE; }
UKL_INLINE uint64_t ukl_sort_mask(uint32_t bits) { return bits >= 64u ? ~(uint64_t)0 : (((uint64_t)1 << bits) - 1u); }

UKL_ONCE void ukl_sort_rank(UklTiles t, uint32_t n, uint32_t bits, bool iota, UKL_COHERENT const uint64_t* keys, UKL_COHERENT const uint32_t* values,
                              UKL_COHERENT uint64_t* keys2, UKL_COHERENT uint32_t* values2, UklSortLocal l)
{
    uint64_t mask = ukl_sort_mask(bits);
    UKL_LOCAL uint32_t* hi = l.tile;
    UKL_LOCAL uint32_t* lo = l.tile + UKL_RADIX_TILE;
    for (uint32_t r = t.lid; r < n; r += t.lsz) {
        uint64_t key = keys[r] & mask;
        hi[r] = (uint32_t)(key >> 32);
        lo[r] = (uint32_t)key;
    }
    ukl_tg_barrier();
    for (uint32_t row = t.tile * t.lsz + t.lid; row < n; row += t.tiles * t.lsz) {
        uint32_t rh = hi[row], rl = lo[row], rank = 0u;
        for (uint32_t o = 0u; o < n; o++)
            rank += ukl_sort_before(hi[o], lo[o], o, rh, rl, row) ? 1u : 0u;
        keys2[rank] = keys[row];
        values2[rank] = iota ? row : values[row];
    }
}

// The rows of `keys2`, `values2` (`from_second`) or `keys`, `values` moved to the other two, a share a tile.
UKL_ONCE void ukl_sort_back(UklTiles t, uint32_t n, bool from_second, UKL_COHERENT uint64_t* keys, UKL_COHERENT uint32_t* values,
                              UKL_COHERENT uint64_t* keys2, UKL_COHERENT uint32_t* values2)
{
    uint32_t rf = ukl_tiles_first(t, n, t.tile), re = ukl_tiles_first(t, n, t.tile + 1u);
    for (uint32_t r = rf + t.lid; r < re; r += t.lsz) {
        if (from_second) {
            keys[r] = keys2[r];
            values[r] = values2[r];
        } else {
            keys2[r] = keys[r];
            values2[r] = values[r];
        }
    }
}

UKL_ONCE void ukl_sort_vary(UklTiles t, uint32_t n, uint32_t bits, bool iota, UKL_COHERENT const uint64_t* keys, UKL_COHERENT uint32_t* values,
                              UKL_COHERENT uint32_t* sums, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    uint64_t mask = ukl_sort_mask(bits);
    uint32_t rf = ukl_tiles_first(t, n, t.tile), re = ukl_tiles_first(t, n, t.tile + 1u);
    uint32_t oh = 0u, ol = 0u, ah = ~0u, al = ~0u;
    for (uint32_t r = rf + t.lid; r < re; r += t.lsz) {
        uint64_t key = keys[r] & mask;
        oh |= (uint32_t)(key >> 32);
        ol |= (uint32_t)key;
        ah &= (uint32_t)(key >> 32);
        al &= (uint32_t)key;
        if (iota)
            values[r] = r;
    }
    oh = ukl_tg_or(oh, t.lid, t.lsz, scratch __UKL_ARGS);
    ol = ukl_tg_or(ol, t.lid, t.lsz, scratch __UKL_ARGS);
    ah = ukl_tg_and(ah, t.lid, t.lsz, scratch __UKL_ARGS);
    al = ukl_tg_and(al, t.lid, t.lsz, scratch __UKL_ARGS);
    if (t.lid == 0u) {
        sums[4u * t.tile] = oh;
        sums[4u * t.tile + 1u] = ol;
        sums[4u * t.tile + 2u] = re > rf ? ah : ~0u;
        sums[4u * t.tile + 3u] = re > rf ? al : ~0u;
    }
}

// One thread: the bits that vary among the rows, from `ukl_sort_vary`'s `tiles` sums.
UKL_INLINE uint64_t ukl_sort_varying(uint32_t tiles, UKL_COHERENT const uint32_t* sums)
{
    uint64_t any = 0u, all = ~(uint64_t)0;
    for (uint32_t k = 0u; k < tiles; k++) {
        any |= ((uint64_t)sums[4u * k] << 32) | (uint64_t)sums[4u * k + 1u];
        all &= ((uint64_t)sums[4u * k + 2u] << 32) | (uint64_t)sums[4u * k + 3u];
    }
    return any ^ all;
}

// The next digit past `p` (from `low / 8`) that varies, or `(bits + 7) / 8` where none is left.
UKL_INLINE uint32_t ukl_sort_next_digit(uint64_t vary, uint32_t p, uint32_t bits)
{
    uint32_t end = (bits + 7u) / 8u;
    while (p < end && ((vary >> (8u * p)) & 0xffu) == 0u)
        p++;
    return p;
}

UKL_INLINE UklRadixArgs ukl_sort_args(uint32_t n, uint32_t digit)
{
    UklRadixArgs args;
    args.n = n;
    args.shift = 8u * digit;
    args.tiles = (n + UKL_RADIX_TILE - 1u) / UKL_RADIX_TILE;
    return args;
}

UKL_ONCE void ukl_sort_hist(UklTiles t, uint32_t n, uint32_t digit, UKL_COHERENT const uint64_t* ki, UKL_COHERENT uint32_t* hist, UklSortLocal l)
{
    UklRadixArgs args = ukl_sort_args(n, digit);
    uint32_t first = ukl_tiles_first(t, args.tiles, t.tile), end = ukl_tiles_first(t, args.tiles, t.tile + 1u);
    for (uint32_t tile = first; tile < end; tile++)
        ukl_radix_hist_tile_u64_stage(ki, hist, args, l.h, tile, t.lid, t.lsz);
}

UKL_ONCE void ukl_sort_scatter(UklTiles t, uint32_t n, uint32_t digit, UKL_COHERENT const uint64_t* ki, UKL_COHERENT const uint32_t* vi,
                                 UKL_COHERENT const uint32_t* hist, UKL_COHERENT uint64_t* ko, UKL_COHERENT uint32_t* vo, UklSortLocal l __UKL_PARAMS)
{
    UklRadixArgs args = ukl_sort_args(n, digit);
    uint32_t first = ukl_tiles_first(t, args.tiles, t.tile), end = ukl_tiles_first(t, args.tiles, t.tile + 1u);
    for (uint32_t tile = first; tile < end; tile++)
        ukl_radix_scatter_tile_u64_stage(ki, vi, hist, ko, vo, args, l.counts, l.before, l.base, l.digits, tile, t.lid, t.lsz __UKL_ARGS);
}
#endif
