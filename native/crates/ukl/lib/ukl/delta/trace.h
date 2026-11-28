// ukl/delta/trace.h: a trace's state, read where its layers lie (`rerail_delta_core::layer` and `::trace`). A layer
// block holds three relations: its own rows, the partitions it changed (`parts(part, digest, added, gone, first,
// kills_at)`, sorted on the partition) and its kills by partition (`gone(part, id)`). Each layer continues the one
// below, found by its block id in the launch's table. A row is live when no layer above its own kills it.
//
// Rows are read by the relation reads written once in CLIF (`<ukl/reads/relation.h>`, decision 0310). Only the Open
// encoding is read here (`Shape::run`'s, every executor trace's): a layer's encoded rows have no reads here yet.
#pragma once
#include <ukl/delta/op.h>
#include <ukl/reads/relation.h>

// A layer, read in place: its three relations, its shape's order columns, its own ids `start..end`, the block below
// (NONE: none), its arity and its partition column (NONE: none).
typedef struct Layer {
    CBlk rows, parts, gone, order;
    uint32_t start, end, below, arity, pcol;
    // Where its reads are counted (`delta_work`; 0: nowhere): a value, and each step of a kill search, as the twins'.
    CLIF_BLOCK_SPACE uint32_t* work;
} Layer;

// Ids `lo..hi`.
typedef struct Span {
    uint32_t lo, hi;
} Span;

// Relation `rel`'s own row `id` in column `c`.
UKL_INLINE int64_t rel_get(CBlk rel, uint32_t id, uint32_t c) { return (int64_t)clif_rel_get(rel, 0u, id, c, 0u); }
UKL_INLINE uint32_t rel_start(CBlk rel) { return U32(rel, REL_BASE_ROWS_AT); }
UKL_INLINE uint32_t rel_end(CBlk rel) { return rel_start(rel) + clif_rel_len(rel, 0u); }

// The layer in block `id`.
UKL_INLINE Layer layer_of(CBlk t, uint32_t id) {
    CBlk b = blk(t, id);
    Layer l;
    l.rows = b + U64(b, LAYER_ROWS_AT);
    l.parts = b + U64(b, LAYER_PARTS_AT);
    l.gone = b + U64(b, LAYER_GONE_AT);
    l.start = rel_start(l.rows);
    l.end = rel_end(l.rows);
    l.below = (U32(l.rows, REL_FLAGS_AT) & REL_FAR_BASE) != 0u ? U32(l.rows, REL_BASE_BLOCK_AT) : NONE;
    l.arity = U32(b, LAYER_SHAPE_AT + SHAPE_ARITY_AT);
    l.pcol = U32(b, LAYER_SHAPE_AT + SHAPE_PARTITION_AT);
    l.order = b + LAYER_SHAPE_AT + SHAPE_ORDER_AT;
    l.work = delta_work(t);
    return l;
}

// Own row `id`'s value in column `c`.
UKL_INLINE int64_t layer_get(Layer l, uint32_t id, uint32_t c) {
    delta_touch(l.work, 1u);
    return rel_get(l.rows, id, c);
}

// Partition `p`'s entry in the layer's `parts`, or NONE when the layer did not change it.
UKL_INLINE uint32_t layer_part(Layer l, int64_t p) {
    uint32_t lo = rel_start(l.parts), hi = rel_end(l.parts), end = hi;
    while (lo < hi) {
        uint32_t m = (lo + hi) / 2u;
        if (rel_get(l.parts, m, 0u) < p) lo = m + 1u; else hi = m;
    }
    return lo < end && rel_get(l.parts, lo, 0u) == p ? lo : NONE;
}

// Partition `p`'s own rows: every own row when the trace has no partition.
UKL_INLINE Span layer_part_rows(Layer l, int64_t p) {
    Span s = {l.start, l.end};
    if (l.pcol == NONE) return s;
    uint32_t e = layer_part(l, p);
    if (e == NONE) { s.hi = s.lo; return s; }
    s.lo = l.start + (uint32_t)rel_get(l.parts, e, 4u);
    s.hi = s.lo + (uint32_t)rel_get(l.parts, e, 2u);
    return s;
}

// Whether the layer kills row `id` of partition `p`: a search of the partition's kills, ascending.
UKL_INLINE bool layer_kills(Layer l, int64_t p, uint32_t id) {
    uint32_t e = layer_part(l, p);
    if (e == NONE) return false;
    uint32_t at = rel_start(l.gone) + (uint32_t)rel_get(l.parts, e, 5u), n = (uint32_t)rel_get(l.parts, e, 3u);
    uint32_t lo = at, hi = at + n;
    while (lo < hi) {
        uint32_t m = (lo + hi) / 2u;
        delta_touch(l.work, 1u);
        if ((uint32_t)rel_get(l.gone, m, 1u) < id) lo = m + 1u; else hi = m;
    }
    return lo < at + n && (uint32_t)rel_get(l.gone, lo, 1u) == id;
}

// The state at layer `head`: its layers, newest first, into `ls` (at most MAX_LAYERS, as its state was checked when
// made); how many.
UKL_INLINE uint32_t state_layers(CBlk t, uint32_t head, UKL_PRIVATE Layer* ls) {
    uint32_t n = 0;
    for (uint32_t k = head; k != NONE && n < MAX_LAYERS; k = ls[n - 1u].below) ls[n++] = layer_of(t, k);
    return n;
}

// Whether row `id` of partition `p`, an own row of layer `k` of `ls`, is killed by a layer above it.
UKL_INLINE bool killed_above(UKL_PRIVATE const Layer* ls, uint32_t k, int64_t p, uint32_t id) {
    for (uint32_t j = 0u; j < k; j++)
        if (layer_kills(ls[j], p, id)) return true;
    return false;
}

// Whether own row `ia` of `a` comes before own row `ib` of `b` in the canonical order: by the shape's order columns.
UKL_INLINE bool row_before(Layer a, uint32_t ia, Layer b, uint32_t ib) {
    for (uint32_t k = 0u; k < a.arity; k++) {
        uint32_t c = a.order[k];
        int64_t x = layer_get(a, ia, c), y = layer_get(b, ib, c);
        if (x != y) return x < y;
    }
    return false;
}

// A state by its head alone (NONE: the empty state), each read a walk down its layers: what a kernel reads a row of
// one by, where order does not matter.

// The first id past the state's.
UKL_INLINE uint32_t state_end(CBlk t, uint32_t head) { return head == NONE ? 0u : layer_of(t, head).end; }

// The layer that holds own row `id`: its block, or NONE.
UKL_INLINE uint32_t state_owner(CBlk t, uint32_t head, uint32_t id) {
    for (uint32_t k = head; k != NONE;) {
        Layer l = layer_of(t, k);
        if (id >= l.start) return id < l.end ? k : NONE;
        k = l.below;
    }
    return NONE;
}

// Own row `id`'s partition: its value in the partition column (0 when the trace has none).
UKL_INLINE int64_t layer_part_of(Layer l, uint32_t id) { return l.pcol == NONE ? 0 : layer_get(l, id, l.pcol); }

// Row `id` in column `c`, of a row the state holds.
UKL_INLINE int64_t state_get(CBlk t, uint32_t head, uint32_t id, uint32_t c) {
    return layer_get(layer_of(t, state_owner(t, head, id)), id, c);
}

// Whether row `id` is live: an own row of a layer, killed by no layer above it.
UKL_INLINE bool state_live(CBlk t, uint32_t head, uint32_t id) {
    uint32_t own = state_owner(t, head, id);
    if (own == NONE) return false;
    int64_t p = layer_part_of(layer_of(t, own), id);
    for (uint32_t k = head; k != own;) {
        Layer l = layer_of(t, k);
        if (layer_kills(l, p, id)) return false;
        k = l.below;
    }
    return true;
}

// Whether own row `id` of layer `l` (in block `own`) is live in the state at `head`: killed by no layer above it.
UKL_INLINE bool own_live(CBlk t, uint32_t head, uint32_t own, Layer l, uint32_t id) {
    int64_t p = layer_part_of(l, id);
    for (uint32_t k = head; k != own;) {
        Layer a = layer_of(t, k);
        if (layer_kills(a, p, id)) return false;
        k = a.below;
    }
    return true;
}

// The layer's rows' hashed index on column `c` alone, or NONE (`Relation::find_index` of `[c]`).
UKL_INLINE uint32_t layer_index_on(Layer l, uint32_t c) {
    uint32_t f = REL_INDEXES_AT, at = (uint32_t)((int64_t)(f & ~7u) + (int64_t)I32(l.rows, f) * 8);
    for (uint32_t x = 0u, n = U32(l.rows, REL_NINDEX_AT); x < n; x++) {
        CBlk ix = l.rows + at + x * INDEX_BYTES;
        if (U32(ix, INDEX_NCOLS_AT) == 1u && U32(ix, INDEX_ORDERED_AT) == 0u && *(CLIF_BLOCK_SPACE const uint16_t*)(ix + INDEX_KEY_AT) == c) return x;
    }
    return NONE;
}

// A probe of the state at `head` for the live rows whose column `c` is `v`, as `trace::ProbeIter` reads them: each
// layer in turn, newest first, by its indexes: a range by binary search when `c` leads the canonical order (its rows
// lie in that order), a hashed index's bucket on `c` alone, else its rows. `probe_next` gives each match once, NONE
// past the last.
#define PROBE_RANGE 0u
#define PROBE_BUCKET 1u
#define PROBE_ROWS 2u
typedef struct Probe {
    CBlk t;
    Layer l;
    int64_t v;
    uint32_t head, own, next, c, kind, x, i, end;
} Probe;

UKL_INLINE Probe probe_of(CBlk t, uint32_t head, uint32_t c, int64_t v) {
    Probe p;
    p.t = t;
    p.head = head;
    p.own = NONE;
    p.next = head;
    p.c = c;
    p.v = v;
    p.kind = PROBE_ROWS;
    p.x = NONE;
    p.i = 0u;
    p.end = 0u;
    return p;
}

// The probe's layer `k` opened: its candidates.
UKL_INLINE void probe_open(UKL_PRIVATE Probe* p, uint32_t k) {
    Layer l = layer_of(p->t, k);
    p->l = l;
    p->own = k;
    p->next = l.below;
    p->i = l.start;
    p->end = l.end;
    p->kind = PROBE_ROWS;
    if (l.end == l.start) return;
    if (l.order[0] == p->c) {
        uint32_t lo = l.start, hi = l.end;
        while (lo < hi) {
            uint32_t m = (lo + hi) / 2u;
            if (layer_get(l, m, p->c) < p->v) lo = m + 1u; else hi = m;
        }
        p->kind = PROBE_RANGE;
        p->i = lo;
        return;
    }
    uint32_t x = layer_index_on(l, p->c);
    if (x == NONE) return;
    uint64_t g = clif_rel_group(l.rows, 0u, x, clif_rel_key_mix(l.rows, 0u, KEY_SEED, (uint64_t)p->v));
    p->kind = PROBE_BUCKET;
    p->x = x;
    p->i = (uint32_t)(g >> 32);
    p->end = p->i + (uint32_t)g;
}

UKL_INLINE uint32_t probe_next(UKL_PRIVATE Probe* p) {
    for (;;) {
        while (p->i < p->end) {
            uint32_t id = p->kind == PROBE_BUCKET ? clif_rel_group_row(p->l.rows, 0u, p->x, p->i) : p->i;
            p->i++;
            if (id < p->l.start || id >= p->l.end) continue;
            if (layer_get(p->l, id, p->c) != p->v) {
                if (p->kind == PROBE_RANGE) p->i = p->end;
                continue;
            }
            if (own_live(p->t, p->head, p->own, p->l, id)) return id;
        }
        if (p->next == NONE) return NONE;
        probe_open(p, p->next);
    }
}

// The first live row whose column `c` is `v` in the probe's order, or NONE: the row a key is taken once at.
UKL_INLINE uint32_t state_first(CBlk t, uint32_t head, uint32_t c, int64_t v) {
    Probe p = probe_of(t, head, c, v);
    return probe_next(&p);
}
UKL_INLINE bool state_has(CBlk t, uint32_t head, uint32_t c, int64_t v) { return state_first(t, head, c, v) != NONE; }

// A trace between two states (`before` NONE: cold): row `id` gone, added, or kept.
typedef struct Bi {
    uint32_t before, after;
} Bi;
UKL_INLINE bool bi_gone(CBlk t, Bi s, uint32_t id) { return state_live(t, s.before, id) && !state_live(t, s.after, id); }
UKL_INLINE bool bi_added(CBlk t, Bi s, uint32_t id) { return state_live(t, s.after, id) && !state_live(t, s.before, id); }
UKL_INLINE bool bi_kept(CBlk t, Bi s, uint32_t id) { return state_live(t, s.before, id) && state_live(t, s.after, id); }
