// Macro expansion on the device (decision 0448): the setup (the definitions, from the tapes in
// place) and, later, the execution. After `pp_eval.metal` (the tape reader, the names, the
// directives) and `pp_blocks.metal` (scans, sorts, searches).

// --- Setup: the definitions --------------------------------------------------------------------
//
// One definition a directive row (zeros where it is no `#define`), read from its node in place:
// its `#`, its word, its name, then a `params` node over `(`…`)` where it is function-like, then
// a `body` node over its replacement list. Its parameters and body tokens are read one thread a
// leaf (`pd_params`, `pd_items`), placed by scans of the rows' counts. Their stores have the
// rung's room (decision 0448 §2): a leaf past it sets `room`, and the unit runs again larger.

constant uint K_PUNCT = 31u, K_HASHHASH = 34u, K_PARAMS = 36u, K_OPERANDS = 39u;
// The tape's fields (`rerail_c_pp::tape::FIELDS`, from 1): `name`, `body`.
constant uint F_NAME = 1u, F_BODY = 5u;

// `PpDef::flags`.
constant uint D_DEFINE = 1u, D_FNLIKE = 2u, D_VARIADIC = 4u, D_BAD = 8u;
// An item's parameter where it is none.
constant uint NO_PARAM = 0xffffu;
// An item's flags (above its parameter's 16 bits): a paste operator, white space before it.
constant uint I_PASTE = 1u << 16, I_SPACE = 1u << 17;

// A definition: the unit's nodes of its name, its `params` node (0: none) and its body's first
// leaf; the leaves of each; its flags; and its first parameter slot and first item (scans).
struct PpDef { uint name, params, n_params, body, n_body, flags, pfirst, ifirst; };

inline uint node_field(Tape t, uint i) { return (node_key(t, i) >> 10) & 63u; }

// Whether the bytes between two tokens are white space or a comment to the lexer (`SPACE`):
// anything but line splices is.
inline bool gap_is_space(device const uchar* data, PpFile f, uint end, uint start) {
    device const uchar* b = data + f.src;
    for (uint i = end; i < start;) {
        if (b[i] != uchar('\\')) return true;
        uint e = splice_end(b, i, start);
        if (e == i) return true;
        i = e;
    }
    return false;
}

// A punctuator leaf spelled `c` (one byte).
inline bool punct_is(Tape t, device const uchar* data, PpFile f, uint i, uchar c) {
    return node_kind(t, i) == K_PUNCT && node_end(t, i) - node_start(t, i) == 1u && data[f.src + node_start(t, i)] == c;
}
inline bool is_dots(Tape t, device const uchar* data, PpFile f, uint i) {
    uint s = node_start(t, i);
    return node_kind(t, i) == K_PUNCT && node_end(t, i) - s == 3u && data[f.src + s] == uchar('.');
}

// S1: each row's definition, and the leaves of its parameters and its body (to be scanned).
kernel void pd_defs(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                    device const uint* bases [[buffer(2)]], device const uint* dir_bases [[buffer(3)]],
                    device const uint* dir_total [[buffer(4)]], device const PpDir* rows [[buffer(5)]],
                    device PpDef* defs [[buffer(6)]], device uint* pcount [[buffer(7)]],
                    device uint* bcount [[buffer(8)]], constant uint& n_files [[buffer(9)]],
                    uint r [[thread_position_in_grid]]) {
    if (r >= dir_total[0]) return;
    PpDef d = {0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u};
    PpDir row = rows[r];
    if (row.kind == K_DEFINE) {
        uint f = file_of(dir_bases, n_files, r);
        Tape t = tape_at(data, files[f].tape);
        uint g = bases[f];
        uint end = row.node + node_size(t, row.node);
        uint j = row.node + 3u;
        d.flags = D_DEFINE;
        if (j < end && node_kind(t, j) == K_IDENTIFIER && node_field(t, j) == F_NAME) {
            d.name = g + j;
            j++;
            if (j < end && node_kind(t, j) == K_PARAMS) {
                d.flags |= D_FNLIKE;
                d.params = g + j;
                d.n_params = node_size(t, j) - 1u;
                j += node_size(t, j);
            }
            if (j < end && node_kind(t, j) == K_OPERANDS && node_field(t, j) == F_BODY) {
                d.body = g + j + 1u;
                d.n_body = node_size(t, j) - 1u;
            }
        } else {
            d.flags |= D_BAD;
        }
    }
    defs[r] = d;
    pcount[r] = d.n_params;
    bcount[r] = d.n_body;
}

// S2: a parameter list's leaf `k` (one thread a leaf over the rows' scanned counts): the list is
// `(` then names or `...` between commas, a name's `...` (GNU's) last, then `)`, as
// `parse_define` reads it; anything else makes the definition bad. Each parameter's key goes to
// its slot (`pfirst` + its leaf), `__VA_ARGS__`'s for a `...` of its own.
kernel void pd_params(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                      device const uint* bases [[buffer(2)]], device const ulong* keys [[buffer(3)]],
                      device const uint* dir_bases [[buffer(4)]], device const uint* dir_total [[buffer(5)]],
                      device PpDef* defs [[buffer(6)]], device const uint* pfirst [[buffer(7)]],
                      device const uint* ptotal [[buffer(8)]], device ulong* pkeys [[buffer(9)]],
                      device atomic_uint* room [[buffer(10)]], constant uint& n_files [[buffer(11)]],
                      constant uint& va_lo [[buffer(12)]], constant uint& va_hi [[buffer(13)]],
                      constant uint& cap [[buffer(14)]], uint s [[thread_position_in_grid]]) {
    if (s >= ptotal[0]) return;
    if (s >= cap) {
        atomic_store_explicit(room, 1u, memory_order_relaxed);
        return;
    }
    uint r = pb_item_of(pfirst, dir_total[0], s);
    uint k = s - pfirst[r];
    uint f = file_of(dir_bases, n_files, r);
    Tape t = tape_at(data, files[f].tape);
    PpFile pf = files[f];
    PpDef d = defs[r];
    if (k == 0u) defs[r].pfirst = pfirst[r];
    uint i = d.params - bases[f] + 1u + k;
    uint last = d.n_params - 1u;
    bool ok;
    bool open = punct_is(t, data, pf, i, '('), close = punct_is(t, data, pf, i, ')'), comma = punct_is(t, data, pf, i, ',');
    bool dots = is_dots(t, data, pf, i), name = node_kind(t, i) == K_IDENTIFIER;
    ulong key = 0ul;
    if (k == 0u) {
        ok = open;
    } else {
        uint p = i - 1u;
        bool p_open = k == 1u, p_comma = punct_is(t, data, pf, p, ','), p_dots = is_dots(t, data, pf, p);
        bool p_name = node_kind(t, p) == K_IDENTIFIER;
        if (p_open || p_comma) {
            ok = name || dots || (p_open && close);
            if (name) key = keys[bases[f] + i];
            if (dots) key = ulong(va_lo) | (ulong(va_hi) << 32);
            if (dots) atomic_fetch_or_explicit((device atomic_uint*)&defs[r].flags, D_VARIADIC, memory_order_relaxed);
        } else if (p_name) {
            ok = comma || close || dots;
            if (dots) atomic_fetch_or_explicit((device atomic_uint*)&defs[r].flags, D_VARIADIC, memory_order_relaxed);
        } else if (p_dots) {
            ok = close;
        } else {
            ok = false;
        }
    }
    // `)` ends the list, and only there.
    if ((k == last) != close) ok = false;
    pkeys[s] = key;
    if (!ok) atomic_fetch_or_explicit((device atomic_uint*)&defs[r].flags, D_BAD, memory_order_relaxed);
}

// The parameter index of the key `key` in definition `d` (`NO_PARAM`: none): its slots are its
// list's leaves, a parameter at each odd one.
inline uint param_of(PpDef d, device const ulong* pkeys, ulong key) {
    if ((d.flags & D_FNLIKE) == 0u || key == 0ul) return NO_PARAM;
    for (uint k = 1u; k < d.n_params; k += 2u) {
        if (pkeys[d.pfirst + k] == key) return (k - 1u) / 2u;
    }
    return NO_PARAM;
}

// S4: a body's leaf `k` (one thread a leaf over the rows' scanned counts): its parameter (by
// its key among its definition's), `##` as the paste operator where it is neither first nor
// last, and white space before it (none before the first).
kernel void pd_items(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                     device const uint* bases [[buffer(2)]], device const ulong* keys [[buffer(3)]],
                     device const uint* dir_bases [[buffer(4)]], device const uint* dir_total [[buffer(5)]],
                     device PpDef* defs [[buffer(6)]], device const uint* ifirst [[buffer(7)]],
                     device const uint* itotal [[buffer(8)]], device const ulong* pkeys [[buffer(9)]],
                     device uint* items [[buffer(10)]], device atomic_uint* room [[buffer(11)]],
                     constant uint& n_files [[buffer(12)]], constant uint& cap [[buffer(13)]],
                     uint s [[thread_position_in_grid]]) {
    if (s >= itotal[0]) return;
    if (s >= cap) {
        atomic_store_explicit(room, 1u, memory_order_relaxed);
        return;
    }
    uint r = pb_item_of(ifirst, dir_total[0], s);
    uint k = s - ifirst[r];
    uint f = file_of(dir_bases, n_files, r);
    Tape t = tape_at(data, files[f].tape);
    PpDef d = defs[r];
    if (k == 0u) defs[r].ifirst = ifirst[r];
    uint i = d.body - bases[f] + k;
    uint item = param_of(d, pkeys, keys[d.body + k]);
    if (node_kind(t, i) == K_HASHHASH && k > 0u && k + 1u < d.n_body) item |= I_PASTE;
    if (k > 0u && gap_is_space(data, files[f], node_end(t, i - 1u), node_start(t, i))) item |= I_SPACE;
    items[s] = item;
}
