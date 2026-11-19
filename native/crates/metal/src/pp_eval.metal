// The preprocessor's evaluator over tapes (decision 0436): plain MSL passes over a unit's files,
// each a source's bytes and its preprocessor tape (format 16, decision 0460 §1), both where the
// slab residency placed them: `data` is the slab the unit's documents were gathered into, and
// `files` their views (`DocView`, the residency's layout). The tape is read through the reads
// written once in CLIF (`<ukl/reads/tape.h>`, decision 0310), each node where it lies: nothing is
// decoded into another array. Every size a pass needs is made on the device (counts, a prefix
// sum, and the next dispatch's arguments).
//
// Preprocessed before Metal compiles it (`rerail_ukl::metal_source`): the view's definition and
// the reads come first (`pp_eval::source`), the view read where the batch holds it.

// A file of the unit: its document's view. Its parts are offsets from its `at` in `data`.
typedef DocView PpFile;

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
    t.d = data + t.v->at;
    // The language's table: `at + codes`, modulo 2^32 (`slabs::codes_offset`).
    t.codes = data + (t.v->at + t.v->codes);
    return t;
}

// A file's source: its first byte's offset in `data`, and its length.
inline uint pf_src(PpFile f) { return f.at + f.source; }

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
inline void pp_names(device const PpFile* files, device const uchar* data, device const uint* bases, device const uint* total, device ulong* keys, uint n_files, uint g) {
    if (g >= total[0]) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files, f);
    uint i = g - bases[f];
    ulong key = 0ul;
    if (node_kind(t, i) == K_IDENTIFIER) {
        uint s = node_start(t, i);
        key = name_key((device const ulong*)data, pf_src(files[f]) + s, node_end(t, i) - s);
    }
    keys[g] = key;
}

// For tests: each node of the unit as the reader gives it (key, start, end, size).
inline void pp_nodes(device const PpFile* files, device const uchar* data, device const uint* bases, device const uint* total, device uint4* out, uint n_files, uint g) {
    if (g >= total[0]) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files, f);
    uint i = g - bases[f];
    out[g] = uint4(node_key(t, i), node_start(t, i), node_end(t, i), node_size(t, i));
}

// --- The directives: passes a node each ----------------------------------------------------------
//
// A node is a directive row where its kind is a directive's (they are items alone: a directive's
// own nodes are tokens and kinds past `K_IDENTIFIER`). A group (`K_GROUP`) holds its chain's arms
// (`#if…`, `#elif…`, `#else`), its `#endif`, and each arm's items after it. Every row's fields come
// from scans over the batch's nodes and searches, never from a walk of a file (decision 0460):
// - a node's depth: the groups around it, a scan of +1 after each group's node and -1 at its end;
// - its place among rows, groups and arms: scans of their flags (a file's from its first node's);
// - the arm an item is read under: the latest arm at the item's own depth before it, a search in
//   the arms' keys (depth, place) sorted; a row's group node, the latest group at one depth less;
// - the text before it: the text tokens' scan (`text_before`, from its file's first node's).
// The spike's P0 tables (`rerail_c_pp::stream::parallel::tables`), made from the tapes in place.

constant uint K_CODE = 1u;
constant uint K_GROUP = 2u;
constant uint K_DIRECTIVES = 3u;
constant uint K_IF = 12u, K_IFNDEF = 14u, K_ELSE = 18u, K_ENDIF = 19u;
constant uint NO_CHAIN = 0xffffffffu;

// A directive's row: its node in its file's tape, its kind, the text tokens before it in the
// file, the group it is read in and the group after it, and its chain (`NO_CHAIN`: none).
struct PpDir { uint node, kind, text_before, group, after, chain; };

constant uint PP_NONE = 0xffffffffu;

// A node's depth: the groups around it (the events' exclusive scan, and its own event).
inline uint pp_depth(device const uint* depth, device const uint* dev, uint g) { return depth[g] + dev[g]; }

// The node of the last key at or before (`d`, `x`) in `keys`' first `n` (sorted), where its depth
// is `d`; `PP_NONE` where there is none.
inline uint pp_latest(device const ulong* keys, uint n, uint d, uint x) {
    ulong q = (ulong(d) << 32) | ulong(x);
    uint lo = 0u, hi = n;
    for (uint k = 0u; k < 40u && lo < hi; k++) {
        uint mid = (lo + hi) / 2u;
        if (keys[mid] <= q) lo = mid + 1u; else hi = mid;
    }
    if (lo == 0u || uint(keys[lo - 1u] >> 32) != d) return PP_NONE;
    return uint(keys[lo - 1u]);
}

// Each node's flags: a row's, a group's; a group's depth events (+1 after its node, -1 past its
// end: `dev`, words that wrap); and the text map's seed (an interior node's place).
inline void pp_node_flags(device const PpFile* files, device const uchar* data, device const uint* bases, device const uint* total, device uint* dflag, device uint* gflag, device atomic_uint* dev, device uint* owner_in, uint n_files, uint g) {
    uint n = total[0];
    if (g >= n) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files, f);
    uint i = g - bases[f];
    uint k = node_kind(t, i), size = node_size(t, i);
    dflag[g] = k >= K_DIRECTIVES && k < K_IDENTIFIER ? 1u : 0u;
    gflag[g] = k == K_GROUP ? 1u : 0u;
    owner_in[g] = size > 1u ? g : 0u;
    if (k == K_GROUP) {
        if (g + 1u < n) atomic_fetch_add_explicit(&dev[g + 1u], 1u, memory_order_relaxed);
        if (g + size < n) atomic_fetch_sub_explicit(&dev[g + size], 1u, memory_order_relaxed);
    }
}

// Each node's arm flag (an arm's kind inside a group), its text flag (a leaf of a text line), and a
// group's key (its depth, its place) at its rank.
inline void pp_node_depths(device const PpFile* files, device const uchar* data, device const uint* bases, device const uint* total, device const uint* depth, device const uint* dev, device const uint* gflag, device const uint* grank, device const uint* owner, device uint* aflag, device uint* tflag, device ulong* gkeys, uint n_files, uint g) {
    if (g >= total[0]) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files, f);
    uint i = g - bases[f];
    uint k = node_kind(t, i), d = pp_depth(depth, dev, g);
    aflag[g] = k >= K_IF && k <= K_ELSE && d > 0u ? 1u : 0u;
    tflag[g] = node_size(t, i) == 1u && node_kind(t, owner[g] - bases[f]) == K_CODE ? 1u : 0u;
    if (gflag[g] != 0u) gkeys[grank[g]] = (ulong(d) << 32) | ulong(g);
}

// Each arm's key (its depth, its place) at its rank.
inline void pp_arm_keys(device const uint* total, device const uint* aflag, device const uint* arank, device const uint* depth, device const uint* dev, device ulong* akeys, uint g) {
    if (g < total[0] && aflag[g] != 0u) akeys[arank[g]] = (ulong(pp_depth(depth, dev, g)) << 32) | ulong(g);
}

// Each file's first row, and its groups: the file's own and one an arm.
inline void pp_file_dirs(device const uint* bases, device const uint* drank, device const uint* arank, device const uint* n_arms, device uint* dir_bases, device uint* groups, uint n_files, uint f) {
    if (f >= n_files) return;
    dir_bases[f] = drank[bases[f]];
    uint a1 = f + 1u < n_files ? arank[bases[f + 1u]] : n_arms[0];
    groups[f] = 1u + a1 - arank[bases[f]];
}

// The group an item at depth `d` is read in: its arm's (the latest arm at its depth before it,
// numbered from 1 in its file), or 0, the file's own.
inline uint pp_arm_of(device const ulong* akeys, uint n_arms, device const uint* arank, uint a0, uint d, uint x) {
    if (d == 0u) return 0u;
    uint a = pp_latest(akeys, n_arms, d, x);
    return a == PP_NONE ? 0u : arank[a] - a0 + 1u;
}

// Each directive's row at its rank: its node, kind and text before it; an arm's group (its
// chain's), the group it opens and its chain (its group's rank in the file); an `#endif`'s its
// chain's group; any other's the group it is read in.
inline void pp_rows(device const PpFile* files, device const uchar* data, device const uint* bases, device const uint* total, device const uint* dflag, device const uint* drank, device const uint* depth, device const uint* dev, device const uint* trank, device const uint* arank, device const uint* grank, device const ulong* akeys, device const uint* n_arms, device const ulong* gkeys, device const uint* n_groups, device PpDir* rows, uint n_files, uint g) {
    if (g >= total[0] || dflag[g] == 0u) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files, f);
    uint b = bases[f], i = g - b, k = node_kind(t, i), d = pp_depth(depth, dev, g);
    uint a0 = arank[b], na = n_arms[0];
    PpDir r;
    r.node = i;
    r.kind = k;
    r.text_before = trank[g] - trank[b];
    r.chain = NO_CHAIN;
    bool arm = k >= K_IF && k <= K_ELSE && d > 0u;
    if (arm || (k == K_ENDIF && d > 0u)) {
        uint grp = pp_latest(gkeys, n_groups[0], d - 1u, g);
        uint outer = grp == PP_NONE ? 0u : pp_arm_of(akeys, na, arank, a0, d - 1u, grp);
        r.group = outer;
        if (arm) {
            r.after = arank[g] - a0 + 1u;
            r.chain = grp == PP_NONE ? NO_CHAIN : grank[grp] - grank[b];
        } else {
            r.after = outer;
        }
    } else {
        r.group = pp_arm_of(akeys, na, arank, a0, d, g);
        r.after = r.group;
    }
    rows[drank[g]] = r;
}

// A file's rows after its first that break an include guard's shape: outside every group, or
// another arm of the first chain (`outside`, `arms`, a count each by file).
inline void pp_guard_marks(device const uint* dir_bases, device const uint* dir_total, device const PpDir* rows, device atomic_uint* outside, device atomic_uint* arms, uint n_files, uint r) {
    if (r >= dir_total[0]) return;
    uint f = file_of(dir_bases, n_files, r);
    if (r == dir_bases[f]) return;
    PpDir d = rows[r];
    if (d.group == 0u || d.after == 0u) atomic_fetch_add_explicit(&outside[f], 1u, memory_order_relaxed);
    if (d.chain == 0u && d.kind != K_ENDIF) atomic_fetch_add_explicit(&arms[f], 1u, memory_order_relaxed);
}

// Each file's include guard, where it has a guard's shape: its first directive an `#ifndef` with
// no text before it, of one arm, its `#endif` the last directive with no text after it, every
// directive between inside it (the spike's `files::guard`); the guard's name is the first row's
// key, else 0.
inline void pp_guards(device const uint* bases, device const uint* dir_bases, device const uint* dir_total, device const PpDir* rows, device const uint* trank, device const uint* ttotal, device const uint* outside, device const uint* arms, device const ulong* dir_keys, device uint* guarded, device ulong* guards, uint n_files, uint f) {
    if (f >= n_files) return;
    uint d0 = dir_bases[f], d1 = f + 1u < n_files ? dir_bases[f + 1u] : dir_total[0];
    uint text = (f + 1u < n_files ? trank[bases[f + 1u]] : ttotal[0]) - trank[bases[f]];
    bool shaped = d1 >= d0 + 2u && rows[d0].kind == K_IFNDEF && rows[d0].text_before == 0u && arms[f] == 0u
                  && rows[d1 - 1u].kind == K_ENDIF && rows[d1 - 1u].after == 0u && rows[d1 - 1u].text_before == text && outside[f] == 1u;
    guarded[f] = shaped ? 1u : 0u;
    guards[f] = shaped ? dir_keys[d0] : 0ul;
}

// --- The directives' names: a line pass ---------------------------------------------------------

constant uint K_DEFINE = 6u, K_UNDEF = 7u, K_IFDEF = 13u, K_ELIFDEF = 16u, K_ELIFNDEF = 17u;

// Each directive's name key (0: none): a `#define`'s or `#undef`'s name, an `#ifdef`'s,
// `#ifndef`'s, or (in a chain) `#elifdef`'s or `#elifndef`'s operand, where the first token after
// its word is an identifier: the key the names pass gave that leaf. One thread a row.
inline void pp_dir_keys(device const PpFile* files, device const uchar* data, device const uint* bases, device const ulong* keys, device const uint* dir_bases, device const uint* dir_total, device const PpDir* rows, device ulong* dir_keys, uint n_files, uint r) {
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

