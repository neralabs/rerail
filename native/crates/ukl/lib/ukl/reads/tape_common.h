// ukl/reads/tape_common.h: what every format's tape reads share (decision 0576): the view's fields and the formats'
// constants, the one load of the tape in place, and the reads no format changes. A kernel includes
// `<ukl/reads/tape.h>` or `<ukl/reads/tape_z.h>`, never this file.
//
// The host's reads are the CLIF of `rerail_clif::reads` (its Rust and its JIT); the kernels' are written by hand to
// the same functions (decision 0524), and a change is made in both. `the_device_reads_read_every_node_as_the_crate_does`
// (rerail-tape-format) reads every node of a tape of each format through both headers on the device against the host's
// reads, and `the_tape_headers_constants_are_the_views` (rerail-clif) holds the constants below to `view.rs`'s: they
// are defined here and nowhere else.
//
// Every read takes the tape's base `t` (each offset counts from it: the view's, and the cache's) and its view `v`.
// Each keeps its defences (decision 0463): a size is at least 1, a hop moves on and stays in the tape, a side list's
// offsets stay in its part, a span stays in the source, and a 16z read of a block another thread is writing is read
// again or given up.

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
#define CLIF_T_SLOT_BYTES 160u
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
#define CLIF_T_C_SLOTS_MAX 131072u
#define CLIF_T_C_TRIES 4u
#define CLIF_T_C_SPIN 65536u
// A slot: the key's tag above TAG_SHIFT, its entry's number plus one below (0: pending, no entry yet).
#define CLIF_T_C_TAG_SHIFT 16u
#define CLIF_T_C_NUMBER 65535u

// What a probe of one generation found.
#define CLIF_T_C_NOT_HERE 0u
#define CLIF_T_C_FOUND 1u
#define CLIF_T_C_PENDING 2u

#define CLIF_T_ARGS UKL_GLOBAL const uint8_t* t, CLIF_VIEW_SPACE const uint32_t* v

// ---- the one load of the tape in place ------------------------------------------------------------------------------

// `w` bytes (1, 2, 4 or 8), little-endian, at byte `base + off` of the tape: one read of the words that hold them,
// whatever the width (a load a width, the width known only at run time, put four loads in each of a read's 38 copies:
// the interpreter's `tape_end` compiled 152 of them on an Arc 140V).
UKL_CALLED uint64_t clif_t_ld(UKL_GLOBAL const uint8_t* t, uint32_t base, uint32_t off, uint32_t w)
{
    uint64_t x = ukl_ld_window(t, (uint64_t)base + (uint64_t)off, w);
    return w >= 8u ? x : x & ukl_mask64(8u * w);
}

// ---- small helpers ---------------------------------------------------------------------------------------------------

// The low `w` bytes' mask (all ones at 8 or more).
UKL_INLINE uint64_t clif_t_mask(uint32_t w) { return w >= 8u ? ~(uint64_t)0 : ((uint64_t)1 << (w * 8u)) - 1u; }

// `c + size`, past `c` however `size` wraps, and no further than the tape's nodes.
// Past `c` after the clamp too: from the view's last node (a short count, a node asked past it) a hop clamped to the
// nodes was a step of zero, and a walk stopped only past its end never stopped (decision 0463).
UKL_INLINE uint32_t clif_t_hop(CLIF_T_ARGS, uint32_t c, uint32_t size) { return ukl_max(ukl_min(ukl_max(c + size, c + 1u), clif_view(v, CLIF_T_NODES)), c + 1u); }

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

// ---- the reads no format changes ----------------------------------------------------------------------------------

// Whether the tape's block `block` (its nodes `PART * block ..`; NONE: the whole document) has a node of kind
// `kind`, by its presence (the view's `presence`, decision 0434 §4): 1 where the tape has none to tell by (format 15,
// or one stored without it), and the document's answer where its blocks' presences are not held.
UKL_INLINE uint32_t clif_tape_has_kind(CLIF_T_ARGS, uint32_t block, uint32_t kind)
{
    uint32_t at = clif_view(v, CLIF_T_PRESENCE);
    if (at == 0u)
        return 1u;
    uint32_t kinds = (uint32_t)clif_t_ld(t, at, 0u, 4u);
    uint32_t word = kind >> 6;
    if (word >= kinds)
        return 0u;
    uint32_t fields = (uint32_t)clif_t_ld(t, at, 4u, 4u);
    uint32_t count = (uint32_t)clif_t_ld(t, at, 8u, 4u);
    // The block's presence follows the document's; NONE + 1 is the document's (0).
    uint32_t index = block + 1u < count ? block + 1u : 0u;
    uint32_t k = ((index * (kinds + fields) + word) << 3) + 16u;
    return (uint32_t)((clif_t_ld(t, at, k, 8u) >> (kind & 63u)) & 1u);
}
