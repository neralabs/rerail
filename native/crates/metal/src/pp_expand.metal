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

// --- Expansion -----------------------------------------------------------------------------------
//
// The top level is the active stretches' tokens, one thread a token (a "top token" `s`, the
// lookups' index), read in place through `text_nodes`. Its outermost calls are lists. A list is a
// macro's expansion, or an argument read in isolation (fenced); its tokens are refs (to tape
// nodes) with flags, in the arena. The queue's workers are SIMD groups:
// - CALL finds a call's arguments and makes a list for each parameter used expanded;
// - an argument list's SCAN, then (once final) its flatten: PLACE items writing its tree's
//   tokens into the arena, counted down;
// - SUBST writes the body with the flattened arguments, then scans it (as SCAN does): each call
//   a child list, pushed;
// - a list whose children are all final adds its size into its parent;
// - after the top level's places (a scan of the sizes), PLACE writes each list at its base.

constant uint TOP = 0x80000000u, NO_DEF = 0xffffffffu;
constant uint IT_SUBST = 0u, IT_PLACE = 1u << 30, IT_CALL = 2u << 30, IT_SCAN = 3u << 30, IT_KIND = 3u << 30;
// Why a unit is refused (`ctl[2]`); `X_ROOM` reruns it at the next rung.
constant uint X_BUILTIN = 1u, X_PAST_END = 4u, X_PASTE = 8u, X_DEPTH = 16u, X_ROOM = 32u, X_QUEUE = 64u;
constant uint X_HASH = 128u, X_ACROSS = 256u, X_VA_OPT = 512u;
// A token's flags: painted (never a candidate again).
constant uint T_NOEXP = 1u;
// A list's kind.
constant uint LK_MACRO = 0u, LK_ARG = 1u;
// The close table's keys: a depth (biased) above a top token's position.
constant uint DEPTH_BIAS = 1u << 15, POS_BITS = 24u;
// A chain's lists at most, and an idle worker's polls.
constant uint MAX_CHAIN = 1024u;
constant uint POLLS = 4096u;

// A list: its parent (a list, or `TOP`), its definition's row (`NO_DEF`: an argument), its
// stretch's slot, its tokens in the arena, the range of its parent it covers (`lo..=hi`) or, an
// argument, the raw range it was read from, its tokens no child covers, its children not yet
// final and their sizes, its size and its base. A call: its arguments' entries and the argument
// lists not yet flattened. An argument: its call's list, and its flatten's PLACE items not done.
// `dest`: where its PLACE writes, the output (0) or the arena for argument list `dest - 1`.
struct PxList {
    uint parent, def, slot, first, len, lo, hi, own;
    atomic_uint pending, acc;
    uint size, base, kind, call, dest, args, nargs;
    atomic_uint apending, fpending;
    uint pad;
};
// A call's argument: its raw range in the call's parent (`lo..hi`) and its list (0 none, 1 being
// made, else the list + 2).
struct PxArg { uint lo, hi; atomic_uint list; };

// The lanes where `b` holds, as bits.
inline ulong px_ballot(bool b) { return ulong(simd_vote::vote_t(simd_ballot(b))); }

static void px_fence() { atomic_thread_fence(mem_flags::mem_device, memory_order_seq_cst, thread_scope_device); }
static void px_refuse(device atomic_uint* ctl, uint why) { atomic_fetch_or_explicit(&ctl[2], why, memory_order_relaxed); }

inline void px_init(device PxList& l, uint parent, uint def, uint slot, uint kind) {
    l.parent = parent; l.def = def; l.slot = slot; l.first = 0u; l.len = 0u; l.lo = 0u; l.hi = 0u; l.own = 0u;
    atomic_store_explicit(&l.pending, 1u, memory_order_relaxed);
    atomic_store_explicit(&l.acc, 0u, memory_order_relaxed);
    l.size = 0u; l.base = 0u; l.kind = kind; l.call = 0u; l.dest = 0u; l.args = 0u; l.nargs = 0u;
    atomic_store_explicit(&l.apending, 1u, memory_order_relaxed);
    atomic_store_explicit(&l.fpending, 0u, memory_order_relaxed);
}

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
    device const uint* tdepth; ulong va_opt;
};

inline bool ref_is(Ux u, uint g, uchar c) {
    At a = node_at(u.files, u.data, u.bases, u.n_files, g);
    return punct_is(a.t, u.data, a.f, a.i, c);
}
inline uint ref_kind(Ux u, uint g) {
    At a = node_at(u.files, u.data, u.bases, u.n_files, g);
    return node_kind(a.t, a.i);
}
// A paren's depth change.
inline uint paren_delta(Ux u, uint g) {
    At a = node_at(u.files, u.data, u.bases, u.n_files, g);
    return punct_is(a.t, u.data, a.f, a.i, '(') ? 1u : punct_is(a.t, u.data, a.f, a.i, ')') ? 0xffffffffu : 0u;
}

// --- The top level: depth, the close table, the calls --------------------------------------------

// Each top token's stretch head (for the depth's segmented scan), its depth change, and whether it
// is a `)` (for the close table).
kernel void px_top_parens(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                          device const uint* bases [[buffer(2)]], device const uint* text_nodes [[buffer(3)]],
                          device const uint* gat [[buffer(4)]], device const uint* n_gaps [[buffer(5)]],
                          device const uint* firsts [[buffer(6)]], device const uint* n_toks [[buffer(7)]],
                          device uint* heads [[buffer(8)]], device uint* delta [[buffer(9)]],
                          device uint* close [[buffer(10)]], constant uint& n_files [[buffer(11)]],
                          uint s [[thread_position_in_grid]]) {
    if (s >= n_toks[0]) return;
    Ux u; u.files = files; u.data = data; u.bases = bases; u.n_files = n_files;
    uint j = pb_item_of(gat, n_gaps[0], s);
    uint d = paren_delta(u, text_nodes[firsts[j] + (s - gat[j])]);
    heads[s] = s == gat[j] ? 1u : 0u;
    delta[s] = d;
    close[s] = d == 0xffffffffu ? 1u : 0u;
}

// Each `)`'s key at its place: its depth after it (biased), its position below.
kernel void px_close_keys(device const uint* depth [[buffer(0)]], device const uint* close [[buffer(1)]],
                          device const uint* at [[buffer(2)]], device const uint* n_toks [[buffer(3)]],
                          device ulong* keys [[buffer(4)]], device atomic_uint* ctl [[buffer(5)]],
                          uint s [[thread_position_in_grid]]) {
    if (s >= n_toks[0] || close[s] == 0u) return;
    uint b = depth[s] + DEPTH_BIAS;
    if (b >= 2u * DEPTH_BIAS || s >= (1u << POS_BITS)) { px_refuse(ctl, X_DEPTH); return; }
    keys[at[s]] = (ulong(b) << POS_BITS) | ulong(s);
}

// The `)` that closes the `(` at top token `open`: the first after it back at the depth before
// it, in its stretch (a search of the sorted close table); `0xffffffff` where none is.
inline uint top_close(device const ulong* closes, uint n_close, device const uint* depth, uint open, uint end) {
    uint b = (depth[open] - 1u + DEPTH_BIAS) & (2u * DEPTH_BIAS - 1u);
    ulong q = (ulong(b) << POS_BITS) | ulong(open + 1u);
    uint c = pb_lower_bound(closes, n_close, q);
    if (c >= n_close || uint(closes[c] >> POS_BITS) != b) return 0xffffffffu;
    uint pos = uint(closes[c] & ((1ul << POS_BITS) - 1ul));
    return pos < end ? pos : 0xffffffffu;
}

// Each top token's call: an object-like name, or a function-like one followed in its stretch by
// `(`, to its `)`. `cend` is the call's last token + 1 (0: no call), the max scan's input.
kernel void px_top_calls(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                         device const uint* bases [[buffer(2)]], device const PpDef* defs [[buffer(3)]],
                         device const uint* lookups [[buffer(4)]], device const uint* text_nodes [[buffer(5)]],
                         device const uint* gat [[buffer(6)]], device const uint* n_gaps [[buffer(7)]],
                         device const uint* firsts [[buffer(8)]], device const uint* n_toks [[buffer(9)]],
                         device const uint* depth [[buffer(10)]], device const ulong* closes [[buffer(11)]],
                         device const uint* n_close [[buffer(12)]], device uint* cend [[buffer(13)]],
                         device atomic_uint* ctl [[buffer(14)]], constant uint& n_files [[buffer(15)]],
                         uint s [[thread_position_in_grid]]) {
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
                uint c = top_close(closes, n_close[0], depth, s + 1u, end);
                // A call whose arguments go on past its stretch (a directive among them, or its
                // file's end) is the CPU's.
                if (c == 0xffffffffu) px_refuse(ctl, X_ACROSS);
                else e = c + 1u;
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

// Each outermost top call's list (its index the scan's) and its item: CALL for a function-like
// macro, SUBST for an object-like one.
kernel void px_top_lists(device const uint* lookups [[buffer(0)]], device const uint* cend [[buffer(1)]],
                         device const uint* state [[buffer(2)]], device const uint* oid [[buffer(3)]],
                         device const uint* n_toks [[buffer(4)]], device const uint* gslot [[buffer(5)]],
                         device const uint* gat [[buffer(6)]], device const uint* n_gaps [[buffer(7)]],
                         device PxList* lists [[buffer(8)]], device atomic_uint* slots [[buffer(9)]],
                         device atomic_uint* ctl [[buffer(10)]], device const PpDef* defs [[buffer(11)]],
                         constant uint& list_cap [[buffer(12)]], uint s [[thread_position_in_grid]]) {
    if (s >= n_toks[0] || state[s] != 1u) return;
    uint c = oid[s];
    if (c >= list_cap) { px_refuse(ctl, X_ROOM); return; }
    uint m = lookups[s];
    device PxList& l = lists[c];
    px_init(l, TOP, m, gslot[pb_item_of(gat, n_gaps[0], s)], LK_MACRO);
    l.lo = s; l.hi = cend[s] - 1u;
    uint item = (defs[m].flags & D_FNLIKE) != 0u ? (c | IT_CALL) : (c | IT_SUBST);
    atomic_store_explicit(&slots[c], item + 1u, memory_order_relaxed);
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

// --- The queue's items ---------------------------------------------------------------------------

// The arena, as every item reads it.
struct Ar {
    device PxList* lists; device uint* tok; device uint* tflag; device uint* tchild; device uint* tdepth;
    device PxArg* args; device uint* out; device atomic_uint* ctl; WorkQueue q;
    uint tok_cap, list_cap, arg_cap, out_cap;
};

inline void px_push(Ar a, uint item) { if (!wq_push(a.q, item)) px_refuse(a.ctl, X_QUEUE); }

// Token `k` of list `p` (or top token `k`, `p` being `TOP`): its ref, depth after it, and flags.
inline uint par_ref(Ux u, Ar a, uint p, uint k) { return (p & TOP) != 0u ? top_ref(u.text_nodes, u.gat, u.n_gaps, u.firsts, k) : a.tok[a.lists[p].first + k]; }
inline uint par_depth(Ux u, Ar a, uint p, uint k) { return (p & TOP) != 0u ? u.tdepth[k] : a.tdepth[a.lists[p].first + k]; }
inline uint par_flags(Ar a, uint p, uint k) { return (p & TOP) != 0u ? 0u : a.tflag[a.lists[p].first + k]; }

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
// covers in its parent, climbing while that is the parent's end; never past a fence (an argument
// list's end) nor a stretch's end.
inline bool px_peek_paren(Ux u, device PxList* lists, device const uint* tok, uint l) {
    uint k = l;
    for (uint step = 0u; step < MAX_CHAIN; step++) {
        if (lists[k].kind == LK_ARG) return false;
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

// List `l` is final where its last child is (lane 0). A macro's list adds its size into its
// parent, climbing; an argument's starts its flatten: room for its tokens, its PLACE pushed.
inline void px_final(Ar a, uint l) {
    uint k = l;
    for (uint step = 0u; step < MAX_CHAIN; step++) {
        px_fence();
        device PxList& K = a.lists[k];
        uint size = K.own + atomic_load_explicit(&K.acc, memory_order_relaxed);
        K.size = size;
        if (K.kind == LK_ARG) {
            uint base = atomic_fetch_add_explicit(&a.ctl[0], size, memory_order_relaxed);
            if (base + size > a.tok_cap) { px_refuse(a.ctl, X_ROOM); return; }
            K.base = base;
            K.dest = k + 1u;
            atomic_store_explicit(&K.fpending, 1u, memory_order_relaxed);
            px_fence();
            px_push(a, k | IT_PLACE);
            return;
        }
        uint p = K.parent;
        if ((p & TOP) != 0u) return;
        atomic_fetch_add_explicit(&a.lists[p].acc, size, memory_order_relaxed);
        px_fence();
        if (atomic_fetch_sub_explicit(&a.lists[p].pending, 1u, memory_order_relaxed) != 1u) return;
        k = p;
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

// SCAN `l`'s tokens (written): their depths (a SIMD scan carried 32 at a time), then their
// calls: a name not painted whose macro is not disabled on the chain, a function-like one
// followed by `(` in the list, to its `)` (a SIMD search a call); the outermost (a carried max of
// the calls' reach) each a child list, pushed.
inline void px_scan(Ux u, Ar a, uint l, uint lane) {
    device PxList& L = a.lists[l];
    uint n = L.len, first = L.first;
    uint carry = 0u;
    for (uint c0 = 0u; c0 < n; c0 += 32u) {
        uint j = c0 + lane;
        uint d = j < n ? paren_delta(u, a.tok[first + j]) : 0u;
        uint depth = carry + simd_prefix_inclusive_sum(d);
        if (j < n) a.tdepth[first + j] = depth;
        carry += simd_sum(d);
    }
    px_fence();
    simdgroup_barrier(mem_flags::mem_device);
    uint covered = 0u, reach = 0u;
    for (uint c0 = 0u; c0 < n; c0 += 32u) {
        uint j = c0 + lane, m = 0u, e = 0u;
        bool call = false, fn = false, open = false;
        if (j < n && (a.tflag[first + j] & T_NOEXP) == 0u) {
            uint g = a.tok[first + j];
            if (ref_kind(u, g) == K_IDENTIFIER) {
                m = visible(u.sorted, u.perm, u.n_rows, u.name_keys, u.values, u.keys[g], L.slot, a.ctl + 2);
                if (m == L_BUILTIN) {
                    px_refuse(a.ctl, X_BUILTIN);
                } else if (m < L_BUILTIN) {
                    if (px_disabled(a.lists, l, m, a.ctl)) {
                        a.tflag[first + j] |= T_NOEXP;
                    } else if ((u.defs[m].flags & D_FNLIKE) == 0u) {
                        call = true; e = j;
                    } else if (j + 1u < n) {
                        open = ref_is(u, a.tok[first + j + 1u], '(');
                        fn = true;
                    } else if (px_peek_paren(u, a.lists, a.tok, l)) {
                        px_refuse(a.ctl, X_PAST_END);
                    }
                }
            }
        }
        // Each `(` after a function-like name: its `)`, searched by the 32 lanes together.
        ulong todo = px_ballot(fn && open);
        for (uint guard = 0u; guard < 32u && todo != 0u; guard++) {
            uint who = uint(ctz(todo));
            todo &= todo - 1u;
            uint jw = simd_shuffle(j, ushort(who));
            uint target = a.tdepth[first + jw + 1u] - 1u;
            uint found = 0xffffffffu;
            for (uint k0 = jw + 2u; k0 < n && found == 0xffffffffu; k0 += 32u) {
                uint k = k0 + lane;
                bool hit = k < n && a.tdepth[first + k] == target && ref_is(u, a.tok[first + k], ')');
                ulong h = px_ballot(hit);
                if (h != 0u) found = k0 + uint(ctz(h));
            }
            if (lane == who) {
                if (found == 0xffffffffu) px_refuse(a.ctl, X_PAST_END);
                else { call = true; e = found; }
            }
        }
        uint incl = px_prefix_max(call ? e + 1u : 0u, lane);
        uint before = max(reach, lane == 0u ? 0u : simd_shuffle_up(incl, 1u));
        reach = max(reach, simd_max(incl));
        call = call && before <= j;
        uint k = simd_sum(call ? 1u : 0u);
        if (k == 0u) continue;
        uint cfirst = 0u;
        if (lane == 0u) {
            cfirst = atomic_fetch_add_explicit(&a.ctl[1], k, memory_order_relaxed);
            atomic_fetch_add_explicit(&L.pending, k, memory_order_relaxed);
        }
        cfirst = simd_broadcast(cfirst, 0);
        uint child = cfirst + simd_prefix_exclusive_sum(call ? 1u : 0u);
        if (call && child >= a.list_cap) {
            px_refuse(a.ctl, X_ROOM);
            call = false;
        }
        if (call) {
            device PxList& C = a.lists[child];
            px_init(C, l, m, L.slot, LK_MACRO);
            C.lo = j; C.hi = e;
            a.tchild[first + j] = child + 1u;
        }
        covered += simd_sum(call ? e - j + 1u : 0u);
        px_fence();
        if (call) px_push(a, child | ((u.defs[m].flags & D_FNLIKE) != 0u ? IT_CALL : IT_SUBST));
    }
    if (lane == 0u) {
        L.own = n - covered;
        px_fence();
        if (atomic_fetch_sub_explicit(&L.pending, 1u, memory_order_relaxed) == 1u) px_final(a, l);
    }
}

// Copies `n` tokens (refs and flags) from `src` to `dst` of the arena, by the 32 lanes.
inline void px_copy(Ar a, uint dst, uint src, uint n, uint lane) {
    for (uint j = lane; j < n; j += 32u) {
        a.tok[dst + j] = a.tok[src + j];
        a.tflag[dst + j] = a.tflag[src + j];
        a.tchild[dst + j] = 0u;
    }
}

// CALL `l`: its arguments in its parent (the top-level commas between its `(` and `)`, by
// ballots carried 32 at a time; past the last parameter a variadic macro's commas join), the
// count checked (a wrong one leaves the name alone, painted), and a list for each parameter used
// expanded (claimed once), its tokens the argument's refs, its SCAN pushed. SUBST follows when the
// last is flattened.
inline void px_call(Ux u, Ar a, uint l, uint lane) {
    device PxList& L = a.lists[l];
    PpDef d = u.defs[L.def];
    uint np = d.n_params >= 2u ? (d.n_params - 1u) / 2u : 0u;
    bool va = (d.flags & D_VARIADIC) != 0u;
    uint p = L.parent, open = L.lo + 1u, close = L.hi;
    uint depth = par_depth(u, a, p, open);
    // The splitting commas.
    uint splits = 0u;
    for (uint c0 = open + 1u; c0 < close; c0 += 32u) {
        uint k = c0 + lane;
        bool comma = k < close && par_depth(u, a, p, k) == depth && ref_is(u, par_ref(u, a, p, k), ',');
        uint before = splits + simd_prefix_exclusive_sum(comma ? 1u : 0u);
        comma = comma && !(va && before + 1u >= np);
        splits += simd_sum(comma ? 1u : 0u);
    }
    bool empty = close == open + 1u;
    uint nargs = np == 0u ? 0u : splits + 1u;
    bool fill = np > 0u && va && nargs + 1u == np;
    bool wrong = np == 0u ? !empty : (fill ? false : nargs != np);
    if (wrong) {
        // The name alone, painted.
        uint base = 0u;
        if (lane == 0u) base = atomic_fetch_add_explicit(&a.ctl[0], 1u, memory_order_relaxed);
        base = simd_broadcast(base, 0);
        if (lane == 0u) {
            if (base + 1u > a.tok_cap) {
                px_refuse(a.ctl, X_ROOM);
                L.len = 0u;
            } else {
                a.tok[base] = par_ref(u, a, p, L.lo);
                a.tflag[base] = par_flags(a, p, L.lo) | T_NOEXP;
                a.tchild[base] = 0u;
                L.len = 1u;
            }
            L.first = base;
            L.own = L.len;
            px_fence();
            if (atomic_fetch_sub_explicit(&L.pending, 1u, memory_order_relaxed) == 1u) px_final(a, l);
        }
        return;
    }
    // The arguments' entries: each one's raw range.
    uint ab = 0u;
    if (lane == 0u) ab = atomic_fetch_add_explicit(&a.ctl[3], np, memory_order_relaxed);
    ab = simd_broadcast(ab, 0);
    if (ab + np > a.arg_cap) {
        if (lane == 0u) px_refuse(a.ctl, X_ROOM);
        np = 0u;
    }
    if (lane == 0u) {
        L.args = ab; L.nargs = np;
        if (np > 0u) a.args[ab].lo = open + 1u;
        if (fill) { a.args[ab + np - 1u].lo = close; a.args[ab + np - 1u].hi = close; }
    }
    for (uint q = lane; q < np; q += 32u) atomic_store_explicit(&a.args[ab + q].list, 0u, memory_order_relaxed);
    uint rank = 0u;
    for (uint c0 = open + 1u; c0 < close && np > 0u; c0 += 32u) {
        uint k = c0 + lane;
        bool comma = k < close && par_depth(u, a, p, k) == depth && ref_is(u, par_ref(u, a, p, k), ',');
        uint before = rank + simd_prefix_exclusive_sum(comma ? 1u : 0u);
        comma = comma && !(va && before + 1u >= np);
        uint r = rank + simd_prefix_exclusive_sum(comma ? 1u : 0u);
        if (comma) { a.args[ab + r].hi = k; a.args[ab + r + 1u].lo = k + 1u; }
        rank += simd_sum(comma ? 1u : 0u);
    }
    if (lane == 0u && np > 0u) a.args[ab + (fill ? np - 2u : np - 1u)].hi = close;
    px_fence();
    simdgroup_barrier(mem_flags::mem_device);
    // The body's uses of parameters: `#` and `##` are later steps' (refused); each parameter used
    // otherwise, with an argument, gets its list once.
    uint n = d.n_body;
    for (uint c0 = 0u; c0 < n; c0 += 32u) {
        uint i = c0 + lane;
        uint q = 0xffffu;
        if (i < n) {
            uint it = u.items[d.ifirst + i];
            if ((it & I_PASTE) != 0u) px_refuse(a.ctl, X_PASTE);
            uint kind = ref_kind(u, d.body + i);
            if (kind == K_HASH) px_refuse(a.ctl, X_HASH);
            if (va && kind == K_IDENTIFIER && u.keys[d.body + i] == u.va_opt) px_refuse(a.ctl, X_VA_OPT);
            q = it & 0xffffu;
        }
        bool make = false;
        if (q != NO_PARAM && q < np && a.args[ab + q].hi > a.args[ab + q].lo) {
            // The weak exchange may fail with the slot still free: try again while it is.
            for (uint t = 0u; t < 64u && !make; t++) {
                uint none = 0u;
                make = atomic_compare_exchange_weak_explicit(&a.args[ab + q].list, &none, 1u, memory_order_relaxed, memory_order_relaxed);
                if (none != 0u) break;
            }
        }
        // Each list made by the 32 lanes, one after another.
        ulong todo = px_ballot(make);
        for (uint guard = 0u; guard < 32u && todo != 0u; guard++) {
            uint who = uint(ctz(todo));
            todo &= todo - 1u;
            uint qw = simd_shuffle(q, ushort(who));
            uint lo = a.args[ab + qw].lo, len = a.args[ab + qw].hi - lo;
            uint id = 0u, base = 0u;
            if (lane == 0u) {
                id = atomic_fetch_add_explicit(&a.ctl[1], 1u, memory_order_relaxed);
                base = atomic_fetch_add_explicit(&a.ctl[0], len, memory_order_relaxed);
            }
            id = simd_broadcast(id, 0);
            base = simd_broadcast(base, 0);
            if (id >= a.list_cap || base + len > a.tok_cap) {
                if (lane == 0u) px_refuse(a.ctl, X_ROOM);
                continue;
            }
            for (uint j = lane; j < len; j += 32u) {
                a.tok[base + j] = par_ref(u, a, p, lo + j);
                a.tflag[base + j] = par_flags(a, p, lo + j);
                a.tchild[base + j] = 0u;
            }
            if (lane == 0u) {
                device PxList& A = a.lists[id];
                px_init(A, p, NO_DEF, L.slot, LK_ARG);
                A.first = base; A.len = len; A.lo = lo; A.hi = lo + len; A.call = l;
                atomic_store_explicit(&a.args[ab + qw].list, id + 2u, memory_order_relaxed);
                atomic_fetch_add_explicit(&L.apending, 1u, memory_order_relaxed);
            }
            px_fence();
            simdgroup_barrier(mem_flags::mem_device);
            if (lane == 0u) px_push(a, id | IT_SCAN);
        }
    }
    if (lane == 0u) {
        px_fence();
        if (atomic_fetch_sub_explicit(&L.apending, 1u, memory_order_relaxed) == 1u) px_push(a, l | IT_SUBST);
    }
}

// SUBST `l`: its body's items, a parameter used expanded its argument's flattened tokens (their
// paint kept), at offsets by a SIMD scan of the items' counts (one atomic add for all); then the
// list is scanned.
inline void px_subst(Ux u, Ar a, uint l, uint lane) {
    device PxList& L = a.lists[l];
    PpDef d = u.defs[L.def];
    uint n = d.n_body, total = 0u;
    // The count of each item: 1, or its argument's flattened size.
    for (uint c0 = 0u; c0 < n; c0 += 32u) {
        uint i = c0 + lane, v = 0u;
        if (i < n) {
            uint q = u.items[d.ifirst + i] & 0xffffu;
            if (q == NO_PARAM || q >= L.nargs) v = 1u;
            else {
                uint id = atomic_load_explicit(&a.args[L.args + q].list, memory_order_relaxed);
                v = id >= 2u ? a.lists[id - 2u].size : 0u;
            }
        }
        total += simd_sum(v);
    }
    uint base = 0u;
    if (lane == 0u) base = atomic_fetch_add_explicit(&a.ctl[0], total, memory_order_relaxed);
    base = simd_broadcast(base, 0);
    if (base + total > a.tok_cap) {
        if (lane == 0u) px_refuse(a.ctl, X_ROOM);
        total = 0u;
        n = 0u;
    }
    uint at = base;
    for (uint c0 = 0u; c0 < n; c0 += 32u) {
        uint i = c0 + lane, v = 0u, src = 0u;
        bool param = false;
        if (i < n) {
            uint q = u.items[d.ifirst + i] & 0xffffu;
            if (q == NO_PARAM || q >= L.nargs) v = 1u;
            else {
                param = true;
                uint id = atomic_load_explicit(&a.args[L.args + q].list, memory_order_relaxed);
                if (id >= 2u) { v = a.lists[id - 2u].size; src = a.lists[id - 2u].base; }
            }
        }
        uint off = at + simd_prefix_exclusive_sum(v);
        if (i < n && !param) {
            a.tok[off] = d.body + i;
            a.tflag[off] = 0u;
            a.tchild[off] = 0u;
        }
        ulong todo = px_ballot(param && v > 0u);
        for (uint guard = 0u; guard < 32u && todo != 0u; guard++) {
            uint who = uint(ctz(todo));
            todo &= todo - 1u;
            px_copy(a, simd_shuffle(off, ushort(who)), simd_shuffle(src, ushort(who)), simd_shuffle(v, ushort(who)), lane);
        }
        at += simd_sum(v);
    }
    if (lane == 0u) { L.first = base; L.len = total; }
    px_fence();
    simdgroup_barrier(mem_flags::mem_device);
    px_scan(u, a, l, lane);
}

// PLACE `l`: its tokens no child covers at its base, in order, and each child's base (a SIMD
// scan of the pieces' sizes, carried 32 at a time), each child's PLACE pushed. It writes to the
// output, or (flattening an argument) to the arena with the tokens' paint, and counts its
// flatten's items down: the last done tells the call.
inline void px_place(Ar a, uint l, uint lane) {
    device PxList& L = a.lists[l];
    uint dest = L.dest;
    uint at = L.base, reach = 0u;
    for (uint c0 = 0u; c0 < L.len; c0 += 32u) {
        uint j = c0 + lane, v = 0u, child = 0u, ends = 0u;
        if (j < L.len) {
            child = a.tchild[L.first + j];
            if (child != 0u) ends = a.lists[child - 1u].hi + 1u;
        }
        uint incl = px_prefix_max(ends, lane);
        uint before = max(reach, lane == 0u ? 0u : simd_shuffle_up(incl, 1u));
        if (j < L.len) v = child != 0u ? a.lists[child - 1u].size : before > j ? 0u : 1u;
        uint off = at + simd_prefix_exclusive_sum(v);
        if (j < L.len && child == 0u && v == 1u) {
            if (dest == 0u) {
                if (off < a.out_cap) a.out[off] = a.tok[L.first + j]; else px_refuse(a.ctl, X_ROOM);
            } else {
                a.tok[off] = a.tok[L.first + j];
                a.tflag[off] = a.tflag[L.first + j];
                a.tchild[off] = 0u;
            }
        }
        uint k = simd_sum(child != 0u ? 1u : 0u);
        if (dest != 0u && lane == 0u && k != 0u) atomic_fetch_add_explicit(&a.lists[dest - 1u].fpending, k, memory_order_relaxed);
        if (child != 0u) {
            a.lists[child - 1u].base = off;
            a.lists[child - 1u].dest = dest;
            px_fence();
            px_push(a, (child - 1u) | IT_PLACE);
        }
        at += simd_sum(v);
        reach = max(reach, simd_max(incl));
    }
    if (dest != 0u && lane == 0u) {
        px_fence();
        device PxList& A = a.lists[dest - 1u];
        if (atomic_fetch_sub_explicit(&A.fpending, 1u, memory_order_relaxed) == 1u) {
            device PxList& C = a.lists[A.call];
            px_fence();
            if (atomic_fetch_sub_explicit(&C.apending, 1u, memory_order_relaxed) == 1u) px_push(a, A.call | IT_SUBST);
        }
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
                    device const uint* top_depth [[buffer(16)]], device PxList* lists [[buffer(17)]],
                    device uint* tok [[buffer(18)]], device uint* tflag [[buffer(19)]],
                    device uint* tchild [[buffer(20)]], device uint* tdepth [[buffer(21)]],
                    device PxArg* args [[buffer(22)]], device uint* out [[buffer(23)]],
                    device atomic_uint* slots [[buffer(24)]], device atomic_uint* qctl [[buffer(25)]],
                    device atomic_uint* ctl [[buffer(26)]], constant uint* words [[buffer(27)]],
                    uint lane [[thread_index_in_simdgroup]]) {
    // words: n_files, tok_cap, list_cap, arg_cap, out_cap, q_cap, va_opt (low, high).
    Ux u;
    u.files = files; u.data = data; u.bases = bases; u.n_files = words[0]; u.keys = keys; u.defs = defs; u.items = items;
    u.sorted = sorted; u.perm = perm; u.n_rows = n_rows[0]; u.name_keys = name_keys; u.values = values;
    u.text_nodes = text_nodes; u.gat = gat; u.n_gaps = n_gaps[0]; u.firsts = firsts; u.n_toks = n_toks[0];
    u.tdepth = top_depth; u.va_opt = ulong(words[6]) | (ulong(words[7]) << 32);
    Ar a;
    a.lists = lists; a.tok = tok; a.tflag = tflag; a.tchild = tchild; a.tdepth = tdepth; a.args = args; a.out = out; a.ctl = ctl;
    a.q.slots = slots; a.q.control = qctl; a.q.capacity = words[5];
    a.tok_cap = words[1]; a.list_cap = words[2]; a.arg_cap = words[3]; a.out_cap = words[4];
    uint polls = 0u;
    for (uint turn = 0u; turn < WQ_TURNS; turn++) {
        uint item = 0u, got = 0u, stop = 0u;
        if (lane == 0u) {
            if (wq_take(a.q, item)) { got = 1u; polls = 0u; }
            else if (wq_idle(a.q, polls, POLLS)) stop = 1u;
        }
        got = simd_broadcast(got, 0);
        if (got == 0u) {
            if (simd_broadcast(stop, 0) != 0u) return;
            continue;
        }
        item = simd_broadcast(item, 0);
        px_fence();
        uint l = item & ~IT_KIND;
        switch (item & IT_KIND) {
            case IT_PLACE: px_place(a, l, lane); break;
            case IT_CALL: px_call(u, a, l, lane); break;
            case IT_SCAN: px_scan(u, a, l, lane); break;
            default: px_subst(u, a, l, lane); break;
        }
        px_fence();
        simdgroup_barrier(mem_flags::mem_device);
        if (lane == 0u) wq_done(a.q);
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
        lists[c].dest = 0u;
        px_fence();
        WorkQueue q; q.slots = slots; q.control = qctl; q.capacity = q_cap;
        if (!wq_push(q, c | IT_PLACE)) px_refuse(ctl, X_QUEUE);
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
