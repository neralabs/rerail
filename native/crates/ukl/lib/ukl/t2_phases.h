// ukl/t2_phases.h: tier 2 as phases on the device (decision 0726 step 4), over `<ukl/phases_run.h>`: from tier 1's
// records (`starts`, its pool's count) the phases the CPU's `Phases::run` runs, each item a chunk of `size` records,
// each block operator the grammar's, printed from the Rust the CPU runs (`ukl_t2_*`).
//
//   LAY  one item: the layout from the records (each tree's and array's offset in the pool), or the words the file
//        needs past the pool's room (`UKL_T2_NEED`: every later phase then has no items, the host runs it again).
//   P1   each chunk's depth summary (`p1_run`), a leaf of P1's tree (`p1_merge`).
//   P1W  each chunk's sum walked down, its records' depths and items written from it (`p1_write`); item 0 says the
//        items (P1's root), and the level sort's chunks (its trees' leaves).
//   P2   the level sort, two digits of 8 bits, each four phases: C, each chunk of items' keyed counts (`p2_run`), a
//        leaf of the digit's tree (`p2_merge`); K, each key's total (the tree's root) a leaf of the keys' tree
//        (`add1_merge`); KW, each key's start (its sum) written; S, each chunk's items scattered at their keys' starts
//        and its counts before them (`p2_write`, its sum counted on in place). The items from `items` to `items2`, and
//        back.
//   P3   over the items in level order, each chunk's segmented last-value of the heads (`p3_run`, `last_merge`); P3W
//        each record's container and each head's place written from its sum (`p3_write`).

#pragma once
#define UKL_PH_FIELDS UKL_GLOBAL const uint8_t* src; UKL_GLOBAL const uint32_t* starts; UKL_GLOBAL uint32_t* arrays; uint32_t n; bool refused;
#include <ukl/phases_run.h>

// The program's header, after the runner's: the host's (the records a chunk, the pool's words, the arrays' words), then
// the layout: the trees in the pool, the arrays (what later phases read: never read in the launch that writes them) in
// `arrays`.
#define UKL_T2_SIZE (UKL_PH_H + 0u)
#define UKL_T2_ROOM (UKL_PH_H + 1u)
#define UKL_T2_AROOM (UKL_PH_H + 8u)
#define UKL_T2_NEED (UKL_PH_H + 2u)
#define UKL_T2_N (UKL_PH_H + 3u)
#define UKL_T2_CHUNKS (UKL_PH_H + 4u)
#define UKL_T2_O_DEPTH (UKL_PH_H + 5u)
#define UKL_T2_O_ITEMS (UKL_PH_H + 6u)
#define UKL_T2_ITEMS (UKL_PH_H + 7u)
#define UKL_T2_ICHUNKS (UKL_PH_H + 9u)
#define UKL_T2_O_ITEMS2 (UKL_PH_H + 10u)
#define UKL_T2_O_KEYS (UKL_PH_H + 11u)
#define UKL_T2_O_CONTAINER (UKL_PH_H + 12u)
#define UKL_T2_O_HEAD_AT (UKL_PH_H + 13u)
#define UKL_T2_H (UKL_PH_H + 32u)

#define UKL_T2_PH_LAY 0u
#define UKL_T2_PH_P1 1u
#define UKL_T2_PH_P1W 2u
// The level sort's digit `d`'s four: `UKL_T2_PH_P2 + 4d` its C, then K, KW and S.
#define UKL_T2_PH_P2 3u
#define UKL_T2_PH_P3 11u
#define UKL_T2_PH_P3W 12u
#define UKL_T2_PHASES 13u

#define UKL_T2_TP1 0u
// Digit `d`'s trees: its chunks' counts `UKL_T2_TP2 + 2d`, its keys' after it.
#define UKL_T2_TP2 1u
#define UKL_T2_TP3 5u
#define UKL_T2_OP_P1 0u
#define UKL_T2_OP_ADD1 1u
#define UKL_T2_OP_KEYS 2u
#define UKL_T2_OP_LAST 3u
#define UKL_T2_W_P1 3u
#define UKL_T2_KEYS 256u

// The grammar's block operators (the printer's).
UKL_INLINE void ukl_t2_p1_run(UKL_GLOBAL const uint8_t* src, UKL_GLOBAL const uint32_t* starts, uint32_t n, uint32_t a, uint32_t b, UKL_COHERENT uint32_t* out);
UKL_INLINE void ukl_t2_p2_run(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, uint32_t shift, UKL_COHERENT uint32_t* out);
UKL_INLINE void ukl_t2_p2_write(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, uint32_t shift, UKL_COHERENT uint32_t* sum, UKL_GLOBAL const uint32_t* keys,
                                UKL_GLOBAL uint32_t* out);
UKL_INLINE void ukl_t2_p3_run(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, UKL_COHERENT uint32_t* out);
UKL_INLINE void ukl_t2_p3_write(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, UKL_COHERENT const uint32_t* sum, UKL_GLOBAL uint32_t* container,
                                UKL_GLOBAL uint32_t* head_at);
UKL_INLINE void ukl_t2_p1_write(UKL_GLOBAL const uint8_t* src, UKL_GLOBAL const uint32_t* starts, uint32_t n, uint32_t a, uint32_t b, UKL_COHERENT const uint32_t* sum,
                                UKL_GLOBAL uint32_t* depth, UKL_GLOBAL uint32_t* items, UKL_GLOBAL uint32_t* head_at);

// Tree `t`'s layout at `at`: `leaves` leaves of `w` words, op `op`; past it. Its nodes (`UKL_PH_NODES`: each level's
// half its level below, rounded up) and two sum records a leaf.
UKL_INLINE uint32_t ukl_t2_tree(UklPhPool m, uint32_t t, uint32_t at, uint32_t leaves, uint32_t w, uint32_t op)
{
    UklPhPool x = m + UKL_PH_TREE + UKL_PH_TW * t;
    x[0] = at;
    x[1] = at + UKL_PH_NODES(leaves) * w;
    x[2] = w;
    x[3] = leaves;
    x[4] = op;
    return x[1] + 2u * leaves * w;
}

UKL_INLINE void ukl_t2_lay(UKL_PRIVATE UklPh* r)
{
    UklPhPool m = r->m;
    uint32_t n = r->n, size = m[UKL_T2_SIZE];
    uint32_t chunks = (n + 1u + size - 1u) / size;
    m[UKL_T2_N] = n;
    m[UKL_T2_CHUNKS] = chunks;
    uint32_t at = ukl_t2_tree(m, UKL_T2_TP1, UKL_T2_H, chunks, UKL_T2_W_P1, UKL_T2_OP_P1);
    // The level sort's trees, at the items' bound (two a record): P1W says their leaves.
    uint32_t most = 2u * (n + 1u), ichunks = (most + size - 1u) / size;
    for (uint32_t d = 0u; d < 2u; d++) {
        at = ukl_t2_tree(m, UKL_T2_TP2 + 2u * d, at, ichunks, UKL_T2_KEYS, UKL_T2_OP_KEYS);
        at = ukl_t2_tree(m, UKL_T2_TP2 + 2u * d + 1u, at, UKL_T2_KEYS, 1u, UKL_T2_OP_ADD1);
    }
    at = ukl_t2_tree(m, UKL_T2_TP3, at, ichunks, 2u, UKL_T2_OP_LAST);
    uint32_t aw = 0u;
    m[UKL_T2_O_DEPTH] = aw;
    aw += n + 1u;
    m[UKL_T2_O_ITEMS] = aw;
    aw += 3u * most;
    m[UKL_T2_O_ITEMS2] = aw;
    aw += 3u * most;
    m[UKL_T2_O_KEYS] = aw;
    aw += 2u * UKL_T2_KEYS;
    m[UKL_T2_O_CONTAINER] = aw;
    aw += n + 1u;
    m[UKL_T2_O_HEAD_AT] = aw;
    aw += n + 1u;
    // Past a room, or past the arrivals' stride: what it needs (the pool's words, else the arrays').
    if (at > m[UKL_T2_ROOM] || UKL_PH_NODES(ichunks) > m[UKL_PH_MS])
        m[UKL_T2_NEED] = at;
    else if (aw > m[UKL_T2_AROOM])
        m[UKL_T2_NEED] = aw;
    ukl_fence_device();
}

UKL_INLINE uint32_t ukl_ph_items(UKL_PRIVATE UklPh* r, uint32_t ph)
{
    if (r->refused)
        return 0u;
    if (ph == UKL_T2_PH_LAY)
        return 1u;
    if (ukl_atomic_poll(&r->m[UKL_T2_NEED]) != 0u)
        return 0u;
    if (ph >= UKL_T2_PH_P2 && ph < UKL_T2_PH_P2 + 8u)
        return (ph - UKL_T2_PH_P2) % 4u == 1u || (ph - UKL_T2_PH_P2) % 4u == 2u ? UKL_T2_KEYS : r->m[UKL_T2_ICHUNKS];
    if (ph == UKL_T2_PH_P3 || ph == UKL_T2_PH_P3W)
        return r->m[UKL_T2_ICHUNKS];
    return r->m[UKL_T2_CHUNKS];
}

UKL_INLINE void ukl_ph_item(UKL_PRIVATE UklPh* r, uint32_t ph, uint32_t c)
{
    UklPhPool m = r->m;
    if (ph == UKL_T2_PH_LAY) {
        ukl_t2_lay(r);
        return;
    }
    uint32_t n = r->n, size = m[UKL_T2_SIZE], a = c * size, b = a + size < n + 1u ? a + size : n + 1u;
    if (ph == UKL_T2_PH_P1) {
        ukl_t2_p1_run(r->src, r->starts, n, a, b, ukl_ph_leaf(r, UKL_T2_TP1, c));
        ukl_ph_up(r, UKL_T2_TP1, c);
    } else if (ph == UKL_T2_PH_P1W) {
        UklPhPool s = ukl_ph_sum(r, UKL_T2_TP1, c);
        ukl_t2_p1_write(r->src, r->starts, n, a, b, s, r->arrays + m[UKL_T2_O_DEPTH], r->arrays + m[UKL_T2_O_ITEMS], r->arrays + m[UKL_T2_O_HEAD_AT]);
        if (c == 0u) {
            uint32_t ni = ukl_ph_total(r, UKL_T2_TP1)[2], ic = (ni + size - 1u) / size;
            m[UKL_T2_ITEMS] = ni;
            m[UKL_T2_ICHUNKS] = ic;
            m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TP2 + 3u] = ic;
            m[UKL_PH_TREE + UKL_PH_TW * (UKL_T2_TP2 + 2u) + 3u] = ic;
            m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TP3 + 3u] = ic;
        }
    } else if (ph == UKL_T2_PH_P3 || ph == UKL_T2_PH_P3W) {
        // Over the items in level order (the sort's last digit's, in `items`).
        uint32_t ni = m[UKL_T2_ITEMS], ia = c * size, ib = ia + size < ni ? ia + size : ni;
        UKL_GLOBAL uint32_t* items = r->arrays + m[UKL_T2_O_ITEMS];
        if (ph == UKL_T2_PH_P3) {
            ukl_t2_p3_run(items, ia, ib, ukl_ph_leaf(r, UKL_T2_TP3, c));
            ukl_ph_up(r, UKL_T2_TP3, c);
        } else {
            ukl_t2_p3_write(items, ia, ib, ukl_ph_sum(r, UKL_T2_TP3, c), r->arrays + m[UKL_T2_O_CONTAINER], r->arrays + m[UKL_T2_O_HEAD_AT]);
        }
    } else {
        // The level sort's digit `d`, its step `k`.
        uint32_t d = (ph - UKL_T2_PH_P2) / 4u, k = (ph - UKL_T2_PH_P2) % 4u, tc = UKL_T2_TP2 + 2u * d, tk = tc + 1u;
        uint32_t ni = m[UKL_T2_ITEMS], ia = c * size, ib = ia + size < ni ? ia + size : ni;
        UKL_GLOBAL uint32_t* from = r->arrays + m[d == 0u ? UKL_T2_O_ITEMS : UKL_T2_O_ITEMS2];
        UKL_GLOBAL uint32_t* to = r->arrays + m[d == 0u ? UKL_T2_O_ITEMS2 : UKL_T2_O_ITEMS];
        UKL_GLOBAL uint32_t* keys = r->arrays + m[UKL_T2_O_KEYS] + d * UKL_T2_KEYS;
        if (k == 0u) {
            ukl_t2_p2_run(from, ia, ib, 8u * d, ukl_ph_leaf(r, tc, c));
            ukl_ph_up(r, tc, c);
        } else if (k == 1u) {
            ukl_ph_leaf(r, tk, c)[0] = ukl_ph_total(r, tc)[c];
            ukl_ph_up(r, tk, c);
        } else if (k == 2u) {
            keys[c] = ukl_ph_sum(r, tk, c)[0];
        } else {
            ukl_t2_p2_write(from, ia, ib, 8u * d, ukl_ph_sum(r, tc, c), keys, to);
        }
    }
}
