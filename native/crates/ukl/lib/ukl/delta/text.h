// ukl/delta/text.h: a name's text where it lies: a run of bytes in a block, read as it is, or a string's body read
// through its escapes as `rerail_rr_syntax::unescape_each` reads them. Read a character at a time, as UTF-8.
#pragma once
#include <ukl/delta/op.h>

// Bytes `at..end` of `s`; `esc`: a string's body, its escapes read.
typedef struct Text {
    CBlk s;
    uint32_t at, end;
    bool esc;
} Text;

// `text_next`'s answer for an escape `unescape_each` refuses.
#define TEXT_BAD 0xFFFFFFFFu

// A code point's UTF-8, low byte first, into `*u`; its length.
UKL_INLINE uint32_t text_utf8(uint32_t c, UKL_PRIVATE uint32_t* u) {
    if (c < 0x80u) { *u = c; return 1u; }
    if (c < 0x800u) { *u = (0xC0u | c >> 6) | (0x80u | (c & 0x3Fu)) << 8; return 2u; }
    if (c < 0x10000u) { *u = (0xE0u | c >> 12) | (0x80u | (c >> 6 & 0x3Fu)) << 8 | (0x80u | (c & 0x3Fu)) << 16; return 3u; }
    *u = (0xF0u | c >> 18) | (0x80u | (c >> 12 & 0x3Fu)) << 8 | (0x80u | (c >> 6 & 0x3Fu)) << 16 | (0x80u | (c & 0x3Fu)) << 24;
    return 4u;
}

// Bytes `at..end` of `s` as a u32 in base 16, as Rust's `u32::from_str_radix` reads it (one leading `+`); TEXT_BAD
// where it refuses (none, a byte not a digit, past 32 bits).
UKL_INLINE uint32_t text_hex(CBlk s, uint32_t at, uint32_t end) {
    if (at < end && s[at] == '+') at++;
    if (at == end) return TEXT_BAD;
    uint64_t v = 0u;
    for (; at < end; at++) {
        uint32_t c = s[at], d = c - '0' < 10u ? c - '0' : (c | 0x20u) - 'a' < 6u ? (c | 0x20u) - 'a' + 10u : 16u;
        if (d == 16u) return TEXT_BAD;
        v = v * 16u + d;
        if (v > 0xFFFFFFFFull) return TEXT_BAD;
    }
    return (uint32_t)v;
}

// Whether `c` is a character (`char::from_u32`).
UKL_INLINE bool text_char(uint32_t c) { return c < 0xD800u || (c >= 0xE000u && c <= 0x10FFFFu); }

// The end of `n` characters from `at` (fewer to `end`), where each is ASCII; TEXT_BAD where one is not: those hex
// digits `text_hex` refuses anyway.
UKL_INLINE uint32_t text_ascii(CBlk s, uint32_t at, uint32_t end, uint32_t n) {
    uint32_t e = ukl_min(at + n, end);
    for (uint32_t k = at; k < e; k++)
        if (s[k] >= 0x80u) return TEXT_BAD;
    return e;
}

// A `\u` escape's value, its digits from `*at` (`\u{…}` or four characters) and a low surrogate's escape after a
// high one, read past; TEXT_BAD where `unescape_each` refuses it.
UKL_INLINE uint32_t text_unicode(Text x, UKL_PRIVATE uint32_t* at) {
    uint32_t a = *at, lo, hi, past;
    if (a < x.end && x.s[a] == '{') {
        lo = a + 1u;
        for (hi = lo; hi < x.end && x.s[hi] != '}'; hi++) {}
        past = hi < x.end ? hi + 1u : hi;
    } else {
        lo = a;
        hi = past = text_ascii(x.s, a, x.end, 4u);
        if (hi == TEXT_BAD) return TEXT_BAD;
    }
    uint32_t c = text_hex(x.s, lo, hi);
    *at = past;
    if (c == TEXT_BAD) return TEXT_BAD;
    if (c >= 0xD800u && c < 0xDC00u && past + 1u < x.end && x.s[past] == '\\' && x.s[past + 1u] == 'u') {
        uint32_t e = text_ascii(x.s, past + 2u, x.end, 4u);
        uint32_t low = e == TEXT_BAD ? TEXT_BAD : text_hex(x.s, past + 2u, e);
        if (low >= 0xDC00u && low < 0xE000u) {
            *at = e;
            return 0x10000u + ((c - 0xD800u) << 10) + (low - 0xDC00u);
        }
    }
    return text_char(c) ? c : TEXT_BAD;
}

// The text's next character from `*at`, read past: its UTF-8 into `*u` and its length; 0 at the end, TEXT_BAD at an
// escape refused.
UKL_INLINE uint32_t text_next(Text x, UKL_PRIVATE uint32_t* at, UKL_PRIVATE uint32_t* u) {
    if (*at >= x.end) return 0u;
    uint32_t c = x.s[(*at)++];
    if (!x.esc || c != '\\') {
        // A character as written: its UTF-8's bytes, as many as its first says.
        uint32_t n = c < 0x80u ? 1u : c < 0xE0u ? 2u : c < 0xF0u ? 3u : 4u;
        *u = c;
        for (uint32_t k = 1u; k < n && *at < x.end; k++) *u |= (uint32_t)x.s[(*at)++] << (8u * k);
        return n;
    }
    if (*at >= x.end) return text_utf8('\\', u);
    uint32_t e = x.s[(*at)++];
    switch (e) {
    case '"': return text_utf8('"', u);
    case '\\': return text_utf8('\\', u);
    case 'n': return text_utf8('\n', u);
    case 't': return text_utf8('\t', u);
    case 'r': return text_utf8('\r', u);
    case 'u': {
        uint32_t v = text_unicode(x, at);
        return v == TEXT_BAD ? TEXT_BAD : text_utf8(v, u);
    }
    default:
        // Any other escape is itself: the backslash, then the character, read as written.
        (*at)--;
        return text_utf8('\\', u);
    }
}

// Whether every escape of the text is one `unescape_each` reads.
UKL_INLINE bool text_ok(Text x) {
    uint32_t at = x.at, u, n;
    while ((n = text_next(x, &at, &u)) != 0u)
        if (n == TEXT_BAD) return false;
    return true;
}

// The text's value's length in bytes.
UKL_INLINE uint32_t text_len(Text x) {
    uint32_t at = x.at, u, n, len = 0u;
    while ((n = text_next(x, &at, &u)) != 0u && n != TEXT_BAD) len += n;
    return len;
}
