// ukl/grid.h: a persistent grid's device-wide barrier, and the primitives written as its phases (decision 0468 §4).
//
// A persistent grid is G threadgroups resident together, each walking its own share of the work. Between two of its
// phases, every group waits for every other at `ukl_grid_barrier`: the writes of every group before it are seen by
// every group after it. So a primitive that took a dispatch a level (a scan's tile sums, their scan, the tiles from
// their offsets) is a few phases of one dispatch, and its count is read on the device.
//
// Metal promises no progress between threadgroups, and no backend promises that G groups are resident together. So
// the wait is bounded (0027: no wait without a bound): a group that has polled `UKL_GRID_SPINS` times sets the grid's
// failure word and goes on; every barrier after it returns false at once, in every group, and the caller ends its
// work. The host then runs the work again with fewer groups (G/2): a choice made for the next run, never between
// phases.
//
// `UklGrid.ctl`: three words, zero before the dispatch: 0 the groups arrived at the current barrier; 1 its
// generation, twice over, with the failure bit (bit 0); 2 the failure word, for a phase's quick test. A barrier's
// outcome is one for every group: the last to arrive steps the generation only by a compare-and-swap that finds no
// failure bit, and a group past its bound sets the bit only by a compare-and-swap that finds the generation it waited
// on. So either every group passes a barrier or none does. One form on every backend (CUDA's too, until the device
// layer makes cooperative launches).
//
// Reads of what another group wrote before a barrier go through `UKL_COHERENT` memory (`ukl_ldc_*`, or a coherent
// pointer): a plain device load may be served from a cache the other group's write did not reach.

#pragma once
#include <ukl/group.h>

// A barrier's polls at most, a group's first thread polling.
#ifndef UKL_GRID_SPINS
#define UKL_GRID_SPINS 1000000u
#endif

typedef struct UklGrid {
    UKL_GLOBAL uint32_t* ctl;
    // The grid's groups, this group, its thread and its size.
    uint32_t groups, group, lid, lsz;
} UklGrid;

// Whether the grid has failed a barrier (in every thread alike after a barrier).
UKL_INLINE bool ukl_grid_failed(UklGrid g) { return ukl_atomic_load(&g.ctl[2]) != 0u; }

// Waits for every group of the grid: true when all arrived, false when the bound was reached (here or in another
// group) or the grid failed before; the same answer in every group. Every thread of every group calls it, the same
// number of times. `flag` is a word of threadgroup memory.
UKL_INLINE bool ukl_grid_barrier(UklGrid g, UKL_LOCAL uint32_t* flag)
{
    // The group's writes, done.
    ukl_tg_barrier_device();
    if (g.lid == 0u) {
        // The generation before this group arrives: it cannot step until every group, this one too, has.
        uint32_t gen = ukl_atomic_load(&g.ctl[1]);
        uint32_t failed = gen & 1u;
        if (failed == 0u) {
            ukl_fence_device();
            if (ukl_atomic_add(&g.ctl[0], 1u) + 1u == g.groups) {
                ukl_atomic_store(&g.ctl[0], 0u);
                ukl_fence_device();
                failed = ukl_atomic_cas(&g.ctl[1], gen, gen + 2u) == gen ? 0u : 1u;
            } else {
                uint32_t now = gen, spins = 0u;
                while (now == gen && spins < UKL_GRID_SPINS) {
                    ukl_poll_yield();
                    now = ukl_atomic_load(&g.ctl[1]);
                    spins++;
                }
                if (now == gen) {
                    // The bound: fail the barrier, unless it stepped meanwhile.
                    uint32_t seen = ukl_atomic_cas(&g.ctl[1], gen, gen | 1u);
                    now = seen == gen ? (gen | 1u) : seen;
                }
                failed = now & 1u;
            }
            if (failed != 0u)
                ukl_atomic_store(&g.ctl[2], 1u);
            ukl_fence_device();
        }
        *flag = failed;
    }
    ukl_tg_barrier_device();
    return *flag == 0u;
}

// A group's share of `n` items: `[first, end)`, contiguous, the groups' shares in order.
UKL_INLINE uint32_t ukl_grid_first(UklGrid g, uint32_t n, uint32_t group)
{
    uint32_t per = (n + g.groups - 1u) / g.groups;
    uint64_t f = (uint64_t)per * group;
    return f < n ? (uint32_t)f : n;
}

// At most this many values, a scan or a segmented scan is group 0's alone: its tiles one after another, with no
// barrier but the last (decision 0468, the fixed latency). Every group reads the same count, so all take the same path.
#define UKL_GRID_SMALL 4096u
// The scans `ukl_grid_scans` runs at once at most.
#define UKL_GRID_SCANS 4u

// One exclusive scan of `ukl_grid_scans`: `in`'s first `n` values into `out` (`in` may be `out`), the total into
// `*total`.
typedef struct UklScanJob {
    uint32_t n;
    UKL_COHERENT const uint32_t* in;
    UKL_COHERENT uint32_t* out;
    UKL_COHERENT uint32_t* total;
} UklScanJob;

// A group's tiles of `in[first..end)` scanned from `run` into `out`; the run after them.
UKL_INLINE uint32_t ukl_grid_scan_tiles(UklGrid g, uint32_t first, uint32_t end, uint32_t run, UKL_COHERENT const uint32_t* in,
                                        UKL_COHERENT uint32_t* out, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    for (uint32_t base = first; base < end; base += g.lsz) {
        uint32_t i = base + g.lid;
        uint32_t v = i < end ? in[i] : 0u;
        UklScan s = ukl_tg_exclusive_sum(v, g.lid, g.lsz, scratch __UKL_ARGS);
        if (i < end)
            out[i] = run + s.exclusive;
        run += s.total;
    }
    return run;
}

// The `k` (at most `UKL_GRID_SCANS`) exclusive scans of `jobs`, sharing the grid's barriers. `sums` is
// `k * (groups + 1)` words of device memory; `scratch` is `UKL_TG_SCRATCH` words of threadgroup memory. Where every
// job holds at most `UKL_GRID_SMALL` values, group 0 scans them all, and one barrier ends it. Else three phases: each
// group sums its share of each job; group 0 scans each job's sums (`sums[groups]` its total); each group scans its
// shares from their offsets. False when a barrier failed (the outputs are then partial). Ends with a barrier: every
// output is seen by every group.
UKL_INLINE bool ukl_grid_scans(UklGrid g, UKL_PRIVATE const UklScanJob* jobs, uint32_t k, UKL_COHERENT uint32_t* sums,
                               UKL_LOCAL uint32_t* scratch, UKL_LOCAL uint32_t* flag __UKL_PARAMS)
{
    bool small = true;
    for (uint32_t j = 0u; j < k; j++)
        small = small && jobs[j].n <= UKL_GRID_SMALL;
    if (small) {
        if (g.group == 0u) {
            for (uint32_t j = 0u; j < k; j++) {
                uint32_t run = ukl_grid_scan_tiles(g, 0u, jobs[j].n, 0u, jobs[j].in, jobs[j].out, scratch __UKL_ARGS);
                if (g.lid == 0u)
                    *jobs[j].total = run;
            }
        }
        return ukl_grid_barrier(g, flag);
    }
    uint32_t w = g.groups + 1u;
    // 1. Each share's sum, a group's tile at a time.
    for (uint32_t j = 0u; j < k; j++) {
        uint32_t first = ukl_grid_first(g, jobs[j].n, g.group), end = ukl_grid_first(g, jobs[j].n, g.group + 1u);
        uint32_t acc = 0u;
        for (uint32_t base = first; base < end; base += g.lsz) {
            uint32_t i = base + g.lid;
            acc += ukl_tg_sum(i < end ? jobs[j].in[i] : 0u, g.lid, g.lsz, scratch __UKL_ARGS);
        }
        if (g.lid == 0u)
            sums[j * w + g.group] = acc;
    }
    if (!ukl_grid_barrier(g, flag))
        return false;
    // 2. Group 0 scans each job's sums, a tile at a time.
    if (g.group == 0u) {
        for (uint32_t j = 0u; j < k; j++) {
            uint32_t run = ukl_grid_scan_tiles(g, 0u, g.groups, 0u, sums + j * w, sums + j * w, scratch __UKL_ARGS);
            if (g.lid == 0u)
                sums[j * w + g.groups] = run;
        }
    }
    if (!ukl_grid_barrier(g, flag))
        return false;
    // 3. Each share, scanned from its offset (each value read before it is written, by the same thread).
    for (uint32_t j = 0u; j < k; j++) {
        uint32_t first = ukl_grid_first(g, jobs[j].n, g.group), end = ukl_grid_first(g, jobs[j].n, g.group + 1u);
        ukl_grid_scan_tiles(g, first, end, sums[j * w + g.group], jobs[j].in, jobs[j].out, scratch __UKL_ARGS);
        if (g.group == 0u && g.lid == 0u)
            *jobs[j].total = sums[j * w + g.groups];
    }
    return ukl_grid_barrier(g, flag);
}

// The exclusive scan of `in`'s first `n` values into `out`, its total into `*total` (`out + n` for an offsets
// array): `ukl_grid_scans` of one job. `sums` is `groups + 1` words.
UKL_INLINE bool ukl_grid_scan_n(UklGrid g, uint32_t n, UKL_COHERENT const uint32_t* in, UKL_COHERENT uint32_t* out,
                                UKL_COHERENT uint32_t* total, UKL_COHERENT uint32_t* sums, UKL_LOCAL uint32_t* scratch, UKL_LOCAL uint32_t* flag __UKL_PARAMS)
{
    UklScanJob job;
    job.n = n;
    job.in = in;
    job.out = out;
    job.total = total;
    return ukl_grid_scans(g, &job, 1u, sums, scratch, flag __UKL_ARGS);
}

// `ukl_grid_scan_n` of `*count` values (the count read on the device), the total at `out[n]`.
UKL_INLINE bool ukl_grid_scan_exclusive_u32(UklGrid g, UKL_GLOBAL const uint32_t* count, UKL_COHERENT const uint32_t* in,
                                            UKL_COHERENT uint32_t* out, UKL_COHERENT uint32_t* sums,
                                            UKL_LOCAL uint32_t* scratch, UKL_LOCAL uint32_t* flag __UKL_PARAMS)
{
    uint32_t n = ukl_atomic_load((UKL_GLOBAL uint32_t*)count);
    return ukl_grid_scan_n(g, n, in, out, out + n, sums, scratch, flag __UKL_ARGS);
}

// --- The segmented scan ------------------------------------------------------------------------------------------

// The segmented scans' operators.
#define UKL_SEG_ADD 0u
#define UKL_SEG_MAX 1u

// A segment's identity is 0 for both.
UKL_INLINE uint32_t ukl_seg_op(uint32_t op, uint32_t a, uint32_t b) { return op == UKL_SEG_ADD ? a + b : (a > b ? a : b); }

// `(fa, va)` then `(fb, vb)`: `vb` alone where `b` starts a segment.
UKL_INLINE uint32_t ukl_seg_join(uint32_t op, uint32_t va, uint32_t fb, uint32_t vb) { return fb != 0u ? vb : ukl_seg_op(op, va, vb); }

// The inclusive segmented scan of `(f, v)` over the group's threads, in order (Hillis–Steele over `sv` and `sf`,
// `lsz` words each of threadgroup memory); in `*fin` whether a head is at or before this thread, and in every
// thread the group's whole aggregate in `*tv`, `*tf`.
UKL_INLINE uint32_t ukl_tg_seg_inclusive(uint32_t op, uint32_t f, uint32_t v, uint32_t lid, uint32_t lsz, UKL_LOCAL uint32_t* sv,
                                         UKL_LOCAL uint32_t* sf, UKL_PRIVATE uint32_t* fin, UKL_PRIVATE uint32_t* tv, UKL_PRIVATE uint32_t* tf)
{
    sv[lid] = v;
    sf[lid] = f;
    ukl_tg_barrier();
    for (uint32_t d = 1u; d < lsz; d <<= 1u) {
        uint32_t pv = 0u, pf = 0u;
        bool has = lid >= d;
        if (has) {
            pv = sv[lid - d];
            pf = sf[lid - d];
        }
        ukl_tg_barrier();
        if (has) {
            v = ukl_seg_join(op, pv, f, v);
            f = f | pf;
            sv[lid] = v;
            sf[lid] = f;
        }
        ukl_tg_barrier();
    }
    *fin = f;
    *tv = sv[lsz - 1u];
    *tf = sf[lsz - 1u];
    ukl_tg_barrier();
    return v;
}

// The inclusive segmented scan (`op`: `UKL_SEG_ADD` or `UKL_SEG_MAX`) of `in`'s first `n` values into `out`, a
// segment starting at each nonzero `heads` word, as phases of the grid; `in` may be `out`. `sums` is `2 * groups`
// words of device memory (each group's aggregate: value, then whether it holds a head); `sv`, `sf` are `lsz` words
// each of threadgroup memory. Three phases, as the scan's. Ends with a barrier.
UKL_INLINE bool ukl_grid_seg_scan_n(UklGrid g, uint32_t op, uint32_t n, UKL_COHERENT const uint32_t* in, UKL_COHERENT const uint32_t* heads,
                                    UKL_COHERENT uint32_t* out, UKL_COHERENT uint32_t* sums, UKL_LOCAL uint32_t* sv,
                                    UKL_LOCAL uint32_t* sf, UKL_LOCAL uint32_t* flag)
{
    if (n <= UKL_GRID_SMALL) {
        // Group 0's alone: its tiles in order, each from the carry of those before it.
        if (g.group == 0u) {
            uint32_t carry = 0u;
            for (uint32_t base = 0u; base < n; base += g.lsz) {
                uint32_t i = base + g.lid;
                bool live = i < n;
                uint32_t f = live && heads[i] != 0u ? 1u : 0u, fin, tv, tf;
                uint32_t x = ukl_tg_seg_inclusive(op, f, live ? in[i] : 0u, g.lid, g.lsz, sv, sf, &fin, &tv, &tf);
                if (live)
                    out[i] = fin != 0u ? x : ukl_seg_op(op, carry, x);
                carry = ukl_seg_join(op, carry, tf, tv);
            }
        }
        return ukl_grid_barrier(g, flag);
    }
    uint32_t first = ukl_grid_first(g, n, g.group), end = ukl_grid_first(g, n, g.group + 1u);
    // 1. The share's aggregate.
    uint32_t av = 0u, af = 0u;
    for (uint32_t base = first; base < end; base += g.lsz) {
        uint32_t i = base + g.lid;
        bool live = i < end;
        uint32_t fin, tv, tf;
        ukl_tg_seg_inclusive(op, live && heads[i] != 0u ? 1u : 0u, live ? in[i] : 0u, g.lid, g.lsz, sv, sf, &fin, &tv, &tf);
        av = ukl_seg_join(op, av, tf, tv);
        af |= tf;
    }
    if (g.lid == 0u) {
        sums[2u * g.group] = av;
        sums[2u * g.group + 1u] = af;
    }
    if (!ukl_grid_barrier(g, flag))
        return false;
    // 2. Group 0 makes each group's carry: the aggregate of the groups before it (in place; a few words a group).
    if (g.group == 0u && g.lid == 0u) {
        uint32_t cv = 0u, cf = 0u;
        for (uint32_t k = 0u; k < g.groups; k++) {
            uint32_t v = sums[2u * k], f = sums[2u * k + 1u];
            sums[2u * k] = cv;
            sums[2u * k + 1u] = cf;
            cv = ukl_seg_join(op, cv, f, v);
            cf |= f;
        }
    }
    if (!ukl_grid_barrier(g, flag))
        return false;
    // 3. The share, scanned from its carry.
    uint32_t carry = sums[2u * g.group];
    for (uint32_t base = first; base < end; base += g.lsz) {
        uint32_t i = base + g.lid;
        bool live = i < end;
        uint32_t f = live && heads[i] != 0u ? 1u : 0u, fin, tv, tf;
        // A thread's prefix within the tile; the carry joins it where no head is at or before it in the tile.
        uint32_t x = ukl_tg_seg_inclusive(op, f, live ? in[i] : 0u, g.lid, g.lsz, sv, sf, &fin, &tv, &tf);
        if (live)
            out[i] = fin != 0u ? x : ukl_seg_op(op, carry, x);
        carry = ukl_seg_join(op, carry, tf, tv);
    }
    return ukl_grid_barrier(g, flag);
}

// --- The radix sort ------------------------------------------------------------------------------------------------

#include <ukl/radix.h>
#if UKL_SIMD_WIDTH >= 8
// The tile bodies over coherent rows: a grid's passes read what other groups scattered.
#define UKL_RADIX_NAME(f) f##_u64_grid
#define UKL_RADIX_KEY uint64_t
#define UKL_RADIX_SPACE UKL_COHERENT
#include <ukl/body/radix_tile.h>

// Where a grid sort's rows end: in its first buffers when its digits (`bits` in bytes) are even.
UKL_INLINE bool ukl_grid_sort_in_input(uint32_t bits) { return ((bits + 7u) / 8u) % 2u == 0u; }

// The threadgroup memory a grid sort takes: `h`, `counts`, `before`, `base`, `digits` (`ukl_radix_*_tile`'s;
// `digits` is `UKL_RADIX_SPILL` words), and `tile`, `UKL_GRID_SORT_TILE_WORDS` words (a tile's keys and values, for
// the sort of one tile).
#define UKL_GRID_SORT_TILE_WORDS (3u * UKL_RADIX_TILE)
// The words of `counts`, `h`, `before`, `base` and `digits` laid end to end (where one block holds them, and `tile`
// aliases it: no phase uses both).
#define UKL_GRID_SORT_RADIX_WORDS (UKL_RADIX_GROUPS * UKL_RADIX_DIGITS + 3u * UKL_RADIX_DIGITS + UKL_RADIX_SPILL)
// The larger of the two.
#define UKL_GRID_SORT_WORDS (UKL_GRID_SORT_TILE_WORDS > UKL_GRID_SORT_RADIX_WORDS ? UKL_GRID_SORT_TILE_WORDS : UKL_GRID_SORT_RADIX_WORDS)
typedef struct UklGridSortLocal {
    UKL_LOCAL uint32_t* h;
    UKL_LOCAL uint32_t* counts;
    UKL_LOCAL uint32_t* before;
    UKL_LOCAL uint32_t* base;
    UKL_LOCAL uint32_t* digits;
    UKL_LOCAL uint32_t* tile;
} UklGridSortLocal;

// `a` before `b` by their low `bits` bits, ties by place.
UKL_INLINE bool ukl_sort_before(uint32_t ah, uint32_t al, uint32_t ai, uint32_t bh, uint32_t bl, uint32_t bi)
{
    return ah < bh || (ah == bh && (al < bl || (al == bl && ai < bi)));
}

// A stable LSD sort of `keys`' first `n` by their low `bits` bits (a multiple of 8), with `values` (`iota`: each
// row's index before the sort, written here), as phases of the grid. Groups of `UKL_RADIX_THREADS` threads. `keys2`,
// `values2` are the other buffers of each pass; `hist` is `256 * tiles + 1` words (`tiles`: `n` in
// `UKL_RADIX_TILE`s) and `sums` `4 * groups + 4`. The rows end in `keys`, `values` where `ukl_grid_sort_in_input`
// says, else in `keys2`, `values2`. `low` (a multiple of 8, at most `bits`): the rows are already in the order of
// their low `low` bits, so only the digits above are passed (0: none is known).
// - few rows (`n <= UKL_RADIX_TILE`, and at most a row a thread of the grid): each group holds every row in
//   threadgroup memory; each SIMD group ranks a row at a time among them all, its lanes taking a share each;
//   the rows go to their places after a barrier where they are sorted in place;
// - more: the digits that vary among the rows (a reduction), and only those passed, three barriers each.
UKL_INLINE bool ukl_grid_sort_u64_from(UklGrid g, uint32_t n, uint32_t low, uint32_t bits, bool iota, UKL_COHERENT uint64_t* keys,
                                       UKL_COHERENT uint32_t* values, UKL_COHERENT uint64_t* keys2, UKL_COHERENT uint32_t* values2,
                                       UKL_COHERENT uint32_t* hist, UKL_COHERENT uint32_t* sums, UklGridSortLocal l,
                                       UKL_LOCAL uint32_t* scratch, UKL_LOCAL uint32_t* flag __UKL_PARAMS)
{
    uint64_t mask = bits >= 64u ? ~(uint64_t)0 : (((uint64_t)1 << bits) - 1u);
    bool in_input = ukl_grid_sort_in_input(bits);
    UKL_COHERENT uint64_t* kd = in_input ? keys : keys2;
    UKL_COHERENT uint32_t* vd = in_input ? values : values2;
    if (n <= UKL_RADIX_TILE && n <= g.groups * g.lsz) {
        UKL_LOCAL uint32_t* hi = l.tile;
        UKL_LOCAL uint32_t* lo = l.tile + UKL_RADIX_TILE;
        for (uint32_t r = g.lid; r < n; r += g.lsz) {
            uint64_t key = keys[r] & mask;
            hi[r] = (uint32_t)(key >> 32);
            lo[r] = (uint32_t)key;
        }
        ukl_tg_barrier();
        // SIMD group `sg` of the grid ranks rows `sg`, `sg + sgs`, …: at most a row a lane, which lane `k` keeps.
        uint32_t lane = g.lid % UKL_SIMD_WIDTH, per = g.lsz / UKL_SIMD_WIDTH;
        uint32_t sg = g.group * per + g.lid / UKL_SIMD_WIDTH, sgs = g.groups * per;
        uint32_t row = n, rank = 0u;
        for (uint32_t k = 0u; k < UKL_SIMD_WIDTH; k++) {
            uint32_t r = sg + k * sgs;
            if (r >= n)
                break;
            uint32_t rh = hi[r], rl = lo[r], at = 0u;
            for (uint32_t o = lane; o < n; o += UKL_SIMD_WIDTH)
                at += ukl_sort_before(hi[o], lo[o], o, rh, rl, r) ? 1u : 0u;
            at = ukl_sg_sum(at __UKL_ARGS);
            if (lane == k) {
                row = r;
                rank = at;
            }
        }
        uint64_t key = row < n ? keys[row] : 0u;
        uint32_t value = row < n ? (iota ? row : values[row]) : 0u;
        // Sorted in place: every group has read its rows before any is written over.
        if (in_input && !ukl_grid_barrier(g, flag))
            return false;
        if (row < n) {
            kd[rank] = key;
            vd[rank] = value;
        }
        return ukl_grid_barrier(g, flag);
    }
    uint32_t tiles = (n + UKL_RADIX_TILE - 1u) / UKL_RADIX_TILE;
    uint32_t first = ukl_grid_first(g, tiles, g.group), end = ukl_grid_first(g, tiles, g.group + 1u);
    // The bits that vary: each group's OR and AND of its rows' keys (and the rows' indexes where `iota`).
    {
        uint32_t rf = ukl_grid_first(g, n, g.group), re = ukl_grid_first(g, n, g.group + 1u);
        uint32_t oh = 0u, ol = 0u, ah = ~0u, al = ~0u;
        for (uint32_t r = rf + g.lid; r < re; r += g.lsz) {
            uint64_t key = keys[r] & mask;
            oh |= (uint32_t)(key >> 32);
            ol |= (uint32_t)key;
            ah &= (uint32_t)(key >> 32);
            al &= (uint32_t)key;
            if (iota)
                values[r] = r;
        }
        oh = ukl_tg_or(oh, g.lid, g.lsz, scratch __UKL_ARGS);
        ol = ukl_tg_or(ol, g.lid, g.lsz, scratch __UKL_ARGS);
        ah = ukl_tg_and(ah, g.lid, g.lsz, scratch __UKL_ARGS);
        al = ukl_tg_and(al, g.lid, g.lsz, scratch __UKL_ARGS);
        if (g.lid == 0u) {
            sums[4u * g.group] = oh;
            sums[4u * g.group + 1u] = ol;
            sums[4u * g.group + 2u] = re > rf ? ah : ~0u;
            sums[4u * g.group + 3u] = re > rf ? al : ~0u;
        }
    }
    if (!ukl_grid_barrier(g, flag))
        return false;
    uint64_t any = 0u, all = ~(uint64_t)0;
    for (uint32_t k = 0u; k < g.groups; k++) {
        any |= ((uint64_t)sums[4u * k] << 32) | (uint64_t)sums[4u * k + 1u];
        all &= ((uint64_t)sums[4u * k + 2u] << 32) | (uint64_t)sums[4u * k + 3u];
    }
    uint64_t vary = any ^ all;
    // Every group has read the sums before the passes' scans reuse them.
    if (!ukl_grid_barrier(g, flag))
        return false;
    uint32_t passes = 0u;
    for (uint32_t p = low / 8u; p < (bits + 7u) / 8u; p++) {
        if (((vary >> (8u * p)) & 0xffu) == 0u)
            continue;
        UklRadixArgs args;
        args.n = n;
        args.shift = 8u * p;
        args.tiles = tiles;
        UKL_COHERENT uint64_t* ki = passes % 2u == 0u ? keys : keys2;
        UKL_COHERENT uint64_t* ko = passes % 2u == 0u ? keys2 : keys;
        UKL_COHERENT uint32_t* vi = passes % 2u == 0u ? values : values2;
        UKL_COHERENT uint32_t* vo = passes % 2u == 0u ? values2 : values;
        for (uint32_t tile = first; tile < end; tile++)
            ukl_radix_hist_tile_u64_grid(ki, hist, args, l.h, tile, g.lid);
        if (!ukl_grid_barrier(g, flag))
            return false;
        if (!ukl_grid_scan_n(g, UKL_RADIX_DIGITS * tiles, hist, hist, hist + UKL_RADIX_DIGITS * tiles, sums, scratch, flag __UKL_ARGS))
            return false;
        for (uint32_t tile = first; tile < end; tile++)
            ukl_radix_scatter_tile_u64_grid(ki, vi, hist, ko, vo, args, l.counts, l.before, l.base, l.digits, tile, g.lid __UKL_ARGS);
        if (!ukl_grid_barrier(g, flag))
            return false;
        passes++;
    }
    if ((passes % 2u == 0u) == in_input)
        return true;
    // The rows where the caller reads them.
    UKL_COHERENT uint64_t* ks = passes % 2u == 0u ? keys : keys2;
    UKL_COHERENT uint32_t* vs = passes % 2u == 0u ? values : values2;
    uint32_t rf = ukl_grid_first(g, n, g.group), re = ukl_grid_first(g, n, g.group + 1u);
    for (uint32_t r = rf + g.lid; r < re; r += g.lsz) {
        kd[r] = ks[r];
        vd[r] = vs[r];
    }
    return ukl_grid_barrier(g, flag);
}

// `ukl_grid_sort_u64_from` with no order known.
UKL_INLINE bool ukl_grid_sort_u64(UklGrid g, uint32_t n, uint32_t bits, bool iota, UKL_COHERENT uint64_t* keys, UKL_COHERENT uint32_t* values,
                                  UKL_COHERENT uint64_t* keys2, UKL_COHERENT uint32_t* values2, UKL_COHERENT uint32_t* hist,
                                  UKL_COHERENT uint32_t* sums, UklGridSortLocal l, UKL_LOCAL uint32_t* scratch, UKL_LOCAL uint32_t* flag __UKL_PARAMS)
{
    return ukl_grid_sort_u64_from(g, n, 0u, bits, iota, keys, values, keys2, values2, hist, sums, l, scratch, flag __UKL_ARGS);
}
#endif

