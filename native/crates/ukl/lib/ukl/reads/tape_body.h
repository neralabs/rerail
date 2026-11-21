// ukl/reads/tape_body.h: the tape's reads for kernels, written by hand (decisions 0310, 0353 step 4b, 0518): every
// node read of a tape of format 15 or 16 and, where `<ukl/reads/tape_z.h>` includes it, of a 16z tape too (a block
// decoded into the rolling cache the view names, `rerail_clif::view::cache`, and read there as format 16).
//
// A kernel includes `<ukl/reads/tape.h>` or `<ukl/reads/tape_z.h>`, never this file. The host's reads are the CLIF of
// `rerail_clif::reads` (its Rust and its JIT); these are the same functions for the kernels, and a change is made in
// both. `the_device_reads_read_every_node_as_the_crate_does` (rerail-tape-format) reads every node of a tape of each
// format through both headers on the device against the host's reads, and `the_tape_headers_constants_are_the_views`
// (rerail-clif) holds the constants below to `view.rs`'s.
//
// Every read takes the tape's base `t` (each offset counts from it: the view's, and the cache's) and its view `v`.
// Each read below is the CLIF read of its name, and keeps its defences (decision 0463): a size is at least 1, a hop
// moves on and stays in the tape, a side list's offsets stay in its part, a span stays in the source, and a 16z read
// of a block another thread is writing reads the dead entry's zeros.

#pragma once
#include <ukl/memory.h>
#include <ukl/clif.h>

// ---- the view's fields (`rerail_clif::view::at`, byte offsets in `TapeView`) ----------------------------------------
#define CLIF_T_NODES 4u
#define CLIF_T_FORMAT 12u
#define CLIF_T_WIDTHS 16u
#define CLIF_T_COLUMNS 20u
#define CLIF_T_STRIDE 40u
#define CLIF_T_SIDE 44u
#define CLIF_T_PARTS 60u
#define CLIF_T_CUT 76u
#define CLIF_T_PER 80u
#define CLIF_T_SOURCE_LEN 92u
#define CLIF_T_LINES 96u
#define CLIF_T_LINE_COUNT 100u
#define CLIF_T_PRESENCE 120u

// ---- the formats (`view::F16_*`, `F15_*`; `rerail_tape_format::newlines`) ---------------------------------------
#define CLIF_T_NONE 4294967295u
#define CLIF_T_F16Z_FORMAT 1016u
// Format 16: a checkpoint every K nodes, side lists in parts of PART nodes, of SUB blocks of BLOCK nodes.
#define CLIF_T_K 8u
#define CLIF_T_BLOCK 256u
#define CLIF_T_PART 1024u
#define CLIF_T_SUB 4u
// Format 15: a checkpoint every 16 nodes; tails measured from the node's own start.
#define CLIF_T_F15_STRIDE_SHIFT 4u
#define CLIF_T_F15_TAIL_FROM_START 4096u
// Format 16's newline index: its first fields' bytes; every 256th zero and one of its high bits sampled.
#define CLIF_T_LINES_HEAD 20u
#define CLIF_T_LINES_SAMPLE_SHIFT 8u

// ---- 16z (`view::F16Z_*`, `view::z`): a block of Z_BLOCK nodes in Z_PIECES pieces, decoded into a cache entry --------
#define CLIF_T_Z_BLOCK 1024u
#define CLIF_T_Z_PIECES 4u
// In an entry: the block's view at SLOT_VIEW, its bytes from SLOT_BYTES, and SLACK bytes of room after them.
#define CLIF_T_SLOT_VIEW 16u
#define CLIF_T_SLOT_BYTES 144u
#define CLIF_T_SLACK 64u
// A piece's kind: its bytes, one LZ4 block, or a repeat of the piece its payload names.
#define CLIF_T_RAW 0u
#define CLIF_T_LZ4 1u
#define CLIF_T_REPEAT 2u

// ---- the rolling cache (`view::cache`) ----------------------------------------------------------------------------
// Its header's words.
#define CLIF_T_C_EPOCH 0u
#define CLIF_T_C_GENS 4u
#define CLIF_T_C_ENTRIES 8u
#define CLIF_T_C_MAX 12u
#define CLIF_T_C_SLOTS 16u
#define CLIF_T_C_GEN_STRIDE 20u
#define CLIF_T_C_FIRST_ENTRY 24u
#define CLIF_T_C_DEAD 28u
#define CLIF_T_C_FAILED 32u
#define CLIF_T_C_MISSES 36u
#define CLIF_T_C_PROMOTED 40u
#define CLIF_T_C_ROTATED 44u
#define CLIF_T_C_STATS 48u
#define CLIF_T_C_HITS 52u
#define CLIF_T_C_WAITED 56u
#define CLIF_T_C_DUPLICATES 60u
#define CLIF_T_C_HEAD 64u
// A generation's words, then its index (u32 slots) and its entries' offsets (u32s) from INDEX.
#define CLIF_T_G_STAMP 0u
#define CLIF_T_G_USED 4u
#define CLIF_T_G_BUMP 8u
#define CLIF_T_G_WRITERS 12u
#define CLIF_T_G_INDEX 16u
// An entry's words, then its payload.
#define CLIF_T_E_KEY 0u
#define CLIF_T_E_READY 8u
#define CLIF_T_E_LEN 12u
#define CLIF_T_E_PAYLOAD 16u
#define CLIF_T_C_ALIGN 64u
#define CLIF_T_C_CLEARING 4294967295u
#define CLIF_T_C_WRITING 2147483648u
#define CLIF_T_C_PROBES 16u
#define CLIF_T_C_TRIES 4u
#define CLIF_T_C_SPIN 65536u
// A slot: the key's tag above TAG_SHIFT, its entry's number plus one below (0: pending, no entry yet).
#define CLIF_T_C_TAG_SHIFT 16u
#define CLIF_T_C_NUMBER 65535u

#define CLIF_T_ARGS UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v

// ---- the reads every format's bodies call back into (defined at the end) --------------------------------------------
UKL_COLD uint64_t clif_tape_f16_side(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t k, uint32_t i);
UKL_INLINE uint32_t clif_tape_size(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i);
UKL_INLINE uint32_t clif_tape_start_stated(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i);
UKL_INLINE uint32_t clif_tape_end_stated(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i);
UKL_INLINE uint32_t clif_tape_start(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i);
UKL_INLINE uint64_t clif_tape_slot(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i);

// ---- where a format 16 body reads -----------------------------------------------------------------------------------

// The tape in place (its view `v`, its bytes from `t`), or a 16z block decoded into a cache entry: the entry's view,
// its bytes, and the entry's size less one, which every offset into them is held to, so a read of an entry another
// thread writes again reads nothing outside the cache. `first` is the block's first node: a read that calls out of
// the block (a side list's value, another read of the node) calls the tape's read of node `first + i`.
typedef struct ClifTAt {
    uint32_t cached;
    uint32_t first;
    uint64_t view;
    uint64_t bytes;
    uint64_t mask;
} ClifTAt;

UKL_INLINE ClifTAt clif_t_in_place(void)
{
    ClifTAt r;
    r.cached = 0u;
    r.first = 0u;
    r.view = 0u;
    r.bytes = 0u;
    r.mask = ~(uint64_t)0;
    return r;
}

// The view's u32 at byte `a`.
UKL_INLINE uint32_t clif_t_view(CLIF_T_ARGS, ClifTAt r, uint32_t a)
{
    return r.cached ? ukl_ldc_u32(clif_coherent(t), r.view + a) : clif_view(v, a);
}

// The tape's byte `base + off` (both offsets of the tape, or of the entry's bytes).
UKL_INLINE uint64_t clif_t_at(ClifTAt r, uint32_t base, uint32_t off) { return r.bytes + (((uint64_t)base + (uint64_t)off) & r.mask); }

UKL_INLINE uint32_t clif_t_u8(UKL_GLOBAL const uint8_t* t, ClifTAt r, uint32_t base, uint32_t off)
{
    uint64_t a = clif_t_at(r, base, off);
    return r.cached ? ukl_ldc_u8(clif_coherent(t), a) : ukl_ld_u8(t, a);
}
UKL_INLINE uint32_t clif_t_u16(UKL_GLOBAL const uint8_t* t, ClifTAt r, uint32_t base, uint32_t off)
{
    uint64_t a = clif_t_at(r, base, off);
    return r.cached ? ukl_ldc_u16(clif_coherent(t), a) : ukl_ld_u16(t, a);
}
UKL_INLINE uint32_t clif_t_u32(UKL_GLOBAL const uint8_t* t, ClifTAt r, uint32_t base, uint32_t off)
{
    uint64_t a = clif_t_at(r, base, off);
    return r.cached ? ukl_ldc_u32(clif_coherent(t), a) : ukl_ld_u32(t, a);
}
// Eight bytes, little-endian, at any alignment.
UKL_INLINE uint64_t clif_t_word(UKL_GLOBAL const uint8_t* t, ClifTAt r, uint32_t base, uint32_t off)
{
    uint64_t a = clif_t_at(r, base, off);
    return r.cached ? ukl_ldc_u64(clif_coherent(t), a) : ukl_ld_u64(t, a);
}

// The low `w` bytes' mask (all ones at 8 or more).
UKL_INLINE uint64_t clif_t_mask(uint32_t w) { return w >= 8u ? ~(uint64_t)0 : ((uint64_t)1 << (w * 8u)) - 1u; }

// The width at nibble `k` of the view's widths.
UKL_INLINE uint32_t clif_t_width(CLIF_T_ARGS, ClifTAt r, uint32_t k) { return (clif_t_view(t, v, r, CLIF_T_WIDTHS) >> (4u * k)) & 15u; }

UKL_INLINE uint32_t clif_t_column(CLIF_T_ARGS, ClifTAt r, uint32_t k) { return clif_t_view(t, v, r, CLIF_T_COLUMNS + 4u * k); }

// ---- format 16 (`rerail_tape_format::f16`, decision 0434) -----------------------------------------------------------

// Column `c` (0 codes, 1 slots, 2 gaps, 3 lengths, 4 checkpoints) of block `b`: blocks are PART nodes at the view's
// stride from block 0's start, each its columns end to end at its rows (the last block's are fewer), column `c` at
// `rows * columns[c]`.
UKL_INLINE uint32_t clif_t16_column(CLIF_T_ARGS, ClifTAt r, uint32_t c, uint32_t b)
{
    uint32_t base = b * clif_t_view(t, v, r, CLIF_T_STRIDE) + clif_t_column(t, v, r, 0u);
    if (c == 0u)
        return base;
    uint32_t rows = ukl_min(clif_t_view(t, v, r, CLIF_T_NODES) - (b << 10), CLIF_T_PART);
    return base + rows * clif_t_column(t, v, r, c);
}

// Checkpoint `c`: the start of node `c * K`, in its block's checkpoints.
UKL_INLINE uint64_t clif_t16_checkpoint(CLIF_T_ARGS, ClifTAt r, uint32_t c)
{
    uint32_t cw = clif_t_width(t, v, r, 3u);
    uint32_t checkpoints = clif_t16_column(t, v, r, 4u, c >> 7);
    return clif_t_word(t, r, checkpoints, (c & (CLIF_T_PART / CLIF_T_K - 1u)) * cw) & clif_t_mask(cw);
}

// Value `i` of array `a` (0 slots, 1 gaps, 2 lengths): one word's load, or (all ones: escaped) its side list's.
UKL_INLINE uint64_t clif_t16_get(CLIF_T_ARGS, ClifTAt r, uint32_t a, uint32_t i)
{
    uint32_t w = clif_t_width(t, v, r, a);
    uint32_t array = clif_t16_column(t, v, r, 1u + a, i >> 10);
    uint64_t m = clif_t_mask(w);
    uint64_t x = clif_t_word(t, r, array, (i & (CLIF_T_PART - 1u)) * w) & m;
    return x == m ? clif_tape_f16_side(t, v, a + 1u, r.first + i) : x;
}

// Node `i`'s value in side list `k` (0 codes' low parts, 1 slots, 2 gaps, 3 lengths), the node listed. A part (PART
// nodes) holds, for each of its SUB blocks, the end of its run of offsets (a u16), then the offsets (each the low
// byte of a listed node), then the values; every offset into the part is held to its end, the next part's start.
UKL_INLINE uint64_t clif_t16_side(CLIF_T_ARGS, ClifTAt r, uint32_t k, uint32_t i)
{
    uint32_t side = clif_t_view(t, v, r, CLIF_T_SIDE + 4u * k);
    uint32_t parts = clif_t_view(t, v, r, CLIF_T_PARTS + 4u * k);
    uint32_t p = i >> 10;
    uint32_t start = clif_t_u32(t, r, parts, p * 4u);
    uint32_t limit = clif_t_u32(t, r, parts, p * 4u + 4u);
    uint32_t blocks = (clif_t_view(t, v, r, CLIF_T_NODES) + (CLIF_T_BLOCK - 1u)) >> 8;
    uint32_t nsub = ukl_min(CLIF_T_SUB, blocks - p * CLIF_T_SUB);
    uint32_t j = (i >> 8) & (CLIF_T_SUB - 1u);
    // The block's run of offsets: from the end of the one before (0 for the part's first) to its own end.
    uint32_t lo = j == 0u ? 0u : clif_t_u16(t, r, side, ukl_min(start + 2u * (j - 1u), limit));
    uint32_t hi = clif_t_u16(t, r, side, ukl_min(start + 2u * j, limit));
    uint32_t offsets = start + 2u * nsub;
    uint32_t low = i & (CLIF_T_BLOCK - 1u);
    // The first offset at or past the node's low byte.
    while (lo < hi) {
        uint32_t middle = (lo + hi) >> 1;
        if (clif_t_u8(t, r, side, ukl_min(offsets + middle, limit)) < low)
            lo = middle + 1u;
        else
            hi = middle;
    }
    // The values follow the part's offsets.
    uint32_t vw = (clif_t_view(t, v, r, CLIF_T_WIDTHS) >> ((4u * k + 16u) & 31u)) & 15u;
    uint32_t count = clif_t_u16(t, r, side, ukl_min(start + 2u * (nsub - 1u), limit));
    uint32_t at = ukl_min(offsets + count + lo * vw, limit);
    return clif_t_word(t, r, side, at) & clif_t_mask(vw);
}

// The node's subtree's size: at least 1, whatever the slot holds.
UKL_INLINE uint32_t clif_t16_size(CLIF_T_ARGS, ClifTAt r, uint32_t i)
{
    uint64_t s = clif_t16_get(t, v, r, 0u, i);
    return (s & 1u) != 0u ? 1u : ukl_max((uint32_t)(s >> 1), 1u);
}

// The node's code's number in its language's table: its byte, or past the cutoff, the byte's range and its low part.
UKL_INLINE uint32_t clif_t16_number(CLIF_T_ARGS, ClifTAt r, uint32_t i)
{
    uint32_t byte = clif_t_u8(t, r, clif_t16_column(t, v, r, 0u, i >> 10), i & (CLIF_T_PART - 1u));
    uint32_t cut = clif_t_view(t, v, r, CLIF_T_CUT);
    if (byte < cut)
        return byte;
    uint32_t low = (uint32_t)clif_tape_f16_side(t, v, 0u, r.first + i);
    return cut + (byte - cut) * clif_t_view(t, v, r, CLIF_T_PER) + low;
}

// The node's start as stated: its checkpoint, then the gaps of the nodes after it, to the node's own.
UKL_INLINE uint32_t clif_t16_start_stated(CLIF_T_ARGS, ClifTAt r, uint32_t i)
{
    uint32_t c = i >> 3;
    uint64_t from = clif_t16_checkpoint(t, v, r, c);
    uint32_t first = c * CLIF_T_K;
    uint32_t n = i - first;
    if (n == 0u)
        return (uint32_t)from;
    // At a byte's width, the gaps of nodes `first + 1 ..= i` are bytes 1 to `n` of the word at the checkpoint's node:
    // summed by folding the word, unless one is escaped (all ones: its side list's).
    if (clif_t_width(t, v, r, 1u) == 1u) {
        uint64_t x = clif_t_word(t, r, clif_t16_column(t, v, r, 2u, first >> 10), first & (CLIF_T_PART - 1u));
        uint64_t m = (~(uint64_t)0 >> ((7u - n) * 8u)) & ~(uint64_t)0xff;
        x &= m;
        uint64_t nx = ~x;
        uint64_t ff = ~(((nx & 0x7f7f7f7f7f7f7f7ful) + 0x7f7f7f7f7f7f7f7ful) | nx) & 0x8080808080808080ul;
        if ((ff & m) == 0u) {
            x = (x & 0x00ff00ff00ff00fful) + ((x >> 8) & 0x00ff00ff00ff00fful);
            x = (x & 0x0000ffff0000fffful) + ((x >> 16) & 0x0000ffff0000fffful);
            x = (x & 0xfffffffful) + (x >> 32);
            return (uint32_t)(from + x);
        }
    }
    // Else gap by gap: at most K - 1 values.
    uint64_t sum = from;
    for (uint32_t j = first + 1u; j <= i; j++)
        sum += clif_t16_get(t, v, r, 1u, j);
    return (uint32_t)sum;
}

// The node's end as stated: its start and its length.
UKL_INLINE uint32_t clif_t16_end_stated(CLIF_T_ARGS, ClifTAt r, uint32_t i)
{
    return clif_tape_start_stated(t, v, r.first + i) + (uint32_t)clif_t16_get(t, v, r, 2u, i);
}

UKL_INLINE uint32_t clif_t16_length(CLIF_T_ARGS, ClifTAt r, uint32_t i) { return (uint32_t)clif_t16_get(t, v, r, 2u, i); }

// The node's start in a scan: the node before's (`prev`) and its gap, or at a checkpoint's node, the checkpoint.
UKL_INLINE uint32_t clif_t16_start_next(CLIF_T_ARGS, ClifTAt r, uint32_t i, uint32_t prev)
{
    if ((i & (CLIF_T_K - 1u)) != 0u)
        return prev + (uint32_t)clif_t16_get(t, v, r, 1u, i);
    return (uint32_t)clif_t16_checkpoint(t, v, r, i >> 3);
}

// The node's parent: the node before is the parent (an inner node), or a leaf naming the previous sibling or, every
// K-th sibling, the parent. Each step moves back, and a step that would not (a slot no tape's) ends the walk.
// `z`: a 16z tape, each slot read in its block (`tape_slot`), back over blocks' starts.
UKL_INLINE uint32_t clif_t16_parent(CLIF_T_ARGS, uint32_t i, uint32_t z)
{
    uint32_t j = i;
    while (j != 0u) {
        uint32_t before = j - 1u;
        uint64_t s = z ? clif_tape_slot(t, v, before) : clif_t16_get(t, v, clif_t_in_place(), 0u, before);
        if ((s & 1u) == 0u)
            return before;
        uint32_t step = (uint32_t)(s >> 2);
        if (step - 1u >= j)
            return CLIF_T_NONE;
        j -= step;
        if ((s & 2u) != 0u)
            return j;
    }
    return CLIF_T_NONE;
}

// ---- format 15 -----------------------------------------------------------------------------------------------------

// Row `i` of a column of width 2 or 4.
UKL_INLINE uint32_t clif_t15_cell(UKL_GLOBAL const uint8_t* t, uint32_t column, uint32_t width, uint32_t i)
{
    return width == 2u ? clif_t_u16(t, clif_t_in_place(), column, i * 2u) : clif_t_u32(t, clif_t_in_place(), column, i * 4u);
}

UKL_INLINE uint32_t clif_t15_size(CLIF_T_ARGS, uint32_t i)
{
    ClifTAt r = clif_t_in_place();
    return ukl_max(clif_t15_cell(t, clif_t_column(t, v, r, 1u), clif_t_width(t, v, r, 0u), i), 1u);
}

UKL_INLINE uint32_t clif_t15_number(CLIF_T_ARGS, uint32_t i) { return clif_t_u16(t, clif_t_in_place(), clif_view(v, CLIF_T_COLUMNS), i * 2u); }

UKL_INLINE uint32_t clif_t15_start_stated(CLIF_T_ARGS, uint32_t i)
{
    ClifTAt r = clif_t_in_place();
    uint32_t checkpoint = clif_t_u32(t, r, clif_t_column(t, v, r, 2u), (i >> CLIF_T_F15_STRIDE_SHIFT) * 4u);
    return checkpoint + clif_t15_cell(t, clif_t_column(t, v, r, 3u), clif_t_width(t, v, r, 1u), i);
}

// A tail from the node's own start, or from its last descendant's.
UKL_INLINE uint32_t clif_t15_end_stated(CLIF_T_ARGS, uint32_t i)
{
    ClifTAt r = clif_t_in_place();
    uint32_t own = clif_view(v, CLIF_T_WIDTHS) & CLIF_T_F15_TAIL_FROM_START;
    uint32_t from = own != 0u ? i : i + clif_tape_size(t, v, i) - 1u;
    return clif_tape_start_stated(t, v, from) + clif_t15_cell(t, clif_t_column(t, v, r, 4u), clif_t_width(t, v, r, 2u), i);
}

// `c + size`, past `c` however `size` wraps, and no further than the tape's nodes.
UKL_INLINE uint32_t clif_t_hop(CLIF_T_ARGS, uint32_t c, uint32_t size) { return ukl_min(ukl_max(c + size, c + 1u), clif_view(v, CLIF_T_NODES)); }

// The nearest node before whose subtree holds the node, back over at most 64 nodes, else a descent from the root
// over the siblings' subtrees.
UKL_INLINE uint32_t clif_t15_parent(CLIF_T_ARGS, uint32_t i)
{
    uint32_t j = i;
    for (uint32_t k = 0u; j != 0u && k < 64u; k++) {
        j -= 1u;
        if (clif_t_hop(t, v, j, clif_tape_size(t, v, j)) > i)
            return j;
    }
    // At each level, the child whose subtree holds the node.
    uint32_t at = 0u;
    for (;;) {
        uint32_t c = at + 1u;
        uint32_t after = clif_t_hop(t, v, c, clif_tape_size(t, v, c));
        while (after <= i) {
            c = after;
            after = clif_t_hop(t, v, c, clif_tape_size(t, v, c));
        }
        if (c == i)
            return at;
        at = c;
    }
}

UKL_INLINE uint32_t clif_t15_length(CLIF_T_ARGS, uint32_t i) { return clif_tape_end_stated(t, v, i) - clif_tape_start_stated(t, v, i); }

// ---- lines: format 16's newline index (`rerail_tape_format::newlines`), format 15's line starts ---------------------

// The newline index's u32 field `k` (0 the newlines' count, 1 the low parts' width, 2 and 3 the zero and one
// samples' counts, 4 the high bits' words).
UKL_INLINE uint32_t clif_t_lines_field(CLIF_T_ARGS, uint32_t k) { return clif_t_u32(t, clif_t_in_place(), clif_view(v, CLIF_T_LINES), 4u * k); }

// Where the newline index's part at `offset` bytes past its first fields lies.
UKL_INLINE uint32_t clif_t_lines_part(CLIF_T_ARGS, uint32_t offset) { return clif_view(v, CLIF_T_LINES) + offset + CLIF_T_LINES_HEAD; }

// Newline `i`'s low part (`l` bits).
UKL_INLINE uint64_t clif_t_line_low(CLIF_T_ARGS, uint32_t i, uint32_t l)
{
    uint32_t before = (clif_t_lines_field(t, v, 2u) + clif_t_lines_field(t, v, 3u) + clif_t_lines_field(t, v, 4u)) * 8u;
    uint64_t bit = (uint64_t)i * (uint64_t)l;
    uint64_t word = clif_t_word(t, clif_t_in_place(), clif_t_lines_part(t, v, before), (uint32_t)(bit >> 3));
    return (word >> (bit & 7u)) & (((uint64_t)1 << (l & 63u)) - 1u);
}

// The position of the `r`-th set bit of `x` (one is there): six halvings, each a popcount.
UKL_INLINE uint32_t clif_t_select_in_word(uint64_t x, uint32_t r)
{
    uint32_t pos = 0u;
    for (uint32_t s = 32u; s != 0u; s >>= 1) {
        uint32_t c = ukl_popcount64(x & (((uint64_t)1 << s) - 1u));
        if (r >= c) {
            r -= c;
            x >>= s;
            pos += s;
        }
    }
    return pos;
}

// The position in the newline index's high bits of its `j`-th one (`ones` 1) or zero (0): from the sample before
// it, a word at a time.
UKL_COLD uint32_t clif_tape_line_select(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t ones, uint32_t j)
{
    ClifTAt r = clif_t_in_place();
    uint32_t zs = clif_t_lines_field(t, v, 2u);
    uint32_t os = clif_t_lines_field(t, v, 3u);
    uint32_t sample = clif_t_lines_part(t, v, (ones != 0u ? zs * 8u : 0u) + (j >> CLIF_T_LINES_SAMPLE_SHIFT) * 8u);
    uint32_t w = clif_t_u32(t, r, sample, 0u);
    uint32_t before = clif_t_u32(t, r, sample, 4u);
    uint32_t high = clif_t_lines_part(t, v, (zs + os) * 8u);
    for (;;) {
        uint64_t word = clif_t_word(t, r, high, w * 8u);
        uint64_t x = ones != 0u ? word : ~word;
        uint32_t through = before + ukl_popcount64(x);
        if (through > j)
            return w * 64u + clif_t_select_in_word(x, j - before);
        w += 1u;
        before = through;
    }
}

// The line byte `x` lies in (from 0): the newlines before it, by the index (the zeros before and after its bucket,
// then a search of the bucket's low parts).
UKL_INLINE uint32_t clif_t16_line(CLIF_T_ARGS, uint32_t x)
{
    if (clif_t_lines_field(t, v, 0u) == 0u)
        return 0u;
    uint32_t l = clif_t_lines_field(t, v, 1u);
    uint32_t h = x >> (l & 31u);
    uint64_t xl = x & ((1u << (l & 31u)) - 1u);
    uint32_t lo = h == 0u ? 0u : clif_tape_line_select(t, v, 0u, h - 1u) - (h - 1u);
    uint32_t hi = clif_tape_line_select(t, v, 0u, h) - h;
    // The bucket's first newline whose low part is at or past `x`'s.
    while (lo < hi) {
        uint32_t middle = (lo + hi) >> 1;
        if (clif_t_line_low(t, v, middle, l) < xl)
            lo = middle + 1u;
        else
            hi = middle;
    }
    return lo;
}

// The last line start at or before `x`.
UKL_INLINE uint32_t clif_t15_line(CLIF_T_ARGS, uint32_t x)
{
    uint32_t lines = clif_view(v, CLIF_T_LINES);
    uint32_t lo = 1u, hi = clif_view(v, CLIF_T_LINE_COUNT);
    while (lo < hi) {
        uint32_t middle = (lo + hi) >> 1;
        if (clif_t_u32(t, clif_t_in_place(), lines, middle * 4u) > x)
            hi = middle;
        else
            lo = middle + 1u;
    }
    return lo - 1u;
}

// Line `k`'s first byte: 0 for the first; else one past newline `k - 1` (its bucket, from its one's position, and
// its low part).
UKL_INLINE uint32_t clif_t16_line_start(CLIF_T_ARGS, uint32_t k)
{
    if (k == 0u)
        return 0u;
    uint32_t i = k - 1u;
    uint32_t bucket = clif_tape_line_select(t, v, 1u, i) - i;
    uint32_t l = clif_t_lines_field(t, v, 1u);
    return ((bucket << (l & 31u)) | (uint32_t)clif_t_line_low(t, v, i, l)) + 1u;
}

// ---- 16z: a block decoded into the rolling cache the view names (decision 0353 step 4b) ----------------------------
#ifdef CLIF_READS_TAPE_Z

// The u64 of the view's two u32s at `a`, the low first.
UKL_INLINE uint64_t clif_t_wide(CLIF_VIEW_SPACE const uint32_t* v, uint32_t a) { return (uint64_t)clif_view(v, a) | ((uint64_t)clif_view(v, a + 4u) << 32); }

// The cache's address (its offset from the tape's base, `side[0..2]`) and a block's key (the tape's id, `side[2..4]`,
// above the block).
UKL_INLINE uint64_t clif_tz_cache(CLIF_VIEW_SPACE const uint32_t* v) { return clif_t_wide(v, CLIF_T_SIDE); }
UKL_INLINE uint64_t clif_tz_key(CLIF_VIEW_SPACE const uint32_t* v, uint32_t block) { return (clif_t_wide(v, CLIF_T_SIDE + 8u) << 22) | (uint64_t)block; }

// A cache's u32 word at `a`, read and written by any thread.
UKL_INLINE uint32_t clif_tc_load(UKL_GLOBAL const uint8_t* t, uint64_t a) { return ukl_atomic_load_at(t, a); }
UKL_INLINE void clif_tc_store(UKL_GLOBAL const uint8_t* t, uint64_t a, uint32_t x) { ukl_atomic_store_at(t, a, x); }
UKL_INLINE uint32_t clif_tc_add(UKL_GLOBAL const uint8_t* t, uint64_t a, uint32_t x) { return ukl_atomic_add_at(t, a, x); }
UKL_INLINE uint32_t clif_tc_cas(UKL_GLOBAL const uint8_t* t, uint64_t a, uint32_t e, uint32_t n) { return ukl_atomic_cas_at(t, a, e, n); }

// The cache's dead entry: what a read with no entry of its own reads (its zeros).
UKL_INLINE uint64_t clif_tc_dead(UKL_GLOBAL const uint8_t* t, uint64_t cache) { return cache + clif_tc_load(t, cache + CLIF_T_C_DEAD); }

// The statistics mode's count at `a`, where the cache keeps them.
UKL_INLINE void clif_tc_stat(UKL_GLOBAL const uint8_t* t, uint64_t cache, uint32_t a)
{
    if (clif_tc_load(t, cache + CLIF_T_C_STATS) != 0u)
        clif_tc_add(t, cache + a, 1u);
}

// A cache's shape, from its header.
typedef struct ClifTCache {
    uint64_t at;
    uint32_t gens, max, slots, stride, first, entries;
} ClifTCache;

UKL_INLINE ClifTCache clif_tc_open(UKL_GLOBAL const uint8_t* t, uint64_t cache)
{
    ClifTCache c;
    c.at = cache;
    c.gens = clif_tc_load(t, cache + CLIF_T_C_GENS);
    c.max = clif_tc_load(t, cache + CLIF_T_C_MAX);
    c.slots = clif_tc_load(t, cache + CLIF_T_C_SLOTS);
    c.stride = clif_tc_load(t, cache + CLIF_T_C_GEN_STRIDE);
    c.first = clif_tc_load(t, cache + CLIF_T_C_FIRST_ENTRY);
    c.entries = clif_tc_load(t, cache + CLIF_T_C_ENTRIES);
    return c;
}

// The generation of epoch `e`.
UKL_INLINE uint64_t clif_tc_gen(ClifTCache c, uint32_t e) { return c.at + CLIF_T_C_HEAD + (uint64_t)((e % c.gens) * c.stride); }

// Where entry `n` of a generation's index's offsets is written, and the entry.
UKL_INLINE uint64_t clif_tc_offset_at(ClifTCache c, uint64_t gen, uint32_t n) { return gen + ((uint64_t)n << 2) + ((uint64_t)c.slots << 2) + CLIF_T_G_INDEX; }
UKL_INLINE uint64_t clif_tc_entry(UKL_GLOBAL const uint8_t* t, ClifTCache c, uint64_t gen, uint32_t n)
{
    return gen + (uint64_t)clif_tc_load(t, clif_tc_offset_at(c, gen, n)) + (uint64_t)c.first;
}

// Slot `p` of a key's probes in a generation's index (its hash `h`).
UKL_INLINE uint64_t clif_tc_slot(ClifTCache c, uint64_t gen, uint32_t h, uint32_t p) { return gen + (uint64_t)(((h + p) & (c.slots - 1u)) * 4u) + CLIF_T_G_INDEX; }

// Generation `gen` (unstamped) cleared and stamped `stamp` as a spare, when no writer is in it: one thread clears it
// (its stamp's compare-and-swap from 0 to CLEARING). With a writer still in it, it stays unstamped, and a later full
// head clears it.
UKL_INLINE void clif_tc_clear(UKL_GLOBAL const uint8_t* t, ClifTCache c, uint64_t gen, uint32_t stamp)
{
    if (clif_tc_load(t, gen + CLIF_T_G_WRITERS) != 0u)
        return;
    if (clif_tc_cas(t, gen + CLIF_T_G_STAMP, 0u, CLIF_T_C_CLEARING) != 0u)
        return;
    for (uint32_t j = 0u; j < c.slots; j++)
        clif_tc_store(t, gen + (uint64_t)(j * 4u) + CLIF_T_G_INDEX, 0u);
    clif_tc_store(t, gen + CLIF_T_G_USED, 0u);
    clif_tc_store(t, gen + CLIF_T_G_BUMP, 0u);
    ukl_fence_device();
    clif_tc_store(t, gen + CLIF_T_G_STAMP, stamp);
}

// The head full at `epoch`: the spare (cleared ahead, stamped `epoch + 1`) turned into the head by the thread whose
// compare-and-swap of the epoch wins, which then clears the next spare (the oldest live generation); a spare left
// uncleared (stamped 0: its writers were not done when its turn came) cleared now, if they are.
UKL_INLINE void clif_tc_full(UKL_GLOBAL const uint8_t* t, ClifTCache c, uint32_t epoch)
{
    uint32_t next = epoch + 1u;
    uint64_t spare = clif_tc_gen(c, next);
    uint32_t ready = clif_tc_load(t, spare + CLIF_T_G_STAMP);
    if (ready != next) {
        if (ready == 0u)
            clif_tc_clear(t, c, spare, next);
        return;
    }
    if (clif_tc_cas(t, c.at + CLIF_T_C_EPOCH, epoch, next) != epoch)
        return;
    clif_tc_add(t, c.at + CLIF_T_C_ROTATED, 1u);
    uint64_t gen = clif_tc_gen(c, epoch + 2u);
    clif_tc_store(t, gen + CLIF_T_G_STAMP, 0u);
    ukl_fence_device();
    clif_tc_clear(t, c, gen, epoch + 2u);
}

// `n` bytes from the tape's `s` to the entry's `d`: 8 a copy, then 4, 2 and 1, so no load passes `s + n` (the last
// block's literals end the tape).
UKL_INLINE void clif_tz_copy(UKL_GLOBAL const uint8_t* t, uint64_t s, uint64_t d, uint64_t n)
{
    uint64_t whole = n & ~(uint64_t)7;
    for (uint64_t k = 0u; k < whole; k += 8u)
        ukl_stc_u64(clif_written(t), d + k, ukl_ld_u64(t, s + k));
    uint64_t k = whole;
    if ((n & 4u) != 0u) {
        ukl_stc_u32(clif_written(t), d + k, ukl_ld_u32(t, s + k));
        k += 4u;
    }
    if ((n & 2u) != 0u) {
        ukl_stc_u16(clif_written(t), d + k, ukl_ld_u16(t, s + k));
        k += 2u;
    }
    if ((n & 1u) != 0u)
        ukl_stc_u8(clif_written(t), d + k, ukl_ld_u8(t, s + k));
}

// An LZ4 length of `nibble`: 15 is followed by bytes added to it, the last under 255; the source moved past them.
UKL_INLINE uint32_t clif_tz_length(UKL_GLOBAL const uint8_t* t, uint32_t nibble, UKL_PRIVATE uint64_t* s, uint64_t src_end)
{
    uint32_t n = nibble;
    if (n != 15u)
        return n;
    while (*s < src_end) {
        uint32_t byte = ukl_ld_u8(t, *s);
        n += byte;
        *s += 1u;
        if (byte != 255u)
            break;
    }
    return n;
}

// LZ4's block format decoded from `src..src_end` to `dst..dst_end` (decision 0353 step 4b): a sequence's literals,
// then its match 8 bytes a copy, an offset under 8 widened first as LZ4's reference decoder widens it (its copies may
// run 8 bytes past `dst_end`, into the entry's room). A sequence that would write past `dst_end` or read before
// `dst`, or literals past the source (a corrupt block), ends the decode. Where it ended in the destination.
UKL_INLINE uint64_t clif_tz_lz4(UKL_GLOBAL const uint8_t* t, uint64_t src, uint64_t src_end, uint64_t dst, uint64_t dst_end)
{
    uint64_t s = src, d = dst;
    while (s < src_end) {
        uint32_t token = ukl_ld_u8(t, s);
        s += 1u;
        uint64_t lit = clif_tz_length(t, token >> 4, &s, src_end);
        uint64_t d_lit = d + lit;
        if (d_lit > dst_end || s + lit > src_end)
            return d;
        clif_tz_copy(t, s, d, lit);
        s += lit;
        // A match's two offset bytes, else the block's last literals ended it.
        if (s + 2u > src_end)
            return d_lit;
        uint64_t off = ukl_ld_u16(t, s);
        s += 2u;
        uint64_t m = (uint64_t)(clif_tz_length(t, token & 15u, &s, src_end) + 4u);
        uint64_t d_end = d_lit + m;
        if (off == 0u || off > d_lit - dst || d_end > dst_end)
            return d_lit;
        uint64_t from = d_lit - off;
        if (off < 8u) {
            // Its first 8 bytes a byte at a time and a word, the source moved on to a multiple of the offset at
            // least 8 back (LZ4's `inc32table` and `dec64table`, a nibble each).
            for (uint32_t j = 0u; j < 4u; j++)
                ukl_stc_u8(clif_written(t), d_lit + j, ukl_ldc_u8(clif_coherent(t), from + j));
            uint32_t shift = (uint32_t)off * 4u;
            from += (0x44401210ul >> shift) & 15u;
            ukl_stc_u32(clif_written(t), d_lit + 4u, ukl_ldc_u32(clif_coherent(t), from));
            from = from + 4u - ((0x76503444ul >> shift) & 15u);
        } else {
            ukl_stc_u64(clif_written(t), d_lit, ukl_ldc_u64(clif_coherent(t), from));
            from += 8u;
        }
        // The rest 8 bytes a copy, at least 8 back.
        for (uint64_t to = d_lit + 8u; to < d_end; to += 8u, from += 8u)
            ukl_stc_u64(clif_written(t), to, ukl_ldc_u64(clif_coherent(t), from));
        d = d_end;
    }
    return d;
}

// Block `block` of the tape decoded into the cache entry `e` (`size` bytes) as a format 16 tape of one block
// (decision 0434 §3's stored form): each of its pieces (codes, slots, gaps, lengths) decoded where its view places
// it, the array's column then its side list's part (an LZ4 piece decoded, a raw one copied, a repeat as the piece it
// names), its checkpoints copied, each side list's one part's starts after them, and its view. Column `q` lies at
// `nb * C[q]` (`nb` its nodes), `C[q]` the least that clears the piece before and 16 bytes of room. Whether it fits
// the entry (1), else 0 and nothing written.
//
// The tape's view names its piece index (`columns[0]`: the pieces' starts in the payload, then their lengths, then
// their kinds, a byte each), its side lists' part starts (`columns[1]`), its checkpoints (`columns[2]`) and its
// payload (`stride`).
UKL_INLINE uint32_t clif_tz_decode(CLIF_T_ARGS, uint32_t block, uint64_t e, uint64_t size)
{
    ClifTAt r = clif_t_in_place();
    uint32_t nodes = clif_view(v, CLIF_T_NODES);
    uint32_t nb = ukl_min(nodes - (block << 10), CLIF_T_Z_BLOCK);
    uint32_t blocks = (nodes + (CLIF_T_Z_BLOCK - 1u)) >> 10;
    // Each side list's part of the block: from its start to the next's (list `q`'s starts at `4 * q * (blocks + 1)`),
    // never past it.
    uint32_t parts = clif_view(v, CLIF_T_COLUMNS + 4u);
    uint32_t side[4];
    for (uint32_t q = 0u; q < 4u; q++) {
        uint32_t at = ((blocks + 1u) * q + block) * 4u;
        uint32_t from = clif_t_u32(t, r, parts, at);
        side[q] = ukl_max(clif_t_u32(t, r, parts, at + 4u), from) - from;
    }
    // The columns' places: column `q` holds `nb * w[q]` bytes, its side part after it, then 16 bytes of room.
    uint32_t widths = clif_view(v, CLIF_T_WIDTHS);
    uint32_t cw = (widths >> 12) & 15u;
    uint32_t c[5], ends[4];
    c[0] = 0u;
    for (uint32_t q = 0u; q < 4u; q++) {
        uint32_t w = q == 0u ? 1u : (widths >> (4u * (q - 1u))) & 15u;
        ends[q] = nb * c[q] + nb * w;
        c[q + 1u] = (ends[q] + side[q] + 16u + nb - 1u) / nb;
    }
    uint32_t ckpt_at = nb * c[4];
    uint32_t ckpt_len = ((nb + CLIF_T_K - 1u) >> 3) * cw;
    uint32_t parts_at = (ckpt_at + ckpt_len + 16u + 7u) & ~7u;
    if ((uint64_t)(parts_at + 40u) + CLIF_T_SLOT_BYTES + CLIF_T_SLACK > size)
        return 0u;
    uint64_t dst = e + CLIF_T_SLOT_BYTES;
    // The block's view: its nodes, format 16's, its columns' places, its side parts, the tape's widths and choices.
    uint64_t view = e + CLIF_T_SLOT_VIEW;
    ukl_stc_u32(clif_written(t), view + CLIF_T_NODES, nb);
    ukl_stc_u32(clif_written(t), view + CLIF_T_FORMAT, 16u);
    ukl_stc_u32(clif_written(t), view + CLIF_T_WIDTHS, widths);
    ukl_stc_u32(clif_written(t), view + CLIF_T_CUT, clif_view(v, CLIF_T_CUT));
    ukl_stc_u32(clif_written(t), view + CLIF_T_PER, clif_view(v, CLIF_T_PER));
    ukl_stc_u32(clif_written(t), view + CLIF_T_SOURCE_LEN, clif_view(v, CLIF_T_SOURCE_LEN));
    ukl_stc_u32(clif_written(t), view + CLIF_T_STRIDE, 0u);
    for (uint32_t k = 0u; k < 5u; k++)
        ukl_stc_u32(clif_written(t), view + CLIF_T_COLUMNS + 4u * k, c[k]);
    // Each side list's one part: its start (0) and its end.
    uint64_t parts_end = dst + parts_at;
    for (uint32_t q = 0u; q < 4u; q++) {
        ukl_stc_u32(clif_written(t), view + CLIF_T_SIDE + 4u * q, ends[q]);
        ukl_stc_u32(clif_written(t), view + CLIF_T_PARTS + 4u * q, parts_at + 8u * q);
        ukl_stc_u32(clif_written(t), parts_end + 8u * q, 0u);
        ukl_stc_u32(clif_written(t), parts_end + 8u * q + 4u, side[q]);
    }
    // Its checkpoints, raw where the tape stores them (Z_BLOCK / K a block, `cw` bytes each).
    uint64_t checkpoints = (uint64_t)clif_view(v, CLIF_T_COLUMNS + 8u) + (uint64_t)(block * (CLIF_T_Z_BLOCK / CLIF_T_K) * cw);
    clif_tz_copy(t, checkpoints, dst + ckpt_at, ckpt_len);
    // Its pieces, one a turn: the piece index's start, length and kind (a repeat's, the piece it names, if the
    // tape has it: else itself, read raw).
    uint32_t pieces = blocks * CLIF_T_Z_PIECES;
    uint32_t starts = clif_view(v, CLIF_T_COLUMNS);
    uint32_t lens = starts + pieces * 4u;
    uint32_t kinds = lens + pieces * 4u;
    uint32_t payload = clif_view(v, CLIF_T_STRIDE);
    for (uint32_t q = 0u; q < CLIF_T_Z_PIECES; q++) {
        uint32_t j = block * CLIF_T_Z_PIECES + q;
        uint32_t named = clif_t_u32(t, r, payload, clif_t_u32(t, r, starts, j * 4u));
        uint32_t k = clif_t_u8(t, r, kinds, j) == CLIF_T_REPEAT && named < pieces ? named : j;
        uint32_t kind = clif_t_u8(t, r, kinds, k);
        uint32_t start = clif_t_u32(t, r, starts, k * 4u);
        uint64_t len = clif_t_u32(t, r, lens, k * 4u);
        uint64_t src = clif_t_at(r, payload, start);
        // Where it decodes: its column to its side part's end.
        uint64_t to = dst + (uint64_t)(nb * c[q]);
        uint64_t to_end = dst + (uint64_t)(ends[q] + side[q]);
        if (kind == CLIF_T_LZ4)
            clif_tz_lz4(t, src, src + len, to, to_end);
        else
            clif_tz_copy(t, src, to, ukl_min(len, to_end - to));
    }
    return 1u;
}

// The payload of entry `from` (`len` bytes) copied into the entry `e` a word at a time, and whether `from` was still
// complete at `stamp` after (a hit in the oldest generation, promoted into the head).
UKL_INLINE uint32_t clif_tc_copy(UKL_GLOBAL const uint8_t* t, uint64_t from, uint32_t stamp, uint64_t e, uint32_t len)
{
    for (uint64_t at = CLIF_T_E_PAYLOAD; at < (uint64_t)len; at += 8u)
        ukl_stc_u64(clif_written(t), e + at, ukl_ldc_u64(clif_coherent(t), from + at));
    ukl_fence_device();
    return clif_tc_load(t, from + CLIF_T_E_READY) == stamp ? 1u : 0u;
}

// An entry of `size` bytes of the head at `epoch` taken for `key` from the head's arena, filled, completed and
// indexed: its address, or 0 where there was none (the head full, its index or its arena; the payload too large).
// It is filled with block `block` decoded, or with entry `from`'s payload (a promotion: `from` not 0).
//
// Its slot in the head's index is claimed first, pending (its tag, no entry yet), so a thread that misses the same
// block at once finds it and waits rather than taking an entry too. A writer counts itself in its generation
// (WRITERS) from before it takes an entry to its completion, and checks the generation is still its epoch's after: a
// generation is cleared only with no writer in it, so no write of an older epoch's lands in an entry of a newer one.
//
// One way out, every step after the claim under `won`: the lanes of a SIMD group that miss one block at once all
// take, one claims it and the rest find it claimed, and all leave together, after the claimant's decode, within the
// lookup's turn (`clif_tc_get`: why a wait is a turn).
UKL_INLINE uint64_t clif_tc_take(CLIF_T_ARGS, ClifTCache c, uint32_t epoch, uint64_t key, uint32_t h, uint32_t tag, uint32_t size, uint32_t block, uint64_t from, uint32_t from_stamp)
{
    uint64_t head = clif_tc_gen(c, epoch);
    uint64_t slot = 0u, e = 0u;
    uint32_t won = 0u, full = 0u;
    // The claim: the first free slot of the key's probes. A slot of the same block already: none (the lookup, tried
    // again, finds it). No slot free: the head full.
    if (clif_tc_load(t, head + CLIF_T_G_STAMP) == epoch) {
        uint32_t looking = 1u;
        for (uint32_t p = 0u; looking != 0u && p < CLIF_T_C_PROBES; p++) {
            slot = clif_tc_slot(c, head, h, p);
            uint32_t was = clif_tc_cas(t, slot, 0u, tag);
            if (was == 0u) {
                won = 1u;
                looking = 0u;
            } else if ((was & ~CLIF_T_C_NUMBER) == tag) {
                uint32_t number = was & CLIF_T_C_NUMBER;
                if (number == 0u || ukl_ldc_u64(clif_coherent(t), clif_tc_entry(t, c, head, number - 1u) + CLIF_T_E_KEY) == key)
                    looking = 0u;
            }
        }
        full = looking;
    }
    // The claimant counted a writer of the head, the head checked still its epoch's.
    if (won != 0u) {
        clif_tc_add(t, head + CLIF_T_G_WRITERS, 1u);
        ukl_fence_device();
        if (clif_tc_load(t, head + CLIF_T_G_STAMP) != epoch) {
            clif_tc_store(t, slot, 0u);
            clif_tc_add(t, head + CLIF_T_G_WRITERS, ~0u);
            won = 0u;
        }
    }
    // An entry's number, then its bytes, rounded to the arena's alignment: past either's end, the slot given back and
    // the head full.
    uint32_t n = 0u, at = 0u;
    if (won != 0u) {
        n = clif_tc_add(t, head + CLIF_T_G_USED, 1u);
        uint32_t room = 0u;
        if (n < c.entries) {
            uint32_t rounded = (size + (CLIF_T_C_ALIGN - 1u)) & ~(CLIF_T_C_ALIGN - 1u);
            at = clif_tc_add(t, head + CLIF_T_G_BUMP, rounded);
            room = (uint64_t)at + (uint64_t)rounded <= (uint64_t)(c.stride - c.first) ? 1u : 0u;
        }
        if (room == 0u) {
            clif_tc_store(t, slot, 0u);
            clif_tc_add(t, head + CLIF_T_G_WRITERS, ~0u);
            won = 0u;
            full = 1u;
        }
    }
    // Its offset under its number, its completion marked as being written and its bytes, its key (a fence after),
    // then its number in the slot, so a waiter finds it being written; then its payload and (a fence after it) its
    // completion, or (too large for an entry) the entry given back.
    if (won != 0u) {
        clif_tc_store(t, clif_tc_offset_at(c, head, n), at);
        uint64_t entry = clif_tc_entry(t, c, head, n);
        clif_tc_store(t, entry + CLIF_T_E_READY, epoch | CLIF_T_C_WRITING);
        clif_tc_store(t, entry + CLIF_T_E_LEN, size);
        ukl_fence_device();
        ukl_stc_u64(clif_written(t), entry + CLIF_T_E_KEY, key);
        ukl_fence_device();
        clif_tc_store(t, slot, tag | (n + 1u));
        uint32_t fits = from != 0u ? clif_tc_copy(t, from, from_stamp, entry, size) : clif_tz_decode(t, v, block, entry, (uint64_t)size);
        ukl_fence_device();
        if (fits != 0u) {
            clif_tc_store(t, entry + CLIF_T_E_READY, epoch);
            ukl_fence_device();
            e = entry;
        } else {
            clif_tc_store(t, entry + CLIF_T_E_READY, 0u);
            ukl_fence_device();
        }
        clif_tc_add(t, head + CLIF_T_G_WRITERS, ~0u);
    }
    if (full != 0u)
        clif_tc_full(t, c, epoch);
    return e;
}

// What a probe of one generation found.
#define CLIF_T_C_NOT_HERE 0u
#define CLIF_T_C_FOUND 1u
#define CLIF_T_C_PENDING 2u

// The key's entry in generation `gen` (stamped `stamp`), by its probes: found complete (`*e`); being written by
// another thread (its slot claimed with no entry yet, or its entry not complete): pending; not here at an empty slot
// or after every probe. A probe never waits: the lookup waits, a turn at a time (`clif_tc_get`).
UKL_INLINE uint32_t clif_tc_probe(UKL_GLOBAL const uint8_t* t, ClifTCache c, uint64_t gen, uint32_t stamp, uint64_t key, uint32_t h, uint32_t tag, UKL_PRIVATE uint64_t* e)
{
    uint32_t found = CLIF_T_C_NOT_HERE;
    uint32_t looking = 1u;
    for (uint32_t p = 0u; looking != 0u && p < CLIF_T_C_PROBES; p++) {
        uint32_t x = clif_tc_load(t, clif_tc_slot(c, gen, h, p));
        // An empty slot ends the generation; another key's tag, on; the key's tag with no entry yet, pending.
        if (x == 0u) {
            looking = 0u;
        } else if ((x & ~CLIF_T_C_NUMBER) == tag) {
            if ((x & CLIF_T_C_NUMBER) == 0u) {
                found = CLIF_T_C_PENDING;
                looking = 0u;
            } else {
                uint64_t entry = clif_tc_entry(t, c, gen, (x & CLIF_T_C_NUMBER) - 1u);
                if (ukl_ldc_u64(clif_coherent(t), entry + CLIF_T_E_KEY) == key) {
                    uint32_t ready = clif_tc_load(t, entry + CLIF_T_E_READY);
                    if (ready == stamp) {
                        clif_tc_stat(t, c.at, CLIF_T_C_HITS);
                        *e = entry;
                        found = CLIF_T_C_FOUND;
                        looking = 0u;
                    } else if (ready == (stamp | CLIF_T_C_WRITING)) {
                        found = CLIF_T_C_PENDING;
                        looking = 0u;
                    }
                }
            }
        }
    }
    return found;
}

// The entry of block `block`'s `key` in the rolling cache at `cache`: looked up in its live generations, head first,
// a hit in the oldest copied into the head; on a miss, an entry of `size` bytes taken from the head and the block
// decoded into it (`clif_tc_take`). A full head turns the spare into the head. After TRIES tries without room, or for
// a `size` past the cache's most, a failure counted and the dead entry given.
//
// An entry pending (another thread writing it) is waited for by turns of this one loop, SPIN turns at most (then a
// miss, decoded here too: a duplicate). Every turn ends at the loop's one way back, at its foot: no `continue`, and
// no wait in a loop of its own. On a GPU whose lanes do not each go their own way (Apple's), lanes of a SIMD group
// that take a way back while others of the group are still in the turn run round it again and again, the others
// standing still: a lane waiting for a block that one of those others had claimed ran its wait to its end while the
// claimant stood before its decode, and every one gave up. A turn so ends for every lane of the group before any
// begins the next, and a claim of the group's is decoded within the turn that made it.
UKL_INLINE uint64_t clif_tc_get(CLIF_T_ARGS, uint64_t cache, uint64_t key, uint32_t size, uint32_t block)
{
    ClifTCache c = clif_tc_open(t, cache);
    // The key's slot, the high bits of a multiplicative hash; its tag in a slot, 16 bits of a second hash, never 0,
    // so a pending slot (no entry yet) is not empty.
    uint32_t h = (uint32_t)((key * 0x9E3779B97F4A7C15ul) >> 32);
    uint32_t tag = ((uint32_t)((key * 0xC2B2AE3D27D4EB4Ful) >> 48) | 1u) << CLIF_T_C_TAG_SHIFT;
    uint32_t live = c.gens - 1u;
    uint32_t tries = 0u, waits = 0u, waited = 0u;
    uint64_t got = 0u;
    for (uint32_t done = 0u; done == 0u;) {
        uint32_t epoch = clif_tc_load(t, cache + CLIF_T_C_EPOCH);
        uint32_t found = CLIF_T_C_NOT_HERE;
        uint64_t e = 0u;
        // The generation it was found in (`k`), and its stamp.
        uint32_t k = 0u, stamp = 0u;
        for (uint32_t j = 0u; found == CLIF_T_C_NOT_HERE && j < live; j++) {
            uint32_t want = epoch - j;
            uint64_t gen = clif_tc_gen(c, want);
            uint32_t s = clif_tc_load(t, gen + CLIF_T_G_STAMP);
            if (s == want) {
                found = clif_tc_probe(t, c, gen, s, key, h, tag, &e);
                k = j;
                stamp = s;
            }
        }
        // Pending: another turn, after a fence; past SPIN turns, a miss. Once any lookup of the cache has failed, no
        // more turns: the run that shares it is refused (decision 0463), so a wait gains nothing, and a launch whose
        // lookups go wrong ends in a few turns of each, not in SPIN turns of every lane (minutes of the GPU's time).
        // (Left pending, it is neither found nor taken below: the lookup fails.)
        if (found == CLIF_T_C_PENDING && clif_tc_load(t, cache + CLIF_T_C_FAILED) != 0u) {
            done = 1u;
        } else if (found == CLIF_T_C_PENDING) {
            waited = 1u;
            waits += 1u;
            if (waits >= CLIF_T_C_SPIN) {
                clif_tc_stat(t, c.at, CLIF_T_C_DUPLICATES);
                waits = 0u;
                found = CLIF_T_C_NOT_HERE;
            } else {
                ukl_fence_device();
            }
        }
        if (found == CLIF_T_C_FOUND) {
            if (waited != 0u)
                clif_tc_stat(t, c.at, CLIF_T_C_WAITED);
            got = e;
            // A hit in the oldest live generation (of two or more): copied into the head (where there is room).
            if (k == live - 1u && live > 1u) {
                uint64_t copied = clif_tc_take(t, v, c, epoch, key, h, tag, clif_tc_load(t, e + CLIF_T_E_LEN), block, e, stamp);
                if (copied != 0u) {
                    clif_tc_add(t, cache + CLIF_T_C_PROMOTED, 1u);
                    got = copied;
                }
            }
            done = 1u;
        } else if (found == CLIF_T_C_NOT_HERE) {
            // A miss: an entry of the head taken and filled, else (the head full, or the block claimed by another
            // thread meanwhile) tried again. A size of 0 has no entry (the dead one); an entry past the most is never
            // taken (no generation turns for it).
            if (size == 0u) {
                got = clif_tc_dead(t, cache);
                done = 1u;
            } else if (size > c.max) {
                done = 1u;
            } else {
                got = clif_tc_take(t, v, c, epoch, key, h, tag, size, block, 0u, 0u);
                if (got != 0u) {
                    clif_tc_add(t, cache + CLIF_T_C_MISSES, 1u);
                    done = 1u;
                } else {
                    tries += 1u;
                    done = tries >= CLIF_T_C_TRIES ? 1u : 0u;
                }
            }
        }
    }
    if (got == 0u) {
        clif_tc_add(t, cache + CLIF_T_C_FAILED, 1u);
        got = clif_tc_dead(t, cache);
    }
    return got;
}

// A 16z tape's block `block`: the address of the cache entry that holds it decoded, found in the cache the view names
// or decoded into its head; the dead entry where the cache had no room, or for a block past the tape's last (a node
// read that is no node of it: a failure counted, so its run is refused, decision 0463).
UKL_COLD uint64_t clif_tape_z_block(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t block)
{
    uint64_t cache = clif_tz_cache(v);
    uint32_t blocks = (clif_view(v, CLIF_T_NODES) + (CLIF_T_Z_BLOCK - 1u)) >> 10;
    if (block >= blocks) {
        clif_tc_add(t, cache + CLIF_T_C_FAILED, 1u);
        return clif_tc_dead(t, cache);
    }
    return clif_tc_get(t, v, cache, clif_tz_key(v, block), clif_view(v, CLIF_T_COLUMNS + 12u), block);
}

// A 16z read of node `i`: its block's entry, read as format 16 at the node's place in it (`r`, `node`). Where
// threads share the cache (the view's `columns[4]`), the entry is checked complete and of this block before the
// read and its completion read again after (`clif_tz_kept`): a read of an entry another thread took meanwhile is
// read again, TRIES times at most, then the dead entry is read (a failure counted). Never the entry as it is, which
// may hold two blocks' bytes: a kernel that went on with such a value would reach anywhere before its run is refused.
typedef struct ClifTZ {
    ClifTAt r;
    uint32_t node, block, tries, before, recheck;
    uint64_t entry;
} ClifTZ;

UKL_INLINE void clif_tz_read(UKL_PRIVATE ClifTZ* z, uint64_t entry, uint32_t recheck)
{
    z->entry = entry;
    z->recheck = recheck;
    z->r.view = entry + CLIF_T_SLOT_VIEW;
    z->r.bytes = entry + CLIF_T_SLOT_BYTES;
}

UKL_INLINE void clif_tz_give_up(CLIF_T_ARGS, UKL_PRIVATE ClifTZ* z)
{
    uint64_t cache = clif_tz_cache(v);
    clif_tc_add(t, cache + CLIF_T_C_FAILED, 1u);
    clif_tz_read(z, clif_tc_dead(t, cache), 0u);
}

UKL_INLINE void clif_tz_find(CLIF_T_ARGS, UKL_PRIVATE ClifTZ* z)
{
    for (;;) {
        uint64_t entry = clif_tape_z_block(t, v, z->block);
        z->before = ukl_atomic_load_at(t, entry + CLIF_T_E_READY);
        if (clif_view(v, CLIF_T_COLUMNS + 16u) == 0u) {
            clif_tz_read(z, entry, 0u);
            return;
        }
        ukl_fence_device();
        uint64_t held = ukl_ldc_u64(clif_coherent(t), entry + CLIF_T_E_KEY);
        if (held == clif_tz_key(v, z->block) && (z->before & CLIF_T_C_WRITING) == 0u && z->before != 0u) {
            clif_tz_read(z, entry, 1u);
            return;
        }
        if (z->tries >= CLIF_T_C_TRIES) {
            clif_tz_give_up(t, v, z);
            return;
        }
        z->tries += 1u;
    }
}

UKL_INLINE ClifTZ clif_tz_open(CLIF_T_ARGS, uint32_t i)
{
    ClifTZ z;
    z.block = i >> 10;
    z.node = i & (CLIF_T_Z_BLOCK - 1u);
    z.tries = 0u;
    z.r.cached = 1u;
    z.r.first = z.block << 10;
    z.r.mask = (uint64_t)clif_view(v, CLIF_T_COLUMNS + 12u) - 1u;
    clif_tz_find(t, v, &z);
    return z;
}

// Whether the value just read from the entry stands: 0 where it is to be read again (from the entry found again, or
// the dead entry).
UKL_INLINE uint32_t clif_tz_kept(CLIF_T_ARGS, UKL_PRIVATE ClifTZ* z)
{
    if (z->recheck == 0u)
        return 1u;
    ukl_fence_device();
    if (ukl_atomic_load_at(t, z->entry + CLIF_T_E_READY) == z->before)
        return 1u;
    if (z->tries >= CLIF_T_C_TRIES) {
        clif_tz_give_up(t, v, z);
        return 0u;
    }
    z->tries += 1u;
    clif_tz_find(t, v, z);
    return 0u;
}

// Each read's 16z body, called apart (inlined, every read's 16z body at every call site made a label kernel's
// compile 18 s at format 15 and 33 s at 16z, against 1.8 s without).
UKL_COLD uint64_t clif_tape_f16_side_z(CLIF_T_ARGS, uint32_t k, uint32_t i)
{
    ClifTZ z = clif_tz_open(t, v, i);
    for (;;) {
        uint64_t x = clif_t16_side(t, v, z.r, k, z.node);
        if (clif_tz_kept(t, v, &z))
            return x;
    }
}
UKL_COLD uint32_t clif_tape_size_z(CLIF_T_ARGS, uint32_t i)
{
    ClifTZ z = clif_tz_open(t, v, i);
    for (;;) {
        uint32_t x = clif_t16_size(t, v, z.r, z.node);
        if (clif_tz_kept(t, v, &z))
            return x;
    }
}
UKL_COLD uint32_t clif_tape_number_z(CLIF_T_ARGS, uint32_t i)
{
    ClifTZ z = clif_tz_open(t, v, i);
    for (;;) {
        uint32_t x = clif_t16_number(t, v, z.r, z.node);
        if (clif_tz_kept(t, v, &z))
            return x;
    }
}
UKL_COLD uint32_t clif_tape_start_stated_z(CLIF_T_ARGS, uint32_t i)
{
    ClifTZ z = clif_tz_open(t, v, i);
    for (;;) {
        uint32_t x = clif_t16_start_stated(t, v, z.r, z.node);
        if (clif_tz_kept(t, v, &z))
            return x;
    }
}
UKL_COLD uint32_t clif_tape_end_stated_z(CLIF_T_ARGS, uint32_t i)
{
    ClifTZ z = clif_tz_open(t, v, i);
    for (;;) {
        uint32_t x = clif_t16_end_stated(t, v, z.r, z.node);
        if (clif_tz_kept(t, v, &z))
            return x;
    }
}
UKL_COLD uint32_t clif_tape_length_z(CLIF_T_ARGS, uint32_t i)
{
    ClifTZ z = clif_tz_open(t, v, i);
    for (;;) {
        uint32_t x = clif_t16_length(t, v, z.r, z.node);
        if (clif_tz_kept(t, v, &z))
            return x;
    }
}
UKL_COLD uint32_t clif_tape_start_next_z(CLIF_T_ARGS, uint32_t i, uint32_t prev)
{
    ClifTZ z = clif_tz_open(t, v, i);
    for (;;) {
        uint32_t x = clif_t16_start_next(t, v, z.r, z.node, prev);
        if (clif_tz_kept(t, v, &z))
            return x;
    }
}
UKL_COLD uint64_t clif_tape_slot_z(CLIF_T_ARGS, uint32_t i)
{
    ClifTZ z = clif_tz_open(t, v, i);
    for (;;) {
        uint64_t x = clif_t16_get(t, v, z.r, 0u, z.node);
        if (clif_tz_kept(t, v, &z))
            return x;
    }
}
UKL_COLD uint32_t clif_tape_parent_z(CLIF_T_ARGS, uint32_t i) { return clif_t16_parent(t, v, i, 1u); }

#define CLIF_T_IS_Z(f) ((f) == CLIF_T_F16Z_FORMAT)
#else
#define CLIF_T_IS_Z(f) 0
#endif

// ---- the reads: each by the view's format ---------------------------------------------------------------------------

// A node's value in side list `k` (0 codes' low parts, 1 slots, 2 gaps, 3 lengths), the node listed.
UKL_COLD uint64_t clif_tape_f16_side(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t k, uint32_t i)
{
#ifdef CLIF_READS_TAPE_Z
    if (CLIF_T_IS_Z(clif_view(v, CLIF_T_FORMAT)))
        return clif_tape_f16_side_z(t, v, k, i);
#endif
    return clif_t16_side(t, v, clif_t_in_place(), k, i);
}

// The node's subtree's size (itself and its descendants).
UKL_INLINE uint32_t clif_tape_size(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    uint32_t f = clif_view(v, CLIF_T_FORMAT);
    if (f == 16u)
        return clif_t16_size(t, v, clif_t_in_place(), i);
#ifdef CLIF_READS_TAPE_Z
    if (CLIF_T_IS_Z(f))
        return clif_tape_size_z(t, v, i);
#endif
    return clif_t15_size(t, v, i);
}

// The node's code's number in its table (format 16: its language's).
UKL_INLINE uint32_t clif_tape_number(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    uint32_t f = clif_view(v, CLIF_T_FORMAT);
    if (f == 16u)
        return clif_t16_number(t, v, clif_t_in_place(), i);
#ifdef CLIF_READS_TAPE_Z
    if (CLIF_T_IS_Z(f))
        return clif_tape_number_z(t, v, i);
#endif
    return clif_t15_number(t, v, i);
}

// The node's start as the tape states it: what a check of the tape reads. A kernel's span reads `tape_start`.
UKL_INLINE uint32_t clif_tape_start_stated(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    uint32_t f = clif_view(v, CLIF_T_FORMAT);
    if (f == 16u)
        return clif_t16_start_stated(t, v, clif_t_in_place(), i);
#ifdef CLIF_READS_TAPE_Z
    if (CLIF_T_IS_Z(f))
        return clif_tape_start_stated_z(t, v, i);
#endif
    return clif_t15_start_stated(t, v, i);
}

// The node's end as the tape states it.
UKL_INLINE uint32_t clif_tape_end_stated(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    uint32_t f = clif_view(v, CLIF_T_FORMAT);
    if (f == 16u)
        return clif_t16_end_stated(t, v, clif_t_in_place(), i);
#ifdef CLIF_READS_TAPE_Z
    if (CLIF_T_IS_Z(f))
        return clif_tape_end_stated_z(t, v, i);
#endif
    return clif_t15_end_stated(t, v, i);
}

// The node's start, held to the source: a span from the tape reads no byte past it, whatever the tape's gaps hold.
UKL_INLINE uint32_t clif_tape_start(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    return ukl_min(clif_tape_start_stated(t, v, i), clif_view(v, CLIF_T_SOURCE_LEN));
}

// The node's end, held between its start and the source's end: a reader's span is never reversed and never past the
// source.
UKL_INLINE uint32_t clif_tape_end(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    uint32_t start = clif_tape_start(t, v, i);
    return ukl_min(ukl_max(clif_tape_end_stated(t, v, i), start), clif_view(v, CLIF_T_SOURCE_LEN));
}

// The node's parent, or NONE at the root.
UKL_COLD uint32_t clif_tape_parent(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    if (i == 0u)
        return CLIF_T_NONE;
    uint32_t f = clif_view(v, CLIF_T_FORMAT);
    if (f == 16u)
        return clif_t16_parent(t, v, i, 0u);
#ifdef CLIF_READS_TAPE_Z
    if (CLIF_T_IS_Z(f))
        return clif_tape_parent_z(t, v, i);
#endif
    return clif_t15_parent(t, v, i);
}

// One past the node's subtree.
UKL_INLINE uint32_t clif_tape_end_of(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    return clif_t_hop(t, v, i, clif_tape_size(t, v, i));
}

// The node after the node's subtree while it is before `end` (its parent's subtree's end), else NONE.
UKL_INLINE uint32_t clif_tape_next_in(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i, uint32_t end)
{
    uint32_t after = clif_tape_end_of(t, v, i);
    return after < end ? after : CLIF_T_NONE;
}

// The node's next sibling, or NONE.
UKL_INLINE uint32_t clif_tape_next(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    uint32_t parent = clif_tape_parent(t, v, i);
    return parent == CLIF_T_NONE ? CLIF_T_NONE : clif_tape_next_in(t, v, i, clif_tape_end_of(t, v, parent));
}

// The node's first child, or NONE.
UKL_INLINE uint32_t clif_tape_first_child(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    return clif_tape_size(t, v, i) > 1u ? i + 1u : CLIF_T_NONE;
}

// The node's children, by hops over their subtrees.
UKL_INLINE uint32_t clif_tape_child_count(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    uint32_t end = clif_tape_end_of(t, v, i);
    uint32_t n = 0u;
    for (uint32_t c = i + 1u; c < end; c = clif_t_hop(t, v, c, clif_tape_size(t, v, c)))
        n += 1u;
    return n;
}

// The node's `k`-th child (from 0), by hops over its elder siblings' subtrees, or NONE.
UKL_INLINE uint32_t clif_tape_child(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i, uint32_t k)
{
    uint32_t end = clif_tape_end_of(t, v, i);
    for (uint32_t c = i + 1u; c < end; c = clif_t_hop(t, v, c, clif_tape_size(t, v, c))) {
        if (k == 0u)
            return c;
        k -= 1u;
    }
    return CLIF_T_NONE;
}

// The node's length in bytes.
UKL_INLINE uint32_t clif_tape_length(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
    uint32_t f = clif_view(v, CLIF_T_FORMAT);
    if (f == 16u)
        return clif_t16_length(t, v, clif_t_in_place(), i);
#ifdef CLIF_READS_TAPE_Z
    if (CLIF_T_IS_Z(f))
        return clif_tape_length_z(t, v, i);
#endif
    return clif_t15_length(t, v, i);
}

// The node's start, given the node before's (`prev`; any value for the first node): in a scan, format 16's from the
// one before, a gap's add, or its checkpoint.
UKL_INLINE uint32_t clif_tape_start_next(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i, uint32_t prev)
{
    uint32_t f = clif_view(v, CLIF_T_FORMAT);
    if (f == 16u)
        return clif_t16_start_next(t, v, clif_t_in_place(), i, prev);
#ifdef CLIF_READS_TAPE_Z
    if (CLIF_T_IS_Z(f))
        return clif_tape_start_next_z(t, v, i, prev);
#endif
    return clif_tape_start(t, v, i);
}

// Format 16's slot of the node: an inner node's subtree size, shifted up by one; a leaf's step back to its previous
// sibling or (bit 1) its parent, shifted up by two, with bit 0 set.
UKL_INLINE uint64_t clif_tape_slot(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t i)
{
#ifdef CLIF_READS_TAPE_Z
    if (CLIF_T_IS_Z(clif_view(v, CLIF_T_FORMAT)))
        return clif_tape_slot_z(t, v, i);
#endif
    return clif_t16_get(t, v, clif_t_in_place(), 0u, i);
}

// The line byte `x` lies in (from 0). A 16z tape holds its newline index raw, as format 16 does.
UKL_INLINE uint32_t clif_tape_line(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t x)
{
    uint32_t f = clif_view(v, CLIF_T_FORMAT);
    if (f == 16u || CLIF_T_IS_Z(f))
        return clif_t16_line(t, v, x);
    return clif_t15_line(t, v, x);
}

// Line `k`'s first byte.
UKL_INLINE uint32_t clif_tape_line_start(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t k)
{
    uint32_t f = clif_view(v, CLIF_T_FORMAT);
    if (f == 16u || CLIF_T_IS_Z(f))
        return clif_t16_line_start(t, v, k);
    return clif_t_u32(t, clif_t_in_place(), clif_view(v, CLIF_T_LINES), k * 4u);
}

// Whether the tape's block `block` (its nodes `PART * block ..`; NONE: the whole document) has a node of kind
// `kind`, by its presence (the view's `presence`, decision 0434 §4): 1 where the tape has none to tell by (format 15,
// or one stored without it), and the document's answer where its blocks' presences are not held.
UKL_INLINE uint32_t clif_tape_has_kind(UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v, uint32_t block, uint32_t kind)
{
    ClifTAt r = clif_t_in_place();
    uint32_t at = clif_view(v, CLIF_T_PRESENCE);
    if (at == 0u)
        return 1u;
    uint32_t kinds = clif_t_u32(t, r, at, 0u);
    uint32_t word = kind >> 6;
    if (word >= kinds)
        return 0u;
    uint32_t fields = clif_t_u32(t, r, at, 4u);
    uint32_t count = clif_t_u32(t, r, at, 8u);
    // The block's presence follows the document's; NONE + 1 is the document's (0).
    uint32_t index = block + 1u < count ? block + 1u : 0u;
    uint32_t k = ((index * (kinds + fields) + word) << 3) + 16u;
    return (uint32_t)((clif_t_word(t, r, at, k) >> (kind & 63u)) & 1u);
}
