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
//   P4   each chunk walked from every place (`ukl_t2_walk`), its levels known (`known_rets`): its map of places, a leaf
//        of P4's tree (`places_merge_w`); P4W each chunk's place (its sum's at place 0), and the first chunk refused
//        at its place (a min).
//   P5   each chunk from its place: its shape (`ukl_t2_walk`'s events folded), a leaf of P5's tree (`shape_merge`);
//        P5W each chunk's sum written, and its walk again writing each event at its number, with its items; item 0
//        says the events, their items and the nodes (P5's root).
//   P7   the events' items sorted by frame depth, as P2 (`ukl_t2_sort`), from `eitems` to `eitems2` and back.
//   P8   over the events' items in frame order: P8a each item's frame, the segmented last-value of the heads (`p3_run`,
//        `last_merge`, `p8a_write`); P8b each item's frame's children so far and the last one's node (`p8b_run`,
//        `kids_merge`, `p8b_write`).

#pragma once
#define UKL_PH_FIELDS UKL_GLOBAL const uint8_t* src; UKL_GLOBAL const uint32_t* starts; UKL_GLOBAL uint32_t* arrays; UKL_GLOBAL const uint32_t* toks; \
    UKL_GLOBAL uint8_t* rets; uint32_t n; uint32_t len; bool refused;
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
#define UKL_T2_O_PLACE (UKL_PH_H + 14u)
#define UKL_T2_O_SHAPE_AT (UKL_PH_H + 15u)
#define UKL_T2_O_EVENTS (UKL_PH_H + 16u)
#define UKL_T2_O_EITEMS (UKL_PH_H + 17u)
// The first chunk refused at its place (`NONE`: none), the events, their items, the nodes.
#define UKL_T2_REFUSED_AT (UKL_PH_H + 18u)
#define UKL_T2_EVENTS (UKL_PH_H + 19u)
#define UKL_T2_EITEMS (UKL_PH_H + 20u)
#define UKL_T2_NODES (UKL_PH_H + 21u)
#define UKL_T2_O_EITEMS2 (UKL_PH_H + 22u)
#define UKL_T2_ECHUNKS (UKL_PH_H + 23u)
#define UKL_T2_O_EKEYS (UKL_PH_H + 24u)
#define UKL_T2_O_FRAME_OF (UKL_PH_H + 25u)
#define UKL_T2_O_KIDS (UKL_PH_H + 26u)
#define UKL_T2_H (UKL_PH_H + 32u)

#define UKL_T2_PH_LAY 0u
#define UKL_T2_PH_P1 1u
#define UKL_T2_PH_P1W 2u
// The level sort's digit `d`'s four: `UKL_T2_PH_P2 + 4d` its C, then K, KW and S.
#define UKL_T2_PH_P2 3u
#define UKL_T2_PH_P3 11u
#define UKL_T2_PH_P3W 12u
#define UKL_T2_PH_P4 13u
#define UKL_T2_PH_P4W 14u
#define UKL_T2_PH_P5 15u
#define UKL_T2_PH_P5W 16u
#define UKL_T2_PH_P7 17u
#define UKL_T2_PH_P8A 25u
#define UKL_T2_PH_P8AW 26u
#define UKL_T2_PH_P8B 27u
#define UKL_T2_PH_P8BW 28u
#define UKL_T2_PHASES 29u

#define UKL_T2_TP1 0u
// Digit `d`'s trees: its chunks' counts `UKL_T2_TP2 + 2d`, its keys' after it.
#define UKL_T2_TP2 1u
#define UKL_T2_TP3 5u
#define UKL_T2_TP4 6u
#define UKL_T2_TP5 7u
#define UKL_T2_TP7 8u
#define UKL_T2_TP8A 12u
#define UKL_T2_TP8B 13u
#define UKL_T2_OP_P1 0u
#define UKL_T2_OP_ADD1 1u
#define UKL_T2_OP_KEYS 2u
#define UKL_T2_OP_LAST 3u
#define UKL_T2_OP_PLACES 4u
#define UKL_T2_OP_SHAPE 5u
#define UKL_T2_OP_KIDS 6u
// A place map's words (the grammar's places, at most this many), a shape's.
#define UKL_T2_W_PLACES 32u
#define UKL_T2_W_SHAPE 10u
#define UKL_T2_NONE 4294967295u
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
UKL_INLINE void ukl_t2_p8a_write(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, UKL_COHERENT const uint32_t* sum, UKL_GLOBAL uint32_t* frame_of);
UKL_INLINE void ukl_t2_p8b_run(UKL_GLOBAL const uint32_t* items, UKL_GLOBAL const uint32_t* events, uint32_t a, uint32_t b, UKL_COHERENT uint32_t* out);
UKL_INLINE void ukl_t2_p8b_write(UKL_GLOBAL const uint32_t* items, UKL_GLOBAL const uint32_t* events, uint32_t a, uint32_t b, UKL_COHERENT const uint32_t* sum,
                                 UKL_GLOBAL uint32_t* kids);
UKL_INLINE uint32_t ukl_t2_places(void);
UKL_INLINE uint32_t ukl_t2_depth(void);
UKL_INLINE uint32_t ukl_t2_walk_refused(void);
UKL_INLINE uint32_t ukl_t2_walk(UKL_GLOBAL const uint8_t* src, UKL_GLOBAL const uint32_t* starts, uint32_t n, uint32_t len, UKL_GLOBAL const uint32_t* toks,
                                UKL_GLOBAL const uint32_t* depth, UKL_GLOBAL const uint32_t* container, uint32_t low, uint32_t a, uint32_t b, uint32_t pl,
                                UKL_GLOBAL uint8_t* ret, UKL_PRIVATE uint32_t* s, bool write, UKL_GLOBAL uint32_t* ev, UKL_GLOBAL uint32_t* items, UKL_COHERENT const uint32_t* at);
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
    at = ukl_t2_tree(m, UKL_T2_TP4, at, chunks, UKL_T2_W_PLACES, UKL_T2_OP_PLACES);
    at = ukl_t2_tree(m, UKL_T2_TP5, at, chunks, UKL_T2_W_SHAPE, UKL_T2_OP_SHAPE);
    // The events' items' trees, at their bound (P5W says their leaves).
    uint32_t most_ev = 2u * (n + 1u) + 2u, echunks = (2u * most_ev + size - 1u) / size;
    for (uint32_t d = 0u; d < 2u; d++) {
        at = ukl_t2_tree(m, UKL_T2_TP7 + 2u * d, at, echunks, UKL_T2_KEYS, UKL_T2_OP_KEYS);
        at = ukl_t2_tree(m, UKL_T2_TP7 + 2u * d + 1u, at, UKL_T2_KEYS, 1u, UKL_T2_OP_ADD1);
    }
    at = ukl_t2_tree(m, UKL_T2_TP8A, at, echunks, 2u, UKL_T2_OP_LAST);
    at = ukl_t2_tree(m, UKL_T2_TP8B, at, echunks, 3u, UKL_T2_OP_KIDS);
    m[UKL_T2_REFUSED_AT] = UKL_T2_NONE;
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
    m[UKL_T2_O_PLACE] = aw;
    aw += chunks;
    m[UKL_T2_O_SHAPE_AT] = aw;
    aw += UKL_T2_W_SHAPE * chunks;
    // The events at their bound (two a record, and two), their items two an event.
    m[UKL_T2_O_EVENTS] = aw;
    aw += 8u * most_ev;
    m[UKL_T2_O_EITEMS] = aw;
    aw += 6u * most_ev;
    m[UKL_T2_O_EITEMS2] = aw;
    aw += 6u * most_ev;
    m[UKL_T2_O_EKEYS] = aw;
    aw += 2u * UKL_T2_KEYS;
    m[UKL_T2_O_FRAME_OF] = aw;
    aw += 2u * most_ev;
    m[UKL_T2_O_KIDS] = aw;
    aw += 6u * most_ev;
    // Past a room, or past the arrivals' stride: what it needs (the pool's words, else the arrays').
    if (at > m[UKL_T2_ROOM] || UKL_PH_NODES(ichunks) > m[UKL_PH_MS] || UKL_PH_NODES(echunks) > m[UKL_PH_MS])
        m[UKL_T2_NEED] = at;
    else if (aw > m[UKL_T2_AROOM])
        m[UKL_T2_NEED] = aw;
    ukl_fence_device();
}

// A level sort (P2 of the records' items, P7 of the events'), its trees from `t0` (digit `d`'s counts `t0 + 2d`, its
// keys' after): digit `d`'s step `k` (C, K, KW, S), item `c`, over `ni` items from `oa` to `ob` (digit 1 back), the
// keys' starts at `okeys`.
UKL_INLINE void ukl_t2_sort(UKL_PRIVATE UklPh* r, uint32_t t0, uint32_t d, uint32_t k, uint32_t c, uint32_t ni, uint32_t oa, uint32_t ob, uint32_t okeys)
{
    uint32_t size = r->m[UKL_T2_SIZE], tc = t0 + 2u * d, tk = tc + 1u, ia = c * size, ib = ia + size < ni ? ia + size : ni;
    UKL_GLOBAL uint32_t* from = r->arrays + (d == 0u ? oa : ob);
    UKL_GLOBAL uint32_t* to = r->arrays + (d == 0u ? ob : oa);
    UKL_GLOBAL uint32_t* keys = r->arrays + okeys + d * UKL_T2_KEYS;
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
    if (ph >= UKL_T2_PH_P5 && ukl_atomic_poll(&r->m[UKL_T2_REFUSED_AT]) != UKL_T2_NONE)
        return 0u;
    if (ph >= UKL_T2_PH_P7 && ph < UKL_T2_PH_P7 + 8u)
        return (ph - UKL_T2_PH_P7) % 4u == 1u || (ph - UKL_T2_PH_P7) % 4u == 2u ? UKL_T2_KEYS : r->m[UKL_T2_ECHUNKS];
    if (ph >= UKL_T2_PH_P8A)
        return r->m[UKL_T2_ECHUNKS];
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
    } else if (ph >= UKL_T2_PH_P4 && ph <= UKL_T2_PH_P5W) {
        UKL_GLOBAL const uint32_t* depth = r->arrays + m[UKL_T2_O_DEPTH];
        UKL_GLOBAL const uint32_t* container = r->arrays + m[UKL_T2_O_CONTAINER];
        UKL_GLOBAL uint8_t* ret = r->rets + c * ukl_t2_depth();
        UKL_GLOBAL uint32_t* place = r->arrays + m[UKL_T2_O_PLACE];
        uint32_t low = ukl_ph_leaf(r, UKL_T2_TP1, c)[1], places = ukl_t2_places(), s[UKL_T2_W_SHAPE];
        if (ph == UKL_T2_PH_P4) {
            UklPhPool map = ukl_ph_leaf(r, UKL_T2_TP4, c);
            for (uint32_t p = 0u; p < places; p++)
                map[p] = ukl_t2_walk(r->src, r->starts, n, r->len, r->toks, depth, container, low, a, b, p, ret, s, false, r->arrays, r->arrays, map);
            ukl_ph_up(r, UKL_T2_TP4, c);
        } else if (ph == UKL_T2_PH_P4W) {
            uint32_t pl = ukl_ph_sum(r, UKL_T2_TP4, c)[0];
            place[c] = pl;
            if (pl < places && ukl_ph_leaf(r, UKL_T2_TP4, c)[pl] == ukl_t2_walk_refused())
                ukl_atomic_min(&m[UKL_T2_REFUSED_AT], c);
        } else if (ph == UKL_T2_PH_P5) {
            UklPhPool leaf = ukl_ph_leaf(r, UKL_T2_TP5, c);
            if (place[c] < places) {
                ukl_t2_walk(r->src, r->starts, n, r->len, r->toks, depth, container, low, a, b, place[c], ret, s, false, r->arrays, r->arrays, leaf);
                for (uint32_t k = 0u; k < UKL_T2_W_SHAPE; k++)
                    leaf[k] = s[k];
            } else {
                ukl_ph_unit(UKL_T2_OP_SHAPE, leaf);
            }
            ukl_ph_up(r, UKL_T2_TP5, c);
        } else {
            UklPhPool at = ukl_ph_sum(r, UKL_T2_TP5, c);
            UKL_GLOBAL uint32_t* shape_at = r->arrays + m[UKL_T2_O_SHAPE_AT] + UKL_T2_W_SHAPE * c;
            for (uint32_t k = 0u; k < UKL_T2_W_SHAPE; k++)
                shape_at[k] = at[k];
            if (place[c] < places)
                ukl_t2_walk(r->src, r->starts, n, r->len, r->toks, depth, container, low, a, b, place[c], ret, s, true, r->arrays + m[UKL_T2_O_EVENTS],
                            r->arrays + m[UKL_T2_O_EITEMS], at);
            if (c == 0u) {
                UklPhPool total = ukl_ph_total(r, UKL_T2_TP5);
                m[UKL_T2_NODES] = total[0];
                m[UKL_T2_EVENTS] = total[1];
                m[UKL_T2_EITEMS] = total[2];
                uint32_t ec = (total[2] + size - 1u) / size;
                m[UKL_T2_ECHUNKS] = ec;
                m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TP7 + 3u] = ec;
                m[UKL_PH_TREE + UKL_PH_TW * (UKL_T2_TP7 + 2u) + 3u] = ec;
                m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TP8A + 3u] = ec;
                m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TP8B + 3u] = ec;
            }
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
    } else if (ph >= UKL_T2_PH_P8A) {
        // Over the events' items in frame order (P7's last digit's, in `eitems`).
        uint32_t ni = m[UKL_T2_EITEMS], ia = c * size, ib = ia + size < ni ? ia + size : ni;
        UKL_GLOBAL uint32_t* items = r->arrays + m[UKL_T2_O_EITEMS];
        UKL_GLOBAL uint32_t* events = r->arrays + m[UKL_T2_O_EVENTS];
        if (ph == UKL_T2_PH_P8A) {
            ukl_t2_p3_run(items, ia, ib, ukl_ph_leaf(r, UKL_T2_TP8A, c));
            ukl_ph_up(r, UKL_T2_TP8A, c);
        } else if (ph == UKL_T2_PH_P8AW) {
            ukl_t2_p8a_write(items, ia, ib, ukl_ph_sum(r, UKL_T2_TP8A, c), r->arrays + m[UKL_T2_O_FRAME_OF]);
        } else if (ph == UKL_T2_PH_P8B) {
            ukl_t2_p8b_run(items, events, ia, ib, ukl_ph_leaf(r, UKL_T2_TP8B, c));
            ukl_ph_up(r, UKL_T2_TP8B, c);
        } else {
            ukl_t2_p8b_write(items, events, ia, ib, ukl_ph_sum(r, UKL_T2_TP8B, c), r->arrays + m[UKL_T2_O_KIDS]);
        }
    } else if (ph >= UKL_T2_PH_P7) {
        uint32_t k = ph - UKL_T2_PH_P7;
        ukl_t2_sort(r, UKL_T2_TP7, k / 4u, k % 4u, c, m[UKL_T2_EITEMS], m[UKL_T2_O_EITEMS], m[UKL_T2_O_EITEMS2], m[UKL_T2_O_EKEYS]);
    } else {
        uint32_t k = ph - UKL_T2_PH_P2;
        ukl_t2_sort(r, UKL_T2_TP2, k / 4u, k % 4u, c, m[UKL_T2_ITEMS], m[UKL_T2_O_ITEMS], m[UKL_T2_O_ITEMS2], m[UKL_T2_O_KEYS]);
    }
}
