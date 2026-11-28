// ukl/writer_mid.h: format 16's cursor from the middle on the device (decision 0726): `writer16::Cursor<X, true>`'s
// ops, one function each, the bytes they write the same. A cursor writes one chunk of a tape's nodes, from node `n`
// with the frames of the nodes open there, while other cursors write the other chunks at once: every block is made
// before (`State::make_blocks`), and none is settled by a cursor (`finish` makes each final).
//
// Chunks write next to each other in one block, so each cell is stored at its width alone (a byte at a time), never as
// the word with the bytes past it kept that `ukl_w_cell_put` stores. The chunks run twice: counting (`count`), each
// side-list entry's row marked in its list's mask (an atomic or, two 32-bit words of each 64-bit word); then writing,
// each entry at its rank among the mask's rows.
//
// The frames are the cursor's own, `UKL_WMID_FRAME` words each in `f` (the node, its children so far, its last child
// or `CLIF_W_NONE`, its start's low and high words), as `Cursor`'s are its `OPEN` bytes each.
#pragma once
#include <ukl/writer.h>

#define UKL_WMID_FRAME 5u
// A side area's masks: after its counts and its four lists (`writer16::SIDE_MASKS`).
#define UKL_WMID_SIDE_MASKS (16u + 4u * CLIF_W_SIDE_LIST)

typedef struct UklWMid {
    UKL_GLOBAL const uint8_t* t;
    uint64_t s;
    uint64_t room;
    uint64_t area;
    uint32_t n;
    uint64_t prev_start;
    UKL_GLOBAL uint32_t* f;
    uint32_t open_len;
    bool count;
} UklWMid;

// `v`'s low `w` bytes at `a`, a byte at a time.
UKL_INLINE void ukl_wmid_store(UKL_W_T, uint64_t a, uint64_t w, uint64_t v)
{
    for (uint64_t k = 0u; k < w; k++)
        ukl_w_st8(t, a + k, (v >> (8u * k)) & 255u);
}

// Block `b`'s room and side area held.
UKL_INLINE void ukl_wmid_hold(UKL_PRIVATE UklWMid* d, uint32_t b)
{
    d->room = ukl_w_room_of(d->t, d->s, b);
    d->area = ukl_w_area_of(d->t, d->s, b);
}

// Row `j`'s entry `v` in side list `q` of the area at `area` (`State::mid_entry`).
UKL_INLINE void ukl_wmid_entry(UKL_PRIVATE UklWMid* d, uint64_t area, uint32_t q, uint32_t j, uint64_t v)
{
    UKL_GLOBAL const uint8_t* t = d->t;
    uint64_t masks = area + UKL_WMID_SIDE_MASKS + (uint64_t)q * (CLIF_W_Z / 8u);
    uint64_t word = masks + 8u * (uint64_t)(j >> 6);
    uint32_t bit = j & 63u;
    if (d->count) {
        ukl_atomic_or((UKL_GLOBAL uint32_t*)(UKL_GLOBAL uint8_t*)(t + word + (bit >= 32u ? 4u : 0u)), 1u << (bit & 31u));
        return;
    }
    uint32_t k = ukl_popcount64(ukl_w_ld64(t, word) & (((uint64_t)1 << bit) - 1u));
    for (uint32_t w = 0u; w < (j >> 6); w++)
        k += ukl_popcount64(ukl_w_ld64(t, masks + 8u * (uint64_t)w));
    UklWList l = ukl_w_list(t, area, q);
    ukl_w_st16(t, l.js + 2u * (uint64_t)k, j);
    ukl_w_st64(t, l.vs + 8u * (uint64_t)k, v);
}

// Row `j`'s value `v` in array `a` of the block at `room` and `area` (`State::mid_put_x`).
UKL_INLINE void ukl_wmid_put_in(UKL_PRIVATE UklWMid* d, uint64_t room, uint64_t area, uint32_t a, uint32_t j, uint64_t v)
{
    uint64_t esc = ukl_w_esc(d->t, d->s, a);
    if (v >= esc)
        ukl_wmid_entry(d, area, a + 1u, j, v);
    uint64_t w = ukl_w_width(d->t, d->s, a);
    ukl_wmid_store(d->t, room + ukl_w_col(d->t, d->s, a + 1u) + (uint64_t)j * w, w, v < esc ? v : esc);
}

// In the held block.
UKL_INLINE void ukl_wmid_put(UKL_PRIVATE UklWMid* d, uint32_t a, uint32_t j, uint64_t v) { ukl_wmid_put_in(d, d->room, d->area, a, j, v); }

// Row `j`'s code `number` in the held block (`State::mid_code_at`).
UKL_INLINE void ukl_wmid_code(UKL_PRIVATE UklWMid* d, uint32_t j, uint32_t number)
{
    UKL_GLOBAL const uint8_t* t = d->t;
    uint32_t cut = ukl_w_get32(t, d->s, CLIF_W_AT_CUT);
    if (number < cut) {
        ukl_w_st8(t, d->room + j, number);
        return;
    }
    uint32_t per = ukl_w_get32(t, d->s, CLIF_W_AT_PER);
    uint32_t past = number - cut;
    ukl_w_st8(t, d->room + j, cut + past / per);
    ukl_wmid_entry(d, d->area, 0u, j, past % per);
}

// Row `j`'s checkpoint `start` in the held block.
UKL_INLINE void ukl_wmid_checkpoint(UKL_PRIVATE UklWMid* d, uint32_t j, uint64_t start)
{
    uint64_t cw = ukl_w_get32(d->t, d->s, CLIF_W_AT_CW);
    ukl_wmid_store(d->t, d->room + ukl_w_col(d->t, d->s, 4u) + (uint64_t)(j / CLIF_W_K) * cw, cw, start);
}

// A frame pushed: node `i`, no children, none last, its start.
UKL_INLINE void ukl_wmid_push(UKL_PRIVATE UklWMid* d, uint32_t i, uint64_t start)
{
    UKL_GLOBAL uint32_t* f = d->f + d->open_len * UKL_WMID_FRAME;
    f[0] = i;
    f[1] = 0u;
    f[2] = CLIF_W_NONE;
    f[3] = (uint32_t)start;
    f[4] = (uint32_t)(start >> 32);
    d->open_len += 1u;
}

// A cursor from the middle on the state at `s` of region `t` (`Cursor::resume`): its next node `n`, the start of
// node `n - 1`, the frames of the nodes open there already in `f` (`open_len` of them, outermost first).
UKL_INLINE UklWMid ukl_wmid_resume(UKL_W_T, uint64_t s, uint32_t n, uint64_t prev_start, UKL_GLOBAL uint32_t* f, uint32_t open_len, bool count)
{
    UklWMid d;
    d.t = t;
    d.s = s;
    d.n = n;
    d.prev_start = prev_start;
    d.f = f;
    d.open_len = open_len;
    d.count = count;
    ukl_wmid_hold(&d, n == 0u ? 0u : (n - 1u) / CLIF_W_Z);
    return d;
}

// The file's first node (`Cursor::first_from_middle`).
UKL_INLINE void ukl_wmid_first(UKL_PRIVATE UklWMid* d, uint32_t number, uint64_t start, bool push)
{
    ukl_wmid_hold(d, 0u);
    ukl_wmid_code(d, 0u, number);
    ukl_wmid_put(d, 1u, 0u, 0u);
    ukl_wmid_checkpoint(d, 0u, start);
    d->prev_start = start;
    d->n = 1u;
    if (push)
        ukl_wmid_push(d, 0u, start);
}

// Node `n` of `number` at `start` in the held block, its block's first where `first` (`Cursor::node`): its row.
UKL_INLINE uint32_t ukl_wmid_node(UKL_PRIVATE UklWMid* d, bool first, uint32_t number, uint64_t start, bool push)
{
    uint32_t i = d->n;
    uint32_t j = i % CLIF_W_Z;
    if (d->open_len != 0u) {
        UKL_GLOBAL uint32_t* f = d->f + (d->open_len - 1u) * UKL_WMID_FRAME;
        uint32_t p = f[0], idx = f[1], last = f[2];
        f[1] = idx + 1u;
        f[2] = i;
        if (i - 1u != p) {
            uint64_t v = idx % CLIF_W_K == 0u ? ((uint64_t)(i - p) << 2) | 3u : ((uint64_t)(i - last) << 2) | 1u;
            if (first) {
                // The node before is the block before's last.
                uint32_t b = (i - 1u) / CLIF_W_Z;
                ukl_wmid_put_in(d, ukl_w_room_of(d->t, d->s, b), ukl_w_area_of(d->t, d->s, b), 0u, (i - 1u) % CLIF_W_Z, v);
            } else {
                ukl_wmid_put(d, 0u, j - 1u, v);
            }
        }
    }
    ukl_wmid_code(d, j, number);
    ukl_wmid_put(d, 1u, j, start - d->prev_start);
    if (j % CLIF_W_K == 0u)
        ukl_wmid_checkpoint(d, j, start);
    d->prev_start = start;
    d->n = i + 1u;
    if (push)
        ukl_wmid_push(d, i, start);
    return j;
}

// A node opened at `start` (`Cursor::open`).
UKL_INLINE void UklWMid_open(UKL_PRIVATE UklWMid* d, uint32_t number, uint64_t start)
{
    if (d->n == 0u) {
        ukl_wmid_first(d, number, start, true);
    } else if (d->n % CLIF_W_Z == 0u) {
        ukl_wmid_hold(d, d->n / CLIF_W_Z);
        ukl_wmid_node(d, true, number, start, true);
    } else {
        ukl_wmid_node(d, false, number, start, true);
    }
}

// A leaf from `start` to `end` (`Cursor::leaf`).
UKL_INLINE void UklWMid_leaf(UKL_PRIVATE UklWMid* d, uint32_t number, uint64_t start, uint64_t end)
{
    if (d->n == 0u) {
        ukl_wmid_first(d, number, start, false);
        ukl_wmid_put(d, 2u, 0u, end - start);
        return;
    }
    bool first = d->n % CLIF_W_Z == 0u;
    if (first)
        ukl_wmid_hold(d, d->n / CLIF_W_Z);
    uint32_t j = ukl_wmid_node(d, first, number, start, false);
    ukl_wmid_put(d, 2u, j, end - start);
}

// A node of one leaf child, both at once (`Cursor::leaf_in`); never a chunk's first node.
UKL_INLINE void UklWMid_leaf_in(UKL_PRIVATE UklWMid* d, uint32_t number, uint32_t child, uint64_t start, uint64_t cs, uint64_t ce, uint64_t end)
{
    bool first = d->n % CLIF_W_Z == 0u;
    if (first)
        ukl_wmid_hold(d, d->n / CLIF_W_Z);
    uint32_t j = ukl_wmid_node(d, first, number, start, false);
    ukl_wmid_put(d, 2u, j, end - start);
    ukl_wmid_put(d, 0u, j, 2u << 1);
    // The child: the node's first, so no slot of the node before describes it.
    uint32_t cj = j + 1u;
    if (cj == CLIF_W_Z)
        ukl_wmid_hold(d, d->n / CLIF_W_Z);
    cj = cj % CLIF_W_Z;
    ukl_wmid_code(d, cj, child);
    ukl_wmid_put(d, 1u, cj, cs - start);
    if (cj % CLIF_W_K == 0u)
        ukl_wmid_checkpoint(d, cj, cs);
    ukl_wmid_put(d, 2u, cj, ce - cs);
    d->prev_start = cs;
    d->n += 1u;
}

// The innermost open node closed at `end` (`Cursor::close`): its length, and an inner node's size.
UKL_INLINE void UklWMid_close(UKL_PRIVATE UklWMid* d, uint64_t end)
{
    UKL_GLOBAL uint32_t* f = d->f + (d->open_len - 1u) * UKL_WMID_FRAME;
    uint32_t i = f[0];
    uint64_t start = (uint64_t)f[3] | ((uint64_t)f[4] << 32);
    d->open_len -= 1u;
    uint32_t j = i % CLIF_W_Z;
    uint32_t size = d->n - i;
    // In the held block, or in a block before, made already.
    uint64_t room = d->room, area = d->area;
    if (i / CLIF_W_Z != (d->n - 1u) / CLIF_W_Z) {
        room = ukl_w_room_of(d->t, d->s, i / CLIF_W_Z);
        area = ukl_w_area_of(d->t, d->s, i / CLIF_W_Z);
    }
    ukl_wmid_put_in(d, room, area, 2u, j, end - start);
    if (size > 1u)
        ukl_wmid_put_in(d, room, area, 0u, j, (uint64_t)size << 1);
}

// `make_blocks`' block `b` of a fresh format 16 writer (no node, no block made, none given back), every block at once,
// a thread a block: its room its place in the tape, its side area the `b`-th, its table entry, its counts and its
// side lists' masks 0, its presence 0 where kept. Each part's room checked; `ukl_wmid_made` sets their lengths.
UKL_INLINE uint32_t ukl_wmid_make_block(UKL_W_T, uint64_t s, uint32_t b)
{
    uint64_t stride = ukl_w_get32(t, s, CLIF_W_AT_STRIDE);
    uint64_t room = (uint64_t)b * stride + CLIF_W_HEAD16;
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_OUT, room + stride));
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_TABLE, ((uint64_t)b + 1u) * CLIF_W_TABLE));
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_SIDES, ((uint64_t)b + 1u) * CLIF_W_SIDE_AREA));
    bool keep = (ukl_w_get32(t, s, CLIF_W_AT_FLAGS) & 1u) != 0u;
    uint64_t bytes = keep ? ukl_w_presence_bytes(t, s) : 0u;
    if (keep)
        UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_PRESENCE, ((uint64_t)b + 2u) * bytes));
    clif_w_zero(t, s, ukl_w_rooms(t, s) + room, stride);
    uint64_t e = ukl_w_entry(t, s, b);
    ukl_w_st32(t, e, (uint32_t)room);
    ukl_w_st32(t, e + 4u, b);
    ukl_w_st32(t, e + 8u, 0u);
    uint64_t area = ukl_w_area_of(t, s, b);
    ukl_w_st64(t, area, 0u);
    ukl_w_st64(t, area + 8u, 0u);
    for (uint32_t w = 0u; w < 4u * CLIF_W_Z / 64u; w++)
        ukl_w_st64(t, area + UKL_WMID_SIDE_MASKS + 8u * (uint64_t)w, 0u);
    if (keep)
        clif_w_zero(t, s, ukl_w_part(t, s, CLIF_W_PART_PRESENCE) + ((uint64_t)b + 1u) * bytes, bytes);
    return 0u;
}

// `make_blocks`' parts once its `nb` blocks are made: their lengths and the blocks' count (one thread, the state's
// fields read by no block's thread). A writer not fresh, or in 16z, is refused as the Rust refuses it.
UKL_INLINE uint32_t ukl_wmid_made(UKL_W_T, uint64_t s, uint32_t nb)
{
    if (ukl_w_is_z(t, s) || ukl_w_get32(t, s, CLIF_W_AT_N) != 0u || ukl_w_get32(t, s, CLIF_W_AT_BLOCKS) != 0u)
        return clif_w_fail(t, s, CLIF_W_OUTSIDE, 0u);
    uint64_t stride = ukl_w_get32(t, s, CLIF_W_AT_STRIDE);
    ukl_w_grow(t, s, CLIF_W_PART_OUT, (uint64_t)nb * stride + CLIF_W_HEAD16);
    ukl_w_grow(t, s, CLIF_W_PART_TABLE, (uint64_t)nb * CLIF_W_TABLE);
    ukl_w_set_len(t, s, CLIF_W_PART_SIDES, (uint64_t)nb * CLIF_W_SIDE_AREA);
    if ((ukl_w_get32(t, s, CLIF_W_AT_FLAGS) & 1u) != 0u)
        ukl_w_grow(t, s, CLIF_W_PART_PRESENCE, ((uint64_t)nb + 1u) * ukl_w_presence_bytes(t, s));
    ukl_w_set32(t, s, CLIF_W_AT_BLOCKS, nb);
    return 0u;
}

// `end_middle`'s block `b`, a thread a block: each side list's count, its mask's rows.
UKL_INLINE void ukl_wmid_end_block(UKL_W_T, uint64_t s, uint32_t b)
{
    uint64_t area = ukl_w_area_of(t, s, b);
    for (uint32_t q = 0u; q < 4u; q++) {
        uint64_t masks = area + UKL_WMID_SIDE_MASKS + (uint64_t)q * (CLIF_W_Z / 8u);
        uint32_t c = 0u;
        for (uint32_t w = 0u; w < CLIF_W_Z / 64u; w++)
            c += ukl_popcount64(ukl_w_ld64(t, masks + 8u * (uint64_t)w));
        ukl_w_st32(t, area + 4u * (uint64_t)q, c);
    }
}

// `end_middle`'s state (one thread): `n` nodes, the last one's start, none open, each node final as written.
UKL_INLINE void ukl_wmid_ended(UKL_W_T, uint64_t s, uint32_t n, uint64_t prev_start)
{
    ukl_w_set32(t, s, CLIF_W_AT_N, n);
    ukl_w_set64(t, s, CLIF_W_AT_PREV_START, prev_start);
    ukl_w_set32(t, s, CLIF_W_AT_OPEN_LEN, 0u);
    ukl_w_set32(t, s, CLIF_W_AT_FRONTIER, n);
}
