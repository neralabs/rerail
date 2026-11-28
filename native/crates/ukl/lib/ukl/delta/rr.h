// ukl/delta/rr.h: a `.rerail` document read where it lies (`rerail_delta_twins::tape::Reads`): its tape by the reads
// written once (`<ukl/reads/tape.h>`), through the view its symbols block holds (offsets from the tape's first byte);
// a node's kind and field by its code's entry in the language's table (`intern::write_language`); its text in its
// source; its symbols in its symbols block (`intern`'s).
#pragma once
#include <ukl/delta/text.h>
#include <ukl/reads/tape.h>

// A document: its source's and its tape's first bytes, its symbols block, the language's table, and the tape's view
// (the thread's, as the reads take it).
typedef struct Rr {
    CBlk src, tape, syms, lang;
    uint32_t v[VIEW_WORDS];
} Rr;

UKL_INLINE Rr rr_of(CBlk src, uint64_t src_at, CBlk tape, uint64_t tape_at, CBlk syms, CBlk lang) {
    Rr r;
    r.src = src + src_at;
    r.tape = tape + tape_at;
    r.syms = syms;
    r.lang = lang;
    for (uint32_t k = 0u; k < VIEW_WORDS; k++) r.v[k] = U32(syms, SYMS_VIEW_AT + 4u * k);
    return r;
}

// Node `n`'s entry in the language's table: its kind, its field, whether it is extra (a comment).
UKL_INLINE uint32_t rr_entry(UKL_PRIVATE const Rr* r, uint32_t n) { return U32(r->lang, LANG_CODES_AT + 4u * clif_f16_number(r->tape, r->v, n)); }
UKL_INLINE uint32_t rr_kind(UKL_PRIVATE const Rr* r, uint32_t n) { return rr_entry(r, n) & 0xFFFFu; }
UKL_INLINE uint32_t rr_field(UKL_PRIVATE const Rr* r, uint32_t n) { return (rr_entry(r, n) & ~LANG_EXTRA) >> LANG_FIELD; }
UKL_INLINE bool rr_extra(UKL_PRIVATE const Rr* r, uint32_t n) { return (rr_entry(r, n) & LANG_EXTRA) != 0u; }

// Whether node `n` is counted (named, no comment), and whether it is a leaf (no children, no comment).
UKL_INLINE bool rr_counted(UKL_PRIVATE const Rr* r, uint32_t n) { return !rr_extra(r, n) && rr_kind(r, n) < FIRST_ANONYMOUS; }
UKL_INLINE bool rr_leaf(UKL_PRIVATE const Rr* r, uint32_t n) { return !rr_extra(r, n) && clif_f16_first_child(r->tape, r->v, n) == CLIF_T_NONE; }

// Node `p`'s first child, and the child after `c`; NONE past the last.
UKL_INLINE uint32_t rr_first(UKL_PRIVATE const Rr* r, uint32_t p) { return clif_f16_first_child(r->tape, r->v, p); }
UKL_INLINE uint32_t rr_next(UKL_PRIVATE const Rr* r, uint32_t p, uint32_t c) {
    uint32_t e = clif_f16_end_of(r->tape, r->v, c);
    return e < clif_f16_end_of(r->tape, r->v, p) ? e : NONE;
}
#define RR_CHILDREN(r, p, c) for (uint32_t c = rr_first(r, p); c != NONE; c = rr_next(r, p, c))

// Node `p`'s first child of field `f`, or NONE.
UKL_INLINE uint32_t rr_child(UKL_PRIVATE const Rr* r, uint32_t p, uint32_t f) {
    RR_CHILDREN(r, p, c) if (rr_field(r, c) == f) return c;
    return NONE;
}

// Node `n`'s source text.
UKL_INLINE Text rr_text(UKL_PRIVATE const Rr* r, uint32_t n) {
    Text x = {r->src, clif_f16_start(r->tape, r->v, n), clif_f16_end(r->tape, r->v, n), false};
    return x;
}

// A string's value, when node `n` is one and its escapes are: its body between its quotes, its escapes read.
UKL_INLINE bool rr_string(UKL_PRIVATE const Rr* r, uint32_t n, UKL_PRIVATE Text* x) {
    Text s = rr_text(r, n);
    if (rr_kind(r, n) != KIND_STRING || s.end - s.at < 2u || s.s[s.at] != '"' || s.s[s.end - 1u] != '"') return false;
    Text b = {s.s, s.at + 1u, s.end - 1u, true};
    *x = b;
    return text_ok(b);
}

// A number's value, when node `n` is one and its text is an i64 (Rust's `str::parse::<i64>`: a sign, then digits).
UKL_INLINE bool rr_number(UKL_PRIVATE const Rr* r, uint32_t n, UKL_PRIVATE int64_t* v) {
    if (rr_kind(r, n) != KIND_NUMBER) return false;
    Text s = rr_text(r, n);
    uint32_t at = s.at;
    bool neg = at < s.end && s.s[at] == '-';
    if (at < s.end && (s.s[at] == '-' || s.s[at] == '+')) at++;
    if (at == s.end) return false;
    uint64_t m = 0u, limit = neg ? 0x8000000000000000ull : 0x7FFFFFFFFFFFFFFFull;
    for (; at < s.end; at++) {
        uint32_t d = (uint32_t)s.s[at] - '0';
        if (d > 9u || m > (limit - d) / 10u) return false;
        m = m * 10u + d;
    }
    *v = neg ? (int64_t)(0u - m) : (int64_t)m;
    return true;
}

// Node `n`'s symbols (`intern`'s): its text's, and its value's (a string's), 0 for none.
UKL_INLINE uint64_t rr_sym_at(UKL_PRIVATE const Rr* r, uint32_t n) {
    return SYMS_NAMES_AT + ((U32(r->syms, SYMS_PATH_LEN_AT) + U32(r->syms, SYMS_MODULE_LEN_AT) + 7u) & ~7u) + 8u * n;
}
UKL_INLINE uint32_t rr_sym_text(UKL_PRIVATE const Rr* r, uint32_t n) { return U32(r->syms, rr_sym_at(r, n)); }
UKL_INLINE uint32_t rr_sym_value(UKL_PRIVATE const Rr* r, uint32_t n) { return U32(r->syms, rr_sym_at(r, n) + 4u); }

// The language's `k`-th name (`intern::language_names`), as a text in the table.
UKL_INLINE Text rr_lang_name(CBlk lang, uint32_t k) {
    uint32_t nc = U32(lang, LANG_CODES_N_AT), nn = U32(lang, LANG_NAMES_N_AT);
    uint32_t ends = LANG_CODES_AT + 4u * nc, first = ends + 4u * nn;
    Text x = {lang, first + (k > 0u ? U32(lang, ends + 4u * (k - 1u)) : 0u), first + U32(lang, ends + 4u * k), false};
    return x;
}
UKL_INLINE uint32_t rr_lang_names(CBlk lang) { return U32(lang, LANG_NAMES_N_AT); }
