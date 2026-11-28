// ukl/lex_run.h: tier 1 from the bytes on the device (decision 0726), a launch a phase, the scans of `lex_typed_in`
// on the device: nothing but the source and its length comes from the host.
//
// The phases (`UKL_LEX_PH_*`), each its items (the chunks, `blocks` blocks of 64 bytes each) taken by the launch's
// SIMD groups by ticket, a lane a chunk:
//
//   ENDS    pass 1 (`stage1_ends`): the chunk's end from each lane's entry, as a map of lanes, a leaf of M, the maps'
//           scan (`entries_in`: a map composed after the maps before it).
//   COUNT   the chunk's entry, M's state walked down from the top to it applied to the file's start lane; pass 2
//           counting from it (`stage1_from`): its records and each type's, a leaf of K, the counts' scan. K's root
//           says the totals, and ends the records (`starts`' three words after them: the length twice, 0).
//   WRITE   the chunk's places, K's state walked down to it (each type's list after the types before it); pass 2
//           writing its records and each type's records' numbers there; each of its records' token none
//           (`Token::NOT`), the token pass writing those of its types' records after.
//
// A scan's tree is as `<ukl/phases_run.h>`'s: a leaf stores its value and arrives at its parent, the second child to
// arrive composing it, so the root's value is the file's; a leaf's state is walked down from the top, a right child
// its parent's after its left sibling's value. M's values are maps of lanes (`UKL_LEX_LANES` of 4 bits, lane `l`'s
// end lane at bits `4l`), composed in file order; K's are each chunk's `UKL_LEX_TYPES + 2` counts (its records, each type's, its newlines), summed.
//
// The grammar's (the printer's): `ukl_lex_ends` (a chunk's end lane from each entry lane) and `ukl_lex_from` (pass 2
// from an entry lane), declared here, and `UKL_LEX_LANES`, `UKL_LEX_TYPES` defined before it is included.
//
// The pool (u32s, coherent): its header (`UKL_LEX_H` words: the host's layout, and what the phases say, which tier
// 2's kernels read), then M (values, arrivals), K (values, `UKL_LEX_TYPES + 2` a node; arrivals) and the entries.

#pragma once
#include <ukl/simd.h>

#include <ukl/lex_pool.h>

#define UKL_LEX_PH_ENDS 0u
#define UKL_LEX_PH_COUNT 1u
#define UKL_LEX_PH_WRITE 2u
#define UKL_LEX_TURNS 100000000u
#define UKL_LEX_K (UKL_LEX_TYPES + 2u)
#define UKL_LEX_NOT_KIND 4294967295u

typedef struct UklLex {
    UKL_GLOBAL const uint8_t* src;
    UklLexPool m;
    // Tier 1's records (with their three words after), each type's records' numbers (one list after another), and
    // each record's token (5 words), its none written here.
    UKL_GLOBAL uint32_t* starts;
    UKL_GLOBAL uint32_t* types;
    UKL_GLOBAL uint32_t* tokens;
    // The trees' arrivals (`ukl_lex_meet`).
    UKL_GLOBAL uint32_t* meet;
    uint32_t phase;
} UklLex;

// Chunk `from..to`'s (blocks) end lane from each entry lane, at `ends`; pass 2 from entry lane `lane`: the counts at
// `at` (each place where `write`), its records and its types' written where `write`, and its newlines' bits into the
// line index's words at `lines` where `shape[3]` (`[L, high bits' word, low parts' word, on]`); whether it refuses.
UKL_INLINE void ukl_lex_ends(UKL_GLOBAL const uint8_t* src, uint32_t n, uint32_t from, uint32_t to, UKL_PRIVATE uint32_t* ends);
UKL_INLINE bool ukl_lex_from(UKL_GLOBAL const uint8_t* src, uint32_t n, uint32_t from, uint32_t to, uint32_t lane, UKL_PRIVATE uint32_t* at,
                             UKL_GLOBAL uint32_t* starts, UKL_GLOBAL uint32_t* types, bool write, UKL_GLOBAL uint32_t* lines,
                             UKL_PRIVATE const uint32_t* shape);

#define UKL_LEX_TM 0u
#define UKL_LEX_TK 1u

UKL_INLINE uint32_t ukl_lex_width(uint32_t tr) { return tr == UKL_LEX_TM ? 1u : UKL_LEX_K; }
// Tree `tr`'s value word `k` of node `x`, its arrivals at `x`.
UKL_INLINE uint32_t ukl_lex_val(UKL_PRIVATE UklLex* r, uint32_t tr, uint32_t x, uint32_t k)
{
    return r->m[tr == UKL_LEX_TM ? UKL_LEX_OM : UKL_LEX_OK] + x * ukl_lex_width(tr) + k;
}
// Node `x` of tree `tr` arrived at: whether its other child arrived first (`<ukl/phases_run.h>`'s `meet`: the run's
// epoch exchanged into its word, which only ever holds an epoch).
UKL_INLINE bool ukl_lex_meet(UKL_PRIVATE UklLex* r, uint32_t tr, uint32_t x)
{
    uint32_t epoch = r->m[UKL_LEX_EPOCH];
    ukl_fence_device();
    bool last = ukl_atomic_exchange(&r->meet[tr * r->m[UKL_LEX_TN] + x], epoch) == epoch;
    ukl_fence_device();
    return last;
}

// The map that is `a` then `b`: lane `l` to `b`'s lane of `a`'s.
UKL_INLINE uint32_t ukl_lex_then(uint32_t a, uint32_t b)
{
    uint32_t out = 0u;
    for (uint32_t l = 0u; l < UKL_LEX_LANES; l++)
        out |= ((b >> (4u * ((a >> (4u * l)) & 15u))) & 15u) << (4u * l);
    return out;
}
UKL_INLINE uint32_t ukl_lex_identity(void)
{
    uint32_t out = 0u;
    for (uint32_t l = 0u; l < UKL_LEX_LANES; l++)
        out |= l << (4u * l);
    return out;
}

// Node `to`'s value: `a` then `b` (`two`), else `a`.
UKL_INLINE void ukl_lex_compose(UKL_PRIVATE UklLex* r, uint32_t tr, uint32_t to, uint32_t a, uint32_t b, bool two)
{
    UklLexPool m = r->m;
    if (tr == UKL_LEX_TM) {
        uint32_t x = m[ukl_lex_val(r, tr, a, 0u)];
        m[ukl_lex_val(r, tr, to, 0u)] = two ? ukl_lex_then(x, m[ukl_lex_val(r, tr, b, 0u)]) : x;
    } else {
        for (uint32_t k = 0u; k < UKL_LEX_K; k++)
            m[ukl_lex_val(r, tr, to, k)] = m[ukl_lex_val(r, tr, a, k)] + (two ? m[ukl_lex_val(r, tr, b, k)] : 0u);
    }
}

// K's root, its value the file's counts: the totals, and the records ended.
UKL_INLINE void ukl_lex_root(UKL_PRIVATE UklLex* r, uint32_t top)
{
    UklLexPool m = r->m;
    uint32_t count = m[ukl_lex_val(r, UKL_LEX_TK, top, 0u)], typed = 0u;
    for (uint32_t k = 1u; k <= UKL_LEX_TYPES; k++) {
        m[UKL_LEX_TYPE0 + k - 1u] = m[ukl_lex_val(r, UKL_LEX_TK, top, k)];
        typed += m[ukl_lex_val(r, UKL_LEX_TK, top, k)];
    }
    m[UKL_LEX_NEWLINES] = m[ukl_lex_val(r, UKL_LEX_TK, top, UKL_LEX_TYPES + 1u)];
    m[UKL_LEX_TYPED] = typed;
    m[UKL_LEX_COUNT] = count;
    // Past the room: what the file needs, and nothing written.
    if (count + 3u > m[UKL_LEX_ROOM] || typed > m[UKL_LEX_ROOM]) {
        m[UKL_LEX_SHORT] = count + 3u;
        return;
    }
    uint32_t n = m[UKL_LEX_LEN];
    r->starts[count] = n;
    r->starts[count + 1u] = n;
    r->starts[count + 2u] = 0u;
}

// Leaf `i` of tree `tr`, its value stored: up while this item is the second child to arrive.
UKL_INLINE void ukl_lex_up(UKL_PRIVATE UklLex* r, uint32_t tr, uint32_t i)
{
    UklLexPool m = r->m;
    uint32_t lv = m[UKL_LEX_CHUNKS], base = 0u;
    for (uint32_t level = 0u; level < 40u; level++) {
        if (lv <= 1u) {
            if (tr == UKL_LEX_TK)
                ukl_lex_root(r, base);
            return;
        }
        uint32_t up = base + lv, p = i >> 1;
        bool two = 2u * p + 1u < lv;
        if (two && !ukl_lex_meet(r, tr, up + p))
            return;
        ukl_lex_compose(r, tr, up + p, base + 2u * p, base + 2u * p + 1u, two);
        base = up;
        lv = (lv + 1u) >> 1;
        i = p;
    }
}

// Leaf `i`'s state of tree `tr` (the values before it, composed), walked down from the top: M's a map, K's at `s`.
UKL_INLINE uint32_t ukl_lex_path(UKL_PRIVATE UklLex* r, uint32_t tr, uint32_t i, UKL_PRIVATE uint32_t* s)
{
    UklLexPool m = r->m;
    uint32_t n = m[UKL_LEX_CHUNKS], lv = n, levels = 0u;
    for (; levels < 40u && lv > 1u; levels++)
        lv = (lv + 1u) >> 1;
    uint32_t map = ukl_lex_identity();
    for (uint32_t k = 0u; k < UKL_LEX_K; k++)
        s[k] = 0u;
    for (uint32_t l = levels; l > 0u; l--) {
        uint32_t below = 0u, size = n;
        for (uint32_t k = 0u; k + 1u < l; k++) {
            below += size;
            size = (size + 1u) >> 1;
        }
        uint32_t c = i >> (l - 1u);
        if (c % 2u == 0u)
            continue;
        if (tr == UKL_LEX_TM)
            map = ukl_lex_then(map, m[ukl_lex_val(r, tr, below + c - 1u, 0u)]);
        else
            for (uint32_t k = 0u; k < UKL_LEX_K; k++)
                s[k] += m[ukl_lex_val(r, tr, below + c - 1u, k)];
    }
    return map;
}

UKL_INLINE uint32_t ukl_lex_entry(UKL_PRIVATE UklLex* r, uint32_t c)
{
    uint32_t s[UKL_LEX_K];
    // The file's start is lane 0.
    return ukl_lex_path(r, UKL_LEX_TM, c, s) & 15u;
}

UKL_INLINE void ukl_lex_item(UKL_PRIVATE UklLex* r, uint32_t ph, uint32_t c)
{
    UklLexPool m = r->m;
    uint32_t n = m[UKL_LEX_LEN], blocks = m[UKL_LEX_BLOCKS], from = c * blocks, to = from + blocks;
    if (ph == UKL_LEX_PH_ENDS) {
        uint32_t ends[UKL_LEX_LANES];
        ukl_lex_ends(r->src, n, from, to, ends);
        uint32_t map = 0u;
        for (uint32_t l = 0u; l < UKL_LEX_LANES; l++)
            map |= ends[l] << (4u * l);
        m[ukl_lex_val(r, UKL_LEX_TM, c, 0u)] = map;
        ukl_lex_up(r, UKL_LEX_TM, c);
    } else if (ph == UKL_LEX_PH_COUNT) {
        uint32_t lane = ukl_lex_entry(r, c), at[UKL_LEX_K];
        m[m[UKL_LEX_OE] + c] = lane;
        for (uint32_t k = 0u; k < UKL_LEX_K; k++)
            at[k] = 0u;
        uint32_t none[4] = {0u, 0u, 0u, 0u};
        if (ukl_lex_from(r->src, n, from, to, lane, at, r->starts, r->types, false, r->starts, none))
            ukl_atomic_or(&m[UKL_LEX_REFUSED], 1u);
        for (uint32_t k = 0u; k < UKL_LEX_K; k++)
            m[ukl_lex_val(r, UKL_LEX_TK, c, k)] = at[k];
        ukl_lex_up(r, UKL_LEX_TK, c);
    } else {
        uint32_t at[UKL_LEX_K];
        ukl_lex_path(r, UKL_LEX_TK, c, at);
        // Each type's list after the types before it.
        uint32_t before = 0u;
        for (uint32_t k = 1u; k <= UKL_LEX_TYPES; k++) {
            at[k] += before;
            before += m[UKL_LEX_TYPE0 + k - 1u];
        }
        uint32_t first = at[0], mine = m[ukl_lex_val(r, UKL_LEX_TK, c, 0u)];
        // (The line index's words: the device's line phases, decision 0726's step 4; none yet.)
        uint32_t none[4] = {0u, 0u, 0u, 0u};
        ukl_lex_from(r->src, n, from, to, m[m[UKL_LEX_OE] + c], at, r->starts, r->types, true, r->starts, none);
        // Its records' tokens none, until the token pass writes its types'.
        for (uint32_t j = first; j < first + mine; j++) {
            r->tokens[5u * j] = 0u;
            r->tokens[5u * j + 1u] = UKL_LEX_NOT_KIND;
            r->tokens[5u * j + 2u] = 0u;
            r->tokens[5u * j + 3u] = 0u;
            r->tokens[5u * j + 4u] = UKL_LEX_NOT_KIND;
        }
    }
}

// Phase `ph`'s items: the chunks; the writes none where the file is refused or past the records' room.
UKL_INLINE uint32_t ukl_lex_items(UKL_PRIVATE UklLex* r, uint32_t ph)
{
    if (ph == UKL_LEX_PH_WRITE && (ukl_atomic_poll(&r->m[UKL_LEX_REFUSED]) != 0u || ukl_atomic_poll(&r->m[UKL_LEX_SHORT]) != 0u))
        return 0u;
    return r->m[UKL_LEX_CHUNKS];
}

// A worker of phase `r->phase`: a SIMD group's lanes take a group's worth of chunks at a time by ticket, each lane its
// own, until none is left.
UKL_INLINE void ukl_lex_worker(UKL_PRIVATE UklLex* r, uint32_t lane __UKL_PARAMS)
{
    uint32_t ph = r->phase, n = ukl_lex_items(r, ph);
    for (uint32_t turn = 0u; turn < UKL_LEX_TURNS; turn++) {
        uint32_t t0 = 0u;
        if (lane == 0u)
            t0 = ukl_atomic_add(&r->m[UKL_LEX_TICKET + ph], UKL_SIMD_WIDTH);
        t0 = ukl_sg_broadcast(t0, 0u __UKL_ARGS);
        if (t0 >= n)
            break;
        uint32_t c = t0 + lane;
        if (c < n) {
            ukl_fence_device();
            ukl_lex_item(r, ph, c);
        }
    }
}
