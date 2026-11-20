// ukl/writer.h: format 16's writer on the device, its ops (decisions 0434 §6 R15, 0440, 0518). Written by hand.
//
// The ops are `rerail_clif::writer`'s, written once in CLIF for the host (its Rust and its JIT); these are the same
// ops written for the kernels, one function each, under the CLIF op's name (`clif_w_open` is `w_open`). They write
// the bytes the CLIF ops write: `the_device_writes_the_references_tape` runs them on the device against the hand
// reference (`f16::reference::Writer`), and `the_writer_headers_constants_are_the_ops` holds this file's layout
// constants to `writer.rs`'s. A change to an op is made in both, and the two tests say whether they agree.
//
// The writer's region is one buffer `t`: the writer's `State` at byte `s`, then its parts, each at its offset from
// the state, with its capacity and length. An address is a byte offset in `t` (a `uint64_t`); every load and store
// is at any alignment. A cell of the tape is written as a word: its bytes in a 64-bit load and store, the bytes
// past it kept, so a part has 8 bytes of room past its last cell.
//
// An op returns 0, or its failure: the first is kept in the state (`clif_w_fail`), and a full part names itself
// (`CLIF_W_FULL` plus its number), its op having written nothing past the part's capacity.
#pragma once
#include <ukl/memory.h>
#include <ukl/clif.h>

// ---- Format 16's constants (writer.rs's) ---------------------------------------------------------------------------

#define CLIF_W_Z 1024u
#define CLIF_W_K 8u
#define CLIF_W_BLOCK 256u
#define CLIF_W_HEAD16 120u
#define CLIF_W_MAX_NODES 1073741824u
#define CLIF_W_NONE 4294967295u
#define CLIF_W_MAGIC16 0x3642504154504d54ul
#define CLIF_W_MAGIC16Z 0x5a42504154504d54ul
// An open node's entry: its index, its children so far, its last child (u32s), its start (a u64 at 16).
#define CLIF_W_OPEN 24u
// A block's table entry: its room (from the rooms' start), its side area (`NONE` once final), whether it is final.
#define CLIF_W_TABLE 16u
// A block's laid parts: four (start in `laid`, length) pairs of u32s.
#define CLIF_W_LAID_AT 32u
// A side area: four lists' entry counts (u32s), then each list's node offsets (u16s, `Z`) and values (u64s, `Z`).
#define CLIF_W_SIDE_LIST 10240u
#define CLIF_W_SIDE_AREA 40976u
// 16z's piece index entry: start, stored length, raw length, kind, next in its bucket (u32s); its hash (u64 at 24).
#define CLIF_W_PIECE 32u
#define CLIF_W_RAW 0u
#define CLIF_W_LZ4 1u
#define CLIF_W_REPEAT 2u
#define CLIF_W_REPEAT_MIN 8u
#define CLIF_W_LZ4_MIN 16u
#define CLIF_W_LZ_HASH_LOG 15u
#define CLIF_W_LZ_ATTEMPTS 8u

// The failures.
#define CLIF_W_BOUNDS 1u
#define CLIF_W_OUTSIDE 2u
#define CLIF_W_FINAL 3u
#define CLIF_W_NOT_OPEN 4u
#define CLIF_W_NO_OPEN 5u
#define CLIF_W_UNCLOSED 6u
#define CLIF_W_FULL 256u

// The parts, in their order in the state.
#define CLIF_W_PART_KINDS 0u
#define CLIF_W_PART_OPEN 1u
#define CLIF_W_PART_OUT 2u
#define CLIF_W_PART_TABLE 3u
#define CLIF_W_PART_SIDES 4u
#define CLIF_W_PART_FREE 5u
#define CLIF_W_PART_LAID 6u
#define CLIF_W_PART_LAID_AT 7u
#define CLIF_W_PART_PRESENCE 8u
#define CLIF_W_PART_LINES 9u
#define CLIF_W_PART_ROOMS 10u
#define CLIF_W_PART_ROOM_FREE 11u
#define CLIF_W_PART_PIECES 12u
#define CLIF_W_PART_SEEN 13u
#define CLIF_W_PART_CKPT 14u
#define CLIF_W_PART_LZ 15u

// The state's fields: their byte offsets (`writer::at`).
#define CLIF_W_AT_ESC 0u
#define CLIF_W_AT_SOURCE_LEN 24u
#define CLIF_W_AT_PREV_START 32u
#define CLIF_W_AT_W 40u
#define CLIF_W_AT_CW 52u
#define CLIF_W_AT_VW 56u
#define CLIF_W_AT_CUT 72u
#define CLIF_W_AT_PER 76u
#define CLIF_W_AT_STRIDE 80u
#define CLIF_W_AT_AT 84u
#define CLIF_W_AT_FLAGS 104u
#define CLIF_W_AT_KW 108u
#define CLIF_W_AT_FW 112u
#define CLIF_W_AT_TAG 116u
#define CLIF_W_AT_MARK 120u
#define CLIF_W_AT_NEWLINES 124u
#define CLIF_W_AT_N 128u
#define CLIF_W_AT_FRONTIER 132u
#define CLIF_W_AT_BAD 136u
#define CLIF_W_AT_BAD_NODE 140u
#define CLIF_W_AT_OPEN_LEN 144u
#define CLIF_W_AT_BLOCKS 148u
#define CLIF_W_AT_ZBUCKETS 152u
#define CLIF_W_AT_ZBASE 156u
#define CLIF_W_AT_ZPREV 160u
#define CLIF_W_AT_ZNEXT 164u
#define CLIF_W_AT_ZOFF 168u
#define CLIF_W_AT_ZBACK 172u
#define CLIF_W_AT_PARTS 176u
#define CLIF_W_AT_PART 24u
#define CLIF_W_AT_P_AT 0u
#define CLIF_W_AT_P_CAP 8u
#define CLIF_W_AT_P_LEN 16u

// ---- The region's bytes, the state's fields, the parts -------------------------------------------------------------

#define UKL_W_T UKL_GLOBAL const uint8_t* t

UKL_INLINE uint32_t ukl_w_ld8(UKL_W_T, uint64_t a) { return ukl_ldc_u8(clif_coherent(t), a); }
UKL_INLINE uint32_t ukl_w_ld16(UKL_W_T, uint64_t a) { return ukl_ldc_u16(clif_coherent(t), a); }
UKL_INLINE uint32_t ukl_w_ld32(UKL_W_T, uint64_t a) { return ukl_ldc_u32(clif_coherent(t), a); }
UKL_INLINE uint64_t ukl_w_ld64(UKL_W_T, uint64_t a) { return ukl_ldc_u64(clif_coherent(t), a); }
UKL_INLINE void ukl_w_st8(UKL_W_T, uint64_t a, uint64_t x) { ukl_stc_u8(clif_written(t), a, x); }
UKL_INLINE void ukl_w_st16(UKL_W_T, uint64_t a, uint64_t x) { ukl_stc_u16(clif_written(t), a, x); }
UKL_INLINE void ukl_w_st32(UKL_W_T, uint64_t a, uint64_t x) { ukl_stc_u32(clif_written(t), a, x); }
UKL_INLINE void ukl_w_st64(UKL_W_T, uint64_t a, uint64_t x) { ukl_stc_u64(clif_written(t), a, x); }

UKL_INLINE uint32_t ukl_w_get32(UKL_W_T, uint64_t s, uint32_t at) { return ukl_w_ld32(t, s + at); }
UKL_INLINE uint64_t ukl_w_get64(UKL_W_T, uint64_t s, uint32_t at) { return ukl_w_ld64(t, s + at); }
UKL_INLINE void ukl_w_set32(UKL_W_T, uint64_t s, uint32_t at, uint32_t x) { ukl_w_st32(t, s + at, x); }
UKL_INLINE void ukl_w_set64(UKL_W_T, uint64_t s, uint32_t at, uint64_t x) { ukl_w_st64(t, s + at, x); }

UKL_INLINE uint32_t ukl_w_part_field(uint32_t k, uint32_t field) { return CLIF_W_AT_PARTS + k * CLIF_W_AT_PART + field; }
// Part `k`'s address, its capacity and its length.
UKL_INLINE uint64_t ukl_w_part(UKL_W_T, uint64_t s, uint32_t k) { return s + ukl_w_get64(t, s, ukl_w_part_field(k, CLIF_W_AT_P_AT)); }
UKL_INLINE uint64_t ukl_w_cap(UKL_W_T, uint64_t s, uint32_t k) { return ukl_w_get64(t, s, ukl_w_part_field(k, CLIF_W_AT_P_CAP)); }
UKL_INLINE uint64_t ukl_w_len(UKL_W_T, uint64_t s, uint32_t k) { return ukl_w_get64(t, s, ukl_w_part_field(k, CLIF_W_AT_P_LEN)); }
UKL_INLINE void ukl_w_set_len(UKL_W_T, uint64_t s, uint32_t k, uint64_t len) { ukl_w_set64(t, s, ukl_w_part_field(k, CLIF_W_AT_P_LEN), len); }
// Part `k`'s length at least `len`.
UKL_INLINE void ukl_w_grow(UKL_W_T, uint64_t s, uint32_t k, uint64_t len)
{
    if (ukl_w_len(t, s, k) < len)
        ukl_w_set_len(t, s, k, len);
}

// The low `w` bytes' mask (all ones from 8).
UKL_INLINE uint64_t ukl_w_mask(uint64_t w) { return w >= 8u ? ~(uint64_t)0 : ((uint64_t)1 << (w * 8u)) - 1u; }
// The cell of `w` bytes at `a`, written as a word: its bytes `v`'s, the bytes past it kept; and read.
UKL_INLINE void ukl_w_cell_put(UKL_W_T, uint64_t a, uint64_t w, uint64_t v)
{
    uint64_t m = ukl_w_mask(w);
    ukl_w_st64(t, a, (ukl_w_ld64(t, a) & ~m) | (v & m));
}
UKL_INLINE uint64_t ukl_w_cell_get(UKL_W_T, uint64_t a, uint64_t w) { return ukl_w_ld64(t, a) & ukl_w_mask(w); }

// An op's result passed on where it is a failure.
#define UKL_W_TRY(x)                                                                                                   \
    do {                                                                                                               \
        uint32_t r_ = (x);                                                                                             \
        if (r_ != 0u)                                                                                                  \
            return r_;                                                                                                 \
    } while (0)

// ---- The ops' leaves: a failure kept, a search, bytes moved -------------------------------------------------------

// `w_fail(code, node)`: the first failure kept; returns `code`.
UKL_COLD uint32_t clif_w_fail(UKL_W_T, uint64_t s, uint32_t code, uint32_t node)
{
    if (ukl_w_get32(t, s, CLIF_W_AT_BAD) == 0u) {
        ukl_w_set32(t, s, CLIF_W_AT_BAD, code);
        ukl_w_set32(t, s, CLIF_W_AT_BAD_NODE, node);
    }
    return code;
}

// 0, or a full part's failure unless part `k` holds `need` bytes.
UKL_INLINE uint32_t ukl_w_room(UKL_W_T, uint64_t s, uint32_t k, uint64_t need)
{
    return need > ukl_w_cap(t, s, k) ? clif_w_fail(t, s, CLIF_W_FULL + k, CLIF_W_NONE) : 0u;
}

// `w_find(js, count, j)`: the first of the `count` ascending u16 offsets at `js` not below `j`.
UKL_COLD uint32_t clif_w_find(UKL_W_T, uint64_t s, uint64_t js, uint32_t count, uint32_t j)
{
    uint32_t lo = 0u, hi = count;
    while (lo < hi) {
        uint32_t mid = (lo + hi) >> 1;
        if (ukl_w_ld16(t, js + 2u * (uint64_t)mid) < j)
            lo = mid + 1u;
        else
            hi = mid;
    }
    return lo;
}

// Whether entry `k` of the `count` offsets at `js` is `j`'s.
UKL_INLINE bool ukl_w_found(UKL_W_T, uint64_t js, uint32_t count, uint32_t k, uint32_t j)
{
    return k < count && ukl_w_ld16(t, js + 2u * (uint64_t)k) == j;
}

// `w_copy(to, from, len)`: `len` bytes moved down or apart, a word at a time, the last word's bytes past `len` kept.
UKL_COLD uint32_t clif_w_copy(UKL_W_T, uint64_t s, uint64_t to, uint64_t from, uint64_t len)
{
    uint64_t k = 0u;
    for (; k + 8u <= len; k += 8u)
        ukl_w_st64(t, to + k, ukl_w_ld64(t, from + k));
    if (len != k)
        ukl_w_cell_put(t, to + k, len - k, ukl_w_ld64(t, from + k));
    return 0u;
}

// `w_zero(to, len)`: `len` bytes (a multiple of 8) zeroed.
UKL_COLD uint32_t clif_w_zero(UKL_W_T, uint64_t s, uint64_t to, uint64_t len)
{
    for (uint64_t k = 0u; k < len; k += 8u)
        ukl_w_st64(t, to + k, 0u);
    return 0u;
}

// `w_fill(to, len, v)`: `len` bytes of `v`'s, a word at a time, the bytes past `len` kept.
UKL_COLD uint32_t clif_w_fill(UKL_W_T, uint64_t s, uint64_t to, uint64_t len, uint64_t v)
{
    uint64_t k = 0u;
    for (; k + 8u <= len; k += 8u)
        ukl_w_st64(t, to + k, v);
    if (len != k)
        ukl_w_cell_put(t, to + k, len - k, v);
    return 0u;
}

// `w_hash(src, n)`: a 64-bit hash of `n` bytes, a word at a time (the seen pieces').
UKL_INLINE uint64_t ukl_w_mix(uint64_t h, uint64_t x)
{
    h = (h ^ x) * 0x9e3779b97f4a7c15ul;
    return h ^ (h >> 29);
}
UKL_COLD uint64_t clif_w_hash(UKL_W_T, uint64_t s, uint64_t src, uint64_t n)
{
    uint64_t h = n, k = 0u;
    for (; k + 8u <= n; k += 8u)
        h = ukl_w_mix(h, ukl_w_ld64(t, src + k));
    if (n != k)
        h = ukl_w_mix(h, ukl_w_cell_get(t, src + k, n - k));
    return h;
}

// `w_eq(a, b, n)`: 1 where the `n` bytes at `a` and at `b` are the same.
UKL_COLD uint32_t clif_w_eq(UKL_W_T, uint64_t s, uint64_t a, uint64_t b, uint64_t n)
{
    uint64_t k = 0u;
    for (; k + 8u <= n; k += 8u)
        if (ukl_w_ld64(t, a + k) != ukl_w_ld64(t, b + k))
            return 0u;
    return ukl_w_cell_get(t, a + k, n - k) == ukl_w_cell_get(t, b + k, n - k) ? 1u : 0u;
}

// ---- A block's table entry, its room, its side area and lists ------------------------------------------------------

UKL_INLINE bool ukl_w_is_z(UKL_W_T, uint64_t s) { return (ukl_w_get32(t, s, CLIF_W_AT_FLAGS) & 2u) != 0u; }
UKL_INLINE uint64_t ukl_w_entry(UKL_W_T, uint64_t s, uint32_t b) { return ukl_w_part(t, s, CLIF_W_PART_TABLE) + (uint64_t)b * CLIF_W_TABLE; }
UKL_INLINE bool ukl_w_final(UKL_W_T, uint64_t s, uint32_t b) { return ukl_w_ld32(t, ukl_w_entry(t, s, b) + 8u) != 0u; }
// Where the blocks' rooms lie: in 16, the tape; in 16z, their own part.
UKL_INLINE uint64_t ukl_w_rooms(UKL_W_T, uint64_t s)
{
    return ukl_w_part(t, s, ukl_w_is_z(t, s) ? CLIF_W_PART_ROOMS : CLIF_W_PART_OUT);
}
UKL_INLINE uint64_t ukl_w_room_of(UKL_W_T, uint64_t s, uint32_t b) { return ukl_w_rooms(t, s) + ukl_w_ld32(t, ukl_w_entry(t, s, b)); }
UKL_INLINE uint64_t ukl_w_area_of(UKL_W_T, uint64_t s, uint32_t b)
{
    return ukl_w_part(t, s, CLIF_W_PART_SIDES) + (uint64_t)ukl_w_ld32(t, ukl_w_entry(t, s, b) + 4u) * CLIF_W_SIDE_AREA;
}

// List `q` of a side area: its count's address, its offsets', its values'.
typedef struct UklWList {
    uint64_t count_at, js, vs;
    uint32_t count;
} UklWList;
UKL_INLINE UklWList ukl_w_list(UKL_W_T, uint64_t area, uint32_t q)
{
    UklWList l;
    l.count_at = area + 4u * (uint64_t)q;
    l.js = area + (uint64_t)q * CLIF_W_SIDE_LIST + 16u;
    l.vs = l.js + 2u * CLIF_W_Z;
    l.count = ukl_w_ld32(t, l.count_at);
    return l;
}
// Entry `(j, v)` inserted at `k`, the ones from `k` moved up one.
UKL_INLINE void ukl_w_insert(UKL_W_T, UklWList l, uint32_t k, uint32_t j, uint64_t v)
{
    for (uint32_t m = l.count; m > k; m--) {
        ukl_w_st16(t, l.js + 2u * (uint64_t)m, ukl_w_ld16(t, l.js + 2u * (uint64_t)(m - 1u)));
        ukl_w_st64(t, l.vs + 8u * (uint64_t)m, ukl_w_ld64(t, l.vs + 8u * (uint64_t)(m - 1u)));
    }
    ukl_w_st16(t, l.js + 2u * (uint64_t)k, j);
    ukl_w_st64(t, l.vs + 8u * (uint64_t)k, v);
    ukl_w_st32(t, l.count_at, l.count + 1u);
}
// Entry `k` removed, the ones after it moved down one.
UKL_INLINE void ukl_w_remove(UKL_W_T, UklWList l, uint32_t k)
{
    uint32_t last = l.count - 1u;
    for (uint32_t m = k; m < last; m++) {
        ukl_w_st16(t, l.js + 2u * (uint64_t)m, ukl_w_ld16(t, l.js + 2u * (uint64_t)(m + 1u)));
        ukl_w_st64(t, l.vs + 8u * (uint64_t)m, ukl_w_ld64(t, l.vs + 8u * (uint64_t)(m + 1u)));
    }
    ukl_w_st32(t, l.count_at, last);
}

// Array `a`'s width (0 slots, 1 gaps, 2 lengths), column `r`'s offset in a room (0 codes .. 4 checkpoints).
UKL_INLINE uint64_t ukl_w_width(UKL_W_T, uint64_t s, uint32_t a) { return ukl_w_get32(t, s, CLIF_W_AT_W + 4u * a); }
UKL_INLINE uint64_t ukl_w_col(UKL_W_T, uint64_t s, uint32_t r) { return ukl_w_get32(t, s, CLIF_W_AT_AT + 4u * r); }
UKL_INLINE uint64_t ukl_w_esc(UKL_W_T, uint64_t s, uint32_t a) { return ukl_w_get64(t, s, CLIF_W_AT_ESC + 8u * a); }
// Node `i`'s cell in array `a`.
UKL_INLINE uint64_t ukl_w_cell(UKL_W_T, uint64_t s, uint32_t a, uint32_t i)
{
    return ukl_w_room_of(t, s, i >> 10) + ukl_w_col(t, s, a + 1u) + (uint64_t)(i & (CLIF_W_Z - 1u)) * ukl_w_width(t, s, a);
}

// ---- A node's cells ------------------------------------------------------------------------------------------------

// `w_put(a, i, v)`: value `v` of node `i`'s cell in array `a`, or its escape and its entry in side list `a + 1`.
UKL_COLD uint32_t clif_w_put(UKL_W_T, uint64_t s, uint32_t a, uint32_t i, uint64_t v)
{
    uint32_t j = i & (CLIF_W_Z - 1u);
    uint64_t esc = ukl_w_esc(t, s, a);
    UklWList l = ukl_w_list(t, ukl_w_area_of(t, s, i >> 10), a + 1u);
    bool escaped = v >= esc;
    if (escaped) {
        // Entries come in row order but for an ancestor's late close.
        uint32_t k = clif_w_find(t, s, l.js, l.count, j);
        if (ukl_w_found(t, l.js, l.count, k, j))
            ukl_w_st64(t, l.vs + 8u * (uint64_t)k, v);
        else
            ukl_w_insert(t, l, k, j, v);
    } else if (l.count != 0u && ukl_w_ld16(t, l.js + 2u * (uint64_t)(l.count - 1u)) >= j) {
        // A cell written again (`set_start`) drops the entry it had.
        uint32_t k = clif_w_find(t, s, l.js, l.count, j);
        if (ukl_w_found(t, l.js, l.count, k, j))
            ukl_w_remove(t, l, k);
    }
    ukl_w_cell_put(t, ukl_w_cell(t, s, a, i), ukl_w_width(t, s, a), escaped ? esc : v);
    return 0u;
}

// `w_value(a, i)`: node `i`'s value in array `a` as written: its cell, or its side-list entry under the escape.
UKL_COLD uint64_t clif_w_value(UKL_W_T, uint64_t s, uint32_t a, uint32_t i)
{
    uint64_t v = ukl_w_cell_get(t, ukl_w_cell(t, s, a, i), ukl_w_width(t, s, a));
    if (v < ukl_w_esc(t, s, a))
        return v;
    UklWList l = ukl_w_list(t, ukl_w_area_of(t, s, i >> 10), a + 1u);
    uint32_t k = clif_w_find(t, s, l.js, l.count, i & (CLIF_W_Z - 1u));
    return k < l.count ? ukl_w_ld64(t, l.vs + 8u * (uint64_t)k) : v;
}

// `w_put_code(i, number)`: node `i`'s code byte, and its codes' side-list entry for `number`.
UKL_COLD uint32_t clif_w_put_code(UKL_W_T, uint64_t s, uint32_t i, uint32_t number)
{
    uint32_t b = i >> 10, j = i & (CLIF_W_Z - 1u);
    uint32_t cut = ukl_w_get32(t, s, CLIF_W_AT_CUT);
    uint64_t code_at = ukl_w_room_of(t, s, b) + j;
    if (number < cut) {
        ukl_w_st8(t, code_at, number);
    } else {
        uint32_t per = ukl_w_get32(t, s, CLIF_W_AT_PER);
        uint32_t past = number - cut;
        ukl_w_st8(t, code_at, cut + past / per);
        UklWList l = ukl_w_list(t, ukl_w_area_of(t, s, b), 0u);
        // Entries come in row order but for a retag.
        ukl_w_insert(t, l, clif_w_find(t, s, l.js, l.count, j), j, past % per);
    }
    return 0u;
}

// Node `j` of block `b`'s checkpoint: `start` at the checkpoints' width.
UKL_INLINE void ukl_w_put_checkpoint(UKL_W_T, uint64_t s, uint32_t b, uint32_t j, uint64_t start)
{
    uint64_t cw = ukl_w_get32(t, s, CLIF_W_AT_CW);
    ukl_w_cell_put(t, ukl_w_room_of(t, s, b) + ukl_w_col(t, s, 4u) + (uint64_t)(j >> 3) * cw, cw, start);
}

// ---- A block begun, made final -------------------------------------------------------------------------------------

// The presence's bytes a block.
UKL_INLINE uint64_t ukl_w_presence_bytes(UKL_W_T, uint64_t s)
{
    return (uint64_t)(ukl_w_get32(t, s, CLIF_W_AT_KW) + ukl_w_get32(t, s, CLIF_W_AT_FW)) << 3;
}

// `w_new_block(b)`: room for block `b` (zeroed), a side area, its table entry, its presence where kept.
UKL_COLD uint32_t clif_w_new_block(UKL_W_T, uint64_t s, uint32_t b)
{
    uint64_t stride = ukl_w_get32(t, s, CLIF_W_AT_STRIDE);
    // The block's room: in 16 its place in the tape; in 16z one given back, else a new one.
    uint64_t room;
    if (ukl_w_is_z(t, s)) {
        uint64_t free = ukl_w_len(t, s, CLIF_W_PART_ROOM_FREE);
        if (free == 0u) {
            room = ukl_w_len(t, s, CLIF_W_PART_ROOMS);
            UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_ROOMS, room + stride));
            ukl_w_set_len(t, s, CLIF_W_PART_ROOMS, room + stride);
        } else {
            ukl_w_set_len(t, s, CLIF_W_PART_ROOM_FREE, free - 4u);
            room = ukl_w_ld32(t, ukl_w_part(t, s, CLIF_W_PART_ROOM_FREE) + free - 4u);
        }
    } else {
        room = (uint64_t)b * stride + CLIF_W_HEAD16;
        UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_OUT, room + stride));
        ukl_w_grow(t, s, CLIF_W_PART_OUT, room + stride);
    }
    uint64_t table = ((uint64_t)b + 1u) * CLIF_W_TABLE;
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_TABLE, table));
    // A side area given back, else a new one.
    uint32_t area;
    uint64_t free = ukl_w_len(t, s, CLIF_W_PART_FREE);
    if (free == 0u) {
        uint64_t made = ukl_w_len(t, s, CLIF_W_PART_SIDES);
        UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_SIDES, made + CLIF_W_SIDE_AREA));
        ukl_w_set_len(t, s, CLIF_W_PART_SIDES, made + CLIF_W_SIDE_AREA);
        area = (uint32_t)(made / CLIF_W_SIDE_AREA);
    } else {
        ukl_w_set_len(t, s, CLIF_W_PART_FREE, free - 4u);
        area = ukl_w_ld32(t, ukl_w_part(t, s, CLIF_W_PART_FREE) + free - 4u);
    }
    clif_w_zero(t, s, ukl_w_rooms(t, s) + room, stride);
    ukl_w_grow(t, s, CLIF_W_PART_TABLE, table);
    uint64_t e = ukl_w_entry(t, s, b);
    ukl_w_st32(t, e, (uint32_t)room);
    ukl_w_st32(t, e + 4u, area);
    ukl_w_st32(t, e + 8u, 0u);
    uint64_t counts = ukl_w_area_of(t, s, b);
    ukl_w_st64(t, counts, 0u);
    ukl_w_st64(t, counts + 8u, 0u);
    ukl_w_set32(t, s, CLIF_W_AT_BLOCKS, b + 1u);
    if ((ukl_w_get32(t, s, CLIF_W_AT_FLAGS) & 1u) != 0u) {
        uint64_t bytes = ukl_w_presence_bytes(t, s);
        uint64_t need = ((uint64_t)b + 2u) * bytes;
        UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_PRESENCE, need));
        ukl_w_grow(t, s, CLIF_W_PART_PRESENCE, need);
        clif_w_zero(t, s, ukl_w_part(t, s, CLIF_W_PART_PRESENCE) + ((uint64_t)b + 1u) * bytes, bytes);
    }
    return 0u;
}

// `w_lay(b, q, rows)`: block `b`'s side list `q` as stored, appended to `laid`: a u16 count a sub-block, each
// entry's offset in its sub-block, then the values at `vw[q]` bytes.
UKL_COLD uint32_t clif_w_lay(UKL_W_T, uint64_t s, uint32_t b, uint32_t q, uint32_t rows)
{
    UklWList l = ukl_w_list(t, ukl_w_area_of(t, s, b), q);
    uint64_t laid = ukl_w_part(t, s, CLIF_W_PART_LAID);
    uint64_t start = ukl_w_len(t, s, CLIF_W_PART_LAID);
    uint64_t vw = ukl_w_get32(t, s, CLIF_W_AT_VW + 4u * q);
    uint32_t subs = (rows + CLIF_W_BLOCK - 1u) >> 8;
    uint64_t ends = (uint64_t)subs << 1;
    uint64_t len = l.count != 0u ? ends + l.count + (uint64_t)l.count * vw : 0u;
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_LAID, start + len));
    if (l.count != 0u) {
        uint64_t at = laid + start;
        // Each sub-block's entries' end.
        uint32_t k = 0u;
        for (uint32_t sub = 0u; sub < subs; sub++) {
            while (k < l.count && ukl_w_ld16(t, l.js + 2u * (uint64_t)k) >> 8 == sub)
                k++;
            ukl_w_st16(t, at + 2u * (uint64_t)sub, k);
        }
        uint64_t offsets = at + ends;
        for (uint32_t m = 0u; m < l.count; m++)
            ukl_w_st8(t, offsets + m, ukl_w_ld16(t, l.js + 2u * (uint64_t)m));
        uint64_t values = offsets + l.count;
        for (uint32_t m = 0u; m < l.count; m++)
            ukl_w_cell_put(t, values + (uint64_t)m * vw, vw, ukl_w_ld64(t, l.vs + 8u * (uint64_t)m));
    }
    ukl_w_set_len(t, s, CLIF_W_PART_LAID, start + len);
    uint64_t need = ((uint64_t)b + 1u) * CLIF_W_LAID_AT;
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_LAID_AT, need));
    ukl_w_grow(t, s, CLIF_W_PART_LAID_AT, need);
    uint64_t e = ukl_w_part(t, s, CLIF_W_PART_LAID_AT) + (uint64_t)b * CLIF_W_LAID_AT + 8u * (uint64_t)q;
    ukl_w_st32(t, e, (uint32_t)start);
    ukl_w_st32(t, e + 4u, (uint32_t)len);
    return 0u;
}

// ---- LZ4 HC, level 4 (decision 0434 §6) ----------------------------------------------------------------------------
//
// liblz4 1.10.0's `LZ4HC_compress_hashChain` with 8 attempts, no dictionary, no pattern analysis and no chain swap,
// as `LZ4_compress_HC(.., 4)` runs it: the same sequences, so the same bytes. Positions are signed offsets from the
// piece's first byte; an index is a position plus the piece's base (`zbase`).

#define UKL_W_MINMATCH 4
#define UKL_W_MFLIMIT 12
#define UKL_W_LASTLITERALS 5
#define UKL_W_OPTIMAL_ML 18
#define UKL_W_DISTANCE_MAX 65535
#define UKL_W_CHAIN_AT (4u << CLIF_W_LZ_HASH_LOG)

// LZ4 HC's hash of a 4-byte word: its hash table's slot, as a byte offset.
UKL_INLINE uint64_t ukl_w_lz_hash(uint32_t x) { return (uint64_t)((x * 2654435761u) >> (32u - CLIF_W_LZ_HASH_LOG)) << 2; }

// `LZ4HC_countBack`: minus the bytes before `ip` and `mp` that are the same, down to `low` and to the piece's first
// byte; a word at a time, the last word's top bytes by its leading zeros.
UKL_INLINE int64_t ukl_w_lz_count_back(UKL_W_T, uint64_t src, int64_t ip, int64_t mp, int64_t low)
{
    int64_t least = low - ip > -mp ? low - ip : -mp;
    int64_t back = 0;
    while (back - least > 3) {
        uint32_t v = ukl_w_ld32(t, src + (uint64_t)(ip + back - 4)) ^ ukl_w_ld32(t, src + (uint64_t)(mp + back - 4));
        if (v != 0u)
            return back - (int64_t)(ukl_clz32(v) >> 3);
        back -= 4;
    }
    // The remainder: fewer than 4 bytes, the top ones of the word that ends at `back`.
    int64_t rest = back - least;
    if (rest > 0) {
        uint32_t v = ukl_w_ld32(t, src + (uint64_t)(ip + back - 4)) ^ ukl_w_ld32(t, src + (uint64_t)(mp + back - 4));
        uint64_t common = ukl_clz32(v) >> 3;
        back -= (int64_t)(common < (uint64_t)rest ? common : (uint64_t)rest);
    }
    return back;
}

// `LZ4_count`: the bytes from `p` and `m` that are the same, up to `limit`, a word at a time.
UKL_INLINE int64_t ukl_w_lz_count(UKL_W_T, uint64_t src, int64_t p, int64_t m, int64_t limit)
{
    int64_t k = 0;
    for (; p + k + 8 <= limit; k += 8) {
        uint64_t v = ukl_w_ld64(t, src + (uint64_t)(p + k)) ^ ukl_w_ld64(t, src + (uint64_t)(m + k));
        if (v != 0u)
            return k + (int64_t)(ukl_ctz64(v) >> 3);
    }
    uint64_t rest = limit - (p + k) > 0 ? (uint64_t)(limit - (p + k)) : 0u;
    uint64_t v = ukl_w_cell_get(t, src + (uint64_t)(p + k), rest) ^ ukl_w_cell_get(t, src + (uint64_t)(m + k), rest);
    uint64_t same = ukl_ctz64(v) >> 3;
    return k + (int64_t)(same < rest ? same : rest);
}

// `w_lz4_find(src, ip, low, high, longest)`: `LZ4HC_InsertAndGetWiderMatch` at level 4 for the piece at `src`: the
// positions before `ip` inserted, then up to 8 entries of `ip`'s chain tried for a match longer than `longest`,
// reaching back to `low` and on to `high`. Returns its length (`longest` if none), its offset and back count in the
// state.
UKL_COLD uint64_t clif_w_lz4_find(UKL_W_T, uint64_t s, uint64_t src, uint64_t ip_, uint64_t low_, uint64_t high_, uint64_t longest_)
{
    int64_t ip = (int64_t)ip_, low = (int64_t)low_, high = (int64_t)high_, longest = (int64_t)longest_;
    uint64_t lz = ukl_w_part(t, s, CLIF_W_PART_LZ);
    uint64_t chain = lz + UKL_W_CHAIN_AT;
    uint64_t base = ukl_w_get32(t, s, CLIF_W_AT_ZBASE);
    uint64_t ip_index = base + (uint64_t)ip;
    // The positions before `ip` in the hash table and the chain (`LZ4HC_Insert`).
    for (uint64_t index = ukl_w_get32(t, s, CLIF_W_AT_ZNEXT); index < ip_index; index++) {
        uint64_t slot = lz + ukl_w_lz_hash(ukl_w_ld32(t, src + (index - base)));
        uint64_t delta = index - ukl_w_ld32(t, slot);
        ukl_w_st16(t, chain + ((index & 0xffffu) << 1), delta < UKL_W_DISTANCE_MAX ? delta : UKL_W_DISTANCE_MAX);
        ukl_w_st32(t, slot, (uint32_t)index);
    }
    ukl_w_set32(t, s, CLIF_W_AT_ZNEXT, (uint32_t)ip_index);
    uint64_t lowest = base + UKL_W_DISTANCE_MAX + 1u > ip_index ? base : ip_index - UKL_W_DISTANCE_MAX;
    int64_t look_back = ip - low;
    uint32_t pattern = ukl_w_ld32(t, src + (uint64_t)ip);
    uint64_t match = ukl_w_ld32(t, lz + ukl_w_lz_hash(pattern));
    int64_t offset = 0, best_back = 0;
    for (int64_t tries = CLIF_W_LZ_ATTEMPTS; match >= lowest && tries > 0; tries--) {
        int64_t mp = (int64_t)(match - base);
        if (ukl_w_ld16(t, src + (uint64_t)(low + longest - 1)) == ukl_w_ld16(t, src + (uint64_t)(mp - look_back + longest - 1))
            && ukl_w_ld32(t, src + (uint64_t)mp) == pattern) {
            int64_t back = ukl_w_lz_count_back(t, src, ip, mp, low);
            int64_t length = ukl_w_lz_count(t, src, ip + UKL_W_MINMATCH, mp + UKL_W_MINMATCH, high) + UKL_W_MINMATCH - back;
            if (length > longest) {
                longest = length;
                offset = (int64_t)(ip_index - match);
                best_back = back;
            }
        }
        match -= ukl_w_ld16(t, chain + ((match & 0xffffu) << 1));
    }
    ukl_w_set32(t, s, CLIF_W_AT_ZOFF, (uint32_t)offset);
    ukl_w_set32(t, s, CLIF_W_AT_ZBACK, (uint32_t)best_back);
    return (uint64_t)longest;
}

// A match: where it starts, its length, its offset.
typedef struct UklWMatch {
    int64_t start, len, off;
} UklWMatch;

// The widest match at `at` (`w_lz4_find`), its start moved back by its back count.
UKL_INLINE UklWMatch ukl_w_lz_find(UKL_W_T, uint64_t s, uint64_t src, int64_t at, int64_t low, int64_t high, int64_t longest)
{
    UklWMatch m;
    m.len = (int64_t)clif_w_lz4_find(t, s, src, (uint64_t)at, (uint64_t)low, (uint64_t)high, (uint64_t)longest);
    m.off = (int64_t)ukl_w_get32(t, s, CLIF_W_AT_ZOFF);
    m.start = at + (int64_t)(int32_t)ukl_w_get32(t, s, CLIF_W_AT_ZBACK);
    return m;
}

// LZ4's length past 15 at `*op`: `len / 255` bytes of 255, then `len % 255`.
UKL_INLINE void ukl_w_lz_length(UKL_W_T, uint64_t s, UKL_PRIVATE uint64_t* op, int64_t len)
{
    uint64_t k = (uint64_t)len / 255u;
    clif_w_fill(t, s, *op, k, ~(uint64_t)0);
    ukl_w_st8(t, *op + k, (uint64_t)len % 255u);
    *op += k + 1u;
}

// `LZ4HC_encodeSequence`: the literals from `*anchor` to `*ip`, then a match of `ml` at `off`; `*ip` past the match,
// `*anchor` at it.
UKL_INLINE void ukl_w_lz_encode(UKL_W_T, uint64_t s, uint64_t src, UKL_PRIVATE int64_t* ip, UKL_PRIVATE int64_t* anchor, UKL_PRIVATE uint64_t* op, int64_t ml, int64_t off)
{
    int64_t len = *ip - *anchor;
    uint64_t token = (*op)++;
    int64_t tok = len;
    if (len >= 15) {
        ukl_w_lz_length(t, s, op, len - 15);
        tok = 15;
    }
    tok <<= 4;
    clif_w_copy(t, s, *op, src + (uint64_t)*anchor, (uint64_t)len);
    *op += (uint64_t)len;
    ukl_w_st16(t, *op, (uint64_t)off);
    *op += 2u;
    int64_t l = ml - UKL_W_MINMATCH;
    if (l >= 15) {
        ukl_w_lz_length(t, s, op, l - 15);
        tok += 15;
    } else {
        tok += l;
    }
    ukl_w_st8(t, token, (uint64_t)tok);
    *ip += ml;
    *anchor = *ip;
}

// Where `LZ4HC_compress_hashChain`'s loop is: its main loop, `_Search2`, `_Search3`.
#define UKL_W_MAIN 0u
#define UKL_W_SEARCH2 1u
#define UKL_W_SEARCH3 2u

// `w_lz4(src, n, dst)`: the `n` bytes at `src` as one LZ4 block at `dst` (`LZ4_compress_HC(.., 4)`'s bytes); returns
// its length. `dst` has room for `lz4_bound(n)` bytes and 8 past a cell; `src`'s 3 bytes before it and 8 after are
// readable.
UKL_COLD uint64_t clif_w_lz4(UKL_W_T, uint64_t s, uint64_t src, uint64_t n_, uint64_t dst)
{
    int64_t n = (int64_t)n_;
    // `LZ4HC_init_internal`, the context reused: this piece's indices start 64 KB past the last one's end.
    uint64_t start = (uint64_t)ukl_w_get32(t, s, CLIF_W_AT_ZBASE) + ukl_w_get32(t, s, CLIF_W_AT_ZPREV);
    if (start > (1u << 30)) {
        clif_w_zero(t, s, ukl_w_part(t, s, CLIF_W_PART_LZ), 4u << CLIF_W_LZ_HASH_LOG);
        start = 0u;
    }
    ukl_w_set32(t, s, CLIF_W_AT_ZBASE, (uint32_t)(start + 65536u));
    ukl_w_set32(t, s, CLIF_W_AT_ZNEXT, (uint32_t)(start + 65536u));
    ukl_w_set32(t, s, CLIF_W_AT_ZPREV, (uint32_t)n);
    int64_t ip = 0, anchor = 0;
    uint64_t op = dst;
    int64_t mflimit = n - UKL_W_MFLIMIT, matchlimit = n - UKL_W_LASTLITERALS;
    // m0: the match a second search may give back to; m1: the one at `ip`; m2, m3: the next ones found.
    UklWMatch m0 = {0, 0, 0}, m1 = {0, 0, 0}, m2 = {0, 0, 0}, m3 = {0, 0, 0};
    uint32_t at = UKL_W_MAIN;
    while (n >= UKL_W_MFLIMIT + 1) {
        if (at == UKL_W_MAIN) {
            if (ip > mflimit)
                break;
            UklWMatch m = ukl_w_lz_find(t, s, src, ip, ip, matchlimit, UKL_W_MINMATCH - 1);
            if (m.len < UKL_W_MINMATCH) {
                ip++;
                continue;
            }
            m0 = m1 = m;
            m0.start = ip;
            at = UKL_W_SEARCH2;
        } else if (at == UKL_W_SEARCH2) {
            if (ip + m1.len <= mflimit) {
                m2 = ukl_w_lz_find(t, s, src, ip + m1.len - 2, ip, matchlimit, m1.len);
            } else {
                m2.len = 0;
                m2.off = 0;
            }
            if (m2.len <= m1.len) {
                ukl_w_lz_encode(t, s, src, &ip, &anchor, &op, m1.len, m1.off);
                at = UKL_W_MAIN;
                continue;
            }
            if (m0.start < ip && m2.start < ip + m0.len) {
                ip = m0.start;
                m1.len = m0.len;
                m1.off = m0.off;
            }
            if (m2.start - ip < 3) {
                // The first dropped: the second becomes it.
                ip = m2.start;
                m1.len = m2.len;
                m1.off = m2.off;
                continue;
            }
            at = UKL_W_SEARCH3;
        } else {
            int64_t gap = m2.start - ip;
            if (gap < UKL_W_OPTIMAL_ML) {
                int64_t ml = m1.len < UKL_W_OPTIMAL_ML ? m1.len : UKL_W_OPTIMAL_ML;
                if (ip + ml > m2.start + m2.len - UKL_W_MINMATCH)
                    ml = gap + m2.len - UKL_W_MINMATCH;
                int64_t correction = ml - gap;
                if (correction > 0) {
                    m2.start += correction;
                    m2.len -= correction;
                }
            }
            if (m2.start + m2.len <= mflimit) {
                m3 = ukl_w_lz_find(t, s, src, m2.start + m2.len - 3, m2.start, matchlimit, m2.len);
            } else {
                m3.len = 0;
                m3.off = 0;
            }
            if (m3.len <= m2.len) {
                // No better third: the first two encoded.
                if (m2.start < ip + m1.len)
                    m1.len = m2.start - ip;
                ukl_w_lz_encode(t, s, src, &ip, &anchor, &op, m1.len, m1.off);
                ip = m2.start;
                ukl_w_lz_encode(t, s, src, &ip, &anchor, &op, m2.len, m2.off);
                at = UKL_W_MAIN;
                continue;
            }
            int64_t end1 = ip + m1.len;
            if (m3.start < end1 + 3) {
                if (m3.start >= end1) {
                    // The first written now; the second dropped, the third becomes the first.
                    if (m2.start < end1) {
                        int64_t correction = end1 - m2.start;
                        m2.start += correction;
                        m2.len -= correction;
                        if (m2.len < UKL_W_MINMATCH)
                            m2 = m3;
                    }
                    ukl_w_lz_encode(t, s, src, &ip, &anchor, &op, m1.len, m1.off);
                    ip = m3.start;
                    m1 = m3;
                    m0 = m2;
                    at = UKL_W_SEARCH2;
                } else {
                    m2 = m3;
                }
                continue;
            }
            // Three ascending matches: the first written, the second becomes the first, the third the second.
            if (m2.start < ip + m1.len) {
                int64_t gap = m2.start - ip;
                if (gap < UKL_W_OPTIMAL_ML) {
                    if (m1.len > UKL_W_OPTIMAL_ML)
                        m1.len = UKL_W_OPTIMAL_ML;
                    if (ip + m1.len > m2.start + m2.len - UKL_W_MINMATCH)
                        m1.len = gap + m2.len - UKL_W_MINMATCH;
                    int64_t correction = m1.len - gap;
                    if (correction > 0) {
                        m2.start += correction;
                        m2.len -= correction;
                    }
                } else {
                    m1.len = gap;
                }
            }
            ukl_w_lz_encode(t, s, src, &ip, &anchor, &op, m1.len, m1.off);
            ip = m2.start;
            m1 = m2;
            m2 = m3;
        }
    }
    // `_last_literals`.
    int64_t run = n - anchor;
    uint64_t token = op++;
    if (run >= 15) {
        ukl_w_st8(t, token, 0xf0u);
        ukl_w_lz_length(t, s, &op, run - 15);
    } else {
        ukl_w_st8(t, token, (uint64_t)run << 4);
    }
    clif_w_copy(t, s, op, src + (uint64_t)anchor, (uint64_t)run);
    return op + (uint64_t)run - dst;
}

// ---- 16z: a block's pieces -----------------------------------------------------------------------------------------

// `w_zpiece(j, pos, raw)`: piece `j`, its `raw` bytes at the payload's end (`pos` in the tape), stored as 16z stores
// it: a repeat of an earlier piece of the same bytes (found by their hash, then compared: a raw piece's bytes with
// these, an LZ4 one's with these compressed), else one LZ4 block where that saves a tenth, else as they are. Its
// entry in the piece index; the payload past it.
UKL_COLD uint32_t clif_w_zpiece(UKL_W_T, uint64_t s, uint32_t j, uint64_t pos, uint64_t raw)
{
    uint64_t out = ukl_w_part(t, s, CLIF_W_PART_OUT);
    uint64_t src = out + pos, dst = src + raw;
    uint64_t pieces = ukl_w_part(t, s, CLIF_W_PART_PIECES);
    uint64_t entry = pieces + (uint64_t)j * CLIF_W_PIECE;
    // The piece compressed, once, where it is asked for (-1 until then).
    int64_t compressed = -1;
    uint64_t hash = 0u;
    uint32_t next = CLIF_W_NONE, kind = CLIF_W_RAW;
    uint64_t len = raw;
    if (raw >= CLIF_W_REPEAT_MIN) {
        hash = clif_w_hash(t, s, src, raw);
        uint32_t bucket = (uint32_t)hash & (ukl_w_get32(t, s, CLIF_W_AT_ZBUCKETS) - 1u);
        uint64_t slot = ukl_w_part(t, s, CLIF_W_PART_SEEN) + 4u * (uint64_t)bucket;
        uint32_t head = ukl_w_ld32(t, slot);
        for (uint32_t e = head; e != CLIF_W_NONE;) {
            uint64_t seen = pieces + (uint64_t)e * CLIF_W_PIECE;
            if (ukl_w_ld64(t, seen + 24u) == hash && ukl_w_ld32(t, seen + 8u) == (uint32_t)raw) {
                uint64_t stored = out + ukl_w_ld32(t, seen) + CLIF_W_HEAD16;
                uint32_t same = 0u;
                if (ukl_w_ld32(t, seen + 12u) == CLIF_W_RAW) {
                    same = clif_w_eq(t, s, stored, src, raw);
                } else {
                    if (compressed < 0)
                        compressed = (int64_t)clif_w_lz4(t, s, src, raw, dst);
                    if ((uint64_t)compressed == ukl_w_ld32(t, seen + 4u))
                        same = clif_w_eq(t, s, stored, dst, (uint64_t)compressed);
                }
                if (same != 0u) {
                    ukl_w_st32(t, src, e);
                    kind = CLIF_W_REPEAT;
                    len = 4u;
                    break;
                }
            }
            e = ukl_w_ld32(t, seen + 16u);
        }
        if (kind != CLIF_W_REPEAT) {
            // The first piece of these bytes: into its bucket.
            next = head;
            ukl_w_st32(t, slot, j);
        }
    }
    if (kind != CLIF_W_REPEAT && raw >= CLIF_W_LZ4_MIN) {
        if (compressed < 0)
            compressed = (int64_t)clif_w_lz4(t, s, src, raw, dst);
        if ((uint64_t)compressed * 10u <= raw * 9u) {
            clif_w_copy(t, s, src, dst, (uint64_t)compressed);
            kind = CLIF_W_LZ4;
            len = (uint64_t)compressed;
        }
    }
    ukl_w_st32(t, entry, (uint32_t)(pos - CLIF_W_HEAD16));
    ukl_w_st32(t, entry + 4u, (uint32_t)len);
    ukl_w_st32(t, entry + 8u, (uint32_t)raw);
    ukl_w_st32(t, entry + 12u, kind);
    ukl_w_st32(t, entry + 16u, next);
    ukl_w_st64(t, entry + 24u, hash);
    ukl_w_set_len(t, s, CLIF_W_PART_OUT, pos + len);
    return 0u;
}

// `w_zblock(b, rows)`: 16z's block `b`, final: its four pieces (each column at its rows, then its side list's part
// as laid) stored in turn at the payload's end (`w_zpiece`), its checkpoints kept, its room given back.
UKL_COLD uint32_t clif_w_zblock(UKL_W_T, uint64_t s, uint32_t b, uint32_t rows)
{
    uint64_t room = ukl_w_room_of(t, s, b);
    uint64_t laid = ukl_w_part(t, s, CLIF_W_PART_LAID);
    uint64_t laid_at = ukl_w_part(t, s, CLIF_W_PART_LAID_AT) + (uint64_t)b * CLIF_W_LAID_AT;
    for (uint32_t q = 0u; q < 4u; q++) {
        // A column's cell: a byte (codes), else its array's width.
        uint64_t column = (uint64_t)rows * (q == 0u ? 1u : ukl_w_width(t, s, q - 1u));
        uint64_t part_at = ukl_w_ld32(t, laid_at + 8u * q), part_len = ukl_w_ld32(t, laid_at + 8u * q + 4u);
        uint64_t raw = column + part_len;
        uint64_t pos = ukl_w_len(t, s, CLIF_W_PART_OUT);
        UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_OUT, pos + raw + raw + raw / 255u + 16u));
        uint32_t j = (b << 2) + q;
        uint64_t need = ((uint64_t)j + 1u) * CLIF_W_PIECE;
        UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_PIECES, need));
        ukl_w_grow(t, s, CLIF_W_PART_PIECES, need);
        uint64_t src = ukl_w_part(t, s, CLIF_W_PART_OUT) + pos;
        clif_w_copy(t, s, src, room + ukl_w_col(t, s, q), column);
        clif_w_copy(t, s, src + column, laid + part_at, part_len);
        UKL_W_TRY(clif_w_zpiece(t, s, j, pos, raw));
    }
    // Its checkpoints, at their place among the blocks'.
    uint64_t cw = ukl_w_get32(t, s, CLIF_W_AT_CW);
    uint64_t at = (uint64_t)b * (CLIF_W_Z / CLIF_W_K) * cw;
    uint64_t len = (((uint64_t)rows + CLIF_W_K - 1u) >> 3) * cw;
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_CKPT, at + len));
    clif_w_copy(t, s, ukl_w_part(t, s, CLIF_W_PART_CKPT) + at, room + ukl_w_col(t, s, 4u), len);
    ukl_w_grow(t, s, CLIF_W_PART_CKPT, at + len);
    // The laid parts are in its pieces; its room serves the next block.
    ukl_w_set_len(t, s, CLIF_W_PART_LAID, 0u);
    uint64_t free = ukl_w_len(t, s, CLIF_W_PART_ROOM_FREE);
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_ROOM_FREE, free + 4u));
    ukl_w_st32(t, ukl_w_part(t, s, CLIF_W_PART_ROOM_FREE) + free, ukl_w_ld32(t, ukl_w_entry(t, s, b)));
    ukl_w_set_len(t, s, CLIF_W_PART_ROOM_FREE, free + 4u);
    return 0u;
}

// ---- A block final, the nodes so far committed ---------------------------------------------------------------------

// Bit `bit` of the words from word `word0` at `base`, set.
UKL_INLINE void ukl_w_set_bit(UKL_W_T, uint64_t base, uint64_t word0, uint32_t bit)
{
    uint64_t at = base + (((uint64_t)(bit >> 6) + word0) << 3);
    ukl_w_st64(t, at, ukl_w_ld64(t, at) | ((uint64_t)1 << (bit & 63u)));
}

// `w_finalize(b, rows)`: block `b`, of `rows` rows, final: its presence taken, its side lists' parts laid, its side
// area given back; in 16z its pieces stored.
UKL_COLD uint32_t clif_w_finalize(UKL_W_T, uint64_t s, uint32_t b, uint32_t rows)
{
    uint64_t e = ukl_w_entry(t, s, b);
    ukl_w_st32(t, e + 8u, 1u);
    uint64_t room = ukl_w_room_of(t, s, b);
    UklWList codes = ukl_w_list(t, ukl_w_area_of(t, s, b), 0u);
    uint32_t cut = ukl_w_get32(t, s, CLIF_W_AT_CUT), per = ukl_w_get32(t, s, CLIF_W_AT_PER);
    uint64_t kinds = ukl_w_part(t, s, CLIF_W_PART_KINDS);
    uint64_t numbers = ukl_w_len(t, s, CLIF_W_PART_KINDS) >> 2;
    uint64_t presence = ukl_w_part(t, s, CLIF_W_PART_PRESENCE);
    uint64_t kw = ukl_w_get32(t, s, CLIF_W_AT_KW);
    bool keep = (ukl_w_get32(t, s, CLIF_W_AT_FLAGS) & 1u) != 0u;
    uint64_t mine = presence + ((uint64_t)b + 1u) * ukl_w_presence_bytes(t, s);
    // Each node's number: its code byte, and past the cutoff its codes' side-list entry, which come in row order.
    uint32_t low = 0u;
    for (uint32_t j = 0u; j < rows; j++) {
        uint32_t byte = ukl_w_ld8(t, room + j), number = byte;
        if (byte >= cut) {
            uint32_t rest = 0u;
            if (low < codes.count) {
                if (ukl_w_ld16(t, codes.js + 2u * (uint64_t)low) == j)
                    rest = (uint32_t)ukl_w_ld64(t, codes.vs + 8u * (uint64_t)low);
                low++;
            }
            number = cut + (byte - cut) * per + rest;
        }
        uint32_t kind_field = (uint64_t)number < numbers ? ukl_w_ld32(t, kinds + 4u * (uint64_t)number) : 0u;
        uint32_t kind = kind_field & 0xffffu, field = kind_field >> 16;
        ukl_w_set_bit(t, presence, 0u, kind);
        ukl_w_set_bit(t, presence, kw, field);
        if (keep) {
            ukl_w_set_bit(t, mine, 0u, kind);
            ukl_w_set_bit(t, mine, kw, field);
        }
    }
    for (uint32_t q = 0u; q < 4u; q++)
        UKL_W_TRY(clif_w_lay(t, s, b, q, rows));
    // Its side area, given back.
    uint64_t free = ukl_w_len(t, s, CLIF_W_PART_FREE);
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_FREE, free + 4u));
    ukl_w_st32(t, ukl_w_part(t, s, CLIF_W_PART_FREE) + free, ukl_w_ld32(t, e + 4u));
    ukl_w_set_len(t, s, CLIF_W_PART_FREE, free + 4u);
    ukl_w_st32(t, e + 4u, CLIF_W_NONE);
    if (ukl_w_is_z(t, s))
        UKL_W_TRY(clif_w_zblock(t, s, b, rows));
    return 0u;
}

// `w_settle(b)`: block `b` made final, if it is: every node in it closed, the node after its last opened, the
// frontier past it.
UKL_COLD uint32_t clif_w_settle(UKL_W_T, uint64_t s, uint32_t b)
{
    uint32_t end = (b + 1u) * CLIF_W_Z;
    if (ukl_w_final(t, s, b) || ukl_w_get32(t, s, CLIF_W_AT_N) <= end || ukl_w_get32(t, s, CLIF_W_AT_FRONTIER) < end)
        return 0u;
    // The open nodes' indices ascend: the last below the block's end is the one that may be in it.
    uint64_t opens = ukl_w_part(t, s, CLIF_W_PART_OPEN);
    uint32_t lo = 0u, hi = ukl_w_get32(t, s, CLIF_W_AT_OPEN_LEN);
    while (lo < hi) {
        uint32_t mid = (lo + hi) >> 1;
        if (ukl_w_ld32(t, opens + (uint64_t)mid * CLIF_W_OPEN) < end)
            lo = mid + 1u;
        else
            hi = mid;
    }
    if (lo != 0u && ukl_w_ld32(t, opens + (uint64_t)(lo - 1u) * CLIF_W_OPEN) >= b * CLIF_W_Z)
        return 0u;
    return clif_w_finalize(t, s, b, CLIF_W_Z);
}

// ---- The public ops ------------------------------------------------------------------------------------------------

// `w_open(number, start)`: a node at the end, in preorder, open.
UKL_COLD uint32_t clif_w_open(UKL_W_T, uint64_t s, uint32_t number, uint64_t start)
{
    uint32_t i = ukl_w_get32(t, s, CLIF_W_AT_N);
    uint64_t prev = ukl_w_get64(t, s, CLIF_W_AT_PREV_START);
    if (start < prev || start > ukl_w_get64(t, s, CLIF_W_AT_SOURCE_LEN) || i + 1u >= CLIF_W_MAX_NODES)
        return clif_w_fail(t, s, CLIF_W_BOUNDS, i);
    uint32_t open_len = ukl_w_get32(t, s, CLIF_W_AT_OPEN_LEN);
    if (i != 0u && open_len == 0u)
        return clif_w_fail(t, s, CLIF_W_OUTSIDE, i);
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_OPEN, ((uint64_t)open_len + 1u) * CLIF_W_OPEN));
    uint32_t b = i >> 10, j = i & (CLIF_W_Z - 1u);
    if (j == 0u)
        UKL_W_TRY(clif_w_new_block(t, s, b));
    // The node before is a leaf unless it is the parent: its slot describes this node.
    if (open_len != 0u) {
        uint64_t top = ukl_w_part(t, s, CLIF_W_PART_OPEN) + (uint64_t)(open_len - 1u) * CLIF_W_OPEN;
        uint32_t parent = ukl_w_ld32(t, top), children = ukl_w_ld32(t, top + 4u), last = ukl_w_ld32(t, top + 8u);
        ukl_w_st32(t, top + 4u, children + 1u);
        ukl_w_st32(t, top + 8u, i);
        if (i - 1u != parent) {
            // At a checkpoint the parent's distance, else the last sibling's.
            uint64_t v = (children & (CLIF_W_K - 1u)) == 0u ? (uint64_t)(i - parent) << 2 | 3u : (uint64_t)(i - last) << 2 | 1u;
            UKL_W_TRY(clif_w_put(t, s, 0u, i - 1u, v));
        }
    }
    UKL_W_TRY(clif_w_put_code(t, s, i, number));
    UKL_W_TRY(clif_w_put(t, s, 1u, i, i == 0u ? 0u : start - prev));
    if ((j & (CLIF_W_K - 1u)) == 0u)
        ukl_w_put_checkpoint(t, s, b, j, start);
    ukl_w_set64(t, s, CLIF_W_AT_PREV_START, start);
    ukl_w_set32(t, s, CLIF_W_AT_N, i + 1u);
    uint64_t entry = ukl_w_part(t, s, CLIF_W_PART_OPEN) + (uint64_t)open_len * CLIF_W_OPEN;
    ukl_w_st32(t, entry, i);
    ukl_w_st32(t, entry + 4u, 0u);
    ukl_w_st32(t, entry + 8u, CLIF_W_NONE);
    ukl_w_st64(t, entry + 16u, start);
    ukl_w_set32(t, s, CLIF_W_AT_OPEN_LEN, open_len + 1u);
    // The block before has its last slot now.
    if (j == 0u && b != 0u)
        UKL_W_TRY(clif_w_settle(t, s, b - 1u));
    return 0u;
}

// `w_close(end)`: the innermost open node's end: its length, and an inner node's slot.
UKL_COLD uint32_t clif_w_close(UKL_W_T, uint64_t s, uint64_t end)
{
    uint32_t open_len = ukl_w_get32(t, s, CLIF_W_AT_OPEN_LEN);
    uint32_t n = ukl_w_get32(t, s, CLIF_W_AT_N);
    if (open_len == 0u)
        return clif_w_fail(t, s, CLIF_W_NO_OPEN, n);
    ukl_w_set32(t, s, CLIF_W_AT_OPEN_LEN, open_len - 1u);
    uint64_t top = ukl_w_part(t, s, CLIF_W_PART_OPEN) + (uint64_t)(open_len - 1u) * CLIF_W_OPEN;
    uint32_t i = ukl_w_ld32(t, top);
    uint64_t start = ukl_w_ld64(t, top + 16u);
    if (end < start || end > ukl_w_get64(t, s, CLIF_W_AT_SOURCE_LEN))
        return clif_w_fail(t, s, CLIF_W_BOUNDS, i);
    UKL_W_TRY(clif_w_put(t, s, 2u, i, end - start));
    if (n - i > 1u)
        UKL_W_TRY(clif_w_put(t, s, 0u, i, (uint64_t)(n - i) << 1));
    return clif_w_settle(t, s, i >> 10);
}

// `w_leaf(number, start, end)`: a closed node with no children.
UKL_COLD uint32_t clif_w_leaf(UKL_W_T, uint64_t s, uint32_t number, uint64_t start, uint64_t end)
{
    UKL_W_TRY(clif_w_open(t, s, number, start));
    return clif_w_close(t, s, end);
}

// `w_commit()`: the nodes so far final but for the open ones.
UKL_COLD uint32_t clif_w_commit(UKL_W_T, uint64_t s)
{
    uint32_t was = ukl_w_get32(t, s, CLIF_W_AT_FRONTIER) >> 10;
    uint32_t n = ukl_w_get32(t, s, CLIF_W_AT_N);
    ukl_w_set32(t, s, CLIF_W_AT_FRONTIER, n);
    for (uint32_t b = was; b < n >> 10; b++)
        UKL_W_TRY(clif_w_settle(t, s, b));
    return 0u;
}

// 0, or a failure unless node `i` is written and its block not final.
UKL_INLINE uint32_t ukl_w_changeable(UKL_W_T, uint64_t s, uint32_t i)
{
    if (i >= ukl_w_get32(t, s, CLIF_W_AT_N) || ukl_w_final(t, s, i >> 10))
        return clif_w_fail(t, s, CLIF_W_FINAL, i);
    return 0u;
}

// `w_retag(i, number)`: node `i`'s code given again; its block not final.
UKL_COLD uint32_t clif_w_retag(UKL_W_T, uint64_t s, uint32_t i, uint32_t number)
{
    UKL_W_TRY(ukl_w_changeable(t, s, i));
    uint32_t j = i & (CLIF_W_Z - 1u);
    UklWList l = ukl_w_list(t, ukl_w_area_of(t, s, i >> 10), 0u);
    uint32_t k = clif_w_find(t, s, l.js, l.count, j);
    if (ukl_w_found(t, l.js, l.count, k, j))
        ukl_w_remove(t, l, k);
    return clif_w_put_code(t, s, i, number);
}

// `w_set_start(i, start)`: open node `i`'s start given again: its gap, its checkpoint and the next node's gap.
UKL_COLD uint32_t clif_w_set_start(UKL_W_T, uint64_t s, uint32_t i, uint64_t start)
{
    // The innermost open entry of node `i`.
    uint64_t opens = ukl_w_part(t, s, CLIF_W_PART_OPEN);
    uint32_t found = CLIF_W_NONE;
    for (uint32_t k = ukl_w_get32(t, s, CLIF_W_AT_OPEN_LEN); k != 0u && found == CLIF_W_NONE; k--)
        if (ukl_w_ld32(t, opens + (uint64_t)(k - 1u) * CLIF_W_OPEN) == i)
            found = k - 1u;
    if (found == CLIF_W_NONE)
        return clif_w_fail(t, s, CLIF_W_NOT_OPEN, i);
    UKL_W_TRY(ukl_w_changeable(t, s, i));
    bool has_next = i + 1u < ukl_w_get32(t, s, CLIF_W_AT_N);
    if (has_next)
        UKL_W_TRY(ukl_w_changeable(t, s, i + 1u));
    uint64_t entry = opens + (uint64_t)found * CLIF_W_OPEN;
    uint64_t old = ukl_w_ld64(t, entry + 16u);
    // The starts before and after it, which bound the new one.
    uint64_t prev = i != 0u ? old - clif_w_value(t, s, 1u, i) : 0u;
    uint64_t next = has_next ? old + clif_w_value(t, s, 1u, i + 1u) : ukl_w_get64(t, s, CLIF_W_AT_SOURCE_LEN);
    if (start < prev || start > next)
        return clif_w_fail(t, s, CLIF_W_BOUNDS, i);
    if (i != 0u)
        UKL_W_TRY(clif_w_put(t, s, 1u, i, start - prev));
    if (has_next)
        UKL_W_TRY(clif_w_put(t, s, 1u, i + 1u, next - start));
    else
        ukl_w_set64(t, s, CLIF_W_AT_PREV_START, start);
    uint32_t j = i & (CLIF_W_Z - 1u);
    if ((j & (CLIF_W_K - 1u)) == 0u)
        ukl_w_put_checkpoint(t, s, i >> 10, j, start);
    ukl_w_st64(t, entry + 16u, start);
    return 0u;
}

// `w_finish_blocks()`: `finish`'s first half: the last node's slot, every block final. A host reserves the tail's
// room after it, knowing the side lists' laid parts (`w_finish_tail`).
UKL_COLD uint32_t clif_w_finish_blocks(UKL_W_T, uint64_t s)
{
    // The first failure, kept, is the op's.
    UKL_W_TRY(ukl_w_get32(t, s, CLIF_W_AT_BAD));
    uint32_t n = ukl_w_get32(t, s, CLIF_W_AT_N);
    if (ukl_w_get32(t, s, CLIF_W_AT_OPEN_LEN) != 0u)
        return clif_w_fail(t, s, CLIF_W_UNCLOSED, n);
    // The last node in preorder is a leaf with nothing after it.
    if (n != 0u)
        UKL_W_TRY(clif_w_put(t, s, 0u, n - 1u, 1u));
    uint32_t count = (n + CLIF_W_Z - 1u) >> 10;
    for (uint32_t b = 0u; b < count; b++) {
        if (!ukl_w_final(t, s, b)) {
            uint32_t rows = n - b * CLIF_W_Z;
            UKL_W_TRY(clif_w_finalize(t, s, b, rows < CLIF_W_Z ? rows : CLIF_W_Z));
        }
    }
    return 0u;
}

// The header at `out`: `magic`, the document's constants, and the tail's offsets (`tail`, ten: the side lists', their
// part starts', the newline index's, the end; for 16z, the tape's it decodes to).
UKL_INLINE void ukl_w_write_head(UKL_W_T, uint64_t s, uint64_t out, uint64_t magic, UKL_PRIVATE const uint64_t* tail)
{
    clif_w_zero(t, s, out, CLIF_W_HEAD16);
    ukl_w_st64(t, out, magic);
    ukl_w_st8(t, out + 8u, ukl_w_get32(t, s, CLIF_W_AT_TAG));
    ukl_w_st8(t, out + 9u, ukl_w_get32(t, s, CLIF_W_AT_MARK));
    ukl_w_st8(t, out + 10u, ukl_w_get32(t, s, CLIF_W_AT_CW));
    ukl_w_st32(t, out + 12u, ukl_w_get32(t, s, CLIF_W_AT_NEWLINES));
    ukl_w_st64(t, out + 16u, ukl_w_get64(t, s, CLIF_W_AT_SOURCE_LEN));
    ukl_w_st64(t, out + 24u, ukl_w_get32(t, s, CLIF_W_AT_N));
    for (uint32_t k = 0u; k < 4u; k++)
        ukl_w_st8(t, out + 32u + k, ukl_w_get32(t, s, CLIF_W_AT_VW + 4u * k));
    for (uint32_t k = 0u; k < 3u; k++)
        ukl_w_st8(t, out + 36u + k, ukl_w_get32(t, s, CLIF_W_AT_W + 4u * k));
    ukl_w_st32(t, out + 40u, ukl_w_get32(t, s, CLIF_W_AT_STRIDE));
    ukl_w_st32(t, out + 44u, CLIF_W_HEAD16);
    for (uint32_t k = 0u; k < 10u; k++)
        ukl_w_st32(t, out + 48u + 4u * k, (uint32_t)tail[k]);
}

// Each side list's part starts by block, `count + 1` u32s each, from `*pos` in the tape at `out`; each list's total.
UKL_INLINE void ukl_w_part_starts(UKL_W_T, uint64_t s, uint64_t out, uint32_t count, UKL_PRIVATE uint64_t* pos, UKL_PRIVATE uint64_t* totals)
{
    uint64_t laid_at = ukl_w_part(t, s, CLIF_W_PART_LAID_AT);
    for (uint32_t q = 0u; q < 4u; q++) {
        uint32_t total = 0u;
        for (uint32_t b = 0u; b <= count; b++) {
            ukl_w_st32(t, out + *pos, total);
            *pos += 4u;
            if (b < count)
                total += ukl_w_ld32(t, laid_at + (uint64_t)b * CLIF_W_LAID_AT + 8u * q + 4u);
        }
        totals[q] = total;
    }
}

// `w_zfinish(count)`: 16z's tail after its payload: each side list's part starts by block, the checkpoints, the
// newline index, the piece index (starts, lengths, kinds); then the header, which says the tape it decodes to and
// where each of these lies.
UKL_COLD uint32_t clif_w_zfinish(UKL_W_T, uint64_t s, uint32_t count)
{
    uint64_t out = ukl_w_part(t, s, CLIF_W_PART_OUT);
    uint64_t payload_end = ukl_w_len(t, s, CLIF_W_PART_OUT);
    uint64_t checkpoints = ukl_w_part(t, s, CLIF_W_PART_CKPT), checkpoints_len = ukl_w_len(t, s, CLIF_W_PART_CKPT);
    uint64_t lines = ukl_w_part(t, s, CLIF_W_PART_LINES), lines_len = ukl_w_len(t, s, CLIF_W_PART_LINES);
    uint64_t pieces = ukl_w_part(t, s, CLIF_W_PART_PIECES);
    uint64_t np = (uint64_t)count << 2;
    uint64_t need = payload_end + ((uint64_t)count + 1u) * 16u + checkpoints_len + lines_len + np * 9u + 30u;
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_OUT, need));
    ukl_w_st64(t, out + payload_end, 0u);
    uint64_t parts_at = (payload_end + 3u) & ~(uint64_t)3;
    uint64_t pos = parts_at, totals[4];
    ukl_w_part_starts(t, s, out, count, &pos, totals);
    uint64_t checkpoints_at = pos;
    clif_w_copy(t, s, out + checkpoints_at, checkpoints, checkpoints_len);
    ukl_w_st64(t, out + checkpoints_at + checkpoints_len, 0u);
    uint64_t lines_at = checkpoints_at + checkpoints_len + 8u;
    clif_w_copy(t, s, out + lines_at, lines, lines_len);
    ukl_w_st64(t, out + lines_at + lines_len, 0u);
    ukl_w_st64(t, out + lines_at + lines_len + 8u, 0u);
    uint64_t starts = (lines_at + lines_len + 11u) & ~(uint64_t)3;
    uint64_t lens = starts + (np << 2), kinds = lens + (np << 2), end = kinds + np;
    for (uint64_t k = 0u; k < np; k++) {
        uint64_t piece = pieces + k * CLIF_W_PIECE;
        ukl_w_st32(t, out + starts + 4u * k, ukl_w_ld32(t, piece));
        ukl_w_st32(t, out + lens + 4u * k, ukl_w_ld32(t, piece + 4u));
        ukl_w_st8(t, out + kinds + k, ukl_w_ld32(t, piece + 12u));
    }
    ukl_w_set_len(t, s, CLIF_W_PART_OUT, end);
    // The tape it decodes to: its blocks' end, then its tail as format 16 places it.
    uint64_t blocks_end = CLIF_W_HEAD16;
    if (count != 0u) {
        uint64_t last = (uint64_t)count - 1u;
        uint64_t rows = (uint64_t)ukl_w_get32(t, s, CLIF_W_AT_N) - last * CLIF_W_Z;
        uint64_t row = (uint64_t)(ukl_w_get32(t, s, CLIF_W_AT_W) + ukl_w_get32(t, s, CLIF_W_AT_W + 4u) + ukl_w_get32(t, s, CLIF_W_AT_W + 8u) + 1u);
        uint64_t cw = ukl_w_get32(t, s, CLIF_W_AT_CW);
        blocks_end = last * (uint64_t)ukl_w_get32(t, s, CLIF_W_AT_STRIDE) + CLIF_W_HEAD16 + rows * row + ((rows + CLIF_W_K - 1u) >> 3) * cw;
    }
    uint64_t tail[10];
    uint64_t at = blocks_end + 8u;
    for (uint32_t q = 0u; q < 4u; q++) {
        tail[q] = at;
        at += totals[q] + 8u;
    }
    at = (at + 3u) & ~(uint64_t)3;
    for (uint32_t q = 0u; q < 4u; q++) {
        tail[4u + q] = at;
        at += ((uint64_t)count + 1u) << 2;
    }
    tail[8] = at;
    tail[9] = at + lines_len + 8u;
    ukl_w_write_head(t, s, out, CLIF_W_MAGIC16Z, tail);
    uint64_t placed[7] = {parts_at, checkpoints_at, lines_at, starts, lens, kinds, end};
    for (uint32_t k = 0u; k < 7u; k++)
        ukl_w_st32(t, out + 88u + 4u * k, (uint32_t)placed[k]);
    return 0u;
}

// `w_finish_tail()`: `finish`'s second half: 16z's tail (`w_zfinish`), or 16's: the last block's columns moved down
// to its rows, then the side lists' parts in block order, their starts, the newline index, the header.
UKL_COLD uint32_t clif_w_finish_tail(UKL_W_T, uint64_t s)
{
    UKL_W_TRY(ukl_w_get32(t, s, CLIF_W_AT_BAD));
    uint32_t n = ukl_w_get32(t, s, CLIF_W_AT_N);
    uint32_t count = (n + CLIF_W_Z - 1u) >> 10;
    if (ukl_w_is_z(t, s))
        return clif_w_zfinish(t, s, count);
    uint64_t out = ukl_w_part(t, s, CLIF_W_PART_OUT);
    uint64_t blocks_end = CLIF_W_HEAD16;
    if (count != 0u) {
        // The last block's columns down to its rows: each column `r` from its place in a full block to `rows` times
        // the bytes a row has before it.
        uint32_t last = count - 1u;
        uint64_t rows = n - last * CLIF_W_Z;
        uint64_t room = ukl_w_room_of(t, s, last);
        uint64_t before = 1u;
        for (uint32_t r = 1u; r < 5u; r++) {
            uint64_t len = r == 4u ? ((rows + CLIF_W_K - 1u) >> 3) * ukl_w_get32(t, s, CLIF_W_AT_CW) : rows * ukl_w_width(t, s, r - 1u);
            uint64_t to = room + rows * before;
            clif_w_copy(t, s, to, room + ukl_w_col(t, s, r), len);
            if (r == 4u)
                blocks_end = to + len - out;
            else
                before += ukl_w_width(t, s, r - 1u);
        }
    }
    // The tail: each side list's parts in block order, their starts, the newline index.
    uint64_t lines_len = ukl_w_len(t, s, CLIF_W_PART_LINES);
    uint64_t need = blocks_end + ukl_w_len(t, s, CLIF_W_PART_LAID) + ((uint64_t)count + 1u) * 16u + lines_len + 56u;
    UKL_W_TRY(ukl_w_room(t, s, CLIF_W_PART_OUT, need));
    ukl_w_st64(t, out + blocks_end, 0u);
    uint64_t pos = blocks_end + 8u;
    uint64_t laid = ukl_w_part(t, s, CLIF_W_PART_LAID), laid_at = ukl_w_part(t, s, CLIF_W_PART_LAID_AT);
    uint64_t tail[10];
    for (uint32_t q = 0u; q < 4u; q++) {
        tail[q] = pos;
        for (uint32_t b = 0u; b < count; b++) {
            uint64_t e = laid_at + (uint64_t)b * CLIF_W_LAID_AT + 8u * q;
            uint64_t len = ukl_w_ld32(t, e + 4u);
            clif_w_copy(t, s, out + pos, laid + ukl_w_ld32(t, e), len);
            pos += len;
        }
        ukl_w_st64(t, out + pos, 0u);
        pos += 8u;
    }
    // The padding to the part starts' alignment is zeros too (the bytes there were the last block's columns).
    ukl_w_st64(t, out + pos, 0u);
    pos = (pos + 3u) & ~(uint64_t)3;
    uint64_t totals[4];
    for (uint32_t q = 0u; q < 4u; q++)
        tail[4u + q] = pos + 4u * ((uint64_t)count + 1u) * q;
    ukl_w_part_starts(t, s, out, count, &pos, totals);
    uint64_t lines_at = pos;
    clif_w_copy(t, s, out + lines_at, ukl_w_part(t, s, CLIF_W_PART_LINES), lines_len);
    ukl_w_st64(t, out + lines_at + lines_len, 0u);
    uint64_t end = lines_at + lines_len + 8u;
    ukl_w_set_len(t, s, CLIF_W_PART_OUT, end);
    tail[8] = lines_at;
    tail[9] = end;
    ukl_w_write_head(t, s, out, CLIF_W_MAGIC16, tail);
    return 0u;
}

// `w_finish()`: the tape: every block final, then its tail and header.
UKL_COLD uint32_t clif_w_finish(UKL_W_T, uint64_t s)
{
    UKL_W_TRY(clif_w_finish_blocks(t, s));
    return clif_w_finish_tail(t, s);
}
