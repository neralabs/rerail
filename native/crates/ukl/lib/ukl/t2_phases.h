// ukl/t2_phases.h: tier 2 as phases on the device (decision 0726 step 4), over `<ukl/phases_run.h>`: from tier 1's
// records (`starts`, its pool's count) the phases the CPU's `Phases::run` runs, each item a chunk of `size` records,
// each block operator the grammar's, printed from the Rust the CPU runs (`ukl_t2_*`).
//
//   LAY  one item: the layout from the records (each tree's and array's offset in the pool), or the words the file
//        needs past the pool's room (`UKL_T2_NEED`: every later phase then has no items, the host runs it again).
//   P1   each chunk's depth summary (`p1_run`), a leaf of P1's tree (`p1_merge`).
//   P1W  each chunk's sum walked down, its records' depths and items written from it (`p1_write`); item 0 says the
//        items (P1's root), and the level sort's chunks (its trees' leaves).
//   P2   the level sort, two digits of 8 bits, each four phases: C, each chunk of items' keyed counts (`p2_run`), a
//        leaf of the digit's tree (`p2_merge`); K, each key's total (the tree's root) a leaf of the keys' tree
//        (`add1_merge`); KW, each key's start (its sum) written; S, each chunk's items scattered at their keys' starts
//        and its counts before them (`p2_write`, its sum counted on in place). The items from `items` to `items2`, and
//        back.
//   P3   over the items in level order, each chunk's segmented last-value of the heads (`p3_run`, `last_merge`); P3W
//        each record's container and each head's place written from its sum (`p3_write`).
//   P4   each chunk walked from every place (`ukl_t2_walk`), its levels known (`known_rets`): its map of places, a leaf
//        of P4's tree (`places_merge_w`); P4W each chunk's place (its sum's at place 0), and the first chunk refused
//        at its place (a min).
//   P5   each chunk from its place: its shape (`ukl_t2_walk`'s events folded), a leaf of P5's tree (`shape_merge`);
//        P5W each chunk's sum written, and its walk again writing each event at its number, with its items; item 0
//        says the events, their items and the nodes (P5's root).
//   P7   the events' items sorted by frame depth, as P2 (`ukl_t2_sort`), from `eitems` to `eitems2` and back.
//   P8   over the events' items in frame order: P8a each item's frame, the segmented last-value of the heads (`p3_run`,
//        `last_merge`, `p8a_write`); P8b each item's frame's children so far and the last one's node (`p8b_run`,
//        `kids_merge`, `p8b_write`).
//   P6   the tape's rows: MAKE each of its blocks (`make_blocks`, from P5's nodes); P6C and P6W each chunk from its
//        state (its place, its levels, its frames by `chunk_frames`, its sum's nodes and last start) on the cursor
//        from the middle on the writer's region `t` (`write_step`), counting its side lists' entries, then writing.
//   F    END each block's side lists counted, the last block ending the middle (`end_middle`); FIN each block final,
//        its laid bytes and faults a leaf of B (an add); TAIL the tail from B's root; PIECES each block's lists laid
//        from its sum, and the newline index, where they go.
//   Z    16z by phases (`Writer::finish_by_phases_with` and the device's encoder, `lz4_greedy`), from the finished
//        tape: ZP1 each piece's bytes where they lie, their hash and their LZ4 block's length (a group a piece, its
//        table the group's); ZS the pieces sorted by their hash's low 16 bits (the level sort); ZH each sorted piece's
//        run of like hashes from its head (a Last scan) and ZF its first piece of the same bytes in it, compared; ZP3
//        each piece's stored bytes (a repeat's 4, its LZ4 block's, or its own) and their prefix sum; ZP3W each one's
//        place; ZP4 each piece at its place (a group a piece: its LZ4 block made again, written), the head, the piece
//        index and the tail's runs.

#pragma once
#include <ukl/writer_mid.h>
#include <ukl/z16_run.h>
#define UKL_PH_GROUP
#define UKL_PH_FIELDS UKL_GLOBAL const uint8_t* src; UKL_GLOBAL const uint32_t* starts; UKL_GLOBAL uint32_t* arrays; UKL_GLOBAL const uint32_t* toks; \
    UKL_GLOBAL uint8_t* rets; UKL_GLOBAL const uint8_t* t; UKL_GLOBAL uint32_t* f2; UKL_GLOBAL uint8_t* z; UKL_LOCAL uint32_t* table; uint32_t n; uint32_t len; bool refused;
#include <ukl/phases_run.h>

// The program's header, after the runner's: the host's (the records a chunk, the pool's words, the arrays' words), then
// the layout: the trees in the pool, the arrays (what later phases read: never read in the launch that writes them) in
// `arrays`.
#define UKL_T2_SIZE (UKL_PH_H + 0u)
#define UKL_T2_ROOM (UKL_PH_H + 1u)
#define UKL_T2_AROOM (UKL_PH_H + 8u)
#define UKL_T2_NEED (UKL_PH_H + 2u)
#define UKL_T2_N (UKL_PH_H + 3u)
#define UKL_T2_CHUNKS (UKL_PH_H + 4u)
#define UKL_T2_O_DEPTH (UKL_PH_H + 5u)
#define UKL_T2_O_ITEMS (UKL_PH_H + 6u)
#define UKL_T2_ITEMS (UKL_PH_H + 7u)
#define UKL_T2_ICHUNKS (UKL_PH_H + 9u)
#define UKL_T2_O_ITEMS2 (UKL_PH_H + 10u)
#define UKL_T2_O_KEYS (UKL_PH_H + 11u)
#define UKL_T2_O_CONTAINER (UKL_PH_H + 12u)
#define UKL_T2_O_HEAD_AT (UKL_PH_H + 13u)
#define UKL_T2_O_PLACE (UKL_PH_H + 14u)
#define UKL_T2_O_SHAPE_AT (UKL_PH_H + 15u)
#define UKL_T2_O_EVENTS (UKL_PH_H + 16u)
#define UKL_T2_O_EITEMS (UKL_PH_H + 17u)
// The first chunk refused at its place (`NONE`: none), the events, their items, the nodes.
#define UKL_T2_REFUSED_AT (UKL_PH_H + 18u)
#define UKL_T2_EVENTS (UKL_PH_H + 19u)
#define UKL_T2_EITEMS (UKL_PH_H + 20u)
#define UKL_T2_NODES (UKL_PH_H + 21u)
#define UKL_T2_O_EITEMS2 (UKL_PH_H + 22u)
#define UKL_T2_ECHUNKS (UKL_PH_H + 23u)
#define UKL_T2_O_EKEYS (UKL_PH_H + 24u)
#define UKL_T2_O_FRAME_OF (UKL_PH_H + 25u)
#define UKL_T2_O_KIDS (UKL_PH_H + 26u)
#define UKL_T2_NB (UKL_PH_H + 27u)
// The host's: the blocks its region has room for.
#define UKL_T2_MOST (UKL_PH_H + 28u)
#define UKL_T2_G_END (UKL_PH_H + 29u)
#define UKL_T2_ENDED (UKL_PH_H + 30u)
#define UKL_T2_TAILED (UKL_PH_H + 31u)
// The finish's layout (`UKL_T2_F_*` words from it).
#define UKL_T2_F (UKL_PH_H + 32u)
#define UKL_T2_F_TAIL 0u
#define UKL_T2_F_BLOCKS_END 10u
#define UKL_T2_F_HOME 11u
#define UKL_T2_F_ERRORS0 12u
#define UKL_T2_F_FAULTS 13u
#define UKL_T2_F_WORDS 14u
#define UKL_T2_F_COUNT 15u
#define UKL_T2_F_LAID0 16u
#define UKL_T2_F_SIZE 17u
// 16z's: its room (the host's), its pieces, their chunks, its arrays, its tape's end.
#define UKL_T2_ZROOM (UKL_PH_H + 50u)
#define UKL_T2_NP (UKL_PH_H + 51u)
#define UKL_T2_ZCHUNKS (UKL_PH_H + 52u)
#define UKL_T2_O_ZR (UKL_PH_H + 53u)
#define UKL_T2_O_ZL (UKL_PH_H + 54u)
#define UKL_T2_O_ZITEMS (UKL_PH_H + 55u)
#define UKL_T2_O_ZITEMS2 (UKL_PH_H + 56u)
#define UKL_T2_O_ZKEYS (UKL_PH_H + 57u)
#define UKL_T2_O_ZREP (UKL_PH_H + 58u)
#define UKL_T2_O_ZSTORED (UKL_PH_H + 59u)
#define UKL_T2_O_ZSTART (UKL_PH_H + 60u)
#define UKL_T2_O_ZKIND (UKL_PH_H + 61u)
#define UKL_T2_ZEND (UKL_PH_H + 62u)
#define UKL_T2_G_FIN (UKL_PH_H + 64u)
#define UKL_T2_FIN_TARGET (UKL_PH_H + 65u)
// 1 once the tape is finished.
#define UKL_T2_DONE (UKL_PH_H + 66u)
#define UKL_T2_NLINES (UKL_PH_H + 67u)
// Each chunk's stop where it failed (the first: its kind and chunk).
#define UKL_T2_STOPPED (UKL_PH_H + 68u)
#define UKL_T2_H (UKL_PH_H + 80u)
#define UKL_T2_LINE_BYTES 512u

#define UKL_T2_PH_LAY 0u
#define UKL_T2_PH_P1 1u
#define UKL_T2_PH_P1W 2u
// The level sort's digit `d`'s four: `UKL_T2_PH_P2 + 4d` its C, then K, KW and S.
#define UKL_T2_PH_P2 3u
#define UKL_T2_PH_P3 11u
#define UKL_T2_PH_P3W 12u
#define UKL_T2_PH_P4 13u
#define UKL_T2_PH_P4W 14u
#define UKL_T2_PH_P5 15u
#define UKL_T2_PH_P5W 16u
#define UKL_T2_PH_P7 17u
#define UKL_T2_PH_P8A 25u
#define UKL_T2_PH_P8AW 26u
#define UKL_T2_PH_P8B 27u
#define UKL_T2_PH_P8BW 28u
#define UKL_T2_PH_MAKE 29u
#define UKL_T2_PH_P6C 30u
#define UKL_T2_PH_P6W 31u
#define UKL_T2_PH_END 32u
#define UKL_T2_PH_FIN 33u
#define UKL_T2_PH_TAIL 34u
#define UKL_T2_PH_PIECES 35u
#define UKL_T2_PH_ZP1 36u
#define UKL_T2_PH_ZS 37u
#define UKL_T2_PH_ZH 45u
#define UKL_T2_PH_ZF 46u
#define UKL_T2_PH_ZP3 47u
#define UKL_T2_PH_ZP3W 48u
#define UKL_T2_PH_ZP4 49u
#define UKL_T2_PHASES 50u
// A repeat's fewest bytes, and its kind's (`writer16`'s REPEAT_MIN, RAW, LZ4, REPEAT).
#define UKL_T2_REPEAT_MIN 8u
#define UKL_T2_ZRAW 0u
#define UKL_T2_ZLZ4 1u
#define UKL_T2_ZREPEAT 2u

#define UKL_T2_TP1 0u
// Digit `d`'s trees: its chunks' counts `UKL_T2_TP2 + 2d`, its keys' after it.
#define UKL_T2_TP2 1u
#define UKL_T2_TP3 5u
#define UKL_T2_TP4 6u
#define UKL_T2_TP5 7u
#define UKL_T2_TP7 8u
#define UKL_T2_TP8A 12u
#define UKL_T2_TP8B 13u
#define UKL_T2_TB 14u
// 16z's: the sort's (digit `d`'s counts `UKL_T2_TZ2 + 2d`, its keys' after), the heads' (Last), the stored bytes'.
#define UKL_T2_TZ2 15u
#define UKL_T2_TZH 19u
#define UKL_T2_TZ3 20u
#define UKL_T2_OP_P1 0u
#define UKL_T2_OP_ADD1 1u
#define UKL_T2_OP_KEYS 2u
#define UKL_T2_OP_LAST 3u
#define UKL_T2_OP_PLACES 4u
#define UKL_T2_OP_SHAPE 5u
#define UKL_T2_OP_KIDS 6u
// A place map's words (the grammar's places, at most this many), a shape's.
#define UKL_T2_W_PLACES 32u
#define UKL_T2_W_SHAPE 10u
#define UKL_T2_NONE 4294967295u
#define UKL_T2_W_P1 3u
#define UKL_T2_KEYS 256u

// The grammar's block operators (the printer's).
UKL_INLINE void ukl_t2_p1_run(UKL_GLOBAL const uint8_t* src, UKL_GLOBAL const uint32_t* starts, uint32_t n, uint32_t a, uint32_t b, UKL_COHERENT uint32_t* out);
UKL_INLINE void ukl_t2_p2_run(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, uint32_t shift, UKL_COHERENT uint32_t* out);
UKL_INLINE void ukl_t2_p2_write(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, uint32_t shift, UKL_COHERENT uint32_t* sum, UKL_GLOBAL const uint32_t* keys,
                                UKL_GLOBAL uint32_t* out);
UKL_INLINE void ukl_t2_p3_run(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, UKL_COHERENT uint32_t* out);
UKL_INLINE void ukl_t2_p3_write(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, UKL_COHERENT const uint32_t* sum, UKL_GLOBAL uint32_t* container,
                                UKL_GLOBAL uint32_t* head_at);
UKL_INLINE void ukl_t2_p8a_write(UKL_GLOBAL const uint32_t* items, uint32_t a, uint32_t b, UKL_COHERENT const uint32_t* sum, UKL_GLOBAL uint32_t* frame_of);
UKL_INLINE void ukl_t2_p8b_run(UKL_GLOBAL const uint32_t* items, UKL_GLOBAL const uint32_t* events, uint32_t a, uint32_t b, UKL_COHERENT uint32_t* out);
UKL_INLINE void ukl_t2_p8b_write(UKL_GLOBAL const uint32_t* items, UKL_GLOBAL const uint32_t* events, uint32_t a, uint32_t b, UKL_COHERENT const uint32_t* sum,
                                 UKL_GLOBAL uint32_t* kids);
UKL_INLINE uint32_t ukl_t2_write(UKL_GLOBAL const uint8_t* src, UKL_GLOBAL const uint32_t* starts, uint32_t n, uint32_t len, UKL_GLOBAL const uint8_t* t,
                                 UKL_GLOBAL const uint32_t* depth, UKL_GLOBAL const uint32_t* container, uint32_t low, uint32_t a, uint32_t b, uint32_t pl,
                                 UKL_GLOBAL uint8_t* ret, UKL_GLOBAL const uint32_t* eitems, uint32_t ne, UKL_GLOBAL const uint32_t* frame_of,
                                 UKL_GLOBAL const uint32_t* kids, UKL_GLOBAL const uint32_t* events, UKL_GLOBAL const uint32_t* at, uint32_t slow,
                                 UKL_GLOBAL uint32_t* room, bool count);
UKL_INLINE uint32_t ukl_t2_places(void);
UKL_INLINE uint32_t ukl_t2_depth(void);
UKL_INLINE uint32_t ukl_t2_walk_refused(void);
UKL_INLINE uint32_t ukl_t2_walk(UKL_GLOBAL const uint8_t* src, UKL_GLOBAL const uint32_t* starts, uint32_t n, uint32_t len, UKL_GLOBAL const uint32_t* toks,
                                UKL_GLOBAL const uint32_t* depth, UKL_GLOBAL const uint32_t* container, uint32_t low, uint32_t a, uint32_t b, uint32_t pl,
                                UKL_GLOBAL uint8_t* ret, UKL_PRIVATE uint32_t* s, bool write, UKL_GLOBAL uint32_t* ev, UKL_GLOBAL uint32_t* items, UKL_COHERENT const uint32_t* at);
UKL_INLINE void ukl_t2_p1_write(UKL_GLOBAL const uint8_t* src, UKL_GLOBAL const uint32_t* starts, uint32_t n, uint32_t a, uint32_t b, UKL_COHERENT const uint32_t* sum,
                                UKL_GLOBAL uint32_t* depth, UKL_GLOBAL uint32_t* items, UKL_GLOBAL uint32_t* head_at);

// Tree `t`'s layout at `at`: `leaves` leaves of `w` words, op `op`; past it. Its nodes (`UKL_PH_NODES`: each level's
// half its level below, rounded up) and two sum records a leaf.
UKL_INLINE uint32_t ukl_t2_tree(UklPhPool m, uint32_t t, uint32_t at, uint32_t leaves, uint32_t w, uint32_t op)
{
    UklPhPool x = m + UKL_PH_TREE + UKL_PH_TW * t;
    x[0] = at;
    x[1] = at + UKL_PH_NODES(leaves) * w;
    x[2] = w;
    x[3] = leaves;
    x[4] = op;
    return x[1] + 2u * leaves * w;
}

UKL_INLINE void ukl_t2_lay(UKL_PRIVATE UklPh* r)
{
    UklPhPool m = r->m;
    uint32_t n = r->n, size = m[UKL_T2_SIZE];
    uint32_t chunks = (n + 1u + size - 1u) / size;
    m[UKL_T2_N] = n;
    m[UKL_T2_CHUNKS] = chunks;
    uint32_t at = ukl_t2_tree(m, UKL_T2_TP1, UKL_T2_H, chunks, UKL_T2_W_P1, UKL_T2_OP_P1);
    // The level sort's trees, at the items' bound (two a record): P1W says their leaves.
    uint32_t most = 2u * (n + 1u), ichunks = (most + size - 1u) / size;
    for (uint32_t d = 0u; d < 2u; d++) {
        at = ukl_t2_tree(m, UKL_T2_TP2 + 2u * d, at, ichunks, UKL_T2_KEYS, UKL_T2_OP_KEYS);
        at = ukl_t2_tree(m, UKL_T2_TP2 + 2u * d + 1u, at, UKL_T2_KEYS, 1u, UKL_T2_OP_ADD1);
    }
    at = ukl_t2_tree(m, UKL_T2_TP3, at, ichunks, 2u, UKL_T2_OP_LAST);
    at = ukl_t2_tree(m, UKL_T2_TP4, at, chunks, UKL_T2_W_PLACES, UKL_T2_OP_PLACES);
    at = ukl_t2_tree(m, UKL_T2_TP5, at, chunks, UKL_T2_W_SHAPE, UKL_T2_OP_SHAPE);
    // The events' items' trees, at their bound (P5W says their leaves).
    uint32_t most_ev = 2u * (n + 1u) + 2u, echunks = (2u * most_ev + size - 1u) / size;
    for (uint32_t d = 0u; d < 2u; d++) {
        at = ukl_t2_tree(m, UKL_T2_TP7 + 2u * d, at, echunks, UKL_T2_KEYS, UKL_T2_OP_KEYS);
        at = ukl_t2_tree(m, UKL_T2_TP7 + 2u * d + 1u, at, UKL_T2_KEYS, 1u, UKL_T2_OP_ADD1);
    }
    at = ukl_t2_tree(m, UKL_T2_TP8A, at, echunks, 2u, UKL_T2_OP_LAST);
    at = ukl_t2_tree(m, UKL_T2_TP8B, at, echunks, 3u, UKL_T2_OP_KIDS);
    // The tape's blocks at the host's room (`most`): B's leaves the blocks MAKE makes.
    at = ukl_t2_tree(m, UKL_T2_TB, at, m[UKL_T2_MOST], 5u, UKL_PH_OP_ADD);
    // 16z's pieces at their bound, four a block.
    uint32_t zchunks = (4u * m[UKL_T2_MOST] + size - 1u) / size;
    for (uint32_t d = 0u; d < 2u; d++) {
        at = ukl_t2_tree(m, UKL_T2_TZ2 + 2u * d, at, zchunks, UKL_T2_KEYS, UKL_T2_OP_KEYS);
        at = ukl_t2_tree(m, UKL_T2_TZ2 + 2u * d + 1u, at, UKL_T2_KEYS, 1u, UKL_T2_OP_ADD1);
    }
    at = ukl_t2_tree(m, UKL_T2_TZH, at, zchunks, 2u, UKL_T2_OP_LAST);
    at = ukl_t2_tree(m, UKL_T2_TZ3, at, zchunks, 1u, UKL_PH_OP_ADD);
    m[UKL_T2_STOPPED] = UKL_T2_NONE;
    m[UKL_T2_REFUSED_AT] = UKL_T2_NONE;
    uint32_t aw = 0u;
    m[UKL_T2_O_DEPTH] = aw;
    aw += n + 1u;
    m[UKL_T2_O_ITEMS] = aw;
    aw += 3u * most;
    m[UKL_T2_O_ITEMS2] = aw;
    aw += 3u * most;
    m[UKL_T2_O_KEYS] = aw;
    aw += 2u * UKL_T2_KEYS;
    m[UKL_T2_O_CONTAINER] = aw;
    aw += n + 1u;
    m[UKL_T2_O_HEAD_AT] = aw;
    aw += n + 1u;
    m[UKL_T2_O_PLACE] = aw;
    aw += chunks;
    m[UKL_T2_O_SHAPE_AT] = aw;
    aw += UKL_T2_W_SHAPE * chunks;
    // The events at their bound (two a record, and two), their items two an event.
    m[UKL_T2_O_EVENTS] = aw;
    aw += 8u * most_ev;
    m[UKL_T2_O_EITEMS] = aw;
    aw += 6u * most_ev;
    m[UKL_T2_O_EITEMS2] = aw;
    aw += 6u * most_ev;
    m[UKL_T2_O_EKEYS] = aw;
    aw += 2u * UKL_T2_KEYS;
    m[UKL_T2_O_FRAME_OF] = aw;
    aw += 2u * most_ev;
    m[UKL_T2_O_KIDS] = aw;
    aw += 6u * most_ev;
    // Past a room, or past the arrivals' stride: what it needs (the pool's words, else the arrays').
    uint32_t zp = 4u * m[UKL_T2_MOST];
    m[UKL_T2_O_ZR] = aw;
    aw += zp;
    m[UKL_T2_O_ZL] = aw;
    aw += zp;
    m[UKL_T2_O_ZITEMS] = aw;
    aw += 3u * zp;
    m[UKL_T2_O_ZITEMS2] = aw;
    aw += 3u * zp;
    m[UKL_T2_O_ZKEYS] = aw;
    aw += 2u * UKL_T2_KEYS;
    m[UKL_T2_O_ZREP] = aw;
    aw += zp;
    m[UKL_T2_O_ZSTORED] = aw;
    aw += zp;
    m[UKL_T2_O_ZSTART] = aw;
    aw += zp;
    m[UKL_T2_O_ZKIND] = aw;
    aw += zp;
    if (at > m[UKL_T2_ROOM] || UKL_PH_NODES(ichunks) > m[UKL_PH_MS] || UKL_PH_NODES(echunks) > m[UKL_PH_MS] || UKL_PH_NODES(m[UKL_T2_MOST]) > m[UKL_PH_MS] || UKL_PH_NODES(zchunks) > m[UKL_PH_MS])
        m[UKL_T2_NEED] = at;
    else if (aw > m[UKL_T2_AROOM])
        m[UKL_T2_NEED] = aw;
    ukl_fence_device();
}

// A level sort (P2 of the records' items, P7 of the events'), its trees from `t0` (digit `d`'s counts `t0 + 2d`, its
// keys' after): digit `d`'s step `k` (C, K, KW, S), item `c`, over `ni` items from `oa` to `ob` (digit 1 back), the
// keys' starts at `okeys`.
UKL_INLINE void ukl_t2_sort(UKL_PRIVATE UklPh* r, uint32_t t0, uint32_t d, uint32_t k, uint32_t c, uint32_t ni, uint32_t oa, uint32_t ob, uint32_t okeys)
{
    uint32_t size = r->m[UKL_T2_SIZE], tc = t0 + 2u * d, tk = tc + 1u, ia = c * size, ib = ia + size < ni ? ia + size : ni;
    UKL_GLOBAL uint32_t* from = r->arrays + (d == 0u ? oa : ob);
    UKL_GLOBAL uint32_t* to = r->arrays + (d == 0u ? ob : oa);
    UKL_GLOBAL uint32_t* keys = r->arrays + okeys + d * UKL_T2_KEYS;
    if (k == 0u) {
        ukl_t2_p2_run(from, ia, ib, 8u * d, ukl_ph_leaf(r, tc, c));
        ukl_ph_up(r, tc, c);
    } else if (k == 1u) {
        ukl_ph_leaf(r, tk, c)[0] = ukl_ph_total(r, tc)[c];
        ukl_ph_up(r, tk, c);
    } else if (k == 2u) {
        keys[c] = ukl_ph_sum(r, tk, c)[0];
    } else {
        ukl_t2_p2_write(from, ia, ib, 8u * d, ukl_ph_sum(r, tc, c), keys, to);
    }
}

// ---- The finish (the old middle's, on the phases' pool and B's tree) --------------------------------------------

// The first failure kept.
UKL_INLINE void ukl_t2f_fail(UKL_PRIVATE UklPh* r, uint32_t code) { ukl_ph_fail(r, code); }

// A counter's arrival: whether it brought it to `target`.
UKL_INLINE bool ukl_t2f_arrive(UKL_PRIVATE UklPh* r, uint32_t at, uint32_t target)
{
    ukl_fence_device();
    bool last = ukl_atomic_add(&r->m[at], 1u) + 1u == target;
    ukl_fence_device();
    return last;
}

UKL_INLINE uint32_t ukl_t2f_nodes_blocks(uint32_t n)
{
    uint32_t nb = (n + CLIF_W_Z - 1u) / CLIF_W_Z;
    return nb == 0u ? 1u : nb;
}

// `len` bytes from `from` to `to` in the region, each byte its own store at the ends (a neighbour's bytes another item
// writes are never read and written back), aligned words between.
UKL_INLINE void ukl_t2f_copy(UKL_GLOBAL const uint8_t* t, uint64_t to, uint64_t from, uint64_t len)
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
UKL_INLINE void ukl_t2f_put_bytes(UKL_GLOBAL const uint8_t* t, uint64_t to, uint64_t v, uint32_t w)
{
    for (uint32_t k = 0u; k < w; k++)
        ukl_w_st8(t, to + k, (v >> (8u * k)) & 0xffu);
}

// Bit `bit` of the presence's words from `word0` at `base`, set by an atomic (blocks set the document's at once).
UKL_INLINE void ukl_t2f_set_bit(UKL_GLOBAL const uint8_t* t, uint64_t base, uint64_t word0, uint32_t bit)
{
    uint64_t at = base + (((uint64_t)(bit >> 6) + word0) << 3) + 4u * ((bit >> 5) & 1u);
    ukl_atomic_or((UKL_GLOBAL uint32_t*)(t + at), 1u << (bit & 31u));
}

// The presence word at `at` given `bits` (its halves by atomics, each only where it has a bit).
UKL_INLINE void ukl_t2f_or_word(UKL_GLOBAL const uint8_t* t, uint64_t at, uint64_t bits)
{
    if ((uint32_t)bits != 0u)
        ukl_atomic_or((UKL_GLOBAL uint32_t*)(t + at), (uint32_t)bits);
    if ((uint32_t)(bits >> 32) != 0u)
        ukl_atomic_or((UKL_GLOBAL uint32_t*)(t + at + 4u), (uint32_t)(bits >> 32));
}

// Block `b`'s rows, its node `j`'s number and its kind field (`finalize`'s), its codes' side list's cursor `low`.
UKL_INLINE uint32_t ukl_t2f_kind_field(UKL_GLOBAL const uint8_t* t, uint64_t room, UklWList codes, uint32_t j, UKL_PRIVATE uint32_t* low)
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

UKL_INLINE uint32_t ukl_t2f_rows(UKL_PRIVATE UklPh* r, uint32_t b)
{
    uint32_t n = r->m[UKL_T2_NODES], rows = n - b * CLIF_W_Z;
    return rows < CLIF_W_Z ? rows : CLIF_W_Z;
}

// Side list `q`'s laid bytes in a block of `rows` (`lay`'s): each sub-block's entries' end, their offsets, values.
UKL_INLINE uint32_t ukl_t2f_laid_len(UKL_GLOBAL const uint8_t* t, uint64_t area, uint32_t q, uint32_t rows)
{
    UklWList l = ukl_w_list(t, area, q);
    uint32_t subs = (rows + CLIF_W_BLOCK - 1u) >> 8;
    return l.count != 0u ? 2u * subs + l.count + l.count * (uint32_t)ukl_w_get32(t, 0u, CLIF_W_AT_VW + 4u * q) : 0u;
}

// FIN(b): block `b` final, as `finalize` makes it but where its bytes go: its presence (the document's by atomics, its
// own where blocks are kept) and its table entry; its counts a leaf of B: each side list's laid bytes, its faults.
UKL_ONCE void ukl_t2f_fin(UKL_PRIVATE UklPh* r, uint32_t b)
{
    UKL_GLOBAL const uint8_t* t = r->t;
    uint64_t e = ukl_w_entry(t, 0u, b);
    if (ukl_w_ld32(t, e + 8u) != 0u)
        ukl_t2f_fail(r, 2u);
    ukl_w_st32(t, e + 8u, 1u);
    uint32_t rows = ukl_t2f_rows(r, b);
    uint64_t room = ukl_w_room_of(t, 0u, b), area = ukl_w_area_of(t, 0u, b);
    UklWList codes = ukl_w_list(t, area, 0u);
    uint64_t presence = ukl_w_part(t, 0u, CLIF_W_PART_PRESENCE), kw = ukl_w_get32(t, 0u, CLIF_W_AT_KW);
    bool keep = (ukl_w_get32(t, 0u, CLIF_W_AT_FLAGS) & 1u) != 0u;
    uint64_t mine = presence + ((uint64_t)b + 1u) * ukl_w_presence_bytes(t, 0u);
    // The block's kinds and fields gathered in registers (each presence's first word: the bits below 64), and set in
    // the document's words once a block: every block sets the same few words, so an atomic a node serialized them all.
    // A bit past 64 is set as it is met.
    uint32_t low = 0u, faults = 0u;
    uint64_t kinds = 0u, fields = 0u;
    for (uint32_t j = 0u; j < rows; j++) {
        uint32_t kf = ukl_t2f_kind_field(t, room, codes, j, &low);
        uint32_t kind = kf & 0x7fffu, field = kf >> 16, fault = (kf >> 15) & 1u;
        if (kind < 64u)
            kinds |= (uint64_t)1 << kind;
        else {
            ukl_t2f_set_bit(t, presence, 0u, kind);
            if (keep)
                ukl_w_set_bit(t, mine, 0u, kind);
        }
        if (field < 64u)
            fields |= (uint64_t)1 << field;
        else {
            ukl_t2f_set_bit(t, presence, kw, field);
            if (keep)
                ukl_w_set_bit(t, mine, kw, field);
        }
        kinds |= (uint64_t)fault;
        faults += kind == 0u || fault != 0u ? 1u : 0u;
    }
    ukl_t2f_or_word(t, presence, kinds);
    ukl_t2f_or_word(t, presence + 8u * kw, fields);
    if (keep) {
        ukl_w_st64(t, mine, ukl_w_ld64(t, mine) | kinds);
        ukl_w_st64(t, mine + 8u * kw, ukl_w_ld64(t, mine + 8u * kw) | fields);
    }
    // Its side area given back, in block order (`finalize`'s free list).
    uint64_t free0 = ukl_w_len(t, 0u, CLIF_W_PART_FREE);
    ukl_w_st32(t, ukl_w_part(t, 0u, CLIF_W_PART_FREE) + free0 + 4u * (uint64_t)b, ukl_w_ld32(t, e + 4u));
    UklPhPool v = ukl_ph_leaf(r, UKL_T2_TB, b);
    for (uint32_t q = 0u; q < 4u; q++)
        v[q] = ukl_t2f_laid_len(t, area, q, rows);
    v[4] = faults;
    ukl_ph_up(r, UKL_T2_TB, b);
}

// B's root (`finish_tail`'s): the state's lengths, the last block's columns down to its rows, the tail laid out and
// its zeros written, the header, the presence's header and the document's words; then each block's pieces (FIN2) and
// the newline index's (LINES) are written where they go.
UKL_ONCE bool ukl_t2f_tail(UKL_PRIVATE UklPh* r)
{
    UKL_GLOBAL const uint8_t* t = r->t;
    UklPhPool m = r->m;
    UklPhPool tot = ukl_ph_total(r, UKL_T2_TB);
    uint32_t n = m[UKL_T2_NODES], count = m[UKL_T2_NB], f = UKL_T2_F;
    uint64_t laid = (uint64_t)tot[0] + tot[1] + tot[2] + tot[3];
    uint64_t errors0 = ukl_w_len(t, 0u, CLIF_W_PART_ERRORS), nfaults = tot[4];
    m[f + UKL_T2_F_ERRORS0] = (uint32_t)errors0;
    if (ukl_w_room(t, 0u, CLIF_W_PART_ERRORS, errors0 + 4u * nfaults) != 0u) {
        ukl_t2f_fail(r, 4u);
        return false;
    }
    m[f + UKL_T2_F_LAID0] = (uint32_t)ukl_w_len(t, 0u, CLIF_W_PART_LAID);
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
        ukl_t2f_fail(r, 5u);
        return false;
    }
    ukl_w_st64(t, out + blocks_end, 0u);
    uint64_t pos = blocks_end + 8u, tail[10];
    for (uint32_t q = 0u; q < 4u; q++) {
        tail[q] = pos;
        pos += tot[q];
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
        m[f + UKL_T2_F_TAIL + k] = (uint32_t)tail[k];
    m[f + UKL_T2_F_BLOCKS_END] = (uint32_t)blocks_end;
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
    m[f + UKL_T2_F_HOME] = (uint32_t)home;
    m[f + UKL_T2_F_WORDS] = (uint32_t)(kw + fw);
    m[f + UKL_T2_F_COUNT] = (uint32_t)pcount;
    m[f + UKL_T2_F_FAULTS] = (uint32_t)(home + 16u + 8u * (kw + fw) * pcount);
    uint32_t nlines = (uint32_t)((lines_len + UKL_T2_LINE_BYTES - 1u) / UKL_T2_LINE_BYTES);
    m[UKL_T2_NLINES] = nlines;
    m[UKL_T2_FIN_TARGET] = count + nlines;
    return true;
}

// The tape finished: every block's pieces and the newline index written.
UKL_INLINE void ukl_t2f_finished(UKL_PRIVATE UklPh* r)
{
    if (ukl_t2f_arrive(r, UKL_T2_G_FIN, r->m[UKL_T2_FIN_TARGET]))
        ukl_atomic_store(&r->m[UKL_T2_DONE], 1u);
}

// FIN2(b), B's leaf: block `b`'s side lists laid where they go in the tape (`lay`'s bytes, a byte a store), their part
// starts, its faults in the ERRORS part and the presence, its own presence words.
UKL_ONCE void ukl_t2f_fin2(UKL_PRIVATE UklPh* r, uint32_t b, UklPhPool before)
{
    UKL_GLOBAL const uint8_t* t = r->t;
    UklPhPool m = r->m;
    uint32_t f = UKL_T2_F, count = m[UKL_T2_NB];
    UklPhPool mine = ukl_ph_leaf(r, UKL_T2_TB, b);
    uint32_t rows = ukl_t2f_rows(r, b);
    uint64_t out = ukl_w_part(t, 0u, CLIF_W_PART_OUT), area = ukl_w_area_of(t, 0u, b), room = ukl_w_room_of(t, 0u, b);
    uint64_t laid_before = (uint64_t)before[0] + before[1] + before[2] + before[3];
    uint64_t laid_at = ukl_w_part(t, 0u, CLIF_W_PART_LAID_AT) + (uint64_t)b * CLIF_W_LAID_AT;
    uint64_t laid0 = m[f + UKL_T2_F_LAID0];
    uint32_t subs = (rows + CLIF_W_BLOCK - 1u) >> 8;
    uint64_t in_block = 0u;
    for (uint32_t q = 0u; q < 4u; q++) {
        UklWList l = ukl_w_list(t, area, q);
        uint64_t vw = ukl_w_get32(t, 0u, CLIF_W_AT_VW + 4u * q), len = mine[q];
        uint64_t at = out + m[f + UKL_T2_F_TAIL + q] + before[q];
        if (l.count != 0u) {
            uint32_t k = 0u;
            for (uint32_t sub = 0u; sub < subs; sub++) {
                while (k < l.count && ukl_w_ld16(t, l.js + 2u * (uint64_t)k) >> 8 == sub)
                    k++;
                ukl_t2f_put_bytes(t, at + 2u * (uint64_t)sub, k, 2u);
            }
            uint64_t offsets = at + 2u * (uint64_t)subs;
            for (uint32_t j = 0u; j < l.count; j++)
                ukl_w_st8(t, offsets + j, ukl_w_ld16(t, l.js + 2u * (uint64_t)j) & 0xffu);
            uint64_t values = offsets + l.count;
            for (uint32_t j = 0u; j < l.count; j++)
                ukl_t2f_put_bytes(t, values + (uint64_t)j * vw, ukl_w_ld64(t, l.vs + 8u * (uint64_t)j), (uint32_t)vw);
        }
        // Its laid part's entry (the laid bytes in block order, each block's lists in turn), and its part start.
        ukl_w_st32(t, laid_at + 8u * q, (uint32_t)(laid0 + laid_before + in_block));
        ukl_w_st32(t, laid_at + 8u * q + 4u, (uint32_t)len);
        in_block += len;
        uint64_t starts = out + m[f + UKL_T2_F_TAIL + 4u + q];
        ukl_w_st32(t, starts + 4u * (uint64_t)b, before[q]);
        if (b + 1u == count)
            ukl_w_st32(t, starts + 4u * (uint64_t)count, before[q] + (uint32_t)len);
    }
    // Its faults, in row order after the blocks' before it: in the ERRORS part and the presence.
    UklWList codes = ukl_w_list(t, area, 0u);
    uint64_t errors = ukl_w_part(t, 0u, CLIF_W_PART_ERRORS) + m[f + UKL_T2_F_ERRORS0] + 4u * (uint64_t)before[4];
    uint64_t faults = out + m[f + UKL_T2_F_FAULTS] + 4u * (uint64_t)before[4];
    uint32_t low = 0u, k = 0u;
    for (uint32_t j = 0u; j < rows; j++) {
        uint32_t kf = ukl_t2f_kind_field(t, room, codes, j, &low);
        if ((kf & 0x7fffu) == 0u || ((kf >> 15) & 1u) != 0u) {
            ukl_w_st32(t, errors + 4u * (uint64_t)k, b * CLIF_W_Z + j);
            ukl_w_st32(t, faults + 4u * (uint64_t)k, b * CLIF_W_Z + j);
            k++;
        }
    }
    // Its own presence words, where blocks are kept.
    uint32_t words = m[f + UKL_T2_F_WORDS];
    if (m[f + UKL_T2_F_COUNT] > 1u) {
        uint64_t from = ukl_w_part(t, 0u, CLIF_W_PART_PRESENCE) + ((uint64_t)b + 1u) * 8u * words;
        uint64_t to = out + m[f + UKL_T2_F_HOME] + 16u + ((uint64_t)b + 1u) * 8u * words;
        for (uint32_t w = 0u; w < words; w++)
            ukl_w_st64(t, to + 8u * w, ukl_w_ld64(t, from + 8u * w));
    }
    ukl_t2f_finished(r);
}

// LINES(k): bytes `512k..` of the newline index into the tail.
UKL_ONCE void ukl_t2f_lines(UKL_PRIVATE UklPh* r, uint32_t k)
{
    UKL_GLOBAL const uint8_t* t = r->t;
    uint64_t len = ukl_w_len(t, 0u, CLIF_W_PART_LINES), at = (uint64_t)k * UKL_T2_LINE_BYTES;
    uint64_t n = len - at < UKL_T2_LINE_BYTES ? len - at : UKL_T2_LINE_BYTES;
    uint64_t out = ukl_w_part(t, 0u, CLIF_W_PART_OUT) + r->m[UKL_T2_F + UKL_T2_F_TAIL + 8u];
    ukl_t2f_copy(t, out + at, ukl_w_part(t, 0u, CLIF_W_PART_LINES) + at, n);
    ukl_t2f_finished(r);
}

// Every block ended (`end_middle`): `finish_blocks`' checks and the last node's slot; then each block final.
UKL_ONCE void ukl_t2f_ended(UKL_PRIVATE UklPh* r)
{
    UKL_GLOBAL const uint8_t* t = r->t;
    UklPhPool m = r->m;
    UklPhPool end = ukl_ph_total(r, UKL_T2_TP5);
    uint32_t n = m[UKL_T2_NODES];
    ukl_wmid_ended(t, 0u, n, end[7] != 0u ? (uint64_t)end[5] | ((uint64_t)end[6] << 32) : 0u);
    if (ukl_w_get32(t, 0u, CLIF_W_AT_BAD) != 0u || ukl_w_get32(t, 0u, CLIF_W_AT_OPEN_LEN) != 0u || ukl_w_is_z(t, 0u)) {
        ukl_t2f_fail(r, 6u);
        return;
    }
    if (n != 0u && clif_w_put(t, 0u, 0u, n - 1u, 1u) != 0u) {
        ukl_t2f_fail(r, 7u);
        return;
    }
    // The rooms the blocks write into at once: the free list (a side area a block, in block order), the laid parts'.
    uint32_t nb = m[UKL_T2_NB];
    if (ukl_w_room(t, 0u, CLIF_W_PART_FREE, ukl_w_len(t, 0u, CLIF_W_PART_FREE) + 4u * (uint64_t)nb) != 0u
        || ukl_w_room(t, 0u, CLIF_W_PART_LAID_AT, (uint64_t)nb * CLIF_W_LAID_AT) != 0u) {
        ukl_t2f_fail(r, 4u);
        return;
    }
    ukl_atomic_store(&m[UKL_T2_ENDED], 1u);
}


// P6 and F's items: phase `ph`'s item `c` (a block, or a chunk for P6C and P6W).
UKL_INLINE void ukl_t2_finish_item(UKL_PRIVATE UklPh* r, uint32_t ph, uint32_t c)
{
    UklPhPool m = r->m;
    UKL_GLOBAL const uint8_t* t = r->t;
    uint32_t nb = m[UKL_T2_NB];
    if (ph == UKL_T2_PH_MAKE) {
        if (ukl_wmid_make_block(t, 0u, c) != 0u)
            ukl_t2f_fail(r, 9u);
        if (c == 0u && ukl_wmid_made(t, 0u, nb) != 0u)
            ukl_t2f_fail(r, 10u);
    } else if (ph == UKL_T2_PH_P6C || ph == UKL_T2_PH_P6W) {
        uint32_t n = r->n, size = m[UKL_T2_SIZE], a = c * size, b = a + size < n + 1u ? a + size : n + 1u, chunks = m[UKL_T2_CHUNKS];
        UKL_GLOBAL uint32_t* place = r->arrays + m[UKL_T2_O_PLACE];
        uint32_t places = ukl_t2_places();
        if (place[c] >= places)
            return;
        UKL_GLOBAL uint32_t* room = r->f2 + UKL_WMID_FRAME * 5u * (a + c);
        uint32_t x = ukl_t2_write(r->src, r->starts, n, r->len, t, r->arrays + m[UKL_T2_O_DEPTH], r->arrays + m[UKL_T2_O_CONTAINER], ukl_ph_leaf(r, UKL_T2_TP1, c)[1],
                                  a, b, place[c], r->rets + c * ukl_t2_depth(), r->arrays + m[UKL_T2_O_EITEMS], m[UKL_T2_EITEMS], r->arrays + m[UKL_T2_O_FRAME_OF],
                                  r->arrays + m[UKL_T2_O_KIDS], r->arrays + m[UKL_T2_O_EVENTS], r->arrays + m[UKL_T2_O_SHAPE_AT] + UKL_T2_W_SHAPE * c,
                                  ukl_ph_leaf(r, UKL_T2_TP5, c)[4], room, ph == UKL_T2_PH_P6C);
        // At its end; the file's end only in the last chunk the walk reaches.
        bool fine = x == 0u || (x == 1u && (c + 1u == chunks || place[c + 1u] >= places));
        if (!fine) {
            ukl_atomic_min(&m[UKL_T2_STOPPED], c);
            ukl_t2f_fail(r, 20u + x);
        }
    } else if (ph == UKL_T2_PH_END) {
        ukl_wmid_end_block(t, 0u, c);
        if (ukl_t2f_arrive(r, UKL_T2_G_END, nb))
            ukl_t2f_ended(r);
    } else if (ph == UKL_T2_PH_FIN) {
        ukl_t2f_fin(r, c);
    } else if (ph == UKL_T2_PH_TAIL) {
        if (ukl_t2f_tail(r))
            ukl_atomic_store(&m[UKL_T2_TAILED], 1u);
    } else if (c < nb) {
        ukl_t2f_fin2(r, c, ukl_ph_sum(r, UKL_T2_TB, c));
    } else {
        ukl_t2f_lines(r, c - nb);
    }
}

// ---- 16z by phases --------------------------------------------------------------------------------------------

// The finished tape: the writer's region's OUT part.
UKL_INLINE UKL_GLOBAL const uint8_t* ukl_t2z_tape(UKL_PRIVATE UklPh* r)
{
    return r->t + ukl_z16_u32(r->t, ukl_w_part_field(CLIF_W_PART_OUT, CLIF_W_AT_P_AT));
}

// The 16z fields (`head::Z`) of a tape of placement `p` whose pieces store `total` bytes: its part starts,
// checkpoints, newline index, piece starts, lengths, kinds, end.
UKL_INLINE void ukl_t2z_fields(UKL_PRIVATE const UklZ16P* p, uint32_t total, UKL_PRIVATE uint32_t* f)
{
    uint32_t np = 4u * p->count;
    f[0] = ukl_z16_round4(UKL_Z16_HEAD + total);
    f[1] = f[0] + (p->lines - p->parts[0]);
    uint32_t ck = p->count == 0u ? 0u : (p->count - 1u) * (UKL_Z16_Z / UKL_Z16_K) * p->cw + (ukl_z16_rows(p, p->count - 1u) + UKL_Z16_K - 1u) / UKL_Z16_K * p->cw;
    f[2] = f[1] + ck + 8u;
    uint32_t lines = p->end - 8u - p->lines;
    f[3] = ukl_z16_round4(f[2] + lines + 8u);
    f[4] = f[3] + 4u * np;
    f[5] = f[4] + 4u * np;
    f[6] = f[5] + np;
}

// The tail's runs of `UKL_Z16_COPY` bytes: the part starts', then the newline index's.
UKL_INLINE uint32_t ukl_t2z_copies(UKL_PRIVATE const UklZ16P* p)
{
    uint32_t lines = p->end - 8u - p->lines;
    return (p->lines - p->parts[0] + UKL_Z16_COPY - 1u) / UKL_Z16_COPY + (lines + UKL_Z16_COPY - 1u) / UKL_Z16_COPY;
}

// A piece's hash (equal bytes, equal hash): its words' (four bytes, from its two spans), each mixed with its place and
// summed across the group's lanes, with its length.
UKL_INLINE uint32_t ukl_t2z_hash(UklLz4Spans s, uint32_t n, uint32_t lane __UKL_PARAMS)
{
    uint32_t h = 0u;
    for (uint32_t k = lane; 4u * k < n; k += UKL_SIMD_WIDTH) {
        uint32_t w = 0u;
        for (uint32_t i = 0u; i < 4u && 4u * k + i < n; i++)
            w |= (uint32_t)ukl_lz4e_byte(s, 4u * k + i) << (8u * i);
        uint32_t x = (w ^ (k * 0x9e3779b9u)) * 0x85ebca6bu;
        h += x ^ (x >> 13);
    }
    h = ukl_sg_sum(h __UKL_ARGS) + n * 0xc2b2ae35u;
    return h ^ (h >> 16);
}

// Whether pieces `i` and `j` (of `n` bytes each) hold the same bytes.
UKL_INLINE bool ukl_t2z_same(UKL_GLOBAL const uint8_t* t, UKL_PRIVATE const UklZ16P* p, uint32_t i, uint32_t j)
{
    uint32_t ni = 0u, nj = 0u;
    UklLz4Spans a = ukl_z16_piece(t, p, i, &ni), b = ukl_z16_piece(t, p, j, &nj);
    if (ni != nj)
        return false;
    for (uint32_t k = 0u; k < ni; k++)
        if (ukl_lz4e_byte(a, k) != ukl_lz4e_byte(b, k))
            return false;
    return true;
}

// Phase `ph`'s items: the pieces (or their chunks), once the tape is finished; ZP4's the pieces, the head, the tail's
// runs.
UKL_INLINE uint32_t ukl_t2z_items(UKL_PRIVATE UklPh* r, uint32_t ph)
{
    UklPhPool m = r->m;
    if (ukl_atomic_poll(&m[UKL_PH_FAIL]) != 0u || ukl_atomic_poll(&m[UKL_T2_DONE]) != 1u)
        return 0u;
    if (ph == UKL_T2_PH_ZP1)
        return m[UKL_T2_NP];
    if (ph == UKL_T2_PH_ZP4) {
        UklZ16P p = ukl_z16_placement(ukl_t2z_tape(r));
        return m[UKL_T2_NP] + 1u + ukl_t2z_copies(&p);
    }
    // The sort's K and KW steps, over its keys.
    if (ph < UKL_T2_PH_ZH && ((ph - UKL_T2_PH_ZS) % 4u == 1u || (ph - UKL_T2_PH_ZS) % 4u == 2u))
        return UKL_T2_KEYS;
    return m[UKL_T2_ZCHUNKS];
}

// ZP1 (group items), piece `j`: its bytes' number, its hash's key (where it can repeat), its LZ4 block's length where
// it saves a tenth (`NONE` else); its item for the sort (key, piece, whether it can repeat).
UKL_INLINE void ukl_t2z_zp1(UKL_PRIVATE UklPh* r, uint32_t j, uint32_t lane __UKL_PARAMS)
{
    UklPhPool m = r->m;
    UKL_GLOBAL const uint8_t* t = ukl_t2z_tape(r);
    UklZ16P p = ukl_z16_placement(t);
    if (4u * p.count != m[UKL_T2_NP]) {
        if (lane == 0u)
            ukl_t2f_fail(r, 30u);
        return;
    }
    uint32_t raw = 0u;
    UklLz4Spans s = ukl_z16_piece(t, &p, j, &raw);
    uint32_t h = ukl_t2z_hash(s, raw, lane __UKL_ARGS);
    uint32_t len = UKL_NONE;
    if (raw >= UKL_Z16_LZ4_MIN)
        len = ukl_lz4_encode2_sg(s, raw, r->z, raw * 9u / 10u, false, r->table, lane __UKL_ARGS);
    if (lane == 0u) {
        bool keyed = raw >= UKL_T2_REPEAT_MIN;
        UKL_GLOBAL uint32_t* items = r->arrays + m[UKL_T2_O_ZITEMS];
        r->arrays[m[UKL_T2_O_ZR] + j] = raw;
        r->arrays[m[UKL_T2_O_ZL] + j] = len;
        items[3u * j] = keyed ? h & 0xffffu : 0u;
        items[3u * j + 1u] = j;
        items[3u * j + 2u] = keyed ? 1u : 0u;
    }
}

// ZP4 (group items): piece `j` at its place (its LZ4 block made again, written; its bytes; or the piece it repeats),
// its block's checkpoints with its first piece; then the head, its pads and the piece index; then the tail's runs.
UKL_INLINE void ukl_t2z_zp4(UKL_PRIVATE UklPh* r, uint32_t i, uint32_t lane __UKL_PARAMS)
{
    UklPhPool m = r->m;
    UKL_GLOBAL const uint8_t* t = ukl_t2z_tape(r);
    UKL_GLOBAL uint8_t* z = r->z;
    UklZ16P p = ukl_z16_placement(t);
    uint32_t np = m[UKL_T2_NP], f[7];
    ukl_t2z_fields(&p, ukl_ph_total(r, UKL_T2_TZ3)[0], f);
    if (f[6] > m[UKL_T2_ZROOM]) {
        if (lane == 0u)
            ukl_t2f_fail(r, 31u);
        return;
    }
    UKL_GLOBAL const uint32_t* stored = r->arrays + m[UKL_T2_O_ZSTORED];
    UKL_GLOBAL const uint32_t* start = r->arrays + m[UKL_T2_O_ZSTART];
    UKL_GLOBAL const uint32_t* kind = r->arrays + m[UKL_T2_O_ZKIND];
    if (i < np) {
        uint32_t raw = 0u;
        UklLz4Spans s = ukl_z16_piece(t, &p, i, &raw);
        UKL_GLOBAL uint8_t* to = z + UKL_Z16_HEAD + start[i];
        if (kind[i] == UKL_T2_ZLZ4) {
            if (ukl_lz4_encode2_sg(s, raw, to, stored[i], true, r->table, lane __UKL_ARGS) != stored[i])
                if (lane == 0u)
                    ukl_t2f_fail(r, 32u);
        } else if (kind[i] == UKL_T2_ZREPEAT) {
            if (lane == 0u)
                ukl_z16_put32(to, r->arrays[m[UKL_T2_O_ZREP] + i]);
        } else {
            ukl_z16_copy(s, raw, to, lane);
        }
        uint32_t b = i / 4u;
        if (i % 4u == 0u) {
            uint32_t n = (ukl_z16_rows(&p, b) + UKL_Z16_K - 1u) / UKL_Z16_K * p.cw;
            ukl_z16_copy1(t + ukl_z16_column(&p, b, 4u), n, z + f[1] + b * (UKL_Z16_Z / UKL_Z16_K) * p.cw, lane);
        }
    } else if (i == np) {
        uint32_t at = UKL_Z16_HEAD + ukl_ph_total(r, UKL_T2_TZ3)[0], lines = p.end - 8u - p.lines;
        for (uint32_t k = lane; k < UKL_Z16_HEAD; k += UKL_SIMD_WIDTH) {
            uint32_t w = (k - UKL_Z16_HZ) / 4u;
            z[k] = k == 7u ? (uint8_t)'Z' : k < UKL_Z16_HZ ? t[k] : w < 7u ? (uint8_t)(f[w] >> (8u * ((k - UKL_Z16_HZ) % 4u))) : 0u;
        }
        ukl_z16_zeros(z + at, f[0] - at, lane);
        ukl_z16_zeros(z + f[2] - 8u, 8u, lane);
        ukl_z16_zeros(z + f[2] + lines, f[3] - f[2] - lines, lane);
        for (uint32_t j = lane; j < np; j += UKL_SIMD_WIDTH) {
            ukl_z16_put32(z + f[3] + 4u * j, start[j]);
            ukl_z16_put32(z + f[4] + 4u * j, stored[j]);
            z[f[5] + j] = (uint8_t)kind[j];
        }
        if (lane == 0u)
            m[UKL_T2_ZEND] = f[6];
    } else {
        uint32_t k = i - np - 1u, parts = p.lines - p.parts[0], na = (parts + UKL_Z16_COPY - 1u) / UKL_Z16_COPY;
        uint32_t from = 0u, to = 0u, n = 0u;
        if (k < na) {
            from = p.parts[0] + k * UKL_Z16_COPY;
            to = f[0] + k * UKL_Z16_COPY;
            n = parts - k * UKL_Z16_COPY;
        } else {
            uint32_t x = (k - na) * UKL_Z16_COPY;
            from = p.lines + x;
            to = f[2] + x;
            n = p.end - 8u - p.lines - x;
        }
        ukl_z16_copy1(t + from, n < UKL_Z16_COPY ? n : UKL_Z16_COPY, z + to, lane);
    }
}

// 16z's lane items: the sort (ZS), the heads (ZH), each piece's first like it (ZF), the stored bytes (ZP3) and their
// places (ZP3W), each over chunk `c` (of the sorted items, or of the pieces by number).
UKL_INLINE void ukl_t2z_item(UKL_PRIVATE UklPh* r, uint32_t ph, uint32_t c)
{
    UklPhPool m = r->m;
    uint32_t np = m[UKL_T2_NP], size = m[UKL_T2_SIZE], a = c * size, b = a + size < np ? a + size : np;
    UKL_GLOBAL uint32_t* items = r->arrays + m[UKL_T2_O_ZITEMS];
    UKL_GLOBAL uint32_t* rep = r->arrays + m[UKL_T2_O_ZREP];
    if (ph < UKL_T2_PH_ZH) {
        uint32_t k = ph - UKL_T2_PH_ZS;
        ukl_t2_sort(r, UKL_T2_TZ2, k / 4u, k % 4u, c, np, m[UKL_T2_O_ZITEMS], m[UKL_T2_O_ZITEMS2], m[UKL_T2_O_ZKEYS]);
    } else if (ph == UKL_T2_PH_ZH) {
        // The last head's place in the chunk (a run of like keys starts at a head).
        UklPhPool leaf = ukl_ph_leaf(r, UKL_T2_TZH, c);
        uint32_t v = UKL_NONE;
        for (uint32_t x = a; x < b; x++)
            if (x == 0u || items[3u * (x - 1u)] != items[3u * x])
                v = x;
        leaf[0] = 0u;
        leaf[1] = v;
        ukl_ph_up(r, UKL_T2_TZH, c);
    } else if (ph == UKL_T2_PH_ZF) {
        UKL_GLOBAL const uint8_t* t = ukl_t2z_tape(r);
        UklZ16P p = ukl_z16_placement(t);
        uint32_t v = ukl_ph_sum(r, UKL_T2_TZH, c)[1];
        for (uint32_t x = a; x < b; x++) {
            if (x == 0u || items[3u * (x - 1u)] != items[3u * x])
                v = x;
            uint32_t j = items[3u * x + 1u], e = j;
            if (items[3u * x + 2u] != 0u)
                for (uint32_t k = v; k < x; k++)
                    if (items[3u * k + 2u] != 0u && ukl_t2z_same(t, &p, items[3u * k + 1u], j)) {
                        e = items[3u * k + 1u];
                        break;
                    }
            rep[j] = e;
        }
    } else if (ph == UKL_T2_PH_ZP3) {
        UKL_GLOBAL const uint32_t* zr = r->arrays + m[UKL_T2_O_ZR];
        UKL_GLOBAL const uint32_t* zl = r->arrays + m[UKL_T2_O_ZL];
        UKL_GLOBAL uint32_t* stored = r->arrays + m[UKL_T2_O_ZSTORED];
        UKL_GLOBAL uint32_t* kind = r->arrays + m[UKL_T2_O_ZKIND];
        uint32_t sum = 0u;
        for (uint32_t j = a; j < b; j++) {
            uint32_t k = rep[j] != j ? UKL_T2_ZREPEAT : zl[j] != UKL_NONE ? UKL_T2_ZLZ4 : UKL_T2_ZRAW;
            uint32_t n = k == UKL_T2_ZREPEAT ? 4u : k == UKL_T2_ZLZ4 ? zl[j] : zr[j];
            kind[j] = k;
            stored[j] = n;
            sum += n;
        }
        ukl_ph_leaf(r, UKL_T2_TZ3, c)[0] = sum;
        ukl_ph_up(r, UKL_T2_TZ3, c);
    } else {
        UKL_GLOBAL const uint32_t* stored = r->arrays + m[UKL_T2_O_ZSTORED];
        UKL_GLOBAL uint32_t* start = r->arrays + m[UKL_T2_O_ZSTART];
        uint32_t at = ukl_ph_sum(r, UKL_T2_TZ3, c)[0];
        for (uint32_t j = a; j < b; j++) {
            start[j] = at;
            at += stored[j];
        }
    }
}

UKL_INLINE void ukl_ph_item_sg(UKL_PRIVATE UklPh* r, uint32_t ph, uint32_t c, uint32_t lane __UKL_PARAMS)
{
    if (ph == UKL_T2_PH_ZP1)
        ukl_t2z_zp1(r, c, lane __UKL_ARGS);
    else
        ukl_t2z_zp4(r, c, lane __UKL_ARGS);
}

UKL_INLINE uint32_t ukl_ph_items(UKL_PRIVATE UklPh* r, uint32_t ph)
{
    if (r->refused)
        return 0u;
    if (ph == UKL_T2_PH_LAY)
        return 1u;
    if (ukl_atomic_poll(&r->m[UKL_T2_NEED]) != 0u)
        return 0u;
    if (ph >= UKL_T2_PH_P2 && ph < UKL_T2_PH_P2 + 8u)
        return (ph - UKL_T2_PH_P2) % 4u == 1u || (ph - UKL_T2_PH_P2) % 4u == 2u ? UKL_T2_KEYS : r->m[UKL_T2_ICHUNKS];
    if (ph == UKL_T2_PH_P3 || ph == UKL_T2_PH_P3W)
        return r->m[UKL_T2_ICHUNKS];
    if (ph >= UKL_T2_PH_P5 && ukl_atomic_poll(&r->m[UKL_T2_REFUSED_AT]) != UKL_T2_NONE)
        return 0u;
    if (ph >= UKL_T2_PH_P7 && ph < UKL_T2_PH_P7 + 8u)
        return (ph - UKL_T2_PH_P7) % 4u == 1u || (ph - UKL_T2_PH_P7) % 4u == 2u ? UKL_T2_KEYS : r->m[UKL_T2_ECHUNKS];
    if (ph >= UKL_T2_PH_MAKE && ukl_atomic_poll(&r->m[UKL_PH_FAIL]) != 0u)
        return 0u;
    if (ph >= UKL_T2_PH_ZP1)
        return ukl_t2z_items(r, ph);
    if (ph == UKL_T2_PH_MAKE || ph == UKL_T2_PH_END)
        return r->m[UKL_T2_NB];
    if (ph == UKL_T2_PH_P6C || ph == UKL_T2_PH_P6W)
        return r->m[UKL_T2_CHUNKS];
    if (ph == UKL_T2_PH_FIN)
        return ukl_atomic_poll(&r->m[UKL_T2_ENDED]) == 1u ? r->m[UKL_T2_NB] : 0u;
    if (ph == UKL_T2_PH_TAIL)
        return ukl_atomic_poll(&r->m[UKL_T2_ENDED]) == 1u ? 1u : 0u;
    if (ph == UKL_T2_PH_PIECES)
        return ukl_atomic_poll(&r->m[UKL_T2_TAILED]) == 1u ? r->m[UKL_T2_NB] + r->m[UKL_T2_NLINES] : 0u;
    if (ph >= UKL_T2_PH_P8A)
        return r->m[UKL_T2_ECHUNKS];
    return r->m[UKL_T2_CHUNKS];
}

UKL_INLINE void ukl_ph_item(UKL_PRIVATE UklPh* r, uint32_t ph, uint32_t c)
{
    UklPhPool m = r->m;
    if (ph == UKL_T2_PH_LAY) {
        ukl_t2_lay(r);
        return;
    }
    uint32_t n = r->n, size = m[UKL_T2_SIZE], a = c * size, b = a + size < n + 1u ? a + size : n + 1u;
    if (ph == UKL_T2_PH_P1) {
        ukl_t2_p1_run(r->src, r->starts, n, a, b, ukl_ph_leaf(r, UKL_T2_TP1, c));
        ukl_ph_up(r, UKL_T2_TP1, c);
    } else if (ph == UKL_T2_PH_P1W) {
        UklPhPool s = ukl_ph_sum(r, UKL_T2_TP1, c);
        ukl_t2_p1_write(r->src, r->starts, n, a, b, s, r->arrays + m[UKL_T2_O_DEPTH], r->arrays + m[UKL_T2_O_ITEMS], r->arrays + m[UKL_T2_O_HEAD_AT]);
        if (c == 0u) {
            uint32_t ni = ukl_ph_total(r, UKL_T2_TP1)[2], ic = (ni + size - 1u) / size;
            m[UKL_T2_ITEMS] = ni;
            m[UKL_T2_ICHUNKS] = ic;
            m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TP2 + 3u] = ic;
            m[UKL_PH_TREE + UKL_PH_TW * (UKL_T2_TP2 + 2u) + 3u] = ic;
            m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TP3 + 3u] = ic;
        }
    } else if (ph >= UKL_T2_PH_P4 && ph <= UKL_T2_PH_P5W) {
        UKL_GLOBAL const uint32_t* depth = r->arrays + m[UKL_T2_O_DEPTH];
        UKL_GLOBAL const uint32_t* container = r->arrays + m[UKL_T2_O_CONTAINER];
        UKL_GLOBAL uint8_t* ret = r->rets + c * ukl_t2_depth();
        UKL_GLOBAL uint32_t* place = r->arrays + m[UKL_T2_O_PLACE];
        uint32_t low = ukl_ph_leaf(r, UKL_T2_TP1, c)[1], places = ukl_t2_places(), s[UKL_T2_W_SHAPE];
        if (ph == UKL_T2_PH_P4) {
            UklPhPool map = ukl_ph_leaf(r, UKL_T2_TP4, c);
            for (uint32_t p = 0u; p < places; p++)
                map[p] = ukl_t2_walk(r->src, r->starts, n, r->len, r->toks, depth, container, low, a, b, p, ret, s, false, r->arrays, r->arrays, map);
            ukl_ph_up(r, UKL_T2_TP4, c);
        } else if (ph == UKL_T2_PH_P4W) {
            uint32_t pl = ukl_ph_sum(r, UKL_T2_TP4, c)[0];
            place[c] = pl;
            if (pl < places && ukl_ph_leaf(r, UKL_T2_TP4, c)[pl] == ukl_t2_walk_refused())
                ukl_atomic_min(&m[UKL_T2_REFUSED_AT], c);
        } else if (ph == UKL_T2_PH_P5) {
            UklPhPool leaf = ukl_ph_leaf(r, UKL_T2_TP5, c);
            if (place[c] < places) {
                ukl_t2_walk(r->src, r->starts, n, r->len, r->toks, depth, container, low, a, b, place[c], ret, s, false, r->arrays, r->arrays, leaf);
                for (uint32_t k = 0u; k < UKL_T2_W_SHAPE; k++)
                    leaf[k] = s[k];
            } else {
                ukl_ph_unit(UKL_T2_OP_SHAPE, leaf);
            }
            ukl_ph_up(r, UKL_T2_TP5, c);
        } else {
            UklPhPool at = ukl_ph_sum(r, UKL_T2_TP5, c);
            UKL_GLOBAL uint32_t* shape_at = r->arrays + m[UKL_T2_O_SHAPE_AT] + UKL_T2_W_SHAPE * c;
            for (uint32_t k = 0u; k < UKL_T2_W_SHAPE; k++)
                shape_at[k] = at[k];
            if (place[c] < places)
                ukl_t2_walk(r->src, r->starts, n, r->len, r->toks, depth, container, low, a, b, place[c], ret, s, true, r->arrays + m[UKL_T2_O_EVENTS],
                            r->arrays + m[UKL_T2_O_EITEMS], at);
            if (c == 0u) {
                UklPhPool total = ukl_ph_total(r, UKL_T2_TP5);
                m[UKL_T2_NODES] = total[0];
                m[UKL_T2_EVENTS] = total[1];
                m[UKL_T2_EITEMS] = total[2];
                uint32_t ec = (total[2] + size - 1u) / size;
                m[UKL_T2_ECHUNKS] = ec;
                m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TP7 + 3u] = ec;
                m[UKL_PH_TREE + UKL_PH_TW * (UKL_T2_TP7 + 2u) + 3u] = ec;
                m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TP8A + 3u] = ec;
                m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TP8B + 3u] = ec;
                // The tape's blocks.
                uint32_t nb = ukl_t2f_nodes_blocks(total[0]);
                m[UKL_T2_NB] = nb;
                m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TB + 3u] = nb;
                // 16z's pieces, four a block.
                uint32_t zc = (4u * nb + size - 1u) / size;
                m[UKL_T2_NP] = 4u * nb;
                m[UKL_T2_ZCHUNKS] = zc;
                m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TZ2 + 3u] = zc;
                m[UKL_PH_TREE + UKL_PH_TW * (UKL_T2_TZ2 + 2u) + 3u] = zc;
                m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TZH + 3u] = zc;
                m[UKL_PH_TREE + UKL_PH_TW * UKL_T2_TZ3 + 3u] = zc;
                if (nb > m[UKL_T2_MOST])
                    ukl_t2f_fail(r, 8u);
            }
        }
    } else if (ph == UKL_T2_PH_P3 || ph == UKL_T2_PH_P3W) {
        // Over the items in level order (the sort's last digit's, in `items`).
        uint32_t ni = m[UKL_T2_ITEMS], ia = c * size, ib = ia + size < ni ? ia + size : ni;
        UKL_GLOBAL uint32_t* items = r->arrays + m[UKL_T2_O_ITEMS];
        if (ph == UKL_T2_PH_P3) {
            ukl_t2_p3_run(items, ia, ib, ukl_ph_leaf(r, UKL_T2_TP3, c));
            ukl_ph_up(r, UKL_T2_TP3, c);
        } else {
            ukl_t2_p3_write(items, ia, ib, ukl_ph_sum(r, UKL_T2_TP3, c), r->arrays + m[UKL_T2_O_CONTAINER], r->arrays + m[UKL_T2_O_HEAD_AT]);
        }
    } else if (ph >= UKL_T2_PH_ZP1) {
        ukl_t2z_item(r, ph, c);
    } else if (ph >= UKL_T2_PH_MAKE) {
        ukl_t2_finish_item(r, ph, c);
    } else if (ph >= UKL_T2_PH_P8A) {
        // Over the events' items in frame order (P7's last digit's, in `eitems`).
        uint32_t ni = m[UKL_T2_EITEMS], ia = c * size, ib = ia + size < ni ? ia + size : ni;
        UKL_GLOBAL uint32_t* items = r->arrays + m[UKL_T2_O_EITEMS];
        UKL_GLOBAL uint32_t* events = r->arrays + m[UKL_T2_O_EVENTS];
        if (ph == UKL_T2_PH_P8A) {
            ukl_t2_p3_run(items, ia, ib, ukl_ph_leaf(r, UKL_T2_TP8A, c));
            ukl_ph_up(r, UKL_T2_TP8A, c);
        } else if (ph == UKL_T2_PH_P8AW) {
            ukl_t2_p8a_write(items, ia, ib, ukl_ph_sum(r, UKL_T2_TP8A, c), r->arrays + m[UKL_T2_O_FRAME_OF]);
        } else if (ph == UKL_T2_PH_P8B) {
            ukl_t2_p8b_run(items, events, ia, ib, ukl_ph_leaf(r, UKL_T2_TP8B, c));
            ukl_ph_up(r, UKL_T2_TP8B, c);
        } else {
            ukl_t2_p8b_write(items, events, ia, ib, ukl_ph_sum(r, UKL_T2_TP8B, c), r->arrays + m[UKL_T2_O_KIDS]);
        }
    } else if (ph >= UKL_T2_PH_P7) {
        uint32_t k = ph - UKL_T2_PH_P7;
        ukl_t2_sort(r, UKL_T2_TP7, k / 4u, k % 4u, c, m[UKL_T2_EITEMS], m[UKL_T2_O_EITEMS], m[UKL_T2_O_EITEMS2], m[UKL_T2_O_EKEYS]);
    } else {
        uint32_t k = ph - UKL_T2_PH_P2;
        ukl_t2_sort(r, UKL_T2_TP2, k / 4u, k % 4u, c, m[UKL_T2_ITEMS], m[UKL_T2_O_ITEMS], m[UKL_T2_O_ITEMS2], m[UKL_T2_O_KEYS]);
    }
}
