// ukl/middle_run.h: tier 2 from the middle as one program (decision 0726): every pass of the chain, from tier 1's
// records to the finished format 16 tape, a phase of one kernel, a launch a phase, so the launches are a fixed eleven
// whatever the file's size or its trees' heights (the user, 2026-11-27: "a launch per phase").
//
// The phases (`UKL_RUN_PH_*`), each its items taken by the launch's SIMD groups by ticket, a group's worth at a time,
// from the count its phase has (set by the phases before it, in the pool: pushing a phase's items is that count):
//
//   RUNS        each run of `size` records: its first record after a separator (`chunk_starts`), a leaf of the runs'
//               scan (P: the count of runs that have one); P's root says the chunks.
//   STARTS      each run's state from P's root (a walk down its path): its chunk's start placed.
//   WORDS       each chunk's bracket word (`Word::of`, a leaf of C, the bracket scan) and its records' change of the
//               depth (a leaf of D, `depths`' scan).
//   KINDS       each chunk's kinds (C's) and depth (D's), each down its path.
//   SPANS       each chunk's one walk (`span`), a leaf of S, the spans' scan; S's root says the file's verdict, nodes
//               and blocks.
//   STATES      each chunk's state (S's, down its path), its levels placed (`levels`: `entries`); and each of the
//               tape's blocks made (`make_blocks`), where the file parses.
//   COUNT       each chunk's side-list entries counted (pass 2, counting);
//   WRITE       each chunk's nodes written (pass 2);
//   END         each block's side lists' counts (`end_middle`), the last to end the state's (`ended`);
//   FIN         each block final (`finalize`'s first half: its presence, its counts), a leaf of B (each side list's
//               laid bytes and the faults, by block); B's root lays the tail;
//   LAY         each block's side lists, faults, part starts and presence words where they go in the tape (B's state
//               down its path), and the newline index, 512 bytes an item.
//
// A scan's tree is the Blelloch tree of `down` and `tree_of` (a level each half the one below): a leaf's item stores
// its value and arrives at its parent; the second child to arrive composes the parent (a node of one child is its
// child) and goes on up, so no item waits for another and the composition is the Rust's. The root stores the state
// after the file and the state at the top. A later phase gives each leaf its state by walking down from the top to it,
// each step the down-sweep's (a left child its parent's state, a right one its parent's after its left sibling's
// value, each kept to what the child reads), so each leaf's state is the down-sweep's, and no level waits for another.
//
// Every value another item reads is in the pool, coherent memory; an item fences before it arrives, and after it
// completes a count, so what it wrote is seen by the item that goes on. The pool's header (`UKL_RUN_H` words) holds the
// counters, the verdict and the layout, which the host writes: where each array and tree lies.
//
// The grammar's parts are the printer's: `ukl_run_bracket`, `ukl_run_sep`, `ukl_run_root`, `ukl_run_cont` and the two
// walks, `ukl_run_span` (pass 1 on `Shadow`) and `ukl_run_write` (pass 2 on the cursor from the middle), declared here.
#pragma once
#include <ukl/middle.h>
#include <ukl/writer_mid.h>
#include <ukl/simd.h>

// ---- The pool's header ------------------------------------------------------------------------------------------

#define UKL_RUN_H 64u
// The counters and the verdict (the host's zeros at the start).
#define UKL_RUN_NC 2u
#define UKL_RUN_NB 3u
// 0 none yet, 1 the file parses, 2 refused (at `UKL_RUN_ERR`), 3 failed (`UKL_RUN_FAIL`: past a room, or the writer's).
#define UKL_RUN_VERDICT 4u
#define UKL_RUN_ERR 5u
#define UKL_RUN_FAIL 6u
#define UKL_RUN_NODES 7u
// Whether every block ended well (`ended`), and the tail laid (B's root).
#define UKL_RUN_ENDED 8u
#define UKL_RUN_G_COUNT 9u
#define UKL_RUN_G_WRITE 10u
#define UKL_RUN_G_END 11u
#define UKL_RUN_G_FIN 12u
#define UKL_RUN_FIN_TARGET 13u
// 1 once the tape is finished.
#define UKL_RUN_DONE 14u
#define UKL_RUN_NLINES 15u
// The layout (the host's): each array's offset in the pool, a word an entry.
#define UKL_RUN_O_FOUND 16u
#define UKL_RUN_O_CHUNKS 17u
#define UKL_RUN_O_READY 18u
#define UKL_RUN_O_DEPTHS 19u
#define UKL_RUN_O_ATS 20u
#define UKL_RUN_O_OUT 21u
// The trees: P (runs), C (words), D (depths), S (spans), B (blocks); each its values (`N`), states (`N + 1`, the last
// the state after the file) and arrivals (`N`).
#define UKL_RUN_O_TREE 22u
#define UKL_RUN_TN 27u
#define UKL_RUN_TBN 28u
// The finish's layout (`UKL_RUN_F_*` words from it), B's root's.
#define UKL_RUN_O_FIN 29u
// The pool's first free word (`m[0]` starts there).
#define UKL_RUN_O_FREE 30u
#define UKL_RUN_TAILED 31u
// Each phase's tickets taken.
#define UKL_RUN_TICKET 32u

#define UKL_RUN_TP 0u
#define UKL_RUN_TC 1u
#define UKL_RUN_TD 2u
#define UKL_RUN_TS 3u
#define UKL_RUN_TB 4u

// The finish's layout: the tail's ten offsets (the header's), the tape's blocks' end, the presence's place, the faults
// before the run, the free list's before, the faults' place in the presence.
#define UKL_RUN_F_TAIL 0u
#define UKL_RUN_F_BLOCKS_END 10u
#define UKL_RUN_F_HOME 11u
#define UKL_RUN_F_ERRORS0 12u
#define UKL_RUN_F_FAULTS 13u
#define UKL_RUN_F_WORDS 14u
#define UKL_RUN_F_COUNT 15u
#define UKL_RUN_F_LAID0 16u
#define UKL_RUN_F_SIZE 17u

// The phases.
#define UKL_RUN_PH_RUNS 0u
#define UKL_RUN_PH_STARTS 1u
#define UKL_RUN_PH_WORDS 2u
#define UKL_RUN_PH_KINDS 3u
#define UKL_RUN_PH_SPANS 4u
#define UKL_RUN_PH_STATES 5u
#define UKL_RUN_PH_COUNT 6u
#define UKL_RUN_PH_WRITE 7u
#define UKL_RUN_PH_END 8u
#define UKL_RUN_PH_FIN 9u
#define UKL_RUN_PH_LAY 10u
#define UKL_RUN_PHASES 11u
#define UKL_RUN_LINE_BYTES 512u
// A worker's turns at most (each a group's worth of items): past every phase's items.
#define UKL_RUN_TURNS 100000000u

typedef struct UklRun {
    UKL_GLOBAL const uint8_t* src;
    UKL_GLOBAL const uint32_t* starts;
    UKL_GLOBAL const uint32_t* toks;
    UklMidPool m;
    // The writer's region: its state at 0.
    UKL_GLOBAL const uint8_t* t;
    // Each chunk's levels' places, `DEPTH` bytes a chunk; pass 1's frames (10 words a record) and writes into its
    // entry's (2 a record); pass 2's frames (25 a record): each a chunk's at its first record and number.
    UKL_GLOBAL uint8_t* rets;
    UKL_GLOBAL uint32_t* f1;
    UKL_GLOBAL uint32_t* touch;
    UKL_GLOBAL uint32_t* f2;
    uint32_t count, len, ntoks, size, runs, most, phase;
} UklRun;

// The grammar's (the two walks each a function compiled once and called, `UKL_ONCE`: inlined into the one kernel with
// the rest, its Metal back end's compile took 37 s on the M3 Max and the M1 Max): a record's bracket by its first byte (its kind twice, plus one where it opens; `UKL_MID_NONE` for
// none), whether a byte is a list's separator, the root's kind, a kind's return place; chunk `c`'s one walk from its
// depth, kinds `k` and word `w` (its span's offset); chunk `c`'s pass 2 from its state `a`, its stop at `out[2c]`.
UKL_INLINE uint32_t ukl_run_bracket(uint32_t b);
UKL_INLINE bool ukl_run_sep(uint32_t b);
UKL_INLINE uint32_t ukl_run_root(void);
UKL_INLINE uint32_t ukl_run_cont(uint32_t k);
UKL_ONCE uint32_t ukl_run_span(UKL_PRIVATE UklRun* r, uint32_t c, uint32_t start, uint32_t stop, uint32_t depth0, uint32_t k, uint32_t w);
UKL_ONCE void ukl_run_write(UKL_PRIVATE UklRun* r, uint32_t c, uint32_t a, uint32_t start, uint32_t stop, bool count);

UKL_INLINE uint32_t ukl_run_at(UKL_PRIVATE UklRun* r, uint32_t field) { return r->m[r->m[field]]; }
UKL_INLINE uint32_t ukl_run_o(UKL_PRIVATE UklRun* r, uint32_t field) { return r->m[field]; }

// The first failure kept: the run's verdict is then a failure.
UKL_INLINE void ukl_run_fail(UKL_PRIVATE UklRun* r, uint32_t code)
{
    ukl_atomic_cas(&r->m[UKL_RUN_FAIL], 0u, code);
    ukl_atomic_store(&r->m[UKL_RUN_VERDICT], 3u);
}

// A counter's arrival: whether it brought it to `target`.
UKL_INLINE bool ukl_run_arrive(UKL_PRIVATE UklRun* r, uint32_t at, uint32_t target)
{
    ukl_fence_device();
    bool last = ukl_atomic_add(&r->m[at], 1u) + 1u == target;
    ukl_fence_device();
    return last;
}

// ---- The scans --------------------------------------------------------------------------------------------------

// Tree `tr`'s composition (0 words, 1 spans, 2 sums, 3 sums of 5), its offset, its stride and its leaves.
UKL_INLINE uint32_t ukl_run_op(uint32_t tr) { return tr == UKL_RUN_TC ? 0u : tr == UKL_RUN_TS ? 1u : tr == UKL_RUN_TB ? 3u : 2u; }
UKL_INLINE uint32_t ukl_run_tree(UKL_PRIVATE UklRun* r, uint32_t tr) { return r->m[UKL_RUN_O_TREE + tr]; }
UKL_INLINE uint32_t ukl_run_stride(UKL_PRIVATE UklRun* r, uint32_t tr) { return r->m[tr == UKL_RUN_TB ? UKL_RUN_TBN : UKL_RUN_TN]; }
UKL_INLINE uint32_t ukl_run_leaves(UKL_PRIVATE UklRun* r, uint32_t tr)
{
    return tr == UKL_RUN_TP ? r->runs : tr == UKL_RUN_TB ? r->m[UKL_RUN_NB] : r->m[UKL_RUN_NC];
}
// Tree `tr`'s value, state and arrival words of node `x` (its state after the file at `x` = N).
UKL_INLINE uint32_t ukl_run_val(UKL_PRIVATE UklRun* r, uint32_t tr, uint32_t x) { return ukl_run_tree(r, tr) + x; }
UKL_INLINE uint32_t ukl_run_st(UKL_PRIVATE UklRun* r, uint32_t tr, uint32_t x) { return ukl_run_tree(r, tr) + ukl_run_stride(r, tr) + x; }
UKL_INLINE uint32_t ukl_run_cnt(UKL_PRIVATE UklRun* r, uint32_t tr, uint32_t x) { return ukl_run_tree(r, tr) + 2u * ukl_run_stride(r, tr) + 1u + x; }

UKL_INLINE uint32_t ukl_run_sum5(UklMidPool m, uint32_t a, uint32_t b)
{
    uint32_t at = ukl_mid_take(m, 5u);
    if (at == UKL_MID_FULL)
        return at;
    for (uint32_t k = 0u; k < 5u; k++)
        m[at + k] = m[a + k] + m[b + k];
    return at;
}

UKL_INLINE uint32_t ukl_run_then(UklMidPool m, uint32_t op, uint32_t a, uint32_t b)
{
    if (op == 2u)
        return a + b;
    if (a >= UKL_MID_FULL || b >= UKL_MID_FULL)
        return UKL_MID_FULL;
    return op == 0u ? ukl_word_then(m, a, b) : op == 1u ? ukl_span_then(m, a, b) : ukl_run_sum5(m, a, b);
}
UKL_INLINE uint32_t ukl_run_after(UklMidPool m, uint32_t op, uint32_t a, uint32_t x)
{
    if (op == 2u)
        return a + x;
    if (x >= UKL_MID_FULL || (op == 3u && a >= UKL_MID_FULL))
        return UKL_MID_FULL;
    return op == 0u ? ukl_kinds_after(m, a, x, ukl_run_root()) : op == 1u ? ukl_at_after(m, a, x) : ukl_run_sum5(m, a, x);
}
UKL_INLINE uint32_t ukl_run_keep(UklMidPool m, uint32_t op, uint32_t a, uint32_t x)
{
    if (op >= 2u)
        return a;
    if (x >= UKL_MID_FULL)
        return a >= UKL_MID_FULL ? a : UKL_MID_FULL;
    return op == 0u ? ukl_kinds_keep(m, a, x) : ukl_at_keep(m, a, x);
}
// The file's start: the root's kinds, the first state, 0, five 0s.
UKL_INLINE uint32_t ukl_run_start(UklMidPool m, uint32_t op)
{
    if (op == 2u)
        return 0u;
    if (op == 1u)
        return ukl_at_new(m, 0u, 0u);
    uint32_t at = ukl_mid_take(m, op == 0u ? 2u : 5u);
    if (at == UKL_MID_FULL)
        return at;
    if (op == 0u) {
        m[at] = 1u;
        m[at + 1u] = ukl_run_root();
    } else {
        for (uint32_t k = 0u; k < 5u; k++)
            m[at + k] = 0u;
    }
    return at;
}

UKL_INLINE void ukl_run_root_done(UKL_PRIVATE UklRun* r, uint32_t tr);

// Leaf `i` of tree `tr`, its value stored: up while this item is the second child to arrive.
UKL_INLINE void ukl_run_up(UKL_PRIVATE UklRun* r, uint32_t tr, uint32_t i)
{
    UklMidPool m = r->m;
    uint32_t op = ukl_run_op(tr), lv = ukl_run_leaves(r, tr), base = 0u;
    for (uint32_t level = 0u; level < 40u; level++) {
        if (lv <= 1u) {
            ukl_run_root_done(r, tr);
            return;
        }
        uint32_t up = base + lv, p = i >> 1;
        bool two = 2u * p + 1u < lv;
        if (two && !ukl_run_arrive(r, ukl_run_cnt(r, tr, up + p), 2u))
            return;
        uint32_t a = m[ukl_run_val(r, tr, base + 2u * p)];
        m[ukl_run_val(r, tr, up + p)] = two ? ukl_run_then(m, op, a, m[ukl_run_val(r, tr, base + 2u * p + 1u)]) : a;
        base = up;
        lv = (lv + 1u) >> 1;
        i = p;
    }
}

UKL_INLINE void ukl_run_leaf(UKL_PRIVATE UklRun* r, uint32_t tr, uint32_t i);

// Leaf `i`'s state, walked down from the top (the root's state) to it as the down-sweep goes, then its own work.
UKL_ONCE void ukl_run_path(UKL_PRIVATE UklRun* r, uint32_t tr, uint32_t i)
{
    UklMidPool m = r->m;
    uint32_t op = ukl_run_op(tr), n = ukl_run_leaves(r, tr), lv = n, levels = 0u, top = 0u;
    for (; levels < 40u && lv > 1u; levels++) {
        top += lv;
        lv = (lv + 1u) >> 1;
    }
    uint32_t s = m[ukl_run_st(r, tr, top)];
    for (uint32_t l = levels; l > 0u; l--) {
        // Level `l - 1`'s first node: the levels below it.
        uint32_t below = 0u, size = n;
        for (uint32_t k = 0u; k + 1u < l; k++) {
            below += size;
            size = (size + 1u) >> 1;
        }
        uint32_t c = i >> (l - 1u);
        if (c % 2u == 1u)
            s = ukl_run_after(m, op, s, m[ukl_run_val(r, tr, below + c - 1u)]);
        s = ukl_run_keep(m, op, s, m[ukl_run_val(r, tr, below + c)]);
    }
    m[ukl_run_st(r, tr, i)] = s;
    ukl_run_leaf(r, tr, i);
}

// Tree `tr`'s root: the state after the file and the top's, its own work, then the down-sweep (unless its work
// failed).
UKL_INLINE bool ukl_run_root_after(UKL_PRIVATE UklRun* r, uint32_t tr);
UKL_INLINE void ukl_run_root_done(UKL_PRIVATE UklRun* r, uint32_t tr)
{
    UklMidPool m = r->m;
    uint32_t op = ukl_run_op(tr), lv = ukl_run_leaves(r, tr), top = 0u;
    for (uint32_t level = 0u; level < 40u && lv > 1u; level++) {
        top += lv;
        lv = (lv + 1u) >> 1;
    }
    uint32_t start = ukl_run_start(m, op), v = m[ukl_run_val(r, tr, top)];
    bool full = op != 2u && start >= UKL_MID_FULL;
    m[ukl_run_st(r, tr, ukl_run_stride(r, tr))] = full ? UKL_MID_FULL : ukl_run_after(m, op, start, v);
    m[ukl_run_st(r, tr, top)] = full ? UKL_MID_FULL : ukl_run_keep(m, op, start, v);
    if (ukl_run_root_after(r, tr) && tr == UKL_RUN_TB)
        ukl_atomic_store(&m[UKL_RUN_TAILED], 1u);
}

// ---- The items --------------------------------------------------------------------------------------------------

UKL_INLINE uint32_t ukl_run_nc(UKL_PRIVATE UklRun* r) { return r->m[UKL_RUN_NC]; }
UKL_INLINE uint32_t ukl_run_chunk(UKL_PRIVATE UklRun* r, uint32_t c) { return r->m[r->m[UKL_RUN_O_CHUNKS] + c]; }
// Chunk `c`'s stop: the next chunk's first record, or the records' end (`count`, the word's and the depth's; `count +
// 1`, the walks', past the file's end).
UKL_INLINE uint32_t ukl_run_stop(UKL_PRIVATE UklRun* r, uint32_t c, uint32_t end)
{
    return c + 1u < ukl_run_nc(r) ? ukl_run_chunk(r, c + 1u) : end;
}

// Run `k` (`chunk_starts`): its first record after a separator, or none; a leaf of P.
UKL_ONCE void ukl_run_run(UKL_PRIVATE UklRun* r, uint32_t k)
{
    uint32_t size = r->size, found = k == 0u ? 0u : UKL_MID_NONE;
    uint32_t end = (k + 1u) * size < r->count ? (k + 1u) * size : r->count;
    for (uint32_t t = k * size; k != 0u && t < end && found == UKL_MID_NONE; t++)
        if (ukl_run_sep(r->src[r->starts[t - 1u]]))
            found = t;
    r->m[r->m[UKL_RUN_O_FOUND] + k] = found;
    r->m[ukl_run_val(r, UKL_RUN_TP, k)] = found != UKL_MID_NONE ? 1u : 0u;
    ukl_run_up(r, UKL_RUN_TP, k);
}

// Chunk `c`'s bracket word and depth change: leaves of C and D.
UKL_ONCE void ukl_run_word(UKL_PRIVATE UklRun* r, uint32_t c)
{
    UklMidPool m = r->m;
    uint32_t start = ukl_run_chunk(r, c), stop = ukl_run_stop(r, c, r->count);
    uint32_t w = ukl_mid_take(m, 3u + stop - start), delta = 0u;
    if (w != UKL_MID_FULL) {
        m[w] = 0u;
        m[w + 1u] = 0u;
        m[w + 2u] = 0u;
    }
    for (uint32_t t = start; t < stop; t++) {
        uint32_t b = ukl_run_bracket(r->src[r->starts[t]]);
        if (b == UKL_MID_NONE)
            continue;
        delta += (b & 1u) != 0u ? 1u : 4294967295u;
        if (w == UKL_MID_FULL)
            continue;
        if ((b & 1u) != 0u) {
            m[w + 3u + m[w] + m[w + 1u]] = b >> 1;
            m[w + 1u] += 1u;
        } else {
            ukl_word_cancel(m, w, b >> 1);
        }
    }
    m[ukl_run_val(r, UKL_RUN_TC, c)] = w;
    m[ukl_run_val(r, UKL_RUN_TD, c)] = delta;
    ukl_run_up(r, UKL_RUN_TC, c);
    ukl_run_up(r, UKL_RUN_TD, c);
}

// Chunk `c`'s walk: a leaf of S.
UKL_ONCE void ukl_run_span_item(UKL_PRIVATE UklRun* r, uint32_t c)
{
    UklMidPool m = r->m;
    uint32_t k = m[ukl_run_st(r, UKL_RUN_TC, c)], w = m[ukl_run_val(r, UKL_RUN_TC, c)];
    uint32_t start = ukl_run_chunk(r, c), stop = ukl_run_stop(r, c, r->count + 1u);
    uint32_t s = k == UKL_MID_FULL || w == UKL_MID_FULL ? UKL_MID_FULL : ukl_run_span(r, c, start, stop, m[m[UKL_RUN_O_DEPTHS] + c], k, w);
    m[ukl_run_val(r, UKL_RUN_TS, c)] = s;
    ukl_run_up(r, UKL_RUN_TS, c);
}

// Chunk `c`'s state with its levels' places (`levels`): `ats[c]`.
UKL_ONCE uint32_t ukl_run_levels(UKL_PRIVATE UklRun* r, uint32_t c, uint32_t a)
{
    UklMidPool m = r->m;
    uint32_t k = m[ukl_run_st(r, UKL_RUN_TC, c)], w = m[ukl_run_val(r, UKL_RUN_TC, c)];
    if (a >= UKL_MID_FULL || m[a + 5u] != 0u || k >= UKL_MID_FULL || w >= UKL_MID_FULL)
        return a;
    uint32_t nk = m[k], anti = m[w], depth = m[a + 2u];
    uint32_t known = anti < nk - 1u ? anti : nk - 1u;
    known = known < depth ? known : depth;
    uint32_t nf = m[a + 10u];
    uint32_t at = ukl_at_new(m, known, nf);
    if (at == UKL_MID_FULL)
        return at;
    for (uint32_t x = 0u; x < 9u; x++)
        m[at + x] = m[a + x];
    for (uint32_t j = 0u; j < known; j++)
        m[at + UKL_AT_HEAD + j] = ukl_run_cont(m[k + 1u + nk - 2u - (known - 1u - j)]);
    for (uint32_t x = 0u; x < UKL_MID_FRAME * nf; x++)
        m[at + UKL_AT_HEAD + known + x] = m[a + UKL_AT_HEAD + m[a + 9u] + x];
    return at;
}

UKL_INLINE uint32_t ukl_run_nodes_blocks(uint32_t n)
{
    uint32_t nb = (n + CLIF_W_Z - 1u) / CLIF_W_Z;
    return nb == 0u ? 1u : nb;
}

// ---- The finish -------------------------------------------------------------------------------------------------

// `len` bytes from `from` to `to` in the region, each byte its own store at the ends (a neighbour's bytes another item
// writes are never read and written back), aligned words between.
UKL_INLINE void ukl_run_copy(UKL_GLOBAL const uint8_t* t, uint64_t to, uint64_t from, uint64_t len)
{
    uint64_t k = 0u;
    for (; k < len && ((to + k) & 7u) != 0u; k++)
        ukl_w_st8(t, to + k, ukl_w_ld8(t, from + k));
    for (; k + 8u <= len; k += 8u)
        ukl_w_st64(t, to + k, ukl_w_ld64(t, from + k));
    for (; k < len; k++)
        ukl_w_st8(t, to + k, ukl_w_ld8(t, from + k));
}
// `v`'s low `w` bytes at `to`, a byte a store.
UKL_INLINE void ukl_run_put_bytes(UKL_GLOBAL const uint8_t* t, uint64_t to, uint64_t v, uint32_t w)
{
    for (uint32_t k = 0u; k < w; k++)
        ukl_w_st8(t, to + k, (v >> (8u * k)) & 0xffu);
}
// Bit `bit` of the presence's words from `word0` at `base`, set by an atomic (blocks set the document's at once).
UKL_INLINE void ukl_run_set_bit(UKL_GLOBAL const uint8_t* t, uint64_t base, uint64_t word0, uint32_t bit)
{
    uint64_t at = base + (((uint64_t)(bit >> 6) + word0) << 3) + 4u * ((bit >> 5) & 1u);
    ukl_atomic_or((UKL_GLOBAL uint32_t*)(t + at), 1u << (bit & 31u));
}

// Block `b`'s rows, its node `j`'s number and its kind field (`finalize`'s), its codes' side list's cursor `low`.
UKL_INLINE uint32_t ukl_run_kind_field(UKL_GLOBAL const uint8_t* t, uint64_t room, UklWList codes, uint32_t j, UKL_PRIVATE uint32_t* low)
{
    uint32_t cut = ukl_w_get32(t, 0u, CLIF_W_AT_CUT), per = ukl_w_get32(t, 0u, CLIF_W_AT_PER);
    uint64_t kinds = ukl_w_part(t, 0u, CLIF_W_PART_KINDS), numbers = ukl_w_len(t, 0u, CLIF_W_PART_KINDS) >> 2;
    uint32_t byte = ukl_w_ld8(t, room + j), number = byte;
    if (byte >= cut) {
        uint32_t rest = 0u;
        if (*low < codes.count) {
            if (ukl_w_ld16(t, codes.js + 2u * (uint64_t)*low) == j)
                rest = (uint32_t)ukl_w_ld64(t, codes.vs + 8u * (uint64_t)*low);
            *low += 1u;
        }
        number = cut + (byte - cut) * per + rest;
    }
    return (uint64_t)number < numbers ? ukl_w_ld32(t, kinds + 4u * (uint64_t)number) : 0u;
}
UKL_INLINE uint32_t ukl_run_rows(UKL_PRIVATE UklRun* r, uint32_t b)
{
    uint32_t n = r->m[UKL_RUN_NODES], rows = n - b * CLIF_W_Z;
    return rows < CLIF_W_Z ? rows : CLIF_W_Z;
}
// Side list `q`'s laid bytes in a block of `rows` (`lay`'s): each sub-block's entries' end, their offsets, values.
UKL_INLINE uint32_t ukl_run_laid_len(UKL_GLOBAL const uint8_t* t, uint64_t area, uint32_t q, uint32_t rows)
{
    UklWList l = ukl_w_list(t, area, q);
    uint32_t subs = (rows + CLIF_W_BLOCK - 1u) >> 8;
    return l.count != 0u ? 2u * subs + l.count + l.count * (uint32_t)ukl_w_get32(t, 0u, CLIF_W_AT_VW + 4u * q) : 0u;
}

// FIN(b): block `b` final, as `finalize` makes it but where its bytes go: its presence (the document's by atomics, its
// own where blocks are kept) and its table entry; its counts a leaf of B: each side list's laid bytes, its faults.
UKL_ONCE void ukl_run_fin(UKL_PRIVATE UklRun* r, uint32_t b)
{
    UKL_GLOBAL const uint8_t* t = r->t;
    uint64_t e = ukl_w_entry(t, 0u, b);
    if (ukl_w_ld32(t, e + 8u) != 0u)
        ukl_run_fail(r, 2u);
    ukl_w_st32(t, e + 8u, 1u);
    uint32_t rows = ukl_run_rows(r, b);
    uint64_t room = ukl_w_room_of(t, 0u, b), area = ukl_w_area_of(t, 0u, b);
    UklWList codes = ukl_w_list(t, area, 0u);
    uint64_t presence = ukl_w_part(t, 0u, CLIF_W_PART_PRESENCE), kw = ukl_w_get32(t, 0u, CLIF_W_AT_KW);
    bool keep = (ukl_w_get32(t, 0u, CLIF_W_AT_FLAGS) & 1u) != 0u;
    uint64_t mine = presence + ((uint64_t)b + 1u) * ukl_w_presence_bytes(t, 0u);
    uint32_t low = 0u, faults = 0u;
    for (uint32_t j = 0u; j < rows; j++) {
        uint32_t kf = ukl_run_kind_field(t, room, codes, j, &low);
        uint32_t kind = kf & 0x7fffu, field = kf >> 16, fault = (kf >> 15) & 1u;
        ukl_run_set_bit(t, presence, 0u, kind);
        ukl_run_set_bit(t, presence, kw, field);
        if (fault != 0u)
            ukl_run_set_bit(t, presence, 0u, 0u);
        if (keep) {
            ukl_w_set_bit(t, mine, 0u, kind);
            ukl_w_set_bit(t, mine, kw, field);
            ukl_w_st64(t, mine, ukl_w_ld64(t, mine) | (uint64_t)fault);
        }
        faults += kind == 0u || fault != 0u ? 1u : 0u;
    }
    // Its side area given back, in block order (`finalize`'s free list).
    uint64_t free0 = ukl_w_len(t, 0u, CLIF_W_PART_FREE);
    ukl_w_st32(t, ukl_w_part(t, 0u, CLIF_W_PART_FREE) + free0 + 4u * (uint64_t)b, ukl_w_ld32(t, e + 4u));
    uint32_t v = ukl_mid_take(r->m, 5u);
    if (v != UKL_MID_FULL) {
        for (uint32_t q = 0u; q < 4u; q++)
            r->m[v + q] = ukl_run_laid_len(t, area, q, rows);
        r->m[v + 4u] = faults;
    }
    r->m[ukl_run_val(r, UKL_RUN_TB, b)] = v;
    ukl_run_up(r, UKL_RUN_TB, b);
}

// B's root (`finish_tail`'s): the state's lengths, the last block's columns down to its rows, the tail laid out and
// its zeros written, the header, the presence's header and the document's words; then each block's pieces (FIN2) and
// the newline index's (LINES) are written where they go.
UKL_ONCE bool ukl_run_tail(UKL_PRIVATE UklRun* r)
{
    UKL_GLOBAL const uint8_t* t = r->t;
    UklMidPool m = r->m;
    uint32_t tot = m[ukl_run_st(r, UKL_RUN_TB, ukl_run_stride(r, UKL_RUN_TB))];
    uint32_t n = m[UKL_RUN_NODES], count = m[UKL_RUN_NB], f = m[UKL_RUN_O_FIN];
    if (tot >= UKL_MID_FULL) {
        ukl_run_fail(r, 3u);
        return false;
    }
    uint64_t laid = (uint64_t)m[tot] + m[tot + 1u] + m[tot + 2u] + m[tot + 3u];
    uint64_t errors0 = ukl_w_len(t, 0u, CLIF_W_PART_ERRORS), nfaults = m[tot + 4u];
    m[f + UKL_RUN_F_ERRORS0] = (uint32_t)errors0;
    if (ukl_w_room(t, 0u, CLIF_W_PART_ERRORS, errors0 + 4u * nfaults) != 0u) {
        ukl_run_fail(r, 4u);
        return false;
    }
    m[f + UKL_RUN_F_LAID0] = (uint32_t)ukl_w_len(t, 0u, CLIF_W_PART_LAID);
    ukl_w_set_len(t, 0u, CLIF_W_PART_ERRORS, errors0 + 4u * nfaults);
    ukl_w_set_len(t, 0u, CLIF_W_PART_FREE, ukl_w_len(t, 0u, CLIF_W_PART_FREE) + 4u * (uint64_t)count);
    ukl_w_set_len(t, 0u, CLIF_W_PART_LAID, ukl_w_len(t, 0u, CLIF_W_PART_LAID) + laid);
    ukl_w_set_len(t, 0u, CLIF_W_PART_LAID_AT, (uint64_t)count * CLIF_W_LAID_AT);
    uint64_t out = ukl_w_part(t, 0u, CLIF_W_PART_OUT);
    // The last block's columns down to its rows.
    uint32_t last = count - 1u;
    uint64_t rows = n - last * CLIF_W_Z, room = ukl_w_room_of(t, 0u, last), before = 1u, blocks_end = CLIF_W_HEAD16;
    for (uint32_t c = 1u; c < 5u; c++) {
        uint64_t len = c == 4u ? ((rows + CLIF_W_K - 1u) >> 3) * ukl_w_get32(t, 0u, CLIF_W_AT_CW) : rows * ukl_w_width(t, 0u, c - 1u);
        uint64_t to = room + rows * before;
        clif_w_copy(t, 0u, to, room + ukl_w_col(t, 0u, c), len);
        if (c == 4u)
            blocks_end = to + len - out;
        else
            before += ukl_w_width(t, 0u, c - 1u);
    }
    uint64_t lines_len = ukl_w_len(t, 0u, CLIF_W_PART_LINES);
    uint64_t kw = ukl_w_get32(t, 0u, CLIF_W_AT_KW), fw = ukl_w_get32(t, 0u, CLIF_W_AT_FW);
    bool keep = (ukl_w_get32(t, 0u, CLIF_W_AT_FLAGS) & 1u) != 0u;
    uint64_t pcount = keep ? (uint64_t)count + 1u : 1u;
    uint64_t need = blocks_end + laid + ((uint64_t)count + 1u) * 16u + lines_len + 64u + 16u + 8u * (kw + fw) * pcount + 4u * nfaults;
    if (ukl_w_room(t, 0u, CLIF_W_PART_OUT, need) != 0u) {
        ukl_run_fail(r, 5u);
        return false;
    }
    ukl_w_st64(t, out + blocks_end, 0u);
    uint64_t pos = blocks_end + 8u, tail[10];
    for (uint32_t q = 0u; q < 4u; q++) {
        tail[q] = pos;
        pos += m[tot + q];
        ukl_w_st64(t, out + pos, 0u);
        pos += 8u;
    }
    ukl_w_st64(t, out + pos, 0u);
    pos = (pos + 3u) & ~(uint64_t)3;
    for (uint32_t q = 0u; q < 4u; q++)
        tail[4u + q] = pos + 4u * ((uint64_t)count + 1u) * q;
    pos += 16u * ((uint64_t)count + 1u);
    uint64_t lines_at = pos, end = lines_at + lines_len + 8u;
    ukl_w_st64(t, out + lines_at + lines_len, 0u);
    tail[8] = lines_at;
    tail[9] = end;
    ukl_w_set_len(t, 0u, CLIF_W_PART_OUT, end);
    ukl_w_write_head(t, 0u, out, CLIF_W_MAGIC16, tail);
    for (uint32_t k = 0u; k < 10u; k++)
        m[f + UKL_RUN_F_TAIL + k] = (uint32_t)tail[k];
    m[f + UKL_RUN_F_BLOCKS_END] = (uint32_t)blocks_end;
    // The presence after the tape (`presence::lay_words`): zeros to a multiple of 8, its header, the words, the faults.
    uint64_t home = (end + 7u) & ~(uint64_t)7;
    for (uint64_t k = end; k < home; k++)
        ukl_w_st8(t, out + k, 0u);
    ukl_w_st32(t, out + home, (uint32_t)kw);
    ukl_w_st32(t, out + home + 4u, (uint32_t)fw);
    ukl_w_st32(t, out + home + 8u, (uint32_t)pcount);
    ukl_w_st32(t, out + home + 12u, (uint32_t)nfaults);
    uint64_t presence = ukl_w_part(t, 0u, CLIF_W_PART_PRESENCE);
    for (uint64_t k = 0u; k < kw + fw; k++)
        ukl_w_st64(t, out + home + 16u + 8u * k, ukl_w_ld64(t, presence + 8u * k));
    m[f + UKL_RUN_F_HOME] = (uint32_t)home;
    m[f + UKL_RUN_F_WORDS] = (uint32_t)(kw + fw);
    m[f + UKL_RUN_F_COUNT] = (uint32_t)pcount;
    m[f + UKL_RUN_F_FAULTS] = (uint32_t)(home + 16u + 8u * (kw + fw) * pcount);
    uint32_t nlines = (uint32_t)((lines_len + UKL_RUN_LINE_BYTES - 1u) / UKL_RUN_LINE_BYTES);
    m[UKL_RUN_NLINES] = nlines;
    m[UKL_RUN_FIN_TARGET] = count + nlines;
    return true;
}

// The tape finished: every block's pieces and the newline index written.
UKL_INLINE void ukl_run_finished(UKL_PRIVATE UklRun* r)
{
    if (ukl_run_arrive(r, UKL_RUN_G_FIN, r->m[UKL_RUN_FIN_TARGET]))
        ukl_atomic_store(&r->m[UKL_RUN_DONE], 1u);
}

// FIN2(b), B's leaf: block `b`'s side lists laid where they go in the tape (`lay`'s bytes, a byte a store), their part
// starts, its faults in the ERRORS part and the presence, its own presence words.
UKL_ONCE void ukl_run_fin2(UKL_PRIVATE UklRun* r, uint32_t b, uint32_t before)
{
    UKL_GLOBAL const uint8_t* t = r->t;
    UklMidPool m = r->m;
    uint32_t f = m[UKL_RUN_O_FIN], count = m[UKL_RUN_NB], mine = m[ukl_run_val(r, UKL_RUN_TB, b)];
    if (before >= UKL_MID_FULL || mine >= UKL_MID_FULL) {
        ukl_run_fail(r, 3u);
        ukl_run_finished(r);
        return;
    }
    uint32_t rows = ukl_run_rows(r, b);
    uint64_t out = ukl_w_part(t, 0u, CLIF_W_PART_OUT), area = ukl_w_area_of(t, 0u, b), room = ukl_w_room_of(t, 0u, b);
    uint64_t laid_before = (uint64_t)m[before] + m[before + 1u] + m[before + 2u] + m[before + 3u];
    uint64_t laid_at = ukl_w_part(t, 0u, CLIF_W_PART_LAID_AT) + (uint64_t)b * CLIF_W_LAID_AT;
    uint64_t laid0 = m[f + UKL_RUN_F_LAID0];
    uint32_t subs = (rows + CLIF_W_BLOCK - 1u) >> 8;
    uint64_t in_block = 0u;
    for (uint32_t q = 0u; q < 4u; q++) {
        UklWList l = ukl_w_list(t, area, q);
        uint64_t vw = ukl_w_get32(t, 0u, CLIF_W_AT_VW + 4u * q), len = m[mine + q];
        uint64_t at = out + m[f + UKL_RUN_F_TAIL + q] + m[before + q];
        if (l.count != 0u) {
            uint32_t k = 0u;
            for (uint32_t sub = 0u; sub < subs; sub++) {
                while (k < l.count && ukl_w_ld16(t, l.js + 2u * (uint64_t)k) >> 8 == sub)
                    k++;
                ukl_run_put_bytes(t, at + 2u * (uint64_t)sub, k, 2u);
            }
            uint64_t offsets = at + 2u * (uint64_t)subs;
            for (uint32_t j = 0u; j < l.count; j++)
                ukl_w_st8(t, offsets + j, ukl_w_ld16(t, l.js + 2u * (uint64_t)j) & 0xffu);
            uint64_t values = offsets + l.count;
            for (uint32_t j = 0u; j < l.count; j++)
                ukl_run_put_bytes(t, values + (uint64_t)j * vw, ukl_w_ld64(t, l.vs + 8u * (uint64_t)j), (uint32_t)vw);
        }
        // Its laid part's entry (the laid bytes in block order, each block's lists in turn), and its part start.
        ukl_w_st32(t, laid_at + 8u * q, (uint32_t)(laid0 + laid_before + in_block));
        ukl_w_st32(t, laid_at + 8u * q + 4u, (uint32_t)len);
        in_block += len;
        uint64_t starts = out + m[f + UKL_RUN_F_TAIL + 4u + q];
        ukl_w_st32(t, starts + 4u * (uint64_t)b, m[before + q]);
        if (b + 1u == count)
            ukl_w_st32(t, starts + 4u * (uint64_t)count, m[before + q] + (uint32_t)len);
    }
    // Its faults, in row order after the blocks' before it: in the ERRORS part and the presence.
    UklWList codes = ukl_w_list(t, area, 0u);
    uint64_t errors = ukl_w_part(t, 0u, CLIF_W_PART_ERRORS) + m[f + UKL_RUN_F_ERRORS0] + 4u * (uint64_t)m[before + 4u];
    uint64_t faults = out + m[f + UKL_RUN_F_FAULTS] + 4u * (uint64_t)m[before + 4u];
    uint32_t low = 0u, k = 0u;
    for (uint32_t j = 0u; j < rows; j++) {
        uint32_t kf = ukl_run_kind_field(t, room, codes, j, &low);
        if ((kf & 0x7fffu) == 0u || ((kf >> 15) & 1u) != 0u) {
            ukl_w_st32(t, errors + 4u * (uint64_t)k, b * CLIF_W_Z + j);
            ukl_w_st32(t, faults + 4u * (uint64_t)k, b * CLIF_W_Z + j);
            k++;
        }
    }
    // Its own presence words, where blocks are kept.
    uint32_t words = m[f + UKL_RUN_F_WORDS];
    if (m[f + UKL_RUN_F_COUNT] > 1u) {
        uint64_t from = ukl_w_part(t, 0u, CLIF_W_PART_PRESENCE) + ((uint64_t)b + 1u) * 8u * words;
        uint64_t to = out + m[f + UKL_RUN_F_HOME] + 16u + ((uint64_t)b + 1u) * 8u * words;
        for (uint32_t w = 0u; w < words; w++)
            ukl_w_st64(t, to + 8u * w, ukl_w_ld64(t, from + 8u * w));
    }
    ukl_run_finished(r);
}

// LINES(k): bytes `512k..` of the newline index into the tail.
UKL_ONCE void ukl_run_lines(UKL_PRIVATE UklRun* r, uint32_t k)
{
    UKL_GLOBAL const uint8_t* t = r->t;
    uint64_t len = ukl_w_len(t, 0u, CLIF_W_PART_LINES), at = (uint64_t)k * UKL_RUN_LINE_BYTES;
    uint64_t n = len - at < UKL_RUN_LINE_BYTES ? len - at : UKL_RUN_LINE_BYTES;
    uint64_t out = ukl_w_part(t, 0u, CLIF_W_PART_OUT) + r->m[r->m[UKL_RUN_O_FIN] + UKL_RUN_F_TAIL + 8u];
    ukl_run_copy(t, out + at, ukl_w_part(t, 0u, CLIF_W_PART_LINES) + at, n);
    ukl_run_finished(r);
}

// Every block ended (`end_middle`): `finish_blocks`' checks and the last node's slot; then each block final.
UKL_ONCE void ukl_run_ended(UKL_PRIVATE UklRun* r)
{
    UKL_GLOBAL const uint8_t* t = r->t;
    UklMidPool m = r->m;
    uint32_t end = m[ukl_run_st(r, UKL_RUN_TS, ukl_run_stride(r, UKL_RUN_TS))], n = m[UKL_RUN_NODES];
    ukl_wmid_ended(t, 0u, n, (uint64_t)m[end + 7u] | ((uint64_t)m[end + 8u] << 32));
    if (ukl_w_get32(t, 0u, CLIF_W_AT_BAD) != 0u || ukl_w_get32(t, 0u, CLIF_W_AT_OPEN_LEN) != 0u || ukl_w_is_z(t, 0u)) {
        ukl_run_fail(r, 6u);
        return;
    }
    if (n != 0u && clif_w_put(t, 0u, 0u, n - 1u, 1u) != 0u) {
        ukl_run_fail(r, 7u);
        return;
    }
    // The rooms the blocks write into at once: the free list (a side area a block, in block order), the laid parts'.
    uint32_t nb = m[UKL_RUN_NB];
    if (ukl_w_room(t, 0u, CLIF_W_PART_FREE, ukl_w_len(t, 0u, CLIF_W_PART_FREE) + 4u * (uint64_t)nb) != 0u
        || ukl_w_room(t, 0u, CLIF_W_PART_LAID_AT, (uint64_t)nb * CLIF_W_LAID_AT) != 0u) {
        ukl_run_fail(r, 4u);
        return;
    }
    ukl_atomic_store(&m[UKL_RUN_ENDED], 1u);
}

// ---- The roots' and the leaves' own work ------------------------------------------------------------------------

UKL_INLINE bool ukl_run_root_after(UKL_PRIVATE UklRun* r, uint32_t tr)
{
    UklMidPool m = r->m;
    uint32_t end = m[ukl_run_st(r, tr, ukl_run_stride(r, tr))];
    if (tr == UKL_RUN_TP) {
        // The chunks: run 0 has one, so at least one. The last has no next start to wait for.
        m[UKL_RUN_NC] = end;
    } else if (tr == UKL_RUN_TS) {
        uint32_t nc = ukl_run_nc(r);
        m[m[UKL_RUN_O_ATS] + nc] = end;
        uint32_t verdict = 3u, nb = 0u;
        if (end < UKL_MID_FULL && m[end + 5u] == 1u) {
            nb = ukl_run_nodes_blocks(m[end + 4u]);
            verdict = nb <= r->most ? 1u : 3u;
        } else if (end < UKL_MID_FULL) {
            verdict = 2u;
            m[UKL_RUN_ERR] = m[end + 5u] == 2u ? m[end + 6u] : 0u;
        }
        m[UKL_RUN_NODES] = end < UKL_MID_FULL ? m[end + 4u] : 0u;
        m[UKL_RUN_NB] = verdict == 1u ? nb : 0u;
        if (verdict == 3u)
            ukl_run_fail(r, 8u);
        else
            ukl_atomic_cas(&m[UKL_RUN_VERDICT], 0u, verdict);
    } else if (tr == UKL_RUN_TB) {
        return ukl_run_tail(r);
    }
    return true;
}

UKL_INLINE void ukl_run_leaf(UKL_PRIVATE UklRun* r, uint32_t tr, uint32_t i)
{
    UklMidPool m = r->m;
    uint32_t st = m[ukl_run_st(r, tr, i)];
    if (tr == UKL_RUN_TP) {
        uint32_t found = m[m[UKL_RUN_O_FOUND] + i];
        if (found == UKL_MID_NONE)
            return;
        m[m[UKL_RUN_O_CHUNKS] + st] = found;
    } else if (tr == UKL_RUN_TC) {
    } else if (tr == UKL_RUN_TD) {
        m[m[UKL_RUN_O_DEPTHS] + i] = i == 0u || (int32_t)st < 0 ? 0u : st;
    } else if (tr == UKL_RUN_TS) {
        m[m[UKL_RUN_O_ATS] + i] = ukl_run_levels(r, i, st);
    } else {
        ukl_run_fin2(r, i, st);
    }
}

// Phase `ph`'s items: its count, from the phases before it; none where the file did not parse or a phase failed.
UKL_INLINE uint32_t ukl_run_items(UKL_PRIVATE UklRun* r, uint32_t ph)
{
    UklMidPool m = r->m;
    bool ok = ukl_atomic_poll(&m[UKL_RUN_VERDICT]) == 1u;
    uint32_t nc = m[UKL_RUN_NC], nb = ok ? m[UKL_RUN_NB] : 0u;
    if (ph <= UKL_RUN_PH_STARTS)
        return r->runs;
    if (ph <= UKL_RUN_PH_SPANS)
        return nc;
    if (ph == UKL_RUN_PH_STATES)
        return nc + nb;
    if (ph <= UKL_RUN_PH_WRITE)
        return ok ? nc : 0u;
    if (ph == UKL_RUN_PH_END)
        return nb;
    if (ph == UKL_RUN_PH_FIN)
        return ukl_atomic_poll(&m[UKL_RUN_ENDED]) == 1u ? nb : 0u;
    return ukl_atomic_poll(&m[UKL_RUN_TAILED]) == 1u ? nb + m[UKL_RUN_NLINES] : 0u;
}

// Whether phase `ph` is `x`, one of a kernel's phases `phases` (a mask its kernel gives as a constant, so the code of
// the phases it does not run is not in it: each walk's kernel compiles apart).
#define UKL_RUN_IS(ph, phases, x) ((ph) == (x) && (((phases) >> (x)) & 1u) != 0u)

// Item `i` of phase `ph`.
UKL_INLINE void ukl_run_item(UKL_PRIVATE UklRun* r, uint32_t ph, uint32_t i, uint32_t phases)
{
    UklMidPool m = r->m;
    uint32_t nc = m[UKL_RUN_NC], nb = m[UKL_RUN_NB];
    if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_RUNS)) {
        ukl_run_run(r, i);
    } else if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_STARTS)) {
        ukl_run_path(r, UKL_RUN_TP, i);
    } else if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_WORDS)) {
        ukl_run_word(r, i);
    } else if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_KINDS)) {
        ukl_run_path(r, UKL_RUN_TC, i);
        ukl_run_path(r, UKL_RUN_TD, i);
    } else if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_SPANS)) {
        ukl_run_span_item(r, i);
    } else if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_STATES) && i < nc) {
        ukl_run_path(r, UKL_RUN_TS, i);
    } else if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_STATES)) {
        UKL_GLOBAL const uint8_t* t = r->t;
        if (ukl_wmid_make_block(t, 0u, i - nc) != 0u)
            ukl_run_fail(r, 9u);
        if (i == nc && ukl_wmid_made(t, 0u, nb) != 0u)
            ukl_run_fail(r, 10u);
    } else if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_COUNT) || UKL_RUN_IS(ph, phases, UKL_RUN_PH_WRITE)) {
        uint32_t a = m[m[UKL_RUN_O_ATS] + i];
        ukl_run_write(r, i, a, ukl_run_chunk(r, i), ukl_run_stop(r, i, r->count + 1u), ph == UKL_RUN_PH_COUNT);
    } else if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_END)) {
        ukl_wmid_end_block(r->t, 0u, i);
        if (ukl_run_arrive(r, UKL_RUN_G_END, nb))
            ukl_run_ended(r);
    } else if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_FIN)) {
        ukl_run_fin(r, i);
    } else if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_LAY) && i < nb) {
        ukl_run_path(r, UKL_RUN_TB, i);
    } else if (UKL_RUN_IS(ph, phases, UKL_RUN_PH_LAY)) {
        ukl_run_lines(r, i - nb);
    }
}

// A worker of phase `r->phase`, one of its kernel's phases `phases`: a SIMD group's lanes take a group's worth of the
// phase's items at a time by ticket, each lane its own, until none is left.
UKL_INLINE void ukl_run_worker(UKL_PRIVATE UklRun* r, uint32_t phases, uint32_t lane __UKL_PARAMS)
{
    uint32_t ph = r->phase, n = ukl_run_items(r, ph);
    for (uint32_t turn = 0u; turn < UKL_RUN_TURNS; turn++) {
        uint32_t t0 = 0u;
        if (lane == 0u)
            t0 = ukl_atomic_add(&r->m[UKL_RUN_TICKET + ph], UKL_SIMD_WIDTH);
        t0 = ukl_sg_broadcast(t0, 0u __UKL_ARGS);
        if (t0 >= n)
            break;
        uint32_t i = t0 + lane;
        if (i < n) {
            ukl_fence_device();
            ukl_run_item(r, ph, i, phases);
        }
    }
}
