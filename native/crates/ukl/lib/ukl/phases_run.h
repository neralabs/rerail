// ukl/phases_run.h: a program of phases on the device (decision 0726, tier 2 as phases): each phase a launch, its items
// (chunks) taken by the launch's SIMD groups by ticket, a lane an item; an item runs the phase's block operator over
// its chunk, reading only its chunk, the arrays earlier phases wrote and its sum, and may write its leaf of the
// phase's tree and an array. A tree is the exclusive prefix of its leaves by the phase's associative merge, in any
// order of arrival: a leaf stores its record and arrives at its parent, the second child to arrive merging the two
// (`<ukl/lex_run.h>`'s arrival tree, here over records of any width and any merge); a
// leaf's sum is walked down from the top in a later phase, the left siblings' values merged in file order. Nothing is
// carried from one item to the next.
//
// The grammar's, declared here: the merges and their units by op (`ukl_ph_merge`, `ukl_ph_unit`, the printer's from
// the Rust the CPU's phases run), and the program's items (`ukl_ph_item`, `ukl_ph_items`).
//
// The pool (u32s, coherent): its header (`UKL_PH_H` words: the epoch, the arrivals' stride, each phase's tickets, each
// tree's layout), then whatever the program lays out.

#pragma once
#include <ukl/simd.h>

typedef UKL_COHERENT uint32_t* UklPhPool;

#define UKL_PH_H 200u
// The run's epoch (the host's, new each run, never 0): a node's two arrivals each exchange it into its word.
#define UKL_PH_EPOCH 0u
// The arrivals' stride: tree `t`'s node `x` at `meet[t * MS + x]`.
#define UKL_PH_MS 1u
// The first failure (0 none).
#define UKL_PH_FAIL 2u
// Each phase's tickets (at most `UKL_PH_PHASES` phases).
#define UKL_PH_TICKET 8u
#define UKL_PH_PHASES 64u
// Each tree's layout, `UKL_PH_TW` words: its values' offset (`2 * leaves` nodes, `width` words each), its sums'
// (two records a leaf), its width, its leaves (the host's, or a phase's before the tree's), its op.
#define UKL_PH_TREE 72u
#define UKL_PH_TW 5u
#define UKL_PH_TREES 24u
#define UKL_PH_TURNS 100000000u
// A tree of `n` leaves' nodes at most: its leaves and each level's half the one below, rounded up (under `2n` plus a
// node a level), the arrivals' stride too.
#define UKL_PH_NODES(n) (2u * (n) + 40u)

// The program's own fields (`UKL_PH_FIELDS`, defined before this is included: its inputs).
#ifndef UKL_PH_FIELDS
#define UKL_PH_FIELDS
#endif
typedef struct UklPh {
    UklPhPool m;
    UKL_GLOBAL uint32_t* meet;
    uint32_t phase;
    UKL_PH_FIELDS
} UklPh;

// Merge `a · b` of op `op` into `out` (never `a` or `b`); op `op`'s unit into `out`.
UKL_INLINE void ukl_ph_merge(uint32_t op, UKL_COHERENT const uint32_t* a, UKL_COHERENT const uint32_t* b, UKL_COHERENT uint32_t* out);
UKL_INLINE void ukl_ph_unit(uint32_t op, UKL_COHERENT uint32_t* out);

// The runner's own op: a sum of the record's words, each added (`UKL_PH_OP_ADD`; the grammar's ops are below it).
#define UKL_PH_OP_ADD 1000u
UKL_INLINE void ukl_ph_merge_w(uint32_t op, uint32_t w, UKL_COHERENT const uint32_t* a, UKL_COHERENT const uint32_t* b, UKL_COHERENT uint32_t* out)
{
    if (op == UKL_PH_OP_ADD) {
        for (uint32_t k = 0u; k < w; k++)
            out[k] = a[k] + b[k];
    } else {
        ukl_ph_merge(op, a, b, out);
    }
}
UKL_INLINE void ukl_ph_unit_w(uint32_t op, uint32_t w, UKL_COHERENT uint32_t* out)
{
    if (op == UKL_PH_OP_ADD) {
        for (uint32_t k = 0u; k < w; k++)
            out[k] = 0u;
    } else {
        ukl_ph_unit(op, out);
    }
}

UKL_INLINE uint32_t ukl_ph_t(UKL_PRIVATE UklPh* r, uint32_t t, uint32_t k) { return r->m[UKL_PH_TREE + UKL_PH_TW * t + k]; }
UKL_INLINE uint32_t ukl_ph_width(UKL_PRIVATE UklPh* r, uint32_t t) { return ukl_ph_t(r, t, 2u); }
UKL_INLINE uint32_t ukl_ph_leaves(UKL_PRIVATE UklPh* r, uint32_t t) { return ukl_ph_t(r, t, 3u); }
// Tree `t`'s node `x` (leaves first, then each level's nodes): its record.
UKL_INLINE UklPhPool ukl_ph_node(UKL_PRIVATE UklPh* r, uint32_t t, uint32_t x) { return r->m + ukl_ph_t(r, t, 0u) + x * ukl_ph_width(r, t); }
UKL_INLINE UklPhPool ukl_ph_leaf(UKL_PRIVATE UklPh* r, uint32_t t, uint32_t c) { return ukl_ph_node(r, t, c); }

UKL_INLINE void ukl_ph_fail(UKL_PRIVATE UklPh* r, uint32_t code) { ukl_atomic_cas(&r->m[UKL_PH_FAIL], 0u, code); }

// Node `x` of tree `t` arrived at: whether its other child arrived first (the epoch already in its word).
UKL_INLINE bool ukl_ph_meet(UKL_PRIVATE UklPh* r, uint32_t t, uint32_t x)
{
    uint32_t epoch = r->m[UKL_PH_EPOCH];
    ukl_fence_device();
    bool last = ukl_atomic_exchange(&r->meet[t * r->m[UKL_PH_MS] + x], epoch) == epoch;
    ukl_fence_device();
    return last;
}

UKL_INLINE void ukl_ph_copy(UKL_COHERENT const uint32_t* a, UKL_COHERENT uint32_t* out, uint32_t w)
{
    for (uint32_t k = 0u; k < w; k++)
        out[k] = a[k];
}

// The levels of a tree of `n` leaves above its leaves.
UKL_INLINE uint32_t ukl_ph_levels(uint32_t n)
{
    uint32_t levels = 0u;
    for (uint32_t lv = n; levels < 40u && lv > 1u; levels++)
        lv = (lv + 1u) >> 1;
    return levels;
}

// Leaf `c` of tree `t`, its record stored: up while this item is the second child to arrive; the root (the file's
// record) is the last node.
UKL_INLINE void ukl_ph_up(UKL_PRIVATE UklPh* r, uint32_t t, uint32_t c)
{
    uint32_t lv = ukl_ph_leaves(r, t), base = 0u, w = ukl_ph_width(r, t), op = ukl_ph_t(r, t, 4u);
    ukl_fence_device();
    for (uint32_t level = 0u; level < 40u && lv > 1u; level++) {
        uint32_t up = base + lv, p = c >> 1;
        bool two = 2u * p + 1u < lv;
        if (two && !ukl_ph_meet(r, t, up + p))
            return;
        if (two)
            ukl_ph_merge_w(op, w, ukl_ph_node(r, t, base + 2u * p), ukl_ph_node(r, t, base + 2u * p + 1u), ukl_ph_node(r, t, up + p));
        else
            ukl_ph_copy(ukl_ph_node(r, t, base + 2u * p), ukl_ph_node(r, t, up + p), w);
        ukl_fence_device();
        base = up;
        lv = (lv + 1u) >> 1;
        c = p;
    }
}

// Tree `t`'s root: the merge of every leaf (its last node).
UKL_INLINE UklPhPool ukl_ph_total(UKL_PRIVATE UklPh* r, uint32_t t)
{
    uint32_t lv = ukl_ph_leaves(r, t), base = 0u;
    while (lv > 1u) {
        base += lv;
        lv = (lv + 1u) >> 1;
    }
    return ukl_ph_node(r, t, base);
}

// Leaf `c`'s sum of tree `t` (the leaves before it merged, in file order), walked down from the top into its two sum
// records: the one it ends in.
UKL_INLINE UklPhPool ukl_ph_sum(UKL_PRIVATE UklPh* r, uint32_t t, uint32_t c)
{
    uint32_t n = ukl_ph_leaves(r, t), w = ukl_ph_width(r, t), op = ukl_ph_t(r, t, 4u), levels = ukl_ph_levels(n);
    UklPhPool s[2];
    s[0] = r->m + ukl_ph_t(r, t, 1u) + 2u * w * c;
    s[1] = s[0] + w;
    uint32_t cur = 0u;
    ukl_ph_unit_w(op, w, s[0]);
    for (uint32_t l = levels; l > 0u; l--) {
        uint32_t below = 0u, size = n;
        for (uint32_t k = 0u; k + 1u < l; k++) {
            below += size;
            size = (size + 1u) >> 1;
        }
        uint32_t x = c >> (l - 1u);
        if (x % 2u == 0u)
            continue;
        ukl_ph_merge_w(op, w, s[cur], ukl_ph_node(r, t, below + x - 1u), s[1u - cur]);
        cur = 1u - cur;
    }
    return s[cur];
}

// The program's: phase `ph`'s item `c`, and its items.
UKL_INLINE void ukl_ph_item(UKL_PRIVATE UklPh* r, uint32_t ph, uint32_t c);
UKL_INLINE uint32_t ukl_ph_items(UKL_PRIVATE UklPh* r, uint32_t ph);
#ifdef UKL_PH_GROUP
// The program's group items (`UKL_PH_GROUP`): phase `ph`'s item `c`, every lane of the group on it together.
UKL_INLINE void ukl_ph_item_sg(UKL_PRIVATE UklPh* r, uint32_t ph, uint32_t c, uint32_t lane __UKL_PARAMS);
#endif

// A worker of phase `r->phase`: a SIMD group's lanes take a group's worth of items at a time by ticket, each lane its
// own, until none is left.
UKL_INLINE void ukl_ph_worker(UKL_PRIVATE UklPh* r, uint32_t lane __UKL_PARAMS)
{
    uint32_t ph = r->phase, n = ukl_ph_items(r, ph);
    for (uint32_t turn = 0u; turn < UKL_PH_TURNS; turn++) {
        uint32_t t0 = 0u;
        if (lane == 0u)
            t0 = ukl_atomic_add(&r->m[UKL_PH_TICKET + ph], UKL_SIMD_WIDTH);
        t0 = ukl_sg_broadcast(t0, 0u __UKL_ARGS);
        if (t0 >= n)
            break;
        uint32_t c = t0 + lane;
        if (c < n) {
            ukl_fence_device();
            ukl_ph_item(r, ph, c);
        }
    }
}

#ifdef UKL_PH_GROUP
// A worker of a phase of group items: a SIMD group takes an item at a time by ticket, its lanes on it together, until
// none is left.
UKL_INLINE void ukl_ph_worker_sg(UKL_PRIVATE UklPh* r, uint32_t lane __UKL_PARAMS)
{
    uint32_t ph = r->phase, n = ukl_ph_items(r, ph);
    for (uint32_t turn = 0u; turn < UKL_PH_TURNS; turn++) {
        uint32_t c = 0u;
        if (lane == 0u)
            c = ukl_atomic_add(&r->m[UKL_PH_TICKET + ph], 1u);
        c = ukl_sg_broadcast(c, 0u __UKL_ARGS);
        if (c >= n)
            break;
        ukl_fence_device();
        ukl_ph_item_sg(r, ph, c, lane __UKL_ARGS);
    }
}
#endif
