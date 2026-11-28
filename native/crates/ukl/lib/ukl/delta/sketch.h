// ukl/delta/sketch.h: a column's sketch worked in fixed memory and kept as its bytes (`rerail_delta_core::sketch`'s
// `SketchW`, form by form): its rows, its registers, its join counters, and its exact counts while they fit (a few
// values, a page of them, a label set, then rank counts). Its kept bytes are `SketchW::encode_into`'s, so every reader
// reads it as the host's (`sketch::reading`). A working sketch lies in a region of the launch's scratch slot.
#pragma once
#include <ukl/delta/op.h>
#include <ukl/reads/relation.h>

#define SK_NO_VALUE (-1ll)
#define SK_PAST 0xFFFFFFFFFFFFFFFFull
#define SK_FEW_FORM 0u
#define SK_PAGE_FORM 1u
#define SK_LABELS_FORM 2u
#define SK_RANKS_FORM 3u

typedef struct SketchWork {
    uint64_t rows, form, few_len;
    int64_t page_base;
    uint64_t page_ndv, page_sq, label_cap, label_len, label_sq, label_zero;
    uint8_t registers[SKETCH_REGISTERS];
    int32_t counters[SKETCH_BUCKETS];
    int64_t few[SKETCH_FEW][2];
    uint32_t page[SKETCH_DENSE_WIDTH];
    int64_t labels[SKETCH_SPARSE_SLOTS][2];
    uint32_t ranks[SKETCH_REGISTERS * SKETCH_RANKS];
} SketchWork;
typedef CLIF_BLOCK_SPACE SketchWork* Sk;
typedef char sketch_work_fits[sizeof(SketchWork) <= SKETCH_WORK ? 1 : -1];

UKL_INLINE uint64_t sk_hash(int64_t v) { return clif_bloom_hash((CLIF_BLOCK_SPACE const uint8_t*)0, 0u, (uint64_t)v); }
UKL_INLINE uint32_t sk_bucket(uint64_t h) { return (uint32_t)((h >> 8) & 0xFFu); }
UKL_INLINE int32_t sk_sign(uint64_t h) { return (h & 1u) == 0u ? 1 : -1; }
UKL_INLINE uint32_t sk_reg(uint64_t h) { return (uint32_t)(h >> 56); }
UKL_INLINE uint32_t sk_rho(uint64_t h) {
    uint64_t x = (h << 8) | 0x80u;
    uint32_t z = 0u;
    while ((x & 0x8000000000000000ull) == 0u) { x <<= 1; z++; }
    return z + 1u;
}
UKL_INLINE uint32_t sk_cell(uint32_t r, uint32_t rho) { return r * SKETCH_RANKS + (rho < SKETCH_RANKS ? rho : SKETCH_RANKS) - 1u; }
UKL_INLINE bool sk_labelled(int64_t v) { return v >= 0 && v < SKETCH_LABEL_LIMIT; }

UKL_INLINE void sk_clear(Sk w) {
    w->rows = 0u;
    w->form = SK_FEW_FORM;
    w->few_len = 0u;
    for (uint32_t r = 0u; r < SKETCH_REGISTERS; r++) w->registers[r] = 0u;
    for (uint32_t b = 0u; b < SKETCH_BUCKETS; b++) w->counters[b] = 0;
}

UKL_INLINE void sk_count(Sk w, uint64_t h, int64_t n) {
    uint32_t b = sk_bucket(h);
    w->counters[b] = (int32_t)((uint32_t)w->counters[b] + (uint32_t)sk_sign(h) * (uint32_t)n);
}

// A register's rank: its highest rank of any row, from its rank counts.
UKL_INLINE uint8_t sk_register(Sk w, uint32_t r) {
    for (uint32_t i = SKETCH_RANKS; i > 0u; i--)
        if (w->ranks[r * SKETCH_RANKS + i - 1u] > 0u) return (uint8_t)i;
    return 0u;
}
UKL_INLINE void sk_ranks_begin(Sk w) {
    for (uint32_t i = 0u; i < SKETCH_REGISTERS * SKETCH_RANKS; i++) w->ranks[i] = 0u;
}
UKL_INLINE void sk_rank_put(Sk w, int64_t x, uint64_t n) {
    uint64_t h = sk_hash(x);
    w->ranks[sk_cell(sk_reg(h), sk_rho(h))] += (uint32_t)n;
}
UKL_INLINE void sk_ranks_made(Sk w) {
    for (uint32_t r = 0u; r < SKETCH_REGISTERS; r++) w->registers[r] = sk_register(w, r);
    w->form = SK_RANKS_FORM;
    w->few_len = SK_PAST;
}

// -- the few values

UKL_INLINE bool sk_few_add(Sk w, int64_t v, int64_t n) {
    for (uint32_t i = 0u; i < w->few_len; i++)
        if (w->few[i][0] == v) { w->few[i][1] += n; return true; }
    if (w->few_len == SKETCH_FEW) return false;
    w->few[w->few_len][0] = v;
    w->few[w->few_len][1] = n;
    w->few_len++;
    return true;
}
// The value's count left, or -1 when it has none to take.
UKL_INLINE int64_t sk_few_remove(Sk w, int64_t v, int64_t n) {
    for (uint32_t i = 0u; i < w->few_len; i++) {
        if (w->few[i][0] != v) continue;
        if (w->few[i][1] <= 0) return -1;
        w->few[i][1] = w->few[i][1] - n > 0 ? w->few[i][1] - n : 0;
        return w->few[i][1];
    }
    return -1;
}

// -- a page

UKL_INLINE uint32_t sk_page_at(Sk w, int64_t v) {
    int64_t d = v - w->page_base;
    return v >= w->page_base && d >= 0 && d < SKETCH_DENSE_WIDTH ? (uint32_t)d : NONE;
}
UKL_INLINE bool sk_page_add(Sk w, int64_t v, uint64_t k) {
    uint32_t i = sk_page_at(w, v);
    if (i == NONE) return false;
    uint64_t n = w->page[i];
    if (n == 0u) w->page_ndv++;
    w->page_sq += 2u * n * k + k * k;
    w->page[i] += (uint32_t)k;
    return true;
}
UKL_INLINE void sk_page_remove(Sk w, int64_t v, uint64_t k) {
    uint32_t i = sk_page_at(w, v);
    if (i == NONE) return;
    uint64_t n = w->page[i];
    k = k < n ? k : n;
    if (k == 0u) return;
    w->page_sq -= 2u * n * k - k * k;
    w->page[i] -= (uint32_t)k;
    if (w->page[i] == 0u) w->page_ndv--;
}

// -- a label set

UKL_INLINE uint32_t sk_label_slot(Sk w, int64_t v) {
    uint32_t m = (uint32_t)w->label_cap - 1u, at = (uint32_t)sk_hash(v) & m;
    while (w->labels[at][0] != SK_NO_VALUE && w->labels[at][0] != v) at = (at + 1u) & m;
    return at;
}
// Room for one more value: its slots doubled, each entry placed again in slot order (its old slots held in the rank
// counts' words meanwhile, four a slot).
UKL_INLINE void sk_label_grow(Sk w) {
    uint32_t cap = (uint32_t)w->label_cap;
    if (2u * ((uint32_t)w->label_len + 1u) <= cap || cap >= SKETCH_SPARSE_SLOTS) return;
    for (uint32_t i = 0u; i < cap; i++)
        for (uint32_t j = 0u; j < 2u; j++) {
            uint64_t x = (uint64_t)w->labels[i][j];
            w->ranks[4u * i + 2u * j] = (uint32_t)x;
            w->ranks[4u * i + 2u * j + 1u] = (uint32_t)(x >> 32);
        }
    w->label_cap = 2u * cap;
    for (uint32_t i = 0u; i < 2u * cap; i++) { w->labels[i][0] = SK_NO_VALUE; w->labels[i][1] = 0; }
    w->label_len = 0u;
    for (uint32_t i = 0u; i < cap; i++) {
        int64_t e0 = (int64_t)((uint64_t)w->ranks[4u * i] | ((uint64_t)w->ranks[4u * i + 1u] << 32));
        int64_t e1 = (int64_t)((uint64_t)w->ranks[4u * i + 2u] | ((uint64_t)w->ranks[4u * i + 3u] << 32));
        if (e0 == SK_NO_VALUE) continue;
        uint32_t at = sk_label_slot(w, e0);
        w->labels[at][0] = e0;
        w->labels[at][1] = e1;
        w->label_len++;
    }
}
UKL_INLINE void sk_labels_begin(Sk w) {
    w->label_cap = SKETCH_SPARSE_FIRST;
    for (uint32_t i = 0u; i < SKETCH_SPARSE_FIRST; i++) { w->labels[i][0] = SK_NO_VALUE; w->labels[i][1] = 0; }
    w->label_len = w->label_sq = w->label_zero = 0u;
    w->form = SK_LABELS_FORM;
    w->few_len = SK_PAST;
}
UKL_INLINE void sk_label_put(Sk w, int64_t v, uint64_t n) {
    if (n == 0u) return;
    sk_label_grow(w);
    uint32_t at = sk_label_slot(w, v);
    if (w->labels[at][0] == SK_NO_VALUE) w->label_len++;
    w->labels[at][0] = v;
    w->labels[at][1] = (int64_t)n;
    w->label_sq += n * n;
}
UKL_INLINE bool sk_label_add(Sk w, int64_t v, uint64_t k) {
    if (!sk_labelled(v)) return false;
    if (w->label_len >= SKETCH_SPARSE_LIMIT && w->labels[sk_label_slot(w, v)][0] == SK_NO_VALUE) return false;
    sk_label_grow(w);
    uint32_t at = sk_label_slot(w, v);
    if (w->labels[at][0] == SK_NO_VALUE) {
        w->labels[at][0] = v;
        w->labels[at][1] = 0;
        w->label_len++;
    } else if (w->labels[at][1] == 0) {
        w->label_zero--;
    }
    uint64_t n = (uint64_t)w->labels[at][1];
    w->label_sq += 2u * n * k + k * k;
    w->labels[at][1] += (int64_t)k;
    return true;
}
UKL_INLINE void sk_label_remove(Sk w, int64_t v, uint64_t k) {
    if (!sk_labelled(v)) return;
    uint32_t at = sk_label_slot(w, v);
    uint64_t n = (uint64_t)w->labels[at][1];
    k = k < n ? k : n;
    if (w->labels[at][0] != v || k == 0u) return;
    w->label_sq -= 2u * n * k - k * k;
    w->labels[at][1] -= (int64_t)k;
    if (n == k) w->label_zero++;
}

// Its `FEW + 1`th value: a page when they lie in one, else a label set when they are labels, else rank counts.
UKL_INLINE void sk_end_few(Sk w, int64_t v, uint64_t n) {
    int64_t base = v & ~((int64_t)SKETCH_DENSE_WIDTH - 1);
    uint32_t len = (uint32_t)w->few_len;
    bool in_page = sk_labelled(v), labels = sk_labelled(v);
    for (uint32_t i = 0u; i < len; i++) {
        in_page = in_page && w->few[i][0] >= base && w->few[i][0] - base < SKETCH_DENSE_WIDTH;
        labels = labels && sk_labelled(w->few[i][0]);
    }
    if (in_page) {
        for (uint32_t i = 0u; i < SKETCH_DENSE_WIDTH; i++) w->page[i] = 0u;
        w->page_base = base;
        w->page_ndv = 0u;
        w->page_sq = 0u;
        for (uint32_t i = 0u; i < len; i++) {
            w->page_ndv += w->few[i][1] > 0 ? 1u : 0u;
            w->page[(uint32_t)(w->few[i][0] - base)] = (uint32_t)w->few[i][1];
            w->page_sq += (uint64_t)w->few[i][1] * (uint64_t)w->few[i][1];
        }
        sk_page_add(w, v, n);
        w->form = SK_PAGE_FORM;
        w->few_len = SK_PAST;
    } else if (labels) {
        sk_labels_begin(w);
        for (uint32_t i = 0u; i < len; i++) sk_label_put(w, w->few[i][0], (uint64_t)w->few[i][1]);
        sk_label_add(w, v, n);
    } else {
        sk_ranks_begin(w);
        for (uint32_t i = 0u; i < len; i++) sk_rank_put(w, w->few[i][0], (uint64_t)w->few[i][1]);
        sk_rank_put(w, v, n);
        sk_ranks_made(w);
    }
}

// `n` rows of `v` (`SketchW::add_n`).
UKL_INLINE void sk_add(Sk w, int64_t v, uint64_t n) {
    if (n == 0u) return;
    uint64_t h = sk_hash(v);
    uint32_t r = sk_reg(h), rho = sk_rho(h);
    if (rho > w->registers[r]) w->registers[r] = (uint8_t)rho;
    if (w->form == SK_RANKS_FORM) w->ranks[sk_cell(r, rho)] += (uint32_t)n;
    sk_count(w, h, (int64_t)n);
    w->rows += n;
    if (w->form == SK_LABELS_FORM) {
        if (sk_label_add(w, v, n)) return;
        sk_ranks_begin(w);
        for (uint32_t i = 0u; i < w->label_cap; i++)
            if (w->labels[i][0] != SK_NO_VALUE && w->labels[i][1] > 0) sk_rank_put(w, w->labels[i][0], (uint64_t)w->labels[i][1]);
        sk_rank_put(w, v, n);
        sk_ranks_made(w);
    } else if (w->form == SK_PAGE_FORM) {
        if (sk_page_add(w, v, n)) return;
        bool labels = sk_labelled(v) && w->page_ndv < SKETCH_SPARSE_LIMIT;
        if (labels) sk_labels_begin(w); else sk_ranks_begin(w);
        for (uint32_t i = 0u; i < SKETCH_DENSE_WIDTH; i++) {
            uint64_t k = w->page[i];
            if (k == 0u) continue;
            if (labels) sk_label_put(w, w->page_base + (int64_t)i, k); else sk_rank_put(w, w->page_base + (int64_t)i, k);
        }
        if (labels) sk_label_add(w, v, n);
        else { sk_rank_put(w, v, n); sk_ranks_made(w); }
    } else if (w->form == SK_FEW_FORM) {
        if (!sk_few_add(w, v, (int64_t)n)) sk_end_few(w, v, n);
    }
}

// `n` rows of `v` taken out, as they were put in (`SketchW::remove_n`).
UKL_INLINE void sk_remove(Sk w, int64_t v, uint64_t n) {
    n = n < w->rows ? n : w->rows;
    if (n == 0u) return;
    uint64_t h = sk_hash(v);
    uint32_t r = sk_reg(h), rho = sk_rho(h);
    sk_count(w, h, -(int64_t)n);
    w->rows -= n;
    if (w->form == SK_LABELS_FORM) sk_label_remove(w, v, n);
    else if (w->form == SK_PAGE_FORM) sk_page_remove(w, v, n);
    else if (w->form == SK_FEW_FORM) {
        if (sk_few_remove(w, v, (int64_t)n) != 0) return;
        // A value with no rows left: the few values' registers made again from those with rows.
        for (uint32_t q = 0u; q < SKETCH_REGISTERS; q++) w->registers[q] = 0u;
        for (uint32_t i = 0u; i < w->few_len; i++) {
            if (w->few[i][1] <= 0) continue;
            uint64_t g = sk_hash(w->few[i][0]);
            uint32_t q = sk_reg(g), p = sk_rho(g);
            if (p > w->registers[q]) w->registers[q] = (uint8_t)p;
        }
    } else {
        uint32_t at = sk_cell(r, rho);
        w->ranks[at] = w->ranks[at] > (uint32_t)n ? w->ranks[at] - (uint32_t)n : 0u;
        uint32_t capped = rho < SKETCH_RANKS ? rho : SKETCH_RANKS;
        if (w->ranks[at] == 0u && capped >= w->registers[r]) w->registers[r] = sk_register(w, r);
    }
}

// ---- its kept bytes ------------------------------------------------------------------------------------------------

// A reader of kept bytes `at..end` of block `b`: false once past them or past a varint's 64 bits.
typedef struct SkIn {
    CBlk b;
    uint32_t at, end;
    bool ok;
} SkIn;
UKL_INLINE uint8_t sk_byte(UKL_PRIVATE SkIn* in) {
    if (in->at >= in->end) { in->ok = false; return 0u; }
    return in->b[in->at++];
}
UKL_INLINE uint64_t sk_varint(UKL_PRIVATE SkIn* in) {
    uint64_t v = 0u;
    for (uint32_t shift = 0u; shift < 64u; shift += 7u) {
        uint8_t x = sk_byte(in);
        v |= (uint64_t)(x & 0x7Fu) << shift;
        if ((x & 0x80u) == 0u) return v;
    }
    in->ok = false;
    return 0u;
}
UKL_INLINE int64_t sk_unzigzag(uint64_t v) { return (int64_t)(v >> 1) ^ -(int64_t)(v & 1u); }
UKL_INLINE uint64_t sk_zigzag(int64_t v) { return (uint64_t)((v << 1) ^ (v >> 63)); }

// The sketch whose kept bytes are `b[at..end]` (`SketchW::decode`): false, and cleared, when they are not a sketch's of
// this format.
UKL_INLINE bool sk_decode_in(Sk w, CBlk b, uint32_t at, uint32_t end) {
    SkIn in = {b, at, end, true};
    if (sk_byte(&in) != SKETCH_FORMAT) return false;
    uint32_t form = sk_byte(&in);
    sk_clear(w);
    w->rows = sk_varint(&in);
    for (uint32_t r = 0u; r < SKETCH_REGISTERS; r++) w->registers[r] = sk_byte(&in);
    for (uint32_t k = 0u; k < SKETCH_BUCKETS; k++) w->counters[k] = (int32_t)sk_unzigzag(sk_varint(&in));
    uint64_t count = sk_varint(&in);
    if (!in.ok) return false;
    if (form == SK_FEW_FORM) {
        if (count > SKETCH_FEW) return false;
        for (uint32_t k = 0u; k < count; k++) {
            w->few[k][0] = sk_unzigzag(sk_varint(&in));
            w->few[k][1] = (int64_t)sk_varint(&in);
            w->few_len = k + 1u;
        }
    } else if (form == SK_PAGE_FORM) {
        w->page_base = sk_unzigzag(sk_varint(&in));
        for (uint32_t i = 0u; i < SKETCH_DENSE_WIDTH; i++) w->page[i] = 0u;
        w->page_ndv = w->page_sq = 0u;
        uint64_t i = 0u;
        for (uint64_t e = 0u; e < count; e++) {
            uint64_t step = sk_varint(&in);
            i += step;
            if ((e > 0u && step == 0u) || i >= SKETCH_DENSE_WIDTH) return false;
            uint64_t k = sk_varint(&in);
            w->page[i] = (uint32_t)k;
            w->page_ndv++;
            w->page_sq += k * k;
        }
        w->form = SK_PAGE_FORM;
        w->few_len = SK_PAST;
    } else if (form == SK_LABELS_FORM) {
        // Read once to check them, then made as `label_put` makes them.
        SkIn from = in;
        for (uint64_t e = 0u; e < count; e++) {
            int64_t v = (int64_t)sk_varint(&in);
            sk_varint(&in);
            if (!sk_labelled(v)) return false;
        }
        if (count > SKETCH_SPARSE_LIMIT || !in.ok) return false;
        sk_labels_begin(w);
        for (uint64_t e = 0u; e < count; e++) {
            int64_t v = (int64_t)sk_varint(&from);
            sk_label_put(w, v, sk_varint(&from));
        }
    } else if (form == SK_RANKS_FORM) {
        sk_ranks_begin(w);
        uint64_t i = 0u;
        for (uint64_t e = 0u; e < count; e++) {
            uint64_t step = sk_varint(&in);
            i += step;
            if ((e > 0u && step == 0u) || i >= SKETCH_REGISTERS * SKETCH_RANKS) return false;
            w->ranks[i] = (uint32_t)sk_varint(&in);
        }
        w->form = SK_RANKS_FORM;
        w->few_len = SK_PAST;
    } else {
        return false;
    }
    return in.ok && in.at == end;
}
UKL_INLINE bool sk_decode(Sk w, CBlk b, uint32_t at, uint32_t end) {
    bool ok = sk_decode_in(w, b, at, end);
    if (!ok) sk_clear(w);
    return ok;
}

// A writer of kept bytes from `at` of block `b`, or a count of them alone (`b` unset).
typedef struct SkOut {
    Blk b;
    uint32_t at, n;
} SkOut;
UKL_INLINE void sk_put(UKL_PRIVATE SkOut* o, uint8_t x) {
    if (o->b != 0) o->b[o->at + o->n] = x;
    o->n++;
}
UKL_INLINE void sk_put_varint(UKL_PRIVATE SkOut* o, uint64_t v) {
    while (v >= 0x80u) { sk_put(o, (uint8_t)(v | 0x80u)); v >>= 7; }
    sk_put(o, (uint8_t)v);
}

// Its kept bytes (`SketchW::emit`), written or counted.
UKL_INLINE void sk_emit(Sk w, UKL_PRIVATE SkOut* o) {
    sk_put(o, SKETCH_FORMAT);
    sk_put(o, (uint8_t)w->form);
    sk_put_varint(o, w->rows);
    for (uint32_t r = 0u; r < SKETCH_REGISTERS; r++) sk_put(o, w->registers[r]);
    for (uint32_t k = 0u; k < SKETCH_BUCKETS; k++) sk_put_varint(o, sk_zigzag(w->counters[k]));
    if (w->form == SK_FEW_FORM) {
        sk_put_varint(o, w->few_len);
        for (uint32_t i = 0u; i < w->few_len; i++) { sk_put_varint(o, sk_zigzag(w->few[i][0])); sk_put_varint(o, (uint64_t)w->few[i][1]); }
    } else if (w->form == SK_PAGE_FORM) {
        uint64_t n = 0u;
        for (uint32_t i = 0u; i < SKETCH_DENSE_WIDTH; i++) n += w->page[i] > 0u ? 1u : 0u;
        sk_put_varint(o, n);
        sk_put_varint(o, sk_zigzag(w->page_base));
        uint32_t last = 0u;
        for (uint32_t i = 0u; i < SKETCH_DENSE_WIDTH; i++) {
            if (w->page[i] == 0u) continue;
            sk_put_varint(o, i - last);
            sk_put_varint(o, w->page[i]);
            last = i;
        }
    } else if (w->form == SK_LABELS_FORM) {
        uint64_t n = 0u;
        for (uint32_t i = 0u; i < w->label_cap; i++) n += w->labels[i][0] != SK_NO_VALUE && w->labels[i][1] > 0 ? 1u : 0u;
        sk_put_varint(o, n);
        for (uint32_t i = 0u; i < w->label_cap; i++) {
            if (w->labels[i][0] == SK_NO_VALUE || w->labels[i][1] <= 0) continue;
            sk_put_varint(o, (uint64_t)w->labels[i][0]);
            sk_put_varint(o, (uint64_t)w->labels[i][1]);
        }
    } else {
        uint64_t n = 0u;
        for (uint32_t i = 0u; i < SKETCH_REGISTERS * SKETCH_RANKS; i++) n += w->ranks[i] > 0u ? 1u : 0u;
        sk_put_varint(o, n);
        uint32_t last = 0u;
        for (uint32_t i = 0u; i < SKETCH_REGISTERS * SKETCH_RANKS; i++) {
            if (w->ranks[i] == 0u) continue;
            sk_put_varint(o, i - last);
            sk_put_varint(o, w->ranks[i]);
            last = i;
        }
    }
}

// Its kept bytes settled into an exact claim of slot `k`'s block `b`: their offset, and their length in `*len`; NONE
// when the block has no room.
UKL_INLINE uint32_t sk_settle(Sk w, Blk b, UKL_GLOBAL uint32_t* meter, uint32_t k, UKL_PRIVATE uint32_t* len) {
    SkOut count = {0, 0u, 0u};
    sk_emit(w, &count);
    uint32_t at = claim(meter, k, count.n, 8u);
    if (at == NONE) return NONE;
    SkOut out = {b, at, 0u};
    sk_emit(w, &out);
    *len = count.n;
    return at;
}
