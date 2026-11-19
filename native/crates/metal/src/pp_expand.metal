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
    device const uchar* b = data + pf_src(f);
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
    return node_kind(t, i) == K_PUNCT && node_end(t, i) - node_start(t, i) == 1u && data[pf_src(f) + node_start(t, i)] == c;
}
inline bool is_dots(Tape t, device const uchar* data, PpFile f, uint i) {
    uint s = node_start(t, i);
    return node_kind(t, i) == K_PUNCT && node_end(t, i) - s == 3u && data[pf_src(f) + s] == uchar('.');
}

// S1: each row's definition, and the leaves of its parameters and its body (to be scanned).
inline void pd_defs(device const PpFile* files, device const uchar* data, device const uint* bases, device const uint* dir_bases, device const uint* dir_total, device const PpDir* rows, device PpDef* defs, device uint* pcount, device uint* bcount, uint n_files, uint r) {
    if (r >= dir_total[0]) return;
    PpDef d = {0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u};
    PpDir row = rows[r];
    if (row.kind == K_DEFINE) {
        uint f = file_of(dir_bases, n_files, r);
        Tape t = tape_at(data, files, f);
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
inline void pd_params(device const PpFile* files, device const uchar* data, device const uint* bases, device const ulong* keys, device const uint* dir_bases, device const uint* dir_total, device PpDef* defs, device const uint* pfirst, device const uint* ptotal, device ulong* pkeys, device atomic_uint* room, uint n_files, uint va_lo, uint va_hi, uint cap, uint s) {
    if (s >= ptotal[0]) return;
    if (s >= cap) {
        atomic_store_explicit(room, 1u, memory_order_relaxed);
        return;
    }
    uint r = pb_item_of(pfirst, dir_total[0], s);
    uint k = s - pfirst[r];
    uint f = file_of(dir_bases, n_files, r);
    Tape t = tape_at(data, files, f);
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
inline void pd_items(device const PpFile* files, device const uchar* data, device const uint* bases, device const ulong* keys, device const uint* dir_bases, device const uint* dir_total, device PpDef* defs, device const uint* ifirst, device const uint* itotal, device const ulong* pkeys, device uint* items, device atomic_uint* room, uint n_files, uint cap, uint s) {
    if (s >= itotal[0]) return;
    if (s >= cap) {
        atomic_store_explicit(room, 1u, memory_order_relaxed);
        return;
    }
    uint r = pb_item_of(ifirst, dir_total[0], s);
    uint k = s - ifirst[r];
    uint f = file_of(dir_bases, n_files, r);
    Tape t = tape_at(data, files, f);
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
inline void pe_owner_in(device const PpFile* files, device const uchar* data, device const uint* bases, device const uint* total, device uint* out, uint n_files, uint g) {
    if (g >= total[0]) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files, f);
    out[g] = node_size(t, g - bases[f]) > 1u ? g : 0u;
}

// Whether each node is a text token: a leaf whose owner is a `code` node.
inline void pe_text_flag(device const PpFile* files, device const uchar* data, device const uint* bases, device const uint* total, device const uint* owner, device uint* flag, uint n_files, uint g) {
    if (g >= total[0]) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files, f);
    uint i = g - bases[f];
    flag[g] = node_size(t, i) == 1u && node_kind(t, owner[g] - bases[f]) == K_CODE ? 1u : 0u;
}

// Each text token's node at its rank.
inline void pe_text_nodes(device const uint* flag, device const uint* rank, device const uint* total, device uint* text_nodes, uint g) {
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

// A row's definition where it is an `#undef`, and a lookup's answers that are no row: no name,
// no definition, a built-in the device refuses, one it expands to one token (`__FILE__`,
// `__LINE__`, `__DATE__`, `__TIME__`, `__TIMESTAMP__`), or one that is a plain name in text
// (`__has_include`, `__has_include_next`, `__has_embed`: operators of `#if` alone). A built-in's
// row holds its answer; a definition's row is below `L_DEFS`.
constant uint V_UNDEF = 0xffffffffu, L_NOT_NAME = 0xffffffffu, L_NONE = 0xfffffffeu, L_BUILTIN = 0xfffffffdu;
constant uint L_FILE = 0xfffffffcu, L_LINE = 0xfffffffbu, L_DATE = 0xfffffffau, L_TIME = 0xfffffff9u;
constant uint L_TIMESTAMP = 0xfffffff8u, L_IDENT = 0xfffffff7u, L_DEFS = 0xfffffff7u;
// A built-in that expands to one token, a use of it an object-like call.
inline bool px_one_token(uint m) { return m >= L_TIMESTAMP && m <= L_FILE; }
constant uint WALK = 64u;

// Each instance's directive `j`'s row, where it makes one (a flag, then its place by a scan).
inline void pe_row_flag(device const PpInst* insts, device const uint* n_insts, device const uint* dactive, device const uint* n_dirs, device const uint* dir_bases, device const PpDir* rows, device const PpDef* defs, device const ulong* dir_keys, device uint* flag, uint j) {
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
inline void pe_rows(device const PpInst* insts, device const uint* n_insts, device const uint* dslot, device const uint* n_dirs, device const uint* dir_bases, device const PpDir* rows, device const ulong* dir_keys, device const uint* flag, device const uint* at, device ulong* sort_keys, device ulong* name_keys, device uint* values, uint n_builtin, uint j) {
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
inline void pe_rows_total(device const uint* scanned, device uint* n_rows, uint n_builtin, uint i) {
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
            return v == V_UNDEF ? L_NONE : v;
        }
    }
    atomic_fetch_or_explicit(bad, 1u << 29, memory_order_relaxed);
    return L_NONE;
}

// --- Setup: the stretches (S8) and their names' meanings (R4) ------------------------------------

// Each stretch's sort key: its slot. The unit's order is the slots'.
inline void pe_gap_keys(device const uint* gslot, device const uint* n_gaps, device ulong* gkeys, uint j) {
    if (j < n_gaps[0]) gkeys[j] = ulong(gslot[j]);
}

// The stretch at place `p` in the unit's order (`order`, the sort's): its tokens (0 where it is
// not active), from the directive before it to the one after it, by the rows' text counts (the
// file's text after its last directive), its first text token and its slot.
inline void pe_gap_counts(device const PpInst* insts, device const uint* n_insts, device const uint* gactive, device const uint* n_gaps, device const uint* dir_bases, device const uint* dir_total, device const PpDir* rows, device const uint* bases, device const uint* total, device const uint* text_rank, device const uint* text_total, device uint* counts, device uint* firsts, device const uint* order, device const ulong* gkeys, device uint* sslot, uint n_files, uint p) {
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
inline void pe_lookups(device const PpFile* files, device const uchar* data, device const uint* bases, device const ulong* keys, device const uint* text_nodes, device const uint* gap_at, device const uint* n_gaps, device const uint* firsts, device const uint* gslot, device const uint* n_toks, device const ulong* sorted, device const uint* perm, device const uint* n_rows, device const ulong* name_keys, device const uint* values, device uint* out, device atomic_uint* bad, uint n_files, uint s) {
    if (s >= n_toks[0]) return;
    uint j = pb_item_of(gap_at, n_gaps[0], s);
    uint g = text_nodes[firsts[j] + (s - gap_at[j])];
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files, f);
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
constant uint X_HASH = 128u, X_ACROSS = 256u, X_VA_OPT = 512u, X_RELEX = 1024u, X_SPLICE = 2048u, X_BARRIER = 4096u;
// A token's flags: painted (never a candidate again); space before it (or a line's start).
constant uint T_NOEXP = 1u, T_SPACE = 2u;
// Before the pastes: a body's `##` (an operator), and a token a paste consumed.
constant uint T_PASTE = 4u, T_DROP = 8u;
// A name its list's scan found disabled (R-paint): stored as `T_NOEXP` only where the token is
// copied out expanded (an argument's flatten), never where it is read raw.
constant uint T_PAINT = 16u;
// No position.
constant uint NO_POS = 0xffffffffu;
// A placemarker (an empty argument beside `##`), before the pastes.
constant uint PLACEMARKER = 0xffffffffu;
// A ref to a scratch token (made by `#`, `##`), not a tape node.
constant uint SCRATCH = 0x80000000u;
struct PxScratch { uint start, end, kind; };
// A list's kind.
constant uint LK_MACRO = 0u, LK_ARG = 1u, LK_JOIN = 2u;
// The frames an exit reads (its own list's rest and each frame's after it); a chain of exits.
constant uint MAX_SEGS = 16u, MAX_EXITS = 1u << 20;
// The close table's keys: a depth (biased) above a top token's position.
constant uint DEPTH_BIAS = 1u << 15, POS_BITS = 24u;
// A chain's lists at most, and an idle worker's polls.
constant uint MAX_CHAIN = 1024u;
constant uint POLLS = 4096u;

// A list: its parent (a list, or `TOP`: the frame that holds its call's `)`), its definition's
// row (`NO_DEF`: an argument), its stretch's slot, its tokens in the arena, the range of its
// parent it covers (`lo..=hi`) or, an argument, the raw range it was read from; its children not
// yet final, its size and its base. A call: the list holding its raw tokens (`src`: its parent, or
// an exit's join), its name and `)` there (`at`, `end`), its arguments' entries and the argument
// lists not yet flattened. Its exit (`exit`, the list + 1 of a call that reads past its end, set
// when it is final) and its own exit's name and call (`xname`, `xcall`, + 1). An argument: its
// call's list, and its flatten's PLACE items not done. `dest`: where its PLACE writes, the output
// (0) or the arena for argument list `dest - 1`.
struct PxList {
    uint parent, def, slot, first, len, lo, hi, src;
    atomic_uint pending;
    uint at, end, exit, xname, xcall;
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
    l.parent = parent; l.def = def; l.slot = slot; l.first = 0u; l.len = 0u; l.lo = 0u; l.hi = 0u; l.src = parent;
    atomic_store_explicit(&l.pending, 1u, memory_order_relaxed);
    l.at = 0u; l.end = 0u; l.exit = 0u; l.xname = 0u; l.xcall = 0u;
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
    a.t = tape_at(data, files, f);
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
    device const uchar* sbytes; device const PxScratch* stoks; uint relex_options;
    device const ulong* closes; uint n_close;
};

// Whether `p[0..n)` holds a line splice (`\`, blanks, a newline): its phase-2 spelling differs.
inline bool has_splice(device const uchar* p, uint n) {
    for (uint b = 0u; b < n; b++) {
        if (p[b] != uchar('\\')) continue;
        uint k = b + 1u;
        while (k < n && (p[k] == uchar(' ') || p[k] == uchar('\t') || p[k] == 0x0bu || p[k] == 0x0cu)) k++;
        if (k < n && (p[k] == uchar('\n') || p[k] == uchar('\r'))) return true;
    }
    return false;
}

// A token's bytes and kind: a tape node's in its file's source, a scratch token's in the scratch
// bytes.
struct Span { device const uchar* p; uint len; uint kind; };
inline Span ref_span(Ux u, uint g) {
    Span r;
    if ((g & SCRATCH) != 0u) {
        PxScratch t = u.stoks[g & ~SCRATCH];
        r.p = u.sbytes + t.start; r.len = t.end - t.start; r.kind = t.kind;
        return r;
    }
    At a = node_at(u.files, u.data, u.bases, u.n_files, g);
    uint st = node_start(a.t, a.i);
    r.p = u.data + pf_src(a.f) + st; r.len = node_end(a.t, a.i) - st; r.kind = node_kind(a.t, a.i);
    return r;
}

// A name's key: a tape node's, or a scratch token's bytes' (a paste's).
inline ulong ref_key(Ux u, uint g) {
    if ((g & SCRATCH) == 0u) return u.keys[g];
    PxScratch t = u.stoks[g & ~SCRATCH];
    return name_key((device const ulong*)u.sbytes, t.start, t.end - t.start);
}

// Whether a text token of the tape has space before it: a line's first (its `code` node just
// before it), or a gap of more than line splices after the token before it.
inline bool top_space(Ux u, uint g) {
    At a = node_at(u.files, u.data, u.bases, u.n_files, g);
    if (a.i == 0u) return false;
    if (node_kind(a.t, a.i - 1u) == K_CODE) return true;
    return gap_is_space(u.data, a.f, node_end(a.t, a.i - 1u), node_start(a.t, a.i));
}

inline bool ref_is(Ux u, uint g, uchar c) {
    Span r = ref_span(u, g);
    return r.kind == K_PUNCT && r.len == 1u && r.p[0] == c;
}
inline uint ref_kind(Ux u, uint g) { return ref_span(u, g).kind; }
// A paren's depth change.
inline uint paren_delta(Ux u, uint g) {
    Span r = ref_span(u, g);
    if (r.kind != K_PUNCT || r.len != 1u) return 0u;
    return r.p[0] == uchar('(') ? 1u : r.p[0] == uchar(')') ? 0xffffffffu : 0u;
}

// --- The top level: depth, the close table, the calls --------------------------------------------

// Each top token's stretch head (for the depth's segmented scan), its depth change, and whether it
// is a `)` (for the close table).
inline void px_top_parens(device const PpFile* files, device const uchar* data, device const uint* bases, device const uint* text_nodes, device const uint* gat, device const uint* n_gaps, device const uint* firsts, device const uint* n_toks, device uint* heads, device uint* delta, device uint* close, uint n_files, uint s) {
    if (s >= n_toks[0]) return;
    Ux u; u.files = files; u.data = data; u.bases = bases; u.n_files = n_files;
    uint j = pb_item_of(gat, n_gaps[0], s);
    uint d = paren_delta(u, text_nodes[firsts[j] + (s - gat[j])]);
    heads[s] = s == gat[j] ? 1u : 0u;
    delta[s] = d;
    close[s] = d == 0xffffffffu ? 1u : 0u;
}

// Each `)`'s key at its place: its depth after it (biased), its position below.
inline void px_close_keys(device const uint* depth, device const uint* close, device const uint* at, device const uint* n_toks, device ulong* keys, device atomic_uint* ctl, uint s) {
    if (s >= n_toks[0] || close[s] == 0u) return;
    uint b = depth[s] + DEPTH_BIAS;
    if (b >= 2u * DEPTH_BIAS || s >= (1u << POS_BITS)) { px_refuse(ctl, X_DEPTH); return; }
    keys[at[s]] = (ulong(b) << POS_BITS) | ulong(s);
}

// The `)` that closes the `(` at top token `open`: the first after it back at the depth before
// it, in its stretch (a search of the sorted close table); `0xffffffff` where none is.
// The first top `)` at or after `from`, before `end`, whose depth after it is `target`: the first
// time the depth comes back to it.
inline uint top_close_to(device const ulong* closes, uint n_close, uint target, uint from, uint end) {
    uint b = (target + DEPTH_BIAS) & (2u * DEPTH_BIAS - 1u);
    ulong q = (ulong(b) << POS_BITS) | ulong(from);
    uint c = pb_lower_bound(closes, n_close, q);
    if (c >= n_close || uint(closes[c] >> POS_BITS) != b) return NO_POS;
    uint pos = uint(closes[c] & ((1ul << POS_BITS) - 1ul));
    return pos < end ? pos : NO_POS;
}
inline uint top_close(device const ulong* closes, uint n_close, device const uint* depth, uint open, uint end) {
    return top_close_to(closes, n_close, depth[open] - 1u, open + 1u, end);
}

// Each top token's call: an object-like name, or a function-like one followed in its stretch by
// `(`, to its `)`. `cend` is the call's last token + 1 (0: no call), the max scan's input.
inline void px_top_calls(device const PpFile* files, device const uchar* data, device const uint* bases, device const PpDef* defs, device const uint* lookups, device const uint* text_nodes, device const uint* gat, device const uint* n_gaps, device const uint* firsts, device const uint* n_toks, device const uint* depth, device const ulong* closes, device const uint* n_close, device uint* cend, device atomic_uint* ctl, uint n_files, uint s) {
    uint n = n_toks[0];
    if (s >= n) return;
    uint m = lookups[s];
    uint e = 0u;
    if (m == L_BUILTIN) {
        px_refuse(ctl, X_BUILTIN);
    } else if (px_one_token(m)) {
        e = s + 1u;
    } else if (m < L_DEFS) {
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
inline void px_top_outer(device const uint* cend, device const uint* reach, device const uint* n_toks, device uint* state, device uint* oflag, uint s) {
    if (s >= n_toks[0]) return;
    uint before = s == 0u ? 0u : reach[s - 1u];
    uint st = before > s ? 2u : cend[s] != 0u ? 1u : 0u;
    state[s] = st;
    oflag[s] = st == 1u ? 1u : 0u;
}

// Each outermost top call's list (its index the scan's) and its item: CALL for a function-like
// macro, SUBST for an object-like one.
inline void px_top_lists(device const uint* lookups, device const uint* cend, device const uint* state, device const uint* oid, device const uint* n_toks, device const uint* gslot, device const uint* gat, device const uint* n_gaps, device PxList* lists, device atomic_uint* slots, device atomic_uint* ctl, device const PpDef* defs, uint list_cap, uint s) {
    if (s >= n_toks[0] || state[s] != 1u) return;
    uint c = oid[s];
    if (c >= list_cap) { px_refuse(ctl, X_ROOM); return; }
    uint m = lookups[s];
    device PxList& l = lists[c];
    px_init(l, TOP, m, gslot[pb_item_of(gat, n_gaps[0], s)], LK_MACRO);
    l.lo = s; l.hi = cend[s] - 1u; l.at = l.lo; l.end = l.hi;
    uint item = m < L_DEFS && (defs[m].flags & D_FNLIKE) != 0u ? (c | IT_CALL) : (c | IT_SUBST);
    atomic_store_explicit(&slots[c], item + 1u, memory_order_relaxed);
}

// The queue holds the top calls' items: head 0, tail and active their number; the arena's lists
// start after them.
inline void px_seed(device const uint* n_calls, device atomic_uint* qctl, device atomic_uint* ctl, uint list_cap, uint i) {
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
    device PxList* lists; device uint* tok; device uint* tflag; device uint* tchild; device uint* tdepth; device uint* texp;
    device PxArg* args; device uint* out; device atomic_uint* ctl; WorkQueue q;
    uint tok_cap, list_cap, arg_cap, out_cap;
    device uchar* sbytes; device PxScratch* stoks; uint sb_cap, st_cap;
};

inline void px_push(Ar a, uint item) { if (!wq_push(a.q, item)) px_refuse(a.ctl, X_QUEUE); }

// Token `k` of list `p` (or top token `k`, `p` being `TOP`): its ref, depth after it, and flags.
inline uint par_ref(Ux u, Ar a, uint p, uint k) { return (p & TOP) != 0u ? top_ref(u.text_nodes, u.gat, u.n_gaps, u.firsts, k) : a.tok[a.lists[p].first + k]; }
inline uint par_depth(Ux u, Ar a, uint p, uint k) { return (p & TOP) != 0u ? u.tdepth[k] : a.tdepth[a.lists[p].first + k]; }
// The list whose substitution wrote token `k` of `p` (+ 1; 0 a token read from a file): the
// serial pass's `exp`, which `__LINE__` climbs.
inline uint par_exp(Ar a, uint p, uint k) { return (p & TOP) != 0u ? 0u : a.texp[a.lists[p].first + k]; }
inline uint par_flags(Ux u, Ar a, uint p, uint k) {
    if ((p & TOP) != 0u) return top_space(u, top_ref(u.text_nodes, u.gat, u.n_gaps, u.firsts, k)) ? T_SPACE : 0u;
    return a.tflag[a.lists[p].first + k];
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

// The max of `x` over the lanes at or before this one.
inline uint px_prefix_max(uint x, uint lane) {
    for (uint k = 1u; k < 32u; k <<= 1u) {
        uint y = simd_shuffle_up(x, k);
        if (lane >= k) x = max(x, y);
    }
    return x;
}

// Child `c` of list `k` (or `TOP`) with its exits' chain in `k`: the last position of `k` the
// chain covers, its sizes, and the exit that goes on past `k`'s end (+ 1; 0 none).
struct Claim { uint end, size, through; };
inline Claim px_claim(Ar a, uint k, uint c) {
    Claim r;
    r.end = a.lists[c].hi; r.size = a.lists[c].size; r.through = 0u;
    uint x = a.lists[c].exit;
    for (uint step = 0u; step < MAX_EXITS && x != 0u; step++) {
        device PxList& N = a.lists[x - 1u];
        if (N.parent != k) { r.through = x; return r; }
        r.end = N.hi; r.size += N.size;
        x = N.exit;
    }
    return r;
}

// The pieces of list `k` once its children are final (rollback, R-done): in order, a child is
// live unless an earlier live child's exits cover it (their claims: a chain of calls in `k` after
// it); a live exit that goes on past `k`'s end (a child's, or `k`'s own call at `xname`) ends `k`,
// and is `k`'s exit. A token is a piece where no live child or claim covers it. The exit-bearing
// children of 32 are decided one after another (a ballot), the rest by a carried prefix max of
// the live claims' ends. With `place`, the pieces are written at `k`'s base: its tokens (to the
// output, or flattening an argument with their paint stored), each live child's and its chain's
// base, their PLACE pushed.
struct Walked { uint size, exit; };
inline Walked px_walk(Ar a, uint k, uint lane, bool place) {
    device PxList& K = a.lists[k];
    uint n = K.len, first = K.first, xj = K.xname == 0u ? NO_POS : K.xname - 1u;
    uint dest = K.dest, at = K.base;
    uint cover = 0u, stop = NO_POS, exit = 0u, reach = 0u, total = 0u;
    for (uint c0 = 0u; c0 < n && c0 < stop; c0 += 32u) {
        uint j = c0 + lane, child = 0u, ends = 0u;
        bool on = j < n, xb = false, own = false;
        Claim r;
        r.end = 0u; r.size = 0u; r.through = 0u;
        if (on) child = a.tchild[first + j];
        if (child != 0u) {
            r = px_claim(a, k, child - 1u);
            ends = a.lists[child - 1u].hi + 1u;
            xb = a.lists[child - 1u].exit != 0u;
        } else if (on && j == xj) {
            xb = true; own = true; r.end = j; r.through = K.xcall;
        }
        uint cover0 = cover;
        bool live_x = false;
        ulong todo = px_ballot(xb);
        for (uint guard = 0u; guard < 32u && todo != 0u; guard++) {
            uint who = uint(ctz(todo));
            todo &= todo - 1u;
            uint jw = c0 + who, ew = simd_shuffle(r.end, ushort(who)), tw = simd_shuffle(r.through, ushort(who));
            bool ow = simd_shuffle(own ? 1u : 0u, ushort(who)) != 0u;
            bool lv = jw >= cover && jw < stop;
            if (lv) {
                cover = max(cover, ew + 1u);
                if (tw != 0u) { stop = ow ? jw : ew + 1u; exit = tw; }
            }
            if (lane == who) live_x = lv;
        }
        uint claims = px_prefix_max(live_x ? r.end + 1u : 0u, lane);
        uint cb = max(cover0, lane == 0u ? 0u : simd_shuffle_up(claims, 1u));
        uint incl = px_prefix_max(ends, lane);
        uint before = max(max(reach, cb), lane == 0u ? 0u : simd_shuffle_up(incl, 1u));
        bool alive = on && j < stop && j >= before && !own;
        uint v = alive ? (child != 0u ? r.size : 1u) : 0u;
        uint off = at + simd_prefix_exclusive_sum(v);
        if (place) {
            if (alive && child == 0u) {
                if (dest == 0u) {
                    if (off < a.out_cap) a.out[off] = a.tok[first + j]; else px_refuse(a.ctl, X_ROOM);
                } else {
                    uint f = a.tflag[first + j];
                    a.tok[off] = a.tok[first + j];
                    a.tflag[off] = (f & T_PAINT) != 0u ? (f | T_NOEXP) & ~T_PAINT : f;
                    a.tchild[off] = 0u;
                    a.texp[off] = a.texp[first + j];
                }
            }
            // The live children and their chains, each placed after the one before.
            uint pushes = 0u;
            if (alive && child != 0u) {
                pushes = 1u;
                uint x = a.lists[child - 1u].exit;
                for (uint step = 0u; step < MAX_EXITS && x != 0u && a.lists[x - 1u].parent == k; step++) { pushes++; x = a.lists[x - 1u].exit; }
            }
            uint kp = simd_sum(pushes);
            if (dest != 0u && lane == 0u && kp != 0u) atomic_fetch_add_explicit(&a.lists[dest - 1u].fpending, kp, memory_order_relaxed);
            if (pushes != 0u) {
                uint c = child - 1u, b = off;
                for (uint step = 0u; step < pushes; step++) {
                    a.lists[c].base = b;
                    a.lists[c].dest = dest;
                    b += a.lists[c].size;
                    px_fence();
                    px_push(a, c | IT_PLACE);
                    c = a.lists[c].exit - 1u;
                }
            }
        }
        uint sum = simd_sum(v);
        at += sum; total += sum;
        reach = max(reach, simd_max(incl));
        cover = max(cover, simd_max(claims));
    }
    Walked w;
    w.size = total; w.exit = exit;
    return w;
}

// List `l` is final where its last child is (all 32 lanes): its pieces walked, its size and exit
// set. A macro's list then counts down its parent's children, climbing where it was the last; an
// argument's starts its flatten: room for its tokens, its PLACE pushed.
inline void px_final(Ar a, uint l, uint lane) {
    uint k = l;
    for (uint step = 0u; step < MAX_CHAIN; step++) {
        px_fence();
        simdgroup_barrier(mem_flags::mem_device);
        Walked w = px_walk(a, k, lane, false);
        device PxList& K = a.lists[k];
        if (lane == 0u) { K.size = w.size; K.exit = w.exit; }
        px_fence();
        simdgroup_barrier(mem_flags::mem_device);
        if (K.kind == LK_ARG) {
            if (lane == 0u) {
                uint base = atomic_fetch_add_explicit(&a.ctl[0], w.size, memory_order_relaxed);
                if (base + w.size > a.tok_cap) { px_refuse(a.ctl, X_ROOM); return; }
                K.base = base;
                K.dest = k + 1u;
                atomic_store_explicit(&K.fpending, 1u, memory_order_relaxed);
                px_fence();
                px_push(a, k | IT_PLACE);
            }
            return;
        }
        uint p = K.parent;
        if ((p & TOP) != 0u) return;
        uint left = 0u;
        if (lane == 0u) left = atomic_fetch_sub_explicit(&a.lists[p].pending, 1u, memory_order_relaxed);
        if (simd_broadcast(left, 0) != 1u) return;
        k = p;
    }
}

// A list's scan or its wrong call is done: its own count down, final where it was the last.
inline void px_done(Ar a, uint l, uint lane) {
    px_fence();
    simdgroup_barrier(mem_flags::mem_device);
    uint left = 0u;
    if (lane == 0u) left = atomic_fetch_sub_explicit(&a.lists[l].pending, 1u, memory_order_relaxed);
    if (simd_broadcast(left, 0) == 1u) px_final(a, l, lane);
}

// The exit of list `l`'s call at `j` (macro `m`), whose `(` or `)` is past `l`'s end (R-peek,
// R-ctx), by the 32 lanes: the frames after it climbed (each one's rest, never past an argument's
// end nor a stretch's), its `)` found where the depth first comes back (a ballot search in a list,
// a search of the top level's close table), its raw tokens joined into a source list (with their
// depths, a carried SIMD scan), and its call list made in the frame holding `)`, counted there,
// its CALL pushed.
inline void px_exit(Ux u, Ar a, uint l, uint j, uint m, uint lane) {
    device PxList& L = a.lists[l];
    uint segl[MAX_SEGS], seglo[MAX_SEGS], seghi[MAX_SEGS];
    uint ns = 1u;
    segl[0] = l; seglo[0] = j; seghi[0] = L.len - 1u;
    uint r = a.tdepth[L.first + L.len - 1u] - a.tdepth[L.first + j];
    uint k = l, x = NO_POS, e = NO_POS;
    for (uint step = 0u; step + 1u < MAX_SEGS; step++) {
        device PxList& K = a.lists[k];
        if (K.kind != LK_MACRO) break;
        uint p = K.parent, s0 = K.hi + 1u;
        if ((p & TOP) != 0u) {
            uint end = top_end(u.gat, u.n_gaps, u.n_toks, K.lo);
            uint c = top_close_to(u.closes, u.n_close, u.tdepth[K.hi] - r, s0, end);
            if (c != NO_POS) { segl[ns] = TOP; seglo[ns] = s0; seghi[ns] = c; ns++; x = TOP; e = c; }
            break;
        }
        device PxList& P = a.lists[p];
        uint base = a.tdepth[P.first + K.hi], target = base - r, found = NO_POS;
        for (uint c0 = s0; c0 < P.len && found == NO_POS; c0 += 32u) {
            uint q = c0 + lane;
            ulong h = px_ballot(q < P.len && a.tdepth[P.first + q] == target && ref_is(u, a.tok[P.first + q], ')'));
            if (h != 0u) found = c0 + uint(ctz(h));
        }
        if (found != NO_POS) { segl[ns] = p; seglo[ns] = s0; seghi[ns] = found; ns++; x = p; e = found; break; }
        if (s0 < P.len) {
            segl[ns] = p; seglo[ns] = s0; seghi[ns] = P.len - 1u; ns++;
            r += a.tdepth[P.first + P.len - 1u] - base;
        }
        k = p;
    }
    // No `)` before an argument's end (the serial pass's error), or past the frames' bound.
    if (x == NO_POS) {
        if (lane == 0u) px_refuse(a.ctl, X_PAST_END);
        return;
    }
    uint total = 0u;
    for (uint t = 0u; t < ns; t++) total += seghi[t] - seglo[t] + 1u;
    uint jid = 0u, base = 0u;
    if (lane == 0u) {
        jid = atomic_fetch_add_explicit(&a.ctl[1], 2u, memory_order_relaxed);
        base = atomic_fetch_add_explicit(&a.ctl[0], total, memory_order_relaxed);
    }
    jid = simd_broadcast(jid, 0);
    base = simd_broadcast(base, 0);
    uint nid = jid + 1u;
    if (nid >= a.list_cap || base + total > a.tok_cap) {
        if (lane == 0u) px_refuse(a.ctl, X_ROOM);
        return;
    }
    uint w = base;
    for (uint t = 0u; t < ns; t++) {
        uint len = seghi[t] - seglo[t] + 1u;
        for (uint q = lane; q < len; q += 32u) {
            a.tok[w + q] = par_ref(u, a, segl[t], seglo[t] + q);
            a.tflag[w + q] = par_flags(u, a, segl[t], seglo[t] + q) & ~(T_PAINT | T_PASTE | T_DROP);
            a.tchild[w + q] = 0u;
            a.texp[w + q] = par_exp(a, segl[t], seglo[t] + q);
        }
        w += len;
    }
    px_fence();
    simdgroup_barrier(mem_flags::mem_device);
    uint carry = 0u;
    for (uint c0 = 0u; c0 < total; c0 += 32u) {
        uint q = c0 + lane;
        uint d = q < total ? paren_delta(u, a.tok[base + q]) : 0u;
        uint depth = carry + simd_prefix_inclusive_sum(d);
        if (q < total) a.tdepth[base + q] = depth;
        carry += simd_sum(d);
    }
    px_fence();
    simdgroup_barrier(mem_flags::mem_device);
    if (lane == 0u) {
        device PxList& J = a.lists[jid];
        px_init(J, x, NO_DEF, L.slot, LK_JOIN);
        J.first = base; J.len = total;
        device PxList& N = a.lists[nid];
        px_init(N, x, m, L.slot, LK_MACRO);
        N.lo = seglo[ns - 1u]; N.hi = e; N.src = jid; N.at = 0u; N.end = total - 1u;
        if ((x & TOP) == 0u) atomic_fetch_add_explicit(&a.lists[x].pending, 1u, memory_order_relaxed);
        L.xname = j + 1u; L.xcall = nid + 1u;
        px_fence();
        px_push(a, nid | IT_CALL);
    }
}

// SCAN `l`'s tokens (written): their depths (a SIMD scan carried 32 at a time), then their
// calls: a name not painted whose macro is not disabled on the chain (else `T_PAINT`), a
// function-like one followed by `(` in the list, to its `)` (a SIMD search a call); the outermost
// (a carried max of the calls' reach) each a child list, pushed. A call whose `(` or `)` is past
// the list's end reaches past every later token: where it is outermost, it is the list's exit.
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
    uint reach = 0u, xj = NO_POS, xm = 0u;
    for (uint c0 = 0u; c0 < n && xj == NO_POS; c0 += 32u) {
        uint j = c0 + lane, m = 0u, e = 0u;
        bool call = false, fn = false, open = false, past = false;
        if (j < n && (a.tflag[first + j] & T_NOEXP) == 0u) {
            uint g = a.tok[first + j];
            if (ref_kind(u, g) == K_IDENTIFIER) {
                m = visible(u.sorted, u.perm, u.n_rows, u.name_keys, u.values, ref_key(u, g), L.slot, a.ctl + 2);
                if (m == L_BUILTIN) {
                    px_refuse(a.ctl, X_BUILTIN);
                } else if (px_one_token(m)) {
                    call = true; e = j;
                } else if (m < L_DEFS) {
                    if (px_disabled(a.lists, l, m, a.ctl)) {
                        a.tflag[first + j] |= T_PAINT;
                    } else if ((u.defs[m].flags & D_FNLIKE) == 0u) {
                        call = true; e = j;
                    } else if (j + 1u < n) {
                        open = ref_is(u, a.tok[first + j + 1u], '(');
                        fn = true;
                    } else if (px_peek_paren(u, a.lists, a.tok, l)) {
                        call = true; e = n; past = true;
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
                call = true;
                if (found == 0xffffffffu) { e = n; past = true; }
                else e = found;
            }
        }
        uint incl = px_prefix_max(call ? e + 1u : 0u, lane);
        uint before = max(reach, lane == 0u ? 0u : simd_shuffle_up(incl, 1u));
        reach = max(reach, simd_max(incl));
        call = call && before <= j;
        // The outermost call past the end: the list's exit (made after the scan).
        ulong xb = px_ballot(call && past);
        if (xb != 0u) {
            uint who = uint(ctz(xb));
            xj = c0 + who;
            xm = simd_shuffle(m, ushort(who));
        }
        call = call && !past;
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
            C.at = j; C.end = e;
            a.tchild[first + j] = child + 1u;
        }
        px_fence();
        if (call) px_push(a, child | (m < L_DEFS && (u.defs[m].flags & D_FNLIKE) != 0u ? IT_CALL : IT_SUBST));
    }
    if (xj != NO_POS) px_exit(u, a, l, xj, xm, lane);
    px_done(a, l, lane);
}

// Copies `n` tokens (refs and flags) from `src` to `dst` of the arena, by the 32 lanes.
inline void px_copy(Ar a, uint dst, uint src, uint n, uint lane) {
    for (uint j = lane; j < n; j += 32u) {
        a.tok[dst + j] = a.tok[src + j];
        a.tflag[dst + j] = a.tflag[src + j];
        a.tchild[dst + j] = 0u;
        a.texp[dst + j] = a.texp[src + j];
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
    // Its raw tokens: in its parent, or an exit's join.
    uint p = L.src, open = L.at + 1u, close = L.end;
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
        // The name alone, painted: an exit's tokens after it would be lost (the CPU's for now).
        if ((p & TOP) == 0u && a.lists[p].kind == LK_JOIN) {
            if (lane == 0u) px_refuse(a.ctl, X_PAST_END);
        }
        uint base = 0u;
        if (lane == 0u) base = atomic_fetch_add_explicit(&a.ctl[0], 1u, memory_order_relaxed);
        base = simd_broadcast(base, 0);
        if (lane == 0u) {
            if (base + 1u > a.tok_cap) {
                px_refuse(a.ctl, X_ROOM);
                L.len = 0u;
            } else {
                a.tok[base] = par_ref(u, a, p, L.at);
                a.tflag[base] = (par_flags(u, a, p, L.at) & ~T_PAINT) | T_NOEXP;
                a.tchild[base] = 0u;
                a.texp[base] = par_exp(a, p, L.at);
                L.len = 1u;
            }
            L.first = base;
        }
        px_done(a, l, lane);
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
    // The body's uses of parameters: one after `#` or beside `##` is read raw; each parameter used
    // otherwise, with an argument, gets its list once.
    uint n = d.n_body;
    for (uint c0 = 0u; c0 < n; c0 += 32u) {
        uint i = c0 + lane;
        uint q = 0xffffu;
        if (i < n) {
            uint it = u.items[d.ifirst + i];
            uint kind = ref_kind(u, d.body + i);
            bool vo = va && kind == K_IDENTIFIER && u.keys[d.body + i] == u.va_opt;
            if (vo && i > 0u && ref_kind(u, d.body + i - 1u) == K_HASH) px_refuse(a.ctl, X_VA_OPT);
            q = it & 0xffffu;
            if (i > 0u && ref_kind(u, d.body + i - 1u) == K_HASH) q = NO_PARAM;
            if ((i > 0u && (u.items[d.ifirst + i - 1u] & I_PASTE) != 0u) || (i + 1u < n && (u.items[d.ifirst + i + 1u] & I_PASTE) != 0u)) q = NO_PARAM;
            // `__VA_OPT__` asks whether the variadic argument, expanded, has tokens.
            if (vo && np > 0u) q = np - 1u;
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
                a.tflag[base + j] = par_flags(u, a, p, lo + j) & ~T_PAINT;
                a.tchild[base + j] = 0u;
                a.texp[base + j] = par_exp(a, p, lo + j);
            }
            if (lane == 0u) {
                // Its context the call's (R-args): the frame holding `)`.
                device PxList& A = a.lists[id];
                px_init(A, L.parent, NO_DEF, L.slot, LK_ARG);
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

// What item `i` of definition `d` writes in call `L`'s substitution (`substitute`'s cases):
constant uint R_LIT = 0u, R_HASH = 1u, R_GONE = 2u, R_RAW = 3u, R_EXP = 4u, R_PASTE = 5u;
struct Role { uint role, count, q; };

// A body's `__VA_OPT__`s (4 at most): each one's item, its `(` and its `)` (its content between),
// and whether the variadic argument, expanded, has tokens.
constant uint MAX_VA_OPT = 4u;
struct Vo { uint n; uint s[MAX_VA_OPT], o[MAX_VA_OPT], c[MAX_VA_OPT]; bool present; };

inline bool px_arg_empty(Ar a, device PxList& L, uint q) { return a.args[L.args + q].hi <= a.args[L.args + q].lo; }

// GNU's `, ## __VA_ARGS__` at item `i` (the variadic parameter): a literal comma, `##`, then it.
inline bool px_gnu_comma(Ux u, PpDef d, device PxList& L, uint i) {
    if ((d.flags & D_VARIADIC) == 0u || i < 2u || L.nargs == 0u) return false;
    if ((u.items[d.ifirst + i] & 0xffffu) != L.nargs - 1u) return false;
    if ((u.items[d.ifirst + i - 1u] & I_PASTE) == 0u) return false;
    return (u.items[d.ifirst + i - 2u] & 0xffffu) == NO_PARAM && ref_is(u, d.body + i - 2u, ',');
}

inline Role px_role(Ux u, Ar a, PpDef d, device PxList& L, uint i, Vo vo) {
    uint n = d.n_body, it = u.items[d.ifirst + i], q = it & 0xffffu;
    Role r; r.role = R_LIT; r.count = 1u; r.q = q;
    // `__VA_OPT__`: its content where the variadic argument has tokens, else a placemarker; its
    // name and parentheses write nothing.
    for (uint k = 0u; k < vo.n; k++) {
        if (i == vo.s[k]) { r.role = vo.present ? R_GONE : R_RAW; r.count = vo.present ? 0u : 1u; r.q = NO_PARAM; return r; }
        if (i == vo.o[k] || i == vo.c[k] || (i > vo.o[k] && i < vo.c[k] && !vo.present)) { r.role = R_GONE; r.count = 0u; return r; }
    }
    bool fn = (d.flags & D_FNLIKE) != 0u;
    if ((it & I_PASTE) != 0u) {
        // `##`: an operator, gone in GNU's comma (`, ## __VA_ARGS__`).
        if (i + 1u < n && px_gnu_comma(u, d, L, i + 1u)) { r.role = R_GONE; r.count = 0u; }
        else r.role = R_PASTE;
        return r;
    }
    if (q == NO_PARAM || q >= L.nargs) {
        if (fn && ref_kind(u, d.body + i) == K_HASH && i + 1u < n && (u.items[d.ifirst + i + 1u] & 0xffffu) != NO_PARAM) r.role = R_HASH;
        // GNU's comma goes with no variadic arguments.
        else if (i + 2u < n && px_gnu_comma(u, d, L, i + 2u) && px_arg_empty(a, L, L.nargs - 1u)) { r.role = R_GONE; r.count = 0u; }
        return r;
    }
    if (i > 0u && ref_kind(u, d.body + i - 1u) == K_HASH) { r.role = R_GONE; r.count = 0u; return r; }
    uint len = a.args[L.args + q].hi - a.args[L.args + q].lo;
    if (px_gnu_comma(u, d, L, i)) { r.role = R_RAW; r.count = len; return r; }
    bool beside = (i > 0u && (u.items[d.ifirst + i - 1u] & I_PASTE) != 0u) || (i + 1u < n && (u.items[d.ifirst + i + 1u] & I_PASTE) != 0u);
    if (beside) { r.role = R_RAW; r.count = len == 0u ? 1u : len; return r; }
    uint id = atomic_load_explicit(&a.args[L.args + q].list, memory_order_relaxed);
    r.role = R_EXP;
    r.count = id >= 2u ? a.lists[id - 2u].size : 0u;
    return r;
}

// `#` of the raw tokens `lo..hi` of list `p` (or the top level): a string literal, written to the
// scratch bytes by the 32 lanes (each token's length, a space before it where it has one, a
// literal's `"` and `\` escaped; a SIMD scan of the lengths, one atomic add), as a scratch token.
inline uint px_stringize(Ux u, Ar a, uint p, uint lo, uint hi, uint lane) {
    uint total = 0u;
    for (uint c0 = lo; c0 < hi; c0 += 32u) {
        uint k = c0 + lane, len = 0u;
        if (k < hi) {
            Span r = ref_span(u, par_ref(u, a, p, k));
            // A token with a line splice inside: its phase-2 spelling is the CPU's.
            if (has_splice(r.p, r.len)) px_refuse(a.ctl, X_SPLICE);
            len = r.len + (k > lo && (par_flags(u, a, p, k) & T_SPACE) != 0u ? 1u : 0u);
            if (r.kind == K_LITERAL) for (uint b = 0u; b < r.len; b++) len += (r.p[b] == uchar('"') || r.p[b] == uchar('\\')) ? 1u : 0u;
        }
        total += simd_sum(len);
    }
    uint base = 0u, id = 0u;
    if (lane == 0u) {
        base = atomic_fetch_add_explicit(&a.ctl[4], total + 2u, memory_order_relaxed);
        id = atomic_fetch_add_explicit(&a.ctl[5], 1u, memory_order_relaxed);
    }
    base = simd_broadcast(base, 0);
    id = simd_broadcast(id, 0);
    if (base + total + 2u > a.sb_cap || id >= a.st_cap) {
        if (lane == 0u) px_refuse(a.ctl, X_ROOM);
        return SCRATCH;
    }
    uint at = base + 1u;
    for (uint c0 = lo; c0 < hi; c0 += 32u) {
        uint k = c0 + lane, len = 0u;
        Span r;
        r.len = 0u; r.kind = 0u;
        bool sp = false;
        if (k < hi) {
            r = ref_span(u, par_ref(u, a, p, k));
            sp = k > lo && (par_flags(u, a, p, k) & T_SPACE) != 0u;
            len = r.len + (sp ? 1u : 0u);
            if (r.kind == K_LITERAL) for (uint b = 0u; b < r.len; b++) len += (r.p[b] == uchar('"') || r.p[b] == uchar('\\')) ? 1u : 0u;
        }
        uint w = at + simd_prefix_exclusive_sum(len);
        if (k < hi) {
            if (sp) a.sbytes[w++] = uchar(' ');
            for (uint b = 0u; b < r.len; b++) {
                uchar c = r.p[b];
                if (r.kind == K_LITERAL && (c == uchar('"') || c == uchar('\\'))) a.sbytes[w++] = uchar('\\');
                a.sbytes[w++] = c;
            }
        }
        at += simd_sum(len);
    }
    if (lane == 0u) {
        a.sbytes[base] = uchar('"');
        a.sbytes[base + total + 1u] = uchar('"');
        a.stoks[id].start = base; a.stoks[id].end = base + total + 2u; a.stoks[id].kind = K_LITERAL;
    }
    return SCRATCH | id;
}

// `l ## r` (one lane): a placemarker gives the other (the left's space kept); else the two
// spellings joined in the scratch bytes, and the relexer's verdict: one token (a scratch token of
// its kind, with the left's space), or none (`ok` false: both stay).
inline uint px_paste(Ux u, Ar a, uint l, uint lf, uint r, uint rf, thread uint& out_f, thread bool& ok) {
    ok = true;
    if (r == PLACEMARKER) { out_f = lf; return l; }
    if (l == PLACEMARKER) { out_f = (rf & ~T_SPACE) | (lf & T_SPACE); return r; }
    Span x = ref_span(u, l), y = ref_span(u, r);
    uint n = x.len + y.len;
    uint base = atomic_fetch_add_explicit(&a.ctl[4], n, memory_order_relaxed);
    if (base + n > a.sb_cap) { px_refuse(a.ctl, X_ROOM); ok = false; return l; }
    for (uint b = 0u; b < x.len; b++) a.sbytes[base + b] = x.p[b];
    for (uint b = 0u; b < y.len; b++) a.sbytes[base + x.len + b] = y.p[b];
    // A line splice inside: its phase-2 spelling is the CPU's.
    if (has_splice(a.sbytes + base, n)) { px_refuse(a.ctl, X_SPLICE); ok = false; return l; }
    uint kind = relex(a.sbytes + base, n, u.relex_options);
    if (kind == RELEX_REFUSED) { px_refuse(a.ctl, X_RELEX); ok = false; return l; }
    if (kind == RELEX_NONE) { ok = false; return l; }
    uint id = atomic_fetch_add_explicit(&a.ctl[5], 1u, memory_order_relaxed);
    if (id >= a.st_cap) { px_refuse(a.ctl, X_ROOM); ok = false; return l; }
    a.stoks[id].start = base; a.stoks[id].end = base + n; a.stoks[id].kind = kind;
    out_f = lf & T_SPACE;
    return SCRATCH | id;
}

// The body's `__VA_OPT__`s (`va_opt_content`): each one's `(` (the next item, else none and no
// content) and its `)` (the first back at the depth before, by a carried SIMD depth scan and a
// ballot; none: the body's end), all lanes together.
inline Vo px_va_opts(Ux u, Ar a, PpDef d, device PxList& L, uint lane) {
    Vo vo;
    vo.n = 0u; vo.present = false;
    if ((d.flags & D_VARIADIC) == 0u || L.nargs == 0u) return vo;
    uint n = d.n_body;
    for (uint c0 = 0u; c0 < n; c0 += 32u) {
        uint i = c0 + lane;
        bool is = i < n && ref_kind(u, d.body + i) == K_IDENTIFIER && u.keys[d.body + i] == u.va_opt;
        ulong todo = px_ballot(is);
        for (uint guard = 0u; guard < 32u && todo != 0u; guard++) {
            uint who = uint(ctz(todo));
            todo &= todo - 1u;
            uint s = c0 + who;
            if (vo.n == MAX_VA_OPT) { if (lane == 0u) px_refuse(a.ctl, X_VA_OPT); return vo; }
            uint o = s, c = s;
            if (s + 1u < n && ref_is(u, d.body + s + 1u, '(')) {
                o = s + 1u;
                c = n;
                uint depth = 0u;
                for (uint k0 = o + 1u; k0 < n && c == n; k0 += 32u) {
                    uint k = k0 + lane;
                    uint dl = k < n ? paren_delta(u, d.body + k) : 0u;
                    uint after = depth + simd_prefix_inclusive_sum(dl);
                    ulong h = px_ballot(k < n && dl == 0xffffffffu && after == 0xffffffffu);
                    if (h != 0u) c = k0 + uint(ctz(h));
                    depth += simd_sum(dl);
                }
            }
            vo.s[vo.n] = s; vo.o[vo.n] = o; vo.c[vo.n] = c;
            vo.n++;
        }
    }
    if (vo.n > 0u) {
        uint id = atomic_load_explicit(&a.args[L.args + L.nargs - 1u].list, memory_order_relaxed);
        vo.present = id >= 2u && a.lists[id - 2u].size > 0u;
    }
    return vo;
}

// --- The built-ins ---------------------------------------------------------------------------------

// The physical line of tape node `g` (from 1): by its file's tape's newline index (decision
// 0460 §2).
inline uint px_line(Ux u, uint g) {
    At at = node_at(u.files, u.data, u.bases, u.n_files, g);
    return tape_line_of(at.t, node_start(at.t, at.i)) + 1u;
}

// A built-in's use, list `l` (an object-like call), by the 32 lanes: `__LINE__` the line of the
// outermost call's end (the serial pass's: the token's origin climbed, each expansion's end
// token, to one read from a file), as a number; `__FILE__` its stretch's file's literal, and
// `__DATE__`, `__TIME__` and `__TIMESTAMP__` the configuration's (scratch tokens the host placed:
// one a file, then those three). One token with the name's space; then scanned.
inline void px_builtin(Ux u, Ar a, uint l, uint lane) {
    device PxList& L = a.lists[l];
    uint ref = SCRATCH;
    if (L.def == L_DATE || L.def == L_TIME || L.def == L_TIMESTAMP) {
        ref = SCRATCH | (u.n_files + (L_DATE - L.def));
    } else if (L.def == L_FILE) {
        uint k = l;
        for (uint step = 0u; step < MAX_CHAIN && (a.lists[k].parent & TOP) == 0u; step++) k = a.lists[k].parent;
        ref = SCRATCH | file_of(u.bases, u.n_files, top_ref(u.text_nodes, u.gat, u.n_gaps, u.firsts, a.lists[k].lo));
    } else {
        uint p = L.src, k = L.at, x = par_exp(a, p, k);
        for (uint step = 0u; step < MAX_CHAIN && x != 0u; step++) {
            p = a.lists[x - 1u].src;
            k = a.lists[x - 1u].end;
            x = par_exp(a, p, k);
        }
        uint g = par_ref(u, a, p, k);
        uint line = (g & SCRATCH) == 0u ? px_line(u, g) : 0u;
        uint nd = 1u;
        for (uint v = line; v >= 10u; v /= 10u) nd++;
        uint base = 0u, id = 0u;
        if (lane == 0u) {
            base = atomic_fetch_add_explicit(&a.ctl[4], nd, memory_order_relaxed);
            id = atomic_fetch_add_explicit(&a.ctl[5], 1u, memory_order_relaxed);
            if (base + nd > a.sb_cap || id >= a.st_cap) {
                px_refuse(a.ctl, X_ROOM);
            } else {
                uint v = line;
                for (uint q = 0u; q < nd; q++) { a.sbytes[base + nd - 1u - q] = uchar('0' + v % 10u); v /= 10u; }
                a.stoks[id].start = base; a.stoks[id].end = base + nd; a.stoks[id].kind = K_NUMBER;
            }
        }
        ref = SCRATCH | simd_broadcast(id, 0);
    }
    if (lane == 0u) {
        uint base = atomic_fetch_add_explicit(&a.ctl[0], 1u, memory_order_relaxed);
        if (base + 1u > a.tok_cap) {
            px_refuse(a.ctl, X_ROOM);
            L.len = 0u;
        } else {
            a.tok[base] = ref;
            a.tflag[base] = par_flags(u, a, L.src, L.at) & T_SPACE;
            a.tchild[base] = 0u;
            a.texp[base] = l + 1u;
            L.len = 1u;
        }
        L.first = base;
    }
    px_fence();
    simdgroup_barrier(mem_flags::mem_device);
    px_scan(u, a, l, lane);
}

// SUBST `l`: its body's items (`px_role`), at offsets by a SIMD scan of their counts (one atomic
// add for all): a parameter used expanded its argument's flattened tokens (their paint kept, the
// first taking the parameter's space), one beside `##` its raw tokens (an empty one a
// placemarker), `#` and its parameter one string literal, `##` an operator. Then the pastes: each
// chain `x ## y ## …` folded left to right by the lane at its head (chains in parallel); then the
// list compacted in place (a carried SIMD scan) without the pasted-away tokens and placemarkers;
// the first token takes the call's name's space; then the list is scanned.
inline void px_subst(Ux u, Ar a, uint l, uint lane) {
    if (px_one_token(a.lists[l].def)) {
        px_builtin(u, a, l, lane);
        return;
    }
    device PxList& L = a.lists[l];
    PpDef d = u.defs[L.def];
    uint n = d.n_body, total = 0u;
    Vo vo = px_va_opts(u, a, d, L, lane);
    bool pastes = false;
    for (uint c0 = 0u; c0 < n; c0 += 32u) {
        uint i = c0 + lane, v = 0u;
        if (i < n) {
            Role r = px_role(u, a, d, L, i, vo);
            v = r.count;
            pastes = pastes || r.role == R_PASTE;
        }
        total += simd_sum(v);
    }
    pastes = simd_any(pastes);
    uint base = 0u;
    if (lane == 0u) base = atomic_fetch_add_explicit(&a.ctl[0], total, memory_order_relaxed);
    base = simd_broadcast(base, 0);
    if (base + total > a.tok_cap) {
        if (lane == 0u) px_refuse(a.ctl, X_ROOM);
        total = 0u;
        n = 0u;
    }
    uint at = base;
    uint vo_off[MAX_VA_OPT], vo_sp[MAX_VA_OPT], vo_end[MAX_VA_OPT];
    for (uint k = 0u; k < MAX_VA_OPT; k++) { vo_off[k] = 0u; vo_sp[k] = 0u; vo_end[k] = base + total; }
    for (uint c0 = 0u; c0 < n; c0 += 32u) {
        uint i = c0 + lane, sp = 0u;
        Role r; r.role = R_GONE; r.count = 0u; r.q = NO_PARAM;
        if (i < n) {
            r = px_role(u, a, d, L, i, vo);
            sp = (u.items[d.ifirst + i] & I_SPACE) != 0u ? T_SPACE : 0u;
        }
        uint off = at + simd_prefix_exclusive_sum(r.count);
        // A present `__VA_OPT__`'s content: its first token takes `__VA_OPT__`'s space (where the
        // content wrote any: its `)` is placed past it).
        for (uint k = 0u; k < vo.n; k++) {
            if (!vo.present) break;
            uint so = vo.s[k] - c0, co = vo.c[k] - c0;
            uint s_off = so < 32u ? simd_shuffle(off, ushort(so)) : 0u, s_sp = so < 32u ? simd_shuffle(sp, ushort(so)) : 0u;
            uint c_off = co < 32u ? simd_shuffle(off, ushort(co)) : 0u;
            if (so < 32u) { vo_off[k] = s_off; vo_sp[k] = s_sp; }
            if (co < 32u) vo_end[k] = c_off;
        }
        if (i < n && (r.role == R_LIT || r.role == R_PASTE)) {
            a.tok[off] = d.body + i;
            a.tflag[off] = sp | (r.role == R_PASTE ? T_PASTE : 0u);
            a.tchild[off] = 0u;
            a.texp[off] = l + 1u;
        }
        // An expanded argument: its flattened tokens. Every shuffle with all 32 lanes active: one
        // in a branch reads an idle lane.
        ulong todo = px_ballot(r.role == R_EXP && r.count > 0u);
        for (uint guard = 0u; guard < 32u && todo != 0u; guard++) {
            uint who = uint(ctz(todo));
            todo &= todo - 1u;
            uint dst = simd_shuffle(off, ushort(who)), spw = simd_shuffle(sp, ushort(who));
            uint qw = simd_shuffle(r.q, ushort(who)), cw = simd_shuffle(r.count, ushort(who));
            uint id = atomic_load_explicit(&a.args[L.args + qw].list, memory_order_relaxed);
            px_copy(a, dst, a.lists[id - 2u].base, cw, lane);
            simdgroup_barrier(mem_flags::mem_device);
            if (lane == 0u) a.tflag[dst] = (a.tflag[dst] & ~T_SPACE) | spw;
        }
        // A raw argument: its tokens from the call's parent, or a placemarker.
        todo = px_ballot(r.role == R_RAW && r.count > 0u);
        for (uint guard = 0u; guard < 32u && todo != 0u; guard++) {
            uint who = uint(ctz(todo));
            todo &= todo - 1u;
            uint dst = simd_shuffle(off, ushort(who)), spw = simd_shuffle(sp, ushort(who)), qw = simd_shuffle(r.q, ushort(who));
            // An absent `__VA_OPT__` is a placemarker (no parameter).
            uint lo = qw < L.nargs ? a.args[L.args + qw].lo : 0u, len = qw < L.nargs ? a.args[L.args + qw].hi - lo : 0u;
            for (uint j = lane; j < len; j += 32u) {
                a.tok[dst + j] = par_ref(u, a, L.src, lo + j);
                a.tflag[dst + j] = par_flags(u, a, L.src, lo + j) & ~(T_PASTE | T_DROP | T_PAINT);
                a.tchild[dst + j] = 0u;
                a.texp[dst + j] = par_exp(a, L.src, lo + j);
            }
            simdgroup_barrier(mem_flags::mem_device);
            if (lane == 0u) {
                if (len == 0u) { a.tok[dst] = PLACEMARKER; a.tflag[dst] = 0u; a.tchild[dst] = 0u; a.texp[dst] = 0u; }
                else a.tflag[dst] = (a.tflag[dst] & ~T_SPACE) | spw;
            }
        }
        todo = px_ballot(r.role == R_HASH);
        for (uint guard = 0u; guard < 32u && todo != 0u; guard++) {
            uint who = uint(ctz(todo));
            todo &= todo - 1u;
            uint dst = simd_shuffle(off, ushort(who)), spw = simd_shuffle(sp, ushort(who));
            uint qw = u.items[d.ifirst + simd_shuffle(i, ushort(who)) + 1u] & 0xffffu;
            uint lo = qw < L.nargs ? a.args[L.args + qw].lo : 0u, hi = qw < L.nargs ? a.args[L.args + qw].hi : 0u;
            uint ref = px_stringize(u, a, L.src, lo, hi, lane);
            if (lane == 0u) {
                a.tok[dst] = ref;
                a.tflag[dst] = spw;
                a.tchild[dst] = 0u;
                a.texp[dst] = l + 1u;
            }
        }
        at += simd_sum(r.count);
    }
    px_fence();
    simdgroup_barrier(mem_flags::mem_device);
    if (lane == 0u && vo.present) {
        for (uint k = 0u; k < vo.n; k++) if (vo_end[k] > vo_off[k]) a.tflag[vo_off[k]] = (a.tflag[vo_off[k]] & ~T_SPACE) | vo_sp[k];
    }
    px_fence();
    simdgroup_barrier(mem_flags::mem_device);
    if (pastes) {
        // Each chain's head: a token with `##` after it and none before.
        for (uint c0 = 0u; c0 < total; c0 += 32u) {
            uint j = c0 + lane;
            bool head = false;
            if (j + 1u < total) {
                uint f0 = a.tflag[base + j], f1 = a.tflag[base + j + 1u];
                if ((f0 & T_PASTE) != 0u && (f1 & T_PASTE) != 0u) px_refuse(a.ctl, X_PASTE);
                head = (f0 & T_PASTE) == 0u && (f1 & T_PASTE) != 0u && (j == 0u || (a.tflag[base + j - 1u] & T_PASTE) == 0u);
            }
            if (head) {
                uint acc = a.tok[base + j], accf = a.tflag[base + j], accx = a.texp[base + j], w = j;
                uint k = j + 1u;
                for (uint step = 0u; step < total && k + 1u < total && (a.tflag[base + k] & T_PASTE) != 0u; step++) {
                    uint r = a.tok[base + k + 1u], rf = a.tflag[base + k + 1u], rx = a.texp[base + k + 1u];
                    a.tflag[base + k] |= T_DROP;
                    uint f = 0u;
                    bool ok = false;
                    uint m = px_paste(u, a, acc, accf, r, rf, f, ok);
                    if (ok) {
                        // A placemarker gives the other token, its origin kept; a paste is this list's.
                        accx = acc == PLACEMARKER ? rx : r == PLACEMARKER ? accx : l + 1u;
                        acc = m; accf = f;
                        a.tflag[base + k + 1u] |= T_DROP;
                    } else {
                        a.tok[base + w] = acc; a.tflag[base + w] = accf; a.texp[base + w] = accx;
                        w = k + 1u; acc = r; accf = rf; accx = rx;
                    }
                    k += 2u;
                }
                a.tok[base + w] = acc;
                a.tflag[base + w] = accf & ~T_DROP;
                a.texp[base + w] = accx;
            }
        }
        px_fence();
        simdgroup_barrier(mem_flags::mem_device);
    }
    // The compaction, in place: a token's new place is never after its old one.
    uint kept = 0u;
    for (uint c0 = 0u; c0 < total; c0 += 32u) {
        uint j = c0 + lane, t = 0u, f = 0u, x = 0u;
        bool keep = false;
        if (j < total) {
            t = a.tok[base + j];
            f = a.tflag[base + j];
            x = a.texp[base + j];
            keep = t != PLACEMARKER && (f & T_DROP) == 0u;
        }
        uint w = kept + simd_prefix_exclusive_sum(keep ? 1u : 0u);
        simdgroup_barrier(mem_flags::mem_device);
        if (keep) {
            a.tok[base + w] = t;
            a.tflag[base + w] = f & ~(T_PASTE | T_DROP);
            a.tchild[base + w] = 0u;
            a.texp[base + w] = x;
        }
        kept += simd_sum(keep ? 1u : 0u);
        simdgroup_barrier(mem_flags::mem_device);
    }
    if (lane == 0u) {
        L.first = base; L.len = kept;
        if (kept > 0u) a.tflag[base] = (a.tflag[base] & ~T_SPACE) | (par_flags(u, a, L.src, L.at) & T_SPACE);
    }
    px_fence();
    simdgroup_barrier(mem_flags::mem_device);
    px_scan(u, a, l, lane);
}

// PLACE `l`: its pieces at its base (`px_walk`), to the output or (flattening an argument) to
// the arena; a flatten's items counted down, the last done telling the call.
inline void px_place(Ar a, uint l, uint lane) {
    uint dest = a.lists[l].dest;
    px_walk(a, l, lane, true);
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
inline void px_worker(device const PpFile* files, device const uchar* data, device const uint* bases, device const ulong* keys, device const PpDef* defs, device const uint* items, device const ulong* sorted, device const uint* perm, device const uint* n_rows, device const ulong* name_keys, device const uint* values, device const uint* text_nodes, device const uint* gat, device const uint* n_gaps, device const uint* firsts, device const uint* n_toks, device const uint* top_depth, device PxList* lists, device uint* cols, device PxArg* args, device uint* out, device atomic_uint* slots, device atomic_uint* qctl, device atomic_uint* ctl, device const uint* words, device PxScratch* scratch, device const ulong* closes, device const uint* n_close, uint lane) {
    // words: n_files, tok_cap, list_cap, arg_cap, out_cap, q_cap, va_opt (low, high), the scratch
    // bytes' and tokens' capacities.
    Ux u;
    u.files = files; u.data = data; u.bases = bases; u.n_files = words[0]; u.keys = keys; u.defs = defs; u.items = items;
    u.sorted = sorted; u.perm = perm; u.n_rows = n_rows[0]; u.name_keys = name_keys; u.values = values;
    u.text_nodes = text_nodes; u.gat = gat; u.n_gaps = n_gaps[0]; u.firsts = firsts; u.n_toks = n_toks[0];
    u.tdepth = top_depth; u.va_opt = ulong(words[6]) | (ulong(words[7]) << 32);
    // The scratch: its tokens (`st_cap`, an even number), then its bytes.
    device PxScratch* stoks = scratch;
    device uchar* sbytes = (device uchar*)(scratch + words[9]);
    u.sbytes = sbytes; u.stoks = stoks; u.relex_options = words[10];
    u.closes = closes; u.n_close = n_close[0];
    Ar a;
    // The tokens' columns, `tok_cap` words each: the ref, the flags, the child, the depth, the origin.
    uint tc = words[1];
    a.lists = lists; a.tok = cols; a.tflag = cols + tc; a.tchild = cols + 2u * tc; a.tdepth = cols + 3u * tc; a.texp = cols + 4u * tc;
    a.args = args; a.out = out; a.ctl = ctl;
    a.q.slots = slots; a.q.control = qctl; a.q.capacity = words[5];
    a.tok_cap = words[1]; a.list_cap = words[2]; a.arg_cap = words[3]; a.out_cap = words[4];
    a.sbytes = sbytes; a.stoks = stoks; a.sb_cap = words[8]; a.st_cap = words[9];
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

// The top level's rollback (after the queue's first phase, when every list is final): each
// outermost top call with an exit flagged (for a scan and a compaction, in order).
inline void px_top_xflag(device const uint* state, device const uint* oid, device const PxList* lists, device const uint* n_toks, device uint* xflag, uint s) {
    if (s >= n_toks[0]) return;
    xflag[s] = state[s] == 1u && lists[oid[s]].exit != 0u ? 1u : 0u;
}

inline void px_top_xlist(device const uint* xflag, device const uint* xat, device const uint* n_toks, device uint* xpos, uint s) {
    if (s >= n_toks[0] || xflag[s] == 0u) return;
    xpos[xat[s]] = s;
}

// The exit-bearing top calls, in order, by one SIMD group (they are few): each live unless an
// earlier live one's claim covers it (decided one after another, a ballot of 32); `xcover[i]` the
// end of the live claims through call `i` (a running max).
inline void px_top_cover(device const uint* xpos, device const uint* n_x, device const uint* oid, device PxList* lists, device uint* xcover, device atomic_uint* ctl, uint t, uint lane) {
    if (t >= 32u) return;
    Ar a;
    a.lists = lists; a.ctl = ctl;
    uint n = n_x[0], cover = 0u;
    for (uint c0 = 0u; c0 < n; c0 += 32u) {
        uint i = c0 + lane, s = 0u, e = 0u;
        if (i < n) {
            s = xpos[i];
            e = px_claim(a, TOP, oid[s]).end;
        }
        uint mine = 0u;
        uint m = min(32u, n - c0);
        for (uint who = 0u; who < m; who++) {
            uint sw = simd_shuffle(s, ushort(who)), ew = simd_shuffle(e, ushort(who));
            if (sw >= cover) cover = max(cover, ew + 1u);
            if (lane == who) mine = cover;
        }
        if (i < n) xcover[i] = mine;
    }
}

// The first of `n` sorted words at or after `q`.
inline uint px_lower_bound(device const uint* v, uint n, uint q) {
    uint lo = 0u, hi = n;
    while (lo < hi) {
        uint mid = (lo + hi) / 2u;
        if (v[mid] < q) lo = mid + 1u; else hi = mid;
    }
    return lo;
}

// Each top token's piece of the output: 0 inside a live exit's claim (its state made 2), else 1
// for a token no call covers, its list's size and its exits' chain's for an outermost call's name,
// 0 inside a call.
inline void px_top_sizes(device uint* state, device const uint* oid, device PxList* lists, device const uint* n_toks, device uint* v, device const uint* xpos, device const uint* n_x, device const uint* xcover, uint s) {
    if (s >= n_toks[0]) return;
    uint i = px_lower_bound(xpos, n_x[0], s);
    if (i > 0u && xcover[i - 1u] > s) {
        state[s] = 2u;
        v[s] = 0u;
        return;
    }
    uint st = state[s];
    Ar a;
    a.lists = lists;
    v[s] = st == 0u ? 1u : st == 1u ? px_claim(a, TOP, oid[s]).size : 0u;
}

// Each top token at its place (`obase`, the sizes' scan), and each live outermost call's list's
// base and its exits' chain's, their PLACE pushed.
inline void px_top_place(device const uint* state, device const uint* oid, device PxList* lists, device const uint* n_toks, device const uint* obase, device const uint* text_nodes, device const uint* gat, device const uint* n_gaps, device const uint* firsts, device uint* out, device atomic_uint* slots, device atomic_uint* qctl, device atomic_uint* ctl, uint out_cap, uint q_cap, uint s) {
    if (s >= n_toks[0]) return;
    uint st = state[s];
    if (st == 0u) {
        if (obase[s] < out_cap) out[obase[s]] = top_ref(text_nodes, gat, n_gaps[0], firsts, s);
        else px_refuse(ctl, X_ROOM);
    } else if (st == 1u) {
        WorkQueue q; q.slots = slots; q.control = qctl; q.capacity = q_cap;
        uint c = oid[s], b = obase[s];
        for (uint step = 0u; step < MAX_EXITS; step++) {
            lists[c].base = b;
            lists[c].dest = 0u;
            b += lists[c].size;
            px_fence();
            if (!wq_push(q, c | IT_PLACE)) px_refuse(ctl, X_QUEUE);
            uint x = lists[c].exit;
            if (x == 0u || lists[x - 1u].parent != TOP) break;
            c = x - 1u;
        }
    }
}

// After the queue's launches: items left is a refusal; the output's length past its room too.
inline void px_check(device const atomic_uint* qctl, device const uint* n_out, device atomic_uint* ctl, uint out_cap, uint i) {
    if (i != 0u) return;
    if (atomic_load_explicit(&qctl[2], memory_order_relaxed) != 0u || atomic_load_explicit(&qctl[3], memory_order_relaxed) != 0u) px_refuse(ctl, X_QUEUE);
    if (n_out[0] > out_cap) px_refuse(ctl, X_ROOM);
}
