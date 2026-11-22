// ukl/reads/tape_f16z.h: 16z's reads (decisions 0353 step 4b, 0576): `clif_f16z_*`. A 16z tape is format 16 cut into
// blocks of Z_BLOCK nodes, each compressed; a read finds its node's block decoded in the rolling cache the view names
// (`rerail_clif::view::cache`), decoding it into the cache on a miss, and reads it there with format 16's body
// (`clif_f16b_*`, `<ukl/body/tape_f16.h>` over the entry). Included by `<ukl/reads/tape_z.h>` only.
//
// One helper finds a block (`clif_z_block`), and one check says whether a read stands (`clif_z_kept`): the public
// reads' macro (`<ukl/body/tape_read.h>`) makes it once per read. The cache's protocol below is the cache's own.

#pragma once
#include <ukl/reads/tape_common.h>
#include <ukl/reads/tape_f16.h>

// ---- the rolling cache (`view::cache`) ----------------------------------------------------------------------------

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
    uint32_t nodes = clif_view(v, CLIF_T_NODES);
    uint32_t nb = ukl_min(nodes - (block << 10), CLIF_T_Z_BLOCK);
    uint32_t blocks = (nodes + (CLIF_T_Z_BLOCK - 1u)) >> 10;
    // Each side list's part of the block: from its start to the next's (list `q`'s starts at `4 * q * (blocks + 1)`),
    // never past it.
    uint32_t parts = clif_view(v, CLIF_T_COLUMNS + 4u);
    uint32_t side[4];
    for (uint32_t q = 0u; q < 4u; q++) {
        uint32_t at = ((blocks + 1u) * q + block) * 4u;
        uint32_t from = (uint32_t)clif_t_ld(t, parts, at, 4u);
        side[q] = ukl_max((uint32_t)clif_t_ld(t, parts, at + 4u, 4u), from) - from;
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
        uint32_t named = (uint32_t)clif_t_ld(t, payload, (uint32_t)clif_t_ld(t, starts, j * 4u, 4u), 4u);
        uint32_t k = (uint32_t)clif_t_ld(t, kinds, j, 1u) == CLIF_T_REPEAT && named < pieces ? named : j;
        uint32_t kind = (uint32_t)clif_t_ld(t, kinds, k, 1u);
        uint32_t start = (uint32_t)clif_t_ld(t, starts, k * 4u, 4u);
        uint64_t len = (uint32_t)clif_t_ld(t, lens, k * 4u, 4u);
        uint64_t src = (uint64_t)payload + (uint64_t)start;
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
// Called apart (`UKL_COLD`): a miss is the rare way, and its decode the largest code of the reads.
UKL_COLD uint64_t clif_tc_take(CLIF_T_ARGS, ClifTCache c, uint32_t epoch, uint64_t key, uint32_t h, uint32_t tag, uint32_t size, uint32_t block, uint64_t from, uint32_t from_stamp)
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
                // A numbered slot fenced before its entry is read, as the lookup's.
                if (number != 0u) ukl_fence_device();
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
                // The number read, then its entry: the fence pairs the writer's before the number (48ce04a06b).
                ukl_fence_device();
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

// ---- a block, found ---------------------------------------------------------------------------------------------------

// The address of the cache entry that holds the tape's block `block` decoded: found in the cache the view names, or
// decoded into its head. The dead entry (its zeros) where the cache had no room, or for a block past the tape's last
// (a node read that is no node of it: a failure counted, so its run is refused, decision 0463).
UKL_COLD uint64_t clif_z_block(CLIF_T_ARGS, uint32_t block)
{
    uint64_t cache = clif_tz_cache(v);
    uint32_t blocks = (clif_view(v, CLIF_T_NODES) + (CLIF_T_Z_BLOCK - 1u)) >> 10;
    if (block >= blocks) {
        clif_tc_add(t, cache + CLIF_T_C_FAILED, 1u);
        return clif_tc_dead(t, cache);
    }
    return clif_tc_get(t, v, cache, clif_tz_key(v, block), clif_view(v, CLIF_T_COLUMNS + 12u), block);
}

// ---- a read, checked ------------------------------------------------------------------------------------------------

// Whether threads share the cache (the view's `columns[4]`): else no other thread takes an entry, and every read
// stands.
UKL_INLINE uint32_t clif_z_shared(CLIF_VIEW_SPACE const uint32_t* v) { return clif_view(v, CLIF_T_COLUMNS + 16u); }

// The cache's epoch, before a read.
UKL_INLINE uint32_t clif_z_epoch(CLIF_T_ARGS)
{
    uint32_t epoch = clif_tc_load(t, clif_tz_cache(v) + CLIF_T_C_EPOCH);
    ukl_fence_device();
    return epoch;
}

// Whether the 16z read just made (try `k`, from 0) stands: 1 it does, 0 it is to be made again, 2 it is given up.
// An entry is cleared only after the epoch moves on (`clif_tc_full`: the epoch's compare-and-swap, then the clear of
// the generation it retired), so a read stands when the epoch after it is the one before it (`*epoch`, then set to
// the epoch seen). After TRIES reads that did not stand, a failure is counted (the run is refused) and the read is
// given up: its value is never used, since it may hold two blocks' bytes.
UKL_INLINE uint32_t clif_z_kept(CLIF_T_ARGS, UKL_PRIVATE uint32_t* epoch, uint32_t k)
{
    if (clif_z_shared(v) == 0u)
        return 1u;
    ukl_fence_device();
    uint32_t now = clif_tc_load(t, clif_tz_cache(v) + CLIF_T_C_EPOCH);
    if (now == *epoch)
        return 1u;
    *epoch = now;
    if (k + 1u < CLIF_T_C_TRIES)
        return 0u;
    clif_tc_add(t, clif_tz_cache(v) + CLIF_T_C_FAILED, 1u);
    return 2u;
}

// The one check of a 16z read (decision 0576): `x`, a read of type `type` of tape `tb` and view `tv`, made until it
// stands (`clif_z_kept`), then returned, or 0 where it is given up. The tape's public reads (`<ukl/body/tape_read.h>`)
// and the view reads (`<ukl/reads/view.h>`) check here, each once per read; the reads they make are unchecked.
#define CLIF_Z_READ(type, tb, tv, x)                                       \
    {                                                                      \
        uint32_t clif_epoch = clif_z_epoch(tb, tv);                        \
        for (uint32_t clif_try = 0u;; clif_try++) {                        \
            type clif_x = x;                                               \
            uint32_t clif_kept = clif_z_kept(tb, tv, &clif_epoch, clif_try); \
            if (clif_kept != 0u)                                           \
                return clif_kept == 1u ? clif_x : (type)0;                 \
        }                                                                  \
    }

// ---- format 16's body over a decoded block --------------------------------------------------------------------------

// The block's reads take the cache entry `e` (its view at SLOT_VIEW, its bytes from SLOT_BYTES) and `m`, the entry's
// size less one: every offset into its bytes is held to it, so a read of an entry another thread writes again reads
// nothing outside the cache.
#define CLIF_ZB_ARGS UKL_GLOBAL const uint8_t* t, uint64_t e, uint64_t m

UKL_INLINE uint32_t clif_f16b_view(CLIF_ZB_ARGS, uint32_t a) { return ukl_ldc_u32(clif_coherent(t), e + CLIF_T_SLOT_VIEW + a); }

UKL_INLINE uint64_t clif_f16b_ld(CLIF_ZB_ARGS, uint32_t base, uint32_t off, uint32_t w)
{
    uint64_t a = e + CLIF_T_SLOT_BYTES + (((uint64_t)base + (uint64_t)off) & m);
    // One coherent read, of the words that hold the `w` bytes (decision 0576 §15): a read for each width was the
    // same read four times, and inlined at each of a 16z kernel's loads (142 in the tape reads' test kernel).
    uint64_t x = ukl_ldc_window(clif_coherent(t), a, w);
    return w >= 8u ? x : x & ukl_mask64(8u * w);
}

#define CLIF_16(name) clif_f16b_##name
#define CLIF_16_ARGS CLIF_ZB_ARGS
#define CLIF_16_PASS t, e, m
// The side read over a decoded block, the rare case: a call (decision 0576 §19).
#define CLIF_16_RARE UKL_COLD
#include <ukl/body/tape_f16.h>

// ---- 16z's reads: each the block's read of the node ---------------------------------------------------------------

// The entry's size less one (every entry's: the cache's slot size, a power of two).
UKL_INLINE uint64_t clif_z_mask(CLIF_VIEW_SPACE const uint32_t* v) { return (uint64_t)clif_view(v, CLIF_T_COLUMNS + 12u) - 1u; }

// Node `i`'s block's entry, and the node's number in the block.
#define CLIF_Z_AT(i) t, clif_z_block(t, v, (i) >> 10), clif_z_mask(v), (i) & (CLIF_T_Z_BLOCK - 1u)

UKL_INLINE uint64_t clif_f16z_slot(CLIF_T_ARGS, uint32_t i) { return clif_f16b_slot(CLIF_Z_AT(i)); }
UKL_INLINE uint32_t clif_f16z_size(CLIF_T_ARGS, uint32_t i) { return clif_f16b_size(CLIF_Z_AT(i)); }
UKL_INLINE uint32_t clif_f16z_number(CLIF_T_ARGS, uint32_t i) { return clif_f16b_number(CLIF_Z_AT(i)); }
UKL_INLINE uint32_t clif_f16z_start_stated(CLIF_T_ARGS, uint32_t i) { return clif_f16b_start_stated(CLIF_Z_AT(i)); }
UKL_INLINE uint32_t clif_f16z_end_stated(CLIF_T_ARGS, uint32_t i) { return clif_f16b_end_stated(CLIF_Z_AT(i)); }
UKL_INLINE uint32_t clif_f16z_length(CLIF_T_ARGS, uint32_t i) { return clif_f16b_length(CLIF_Z_AT(i)); }
UKL_INLINE uint32_t clif_f16z_start_next(CLIF_T_ARGS, uint32_t i, uint32_t prev) { return clif_f16b_start_next(CLIF_Z_AT(i), prev); }

// The node's parent, or NONE at the root: format 16's walk back over the slots, each read in its block, the block's
// entry found again only where the walk crosses into another.
UKL_INLINE uint32_t clif_f16z_parent(CLIF_T_ARGS, uint32_t i)
{
    uint64_t m = clif_z_mask(v);
    uint32_t held = CLIF_T_NONE;
    uint64_t e = 0u;
    uint32_t j = i;
    while (j != 0u) {
        uint32_t before = j - 1u;
        if ((before >> 10) != held) {
            held = before >> 10;
            e = clif_z_block(t, v, held);
        }
        uint64_t s = clif_f16b_slot(t, e, m, before & (CLIF_T_Z_BLOCK - 1u));
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

// The newline index: held raw, in place, as format 16 holds it.
UKL_INLINE uint32_t clif_f16z_line(CLIF_T_ARGS, uint32_t x) { return clif_f16_line(t, v, x); }
UKL_INLINE uint32_t clif_f16z_line_start(CLIF_T_ARGS, uint32_t k) { return clif_f16_line_start(t, v, k); }
