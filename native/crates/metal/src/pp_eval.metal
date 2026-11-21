// The preprocessor's evaluator over tapes (decision 0436): plain MSL passes over a unit's files,
// each a source's bytes and its preprocessor tape (format 16, decision 0468 §1), both where the
// slab residency placed them: `data` is the first document's slab, the others reached from it by
// the slabs' addresses (0353 step 4, 0471), and `files` their views (`DocView`, the residency's
// layout). The tape is read through the reads
// written once in CLIF (decision 0310; `<ukl/reads/tape.h>`, their kernels' copy, 0518), each node where it lies: nothing is
// decoded into another array. Every size a pass needs is made on the device (counts, a prefix
// sum, and the next dispatch's arguments).
//
// Preprocessed before Metal compiles it (`rerail_ukl::metal_source`): the view's definition and
// the reads come first (`pp_eval::source`), the view read where the batch holds it.

// A file of the unit: its document's view. Its parts are offsets from its first byte, `at` and
// `at_high` (an i64) from `data`.
typedef DocView PpFile;

// A file's first byte from `data`: `at` and `at_high`, an i64.
inline long pf_at(PpFile f) { return long((ulong(f.at_high) << 32) | ulong(f.at)); }

// The tape's kinds the passes read (`rerail_c_pp::tape`).
constant uint K_IDENTIFIER = 28u;

// --- A format 16 tape, read through the CLIF reads ---------------------------------------------

// A document: its first byte in the slab, and its view.
struct Tape {
    device const uchar* d;
    device const DocView* v;
    device const uchar* codes;
};

inline Tape tape_at(device const uchar* data, device const PpFile* files, uint f) {
    Tape t;
    t.v = files + f;
    t.d = data + pf_at(*t.v);
    // The language's table: `codes` from the document's first byte, modulo 2^32 (`slabs::codes_offset`).
    t.codes = t.d + int(t.v->codes);
    return t;
}

// A file's source: its first byte's offset from `data`.
inline long pf_src(PpFile f) { return pf_at(f) + long(f.source); }

// Node `i`'s key: its code's kind and field (`kind | field << 10`; a preprocessor tape's codes
// have no flags), from its number in the language's table (`DeviceCode`s, 24 bytes each).
inline uint node_key(Tape t, uint i) {
    device const ushort* c = (device const ushort*)(t.codes + 24u * clif_tape_number(t.d, (device const uint*)t.v, i));
    return uint(c[0]) | uint(c[1]) << 10;
}
inline uint node_kind(Tape t, uint i) { return node_key(t, i) & 0x3ffu; }
inline uint node_size(Tape t, uint i) { return clif_tape_size(t.d, (device const uint*)t.v, i); }
inline uint node_start(Tape t, uint i) { return clif_tape_start(t.d, (device const uint*)t.v, i); }
inline uint node_end(Tape t, uint i) { return clif_tape_end(t.d, (device const uint*)t.v, i); }
// The line (from 0) of byte `x` of the file: by its tape's newline index.
inline uint tape_line_of(Tape t, uint x) { return clif_tape_line(t.d, (device const uint*)t.v, x); }

// --- The unit's nodes: a count a file, a prefix sum, a node's file --------------------------------

// Each file's node count (its view's).
kernel void pp_counts(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                      device uint* counts [[buffer(2)]], device atomic_uint* bad [[buffer(3)]],
                      constant uint& n_files [[buffer(4)]], uint f [[thread_position_in_grid]]) {
    if (f >= n_files) return;
    counts[f] = files[f].nodes;
}

// The file of the unit's node `g`: the last whose first node is at or before it.
inline uint file_of(device const uint* bases, uint n_files, uint g) {
    uint lo = 0u, hi = n_files;
    while (hi - lo > 1u) {
        uint mid = (lo + hi) / 2u;
        if (bases[mid] <= g) lo = mid; else hi = mid;
    }
    return lo;
}

// --- Names -------------------------------------------------------------------------------------

// The 8 bytes of `data` at byte `a`, as a little-endian word.
inline ulong word_at(device const ulong* d, uint a) {
    uint q = a >> 3, r = (a & 7u) * 8u;
    ulong lo = d[q];
    return r == 0u ? lo : (lo >> r) | (d[q + 1u] << (64u - r));
}

// Where a line splice at byte `i` (a `\\`, blanks, then a newline) ends, before `end`; `i`
// where there is none (`rerail_c_lean::pp`'s `splice_at`).
inline uint splice_end(device const uchar* b, uint i, uint end) {
    uint k = i + 1u;
    for (; k < end && (b[k] == uchar(' ') || b[k] == uchar('\t') || b[k] == 0x0bu || b[k] == 0x0cu); k++) {}
    if (k < end && b[k] == uchar('\n')) return k + 1u;
    if (k + 1u < end && b[k] == uchar('\r') && b[k + 1u] == uchar('\n')) return k + 2u;
    return i;
}

// Whether a word holds the byte `c` (a zero byte of `w ^ c…c`).
inline bool word_has(ulong w, uchar c) {
    ulong x = w ^ (0x0101010101010101ul * ulong(c));
    return ((x - 0x0101010101010101ul) & ~x & 0x8080808080808080ul) != 0ul;
}

inline ulong key_mix(ulong h, ulong w) {
    h = (h ^ w) * 0xff51afd7ed558ccdul;
    return h ^ (h >> 32);
}

// A name's key: its bytes a word at a time, line splices left out (its phase-2 spelling,
// `rerail_metal::pp_eval::name_key`). A name with no `\\` is read a word at a time; one with
// one is read again a byte at a time, which only a splice inside a name needs.
inline ulong name_key(device const ulong* d, uint a, uint len) {
    bool slash = false;
    for (uint k = 0u; k < len; k += 8u) {
        ulong w = word_at(d, a + k);
        uint left = len - k;
        if (left < 8u) w &= (1ul << (left * 8u)) - 1ul;
        slash |= word_has(w, uchar('\\'));
    }
    ulong h;
    if (!slash) {
        h = 0x9e3779b97f4a7c15ul ^ ulong(len);
        for (uint k = 0u; k < len; k += 8u) {
            ulong w = word_at(d, a + k);
            uint left = len - k;
            if (left < 8u) w &= (1ul << (left * 8u)) - 1ul;
            h = key_mix(h, w);
        }
    } else {
        device const uchar* b = (device const uchar*)d;
        uint end = a + len, n = 0u;
        for (uint i = a; i < end;) {
            uint e = b[i] == uchar('\\') ? splice_end(b, i, end) : i;
            if (e != i) { i = e; continue; }
            n++;
            i++;
        }
        h = 0x9e3779b97f4a7c15ul ^ ulong(n);
        ulong w = 0ul;
        uint m = 0u;
        for (uint i = a; i < end;) {
            uint e = b[i] == uchar('\\') ? splice_end(b, i, end) : i;
            if (e != i) { i = e; continue; }
            w |= ulong(b[i]) << (8u * m);
            i++;
            if (++m == 8u) {
                h = key_mix(h, w);
                w = 0ul;
                m = 0u;
            }
        }
        if (m > 0u) h = key_mix(h, w);
    }
    h *= 0xc4ceb9fe1a85ec53ul;
    return h ^ (h >> 29);
}

// Each identifier leaf's key, 0 for every other node: one thread a node of the unit.
kernel void pp_names(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                     device const uint* bases [[buffer(2)]], device const uint* total [[buffer(3)]],
                     device ulong* keys [[buffer(4)]], constant uint& n_files [[buffer(5)]],
                     uint g [[thread_position_in_grid]]) {
    if (g >= total[0]) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files, f);
    uint i = g - bases[f];
    ulong key = 0ul;
    if (node_kind(t, i) == K_IDENTIFIER) {
        uint s = node_start(t, i);
        // From the document's first byte, 16-byte aligned in its slab.
        key = name_key((device const ulong*)t.d, files[f].source + s, node_end(t, i) - s);
    }
    keys[g] = key;
}

// For tests: each node of the unit as the reader gives it (key, start, end, size).
kernel void pp_nodes(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                     device const uint* bases [[buffer(2)]], device const uint* total [[buffer(3)]],
                     device uint4* out [[buffer(4)]], constant uint& n_files [[buffer(5)]],
                     uint g [[thread_position_in_grid]]) {
    if (g >= total[0]) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files, f);
    uint i = g - bases[f];
    out[g] = uint4(node_key(t, i), node_start(t, i), node_end(t, i), node_size(t, i));
}

// --- The directives: a file pass, a prefix sum, a file pass ------------------------------------
//
// One thread a file walks its tape's items in order: a text line (`code`) is passed by its
// subtree's size, a directive is one row, and a group is entered (its chain's directives are its
// children; its branches' items after each). The first walk counts each file's directives,
// a scan (`pp_blocks`) gives each file its first row, and the second writes the rows: the spike's P0
// tables (`rerail_c_pp::stream::parallel::tables`), made from the tape in place.

constant uint K_CODE = 1u;
constant uint K_GROUP = 2u;
constant uint K_DIRECTIVES = 3u;
constant uint K_IF = 12u, K_IFNDEF = 14u, K_ELSE = 18u, K_ENDIF = 19u;
constant uint NO_CHAIN = 0xffffffffu;
// The groups a walk holds open: deeper sends the unit to the CPU (bit 30 of `bad`).
constant uint MAX_OPEN = 128u;

// A directive's row: its node in its file's tape, its kind, the text tokens before it in the
// file, the group it is read in and the group after it, and its chain (`NO_CHAIN`: none).
struct PpDir { uint node, kind, text_before, group, after, chain; };

// File `t`'s walk: each directive's row to `out` (where `out` is set) from row `at`; the number
// of rows, the file's groups in `groups`, and in `guarded` whether it has an include guard's
// shape: its first directive an `#ifndef` with no text before it, of one arm, its `#endif` the
// last directive with no text after it, every directive between inside it (the spike's
// `files::guard`; the name is the first row's key, `pp_guards`).
inline uint walk_dirs(Tape t, device PpDir* out, uint at, thread uint& groups, thread bool& guarded, device atomic_uint* bad) {
    uint ends[MAX_OPEN], outer[MAX_OPEN], chain_of[MAX_OPEN];
    uint depth = 0u, cur = 0u, chains = 0u, text = 0u, rows = 0u;
    groups = 1u;
    // The guard's shape: the first row's, the arms of its chain after it, the rows after the
    // first outside a group, and the last row.
    bool first_ok = false, more_arms = false, last_violates = false;
    uint outside = 0u, last_kind = 0u, last_after = 0u, last_text = 0u;
    // Node 0 is the file's root (`pp`).
    for (uint i = 1u; i < t.v->nodes;) {
        while (depth > 0u && i >= ends[depth - 1u]) {
            depth--;
            cur = outer[depth];
        }
        uint k = node_kind(t, i), size = node_size(t, i);
        if (k == K_CODE) {
            // A line of text: its leaves are its tokens.
            text += size - 1u;
            i += size;
            continue;
        }
        if (k == K_GROUP) {
            if (depth == MAX_OPEN) {
                atomic_fetch_or_explicit(bad, 1u << 30, memory_order_relaxed);
                return rows;
            }
            ends[depth] = i + size;
            outer[depth] = cur;
            chain_of[depth] = chains++;
            depth++;
            i += 1u;
            continue;
        }
        if (k >= K_DIRECTIVES && k < K_IDENTIFIER) {
            PpDir d;
            d.node = i;
            d.kind = k;
            d.text_before = text;
            d.chain = NO_CHAIN;
            // A group's child is its chain's: an arm opens (`#if…`, `#elif…`, `#else`), read in
            // the group the chain opened in; or the `#endif` closes it.
            if (depth > 0u && k >= K_IF && k <= K_ELSE) {
                d.group = outer[depth - 1u];
                d.after = groups++;
                d.chain = chain_of[depth - 1u];
                cur = d.after;
            } else if (depth > 0u && k == K_ENDIF) {
                cur = outer[depth - 1u];
                d.group = cur;
                d.after = cur;
            } else {
                d.group = cur;
                d.after = cur;
            }
            if (out) out[at + rows] = d;
            if (rows == 0u) {
                first_ok = k == K_IFNDEF && text == 0u;
            } else {
                more_arms |= d.chain == 0u && k != K_ENDIF;
                last_violates = d.group == 0u || d.after == 0u;
                outside += last_violates ? 1u : 0u;
            }
            last_kind = k;
            last_after = d.after;
            last_text = text;
            rows++;
        }
        i += size;
    }
    guarded = rows >= 2u && first_ok && !more_arms && last_kind == K_ENDIF && last_after == 0u && last_text == text
              && outside == (last_violates ? 1u : 0u);
    return rows;
}

// Each file's directives.
kernel void pp_dir_counts(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                          device uint* counts [[buffer(2)]], device atomic_uint* bad [[buffer(3)]],
                          constant uint& n_files [[buffer(4)]], uint f [[thread_position_in_grid]]) {
    if (f >= n_files) return;
    uint groups;
    bool guarded;
    counts[f] = walk_dirs(tape_at(data, files, f), (device PpDir*)nullptr, 0u, groups, guarded, bad);
}

// Each file's rows from its first (the scan's), its groups, and whether it has a guard's shape.
kernel void pp_dirs(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                    device const uint* dir_bases [[buffer(2)]], device PpDir* rows [[buffer(3)]],
                    device uint* groups [[buffer(4)]], device uint* guarded [[buffer(5)]],
                    device atomic_uint* bad [[buffer(6)]],
                    constant uint& n_files [[buffer(7)]], uint f [[thread_position_in_grid]]) {
    if (f >= n_files) return;
    uint g;
    bool shaped;
    walk_dirs(tape_at(data, files, f), rows, dir_bases[f], g, shaped, bad);
    groups[f] = g;
    guarded[f] = shaped ? 1u : 0u;
}

// --- The directives' names: a line pass ---------------------------------------------------------

constant uint K_DEFINE = 6u, K_UNDEF = 7u, K_IFDEF = 13u, K_ELIFDEF = 16u, K_ELIFNDEF = 17u;

// Each directive's name key (0: none): a `#define`'s or `#undef`'s name, an `#ifdef`'s,
// `#ifndef`'s, or (in a chain) `#elifdef`'s or `#elifndef`'s operand, where the first token after
// its word is an identifier: the key the names pass gave that leaf. One thread a row.
kernel void pp_dir_keys(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                        device const uint* bases [[buffer(2)]], device const ulong* keys [[buffer(3)]],
                        device const uint* dir_bases [[buffer(4)]], device const uint* dir_total [[buffer(5)]],
                        device const PpDir* rows [[buffer(6)]], device ulong* dir_keys [[buffer(7)]],
                        constant uint& n_files [[buffer(8)]], uint r [[thread_position_in_grid]]) {
    if (r >= dir_total[0]) return;
    uint f = file_of(dir_bases, n_files, r);
    Tape t = tape_at(data, files, f);
    PpDir d = rows[r];
    bool named = d.kind == K_DEFINE || d.kind == K_UNDEF || d.kind == K_IFDEF || d.kind == K_IFNDEF
                 || ((d.kind == K_ELIFDEF || d.kind == K_ELIFNDEF) && d.chain != NO_CHAIN);
    ulong key = 0ul;
    // Its `#`, its word, then the first token after them: a leaf, or an operand node's first.
    uint end = d.node + node_size(t, d.node);
    uint j = d.node + 3u;
    if (named && j < end) {
        while (node_size(t, j) > 1u) j++;
        if (node_kind(t, j) == K_IDENTIFIER) key = keys[bases[f] + j];
    }
    dir_keys[r] = key;
}

// Each file's include guard's key (0: none): its first directive's name, where the file has a
// guard's shape. One thread a file.
kernel void pp_guards(device const uint* guarded [[buffer(0)]], device const uint* dir_bases [[buffer(1)]],
                      device const ulong* dir_keys [[buffer(2)]], device ulong* guards [[buffer(3)]],
                      constant uint& n_files [[buffer(4)]], uint f [[thread_position_in_grid]]) {
    if (f >= n_files) return;
    guards[f] = guarded[f] != 0u ? dir_keys[dir_bases[f]] : 0ul;
}
