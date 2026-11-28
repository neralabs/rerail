// ukl/z16_run.h: a finished format 16 tape stored as 16z (decision 0726, S6) by one program on a work queue, read
// where it lies: the device's half of `z_of(t, lz4_greedy)`, byte for byte, from the tape the chain finished in place.
//
// The items (`UKL_Z16_*`), each a SIMD group's (the group's lanes take up to a group's worth of items and do each
// together, its LZ4 table the group's threadgroup memory):
//
//   COUNT(j)   piece `j` (block `j / 4`, array `j % 4`: its column at the block's rows, then its side list's part,
//              each where it lies in the tape): its LZ4 length, counted, where it holds at least `UKL_Z16_LZ4_MIN`
//              bytes and saves a tenth (the count stops at nine tenths of it), else its bytes; its kind. The last to
//              arrive pushes LAYOUT.
//   LAYOUT     the pieces' places (a scan of their lengths, the group's), the tail's regions placed after them; the
//              head (the tape's, its magic and its 16z fields), the piece index and the pads written; pushes WRITE(0)
//              and COPY(0).
//   WRITE(j)   piece `j` at its place: encoded again, written this time, or its bytes; its block's checkpoints too,
//              with its first piece.
//   COPY(k)    the tail's `k`th run of `UKL_Z16_COPY` bytes: the part starts, then the newline index.
//
// A "for each" is a fan: item `i` pushes `2i + 1` and `2i + 2`, then does `i`. The host seeds COUNT(0) and launches
// the dispatch again while items are left (`<ukl/workqueue.h>`): a fixed few launches whatever the tape's size.
//
// The pool (u32s, coherent): its header (`UKL_Z16_H` words) and then each piece's length, kind and start.

#pragma once
#include <ukl/lz4_encode.h>
#include <ukl/workqueue.h>

#define UKL_Z16_Z 1024u
#define UKL_Z16_K 8u
#define UKL_Z16_HEAD 120u
#define UKL_Z16_HZ 88u
#define UKL_Z16_LZ4_MIN 16u
#define UKL_Z16_COPY 4096u

// The pool's header: the pieces counted and written, a failure, the 16z tape's end, the tail's copies, and the 16z
// fields (`head::Z`: part starts, checkpoints, newline index, piece starts, lengths, kinds, end).
#define UKL_Z16_COUNTED 0u
#define UKL_Z16_WRITTEN 1u
#define UKL_Z16_FAIL 2u
#define UKL_Z16_END 3u
#define UKL_Z16_NCOPY 4u
#define UKL_Z16_FIELDS 8u
#define UKL_Z16_H 16u

#define UKL_Z16_SHIFT 27u
#define UKL_Z16_COUNT 0u
#define UKL_Z16_LAYOUT 1u
#define UKL_Z16_WRITE 2u
#define UKL_Z16_COPYK 3u

#if defined(UKL_CPU) || defined(UKL_VEC)
typedef UKL_GLOBAL uint32_t* UklZ16Pool;
#else
typedef UKL_COHERENT uint32_t* UklZ16Pool;
#endif

typedef struct UklZ16 {
    UKL_GLOBAL const uint8_t* t;
    UKL_GLOBAL uint8_t* z;
    UklZ16Pool m;
    UklWorkQueue q;
    UKL_LOCAL uint32_t* table;
} UklZ16;

// The tape's placement, from its header (`f16::header`).
typedef struct UklZ16P {
    uint32_t n, count, cw, stride, blocks, lines, end;
    uint32_t w[3];
    uint32_t side[4];
    uint32_t parts[4];
} UklZ16P;

UKL_INLINE uint32_t ukl_z16_u32(UKL_GLOBAL const uint8_t* t, uint32_t at)
{
    return (uint32_t)t[at] | ((uint32_t)t[at + 1u] << 8) | ((uint32_t)t[at + 2u] << 16) | ((uint32_t)t[at + 3u] << 24);
}

UKL_INLINE UklZ16P ukl_z16_placement(UKL_GLOBAL const uint8_t* t)
{
    UklZ16P p;
    p.n = ukl_z16_u32(t, 24u);
    p.count = (p.n + UKL_Z16_Z - 1u) / UKL_Z16_Z;
    p.cw = t[10];
    for (uint32_t k = 0u; k < 3u; k++)
        p.w[k] = t[36u + k];
    p.stride = ukl_z16_u32(t, 40u);
    p.blocks = ukl_z16_u32(t, 44u);
    for (uint32_t k = 0u; k < 4u; k++) {
        p.side[k] = ukl_z16_u32(t, 48u + 4u * k);
        p.parts[k] = ukl_z16_u32(t, 64u + 4u * k);
    }
    p.lines = ukl_z16_u32(t, 80u);
    p.end = ukl_z16_u32(t, 84u);
    return p;
}

// A row's bytes before column `r` (0 codes, 1 slots, 2 gaps, 3 lengths, 4 checkpoints); column `q`'s cell.
UKL_INLINE uint32_t ukl_z16_before(UKL_PRIVATE const UklZ16P* p, uint32_t r)
{
    return r == 0u ? 0u : r == 1u ? 1u : r == 2u ? 1u + p->w[0] : r == 3u ? 1u + p->w[0] + p->w[1] : 1u + p->w[0] + p->w[1] + p->w[2];
}
UKL_INLINE uint32_t ukl_z16_cell(UKL_PRIVATE const UklZ16P* p, uint32_t q) { return q == 0u ? 1u : p->w[q - 1u]; }
UKL_INLINE uint32_t ukl_z16_rows(UKL_PRIVATE const UklZ16P* p, uint32_t b) { return p->n - b * UKL_Z16_Z < UKL_Z16_Z ? p->n - b * UKL_Z16_Z : UKL_Z16_Z; }
UKL_INLINE uint32_t ukl_z16_column(UKL_PRIVATE const UklZ16P* p, uint32_t b, uint32_t r)
{
    return p->blocks + b * p->stride + ukl_z16_rows(p, b) * ukl_z16_before(p, r);
}
UKL_INLINE uint32_t ukl_z16_part(UKL_GLOBAL const uint8_t* t, UKL_PRIVATE const UklZ16P* p, uint32_t q, uint32_t j)
{
    return ukl_z16_u32(t, p->parts[q] + 4u * j);
}

// Piece `j`'s bytes where they lie, and their number.
UKL_INLINE UklLz4Spans ukl_z16_piece(UKL_GLOBAL const uint8_t* t, UKL_PRIVATE const UklZ16P* p, uint32_t j, UKL_PRIVATE uint32_t* raw)
{
    uint32_t b = j / 4u, q = j % 4u;
    UklLz4Spans s;
    s.a = t + ukl_z16_column(p, b, q);
    s.an = ukl_z16_rows(p, b) * ukl_z16_cell(p, q);
    uint32_t from = ukl_z16_part(t, p, q, b);
    s.b = t + p->side[q] + from;
    *raw = s.an + ukl_z16_part(t, p, q, b + 1u) - from;
    return s;
}

UKL_INLINE void ukl_z16_fail(UKL_PRIVATE UklZ16* r, uint32_t code) { ukl_atomic_cas(&r->m[UKL_Z16_FAIL], 0u, code); }

// Lane 0's push.
UKL_INLINE void ukl_z16_push(UKL_PRIVATE UklZ16* r, uint32_t kind, uint32_t i, uint32_t lane)
{
    if (lane != 0u)
        return;
    ukl_fence_device();
    if (!ukl_wq_push(r->q, kind << UKL_Z16_SHIFT | i))
        ukl_z16_fail(r, 1u);
}

// A fan's children of item `i` of `n`.
UKL_INLINE void ukl_z16_fan(UKL_PRIVATE UklZ16* r, uint32_t kind, uint32_t i, uint32_t n, uint32_t lane)
{
    for (uint32_t c = 2u * i + 1u; c <= 2u * i + 2u; c++)
        if (c < n)
            ukl_z16_push(r, kind, c, lane);
}

// Lane 0's arrival at counter `at`, every lane told whether it was the last of `target`.
UKL_INLINE bool ukl_z16_arrive(UKL_PRIVATE UklZ16* r, uint32_t at, uint32_t target, uint32_t lane __UKL_PARAMS)
{
    ukl_fence_device();
    uint32_t last = 0u;
    if (lane == 0u)
        last = ukl_atomic_add(&r->m[at], 1u) + 1u == target ? 1u : 0u;
    last = ukl_sg_broadcast(last, 0u __UKL_ARGS);
    ukl_fence_device();
    return last != 0u;
}

// `n` bytes from `from` (two spans, as a piece's) to `to`, a byte a lane.
UKL_INLINE void ukl_z16_copy(UklLz4Spans s, uint32_t n, UKL_GLOBAL uint8_t* to, uint32_t lane)
{
    for (uint32_t i = lane; i < n; i += UKL_SIMD_WIDTH)
        to[i] = ukl_lz4e_byte(s, i);
}
UKL_INLINE void ukl_z16_copy1(UKL_GLOBAL const uint8_t* from, uint32_t n, UKL_GLOBAL uint8_t* to, uint32_t lane)
{
    for (uint32_t i = lane; i < n; i += UKL_SIMD_WIDTH)
        to[i] = from[i];
}
UKL_INLINE void ukl_z16_zeros(UKL_GLOBAL uint8_t* to, uint32_t n, uint32_t lane)
{
    for (uint32_t i = lane; i < n; i += UKL_SIMD_WIDTH)
        to[i] = 0u;
}
UKL_INLINE void ukl_z16_put32(UKL_GLOBAL uint8_t* to, uint32_t v)
{
    for (uint32_t k = 0u; k < 4u; k++)
        to[k] = (uint8_t)(v >> (8u * k));
}

// The pieces' lengths, kinds and starts in the pool.
UKL_INLINE uint32_t ukl_z16_len(uint32_t np, uint32_t j) { return UKL_Z16_H + j; }
UKL_INLINE uint32_t ukl_z16_kind(uint32_t np, uint32_t j) { return UKL_Z16_H + np + j; }
UKL_INLINE uint32_t ukl_z16_start(uint32_t np, uint32_t j) { return UKL_Z16_H + 2u * np + j; }

UKL_INLINE void ukl_z16_count(UKL_PRIVATE UklZ16* r, UKL_PRIVATE const UklZ16P* p, uint32_t j, uint32_t lane __UKL_PARAMS)
{
    uint32_t np = 4u * p->count, raw = 0u;
    if (np == 0u) {
        ukl_z16_push(r, UKL_Z16_LAYOUT, 0u, lane);
        return;
    }
    ukl_z16_fan(r, UKL_Z16_COUNT, j, np, lane);
    UklLz4Spans s = ukl_z16_piece(r->t, p, j, &raw);
    uint32_t len = UKL_NONE;
    if (raw >= UKL_Z16_LZ4_MIN)
        len = ukl_lz4_encode2_sg(s, raw, r->z, raw * 9u / 10u, false, r->table, lane __UKL_ARGS);
    if (lane == 0u) {
        r->m[ukl_z16_len(np, j)] = len == UKL_NONE ? raw : len;
        r->m[ukl_z16_kind(np, j)] = len == UKL_NONE ? 0u : 1u;
    }
    if (ukl_z16_arrive(r, UKL_Z16_COUNTED, np, lane __UKL_ARGS))
        ukl_z16_push(r, UKL_Z16_LAYOUT, 0u, lane);
}

UKL_INLINE uint32_t ukl_z16_round4(uint32_t x) { return (x + 3u) & ~3u; }

UKL_INLINE void ukl_z16_layout(UKL_PRIVATE UklZ16* r, UKL_PRIVATE const UklZ16P* p, uint32_t lane __UKL_PARAMS)
{
    UklZ16Pool m = r->m;
    UKL_GLOBAL uint8_t* z = r->z;
    uint32_t np = 4u * p->count;
    // The pieces' starts in the payload, a scan of their lengths a group's worth at a time.
    uint32_t total = 0u;
    for (uint32_t base = 0u; base < np; base += UKL_SIMD_WIDTH) {
        uint32_t j = base + lane;
        uint32_t v = j < np ? m[ukl_z16_len(np, j)] : 0u;
        uint32_t ex = ukl_sg_exclusive_sum(v, lane __UKL_ARGS);
        if (j < np)
            m[ukl_z16_start(np, j)] = total + ex;
        total += ukl_sg_sum(v __UKL_ARGS);
    }
    uint32_t at = UKL_Z16_HEAD + total;
    uint32_t f[7];
    f[0] = ukl_z16_round4(at);
    f[1] = f[0] + (p->lines - p->parts[0]);
    uint32_t ck = p->count == 0u ? 0u : (p->count - 1u) * (UKL_Z16_Z / UKL_Z16_K) * p->cw + (ukl_z16_rows(p, p->count - 1u) + UKL_Z16_K - 1u) / UKL_Z16_K * p->cw;
    f[2] = f[1] + ck + 8u;
    uint32_t lines = p->end - 8u - p->lines;
    f[3] = ukl_z16_round4(f[2] + lines + 8u);
    f[4] = f[3] + 4u * np;
    f[5] = f[4] + 4u * np;
    f[6] = f[5] + np;
    // The head: the tape's, its magic's last byte `Z`, its fields, then its pad.
    for (uint32_t i = lane; i < UKL_Z16_HEAD; i += UKL_SIMD_WIDTH) {
        uint32_t k = (i - UKL_Z16_HZ) / 4u;
        z[i] = i == 7u ? (uint8_t)'Z' : i < UKL_Z16_HZ ? r->t[i] : k < 7u ? (uint8_t)(f[k] >> (8u * ((i - UKL_Z16_HZ) % 4u))) : 0u;
    }
    // The pads: after the payload, after the checkpoints, after the newline index.
    ukl_z16_zeros(z + at, f[0] - at, lane);
    ukl_z16_zeros(z + f[2] - 8u, 8u, lane);
    ukl_z16_zeros(z + f[2] + lines, f[3] - f[2] - lines, lane);
    // The piece index.
    for (uint32_t j = lane; j < np; j += UKL_SIMD_WIDTH) {
        ukl_z16_put32(z + f[3] + 4u * j, m[ukl_z16_start(np, j)]);
        ukl_z16_put32(z + f[4] + 4u * j, m[ukl_z16_len(np, j)]);
        z[f[5] + j] = (uint8_t)m[ukl_z16_kind(np, j)];
    }
    uint32_t ncopy = (p->lines - p->parts[0] + UKL_Z16_COPY - 1u) / UKL_Z16_COPY + (lines + UKL_Z16_COPY - 1u) / UKL_Z16_COPY;
    if (lane == 0u) {
        for (uint32_t k = 0u; k < 7u; k++)
            m[UKL_Z16_FIELDS + k] = f[k];
        m[UKL_Z16_END] = f[6];
        m[UKL_Z16_NCOPY] = ncopy;
    }
    ukl_fence_device();
    if (np != 0u)
        ukl_z16_push(r, UKL_Z16_WRITE, 0u, lane);
    if (ncopy != 0u)
        ukl_z16_push(r, UKL_Z16_COPYK, 0u, lane);
}

UKL_INLINE void ukl_z16_write(UKL_PRIVATE UklZ16* r, UKL_PRIVATE const UklZ16P* p, uint32_t j, uint32_t lane __UKL_PARAMS)
{
    UklZ16Pool m = r->m;
    uint32_t np = 4u * p->count, raw = 0u;
    ukl_z16_fan(r, UKL_Z16_WRITE, j, np, lane);
    UklLz4Spans s = ukl_z16_piece(r->t, p, j, &raw);
    uint32_t len = m[ukl_z16_len(np, j)];
    UKL_GLOBAL uint8_t* to = r->z + UKL_Z16_HEAD + m[ukl_z16_start(np, j)];
    if (m[ukl_z16_kind(np, j)] == 1u) {
        if (ukl_lz4_encode2_sg(s, raw, to, len, true, r->table, lane __UKL_ARGS) != len)
            if (lane == 0u)
                ukl_z16_fail(r, 2u);
    } else {
        ukl_z16_copy(s, raw, to, lane);
    }
    // Its block's checkpoints, with its first piece.
    uint32_t b = j / 4u;
    if (j % 4u == 0u) {
        uint32_t n = (ukl_z16_rows(p, b) + UKL_Z16_K - 1u) / UKL_Z16_K * p->cw;
        ukl_z16_copy1(r->t + ukl_z16_column(p, b, 4u), n, r->z + m[UKL_Z16_FIELDS + 1u] + b * (UKL_Z16_Z / UKL_Z16_K) * p->cw, lane);
    }
    ukl_z16_arrive(r, UKL_Z16_WRITTEN, np, lane __UKL_ARGS);
}

UKL_INLINE void ukl_z16_tail(UKL_PRIVATE UklZ16* r, UKL_PRIVATE const UklZ16P* p, uint32_t k, uint32_t lane)
{
    UklZ16Pool m = r->m;
    ukl_z16_fan(r, UKL_Z16_COPYK, k, m[UKL_Z16_NCOPY], lane);
    uint32_t parts = p->lines - p->parts[0], na = (parts + UKL_Z16_COPY - 1u) / UKL_Z16_COPY;
    uint32_t from = 0u, to = 0u, n = 0u;
    if (k < na) {
        from = p->parts[0] + k * UKL_Z16_COPY;
        to = m[UKL_Z16_FIELDS] + k * UKL_Z16_COPY;
        n = parts - k * UKL_Z16_COPY;
    } else {
        uint32_t i = (k - na) * UKL_Z16_COPY;
        from = p->lines + i;
        to = m[UKL_Z16_FIELDS + 2u] + i;
        n = p->end - 8u - p->lines - i;
    }
    ukl_z16_copy1(r->t + from, n < UKL_Z16_COPY ? n : UKL_Z16_COPY, r->z + to, lane);
}

UKL_INLINE void ukl_z16_item(UKL_PRIVATE UklZ16* r, uint32_t item, uint32_t lane __UKL_PARAMS)
{
    UklZ16P p = ukl_z16_placement(r->t);
    uint32_t kind = item >> UKL_Z16_SHIFT, i = item & ((1u << UKL_Z16_SHIFT) - 1u);
    if (kind == UKL_Z16_COUNT)
        ukl_z16_count(r, &p, i, lane __UKL_ARGS);
    else if (kind == UKL_Z16_LAYOUT)
        ukl_z16_layout(r, &p, lane __UKL_ARGS);
    else if (kind == UKL_Z16_WRITE)
        ukl_z16_write(r, &p, i, lane __UKL_ARGS);
    else
        ukl_z16_tail(r, &p, i, lane);
}

// A SIMD group's worker: its lanes take up to a group's worth of items, and the group does each in turn, together.
UKL_INLINE void ukl_z16_worker(UKL_PRIVATE UklZ16* r, uint32_t lane __UKL_PARAMS)
{
    uint32_t taken = 0u;
    for (uint32_t turn = 0u; turn < UKL_WQ_TURNS && !ukl_wq_spent(r->q, taken); turn++) {
        uint32_t item = 0u;
        bool got = ukl_wq_take_sg(r->q, lane, &item __UKL_ARGS);
        ukl_fence_device();
        ukl_mask_t has = ukl_sg_ballot(got __UKL_ARGS);
        if (has == 0u)
            break;
        for (uint32_t k = 0u; k < UKL_SIMD_WIDTH; k++)
            if (((has >> k) & 1u) != 0u)
                ukl_z16_item(r, ukl_sg_broadcast(item, k __UKL_ARGS), lane __UKL_ARGS);
        if (got)
            ukl_wq_done(r->q);
        taken++;
    }
}
