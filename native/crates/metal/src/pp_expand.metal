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

// --- The paste relexer ---------------------------------------------------------------------------
//
// `l ## r` is one token where the lexer reads the two spellings, joined, as one token from the
// first byte to the last (`Pp::paste`). This is the lean lexer's `token_at` (`rerail_c_lean::pp`),
// branch for branch, over ASCII; a spelling with a byte past ASCII, a `\` or a raw string's `R"`
// sends the unit to the CPU (`RELEX_REFUSED`), as the device reads no Unicode classes.

constant uint K_NUMBER = 29u, K_LITERAL = 30u, K_DIGRAPH = 32u, K_HASH = 33u;
// The relexer's answer where the spelling is no one token, and where the device refuses it.
constant uint RELEX_NONE = 0u, RELEX_REFUSED = 0xffffu;
// The dialect's options (`rerail_c_lean::pp::Options`), as bits.
constant uint O_CPP = 1u, O_COLONS = 2u, O_DIGIT_SEP = 4u, O_RAW = 8u;

inline bool is_word(uint c) { return (c >= uint('a') && c <= uint('z')) || (c >= uint('A') && c <= uint('Z')) || (c >= uint('0') && c <= uint('9')) || c == uint('_'); }
inline bool is_alpha(uint c) { return (c >= uint('a') && c <= uint('z')) || (c >= uint('A') && c <= uint('Z')); }
inline bool is_digit(uint c) { return c >= uint('0') && c <= uint('9'); }

// The punctuator's length at the spelling's start (`Lexer::punct`), 0 where none starts.
inline uint punct_len(device const uchar* b, uint n, uint o) {
    uint c = b[0], d = n > 1u ? b[1] : 0u, e = n > 2u ? b[2] : 0u;
    bool cpp = (o & O_CPP) != 0u;
    switch (c) {
        case '(': case ')': case '[': case ']': case '{': case '}': case ';': case ',': case '~': case '?': return 1u;
        case ':': return (d == ':' && cpp) ? 2u : 1u;
        case '.': return (d == '.' && e == '.') ? 3u : (d == '*' && cpp) ? 2u : 1u;
        case '=': case '!': case '*': case '/': case '%': case '^': return d == '=' ? 2u : 1u;
        case '+': return (d == '+' || d == '=') ? 2u : 1u;
        case '-': return (d == '-' || d == '=') ? 2u : (d == '>' && e == '*' && cpp) ? 3u : d == '>' ? 2u : 1u;
        case '&': return (d == '&' || d == '=') ? 2u : 1u;
        case '|': return (d == '|' || d == '=') ? 2u : 1u;
        case '<': return (d == '<' && e == '=') ? 3u : d == '<' ? 2u : (d == '=' && e == '>' && cpp) ? 3u : d == '=' ? 2u : 1u;
        case '>': return (d == '>' && e == '=') ? 3u : (d == '>' || d == '=') ? 2u : 1u;
        default: return 0u;
    }
}

// A pp-number's end from 0 (`pp_number_end`), over ASCII.
inline uint number_end(device const uchar* b, uint n, uint o) {
    uint i = 1u;
    for (uint k = 0u; k < n && i < n; k++) {
        uint c = b[i], prev = b[i - 1u];
        if (is_word(c) || c == uint('.') || c == uint('$')) i++;
        else if ((c == uint('+') || c == uint('-')) && (prev == 'e' || prev == 'E' || prev == 'p' || prev == 'P')) i++;
        else if (c == uint('\'') && (o & O_DIGIT_SEP) != 0u && i + 1u < n && is_word(b[i + 1u])) i += 2u;
        else break;
    }
    return i;
}

// A literal's end from its quote at `q` (`quoted`), then its C++ suffix; 0 where it is unclosed.
inline uint quoted_end(device const uchar* b, uint n, uint q, uint o) {
    uint close = b[q];
    uint i = q + 1u, end = 0u;
    for (uint k = 0u; k < n && i < n; k++) {
        if (b[i] == close) { end = i + 1u; break; }
        if (b[i] == uchar('\n')) break;
        i += b[i] == uchar('\\') ? 2u : 1u;
    }
    if (end == 0u) return 0u;
    if ((o & O_CPP) != 0u && end < n && (is_alpha(b[end]) || b[end] == uchar('_'))) {
        uint e = end;
        for (uint k = 0u; k < n && e < n && (is_word(b[e]) || b[e] == uchar('$')); k++) e++;
        end = e;
    }
    return end;
}

// `b[0..n)` as a name: the identifier's kind where it is one name to its end.
inline uint ident_whole(device const uchar* b, uint n) {
    uint e = 1u;
    for (uint k = 0u; k < n && e < n && (is_word(b[e]) || b[e] == uchar('$')); k++) e++;
    return e == n ? K_IDENTIFIER : RELEX_NONE;
}

// A raw string's end from `i`, after its `"` (`Lexer::raw_end`): its delimiter to `(`, then the
// first `)` with the delimiter and `"` after it; then its C++ suffix. 0 where it is unclosed.
inline uint raw_end(device const uchar* b, uint n, uint i, uint o) {
    uint k = i;
    for (; k < n && k - i <= 16u; k++) {
        uint c = b[k];
        if (c == '(' || c == ')' || c == '\\' || c == ' ' || c == '\t' || c == '\n' || c == '"') break;
    }
    if (k >= n || b[k] != uchar('(')) return 0u;
    uint dl = k - i;
    for (uint j = k + 1u; j < n; j++) {
        if (b[j] != uchar(')')) continue;
        uint after = j + 1u;
        bool same = after + dl < n;
        for (uint q = 0u; same && q < dl; q++) same = b[after + q] == b[i + q];
        if (same && b[after + dl] == uchar('"')) {
            uint end = after + dl + 1u;
            if ((o & O_CPP) != 0u && end < n && (is_alpha(b[end]) || b[end] == uchar('_'))) {
                uint e = end;
                for (uint q = 0u; q < n && e < n && (is_word(b[e]) || b[e] == uchar('$')); q++) e++;
                end = e;
            }
            return end;
        }
    }
    return 0u;
}

// The tape kind of the one token `b[0..n)` spells, `RELEX_NONE` where it spells none or more than
// one, `RELEX_REFUSED` where the device does not read it.
inline uint relex(device const uchar* b, uint n, uint o) {
    if (n == 0u) return RELEX_NONE;
    for (uint k = 0u; k < n; k++) {
        if (b[k] >= 0x80u || (b[k] == uchar('\\') && k + 1u < n && (b[k + 1u] == uchar('u') || b[k + 1u] == uchar('U')))) return RELEX_REFUSED;
    }
    uint c = b[0], d = n > 1u ? b[1] : 0u;
    // A comment where a token would start: the lexer passes it, so no token starts at 0.
    if (c == '/' && (d == '/' || d == '*')) return RELEX_NONE;
    if (is_digit(c) || (c == '.' && is_digit(d))) return number_end(b, n, o) == n ? K_NUMBER : RELEX_NONE;
    if (c == '#') return (d == '#' ? n == 2u : n == 1u) ? (d == '#' ? K_HASHHASH : K_HASH) : RELEX_NONE;
    if (c == '%' && d == ':') {
        bool two = n >= 4u && b[2] == '%' && b[3] == ':';
        return n == (two ? 4u : 2u) ? (two ? K_HASHHASH : K_HASH) : RELEX_NONE;
    }
    if (c == '<' && d == ':') {
        // C++'s `<::` is `<`, unless `:` or `>` follows.
        if ((o & O_CPP) != 0u && n > 2u && b[2] == ':' && !(n > 3u && (b[3] == ':' || b[3] == '>'))) return RELEX_NONE;
        return n == 2u ? K_DIGRAPH : RELEX_NONE;
    }
    if ((c == ':' && d == '>') || (c == '<' && d == '%') || (c == '%' && d == '>')) return n == 2u ? K_DIGRAPH : RELEX_NONE;
    if (c == ':' && d == ':' && (o & O_COLONS) != 0u) return n == 2u ? K_PUNCT : RELEX_NONE;
    if (c == '"' || c == '\'') return quoted_end(b, n, 0u, o) == n ? K_LITERAL : RELEX_NONE;
    if (is_alpha(c) || c == '_' || c == '$') {
        // A literal's prefix: `L`, `U`, `u`, `u8`, then `R` where raw strings are read.
        uint k = 0u;
        if (c == 'L' || c == 'U') k = 1u;
        else if (c == 'u') k = d == '8' ? 2u : 1u;
        bool raw = (o & O_RAW) != 0u && k < n && b[k] == 'R';
        uint kq = raw ? k + 1u : k;
        if (kq > 0u && kq < n && (b[kq] == '"' || (b[kq] == '\'' && !raw))) {
            uint e = raw ? raw_end(b, n, kq + 1u, o) : quoted_end(b, n, kq, o);
            // An unclosed raw string's prefix is a name.
            if (raw && e == 0u) return ident_whole(b, n);
            // An unclosed literal is an `Other` of its quote: no one token.
            return e == n ? K_LITERAL : RELEX_NONE;
        }
        return ident_whole(b, n);
    }
    uint p = punct_len(b, n, o);
    return p != 0u && p == n ? K_PUNCT : RELEX_NONE;
}

// For tests: each spelling's answer (`offsets[i]..offsets[i + 1]` of `bytes`).
kernel void pd_relex_test(device const uchar* bytes [[buffer(0)]], device const uint* offsets [[buffer(1)]],
                          device uint* out [[buffer(2)]], constant uint& n [[buffer(3)]],
                          constant uint& o [[buffer(4)]], uint i [[thread_position_in_grid]]) {
    if (i >= n) return;
    uint a = offsets[i];
    out[i] = relex(bytes + a, offsets[i + 1u] - a, o);
}

// --- Setup: the text map -----------------------------------------------------------------------
//
// A file's text tokens are the leaves of its `code` nodes, in order. Each node's owner is the
// nearest node at or before it with leaves (a max scan of their indexes); a text token is a leaf
// whose owner is a `code` node. A scan of those flags gives each its rank, and the ranks place
// the unit's text tokens (`text_nodes`), each file's from its root's rank.

// Each node's index where it has leaves, else 0: the max scan's input.
kernel void pe_owner_in(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                        device const uint* bases [[buffer(2)]], device const uint* total [[buffer(3)]],
                        device uint* out [[buffer(4)]], constant uint& n_files [[buffer(5)]],
                        uint g [[thread_position_in_grid]]) {
    if (g >= total[0]) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files[f].tape);
    out[g] = node_size(t, g - bases[f]) > 1u ? g : 0u;
}

// Whether each node is a text token: a leaf whose owner is a `code` node.
kernel void pe_text_flag(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                         device const uint* bases [[buffer(2)]], device const uint* total [[buffer(3)]],
                         device const uint* owner [[buffer(4)]], device uint* flag [[buffer(5)]],
                         constant uint& n_files [[buffer(6)]], uint g [[thread_position_in_grid]]) {
    if (g >= total[0]) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files[f].tape);
    uint i = g - bases[f];
    flag[g] = node_size(t, i) == 1u && node_kind(t, owner[g] - bases[f]) == K_CODE ? 1u : 0u;
}

// Each text token's node at its rank.
kernel void pe_text_nodes(device const uint* flag [[buffer(0)]], device const uint* rank [[buffer(1)]],
                          device const uint* total [[buffer(2)]], device uint* text_nodes [[buffer(3)]],
                          uint g [[thread_position_in_grid]]) {
    if (g < total[0] && flag[g] != 0u) text_nodes[rank[g]] = g;
}

// --- Setup: the visibility rows (S6, S7) --------------------------------------------------------
//
// Each active `#define` (well-formed) and `#undef` (named) of each instance is a row at its
// directive's slot: its name's key, and its definition (its directive row) or none. A scan
// places the rows after the built-ins' (at slot 0, given by the configuration); one stable radix
// sort orders them by (the key's upper 32 bits, slot). A use at slot `p` sees the latest row of
// its name before `p`: a search, then a walk back past rows of other names whose keys share the
// upper 32 bits (a collision; the walk is bounded, past it the unit is refused).

// An instance of a file in the unit's layout (test input until P1 runs on the device): its file,
// its first directive and its first stretch in the layout's per-directive and per-stretch arrays.
struct PpInst { uint file, dir_first, gap_first; };

// A row's definition where it is an `#undef`, and a lookup's answers that are no row.
constant uint V_UNDEF = 0xffffffffu, L_NOT_NAME = 0xffffffffu, L_NONE = 0xfffffffeu, L_BUILTIN = 0xfffffffdu;
// A row's definition where it is a built-in.
constant uint V_BUILTIN = 0xfffffffdu;
constant uint WALK = 64u;

// Each instance's directive `j`'s row, where it makes one (a flag, then its place by a scan).
kernel void pe_row_flag(device const PpInst* insts [[buffer(0)]], device const uint* n_insts [[buffer(1)]],
                        device const uint* dactive [[buffer(2)]], device const uint* n_dirs [[buffer(3)]],
                        device const uint* dir_bases [[buffer(4)]], device const PpDir* rows [[buffer(5)]],
                        device const PpDef* defs [[buffer(6)]], device const ulong* dir_keys [[buffer(7)]],
                        device uint* flag [[buffer(8)]], uint j [[thread_position_in_grid]]) {
    if (j >= n_dirs[0]) return;
    uint i = 0u, lo = 0u, hi = n_insts[0];
    for (uint k = 0u; k < 32u && hi - lo > 1u; k++) {
        uint mid = (lo + hi) / 2u;
        if (insts[mid].dir_first <= j) lo = mid; else hi = mid;
    }
    i = lo;
    uint r = dir_bases[insts[i].file] + (j - insts[i].dir_first);
    uint kind = rows[r].kind;
    bool row = dactive[j] != 0u && ((kind == K_DEFINE && (defs[r].flags & D_BAD) == 0u) || (kind == K_UNDEF && dir_keys[r] != 0ul));
    flag[j] = row ? 1u : 0u;
}

// The rows at their places (`at`, the flags' scan) after the `n_builtin` built-ins': each its sort
// key (the name's upper 32 bits, its slot), its name's key, and its definition.
kernel void pe_rows(device const PpInst* insts [[buffer(0)]], device const uint* n_insts [[buffer(1)]],
                    device const uint* dslot [[buffer(2)]], device const uint* n_dirs [[buffer(3)]],
                    device const uint* dir_bases [[buffer(4)]], device const PpDir* rows [[buffer(5)]],
                    device const ulong* dir_keys [[buffer(6)]], device const uint* flag [[buffer(7)]],
                    device const uint* at [[buffer(8)]], device ulong* sort_keys [[buffer(9)]],
                    device ulong* name_keys [[buffer(10)]], device uint* values [[buffer(11)]],
                    constant uint& n_builtin [[buffer(12)]], uint j [[thread_position_in_grid]]) {
    if (j >= n_dirs[0] || flag[j] == 0u) return;
    uint lo = 0u, hi = n_insts[0];
    for (uint k = 0u; k < 32u && hi - lo > 1u; k++) {
        uint mid = (lo + hi) / 2u;
        if (insts[mid].dir_first <= j) lo = mid; else hi = mid;
    }
    uint r = dir_bases[insts[lo].file] + (j - insts[lo].dir_first);
    ulong key = dir_keys[r];
    uint place = n_builtin + at[j];
    sort_keys[place] = (key & 0xffffffff00000000ul) | ulong(dslot[j]);
    name_keys[place] = key;
    values[place] = rows[r].kind == K_DEFINE ? r : V_UNDEF;
}

// The rows in all: the built-ins' and the scan's.
kernel void pe_rows_total(device const uint* scanned [[buffer(0)]], device uint* n_rows [[buffer(1)]],
                          constant uint& n_builtin [[buffer(2)]], uint i [[thread_position_in_grid]]) {
    if (i == 0u) n_rows[0] = scanned[0] + n_builtin;
}

// What `key` means at slot `slot`: the latest row of it before the slot, in the sorted rows
// (`sorted`, their places in `perm`).
inline uint visible(device const ulong* sorted, device const uint* perm, uint n, device const ulong* name_keys,
                    device const uint* values, ulong key, uint slot, device atomic_uint* bad) {
    ulong q = (key & 0xffffffff00000000ul) | ulong(slot);
    uint c = pb_lower_bound(sorted, n, q);
    for (uint k = 0u; k < WALK; k++) {
        if (c == 0u || (sorted[c - 1u] >> 32) != (key >> 32)) return L_NONE;
        c--;
        uint p = perm[c];
        if (name_keys[p] == key) {
            uint v = values[p];
            return v == V_UNDEF ? L_NONE : v == V_BUILTIN ? L_BUILTIN : v;
        }
    }
    atomic_fetch_or_explicit(bad, 1u << 29, memory_order_relaxed);
    return L_NONE;
}

// --- Setup: the stretches (S8) and their names' meanings (R4) ------------------------------------

// Each stretch's sort key: its slot. The unit's order is the slots'.
kernel void pe_gap_keys(device const uint* gslot [[buffer(0)]], device const uint* n_gaps [[buffer(1)]],
                        device ulong* gkeys [[buffer(2)]], uint j [[thread_position_in_grid]]) {
    if (j < n_gaps[0]) gkeys[j] = ulong(gslot[j]);
}

// The stretch at place `p` in the unit's order (`order`, the sort's): its tokens (0 where it is
// not active), from the directive before it to the one after it, by the rows' text counts (the
// file's text after its last directive), its first text token and its slot.
kernel void pe_gap_counts(device const PpInst* insts [[buffer(0)]], device const uint* n_insts [[buffer(1)]],
                          device const uint* gactive [[buffer(2)]], device const uint* n_gaps [[buffer(3)]],
                          device const uint* dir_bases [[buffer(4)]], device const uint* dir_total [[buffer(5)]],
                          device const PpDir* rows [[buffer(6)]], device const uint* bases [[buffer(7)]],
                          device const uint* total [[buffer(8)]], device const uint* text_rank [[buffer(9)]],
                          device const uint* text_total [[buffer(10)]], device uint* counts [[buffer(11)]],
                          device uint* firsts [[buffer(12)]], device const uint* order [[buffer(13)]],
                          device const ulong* gkeys [[buffer(14)]], device uint* sslot [[buffer(15)]],
                          constant uint& n_files [[buffer(16)]], uint p [[thread_position_in_grid]]) {
    if (p >= n_gaps[0]) return;
    uint j = order[p];
    uint lo = 0u, hi = n_insts[0];
    for (uint k = 0u; k < 32u && hi - lo > 1u; k++) {
        uint mid = (lo + hi) / 2u;
        if (insts[mid].gap_first <= j) lo = mid; else hi = mid;
    }
    uint f = insts[lo].file, k = j - insts[lo].gap_first;
    uint d0 = dir_bases[f], d1 = f + 1u < n_files ? dir_bases[f + 1u] : dir_total[0];
    uint t0 = text_rank[bases[f]], t1 = f + 1u < n_files ? text_rank[bases[f + 1u]] : text_total[0];
    uint first = k == 0u ? 0u : rows[d0 + k - 1u].text_before;
    uint end = d0 + k < d1 ? rows[d0 + k].text_before : t1 - t0;
    counts[p] = gactive[j] != 0u ? end - first : 0u;
    firsts[p] = t0 + first;
    sslot[p] = uint(gkeys[p]);
}

// Each active stretch token's meaning where it stands (R4's test): `L_NOT_NAME` where it is no
// identifier, else what its key means at its stretch's slot.
kernel void pe_lookups(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                       device const uint* bases [[buffer(2)]], device const ulong* keys [[buffer(3)]],
                       device const uint* text_nodes [[buffer(4)]], device const uint* gap_at [[buffer(5)]],
                       device const uint* n_gaps [[buffer(6)]], device const uint* firsts [[buffer(7)]],
                       device const uint* gslot [[buffer(8)]], device const uint* n_toks [[buffer(9)]],
                       device const ulong* sorted [[buffer(10)]], device const uint* perm [[buffer(11)]],
                       device const uint* n_rows [[buffer(12)]], device const ulong* name_keys [[buffer(13)]],
                       device const uint* values [[buffer(14)]], device uint* out [[buffer(15)]],
                       device atomic_uint* bad [[buffer(16)]], constant uint& n_files [[buffer(17)]],
                       uint s [[thread_position_in_grid]]) {
    if (s >= n_toks[0]) return;
    uint j = pb_item_of(gap_at, n_gaps[0], s);
    uint g = text_nodes[firsts[j] + (s - gap_at[j])];
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files[f].tape);
    if (node_kind(t, g - bases[f]) != K_IDENTIFIER) {
        out[s] = L_NOT_NAME;
        return;
    }
    out[s] = visible(sorted, perm, n_rows[0], name_keys, values, keys[g], gslot[j], bad);
}

// --- Expansion (R5): the top level's grid passes, then the queue's items -------------------------
//
// The top level is the active stretches' tokens, one thread a token (a "top token" `s`, the
// lookups' index), read in place through `text_nodes`. Its outermost calls are lists, each a macro's
// expansion; a list's tokens are refs to tape nodes, written into the arena. The queue's workers
// are SIMD groups: an EXPAND item writes a list's tokens and scans them for calls, each a child
// list pushed as EXPAND; a list whose children are all final adds its size into its parent. After
// the top level's places (a scan of the sizes), PLACE items write each list's tokens at its base,
// top down, and push their children's.

constant uint TOP = 0x80000000u;
constant uint ITEM_PLACE = 0x80000000u;
// Why a unit is refused (`ctl[2]`); `X_ROOM` reruns it at the next rung.
constant uint X_BUILTIN = 1u, X_ARGS = 2u, X_PAST_END = 4u, X_PASTE = 8u, X_DEPTH = 16u, X_ROOM = 32u, X_QUEUE = 64u;
// A chain's lists at most, and an idle worker's polls.
constant uint MAX_CHAIN = 1024u;
constant uint POLLS = 4096u;

// A list: its parent (a list, or `TOP`), its definition's row, its stretch's slot, its tokens in
// the arena, the range of its parent it covers (`lo..=hi`, tokens of the parent, or top tokens),
// its tokens no child covers, its children not yet final and their sizes, its size and its base
// in the output.
struct PxList {
    uint parent, def, slot, first, len, lo, hi, own;
    atomic_uint pending, acc;
    uint size, base;
};

static void px_fence() { atomic_thread_fence(mem_flags::mem_device, memory_order_seq_cst, thread_scope_device); }
static void px_refuse(device atomic_uint* ctl, uint why) { atomic_fetch_or_explicit(&ctl[2], why, memory_order_relaxed); }

// Reading a node of the unit: its file's tape and the node in it.
struct At { Tape t; PpFile f; uint i; };
inline At node_at(device const PpFile* files, device const uchar* data, device const uint* bases, uint n_files, uint g) {
    uint f = file_of(bases, n_files, g);
    At a;
    a.f = files[f];
    a.t = tape_at(data, a.f.tape);
    a.i = g - bases[f];
    return a;
}

// The node of top token `s`, and the end of its stretch.
inline uint top_ref(device const uint* text_nodes, device const uint* gat, uint n_gaps, device const uint* firsts, uint s) {
    uint j = pb_item_of(gat, n_gaps, s);
    return text_nodes[firsts[j] + (s - gat[j])];
}
inline uint top_end(device const uint* gat, uint n_gaps, uint n_toks, uint s) {
    uint j = pb_item_of(gat, n_gaps, s);
    return j + 1u < n_gaps ? gat[j + 1u] : n_toks;
}

// The unit, as every expansion kernel reads it.
struct Ux {
    device const PpFile* files; device const uchar* data; device const uint* bases; uint n_files;
    device const ulong* keys; device const PpDef* defs; device const uint* items;
    device const ulong* sorted; device const uint* perm; uint n_rows; device const ulong* name_keys; device const uint* values;
    device const uint* text_nodes; device const uint* gat; uint n_gaps; device const uint* firsts; uint n_toks;
};

inline bool ref_is(Ux u, uint g, uchar c) {
    At a = node_at(u.files, u.data, u.bases, u.n_files, g);
    return punct_is(a.t, u.data, a.f, a.i, c);
}

// Each top token's call (R5's): an object-like name, or a function-like one followed in its
// stretch by `(` and `)`. `cend` is the call's last token + 1 (0: no call), the max scan's input.
kernel void px_top_calls(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                         device const uint* bases [[buffer(2)]], device const PpDef* defs [[buffer(3)]],
                         device const uint* lookups [[buffer(4)]], device const uint* text_nodes [[buffer(5)]],
                         device const uint* gat [[buffer(6)]], device const uint* n_gaps [[buffer(7)]],
                         device const uint* firsts [[buffer(8)]], device const uint* n_toks [[buffer(9)]],
                         device uint* cend [[buffer(10)]], device atomic_uint* ctl [[buffer(11)]],
                         constant uint& n_files [[buffer(12)]], uint s [[thread_position_in_grid]]) {
    uint n = n_toks[0];
    if (s >= n) return;
    uint m = lookups[s];
    uint e = 0u;
    if (m == L_BUILTIN) {
        px_refuse(ctl, X_BUILTIN);
    } else if (m < L_BUILTIN) {
        if ((defs[m].flags & D_FNLIKE) == 0u) {
            e = s + 1u;
        } else {
            uint end = top_end(gat, n_gaps[0], n, s);
            Ux u; u.files = files; u.data = data; u.bases = bases; u.n_files = n_files;
            if (s + 1u < end && ref_is(u, top_ref(text_nodes, gat, n_gaps[0], firsts, s + 1u), '(')) {
                if (s + 2u < end && ref_is(u, top_ref(text_nodes, gat, n_gaps[0], firsts, s + 2u), ')')) e = s + 3u;
                else px_refuse(ctl, X_ARGS);
            }
        }
    }
    cend[s] = e;
}

// Each top token's state: 1 an outermost call's name (no earlier call reaches it), 2 inside an
// earlier call, 0 neither; and the outermost calls' flags, for their scan.
kernel void px_top_outer(device const uint* cend [[buffer(0)]], device const uint* reach [[buffer(1)]],
                         device const uint* n_toks [[buffer(2)]], device uint* state [[buffer(3)]],
                         device uint* oflag [[buffer(4)]], uint s [[thread_position_in_grid]]) {
    if (s >= n_toks[0]) return;
    uint before = s == 0u ? 0u : reach[s - 1u];
    uint st = before > s ? 2u : cend[s] != 0u ? 1u : 0u;
    state[s] = st;
    oflag[s] = st == 1u ? 1u : 0u;
}

// Each outermost top call's list (its index the scan's) and its EXPAND item.
kernel void px_top_lists(device const uint* lookups [[buffer(0)]], device const uint* cend [[buffer(1)]],
                         device const uint* state [[buffer(2)]], device const uint* oid [[buffer(3)]],
                         device const uint* n_toks [[buffer(4)]], device const uint* gslot [[buffer(5)]],
                         device const uint* gat [[buffer(6)]], device const uint* n_gaps [[buffer(7)]],
                         device PxList* lists [[buffer(8)]], device atomic_uint* slots [[buffer(9)]],
                         device atomic_uint* ctl [[buffer(10)]], constant uint& list_cap [[buffer(11)]],
                         uint s [[thread_position_in_grid]]) {
    if (s >= n_toks[0] || state[s] != 1u) return;
    uint c = oid[s];
    if (c >= list_cap) { px_refuse(ctl, X_ROOM); return; }
    device PxList& l = lists[c];
    l.parent = TOP; l.def = lookups[s]; l.slot = gslot[pb_item_of(gat, n_gaps[0], s)];
    l.first = 0u; l.len = 0u; l.lo = s; l.hi = cend[s] - 1u; l.own = 0u;
    atomic_store_explicit(&l.pending, 1u, memory_order_relaxed);
    atomic_store_explicit(&l.acc, 0u, memory_order_relaxed);
    l.size = 0u; l.base = 0u;
    atomic_store_explicit(&slots[c], c + 1u, memory_order_relaxed);
}

// The queue holds the top calls' items: head 0, tail and active their number; the arena's lists
// start after them.
kernel void px_seed(device const uint* n_calls [[buffer(0)]], device atomic_uint* qctl [[buffer(1)]],
                    device atomic_uint* ctl [[buffer(2)]], constant uint& list_cap [[buffer(3)]],
                    uint i [[thread_position_in_grid]]) {
    if (i != 0u) return;
    uint n = min(n_calls[0], list_cap);
    atomic_store_explicit(&qctl[0], 0u, memory_order_relaxed);
    atomic_store_explicit(&qctl[1], n, memory_order_relaxed);
    atomic_store_explicit(&qctl[2], n, memory_order_relaxed);
    atomic_store_explicit(&qctl[3], 0u, memory_order_relaxed);
    atomic_store_explicit(&ctl[1], n, memory_order_relaxed);
}

// Whether definition `m` is disabled for list `l`'s tokens: a list on its chain expands it.
inline bool px_disabled(device PxList* lists, uint l, uint m, device atomic_uint* ctl) {
    uint k = l;
    for (uint step = 0u; step < MAX_CHAIN; step++) {
        if ((k & TOP) != 0u) return false;
        if (lists[k].def == m) return true;
        k = lists[k].parent;
    }
    px_refuse(ctl, X_DEPTH);
    return true;
}

// Whether the next raw token after list `l`'s end is `(` (R-peek): the token after the range it
// covers in its parent, climbing while that is the parent's end; never past a stretch's end.
inline bool px_peek_paren(Ux u, device PxList* lists, device const uint* tok, uint l) {
    uint k = l;
    for (uint step = 0u; step < MAX_CHAIN; step++) {
        uint p = lists[k].parent, at = lists[k].hi + 1u;
        if ((p & TOP) != 0u) {
            if (at >= top_end(u.gat, u.n_gaps, u.n_toks, lists[k].lo)) return false;
            return ref_is(u, top_ref(u.text_nodes, u.gat, u.n_gaps, u.firsts, at), '(');
        }
        if (at < lists[p].len) return ref_is(u, tok[lists[p].first + at], '(');
        k = p;
    }
    return false;
}

// List `l` is final where its last child is: its size into its parent, climbing (lane 0).
inline void px_final(device PxList* lists, uint l) {
    uint k = l;
    for (uint step = 0u; step < MAX_CHAIN; step++) {
        px_fence();
        uint size = lists[k].own + atomic_load_explicit(&lists[k].acc, memory_order_relaxed);
        lists[k].size = size;
        uint p = lists[k].parent;
        if ((p & TOP) != 0u) return;
        atomic_fetch_add_explicit(&lists[p].acc, size, memory_order_relaxed);
        px_fence();
        if (atomic_fetch_sub_explicit(&lists[p].pending, 1u, memory_order_relaxed) != 1u) return;
        k = p;
    }
}

// EXPAND `l`: its definition's body leaves as its tokens (one atomic add for all), then its
// calls, 32 tokens at a time: a name whose macro is not disabled on its chain, function-like
// ones followed by `(` and `)` in the list. Each call is a child list, pushed.
inline void px_expand(Ux u, device PxList* lists, device uint* tok, device uint* tchild, device atomic_uint* ctl,
                      WorkQueue q, uint tok_cap, uint list_cap, uint l, uint lane) {
    device PxList& L = lists[l];
    PpDef d = u.defs[L.def];
    uint n = d.n_body;
    bool paste = false;
    for (uint j = lane; j < n; j += 32u) paste = paste || (u.items[d.ifirst + j] & I_PASTE) != 0u;
    if (simd_any(paste)) px_refuse(ctl, X_PASTE);
    uint base = 0u;
    if (lane == 0u) base = atomic_fetch_add_explicit(&ctl[0], n, memory_order_relaxed);
    base = simd_broadcast(base, 0);
    if (base + n > tok_cap) {
        if (lane == 0u) px_refuse(ctl, X_ROOM);
        n = 0u;
    }
    for (uint j = lane; j < n; j += 32u) {
        tok[base + j] = d.body + j;
        tchild[base + j] = 0u;
    }
    if (lane == 0u) { L.first = base; L.len = n; }
    px_fence();
    simdgroup_barrier(mem_flags::mem_device);
    uint covered = 0u;
    for (uint c0 = 0u; c0 < n; c0 += 32u) {
        uint j = c0 + lane, m = 0u, e = 0u;
        bool call = false;
        if (j < n) {
            uint g = tok[base + j];
            At a = node_at(u.files, u.data, u.bases, u.n_files, g);
            if (node_kind(a.t, a.i) == K_IDENTIFIER) {
                m = visible(u.sorted, u.perm, u.n_rows, u.name_keys, u.values, u.keys[g], L.slot, ctl + 2);
                if (m == L_BUILTIN) {
                    px_refuse(ctl, X_BUILTIN);
                } else if (m < L_BUILTIN && !px_disabled(lists, l, m, ctl)) {
                    if ((u.defs[m].flags & D_FNLIKE) == 0u) {
                        call = true; e = j;
                    } else if (j + 1u < n) {
                        if (ref_is(u, tok[base + j + 1u], '(')) {
                            if (j + 2u < n && ref_is(u, tok[base + j + 2u], ')')) { call = true; e = j + 2u; }
                            else px_refuse(ctl, X_ARGS);
                        }
                    } else if (px_peek_paren(u, lists, tok, l)) {
                        px_refuse(ctl, X_PAST_END);
                    }
                }
            }
        }
        uint k = simd_sum(call ? 1u : 0u);
        if (k == 0u) continue;
        uint first = 0u;
        if (lane == 0u) {
            first = atomic_fetch_add_explicit(&ctl[1], k, memory_order_relaxed);
            atomic_fetch_add_explicit(&L.pending, k, memory_order_relaxed);
        }
        first = simd_broadcast(first, 0);
        uint child = first + simd_prefix_exclusive_sum(call ? 1u : 0u);
        if (call && child >= list_cap) {
            px_refuse(ctl, X_ROOM);
            call = false;
            // Its count stays pending: the unit reruns at the next rung.
        }
        if (call) {
            device PxList& C = lists[child];
            C.parent = l; C.def = m; C.slot = L.slot; C.first = 0u; C.len = 0u; C.lo = j; C.hi = e; C.own = 0u;
            atomic_store_explicit(&C.pending, 1u, memory_order_relaxed);
            atomic_store_explicit(&C.acc, 0u, memory_order_relaxed);
            C.size = 0u; C.base = 0u;
            tchild[base + j] = child + 1u;
        }
        covered += simd_sum(call ? e - j + 1u : 0u);
        px_fence();
        if (call && !wq_push(q, child)) px_refuse(ctl, X_QUEUE);
    }
    if (lane == 0u) {
        L.own = n - covered;
        px_fence();
        if (atomic_fetch_sub_explicit(&L.pending, 1u, memory_order_relaxed) == 1u) px_final(lists, l);
    }
}

// The max of `x` over the lanes at or before this one.
inline uint px_prefix_max(uint x, uint lane) {
    for (uint k = 1u; k < 32u; k <<= 1u) {
        uint y = simd_shuffle_up(x, k);
        if (lane >= k) x = max(x, y);
    }
    return x;
}

// PLACE `l`: its tokens no child covers at its base, in order, and each child's base (a SIMD
// scan of the pieces' sizes, carried 32 tokens at a time); each child pushed.
inline void px_place(device PxList* lists, device const uint* tok, device const uint* tchild, device uint* out,
                     device atomic_uint* ctl, WorkQueue q, uint out_cap, uint l, uint lane) {
    device PxList& L = lists[l];
    uint at = L.base, reach = 0u;
    for (uint c0 = 0u; c0 < L.len; c0 += 32u) {
        uint j = c0 + lane, v = 0u, child = 0u, ends = 0u;
        if (j < L.len) {
            child = tchild[L.first + j];
            if (child != 0u) ends = lists[child - 1u].hi + 1u;
        }
        uint incl = px_prefix_max(ends, lane);
        uint before = max(reach, simd_shuffle_up(incl, 1u) * (lane == 0u ? 0u : 1u));
        if (j < L.len) v = child != 0u ? lists[child - 1u].size : before > j ? 0u : 1u;
        uint off = at + simd_prefix_exclusive_sum(v);
        if (j < L.len && child == 0u && v == 1u) {
            if (off < out_cap) out[off] = tok[L.first + j]; else px_refuse(ctl, X_ROOM);
        }
        if (child != 0u) {
            lists[child - 1u].base = off;
            px_fence();
            if (!wq_push(q, (child - 1u) | ITEM_PLACE)) px_refuse(ctl, X_QUEUE);
        }
        at += simd_sum(v);
        reach = max(reach, simd_max(incl));
    }
}

// The queue's workers: a SIMD group each, lane 0 taking an item and the 32 lanes doing it.
kernel void px_work(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                    device const uint* bases [[buffer(2)]], device const ulong* keys [[buffer(3)]],
                    device const PpDef* defs [[buffer(4)]], device const uint* items [[buffer(5)]],
                    device const ulong* sorted [[buffer(6)]], device const uint* perm [[buffer(7)]],
                    device const uint* n_rows [[buffer(8)]], device const ulong* name_keys [[buffer(9)]],
                    device const uint* values [[buffer(10)]], device const uint* text_nodes [[buffer(11)]],
                    device const uint* gat [[buffer(12)]], device const uint* n_gaps [[buffer(13)]],
                    device const uint* firsts [[buffer(14)]], device const uint* n_toks [[buffer(15)]],
                    device PxList* lists [[buffer(16)]], device uint* tok [[buffer(17)]],
                    device uint* tchild [[buffer(18)]], device uint* out [[buffer(19)]],
                    device atomic_uint* slots [[buffer(20)]], device atomic_uint* qctl [[buffer(21)]],
                    device atomic_uint* ctl [[buffer(22)]], constant uint& n_files [[buffer(23)]],
                    constant uint& tok_cap [[buffer(24)]], constant uint& list_cap [[buffer(25)]],
                    constant uint& out_cap [[buffer(26)]], constant uint& q_cap [[buffer(27)]],
                    uint lane [[thread_index_in_simdgroup]]) {
    Ux u;
    u.files = files; u.data = data; u.bases = bases; u.n_files = n_files; u.keys = keys; u.defs = defs; u.items = items;
    u.sorted = sorted; u.perm = perm; u.n_rows = n_rows[0]; u.name_keys = name_keys; u.values = values;
    u.text_nodes = text_nodes; u.gat = gat; u.n_gaps = n_gaps[0]; u.firsts = firsts; u.n_toks = n_toks[0];
    WorkQueue q; q.slots = slots; q.control = qctl; q.capacity = q_cap;
    uint polls = 0u;
    for (uint turn = 0u; turn < WQ_TURNS; turn++) {
        uint item = 0u, got = 0u, stop = 0u;
        if (lane == 0u) {
            if (wq_take(q, item)) { got = 1u; polls = 0u; }
            else if (wq_idle(q, polls, POLLS)) stop = 1u;
        }
        got = simd_broadcast(got, 0);
        if (got == 0u) {
            if (simd_broadcast(stop, 0) != 0u) return;
            continue;
        }
        item = simd_broadcast(item, 0);
        px_fence();
        if ((item & ITEM_PLACE) != 0u) px_place(lists, tok, tchild, out, ctl, q, out_cap, item & ~ITEM_PLACE, lane);
        else px_expand(u, lists, tok, tchild, ctl, q, tok_cap, list_cap, item, lane);
        px_fence();
        simdgroup_barrier(mem_flags::mem_device);
        if (lane == 0u) wq_done(q);
    }
}

// Each top token's piece of the output: 1 for a token no call covers, its list's size for an
// outermost call's name, 0 inside a call.
kernel void px_top_sizes(device const uint* state [[buffer(0)]], device const uint* oid [[buffer(1)]],
                         device const PxList* lists [[buffer(2)]], device const uint* n_toks [[buffer(3)]],
                         device uint* v [[buffer(4)]], uint s [[thread_position_in_grid]]) {
    if (s >= n_toks[0]) return;
    uint st = state[s];
    v[s] = st == 0u ? 1u : st == 1u ? lists[oid[s]].size : 0u;
}

// Each top token at its place (`obase`, the sizes' scan), and each outermost call's list's base,
// its PLACE pushed.
kernel void px_top_place(device const uint* state [[buffer(0)]], device const uint* oid [[buffer(1)]],
                         device PxList* lists [[buffer(2)]], device const uint* n_toks [[buffer(3)]],
                         device const uint* obase [[buffer(4)]], device const uint* text_nodes [[buffer(5)]],
                         device const uint* gat [[buffer(6)]], device const uint* n_gaps [[buffer(7)]],
                         device const uint* firsts [[buffer(8)]], device uint* out [[buffer(9)]],
                         device atomic_uint* slots [[buffer(10)]], device atomic_uint* qctl [[buffer(11)]],
                         device atomic_uint* ctl [[buffer(12)]], constant uint& out_cap [[buffer(13)]],
                         constant uint& q_cap [[buffer(14)]], uint s [[thread_position_in_grid]]) {
    if (s >= n_toks[0]) return;
    uint st = state[s];
    if (st == 0u) {
        if (obase[s] < out_cap) out[obase[s]] = top_ref(text_nodes, gat, n_gaps[0], firsts, s);
        else px_refuse(ctl, X_ROOM);
    } else if (st == 1u) {
        uint c = oid[s];
        lists[c].base = obase[s];
        px_fence();
        WorkQueue q; q.slots = slots; q.control = qctl; q.capacity = q_cap;
        if (!wq_push(q, c | ITEM_PLACE)) px_refuse(ctl, X_QUEUE);
    }
}

// After the queue's launches: items left is a refusal; the output's length past its room too.
kernel void px_check(device const atomic_uint* qctl [[buffer(0)]], device const uint* n_out [[buffer(1)]],
                     device atomic_uint* ctl [[buffer(2)]], constant uint& out_cap [[buffer(3)]],
                     uint i [[thread_position_in_grid]]) {
    if (i != 0u) return;
    if (atomic_load_explicit(&qctl[2], memory_order_relaxed) != 0u || atomic_load_explicit(&qctl[3], memory_order_relaxed) != 0u) px_refuse(ctl, X_QUEUE);
    if (n_out[0] > out_cap) px_refuse(ctl, X_ROOM);
}
