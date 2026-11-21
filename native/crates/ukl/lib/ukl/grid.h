// ukl/grid.h: a persistent grid's device-wide barrier, and the primitives written as its phases (decision 0468 §4).
//
// Not for new code (decision 0540 step 7): no thread waits at a barrier for others. On the M3 Max, groups that had
// joined a grid were kept from reaching its barrier (0540 step 6), and how many groups run at once is no kernel's to
// rely on. A step that needs other groups' results is a continuation that the group finishing them pushes onto a
// work queue (`<ukl/workqueue.h>`; `label_phase` in label_passes.uglykl). The engine's kernels include this header no
// more.
//
// A persistent grid is G threadgroups resident together, each walking its own share of the work. Between two of its
// phases, every group waits for every other at `ukl_grid_barrier`: the writes of every group before it are seen by
// every group after it. So a primitive that took a dispatch a level (a scan's tile sums, their scan, the tiles from
// their offsets) is a few phases of one dispatch, and its count is read on the device.
//
// No backend promises that a dispatch's groups are resident together, and a group that waits for one that is not
// keeps it from running. So the grid is the groups that are running, found on the device (decision 0540 §3,
// `ukl_grid_join`): each group takes a ticket from a counter as it starts; the first, after a short window, closes
// the count; the groups that took a ticket before it closed are the grid, numbered in ticket order, and every
// other group ends at once. A barrier so waits only for groups that are running. Its wait is bounded still (0027:
// no wait without a bound): a group that has polled `UKL_GRID_SPINS` times sets the grid's failure word and goes on;
// every barrier after it returns false at once, in every group, and the caller ends its work.
//
// `UklGrid.ctl`: four words, zero before the dispatch: 0 the groups arrived at the current barrier; 1 its
// generation, twice over, with the failure bit (bit 0); 2 the failure word, for a phase's quick test; 3 the tickets
// taken, with `UKL_GRID_CLOSED` once the count is closed. A barrier's
// outcome is one for every group: the last to arrive steps the generation only by a compare-and-swap that finds no
// failure bit, and a group past its bound sets the bit only by a compare-and-swap that finds the generation it waited
// on. So either every group passes a barrier or none does. One form on every backend (CUDA's too, until the device
// layer makes cooperative launches).
//
// The grid's control words are read by polls (`ukl_atomic_poll`): a relaxed load in a loop may never see another
// group's store on Apple's GPUs (Q123; the reason the barrier failed at 8 groups, Q367).
//
// Reads of what another group wrote before a barrier go through `UKL_COHERENT` memory (`ukl_ldc_*`, or a coherent
// pointer): a plain device load may be served from a cache the other group's write did not reach.

#pragma once
#include <ukl/group.h>

// A barrier's polls at most, a group's first thread polling.
#ifndef UKL_GRID_SPINS
#define UKL_GRID_SPINS 1000000u
#endif
// The first group's polls for others to join, after the last that did (a window, not a wait: a group that has not
// started by its end is not the grid's). A dispatch whose groups all start ends it when the last joins. The CPU
// backend runs a dispatch's groups one at a time, so its grid is one group. A longer window costs a dispatch past
// residency its time: at 65,536 polls, 843 ms on a GB10 (48 groups polling one word), against 30 ms.
#ifndef UKL_GRID_WINDOW
#define UKL_GRID_WINDOW 2048u
#endif
#define UKL_GRID_CLOSED 0x80000000u

typedef struct UklGrid {
    UKL_GLOBAL uint32_t* ctl;
    // The grid's groups, this group, its thread and its size.
    uint32_t groups, group, lid, lsz;
} UklGrid;

// Whether the grid has failed a barrier (in every thread alike after a barrier).
UKL_INLINE bool ukl_grid_failed(UklGrid g) { return ukl_atomic_poll(&g.ctl[2]) != 0u; }

// The grid of the groups that are running (decision 0540 §3): a group of a dispatch of `dispatched` groups takes a
// ticket, and is the grid's (true: `g.group` its ticket, `g.groups` the grid's count) if it took it before the count
// closed; else (false) it is not, and ends without touching the grid's work. Every thread of every group calls it
// once, before any barrier; `words` is two words of threadgroup memory.
//
// The first group (ticket 0) closes the count once no group has joined for `UKL_GRID_WINDOW` of its polls, or all
// `dispatched` have. Every other one waits for that: for a group that is running (it took the first ticket) and waits
// for nothing but its window, so the wait ends. Bounded still: past `UKL_GRID_SPINS` polls, the grid fails.
UKL_INLINE bool ukl_grid_join(UKL_PRIVATE UklGrid* g, uint32_t dispatched, UKL_LOCAL uint32_t* words)
{
    if (g->lid == 0u) {
        // A ticket, by a compare-and-swap that finds the count open.
        uint32_t ticket = UKL_GRID_CLOSED, w = ukl_atomic_poll(&g->ctl[3]);
        for (uint32_t tries = 0u; tries < dispatched && (w & UKL_GRID_CLOSED) == 0u; tries++) {
            uint32_t seen = ukl_atomic_cas(&g->ctl[3], w, w + 1u);
            if (seen == w) {
                ticket = w;
                tries = dispatched;
            } else {
                w = seen;
            }
        }
        uint32_t count = 0u;
        if (ticket == 0u) {
            // The window: polled until all have joined, or none has for `UKL_GRID_WINDOW` polls; then closed, by a
            // compare-and-swap that finds the count it closes.
            uint32_t last = 1u, quiet = 0u;
            while (quiet < UKL_GRID_WINDOW && last < dispatched) {
                uint32_t now = ukl_atomic_poll(&g->ctl[3]);
                quiet = now == last ? quiet + 1u : 0u;
                last = now;
            }
            uint32_t at = ukl_atomic_poll(&g->ctl[3]);
            for (uint32_t tries = 0u; tries <= dispatched; tries++) {
                uint32_t seen = ukl_atomic_cas(&g->ctl[3], at, at | UKL_GRID_CLOSED);
                if (seen == at) {
                    count = at;
                    tries = dispatched;
                } else {
                    at = seen;
                }
            }
        } else if (ticket != UKL_GRID_CLOSED) {
            uint32_t now = ukl_atomic_poll(&g->ctl[3]);
            for (uint32_t spins = 0u; (now & UKL_GRID_CLOSED) == 0u && spins < UKL_GRID_SPINS; spins++)
                now = ukl_atomic_poll(&g->ctl[3]);
            count = now & ~UKL_GRID_CLOSED;
        }
        // Not closed (the first's closing lost every race, or the wait ran out): the grid fails, its barriers false.
        if (ticket != UKL_GRID_CLOSED && count == 0u) {
            ukl_atomic_store(&g->ctl[2], 1u);
            ukl_atomic_or(&g->ctl[1], 1u);
            count = ticket + 1u;
        }
        ukl_fence_device();
        words[0] = ticket;
        words[1] = count;
    }
    ukl_tg_barrier();
    g->group = words[0];
    g->groups = words[1];
    return words[0] != UKL_GRID_CLOSED;
}

// Waits for every group of the grid: true when all arrived, false when the bound was reached (here or in another
// group) or the grid failed before; the same answer in every group. Every thread of every group calls it, the same
// number of times. `flag` is a word of threadgroup memory.
UKL_INLINE bool ukl_grid_barrier(UklGrid g, UKL_LOCAL uint32_t* flag)
{
    // The group's writes, done.
    ukl_tg_barrier_device();
    if (g.lid == 0u) {
        // The generation before this group arrives: it cannot step until every group, this one too, has.
        uint32_t gen = ukl_atomic_poll(&g.ctl[1]);
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
                    now = ukl_atomic_poll(&g.ctl[1]);
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

// The exclusive scan of `in`'s first `n` values into `out`, its total into `*total` (`out + n` for an offsets
// array), as phases of the grid; `in` may be `out`. `sums` is `groups + 1` words of device memory; `scratch` is `UKL_TG_SCRATCH` words of threadgroup memory.
// Three phases: each group sums its share; group 0 scans the sums (`sums[groups]` the total); each group scans its
// share from its offset. False when a barrier failed (`out` is then partial). Ends with a barrier: `out` is seen by
// every group.
UKL_INLINE bool ukl_grid_scan_n(UklGrid g, uint32_t n, UKL_COHERENT const uint32_t* in, UKL_COHERENT uint32_t* out,
                                UKL_COHERENT uint32_t* total, UKL_COHERENT uint32_t* sums, UKL_LOCAL uint32_t* scratch, UKL_LOCAL uint32_t* flag __UKL_PARAMS)
{
    uint32_t first = ukl_grid_first(g, n, g.group), end = ukl_grid_first(g, n, g.group + 1u);
    // 1. The share's sum, a group's tile at a time.
    uint32_t acc = 0u;
    for (uint32_t base = first; base < end; base += g.lsz) {
        uint32_t i = base + g.lid;
        acc += ukl_tg_sum(i < end ? in[i] : 0u, g.lid, g.lsz, scratch __UKL_ARGS);
    }
    if (g.lid == 0u)
        sums[g.group] = acc;
    if (!ukl_grid_barrier(g, flag))
        return false;
    // 2. Group 0 scans the groups' sums, a tile at a time.
    if (g.group == 0u) {
        uint32_t run = 0u;
        for (uint32_t base = 0u; base < g.groups; base += g.lsz) {
            uint32_t i = base + g.lid;
            uint32_t v = i < g.groups ? sums[i] : 0u;
            UklScan s = ukl_tg_exclusive_sum(v, g.lid, g.lsz, scratch __UKL_ARGS);
            if (i < g.groups)
                sums[i] = run + s.exclusive;
            run += s.total;
        }
        if (g.lid == 0u)
            sums[g.groups] = run;
    }
    if (!ukl_grid_barrier(g, flag))
        return false;
    // 3. The share, scanned from its offset (each value read before it is written, by the same thread).
    uint32_t run = sums[g.group];
    for (uint32_t base = first; base < end; base += g.lsz) {
        uint32_t i = base + g.lid;
        uint32_t v = i < end ? in[i] : 0u;
        UklScan s = ukl_tg_exclusive_sum(v, g.lid, g.lsz, scratch __UKL_ARGS);
        if (i < end)
            out[i] = run + s.exclusive;
        run += s.total;
    }
    if (g.group == 0u && g.lid == 0u)
        *total = sums[g.groups];
    return ukl_grid_barrier(g, flag);
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

// The threadgroup memory a grid sort takes: `h`, `counts`, `before`, `base` (`ukl_radix_*_tile`'s).
typedef struct UklGridSortLocal {
    UKL_LOCAL uint32_t* h;
    UKL_LOCAL uint32_t* counts;
    UKL_LOCAL uint32_t* before;
    UKL_LOCAL uint32_t* base;
} UklGridSortLocal;

// A stable LSD sort of `keys`' first `n` by their low `bits` bits (a multiple of 8), with `values` (`iota`: each
// row's index before the sort, written here), as phases of the grid. Groups of `UKL_RADIX_THREADS` threads. `keys2`,
// `values2` are the other buffers of each pass; `hist` is `256 * tiles + 1` words (`tiles`: `n` in
// `UKL_RADIX_TILE`s) and `sums` `groups + 1`. The rows end in `keys`, `values` where the passes are even
// (`ukl_grid_sort_in_input`), else in `keys2`, `values2`. Ends with a barrier.
UKL_INLINE bool ukl_grid_sort_u64(UklGrid g, uint32_t n, uint32_t bits, bool iota, UKL_COHERENT uint64_t* keys, UKL_COHERENT uint32_t* values,
                                  UKL_COHERENT uint64_t* keys2, UKL_COHERENT uint32_t* values2, UKL_COHERENT uint32_t* hist,
                                  UKL_COHERENT uint32_t* sums, UklGridSortLocal l, UKL_LOCAL uint32_t* scratch, UKL_LOCAL uint32_t* flag __UKL_PARAMS)
{
    uint32_t tiles = (n + UKL_RADIX_TILE - 1u) / UKL_RADIX_TILE;
    uint32_t first = ukl_grid_first(g, tiles, g.group), end = ukl_grid_first(g, tiles, g.group + 1u);
    if (iota) {
        uint32_t rf = ukl_grid_first(g, n, g.group), re = ukl_grid_first(g, n, g.group + 1u);
        for (uint32_t r = rf + g.lid; r < re; r += g.lsz)
            values[r] = r;
        if (!ukl_grid_barrier(g, flag))
            return false;
    }
    for (uint32_t p = 0u; p < (bits + 7u) / 8u; p++) {
        UklRadixArgs args;
        args.n = n;
        args.shift = 8u * p;
        args.tiles = tiles;
        UKL_COHERENT uint64_t* ki = p % 2u == 0u ? keys : keys2;
        UKL_COHERENT uint64_t* ko = p % 2u == 0u ? keys2 : keys;
        UKL_COHERENT uint32_t* vi = p % 2u == 0u ? values : values2;
        UKL_COHERENT uint32_t* vo = p % 2u == 0u ? values2 : values;
        for (uint32_t tile = first; tile < end; tile++)
            ukl_radix_hist_tile_u64_grid(ki, hist, args, l.h, tile, g.lid);
        if (!ukl_grid_barrier(g, flag))
            return false;
        if (!ukl_grid_scan_n(g, UKL_RADIX_DIGITS * tiles, hist, hist, hist + UKL_RADIX_DIGITS * tiles, sums, scratch, flag __UKL_ARGS))
            return false;
        for (uint32_t tile = first; tile < end; tile++)
            ukl_radix_scatter_tile_u64_grid(ki, vi, hist, ko, vo, args, l.counts, l.before, l.base, tile, g.lid __UKL_ARGS);
        if (!ukl_grid_barrier(g, flag))
            return false;
    }
    return true;
}
#endif

// Where a grid sort's rows end: in its first buffers when its passes are even.
UKL_INLINE bool ukl_grid_sort_in_input(uint32_t bits) { return ((bits + 7u) / 8u) % 2u == 0u; }
